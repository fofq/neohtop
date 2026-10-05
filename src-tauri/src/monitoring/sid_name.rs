//! Friendly account names for Windows SIDs
//!
//! sysinfo reports the process owner as a raw SID string
//! ("S-1-5-21-..."), which is unreadable in the user column. This module
//! converts each SID to its account name via ConvertStringSidToSidW +
//! LookupAccountSidW (well-known SIDs like SYSTEM come out named too) and
//! caches results forever — SIDs are stable for the lifetime of a boot.

#[cfg(windows)]
pub fn friendly_name(sid: &str) -> String {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::sync::{Mutex, OnceLock};
    use windows_sys::Win32::Foundation::LocalFree;
    // ConvertStringSidToSidW is declared in Sddl.h and lives under the
    // Authorization submodule in windows-sys, not Security directly
    use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
    use windows_sys::Win32::Security::LookupAccountSidW;

    static CACHE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

    fn lock_cache() -> std::sync::MutexGuard<'static, HashMap<String, String>> {
        match CACHE.get_or_init(|| Mutex::new(HashMap::new())).lock() {
            Ok(cache) => cache,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    /// Reads a NUL-terminated UTF-16 Win32 string
    fn from_wide(ptr: *const u16) -> String {
        let mut len = 0usize;
        while unsafe { *ptr.add(len) } != 0 {
            len += 1;
        }
        let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
        OsString::from_wide(slice).to_string_lossy().into_owned()
    }

    fn lookup(sid: &str) -> Option<String> {
        let wide: Vec<u16> = sid.encode_utf16().chain(Some(0)).collect();
        let mut psid: *mut core::ffi::c_void = std::ptr::null_mut();
        // SAFETY: wide is a NUL-terminated buffer; psid receives a
        // LocalAlloc'd SID owned by us and freed below
        if unsafe { ConvertStringSidToSidW(wide.as_ptr(), &mut psid) } == 0 {
            return None;
        }
        let mut name_buf = [0u16; 256];
        let mut domain_buf = [0u16; 256];
        let mut name_len = name_buf.len() as u32;
        let mut domain_len = domain_buf.len() as u32;
        // SID_NAME_USE is an i32 alias in windows-sys
        let mut use_type: i32 = 0;
        // SAFETY: buffers are sized by the len arguments; psid is the
        // valid SID from ConvertStringSidToSidW
        let ok = unsafe {
            LookupAccountSidW(
                std::ptr::null(),
                psid,
                name_buf.as_mut_ptr(),
                &mut name_len,
                domain_buf.as_mut_ptr(),
                &mut domain_len,
                &mut use_type,
            )
        };
        // SAFETY: psid was allocated by ConvertStringSidToSidW
        unsafe { LocalFree(psid) };
        if ok == 0 {
            return None;
        }
        let name = from_wide(name_buf.as_ptr());
        if name.is_empty() {
            None
        } else {
            Some(name)
        }
    }

    if let Some(hit) = lock_cache().get(sid) {
        return hit.clone();
    }
    let resolved = lookup(sid).unwrap_or_else(|| sid.to_string());
    lock_cache().insert(sid.to_string(), resolved.clone());
    resolved
}

#[cfg(not(windows))]
pub fn friendly_name(sid: &str) -> String {
    sid.to_string()
}
