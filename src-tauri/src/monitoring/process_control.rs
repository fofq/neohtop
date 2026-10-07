//! Process control operations
//!
//! Implements suspend/resume, priority class, CPU affinity and efficiency
//! mode (power throttling) control for a single process, whole-process-tree
//! kills, plus elevation helpers the frontend uses to decide when to offer
//! relaunching the app with administrator privileges. State-changing
//! operations return `Ok(true)` on success so the frontend can track the
//! state it changed; the tree kill instead reports its outcome as a
//! [`KillTreeResult`] with the requested and killed counts.

use serde::Serialize;
use std::collections::{HashMap, HashSet, VecDeque};
// Re-exported by the parent module from process_monitor.rs; reused here so
// the per-PID kill behaves exactly like the single-process command
use super::ProcessMonitor;

/// Priority, affinity and efficiency-mode information of a process
///
/// The affinity masks are serialized as decimal strings because they are
/// 64-bit values that lose precision as JSON numbers above 2^53.
#[derive(Serialize, Debug)]
pub struct ProcessPriorityInfo {
    /// Priority class name: one of "idle", "below_normal", "normal",
    /// "above_normal", "high" or "realtime"
    pub priority_class: String,
    /// CPU affinity mask of the process, as a decimal string
    pub affinity_mask: String,
    /// CPU affinity mask of the system (which CPUs the process may use),
    /// as a decimal string
    pub system_affinity_mask: String,
    /// Whether efficiency mode (power throttling) is enabled for the process
    pub efficiency_mode: bool,
}

/// Outcome of a whole-process-tree kill
#[derive(Serialize, Debug)]
pub struct KillTreeResult {
    /// Number of processes the traversal collected to kill: every descendant
    /// of the target plus the target itself
    pub requested: u32,
    /// How many of those processes were actually killed successfully
    pub killed: u32,
}

/// Suspends all threads of the process with the given PID
pub fn suspend(pid: u32) -> Result<bool, String> {
    platform::suspend(pid)
}

/// Resumes a process that was suspended with [`suspend`]
pub fn resume(pid: u32) -> Result<bool, String> {
    platform::resume(pid)
}

/// Reads the priority class, affinity masks and efficiency mode of a process
pub fn get_priority_info(pid: u32) -> Result<ProcessPriorityInfo, String> {
    platform::get_priority_info(pid)
}

/// Sets the priority class of a process
///
/// `class` must be one of "idle", "below_normal", "normal", "above_normal",
/// "high" or "realtime".
pub fn set_priority(pid: u32, class: &str) -> Result<bool, String> {
    platform::set_priority(pid, class)
}

/// Sets the CPU affinity mask of a process
pub fn set_affinity(pid: u32, mask: u64) -> Result<bool, String> {
    platform::set_affinity(pid, mask)
}

/// Enables or disables efficiency mode (power throttling) for a process
pub fn set_efficiency(pid: u32, enabled: bool) -> Result<bool, String> {
    platform::set_efficiency(pid, enabled)
}

/// Returns whether the current process runs with elevated privileges
pub fn is_elevated() -> bool {
    platform::is_elevated()
}

/// Relaunches the app with administrator privileges
///
/// Triggers the UAC prompt on Windows and terminates the current instance
/// once the elevated instance has been launched. Returns `Ok(false)` when
/// the user declined the prompt.
pub fn restart_as_admin() -> Result<bool, String> {
    platform::restart_as_admin()
}

