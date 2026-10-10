//! Process control operations
//!
//! Implements suspend/resume, priority class, CPU affinity and efficiency
//! mode (power throttling) control for a single process, whole-application
//! kills, plus elevation helpers the frontend uses to decide when to offer
//! relaunching the app with administrator privileges. State-changing
//! operations return `Ok(true)` on success so the frontend can track the
//! state it changed; the application kill reports its outcome as an
//! [`AppKillResult`] with the requested and killed counts.

use serde::Serialize;
use std::collections::{HashMap, HashSet};
// Re-exported by the parent module from process_monitor.rs; reused on
// non-Windows platforms where the kill falls back to the sysinfo kill
#[cfg(not(windows))]
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

/// Terminates a single process with the platform kill semantics
///
/// On Windows this opens the process with `PROCESS_TERMINATE` and calls
/// `TerminateProcess` with exit code 1 — the same convention Task Manager and
/// Process Explorer use: winlogon restarts interactive processes such as
/// explorer.exe when they exit with a non-1 status, so an off-convention
/// exit code makes the process look "resurrected" right after a kill.
/// Elsewhere the sysinfo kill signal (SIGKILL) applies.
pub fn terminate_process(sys: &sysinfo::System, pid: u32) -> bool {
    #[cfg(windows)]
    {
        let _ = sys;
        platform::terminate(pid)
    }
    #[cfg(not(windows))]
    {
        ProcessMonitor::kill_process(sys, pid)
    }
}

/// Outcome of an entire-application kill: what was requested and killed,
/// plus the new PIDs of the same application that started up *after* the
/// kill (a supervisor relaunched it)
#[derive(Serialize, Debug)]
pub struct AppKillResult {
    /// How many processes the scope resolution set out to kill
    pub requested: u32,
    /// How many of those actually died
    pub killed: u32,
    /// PIDs of the same application started after the kill — evidence a
    /// supervisor relaunched it. They are reported, not killed: Task
    /// Manager leaves respawns to the user too
    pub respawns: Vec<u32>,
}

/// One application kill must never target more than this many processes;
/// a bigger scope almost certainly means the family rule matched the
/// wrong thing, and refusing is cheaper than mass-killing
const MAX_APP_KILL_TARGETS: usize = 512;

/// Chromium-style data-directory identity: a process launched with
/// `--user-data-dir=<dir>` (WebView2 runtimes ALWAYS carry it; browsers
/// with a custom profile do too) belongs to the application owning that
/// data directory, NOT to everyone running the same runtime binary —
/// without it every host's msedgewebview2 processes would collapse into
/// one kill. The value comes from the parsed argv (immune to spaces in
/// the path) and is lowercased; empty when absent. Mirrors the frontend's
/// `identityKeyOf` so the confirmation-dialog count always equals the
/// kill set.
fn data_dir_identity_suffix(process: &sysinfo::Process) -> String {
    let Some(arg) = process
        .cmd()
        .iter()
        .find(|a| a.to_string_lossy().starts_with("--user-data-dir="))
    else {
        return String::new();
    };
    let value = arg.to_string_lossy();
    let value = value["--user-data-dir=".len()..].trim();
    if value.is_empty() {
        return String::new();
    }
    format!("|{}", value.to_lowercase())
}

/// Executable identity key of a process: the exe path when sysinfo could
/// read one (non-empty), else the process name, lowercased — plus the
/// Chromium data-directory segment when the command line carries one.
/// `exe()` must be used here — `root()` is the cwd's drive root on
/// Windows ("/" on Linux) and would collapse every same-drive process
/// onto one key. The frontend mirrors this exact key in `identityKeyOf`/
/// `buildAppRows`, so the confirmation-dialog count always equals the
/// backend kill set.
fn exe_identity_of(process: &sysinfo::Process) -> String {
    let exe = process
        .exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let base = if exe.trim().is_empty() {
        process.name().to_string_lossy().into_owned()
    } else {
        exe
    };
    format!(
        "{}{}",
        base.to_lowercase(),
        data_dir_identity_suffix(process)
    )
}

/// Refreshes the whole process table with the same fields the default
/// refresh covers PLUS the command line (`with_cmd` OnlyIfNotSet: read
/// once per process, never re-read): the app-family identity needs
/// `--user-data-dir=` from the command line of processes started after
/// the initial full refresh too.
pub fn refresh_process_table(sys: &mut sysinfo::System) {
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::All,
        true,
        sysinfo::ProcessRefreshKind::nothing()
            .with_memory()
            .with_cpu()
            .with_disk_usage()
            .with_exe(sysinfo::UpdateKind::OnlyIfNotSet)
            .with_cmd(sysinfo::UpdateKind::OnlyIfNotSet)
            .with_tasks(),
    );
}

