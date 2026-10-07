//! Top-level window listing
//!
//! Enumerates the session's top-level windows with `EnumWindows`, invisible
//! and minimized ones included, and reports the owning process of each
//! window. Windows without a title are skipped: they are mostly internal
//! message windows that would flood the list. On other platforms there is
//! no portable equivalent, so an error is reported.

use serde::Serialize;

/// A top-level window of the session
#[derive(Serialize, Debug)]
pub struct AppWindow {
    /// Window handle (HWND), stable while the window lives; the key used
    /// by [`show`]
    pub id: isize,
    /// Title bar text
    pub title: String,
    /// PID of the process that owns the window
    pub pid: u32,
    /// File name of the owning process' executable, empty when the process
    /// could not be queried (e.g. a protected process)
    pub process_name: String,
    /// Whether the window is currently visible (not hidden)
    pub is_visible: bool,
    /// Whether the window is currently minimized
    pub is_minimized: bool,
}

/// Returns the session's top-level windows, hidden and minimized included
///
/// On non-Windows platforms this always returns an error: the Win32 window
/// enumeration backing this command only exists on Windows.
pub fn collect() -> Result<Vec<AppWindow>, String> {
    platform::collect()
}

/// Restores, shows and focuses a window by its handle
///
/// A minimized window is restored, a hidden one is made visible, and the
/// window is brought to the foreground. Bringing to the foreground can
/// still be rejected by the system's foreground-lock rules; the window is
/// shown either way.
pub fn show(id: isize) -> Result<bool, String> {
    platform::show(id)
}

/// Shows, hides, minimizes or restores a window by handle. Plain
/// visibility/state switches — the foreground-stealing "bring to front"
/// variant lives in [`show`].
pub fn control(id: isize, action: &str) -> Result<bool, String> {
    platform::control(id, action)
}

