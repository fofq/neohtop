//! Shared grow-and-retry helpers for Win32 buffer enumeration
//!
//! The commands in this module query Win32 APIs that fill a caller-provided
//! buffer whose size cannot be known upfront (service lists, connection
//! tables, module lists). The documented "buffer too small" signal differs
//! per API family: the IpHelper table functions and other classic
//! GetLastError-style APIs document ERROR_INSUFFICIENT_BUFFER (122), while
//! APIs that report what did not fit (EnumServicesStatusExW, RmGetList)
//! document ERROR_MORE_DATA (234) for the same condition. The helpers below
//! centralize the retry handling so every command accepts both codes and
//! keeps the original error code in its messages.

use windows_sys::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_MORE_DATA, ERROR_NO_DATA};

/// Initial size for the table buffers; grown when the API reports the
/// buffer is too small
const INITIAL_BUFFER_SIZE: usize = 16 * 1024;

/// How many times a table query is retried with a larger buffer (the table
/// can grow between calls as connections are constantly created and
/// destroyed)
const MAX_BUFFER_GROWTH_RETRIES: u32 = 3;

/// Returns whether the Win32 error code means the caller's buffer was too
/// small. ERROR_INSUFFICIENT_BUFFER is what the IpHelper table functions and
/// other classic GetLastError-style APIs document, while APIs such as
/// EnumServicesStatusExW and RmGetList document ERROR_MORE_DATA for the same
/// condition.
pub(crate) fn is_buffer_too_small(error: u32) -> bool {
    error == ERROR_INSUFFICIENT_BUFFER || error == ERROR_MORE_DATA
}

/// Calls an IpHelper table function (GetExtendedTcpTable /
/// GetExtendedUdpTable) against a growing buffer and returns the raw table.
/// The query receives the buffer pointer and a size out-parameter, reports
/// the required size in the latter and is retried while it reports the
/// buffer too small, because the table can grow between calls. An empty
/// table (ERROR_NO_DATA) is reported as an empty buffer, not an error.
/// `what` names the table in error messages, which keep the original error
/// code. The buffer is backed by u32 elements so it is suitably aligned for
/// the table structs.
pub(crate) fn query_growing_table(
    what: &str,
    query: impl Fn(*mut std::ffi::c_void, *mut u32) -> u32,
) -> Result<Vec<u32>, String> {
    let mut size: u32 = INITIAL_BUFFER_SIZE as u32;
    let mut buffer: Vec<u32> = vec![0; INITIAL_BUFFER_SIZE / std::mem::size_of::<u32>()];
    let mut result = query(buffer.as_mut_ptr() as *mut _, &mut size);
    for _ in 0..MAX_BUFFER_GROWTH_RETRIES {
        if !is_buffer_too_small(result) {
            break;
        }
        // size now holds the required size; round up to whole u32 slots,
        // refusing a size whose rounding would overflow the usize math on
        // 32-bit targets
        let byte_len = (size as usize)
            .checked_add(std::mem::size_of::<u32>() - 1)
            .ok_or_else(|| format!("Failed to query the {}: the required size overflowed", what))?;
        buffer = vec![0; byte_len / std::mem::size_of::<u32>()];
        result = query(buffer.as_mut_ptr() as *mut _, &mut size);
    }
    if result == ERROR_NO_DATA {
        // An empty table is not an error for our purposes
        return Ok(Vec::new());
    }
    if result != 0 {
        return Err(format!(
            "Failed to query the {} (Windows error {})",
            what, result
        ));
    }
    Ok(buffer)
}
