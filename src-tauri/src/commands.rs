//! Tauri command handlers
//!
//! This module contains the command handlers that are exposed to the frontend
//! through Tauri's IPC mechanism. These commands provide the interface between
//! the frontend and the system monitoring functionality.

use crate::monitoring::{
    AppWindow, DriverInfo, FileLocker, KillTreeResult, ModuleInfo, PortConnection, PortProbe,
    ProcessInfo, ProcessMetadata, ProcessMonitor, ProcessPriorityInfo, ServiceInfo, StartupItem,
    SystemStats, TrafficCounters, collect_network_ports, file_lockers, listening_ports,
    network_ports, port_probe, process_control, process_inspection, services, startup_items,
    tcp_control, window_list,
};
use crate::state::AppState;
use tauri::State;

/// Retrieves the current list of processes and system statistics
///
/// # Arguments
///
/// * `state` - The application state containing system monitoring components
///
/// # Returns
///
/// A tuple containing:
/// * A vector of process information
/// * Current system statistics
///
/// # Errors
///
/// Returns an error string if:
/// * Failed to acquire locks on system state
/// * Failed to collect process information
#[tauri::command]
pub async fn get_processes(
    state: State<'_, AppState>,
) -> Result<(Vec<ProcessInfo>, SystemStats), String> {
    let mut sys = state.sys.lock().map_err(|e| e.to_string())?;
    let mut disks = state.disks.lock().map_err(|e| e.to_string())?;
    let mut networks = state.networks.lock().map_err(|e| e.to_string())?;
    sys.refresh_all();
    disks.refresh(true);
    networks.refresh(true);

    let mut process_monitor = state.process_monitor.lock().map_err(|e| e.to_string())?;
    let mut system_monitor = state.system_monitor.lock().map_err(|e| e.to_string())?;

    let processes = process_monitor.collect_processes(&sys)?;
    let system_stats = system_monitor.collect_stats(&sys, &networks, &disks);

    Ok((processes, system_stats))
}

/// Attempts to kill a process with the specified PID
///
/// # Arguments
///
/// * `pid` - Process ID to kill
/// * `state` - The application state
///
/// # Returns
///
/// * `true` if the process was successfully killed
/// * `false` if the process couldn't be killed or wasn't found
///
/// # Errors
///
/// Returns an error string if failed to acquire lock on system state
#[tauri::command]
pub async fn kill_process(pid: u32, state: State<'_, AppState>) -> Result<bool, String> {
    let sys = state.sys.lock().map_err(|e| e.to_string())?;
    Ok(ProcessMonitor::kill_process(&sys, pid))
}

/// Attempts to kill an entire process tree rooted at the specified PID
///
/// Collects every descendant of the target from the parent PIDs of the
/// process snapshot, refreshed first so children forked since the last
/// polling tick are included, and kills the tree in reverse discovery
/// order: descendants first, the target last, so a dying parent cannot
/// re-parent its children mid-run.
///
/// # Arguments
///
/// * `pid` - Process ID of the tree root to kill
/// * `state` - The application state
///
/// # Returns
///
/// A [`KillTreeResult`] with the number of processes the traversal asked to
/// kill (descendants plus the target) and how many were actually killed
///
/// # Errors
///
/// Returns an error string if failed to acquire lock on system state or the
/// target process wasn't found in the current snapshot
#[tauri::command]
pub async fn kill_process_tree(
    pid: u32,
    state: State<'_, AppState>,
) -> Result<KillTreeResult, String> {
    let mut sys = state.sys.lock().map_err(|e| e.to_string())?;
    // Refresh up front so the traversal sees children forked since the
    // last polling tick, and a target that already exited is reported as
    // missing instead of being killed through a stale snapshot entry
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    process_control::kill_tree(&sys, pid)
}

