//! Network diagnostics
//!
//! Runs the platform's command-line ping for the ports panel's connection
//! diagnostics dialog. Arguments go straight to the OS (no shell), so the
//! target host cannot inject additional commands; ping's own per-probe
//! timeout bounds the total runtime to a few seconds.

/// Pings `host` (a name or address) a fixed number of times and returns
/// the raw command output for display. Empty or whitespace-bearing targets
/// are rejected before they reach the command line.
pub fn ping(host: &str) -> Result<String, String> {
    let host = host.trim();
    if host.is_empty() || host.chars().any(|c| c.is_whitespace()) {
        return Err("Invalid target host".to_string());
    }
    platform::ping(host)
}

#[cfg(windows)]
mod platform {
    use crate::monitoring::winbuf::decode_console_output;
    use std::os::windows::process::CommandExt;

    // Same flag the other console-tool spawns in this codebase use; not
    // exposed as a named windows-sys constant here to match container_ports
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn ping(host: &str) -> Result<String, String> {
        // Four probes with a 1.5s reply timeout each bound the run to a
        // few seconds even for fully unreachable hosts
        let output = std::process::Command::new("ping")
            .args(["-n", "4", "-w", "1500", host])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("Failed to run ping: {}", e))?;
        let text = decode_console_output(&output.stdout);
        if text.is_empty() {
            return Ok(String::from_utf8_lossy(&output.stderr).into_owned());
        }
        Ok(text)
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn ping(host: &str) -> Result<String, String> {
        let output = std::process::Command::new("ping")
            .args(["-c", "4", "-W", "2", host])
            .output()
            .map_err(|e| format!("Failed to run ping: {}", e))?;
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }
}