/// Kills the whole process tree rooted at `pid`
///
/// Builds a parent-to-children map from the parent PID of every process in
/// the current snapshot, collects the descendants of `pid` with a BFS, then
/// kills them in reverse discovery order (children before their parents) and
/// the target last, so a dying parent cannot re-parent its children mid-run.
/// Descendants that refuse to die (e.g. protected processes) are skipped and
/// only reflected in the killed count.
///
/// Must be called while holding the caller's `sys` lock: everything here is
/// synchronous, so the lock is never held across an `.await`.
pub fn kill_tree(sys: &sysinfo::System, pid: u32) -> Result<KillTreeResult, String> {
    let target_pid = sysinfo::Pid::from(pid as usize);
    if sys.process(target_pid).is_none() {
        return Err(format!("Process with PID {} not found", pid));
    }

    let mut children_of: HashMap<u32, Vec<u32>> = HashMap::new();
    for (child, process) in sys.processes() {
        if let Some(parent) = process.parent() {
            children_of
                .entry(parent.as_u32())
                .or_default()
                .push(child.as_u32());
        }
    }

    // BFS discovery order guarantees every child is discovered after its
    // parent, so the reversed order kills children before their ancestors
    let mut descendants: Vec<u32> = Vec::new();
    let mut visited: HashSet<u32> = HashSet::new();
    visited.insert(pid);
    let mut queue: VecDeque<u32> = VecDeque::new();
    queue.push_back(pid);
    while let Some(current) = queue.pop_front() {
        if let Some(children) = children_of.get(&current) {
            for &child in children {
                if visited.insert(child) {
                    queue.push_back(child);
                    descendants.push(child);
                }
            }
        }
    }

    let mut killed: u32 = 0;
    for descendant in descendants.iter().rev() {
        if ProcessMonitor::kill_process(sys, *descendant) {
            killed += 1;
        }
    }
    // The target dies last, after its whole subtree is gone
    if ProcessMonitor::kill_process(sys, pid) {
        killed += 1;
    }

    Ok(KillTreeResult {
        requested: descendants.len() as u32 + 1,
        killed,
    })
}

/// Outcome of a deep kill: how many established TCP connections belonged to
/// the target, how many the backend actually closed, and whether the kill
/// phase itself succeeded.
#[derive(Serialize, Debug)]
pub struct DeepKillReport {
    /// Established IPv4 TCP connections of the target found in the table
    pub attempted: u32,
    /// Connections the close call accepted (SET: deletion was queued)
    pub closed: u32,
    /// Whether the kill phase reported success
    pub killed: bool,
}

/// Deep kill (port-killer #100): closes the target's established TCP
/// connections first — so nothing keeps a dying process alive as a
/// connection holder — then kills the process itself. Connection closing is
/// IPv4-only (`SetTcpEntry` has no IPv6 equivalent) and elevation-gated;
/// close failures never block the kill phase, they only show up in the
/// report. Must be called while holding the caller's `sys` lock.
pub fn deep_kill(pid: u32, sys: &sysinfo::System) -> Result<DeepKillReport, String> {
    // PID 0 (System Idle) and, on Windows, PID 4 (System) are kernel
    // pseudo-processes: they show up in the connection table through other
    // processes' TIME_WAIT entries and must never be offered a kill. On
    // other platforms low PIDs are ordinary processes, so only 0 is refused.
    if pid == 0 || (cfg!(windows) && pid <= 4) {
        return Err(format!(
            "Refusing to kill PID {}: kernel pseudo-process, not a user process",
            pid
        ));
    }
    let mut report = close_process_connections(pid);
    report.killed = ProcessMonitor::kill_process(sys, pid);
    Ok(report)
}

/// Closes every established IPv4 TCP connection owned by `pid`, returning
/// the close-phase tally. Non-Windows platforms (no `SetTcpEntry` backend)
/// and table-query failures report an empty tally; neither is an error —
/// the kill phase always runs.
#[cfg(windows)]
fn close_process_connections(pid: u32) -> DeepKillReport {
    let connections = match crate::monitoring::network_ports::collect() {
        Ok(connections) => connections,
        Err(_) => {
            return DeepKillReport {
                attempted: 0,
                closed: 0,
                killed: false,
            }
        }
    };
    let mut attempted = 0;
    let mut closed = 0;
    for connection in connections.iter().filter(|c| {
        c.pid == pid
            && c.protocol == "TCP"
            && c.state == "ESTABLISHED"
            // IPv6 endpoints have no SetTcpEntry equivalent; skip them
            && !c.local_addr.contains(':')
    }) {
        attempted += 1;
        let closed_ok = crate::monitoring::tcp_control::close(
            &connection.local_addr,
            connection.local_port,
            &connection.remote_addr,
            connection.remote_port,
            pid,
        )
        .unwrap_or(false);
        if closed_ok {
            closed += 1;
        }
    }
    DeepKillReport {
        attempted,
        closed,
        killed: false,
    }
}