/// Attempts to restart a process with the specified PID
///
/// Determines the executable path of the process, kills it, waits for it to
/// exit, and then spawns a new instance of the same executable.
///
/// # Arguments
///
/// * `pid` - Process ID to restart
/// * `state` - The application state
///
/// # Returns
///
/// * `true` if the process was successfully restarted
///
/// # Errors
///
/// Returns an error string if:
/// * Failed to acquire lock on system state
/// * The process wasn't found or its executable path couldn't be determined
/// * The process couldn't be killed or didn't exit in time
/// * The new process couldn't be spawned
#[tauri::command]
pub async fn restart_process(pid: u32, state: State<'_, AppState>) -> Result<bool, String> {
    // Grab the executable path first, while the process is still alive
    let (exe_path, start_time) = {
        let sys = state.sys.lock().map_err(|e| e.to_string())?;
        let process = sys
            .process(sysinfo::Pid::from(pid as usize))
            .ok_or_else(|| format!("Process with PID {} not found", pid))?;
        // On Linux and Windows, a failed exe lookup can come back as an empty
        // path instead of None, so treat an empty path as unknown too
        let exe_path = process
            .exe()
            .filter(|path| !path.as_os_str().is_empty())
            .map(|path| path.to_path_buf())
            .ok_or_else(|| {
                format!("Cannot determine executable path for process with PID {}", pid)
            })?;
        (exe_path, process.start_time())
    };

    {
        let sys = state.sys.lock().map_err(|e| e.to_string())?;
        if !ProcessMonitor::kill_process(&sys, pid) {
            return Err(format!("Failed to kill process with PID {}", pid));
        }
    }

    // Poll until the process has fully exited before restarting it. The poll
    // blocks, so run it on a blocking thread to keep the async runtime free.
    // A killed process can linger as a zombie (Linux/macOS) until its parent
    // reaps it, and its PID can get reused meanwhile, so also treat a zombie
    // or a changed start time as exited.
    //
    // start_time only has 1-second resolution, so a PID reused within the
    // same epoch second (only possible if the killed process lived under a
    // second) is misread as still alive, failing the restart with a false
    // "didn't exit" error. Fail-safe (no wrong restart), just rare.
    let target_pid = sysinfo::Pid::from(pid as usize);
    let exited = tauri::async_runtime::spawn_blocking(move || {
        let mut confirmation_sys = sysinfo::System::new();
        for _ in 0..50 {
            confirmation_sys
                .refresh_processes(sysinfo::ProcessesToUpdate::Some(&[target_pid]), true);
            match confirmation_sys.process(target_pid) {
                None => return true,
                // The PID was reused by a different process, or the process
                // is a zombie waiting to be reaped by its parent
                Some(process) => {
                    if process.start_time() != start_time
                        || process.status() == sysinfo::ProcessStatus::Zombie
                    {
                        return true;
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        false
    })
    .await
    .map_err(|e| format!("Failed to wait for process {} to exit: {}", pid, e))?;

    if !exited {
        return Err(format!("Process with PID {} didn't exit after being killed", pid));
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP: spawn the process
        // without flashing a console window
        std::process::Command::new(&exe_path)
            .creation_flags(0x08000000 | 0x00000200)
            .spawn()
            .map_err(|e| format!("Failed to restart process: {}", e))?;
    }

    #[cfg(not(windows))]
    {
        std::process::Command::new(&exe_path)
            .spawn()
            .map_err(|e| format!("Failed to restart process: {}", e))?;
    }

    Ok(true)
}

/// Retrieves the network connections (TCP/UDP) currently owned by processes
///
/// The collection is stateless and runs on a blocking thread so the async
/// runtime and the app state locks stay free while it works.
///
/// # Returns
///
/// A vector of port connections, sorted by protocol, local port and PID
///
/// # Errors
///
/// Returns an error string if:
/// * The blocking collection task failed
/// * The underlying platform query failed
#[tauri::command]
pub async fn get_network_ports() -> Result<Vec<PortConnection>, String> {
    tauri::async_runtime::spawn_blocking(collect_network_ports)
        .await
        .map_err(|e| format!("Failed to collect network ports: {}", e))?
}

/// Lists the local TCP ports that currently have a process listening
///
/// Lightweight companion of `get_network_ports` for the port-watch
/// feature: the frontend polls only port numbers, not whole connection
/// rows. Collection runs on a blocking thread like the full snapshot.
///
/// # Errors
///
/// Returns an error string if the blocking collection task failed
#[tauri::command]
pub async fn get_listening_ports() -> Result<Vec<u16>, String> {
    // listening_ports returns a plain Vec (unlike collect's Result), so the
    // join of the blocking result and the JoinError mapping ends here
    Ok(tauri::async_runtime::spawn_blocking(listening_ports)
        .await
        .map_err(|e| format!("Failed to collect listening ports: {}", e))?)
}

/// Probes one TCP listener to find out what protocol it speaks
///
/// Runs the on-demand probe battery (TLS / SOCKS5 / HTTP / clash-style
/// version endpoint / DNS) against a single listener on a blocking
/// thread. The frontend resolves wildcard binds to 127.0.0.1 first.
///
/// # Errors
///
/// Returns an error string if the host is not an IP literal or the
/// blocking task failed; silent listeners are reported as all-false
/// findings, not errors.
#[tauri::command]
pub async fn identify_port(host: String, port: u16) -> Result<PortProbe, String> {
    // Module path required: the command's own name shadows the re-exported
    // function inside this body (same collision process_control has)
    tauri::async_runtime::spawn_blocking(move || port_probe::identify_port(&host, port))
        .await
        .map_err(|e| format!("Failed to probe port: {}", e))?
}

/// Suspends all threads of the process with the specified PID
///
/// On Windows this uses ntdll's `NtSuspendProcess` with a handle opened for
/// `PROCESS_SUSPEND_RESUME`; on macOS/Linux it sends `SIGSTOP` via `kill(2)`.
///
/// # Arguments
///
/// * `pid` - Process ID to suspend
///
/// # Returns
///
/// * `true` if the process was successfully suspended. The frontend should
///   track this flag itself: on Windows sysinfo always reports processes as
///   running, so the process list cannot show suspension by itself, while on
///   macOS/Linux the process status becomes "Stopped" after a refresh
///
/// # Errors
///
/// Returns an error string if the process couldn't be opened (e.g. a
/// protected or system process, which requires elevated privileges) or the
/// platform call failed
#[tauri::command]
pub async fn suspend_process(pid: u32) -> Result<bool, String> {
    // NtSuspendProcess can block on processes with many threads, so keep
    // it off the async runtime like the other blocking commands
    tauri::async_runtime::spawn_blocking(move || process_control::suspend(pid))
        .await
        .map_err(|e| format!("Failed to suspend process {}: {}", pid, e))?
}

/// Resumes a process that was suspended with [`suspend_process`]
///
/// On Windows this uses ntdll's `NtResumeProcess` with a handle opened for
/// `PROCESS_SUSPEND_RESUME`; on macOS/Linux it sends `SIGCONT` via `kill(2)`.
///
/// # Arguments
///
/// * `pid` - Process ID to resume
///
/// # Returns
///
/// * `true` if the process was successfully resumed
///
/// # Errors
///
/// Returns an error string if the process couldn't be opened (e.g. a
/// protected or system process, which requires elevated privileges) or the
/// platform call failed
#[tauri::command]
pub async fn resume_process(pid: u32) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || process_control::resume(pid))
        .await
        .map_err(|e| format!("Failed to resume process {}: {}", pid, e))?
}

/// Retrieves the priority class, affinity masks and efficiency mode of a
/// process (Windows only)
///
/// # Arguments
///
/// * `pid` - Process ID to inspect
///
/// # Returns
///
/// A [`ProcessPriorityInfo`] with the priority class name, the process and
/// system affinity masks, and whether efficiency mode (power throttling) is
/// currently enabled
///
/// # Errors
///
/// Returns an error string if the process couldn't be opened (e.g. a
/// protected or system process, which requires elevated privileges) or a
/// query failed
#[tauri::command]
pub async fn get_process_priority_info(pid: u32) -> Result<ProcessPriorityInfo, String> {
    process_control::get_priority_info(pid)
}

/// Sets the priority class of a process (Windows only)
///
/// # Arguments
///
/// * `pid` - Process ID to modify
/// * `class` - One of "idle", "below_normal", "normal", "above_normal",
///   "high" or "realtime", applied via `SetPriorityClass`
///
/// # Returns
///
/// * `true` if the priority class was applied
///
/// # Errors
///
/// Returns an error string if the class name is unknown, the process
/// couldn't be opened (system processes require elevated privileges) or the
/// call failed
#[tauri::command]
pub async fn set_process_priority(pid: u32, class: String) -> Result<bool, String> {
    process_control::set_priority(pid, &class)
}

/// Sets the CPU affinity mask of a process (Windows only)
///
/// # Arguments
///
/// * `pid` - Process ID to modify
/// * `mask` - Bitfield of CPUs the process may run on as a decimal string,
///   applied via `SetProcessAffinityMask` (truncated to the pointer width
///   on 32-bit targets)
///
/// # Returns
///
/// * `true` if the affinity mask was applied
///
/// # Errors
///
/// Returns an error string if the mask is not a valid decimal number, the
/// process couldn't be opened (system processes require elevated
/// privileges) or the call failed
#[tauri::command]
pub async fn set_process_affinity(pid: u32, mask: String) -> Result<bool, String> {
    // The mask travels as a string because 64-bit values lose precision
    // as JSON numbers above 2^53
    let mask = mask
        .parse::<u64>()
        .map_err(|_| format!("Invalid affinity mask '{}'", mask))?;
    process_control::set_affinity(pid, mask)
}

/// Enables or disables efficiency mode (power throttling) for a process
/// (Windows 10 1709+)
///
/// # Arguments
///
/// * `pid` - Process ID to modify
/// * `enabled` - `true` to throttle the process (`SetProcessInformation`
///   with `ProcessPowerThrottling`), `false` to restore normal speed
///
/// # Returns
///
/// * `true` if the state was applied
///
/// # Errors
///
/// Returns an error string if the Windows version doesn't support power
/// throttling (pre-1709), the process couldn't be opened (system processes
/// require elevated privileges) or the call failed
#[tauri::command]
pub async fn set_process_efficiency(pid: u32, enabled: bool) -> Result<bool, String> {
    process_control::set_efficiency(pid, enabled)
}

/// Returns whether the app itself runs with elevated privileges
///
/// On Windows this checks the process token elevation
/// (`OpenProcessToken` + `GetTokenInformation(TokenElevation)`); on Unix it
/// checks for an effective user ID of 0. Never fails: a token query error
/// is reported as not elevated.
#[tauri::command]
pub async fn is_elevated() -> bool {
    process_control::is_elevated()
}

/// Relaunches the app with administrator privileges
///
/// Triggers the UAC prompt via `ShellExecuteExW` with the "runas" verb and
/// the current executable, then terminates this non-elevated instance.
///
/// # Returns
///
/// * `true` if the elevated instance was launched (the process then exits)
/// * `false` if the user declined the UAC prompt
///
/// # Errors
///
/// Returns an error string if the current executable couldn't be located or
/// the relaunch failed
///
/// The call blocks while the UAC prompt is open, so it runs on a blocking
/// thread to keep the async runtime free
#[tauri::command]
pub async fn restart_as_admin() -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(process_control::restart_as_admin)
        .await
        .map_err(|e| format!("Failed to relaunch as administrator: {}", e))?
}

/// Closes an existing IPv4 TCP connection (Windows only)
///
/// The connection is located in the owner-PID TCP table by all four
/// endpoints and the owning PID, then reset through `SetTcpEntry` with the
/// `MIB_TCP_STATE_DELETE_TCB` state. IPv6 connections cannot be closed
/// this way and are rejected.
///
/// # Arguments
///
/// * `local_addr` - Local IPv4 address of the connection (dotted quad)
/// * `local_port` - Local port
/// * `remote_addr` - Remote IPv4 address ("0.0.0.0" for listening sockets)
/// * `remote_port` - Remote port (0 for listening sockets)
/// * `pid` - PID of the process owning the connection
///
/// # Returns
///
/// * `true` if the connection was reset
///
/// # Errors
///
/// Returns an error string if the app is not elevated, an address is not a
/// valid IPv4 address, no matching connection exists (it may have already
/// closed) or the `SetTcpEntry` call failed
#[tauri::command]
pub async fn close_tcp_connection(
    local_addr: String,
    local_port: u16,
    remote_addr: String,
    remote_port: u16,
    pid: u32,
) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || {
        tcp_control::close(&local_addr, local_port, &remote_addr, remote_port, pid)
    })
    .await
    .map_err(|e| format!("Failed to close the TCP connection: {}", e))?
}

