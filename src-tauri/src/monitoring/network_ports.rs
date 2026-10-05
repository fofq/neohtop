//! Network port monitoring
//!
//! Collects the network connections (TCP/UDP) currently owned by running
//! processes, similar to System Informer's per-process connections view.
//! Each entry reports the protocol, local/remote endpoints, TCP state and
//! the PID of the owning process.

use serde::Serialize;
use std::collections::HashMap;

/// A single network connection owned by a process
#[derive(Serialize, Debug)]
pub struct PortConnection {
    /// Protocol: "TCP" or "UDP"
    pub protocol: String,
    /// Local IP address
    pub local_addr: String,
    /// Local port
    pub local_port: u16,
    /// Remote IP address (empty for UDP endpoints)
    pub remote_addr: String,
    /// Remote port (0 for UDP endpoints)
    pub remote_port: u16,
    /// TCP state (e.g. "LISTEN", "ESTABLISHED"); "-" for UDP
    pub state: String,
    /// PID of the owning process (0 when unavailable)
    pub pid: u32,
    /// Cumulative bytes sent over this TCP connection (0 for UDP or when
    /// the platform cannot report per-connection traffic)
    pub bytes_sent: u64,
    /// Cumulative bytes received over this TCP connection
    pub bytes_received: u64,
}

/// Collects all current connections, sorted by protocol, port and PID
pub fn collect() -> Result<Vec<PortConnection>, String> {
    let mut connections = platform::collect()?;
    connections.sort_by(|a, b| {
        a.protocol
            .cmp(&b.protocol)
            .then(a.local_port.cmp(&b.local_port))
            .then(a.pid.cmp(&b.pid))
    });
    Ok(connections)
}