#[cfg(windows)]
mod platform {
    use super::AppWindow;
    use windows_sys::Win32::Foundation::{BOOL, CloseHandle, GetLastError, HWND, LPARAM};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, GetWindowTextLengthW, GetWindowTextW, IsIconic,
        IsWindow, IsWindowVisible, SetForegroundWindow, ShowWindow, SW_HIDE, SW_MINIMIZE,
        SW_RESTORE, SW_SHOW,
    };

    /// State threaded through `EnumWindows`' LPARAM
    struct EnumState {
        handles: Vec<isize>,
    }

    /// Collects every titled top-level window handle. Windows without a
    /// title are internal message windows that would flood the list, so
    /// they never reach the UI.
    unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam as *mut EnumState);
        // SAFETY: hwnd is the window being visited by the enumeration
        let length = unsafe { GetWindowTextLengthW(hwnd) };
        if length > 0 {
            state.handles.push(hwnd as isize);
        }
        1
    }

    pub fn collect() -> Result<Vec<AppWindow>, String> {
        let mut state = EnumState {
            handles: Vec::new(),
        };
        // SAFETY: state is passed as an opaque LPARAM and only used inside
        // enum_proc, which merely pushes to the vector
        let ok = unsafe { EnumWindows(Some(enum_proc), &mut state as *mut _ as LPARAM) };
        if ok == 0 {
            let error = unsafe { GetLastError() };
            return Err(format!(
                "Failed to enumerate the windows (Windows error {})",
                error
            ));
        }

        let mut windows = Vec::with_capacity(state.handles.len());
        for id in state.handles {
            // SAFETY: id is a window handle handed out by EnumWindows
            let hwnd = id as HWND;
            // A window can be destroyed between the enumeration and this
            // query; such windows are skipped instead of failing the call
            if unsafe { IsWindow(hwnd) } == 0 {
                continue;
            }
            let title = read_title(hwnd);
            let (process_name, pid) = read_owner(hwnd);
            windows.push(AppWindow {
                id,
                title,
                pid,
                process_name,
                is_visible: unsafe { IsWindowVisible(hwnd) } != 0,
                is_minimized: unsafe { IsIconic(hwnd) } != 0,
            });
        }
        Ok(windows)
    }

    /// Reads the title text of a window; the enumeration only probed the
    /// length, so it is re-probed here to size the buffer
    fn read_title(hwnd: HWND) -> String {
        // SAFETY: hwnd is a live window handle (validated by IsWindow)
        let length = unsafe { GetWindowTextLengthW(hwnd) };
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        // SAFETY: buffer is a writable UTF-16 area of length + 1 units,
        // leaving room for the NUL the API appends
        let written = unsafe { GetWindowTextW(hwnd, buffer.as_mut_ptr(), length + 1) };
        if written <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buffer[..written as usize])
    }

    /// Returns the executable file name and PID of the process owning the
    /// window; both degrade gracefully for protected or gone processes
    fn read_owner(hwnd: HWND) -> (String, u32) {
        let mut pid: u32 = 0;
        // SAFETY: pid is a plain out-parameter
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == 0 {
            return (String::new(), 0);
        }
        // SAFETY: a limited query right is enough for the image path and
        // keeps working for elevated or protected owners
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
        if process.is_null() {
            return (String::new(), pid);
        }
        let mut buffer = [0u16; 1024];
        let mut size = buffer.len() as u32;
        // SAFETY: buffer is a writable UTF-16 area with its capacity given
        // in size, which the call replaces by the written length
        let ok = unsafe { QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut size) };
        // SAFETY: process is the valid handle from OpenProcess
        unsafe { CloseHandle(process) };
        if ok == 0 {
            return (String::new(), pid);
        }
        let full_path = String::from_utf16_lossy(&buffer[..size as usize]);
        let file_name = full_path
            .rsplit('\\')
            .next()
            .unwrap_or(&full_path)
            .to_string();
        (file_name, pid)
    }

    pub fn show(id: isize) -> Result<bool, String> {
        // SAFETY: id comes from a previous list_windows call; IsWindow
        // validates the handle before any further use
        let hwnd = id as HWND;
        if unsafe { IsWindow(hwnd) } == 0 {
            return Err(format!("No window with handle {} exists anymore", id));
        }
        // Restoring a minimized window also makes it visible, so the
        // plain SW_SHOW is only needed for hidden unminimized windows
        if unsafe { IsIconic(hwnd) } != 0 {
            // SAFETY: hwnd is a validated live window handle
            unsafe { ShowWindow(hwnd, SW_RESTORE) };
        } else if unsafe { IsWindowVisible(hwnd) } == 0 {
            // SAFETY: hwnd is a validated live window handle
            unsafe { ShowWindow(hwnd, SW_SHOW) };
        }
        // The system may refuse to steal the foreground from another app;
        // the window is shown regardless, so the result is not an error
        unsafe { SetForegroundWindow(hwnd) };
        Ok(true)
    }

    pub fn control(id: isize, action: &str) -> Result<bool, String> {
        let command = match action {
            "show" => SW_SHOW,
            "hide" => SW_HIDE,
            "minimize" => SW_MINIMIZE,
            "restore" => SW_RESTORE,
            _ => {
                return Err(format!(
                    "Unknown window action '{}': expected show, hide, minimize or restore",
                    action
                ))
            }
        };
        // SAFETY: id comes from a previous list_windows call; IsWindow
        // validates the handle before any further use
        let hwnd = id as HWND;
        if unsafe { IsWindow(hwnd) } == 0 {
            return Err(format!("No window with handle {} exists anymore", id));
        }
        // SAFETY: hwnd is a validated live window handle. A zero return
        // only means the window was already in the requested state
        unsafe { ShowWindow(hwnd, command) };
        Ok(true)
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn collect() -> Result<Vec<super::AppWindow>, String> {
        Err("Window enumeration is only supported on Windows".to_string())
    }

    pub fn show(_id: isize) -> Result<bool, String> {
        Err("Showing windows is only supported on Windows".to_string())
    }

    pub fn control(_id: isize, _action: &str) -> Result<bool, String> {
        Err("Controlling windows is only supported on Windows".to_string())
    }
}