/// Finds the processes currently using a file (Windows Restart Manager)
///
/// # Arguments
///
/// * `path` - Absolute path of the file to inspect
///
/// # Returns
///
/// The list of processes holding the file open, with their PID, friendly
/// application name and service short name. Always empty on non-Windows
/// platforms, where the Restart Manager does not exist.
///
/// # Errors
///
/// Returns an error string if the path is empty, the file could not be
/// registered (e.g. it does not exist) or a Restart Manager call failed
#[tauri::command]
pub async fn get_file_lockers(path: String) -> Result<Vec<FileLocker>, String> {
    tauri::async_runtime::spawn_blocking(move || file_lockers::collect(&path))
        .await
        .map_err(|e| format!("Failed to find the processes using the file: {}", e))?
}

/// Lists all services known to the Service Control Manager (Windows only)
///
/// Both win32 services and drivers are reported, active or not. Several
/// services may share one hosting process (svchost), so the returned PID
/// is not unique and can be grouped to split shared hosts.
///
/// # Returns
///
/// One entry per service with its internal name, display name, state,
/// start type and hosting PID
///
/// # Errors
///
/// Returns an error string if a Service Control Manager call failed
#[tauri::command]
pub async fn list_services() -> Result<Vec<ServiceInfo>, String> {
    tauri::async_runtime::spawn_blocking(services::collect)
        .await
        .map_err(|e| format!("Failed to list services: {}", e))?
}