/// Kills the entire application of `pid` — the "End Application" equivalent
///
/// The scope is the WHOLE snapshot scan: the target plus every process of
/// its session that shares its executable — exactly what the tree view's
/// "app" grouping shows in one "App (N)" row. Parentage is irrelevant: an
/// instance whose launcher died (or never kept a same-exe chain) is still
/// the same application to the user. Everything else is excluded on
/// purpose: any process running a *different* executable and any process
/// in another session, so the kill can never cross into other software.
///
/// The kill set dies in start-time order, so the supervising process dies
/// before the workers it would relaunch. After the kill the table is
/// refreshed once and PIDs of the application that started up in the
/// meantime are reported as respawns, not killed.
///
/// Takes a mutable `sys` because the respawn re-scan refreshes the process
/// table in place. Must be called while holding the caller's `sys` lock.
/// The respawn re-scan sleeps ~1.5 s, which stalls any refresh tick that
/// lands in that window.
pub fn kill_app_family(sys: &mut sysinfo::System, pid: u32) -> Result<AppKillResult, String> {
    if pid == 0 || (cfg!(windows) && pid <= 4) {
        return Err(format!(
            "Refusing to kill PID {}: kernel pseudo-process, not a user process",
            pid
        ));
    }
    let target = sys
        .process(sysinfo::Pid::from(pid as usize))
        .ok_or_else(|| format!("Process with PID {} not found", pid))?;

    // --- App scope: the target plus every same-executable session-mate ---
    //
    // The whole snapshot is scanned: every process of the target's session
    // that shares its executable joins the kill set, regardless of
    // parentage — a launcher-detached instance is still the same
    // application to the user. Anything running a *different* executable,
    // or sitting in another session, is excluded, so ending a shared
    // launcher's name-mates can never cross into other software. Unknown
    // session sides are kept — they cannot be judged.
    let target_exe_key = exe_identity_of(target);
    let target_session = target.session_id().map(|s| s.as_u32());

    let kill_set: HashSet<u32> = sys
        .processes()
        .iter()
        .filter(|(_p, process)| {
            if exe_identity_of(process) != target_exe_key {
                return false;
            }
            // Unknown session sides are kept — they cannot be judged
            match (target_session, process.session_id().map(|s| s.as_u32())) {
                (Some(t), Some(s)) => t == s,
                _ => true,
            }
        })
        .map(|(p, _)| p.as_u32())
        .collect();

    if kill_set.len() > MAX_APP_KILL_TARGETS {
        return Err(format!(
            "Application has {} processes, more than the {} allowed per kill",
            kill_set.len(),
            MAX_APP_KILL_TARGETS
        ));
    }

    // --- Kill in start-time order (supervisors die first) ---
    let mut start_time_of: HashMap<u32, u64> = HashMap::new();
    for (p, process) in sys.processes() {
        start_time_of.insert(p.as_u32(), process.start_time());
    }
    let mut kill_order: Vec<u32> = kill_set.iter().copied().collect();
    kill_order.sort_by_key(|p| start_time_of.get(p).copied().unwrap_or(u64::MAX));

    let mut killed: u32 = 0;
    for p in &kill_order {
        if terminate_process(sys, *p) {
            killed += 1;
        }
    }

    // --- Respawn re-scan: refresh and report the application's new PIDs ---
    // A supervisor relauncher (or the OS) that brings the application
    // back within the window shows up as the same executable started
    // after the kill. Those PIDs are reported, not chased — the user can
    // run the kill again, exactly as Task Manager leaves respawns to the
    // user. New PIDs were not in the pre-kill snapshot, so their
    // executable is read live from the refreshed table.
    let kill_time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    std::thread::sleep(std::time::Duration::from_millis(1500));
    refresh_process_table(sys);
    let mut respawns: Vec<u32> = sys
        .processes()
        .iter()
        .filter(|(child, process)| {
            let child_pid = child.as_u32();
            if kill_set.contains(&child_pid) || process.start_time() < kill_time {
                return false;
            }
            if let (Some(t), Some(s)) = (
                target_session,
                process.session_id().map(|id| id.as_u32()),
            ) {
                if t != s {
                    return false;
                }
            }
            exe_identity_of(process) == target_exe_key
        })
        .map(|(child, _)| child.as_u32())
        .collect();
    respawns.sort();

    Ok(AppKillResult {
        requested: kill_set.len() as u32,
        killed,
        respawns,
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
    report.killed = terminate_process(sys, pid);
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
        PROCESS_SUSPEND_RESUME, PROCESS_TERMINATE, REALTIME_PRIORITY_CLASS, SetPriorityClass,
        SetProcessAffinityMask, SetProcessInformation, TerminateProcess,
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

    /// Terminates a process with exit code 1
    ///
    /// The exit code is a compatibility convention, not a status: Task
    /// Manager and Process Explorer both terminate with 1, and winlogon
    /// restarts interactive processes such as explorer.exe when they exit
    /// with any other status — an off-convention code makes the process
    /// look resurrected immediately after the kill
    pub fn terminate(pid: u32) -> bool {
        let Ok(handle) = open_process(pid, PROCESS_TERMINATE) else {
            return false;
        };
        // SAFETY: handle is valid with PROCESS_TERMINATE access; exit code
        // 1 per the compatibility convention above
        let ok = unsafe { TerminateProcess(handle, 1) };
        unsafe { CloseHandle(handle) };
        ok != 0
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