/// Connection closing is a Windows-only capability; elsewhere the report
/// carries an empty close tally and the kill phase does all the work.
#[cfg(not(windows))]
fn close_process_connections(_pid: u32) -> DeepKillReport {
    DeepKillReport {
        attempted: 0,
        closed: 0,
        killed: false,
    }
}

#[cfg(windows)]
mod platform {
    use super::ProcessPriorityInfo;
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{
        BOOL, CloseHandle, ERROR_ACCESS_DENIED, ERROR_CANCELLED, ERROR_INVALID_PARAMETER,
        GetLastError, HANDLE, NTSTATUS,
    };
    use windows_sys::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows_sys::Win32::System::Threading::{
        ABOVE_NORMAL_PRIORITY_CLASS, BELOW_NORMAL_PRIORITY_CLASS, GetCurrentProcess,
        GetPriorityClass, GetProcessAffinityMask, GetProcessInformation, HIGH_PRIORITY_CLASS,
        IDLE_PRIORITY_CLASS, NORMAL_PRIORITY_CLASS, OpenProcess, OpenProcessToken,
        PROCESS_ACCESS_RIGHTS, PROCESS_INFORMATION_CLASS, PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
        PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION,
        PROCESS_SUSPEND_RESUME,
        REALTIME_PRIORITY_CLASS, SetPriorityClass, SetProcessAffinityMask, SetProcessInformation,
    };
    use windows_sys::Win32::UI::Shell::{SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    /// NtSuspendProcess/NtResumeProcess are undocumented APIs exported by
    /// ntdll.dll; windows-sys does not declare them, so they are declared
    /// here and resolved through the ntdll.lib import library that ships
    /// with the Windows SDK. Available since Windows XP; they return
    /// NTSTATUS 0 (STATUS_SUCCESS) on success.
    /// Source: same declarations used by the ntapi crate / Process Hacker.
    #[link(name = "ntdll")]
    extern "system" {
        fn NtSuspendProcess(processhandle: HANDLE) -> NTSTATUS;
        fn NtResumeProcess(processhandle: HANDLE) -> NTSTATUS;
    }

    /// All supported priority classes by name and raw value
    const PRIORITY_CLASSES: [(&str, u32); 6] = [
        ("idle", IDLE_PRIORITY_CLASS),
        ("below_normal", BELOW_NORMAL_PRIORITY_CLASS),
        ("normal", NORMAL_PRIORITY_CLASS),
        ("above_normal", ABOVE_NORMAL_PRIORITY_CLASS),
        ("high", HIGH_PRIORITY_CLASS),
        ("realtime", REALTIME_PRIORITY_CLASS),
    ];

    /// Opens a process handle, mapping access-denied to a message the
    /// frontend recognizes to offer a privileged relaunch
    fn open_process(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Result<HANDLE, String> {
        // SAFETY: OpenProcess only reads its arguments and returns a fresh
        // handle (or null) that we own and close on every path
        let handle = unsafe { OpenProcess(access, 0, pid) };
        if handle.is_null() {
            let error = unsafe { GetLastError() };
            return Err(match error {
                ERROR_ACCESS_DENIED => format!(
                    "Access denied when opening process {} (Windows error {}): the process is protected or the operation requires administrator privileges",
                    pid, error
                ),
                ERROR_INVALID_PARAMETER => format!(
                    "Failed to open process {} (Windows error {}): the process may no longer exist",
                    pid, error
                ),
                _ => format!("Failed to open process {} (Windows error {})", pid, error),
            });
        }
        Ok(handle)
    }

    /// Closes the process handle and turns a BOOL API result into a command
    /// result. The error code is captured before CloseHandle because that
    /// call may overwrite the thread's last-error value.
    fn finish(handle: HANDLE, ok: BOOL, pid: u32, action: &str) -> Result<bool, String> {
        let error = if ok == 0 {
            unsafe { GetLastError() }
        } else {
            0
        };
        // SAFETY: handle is the valid process handle from open_process
        unsafe { CloseHandle(handle) };
        if ok != 0 {
            Ok(true)
        } else {
            Err(format!(
                "Failed to {} process {} (Windows error {})",
                action, pid, error
            ))
        }
    }

    /// Turns an NTSTATUS into a command result; 0 is STATUS_SUCCESS
    fn check_nt_status(pid: u32, status: NTSTATUS, action: &str) -> Result<bool, String> {
        if status == 0 {
            Ok(true)
        } else {
            Err(format!(
                "Failed to {} process {} (NTSTATUS 0x{:08X})",
                action, pid, status
            ))
        }
    }

    pub fn suspend(pid: u32) -> Result<bool, String> {
        let handle = open_process(pid, PROCESS_SUSPEND_RESUME)?;
        // SAFETY: handle is valid with PROCESS_SUSPEND_RESUME access
        let status = unsafe { NtSuspendProcess(handle) };
        unsafe { CloseHandle(handle) };
        check_nt_status(pid, status, "suspend")
    }

    pub fn resume(pid: u32) -> Result<bool, String> {
        let handle = open_process(pid, PROCESS_SUSPEND_RESUME)?;
        // SAFETY: handle is valid with PROCESS_SUSPEND_RESUME access
        let status = unsafe { NtResumeProcess(handle) };
        unsafe { CloseHandle(handle) };
        check_nt_status(pid, status, "resume")
    }

    pub fn get_priority_info(pid: u32) -> Result<ProcessPriorityInfo, String> {
        // GetPriorityClass/GetProcessAffinityMask/GetProcessInformation all
        // accept a limited-query handle, and the full PROCESS_QUERY_INFORMATION
        // is denied by some service-hosted processes even for elevated admins
        // (error 5) — the very processes this app's metadata query reads fine
        // with the limited right.
        let handle = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        // SAFETY: all three calls below only write into our own output
        // variables and the handle stays valid until CloseHandle
        let raw_class = unsafe { GetPriorityClass(handle) };
        let mut affinity: usize = 0;
        let mut system_affinity: usize = 0;
        let masks_ok = unsafe {
            GetProcessAffinityMask(handle, &mut affinity, &mut system_affinity)
        };
        // Efficiency mode: only meaningful when GetProcessInformation knows
        // about ProcessPowerThrottling (Windows 10 1709+); on older systems
        // it fails and the process is simply reported as not throttled
        let mut throttling = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: 0,
            StateMask: 0,
        };
        let throttling_ok = unsafe {
            GetProcessInformation(
                handle,
                PROCESS_POWER_THROTTLING_EXECUTION_SPEED as PROCESS_INFORMATION_CLASS,
                &mut throttling as *mut PROCESS_POWER_THROTTLING_STATE as *mut c_void,
                std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
            )
        };
        // Efficiency mode is on when the EXECUTION_SPEED mechanism is taken
        // over by us (ControlMask) AND declared on (StateMask), per the
        // SetProcessInformation docs; note windows-sys 0.59 has no separate
        // ENABLE_STATE/DISABLE_STATE constants (dropped from the SDK), the
        // StateMask reuses the EXECUTION_SPEED flag itself
        let efficiency_mode = throttling_ok != 0
            && throttling.ControlMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED != 0
            && throttling.StateMask & PROCESS_POWER_THROTTLING_EXECUTION_SPEED != 0;
        if raw_class == 0 {
            let error = unsafe { GetLastError() };
            unsafe { CloseHandle(handle) };
            return Err(format!(
                "Failed to query the priority class of process {} (Windows error {})",
                pid, error
            ));
        }
        if masks_ok == 0 {
            let error = unsafe { GetLastError() };
            unsafe { CloseHandle(handle) };
            return Err(format!(
                "Failed to query the affinity mask of process {} (Windows error {})",
                pid, error
            ));
        }
        unsafe { CloseHandle(handle) };
        let priority_class = PRIORITY_CLASSES
            .iter()
            .find(|(_, value)| *value == raw_class)
            .map(|(name, _)| (*name).to_string())
            .unwrap_or_else(|| format!("unknown ({})", raw_class));
        Ok(ProcessPriorityInfo {
            priority_class,
            affinity_mask: affinity.to_string(),
            system_affinity_mask: system_affinity.to_string(),
            efficiency_mode,
        })
    }

    pub fn set_priority(pid: u32, class: &str) -> Result<bool, String> {
        let class_value = PRIORITY_CLASSES
            .iter()
            .find(|(name, _)| *name == class)
            .map(|(_, value)| *value)
            .ok_or_else(|| {
                format!(
                    "Unknown priority class '{}': expected one of {}",
                    class,
                    PRIORITY_CLASSES
                        .iter()
                        .map(|(name, _)| *name)
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            })?;
        let handle = open_process(pid, PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION)?;
        // SAFETY: handle is valid with PROCESS_SET_INFORMATION access and
        // class_value is one of the documented PRIORITY_CLASS values
        let ok = unsafe { SetPriorityClass(handle, class_value) };
        finish(handle, ok, pid, "set the priority class of")
    }

    pub fn set_affinity(pid: u32, mask: u64) -> Result<bool, String> {
        let handle = open_process(pid, PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION)?;
        // SAFETY: handle is valid with PROCESS_SET_INFORMATION access; the
        // mask is truncated to the pointer width on 32-bit targets
        let ok = unsafe { SetProcessAffinityMask(handle, mask as usize) };
        finish(handle, ok, pid, "set the affinity mask of")
    }

    pub fn set_efficiency(pid: u32, enabled: bool) -> Result<bool, String> {
        let handle = open_process(pid, PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION)?;
        let mut throttling = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            // ControlMask selects the mechanism and StateMask declares
            // whether it is on or off (SetProcessInformation docs): on is
            // EcoQoS (efficiency mode), off is HighQoS
            ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            StateMask: if enabled {
                PROCESS_POWER_THROTTLING_EXECUTION_SPEED
            } else {
                0
            },
        };
        // SAFETY: throttling outlives the call and the size matches its type
        let ok = unsafe {
            SetProcessInformation(
                handle,
                PROCESS_POWER_THROTTLING_EXECUTION_SPEED as PROCESS_INFORMATION_CLASS,
                &mut throttling as *mut PROCESS_POWER_THROTTLING_STATE as *const c_void,
                std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
            )
        };
        let error = if ok == 0 {
            unsafe { GetLastError() }
        } else {
            0
        };
        unsafe { CloseHandle(handle) };
        if ok != 0 {
            Ok(true)
        } else if error == ERROR_INVALID_PARAMETER {
            Err(format!(
                "Failed to set efficiency mode for process {}: power throttling is not supported on this Windows version (requires Windows 10 1709 or later)",
                pid
            ))
        } else {
            Err(format!(
                "Failed to set efficiency mode for process {} (Windows error {})",
                pid, error
            ))
        }
    }

    pub fn is_elevated() -> bool {
        // SAFETY: GetCurrentProcess returns the constant pseudo handle which
        // stays valid; the opened token is owned by us and closed below
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
            let mut returned: u32 = 0;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                &mut elevation as *mut TOKEN_ELEVATION as *mut c_void,
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            );
            CloseHandle(token);
            ok != 0 && elevation.TokenIsElevated != 0
        }
    }

    pub fn restart_as_admin() -> Result<bool, String> {
        let exe = std::env::current_exe()
            .map_err(|e| format!("Failed to locate the current executable: {}", e))?;
        let verb = to_wide("runas");
        let file = to_wide(&exe.to_string_lossy());
        // SAFETY: info and the wide buffers outlive the call and are only
        // read by ShellExecuteExW; zeroed gives valid defaults for the
        // embedded union and unused fields
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        // fMask stays 0: SEE_MASK_NOCLOSEPROCESS would return a process
        // handle in info.hProcess that we would have to CloseHandle, and
        // we never use it
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.nShow = SW_SHOWNORMAL;
        // SAFETY: info is a valid, fully initialized SHELLEXECUTEINFOW
        let ok = unsafe { ShellExecuteExW(&mut info) };
        if ok == 0 {
            let error = unsafe { GetLastError() };
            if error == ERROR_CANCELLED {
                // The user declined the UAC prompt; nothing to report
                return Ok(false);
            }
            return Err(format!(
                "Failed to relaunch with administrator privileges (Windows error {})",
                error
            ));
        }
        // The elevated instance is launching; terminate this non-elevated
        // one. exit() diverges, so this is the function's tail expression.
        std::process::exit(0)
    }

    /// Encodes a string as a NUL-terminated UTF-16 buffer for Win32 APIs
    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(unix)]