/// Starts, stops, pauses or resumes a service (Windows only)
///
/// Before sending the control the command verifies the service accepts it
/// in its current state (`QueryServiceStatusEx`, `dwControlsAccepted`).
/// The state change completes asynchronously afterwards.
///
/// # Arguments
///
/// * `name` - Internal service name, as returned by [`list_services`]
/// * `action` - One of "start", "stop", "pause" or "continue"
///
/// # Returns
///
/// * `true` if the control was accepted by the Service Control Manager
///
/// # Errors
///
/// Returns an error string if the app is not elevated, the action name is
/// unknown, the service doesn't exist or rejects the control, or an SCM
/// call failed
#[tauri::command]
pub async fn control_service(name: String, action: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || services::control(&name, &action))
        .await
        .map_err(|e| format!("Failed to control the service: {}", e))?
}

/// Lists the session's top-level windows (Windows only)
///
/// Titled windows are reported with their title, owning process, visibility
/// and minimized state; hidden and minimized windows are included. The
/// window handle doubles as the stable identifier for [`show_window`].
///
/// # Returns
///
/// One entry per titled top-level window, in enumeration order
///
/// # Errors
///
/// Returns an error string if an `EnumWindows` call failed
#[tauri::command]
pub async fn list_windows() -> Result<Vec<AppWindow>, String> {
    tauri::async_runtime::spawn_blocking(window_list::collect)
        .await
        .map_err(|e| format!("Failed to list windows: {}", e))?
}

