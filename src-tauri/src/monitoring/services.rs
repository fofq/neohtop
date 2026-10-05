//! Windows service enumeration and control
//!
//! Enumerates every installed service (win32 services and drivers, both
//! active and inactive) through the Service Control Manager, reporting the
//! current state, start type and hosting PID of each. Because several
//! services can share one process (svchost), the PID is not unique: the
//! frontend can group the list by PID to split shared hosts into their
//! services. Control operations go through the SCM as well and check the
//! controls a service currently accepts before sending them.

use serde::Serialize;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// A service installed on the system
#[derive(Serialize, Debug)]
pub struct ServiceInfo {
    /// Internal service name, the key used by [`control`]
    pub name: String,
    /// Localized display name
    pub display_name: String,
    /// Current state: "running", "stopped", "paused", "start_pending",
    /// "stop_pending", "continue_pending", "pause_pending" or "unknown"
    pub status: String,
    /// Start type: "auto", "manual", "disabled", "boot", "system" or
    /// "unknown"
    pub start_type: String,
    /// PID of the process hosting the service (0 when stopped)
    pub pid: u32,
    /// Full binary path from the service configuration (empty when
    /// unavailable)
    pub binary_path: String,
}

/// Lists all services with their state, start type and hosting PID
pub fn collect() -> Result<Vec<ServiceInfo>, String> {
    platform::collect()
}

/// Starts, stops, pauses or resumes a service by name
///
/// `action` must be one of "start", "stop", "pause" or "continue". Requires
/// administrator privileges on Windows. The state change itself happens
/// asynchronously: the command only verifies the service accepts the
/// control and that the SCM took the request.
pub fn control(name: &str, action: &str) -> Result<bool, String> {
    platform::control(name, action)
}

/// Changes the start type of one service ("auto" or "disabled"); used by
/// the startup-items panel to toggle auto-start services. Requires
/// elevation.
pub fn set_start_type(name: &str, start_type: &str) -> Result<bool, String> {
    platform::set_start_type(name, start_type)
}

/// Services disabled through the startup-items panel during this
/// session. Disabled auto-start services drop out of the "auto" filter, so
/// the panel needs this bookmark to keep showing (and re-enabling) them;
/// the set is session-scoped by design.
pub(crate) fn disabled_by_panel() -> &'static Mutex<HashSet<String>> {
    static DISABLED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    DISABLED.get_or_init(|| Mutex::new(HashSet::new()))
}