mod platform {
    use super::ProcessPriorityInfo;

    // SIGSTOP/SIGCONT numbers are part of the stable platform ABI but differ
    // between OSes; std links libc so no extra crate is needed
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    const SIGSTOP: i32 = 17;
    #[cfg(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    const SIGCONT: i32 = 19;
    #[cfg(not(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    )))]
    const SIGSTOP: i32 = 19;
    #[cfg(not(any(
        target_os = "macos",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    )))]
    const SIGCONT: i32 = 18;

    extern "C" {
        fn kill(pid: i32, sig: i32) -> i32;
        fn geteuid() -> u32;
    }

    /// Sends a signal to the process, mapping EPERM to a message the
    /// frontend can recognize to offer a privileged relaunch
    fn signal_pid(pid: u32, sig: i32, action: &str) -> Result<bool, String> {
        // SAFETY: kill is signal-safe and only touches its arguments
        let result = unsafe { kill(pid as i32, sig) };
        if result == 0 {
            return Ok(true);
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(1) {
            // EPERM: the process belongs to another user or to the system
            return Err(format!(
                "Failed to {} process {}: permission denied (acting on processes you do not own requires elevated privileges)",
                action, pid
            ));
        }
        Err(format!("Failed to {} process {}: {}", action, pid, error))
    }

    pub fn suspend(pid: u32) -> Result<bool, String> {
        signal_pid(pid, SIGSTOP, "suspend")
    }

    pub fn resume(pid: u32) -> Result<bool, String> {
        signal_pid(pid, SIGCONT, "resume")
    }

    /// Priority classes, affinity masks and power throttling are Windows
    /// concepts without a cross-platform equivalent; the frontend keeps
    /// these controls Windows-only
    pub fn get_priority_info(_pid: u32) -> Result<ProcessPriorityInfo, String> {
        Err("Priority and affinity information is only available on Windows".to_string())
    }

    pub fn set_priority(_pid: u32, _class: &str) -> Result<bool, String> {
        Err("Setting the priority class is only supported on Windows".to_string())
    }

    pub fn set_affinity(_pid: u32, _mask: u64) -> Result<bool, String> {
        Err("Setting the CPU affinity is only supported on Windows".to_string())
    }

    pub fn set_efficiency(_pid: u32, _enabled: bool) -> Result<bool, String> {
        Err("Efficiency mode is only supported on Windows".to_string())
    }

    pub fn is_elevated() -> bool {
        // SAFETY: geteuid takes no arguments and cannot fail; running as
        // root is the closest equivalent of an elevated process on Unix
        unsafe { geteuid() == 0 }
    }

    pub fn restart_as_admin() -> Result<bool, String> {
        Err("Restarting with elevated privileges is not supported on this platform".to_string())
    }
}