/// Restores, shows and focuses a window by its handle (Windows only)
///
/// # Arguments
///
/// * `id` - Window handle, as returned by [`list_windows`]
///
/// # Returns
///
/// * `true` if the window was shown
///
/// # Errors
///
/// Returns an error string if the window does not exist anymore or a
/// `ShowWindow` call failed
#[tauri::command]
pub async fn show_window(id: isize) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || window_list::show(id))
        .await
        .map_err(|e| format!("Failed to show the window: {}", e))?
}

/// Reads the metadata (company, description, version, elevation) of a process
///
/// The data comes from the version resource of the executable plus its
/// process token; empty strings mean the information is missing.
///
/// # Arguments
///
/// * `pid` - Process ID to inspect
///
/// # Returns
///
/// A [`ProcessMetadata`] with the executable path, company name, file
/// description, file version and whether the process runs elevated
///
/// # Errors
///
/// Returns an error string if the process could not be opened (e.g. a
/// protected process, which requires elevated privileges)
#[tauri::command]
pub async fn get_process_metadata(pid: u32) -> Result<ProcessMetadata, String> {
    tauri::async_runtime::spawn_blocking(move || process_inspection::get_metadata(pid))
        .await
        .map_err(|e| format!("Failed to read the process metadata: {}", e))?
}