#[cfg(windows)]
mod platform {
    use super::ServiceInfo;
    use crate::monitoring::process_control;
    use crate::monitoring::winbuf;
    use std::collections::HashSet;
    use std::sync::{Mutex, OnceLock};
    use windows_sys::core::PCWSTR;
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_SERVICE_ALREADY_RUNNING, ERROR_SERVICE_CANNOT_ACCEPT_CTRL,
        ERROR_SERVICE_DATABASE_LOCKED, ERROR_SERVICE_DOES_NOT_EXIST, ERROR_SERVICE_NOT_ACTIVE,
        GetLastError,
    };
    use windows_sys::Win32::System::Services::{
        ChangeServiceConfigW, CloseServiceHandle, ControlService,
        ENUM_SERVICE_STATUS_PROCESSW, EnumServicesStatusExW,
        OpenSCManagerW, OpenServiceW, QUERY_SERVICE_CONFIGW, QueryServiceConfigW,
        QueryServiceStatusEx, SC_ENUM_PROCESS_INFO, SC_HANDLE, SC_MANAGER_CONNECT,
        SC_MANAGER_ENUMERATE_SERVICE, SC_STATUS_PROCESS_INFO, SERVICE_ACCEPT_PAUSE_CONTINUE,
        SERVICE_ACCEPT_STOP, SERVICE_AUTO_START, SERVICE_BOOT_START, SERVICE_CONTROL_CONTINUE,
        SERVICE_CONTROL_PAUSE, SERVICE_CONTROL_STOP, SERVICE_CONTINUE_PENDING,
        SERVICE_DEMAND_START, SERVICE_DISABLED, SERVICE_DRIVER, SERVICE_PAUSED,
        SERVICE_PAUSE_CONTINUE, SERVICE_PAUSE_PENDING, SERVICE_QUERY_CONFIG, SERVICE_QUERY_STATUS,
        SERVICE_NO_CHANGE, SERVICE_RUNNING, SERVICE_CHANGE_CONFIG, SERVICE_START,
        SERVICE_START_PENDING, SERVICE_STATE_ALL, SERVICE_STOP,
        SERVICE_STOPPED, SERVICE_STOP_PENDING, SERVICE_STATUS, SERVICE_STATUS_PROCESS,
        SERVICE_SYSTEM_START, SERVICE_WIN32, StartServiceW,
    };

    /// Initial size for the service list buffer; grown when the API reports
    /// the buffer is too small
    const INITIAL_BUFFER_SIZE: usize = 16 * 1024;

    /// How many times the service list query is retried with a larger
    /// buffer (services can be installed while we enumerate)
    const MAX_BUFFER_GROWTH_RETRIES: u32 = 3;

    /// Access mask used to connect to the local SCM for enumeration; the
    /// narrow mask works without elevation
    const SCM_ENUMERATE_ACCESS: u32 = SC_MANAGER_CONNECT | SC_MANAGER_ENUMERATE_SERVICE;

    pub fn collect() -> Result<Vec<ServiceInfo>, String> {
        let scm = open_scm(SCM_ENUMERATE_ACCESS)?;
        // usize backing gives the buffer pointer alignment, required by the
        // PWSTR fields inside ENUM_SERVICE_STATUS_PROCESSW
        let mut buffer: Vec<usize> = vec![0; INITIAL_BUFFER_SIZE / std::mem::size_of::<usize>()];
        let mut size = (buffer.len() * std::mem::size_of::<usize>()) as u32;
        let mut returned: u32 = 0;
        let mut succeeded = false;
        for _ in 0..MAX_BUFFER_GROWTH_RETRIES {
            let mut needed: u32 = 0;
            // The resume handle is reset to 0 on every attempt: each retry
            // re-enumerates the whole table instead of continuing a partial
            // pass, so a big-enough buffer returns every service at once
            let mut resume: u32 = 0;
            // SAFETY: buffer is sized to `size` bytes and holds no
            // initialized data; every out-parameter points to our locals
            let ok = unsafe {
                EnumServicesStatusExW(
                    scm,
                    SC_ENUM_PROCESS_INFO,
                    SERVICE_WIN32 | SERVICE_DRIVER,
                    SERVICE_STATE_ALL,
                    buffer.as_mut_ptr() as *mut u8,
                    size,
                    &mut needed,
                    &mut returned,
                    &mut resume,
                    std::ptr::null(),
                )
            };
            if ok != 0 {
                succeeded = true;
                break;
            }
            let error = unsafe { GetLastError() };
            // ERROR_MORE_DATA (not ERROR_INSUFFICIENT_BUFFER) is the
            // documented too-small signal of this API
            if !winbuf::is_buffer_too_small(error) {
                // SAFETY: scm is the valid handle from open_scm
                unsafe { CloseServiceHandle(scm) };
                return Err(format!(
                    "Failed to enumerate services (Windows error {})",
                    error
                ));
            }
            // `needed` holds the bytes required for the entries that did not
            // fit, on top of whatever the failed attempt already packed into
            // the buffer, so the whole table needs up to the old size plus
            // `needed` bytes. Re-enumerating from scratch with that much
            // room returns everything in one pass. checked_add refuses a sum
            // that would overflow the usize math on 32-bit targets.
            let byte_len = match (size as usize).checked_add(needed as usize) {
                Some(len) => len,
                None => {
                    // SAFETY: scm is the valid handle from open_scm
                    unsafe { CloseServiceHandle(scm) };
                    return Err(
                        "Failed to enumerate services: the required size overflowed".to_string(),
                    );
                }
            };
            buffer = vec![0; (byte_len + std::mem::size_of::<usize>() - 1)
                / std::mem::size_of::<usize>()];
            size = (buffer.len() * std::mem::size_of::<usize>()) as u32;
        }
        if !succeeded {
            // SAFETY: scm is the valid handle from open_scm
            unsafe { CloseServiceHandle(scm) };
            return Err("Failed to enumerate services: the service list kept growing".to_string());
        }

        // SAFETY: on success the buffer holds `returned`
        // ENUM_SERVICE_STATUS_PROCESSW entries whose PWSTR fields point
        // into the same buffer, which stays alive while they are read
        let entries = unsafe {
            std::slice::from_raw_parts(
                buffer.as_ptr() as *const ENUM_SERVICE_STATUS_PROCESSW,
                returned as usize,
            )
        };
        let mut services = Vec::with_capacity(entries.len());
        for entry in entries {
            let name = pwstr_to_string(entry.lpServiceName);
            if name.is_empty() {
                continue;
            }
            let (start_type, binary_path) = query_config(scm, &name);
            services.push(ServiceInfo {
                display_name: pwstr_to_string(entry.lpDisplayName),
                status: state_name(entry.ServiceStatusProcess.dwCurrentState).to_string(),
                start_type,
                pid: entry.ServiceStatusProcess.dwProcessId,
                name,
                binary_path,
            });
        }
        // SAFETY: scm is the valid handle from open_scm
        unsafe { CloseServiceHandle(scm) };
        Ok(services)
    }

    pub fn control(name: &str, action: &str) -> Result<bool, String> {
        let Some((access, control_code, verb, gerund)) = (match action {
            "start" => Some((SERVICE_START, None, "start", "Starting")),
            "stop" => Some((
                SERVICE_STOP | SERVICE_QUERY_STATUS,
                Some(SERVICE_CONTROL_STOP),
                "stop",
                "Stopping",
            )),
            "pause" => Some((
                SERVICE_PAUSE_CONTINUE | SERVICE_QUERY_STATUS,
                Some(SERVICE_CONTROL_PAUSE),
                "pause",
                "Pausing",
            )),
            "continue" => Some((
                SERVICE_PAUSE_CONTINUE | SERVICE_QUERY_STATUS,
                Some(SERVICE_CONTROL_CONTINUE),
                "resume",
                "Resuming",
            )),
            _ => None,
        }) else {
            return Err(format!(
                "Unknown service action '{}': expected start, stop, pause or continue",
                action
            ));
        };
        if !process_control::is_elevated() {
            return Err(format!(
                "{} a service requires administrator privileges; restart the app as administrator first",
                gerund
            ));
        }
        let scm = open_scm(SC_MANAGER_CONNECT)?;
        let name_wide = to_wide(name);
        // SAFETY: name_wide is a NUL-terminated buffer; the returned handle
        // is closed on every path below
        let service = unsafe { OpenServiceW(scm, name_wide.as_ptr(), access) };
        if service.is_null() {
            let error = unsafe { GetLastError() };
            // SAFETY: scm is the valid handle from open_scm
            unsafe { CloseServiceHandle(scm) };
            return Err(match error {
                ERROR_SERVICE_DOES_NOT_EXIST => format!("No service named '{}' exists", name),
                ERROR_ACCESS_DENIED => format!(
                    "Accessing the service '{}' requires administrator privileges (Windows error {})",
                    name, error
                ),
                _ => format!("Failed to open the service '{}' (Windows error {})", name, error),
            });
        }
        let outcome = match control_code {
            None => start_service(service, name),
            Some(code) => control_running_service(service, name, code, verb),
        };
        // SAFETY: service and scm are the valid handles opened above
        unsafe { CloseServiceHandle(service) };
        unsafe { CloseServiceHandle(scm) };
        outcome
    }

    /// Changes a service's start type between auto-start and disabled via
    /// ChangeServiceConfigW. Requires elevation.
    pub fn set_start_type(name: &str, start_type: &str) -> Result<bool, String> {
        let desired = match start_type {
            "auto" => SERVICE_AUTO_START,
            "disabled" => SERVICE_DISABLED,
            _ => {
                return Err(format!(
                    "Unsupported service start type '{}': expected auto or disabled",
                    start_type
                ))
            }
        };
        if !process_control::is_elevated() {
            return Err(
                "Changing a service start type requires administrator privileges; restart the app as administrator first"
                    .to_string(),
            );
        }
        let scm = open_scm(SC_MANAGER_CONNECT)?;
        let name_wide = to_wide(name);
        // SAFETY: name_wide is a NUL-terminated buffer; both handles are
        // closed on every path below
        let service = unsafe {
            OpenServiceW(scm, name_wide.as_ptr(), SERVICE_CHANGE_CONFIG)
        };
        if service.is_null() {
            let error = unsafe { GetLastError() };
            // SAFETY: scm is the valid handle from open_scm
            unsafe { CloseServiceHandle(scm) };
            return Err(match error {
                ERROR_SERVICE_DOES_NOT_EXIST => {
                    format!("No service named '{}' exists", name)
                }
                ERROR_ACCESS_DENIED => format!(
                    "Changing the start type of '{}' requires administrator privileges (Windows error {})",
                    name, error
                ),
                _ => format!(
                    "Failed to open the service '{}' (Windows error {})",
                    name, error
                ),
            });
        }
        // SAFETY: service is the valid handle above; every SERVICE_NO_CHANGE
        // / null argument leaves the corresponding field untouched
        let result = unsafe {
            ChangeServiceConfigW(
                service,
                SERVICE_NO_CHANGE,
                desired,
                SERVICE_NO_CHANGE,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        // SAFETY: service and scm are the valid handles opened above
        unsafe { CloseServiceHandle(service) };
        unsafe { CloseServiceHandle(scm) };
        if result == 0 {
            return Err(format!(
                "Failed to change the start type of '{}' (Windows error {})",
                name,
                unsafe { GetLastError() }
            ));
        }
        // Bookmark the panel's own change so the disabled service stays
        // visible (and re-enableable) in the startup-items panel
        let mut disabled = super::disabled_by_panel()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if desired == SERVICE_DISABLED {
            disabled.insert(name.to_string());
        } else {
            disabled.remove(name);
        }
        Ok(true)
    }

    /// Starts a service that is not running yet
    fn start_service(service: SC_HANDLE, name: &str) -> Result<bool, String> {
        // SAFETY: service is the valid handle from OpenServiceW; no
        // arguments are passed to the service
        let ok = unsafe { StartServiceW(service, 0, std::ptr::null()) };
        if ok == 0 {
            let error = unsafe { GetLastError() };
            return Err(match error {
                ERROR_SERVICE_ALREADY_RUNNING => format!("The service '{}' is already running", name),
                ERROR_SERVICE_DATABASE_LOCKED => format!(
                    "The service database is locked; try again in a moment (Windows error {})",
                    error
                ),
                _ => format!("Failed to start the service '{}' (Windows error {})", name, error),
            });
        }
        Ok(true)
    }

    /// Sends a stop/pause/continue control after checking the service
    /// accepts it in its current state
    fn control_running_service(
        service: SC_HANDLE,
        name: &str,
        code: u32,
        verb: &str,
    ) -> Result<bool, String> {
        let mut status_process: SERVICE_STATUS_PROCESS = unsafe { std::mem::zeroed() };
        let mut needed: u32 = 0;
        // SAFETY: status_process is a zeroed, correctly sized
        // SERVICE_STATUS_PROCESS and needed is a plain out-parameter
        let ok = unsafe {
            QueryServiceStatusEx(
                service,
                SC_STATUS_PROCESS_INFO,
                &mut status_process as *mut SERVICE_STATUS_PROCESS as *mut u8,
                std::mem::size_of::<SERVICE_STATUS_PROCESS>() as u32,
                &mut needed,
            )
        };
        if ok == 0 {
            let error = unsafe { GetLastError() };
            return Err(format!(
                "Failed to query the state of the service '{}' (Windows error {})",
                name, error
            ));
        }
        let accepted_flag = if code == SERVICE_CONTROL_STOP {
            SERVICE_ACCEPT_STOP
        } else {
            SERVICE_ACCEPT_PAUSE_CONTINUE
        };
        if status_process.dwControlsAccepted & accepted_flag == 0 {
            return Err(format!(
                "The service '{}' does not accept the '{}' control in its current state ({})",
                name,
                verb,
                state_name(status_process.dwCurrentState)
            ));
        }
        let mut status: SERVICE_STATUS = unsafe { std::mem::zeroed() };
        // SAFETY: service is the valid handle from OpenServiceW and status
        // is a zeroed SERVICE_STATUS out-parameter
        let ok = unsafe { ControlService(service, code, &mut status) };
        if ok == 0 {
            let error = unsafe { GetLastError() };
            return Err(match error {
                ERROR_SERVICE_NOT_ACTIVE => format!("The service '{}' is not running", name),
                ERROR_SERVICE_CANNOT_ACCEPT_CTRL => format!(
                    "The service '{}' cannot accept the '{}' control right now",
                    name, verb
                ),
                _ => format!("Failed to {} the service '{}' (Windows error {})", verb, name, error),
            });
        }
        Ok(true)
    }

    /// Connects to the local service control manager
    fn open_scm(access: u32) -> Result<SC_HANDLE, String> {
        // SAFETY: null machine/database names mean the local machine; the
        // returned handle is closed by the caller
        let handle = unsafe { OpenSCManagerW(std::ptr::null(), std::ptr::null(), access) };
        if handle.is_null() {
            let error = unsafe { GetLastError() };
            return Err(match error {
                ERROR_ACCESS_DENIED => format!(
                    "Access to the service control manager was denied (Windows error {})",
                    error
                ),
                _ => format!(
                    "Failed to open the service control manager (Windows error {})",
                    error
                ),
            });
        }
        Ok(handle)
    }

    /// Reads the start type and binary path of one service via
    /// QueryServiceConfigW
    fn query_config(scm: SC_HANDLE, name: &str) -> (String, String) {
        let name_wide = to_wide(name);
        // SAFETY: name_wide is a NUL-terminated buffer; the handle is
        // closed on every path below
        let service = unsafe { OpenServiceW(scm, name_wide.as_ptr(), SERVICE_QUERY_CONFIG) };
        if service.is_null() {
            return ("unknown".to_string(), String::new());
        }
        let mut needed: u32 = 0;
        // SAFETY: the null config buffer makes the API report the required
        // size in `needed`
        let ok = unsafe { QueryServiceConfigW(service, std::ptr::null_mut(), 0, &mut needed) };
        let mut start_type = None;
        let mut binary_path = None;
        if ok == 0 {
            let error = unsafe { GetLastError() };
            if winbuf::is_buffer_too_small(error) && needed > 0 {
                // usize backing again: QUERY_SERVICE_CONFIGW holds PWSTRs
                let mut buffer: Vec<usize> = vec![0; (needed as usize
                    + std::mem::size_of::<usize>()
                    - 1)
                    / std::mem::size_of::<usize>()];
                let size = (buffer.len() * std::mem::size_of::<usize>()) as u32;
                // SAFETY: buffer is sized to `size` bytes; the PWSTR fields
                // written by the API point into this same buffer, which
                // outlives the field read below
                let ok = unsafe {
                    QueryServiceConfigW(
                        service,
                        buffer.as_mut_ptr() as *mut QUERY_SERVICE_CONFIGW,
                        size,
                        &mut needed,
                    )
                };
                if ok != 0 {
                    // SAFETY: the buffer now holds a valid config struct;
                    // both fields are read while the buffer is alive
                    let config =
                        unsafe { *(buffer.as_ptr() as *const QUERY_SERVICE_CONFIGW) };
                    start_type = Some(config.dwStartType);
                    binary_path = Some(pwstr_to_string(config.lpBinaryPathName));
                }
            }
        }
        // SAFETY: service is the valid handle from OpenServiceW
        unsafe { CloseServiceHandle(service) };
        match (start_type, binary_path) {
            (Some(start_type), binary_path) => (
                start_type_name(start_type).to_string(),
                binary_path.unwrap_or_default(),
            ),
            (None, _) => ("unknown".to_string(), String::new()),
        }
    }

    /// Maps a SERVICE_STATUS_PROCESS current state to its name
    fn state_name(state: u32) -> &'static str {
        match state {
            SERVICE_STOPPED => "stopped",
            SERVICE_START_PENDING => "start_pending",
            SERVICE_STOP_PENDING => "stop_pending",
            SERVICE_RUNNING => "running",
            SERVICE_CONTINUE_PENDING => "continue_pending",
            SERVICE_PAUSE_PENDING => "pause_pending",
            SERVICE_PAUSED => "paused",
            _ => "unknown",
        }
    }

    /// Maps a QUERY_SERVICE_CONFIGW start type to its name
    fn start_type_name(start_type: u32) -> &'static str {
        match start_type {
            SERVICE_BOOT_START => "boot",
            SERVICE_SYSTEM_START => "system",
            SERVICE_AUTO_START => "auto",
            SERVICE_DEMAND_START => "manual",
            SERVICE_DISABLED => "disabled",
            _ => "unknown",
        }
    }

    /// Reads a NUL-terminated UTF-16 PWSTR into a String
    fn pwstr_to_string(pointer: PCWSTR) -> String {
        if pointer.is_null() {
            return String::new();
        }
        // SAFETY: the API guarantees the PWSTRs point at NUL-terminated
        // UTF-16 strings inside the buffer that outlives this read
        unsafe {
            let mut len = 0usize;
            while *pointer.add(len) != 0 {
                len += 1;
            }
            String::from_utf16_lossy(std::slice::from_raw_parts(pointer, len))
        }
    }

    /// Encodes a string as a NUL-terminated UTF-16 buffer for Win32 APIs
    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn collect() -> Result<Vec<super::ServiceInfo>, String> {
        Err("Service enumeration is only supported on Windows".to_string())
    }

    pub fn control(_name: &str, _action: &str) -> Result<bool, String> {
        Err("Controlling services is only supported on Windows".to_string())
    }

    pub fn set_start_type(_name: &str, _start_type: &str) -> Result<bool, String> {
        Err("Service management is only supported on Windows".to_string())
    }
}
