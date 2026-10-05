//! TCP connection control
//!
//! Closes an existing IPv4 TCP connection via `SetTcpEntry`, the only
//! documented user-mode API for resetting a connection without a handle to
//! the owning process. The row to delete is located in the owner-PID TCP
//! table first, so the caller must pass the exact local/remote endpoints of
//! a live connection together with the PID that owns it; this prevents
//! accidentally resetting a different connection (e.g. after a PID was
//! reused) and lets us copy the raw network-order address DWORDs straight
//! from the system table into the `SetTcpEntry` row.

/// Closes the IPv4 TCP connection with the given endpoints, owned by `pid`
///
/// The addresses are dotted-quad strings as reported by the network ports
/// command (e.g. "93.184.216.34"); listening sockets therefore pass
/// "0.0.0.0" and port 0 as the remote endpoint. Requires administrator
/// privileges on Windows; IPv6 connections are not supported because
/// `SetTcpEntry` has no IPv6 equivalent.
pub fn close(
    local_addr: &str,
    local_port: u16,
    remote_addr: &str,
    remote_port: u16,
    pid: u32,
) -> Result<bool, String> {
    platform::close(local_addr, local_port, remote_addr, remote_port, pid)
}

#[cfg(windows)]
mod platform {
    use crate::monitoring::process_control;
    use crate::monitoring::winbuf;
    use std::net::Ipv4Addr;
    use windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED;
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, MIB_TCPROW_LH, MIB_TCPROW_LH_0, MIB_TCPROW_OWNER_PID,
        MIB_TCPTABLE_OWNER_PID, MIB_TCP_STATE_DELETE_TCB, SetTcpEntry, TCP_TABLE_OWNER_PID_ALL,
    };
    use windows_sys::Win32::Networking::WinSock::AF_INET;

    pub fn close(
        local_addr: &str,
        local_port: u16,
        remote_addr: &str,
        remote_port: u16,
        pid: u32,
    ) -> Result<bool, String> {
        if !process_control::is_elevated() {
            return Err(
                "Closing a TCP connection requires administrator privileges; restart the app as administrator first"
                    .to_string(),
            );
        }
        let Ok(local) = local_addr.parse::<Ipv4Addr>() else {
            return Err(format!(
                "Invalid local address '{}': expected an IPv4 address",
                local_addr
            ));
        };
        let Ok(remote) = remote_addr.parse::<Ipv4Addr>() else {
            return Err(format!(
                "Invalid remote address '{}': expected an IPv4 address (IPv6 connections cannot be closed)",
                remote_addr
            ));
        };

        // Locate the live row so the raw network-order DWORDs handed to
        // SetTcpEntry are guaranteed to match what the stack currently has
        let table = query_tcp_table()?;
        let row =
            find_row(&table, local, local_port, remote, remote_port, pid).ok_or_else(|| {
                format!(
                    "No IPv4 TCP connection from {}:{} to {}:{} owned by process {} was found; it may have already closed",
                    local_addr, local_port, remote_addr, remote_port, pid
                )
            })?;

        let entry = MIB_TCPROW_LH {
            // Only DELETE_TCB may be requested through SetTcpEntry
            Anonymous: MIB_TCPROW_LH_0 {
                dwState: MIB_TCP_STATE_DELETE_TCB as u32,
            },
            dwLocalAddr: row.dwLocalAddr,
            dwLocalPort: row.dwLocalPort,
            dwRemoteAddr: row.dwRemoteAddr,
            dwRemotePort: row.dwRemotePort,
        };
        // SAFETY: entry only holds values copied out of the system table and
        // SetTcpEntry only reads the pointed-to row
        let result = unsafe { SetTcpEntry(&entry) };
        if result != 0 {
            // SetTcpEntry returns its error code directly instead of
            // reporting it through GetLastError
            return Err(match result {
                ERROR_ACCESS_DENIED => format!(
                    "Closing the connection {}:{} -> {}:{} requires administrator privileges (Windows error {})",
                    local_addr, local_port, remote_addr, remote_port, result
                ),
                _ => format!(
                    "Failed to close the connection {}:{} -> {}:{} (Windows error {})",
                    local_addr, local_port, remote_addr, remote_port, result
                ),
            });
        }
        Ok(true)
    }

    /// Fetches the IPv4 TCP table with owning PIDs as a raw DWORD buffer
    fn query_tcp_table() -> Result<Vec<u32>, String> {
        winbuf::query_growing_table("IPv4 TCP table", |table, size| unsafe {
            GetExtendedTcpTable(table, size, 0, AF_INET as u32, TCP_TABLE_OWNER_PID_ALL, 0)
        })
    }

    /// Finds the row matching all four endpoints and the owning PID
    fn find_row<'a>(
        buffer: &'a [u32],
        local: Ipv4Addr,
        local_port: u16,
        remote: Ipv4Addr,
        remote_port: u16,
        pid: u32,
    ) -> Option<&'a MIB_TCPROW_OWNER_PID> {
        if buffer.is_empty() {
            return None;
        }
        // Same encoding as the parse direction in network_ports.rs, inverted
        let local_addr = u32::from(local).to_be();
        let remote_addr = u32::from(remote).to_be();
        let local_port_dword = local_port.to_be() as u32;
        let remote_port_dword = remote_port.to_be() as u32;
        let table = buffer.as_ptr() as *const MIB_TCPTABLE_OWNER_PID;
        // SAFETY: the buffer is owned by us and GetExtendedTcpTable
        // guarantees it holds dwNumEntries MIB_TCPROW_OWNER_PID rows after
        // the header
        let (count, rows_ptr) = unsafe {
            (
                (*table).dwNumEntries as usize,
                std::ptr::addr_of!((*table).table) as *const MIB_TCPROW_OWNER_PID,
            )
        };
        let rows = unsafe { std::slice::from_raw_parts(rows_ptr, count) };
        rows.iter().find(|row| {
            row.dwOwningPid == pid
                && row.dwLocalAddr == local_addr
                && (row.dwLocalPort & 0xFFFF) == local_port_dword
                && row.dwRemoteAddr == remote_addr
                && (row.dwRemotePort & 0xFFFF) == remote_port_dword
        })
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn close(
        _local_addr: &str,
        _local_port: u16,
        _remote_addr: &str,
        _remote_port: u16,
        _pid: u32,
    ) -> Result<bool, String> {
        Err("Closing a TCP connection is only supported on Windows".to_string())
    }
}