/// Distinct local TCP ports that have a LISTEN row in a fresh snapshot,
/// sorted ascending. Runs its own table query so the frontend's port
/// watchers stay current even while the ports modal (the usual poller of
/// `collect`) is closed.
pub fn listening_ports() -> Vec<u16> {
    let mut ports: Vec<u16> = collect()
        .unwrap_or_default()
        .into_iter()
        .filter(|connection| connection.state == "LISTEN")
        .map(|connection| connection.local_port)
        .collect();
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// Cumulative per-connection counters for the fast traffic ticker
#[derive(Serialize, Debug, Clone)]
pub struct TrafficCounters {
    pub sent: u64,
    pub received: u64,
}

/// Reads counters for exactly the requested connection keys (see the
/// windows platform for the lookup details); untracked keys report zeros
pub fn traffic_counters(keys: &[String]) -> HashMap<String, TrafficCounters> {
    #[cfg(windows)]
    {
        return platform::traffic_counters(keys);
    }
    #[cfg(not(windows))]
    {
        keys.iter()
            .map(|key| {
                (
                    key.clone(),
                    TrafficCounters {
                        sent: 0,
                        received: 0,
                    },
                )
            })
            .collect()
    }
}

#[cfg(windows)]
mod platform {
    use super::{PortConnection, TrafficCounters};
    use crate::monitoring::winbuf;
    use std::net::{Ipv4Addr, Ipv6Addr};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, GetExtendedUdpTable, GetPerTcpConnectionEStats, MIB_TCP6ROW_OWNER_PID,
        MIB_TCP6TABLE_OWNER_PID, MIB_TCPROW_LH, MIB_TCPROW_OWNER_PID, MIB_TCPTABLE_OWNER_PID,
        MIB_TCP_STATE_CLOSE_WAIT, MIB_TCP_STATE_CLOSED, MIB_TCP_STATE_CLOSING,
        MIB_TCP_STATE_DELETE_TCB, MIB_TCP_STATE_ESTAB, MIB_TCP_STATE_FIN_WAIT1,
        MIB_TCP_STATE_FIN_WAIT2, MIB_TCP_STATE_LAST_ACK, MIB_TCP_STATE_LISTEN,
        MIB_TCP_STATE_SYN_RCVD, MIB_TCP_STATE_SYN_SENT, MIB_TCP_STATE_TIME_WAIT,
        MIB_UDP6ROW_OWNER_PID, MIB_UDP6TABLE_OWNER_PID, MIB_UDPROW_OWNER_PID,
        MIB_UDPTABLE_OWNER_PID, SetPerTcpConnectionEStats, TCP_ESTATS_DATA_ROD_v0,
        TCP_ESTATS_DATA_RW_v0, TcpConnectionEstatsData, TCP_TABLE_OWNER_PID_ALL,
        UDP_TABLE_OWNER_PID,
    };
    use windows_sys::Win32::Foundation::BOOLEAN;
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
    use std::collections::{HashMap, HashSet};
    use std::sync::{Mutex, OnceLock};

    /// Connections whose ESTATS data collection has been ENABLED
    /// successfully, keyed by the address/port 4-tuple (without state, so
    /// a SYN_SENT→ESTABLISHED transition does not re-enable and reset the
    /// counters). SetPerTcpConnectionEStats requires elevation (error 5
    /// without it) — a row whose Set failed must never appear here, so its
    /// counters are reported as zero instead of the meaningless values the
    /// OS hands back for un-collected connections.
    fn watched_rows() -> &'static Mutex<HashSet<[u32; 4]>> {
        static WATCHED: OnceLock<Mutex<HashSet<[u32; 4]>>> = OnceLock::new();
        WATCHED.get_or_init(|| Mutex::new(HashSet::new()))
    }

    /// Latest snapshot's rows keyed by the SAME string the frontend uses
    /// for `connectionKey` ("TCP|local|port|remote|port|pid"), so the fast
    /// traffic ticker can ask for counters of exactly the rows it shows
    /// without re-enumerating the whole table.
    fn known_rows() -> &'static Mutex<HashMap<String, [u32; 5]>> {
        static KNOWN: OnceLock<Mutex<HashMap<String, [u32; 5]>>> = OnceLock::new();
        KNOWN.get_or_init(|| Mutex::new(HashMap::new()))
    }

    /// Maps a MIB_TCP_STATE value to its official name. `dwState` is a u32
    /// while the MIB_TCP_STATE_* constants are typed i32, hence the cast.
    fn tcp_state_name(state: u32) -> &'static str {
        match state as i32 {
            MIB_TCP_STATE_CLOSED => "CLOSED",
            MIB_TCP_STATE_LISTEN => "LISTEN",
            MIB_TCP_STATE_SYN_SENT => "SYN_SENT",
            MIB_TCP_STATE_SYN_RCVD => "SYN_RCVD",
            MIB_TCP_STATE_ESTAB => "ESTABLISHED",
            MIB_TCP_STATE_FIN_WAIT1 => "FIN_WAIT1",
            MIB_TCP_STATE_FIN_WAIT2 => "FIN_WAIT2",
            MIB_TCP_STATE_CLOSE_WAIT => "CLOSE_WAIT",
            MIB_TCP_STATE_CLOSING => "CLOSING",
            MIB_TCP_STATE_LAST_ACK => "LAST_ACK",
            MIB_TCP_STATE_TIME_WAIT => "TIME_WAIT",
            MIB_TCP_STATE_DELETE_TCB => "DELETE_TCB",
            _ => "UNKNOWN",
        }
    }

    /// Converts a DWORD holding an in_addr in network byte order into an
    /// IPv4 address. On a little-endian machine the DWORD read reverses the
    /// octets, so convert back with to_be() before building the address.
    fn ipv4_from_dword(value: u32) -> Ipv4Addr {
        Ipv4Addr::from(value.to_be())
    }

    /// Cumulative bytes sent/received for one IPv4 TCP connection row via
    /// the per-connection ESTATS API (0 when unavailable, e.g. listeners).
    ///
    /// SAFETY: `row` lays out the five DWORDs of MIB_TCPROW_LH exactly as
    /// the header defines them (state union first, network-order ports);
    /// the pointer cast is layout-identical for the duration of the call.
    fn tcp_row_traffic(row: &[u32; 5]) -> (u64, u64) {
        if row[0] == MIB_TCP_STATE_LISTEN as u32 {
            return (0, 0);
        }
        // SAFETY: all-zeroed POD buffer sized for exactly one ROD struct
        let mut rod: TCP_ESTATS_DATA_ROD_v0 = unsafe { std::mem::zeroed() };
        let result = unsafe {
            GetPerTcpConnectionEStats(
                row.as_ptr() as *const MIB_TCPROW_LH,
                TcpConnectionEstatsData,
                std::ptr::null_mut(),
                0,
                0,
                std::ptr::null_mut(),
                0,
                0,
                (&mut rod as *mut TCP_ESTATS_DATA_ROD_v0) as *mut u8,
                0,
                std::mem::size_of::<TCP_ESTATS_DATA_ROD_v0>() as u32,
            )
        };
        if result == 0 {
            (rod.DataBytesOut, rod.DataBytesIn)
        } else {
            (0, 0)
        }
    }

    /// Turns on per-connection data collection for one row. Returns true
    /// only when the OS accepted it — without elevation this fails with
    /// ERROR_ACCESS_DENIED and the row must not be trusted with counters.
    ///
    /// SAFETY: same MIB_TCPROW_LH layout cast as tcp_v4_traffic; rw points
    /// at one valid RW struct sized by rwsize.
    fn enable_data_collection(row: &[u32; 5]) -> bool {
        let rw = TCP_ESTATS_DATA_RW_v0 {
            EnableCollection: 1 as BOOLEAN,
        };
        let result = unsafe {
            SetPerTcpConnectionEStats(
                row.as_ptr() as *const MIB_TCPROW_LH,
                TcpConnectionEstatsData,
                (&rw as *const TCP_ESTATS_DATA_RW_v0) as *const u8,
                0,
                std::mem::size_of::<TCP_ESTATS_DATA_RW_v0>() as u32,
                0,
            )
        };
        result == 0
    }

    /// Extracts the port from a DWORD holding it in network byte order in
    /// the lower 16 bits
    fn port_from_dword(value: u32) -> u16 {
        u16::from_be((value & 0xFFFF) as u16)
    }

    fn query_tcp(af: u32, what: &str) -> Result<Vec<u32>, String> {
        winbuf::query_growing_table(what, |table, size| unsafe {
            GetExtendedTcpTable(table, size, 0, af, TCP_TABLE_OWNER_PID_ALL, 0)
        })
    }

    fn query_udp(af: u32, what: &str) -> Result<Vec<u32>, String> {
        winbuf::query_growing_table(what, |table, size| unsafe {
            GetExtendedUdpTable(table, size, 0, af, UDP_TABLE_OWNER_PID, 0)
        })
    }

    fn parse_tcp_v4(
        buffer: &[u32],
        watched: &mut HashSet<[u32; 4]>,
        known: &mut HashMap<String, [u32; 5]>,
    ) -> Vec<PortConnection> {
        if buffer.is_empty() {
            return Vec::new();
        }
        let table = buffer.as_ptr() as *const MIB_TCPTABLE_OWNER_PID;
        // SAFETY: the buffer is owned by us and GetExtendedTcpTable guarantees
        // it holds dwNumEntries MIB_TCPROW_OWNER_PID rows after the header
        let (count, rows_ptr) = unsafe {
            (
                (*table).dwNumEntries as usize,
                std::ptr::addr_of!((*table).table) as *const MIB_TCPROW_OWNER_PID,
            )
        };
        let rows = unsafe { std::slice::from_raw_parts(rows_ptr, count) };
        let mut connections = Vec::with_capacity(count);
        let mut seen = HashSet::with_capacity(count);
        for row in rows {
            // Watched key: the address/port 4-tuple WITHOUT the state, so
            // ordinary state transitions never reset the counters
            let tuple: [u32; 4] = [
                row.dwLocalAddr,
                row.dwLocalPort,
                row.dwRemoteAddr,
                row.dwRemotePort,
            ];
            let estats_row: [u32; 5] = [
                row.dwState,
                row.dwLocalAddr,
                row.dwLocalPort,
                row.dwRemoteAddr,
                row.dwRemotePort,
            ];
            seen.insert(tuple);
            if watched.insert(tuple) {
                // First time we see this connection: try to turn its
                // counters on. When the Set fails (non-elevated) the row
                // leaves the watched set again so its traffic is reported
                // as zero instead of the OS's meaningless numbers.
                if !enable_data_collection(&estats_row) {
                    watched.remove(&tuple);
                }
            }
            let local_addr = ipv4_from_dword(row.dwLocalAddr).to_string();
            let remote_addr = ipv4_from_dword(row.dwRemoteAddr).to_string();
            let local_port = port_from_dword(row.dwLocalPort);
            let remote_port = port_from_dword(row.dwRemotePort);
            let (bytes_sent, bytes_received) = if watched.contains(&tuple) {
                tcp_row_traffic(&estats_row)
            } else {
                (0, 0)
            };
            // Mirror the frontend's connectionKey so the fast traffic
            // ticker can address rows directly
            known.insert(
                format!(
                    "TCP|{}|{}|{}|{}|{}",
                    local_addr, local_port, remote_addr, remote_port, row.dwOwningPid
                ),
                estats_row,
            );
            connections.push(PortConnection {
                protocol: "TCP".to_string(),
                local_addr,
                local_port,
                remote_addr,
                remote_port,
                state: tcp_state_name(row.dwState).to_string(),
                pid: row.dwOwningPid,
                bytes_sent,
                bytes_received,
            });
        }
        // Rows that vanished from this snapshot leave both maps so a future
        // row reusing the same tuple/key gets enrolled (and zeroed) again
        watched.retain(|key| seen.contains(key));
        known.retain(|_, estats_row| seen.contains(&estats_row[1..5]));
        connections
    }

    fn parse_tcp_v6(buffer: &[u32]) -> Vec<PortConnection> {
        if buffer.is_empty() {
            return Vec::new();
        }
        let table = buffer.as_ptr() as *const MIB_TCP6TABLE_OWNER_PID;
        // SAFETY: the buffer is owned by us and GetExtendedTcpTable guarantees
        // it holds dwNumEntries MIB_TCP6ROW_OWNER_PID rows after the header
        let (count, rows_ptr) = unsafe {
            (
                (*table).dwNumEntries as usize,
                std::ptr::addr_of!((*table).table) as *const MIB_TCP6ROW_OWNER_PID,
            )
        };
        let rows = unsafe { std::slice::from_raw_parts(rows_ptr, count) };
        rows.iter()
            .map(|row| PortConnection {
                protocol: "TCP".to_string(),
                local_addr: Ipv6Addr::from(row.ucLocalAddr).to_string(),
                local_port: port_from_dword(row.dwLocalPort),
                remote_addr: Ipv6Addr::from(row.ucRemoteAddr).to_string(),
                remote_port: port_from_dword(row.dwRemotePort),
                state: tcp_state_name(row.dwState).to_string(),
                pid: row.dwOwningPid,
                // Per-connection ESTATS has no IPv6 helper wired up here
                bytes_sent: 0,
                bytes_received: 0,
            })
            .collect()
    }

    fn parse_udp_v4(buffer: &[u32]) -> Vec<PortConnection> {
        if buffer.is_empty() {
            return Vec::new();
        }
        let table = buffer.as_ptr() as *const MIB_UDPTABLE_OWNER_PID;
        // SAFETY: the buffer is owned by us and GetExtendedUdpTable guarantees
        // it holds dwNumEntries MIB_UDPROW_OWNER_PID rows after the header
        let (count, rows_ptr) = unsafe {
            (
                (*table).dwNumEntries as usize,
                std::ptr::addr_of!((*table).table) as *const MIB_UDPROW_OWNER_PID,
            )
        };
        let rows = unsafe { std::slice::from_raw_parts(rows_ptr, count) };
        rows.iter()
            .map(|row| PortConnection {
                protocol: "UDP".to_string(),
                local_addr: ipv4_from_dword(row.dwLocalAddr).to_string(),
                local_port: port_from_dword(row.dwLocalPort),
                // UDP is connectionless; present it uniformly as remote-less
                remote_addr: String::new(),
                remote_port: 0,
                state: "-".to_string(),
                pid: row.dwOwningPid,
                // UDP has no per-connection byte counters
                bytes_sent: 0,
                bytes_received: 0,
            })
            .collect()
    }

    fn parse_udp_v6(buffer: &[u32]) -> Vec<PortConnection> {
        if buffer.is_empty() {
            return Vec::new();
        }
        let table = buffer.as_ptr() as *const MIB_UDP6TABLE_OWNER_PID;
        // SAFETY: the buffer is owned by us and GetExtendedUdpTable guarantees
        // it holds dwNumEntries MIB_UDP6ROW_OWNER_PID rows after the header
        let (count, rows_ptr) = unsafe {
            (
                (*table).dwNumEntries as usize,
                std::ptr::addr_of!((*table).table) as *const MIB_UDP6ROW_OWNER_PID,
            )
        };
        let rows = unsafe { std::slice::from_raw_parts(rows_ptr, count) };
        rows.iter()
            .map(|row| PortConnection {
                protocol: "UDP".to_string(),
                local_addr: Ipv6Addr::from(row.ucLocalAddr).to_string(),
                local_port: port_from_dword(row.dwLocalPort),
                remote_addr: String::new(),
                remote_port: 0,
                state: "-".to_string(),
                pid: row.dwOwningPid,
                // UDP has no per-connection byte counters
                bytes_sent: 0,
                bytes_received: 0,
            })
            .collect()
    }

    pub fn collect() -> Result<Vec<PortConnection>, String> {
        let mut connections = Vec::new();
        let mut failed = 0;
        // Hold both maps' locks across the TCP v4 parse so enrollment and
        // pruning happen atomically per snapshot
        let mut watched = watched_rows()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut known = known_rows()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match query_tcp(AF_INET as u32, "IPv4 TCP table") {
            Ok(buffer) => {
                connections.extend(parse_tcp_v4(&buffer, &mut watched, &mut known))
            }
            Err(_) => failed += 1,
        }
        match query_tcp(AF_INET6 as u32, "IPv6 TCP table") {
            Ok(buffer) => connections.extend(parse_tcp_v6(&buffer)),
            // IPv6 may be unsupported on the system; keep the IPv4 rows
            Err(_) => failed += 1,
        }
        match query_udp(AF_INET as u32, "IPv4 UDP table") {
            Ok(buffer) => connections.extend(parse_udp_v4(&buffer)),
            Err(_) => failed += 1,
        }
        match query_udp(AF_INET6 as u32, "IPv6 UDP table") {
            Ok(buffer) => connections.extend(parse_udp_v6(&buffer)),
            Err(_) => failed += 1,
        }
        if failed == 4 {
            return Err("Failed to query network tables".to_string());
        }
        Ok(connections)
    }

    /// Fast traffic tick: reads the ESTATS counters for exactly the
    /// requested connection keys (frontend `connectionKey` format) from
    /// the last snapshot's row map, without re-enumerating any table.
    /// Keys that are unknown, IPv6/UDP, or not enrolled for collection
    /// report zeros.
    pub fn traffic_counters(keys: &[String]) -> HashMap<String, TrafficCounters> {
        let known = known_rows()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let watched = watched_rows()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut out = HashMap::with_capacity(keys.len());
        for key in keys {
            let counters = match known.get(key) {
                // SAFETY comment lives on tcp_row_traffic; the row was
                // captured from the same table layout this module parses
                Some(row) if watched.contains(&row[1..5]) => {
                    let (sent, received) = tcp_row_traffic(row);
                    TrafficCounters { sent, received }
                }
                _ => TrafficCounters {
                    sent: 0,
                    received: 0,
                },
            };
            out.insert(key.clone(), counters);
        }
        out
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::PortConnection;
    use std::collections::HashMap;
    use std::fs;
    use std::net::{Ipv4Addr, Ipv6Addr};

    /// The /proc/net tables to read, with their protocol and address family
    const PROC_NET_TABLES: [(&str, &str, bool); 4] = [
        ("/proc/net/tcp", "TCP", false),
        ("/proc/net/tcp6", "TCP", true),
        ("/proc/net/udp", "UDP", false),
        ("/proc/net/udp6", "UDP", true),
    ];

    /// Parses one /proc/net/{tcp,udp,...} table. Fields per line:
    /// sl local_address rem_address st ... uid timeout inode ...
    fn parse_proc_net(
        content: &str,
        protocol: &str,
        is_ipv6: bool,
        inode_to_pid: &HashMap<u64, u32>,
    ) -> Vec<PortConnection> {
        let mut connections = Vec::new();
        for line in content.lines().skip(1) {
            let fields: Vec<&str> = line.split_whitespace().collect();
            if fields.len() < 10 {
                continue;
            }
            let Some((local_addr, local_port)) = parse_endpoint(fields[1], is_ipv6) else {
                continue;
            };
            let Some((remote_addr, remote_port)) = parse_endpoint(fields[2], is_ipv6) else {
                continue;
            };
            let inode = fields[9].parse::<u64>().unwrap_or(0);
            connections.push(PortConnection {
                protocol: protocol.to_string(),
                local_addr,
                local_port,
                // UDP is connectionless; present it uniformly as remote-less
                remote_addr: if protocol == "UDP" {
                    String::new()
                } else {
                    remote_addr
                },
                remote_port: if protocol == "UDP" { 0 } else { remote_port },
                state: if protocol == "UDP" {
                    "-".to_string()
                } else {
                    state_name(fields[3]).to_string()
                },
                pid: inode_to_pid.get(&inode).copied().unwrap_or(0),
                bytes_sent: 0,
                bytes_received: 0,
            });
        }
        connections
    }

    /// Parses a "ADDR:PORT" hex field from /proc/net. IPv4 addresses are
    /// printed as the little-endian DWORD (e.g. 127.0.0.1 is "0100007F");
    /// IPv6 addresses have each 32-bit word byte-swapped (::1 ends with
    /// "01000000"). Ports are plain hex.
    fn parse_endpoint(field: &str, is_ipv6: bool) -> Option<(String, u16)> {
        let (addr_hex, port_hex) = field.split_once(':')?;
        let port = u16::from_str_radix(port_hex, 16).ok()?;
        if is_ipv6 {
            if addr_hex.len() != 32 {
                return None;
            }
            let mut printed = [0u8; 16];
            for (i, byte) in printed.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&addr_hex[i * 2..i * 2 + 2], 16).ok()?;
            }
            let mut octets = [0u8; 16];
            for word in 0..4 {
                for byte in 0..4 {
                    octets[word * 4 + byte] = printed[word * 4 + (3 - byte)];
                }
            }
            Some((Ipv6Addr::from(octets).to_string(), port))
        } else {
            if addr_hex.len() != 8 {
                return None;
            }
            let mut bytes = [0u8; 4];
            for (i, byte) in bytes.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&addr_hex[i * 2..i * 2 + 2], 16).ok()?;
            }
            Some((Ipv4Addr::from(u32::from_le_bytes(bytes)).to_string(), port))
        }
    }

    /// Maps the st column value to its official name
    fn state_name(st: &str) -> &'static str {
        match st {
            "01" => "ESTABLISHED",
            "02" => "SYN_SENT",
            "03" => "SYN_RECV",
            "04" => "FIN_WAIT1",
            "05" => "FIN_WAIT2",
            "06" => "TIME_WAIT",
            "07" => "CLOSE",
            "08" => "CLOSE_WAIT",
            "09" => "LAST_ACK",
            "0A" => "LISTEN",
            "0B" => "CLOSING",
            "0C" => "NEW_SYN_RECV",
            _ => "UNKNOWN",
        }
    }

    /// Scans /proc/<pid>/fd for socket:[inode] symlinks to map inodes to
    /// owning PIDs. Directories without read permission are skipped.
    fn build_inode_to_pid_map() -> HashMap<u64, u32> {
        let mut map = HashMap::new();
        let Ok(entries) = fs::read_dir("/proc") else {
            return map;
        };
        for entry in entries.flatten() {
            let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u32>().ok())
            else {
                continue;
            };
            let Ok(fds) = fs::read_dir(entry.path().join("fd")) else {
                continue;
            };
            for fd in fds.flatten() {
                let Ok(target) = fs::read_link(fd.path()) else {
                    continue;
                };
                let target = target.to_string_lossy();
                if let Some(inode) = target
                    .strip_prefix("socket:[")
                    .and_then(|rest| rest.strip_suffix(']'))
                    .and_then(|inode| inode.parse::<u64>().ok())
                {
                    map.insert(inode, pid);
                }
            }
        }
        map
    }

    pub fn collect() -> Result<Vec<PortConnection>, String> {
        let inode_to_pid = build_inode_to_pid_map();
        let mut connections = Vec::new();
        let mut unreadable = 0;
        for (path, protocol, is_ipv6) in PROC_NET_TABLES {
            match fs::read_to_string(path) {
                Ok(content) => {
                    connections.extend(parse_proc_net(
                        &content,
                        protocol,
                        is_ipv6,
                        &inode_to_pid,
                    ));
                }
                // tcp6/udp6 may be missing on IPv4-only systems
                Err(_) => unreadable += 1,
            }
        }
        if unreadable == PROC_NET_TABLES.len() {
            return Err("Failed to read /proc/net tables".to_string());
        }
        Ok(connections)
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::PortConnection;
    use std::process::Command;

    pub fn collect() -> Result<Vec<PortConnection>, String> {
        let output = match Command::new("lsof").args(["-nP", "-iTCP", "-iUDP"]).output() {
            Ok(output) => output,
            // lsof may be missing or fail; treat as no connections instead of
            // surfacing an error for the whole command
            Err(_) => return Ok(Vec::new()),
        };
        if !output.status.success() {
            return Ok(Vec::new());
        }
        Ok(parse_lsof(&String::from_utf8_lossy(&output.stdout)))
    }

    /// Parses lsof -nP -iTCP -iUDP output. Example lines:
    /// COMMAND   PID USER   FD   TYPE  DEVICE SIZE/OFF NODE NAME
    /// rapportd  434  user    4u  IPv4  0x...      0t0  TCP *:49152 (LISTEN)
    /// rapportd  434  user    5u  IPv6  0x...      0t0  UDP *:5353
    fn parse_lsof(output: &str) -> Vec<PortConnection> {
        let mut connections = Vec::new();
        for line in output.lines().skip(1) {
            let mut fields = line.split_whitespace();
            // COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME [(STATE)]
            let _command = fields.next();
            let Some(pid) = fields.next().and_then(|field| field.parse::<u32>().ok()) else {
                continue;
            };
            let _user = fields.next();
            let _fd = fields.next();
            let _type = fields.next();
            let _device = fields.next();
            let _size = fields.next();
            let Some(node) = fields.next() else {
                continue;
            };
            let protocol = match node {
                "TCP" => "TCP",
                "UDP" => "UDP",
                _ => continue,
            };
            let mut name_parts: Vec<&str> = fields.collect();
            if name_parts.is_empty() {
                continue;
            }
            // TCP lines carry a trailing "(STATE)" token
            let mut state = "-";
            if name_parts.len() > 1 && name_parts.last().unwrap().starts_with('(') {
                let last = name_parts.pop().unwrap();
                state = last.trim_start_matches('(').trim_end_matches(')');
            }
            // TCP connections are "local->remote"; listeners and UDP are
            // local-only. UDP is presented uniformly as remote-less.
            let (local_name, remote_name) = match name_parts[0].split_once("->") {
                Some((local, remote)) => (local, Some(remote)),
                None => (name_parts[0], None),
            };
            let Some((local_addr, local_port)) = parse_endpoint(local_name) else {
                continue;
            };
            let (remote_addr, remote_port) = if protocol == "UDP" {
                (String::new(), 0)
            } else {
                remote_name
                    .and_then(parse_endpoint)
                    .unwrap_or((String::new(), 0))
            };
            connections.push(PortConnection {
                protocol: protocol.to_string(),
                local_addr,
                local_port,
                remote_addr,
                remote_port,
                state: if protocol == "UDP" {
                    "-".to_string()
                } else {
                    state.to_string()
                },
                pid,
                bytes_sent: 0,
                bytes_received: 0,
            });
        }
        connections
    }

    /// Splits "host:port", "[v6host]:port" or "*:port" into address and
    /// port. The host part is kept verbatim (lsof prints "*" for wildcard
    /// binds).
    fn parse_endpoint(endpoint: &str) -> Option<(String, u16)> {
        let (host, port) = endpoint.rsplit_once(':')?;
        let port = port.parse::<u16>().ok()?;
        let host = host.trim_start_matches('[').trim_end_matches(']');
        Some((host.to_string(), port))
    }
}

#[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
mod platform {
    use super::PortConnection;

    pub fn collect() -> Result<Vec<PortConnection>, String> {
        // Network port collection is not implemented on this platform
        Ok(Vec::new())
    }
}