/// Lists the modules (DLLs) loaded by a process (Windows only)
///
/// # Arguments
///
/// * `pid` - Process ID to inspect
///
/// # Returns
///
/// One entry per loaded module with its file name, full path, mapped image
/// size and base address
///
/// # Errors
///
/// Returns an error string if the process could not be opened (system and
/// protected processes require elevated privileges) or a psapi call failed
#[tauri::command]
pub async fn list_process_modules(pid: u32) -> Result<Vec<ModuleInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || process_inspection::list_modules(pid))
        .await
        .map_err(|e| format!("Failed to list the process modules: {}", e))?
}

/// Lists the kernel-mode drivers loaded on the system (Windows only)
///
/// # Returns
///
/// One entry per loaded driver with its image name, full path and base
/// address
///
/// # Errors
///
/// Returns an error string if a psapi call failed
#[tauri::command]
pub async fn list_drivers() -> Result<Vec<DriverInfo>, String> {
    tauri::async_runtime::spawn_blocking(process_inspection::list_drivers)
        .await
        .map_err(|e| format!("Failed to list the drivers: {}", e))?
}

/// Lists the autostart entries: registry Run keys, Startup folders and
/// non-Microsoft scheduled tasks
///
/// # Returns
///
/// One entry per autostart item with its kind, command, location and
/// enabled state
///
/// # Errors
///
/// Only fails if the blocking task itself failed; enumeration problems
/// are reported by missing entries instead
#[tauri::command]
pub async fn list_startup_items() -> Result<Vec<StartupItem>, String> {
    tauri::async_runtime::spawn_blocking(startup_items::list)
        .await
        .map_err(|e| format!("Failed to list startup items: {}", e))
}

/// Enables or disables one autostart item by id
///
/// Registry and Startup-folder items are flagged through Task Manager's
/// StartupApproved convention; scheduled tasks go through schtasks
///
/// # Errors
///
/// Returns an error string when the owning key/file/task is gone or the
/// change could not be written (e.g. missing elevation for HKLM/tasks)
#[tauri::command]
pub async fn set_startup_item_enabled(id: String, enabled: bool) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || startup_items::set_enabled(&id, enabled))
        .await
        .map_err(|e| format!("Failed to change the startup item: {}", e))?
}

/// Permanently removes one autostart item (registry value, file or task)
///
/// # Errors
///
/// Returns an error string when the item is gone or the deletion failed
/// (missing elevation for HKLM values and tasks)
#[tauri::command]
pub async fn delete_startup_item(id: String) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || startup_items::delete(&id))
        .await
        .map_err(|e| format!("Failed to delete the startup item: {}", e))?
}

/// Fast traffic tick for the ports modal: reads the cumulative ESTATS
/// counters of exactly the requested connection keys from the last
/// snapshot's row map (no table re-enumeration)
///
/// # Arguments
///
/// * `keys` - Frontend connection keys ("TCP|local|port|remote|port|pid")
///
/// # Returns
///
/// A map from requested key to counters; untracked keys report zeros
///
/// # Errors
///
/// Only fails if the blocking task itself failed; lookup problems are
/// reported as zero counters instead
#[tauri::command]
pub async fn get_traffic_counters(
    keys: Vec<String>,
) -> Result<std::collections::HashMap<String, TrafficCounters>, String> {
    tauri::async_runtime::spawn_blocking(move || network_ports::traffic_counters(&keys))
        .await
        .map_err(|e| format!("Failed to read traffic counters: {}", e))
}

/// Reads the mandatory integrity level of one process ("low", "medium",
/// "high", "system", "protected", "untrusted"); "unknown" on other
/// platforms or when the token cannot be opened
///
/// # Errors
///
/// Returns an error string when the process is gone or the token query
/// failed
#[tauri::command]
pub async fn get_process_integrity(pid: u32) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || process_inspection::get_process_integrity(pid))
        .await
        .map_err(|e| format!("Failed to read the integrity level: {}", e))?
}