#[cfg(not(any(windows, unix)))]
mod platform {
    use super::ProcessPriorityInfo;

    /// Process control is not implemented on this platform
    pub fn suspend(_pid: u32) -> Result<bool, String> {
        Err("Suspending processes is not supported on this platform".to_string())
    }

    /// Process control is not implemented on this platform
    pub fn resume(_pid: u32) -> Result<bool, String> {
        Err("Resuming processes is not supported on this platform".to_string())
    }

    /// Process control is not implemented on this platform
    pub fn get_priority_info(_pid: u32) -> Result<ProcessPriorityInfo, String> {
        Err("Priority information is not supported on this platform".to_string())
    }

    /// Process control is not implemented on this platform
    pub fn set_priority(_pid: u32, _class: &str) -> Result<bool, String> {
        Err("Setting the priority class is not supported on this platform".to_string())
    }

    /// Process control is not implemented on this platform
    pub fn set_affinity(_pid: u32, _mask: u64) -> Result<bool, String> {
        Err("Setting the CPU affinity is not supported on this platform".to_string())
    }

    /// Process control is not implemented on this platform
    pub fn set_efficiency(_pid: u32, _enabled: bool) -> Result<bool, String> {
        Err("Efficiency mode is not supported on this platform".to_string())
    }

    /// Process control is not implemented on this platform
    pub fn is_elevated() -> bool {
        false
    }

    /// Process control is not implemented on this platform
    pub fn restart_as_admin() -> Result<bool, String> {
        Err("Restarting with elevated privileges is not supported on this platform".to_string())
    }
}
