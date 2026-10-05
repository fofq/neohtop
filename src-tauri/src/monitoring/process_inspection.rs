//! Process inspection
//!
//! Read-only inspection of a single process and of the kernel: executable
//! metadata (company, description, version and elevation read from the
//! version resource of the executable), loaded modules (DLLs) of a process
//! and the loaded device drivers of the system. Metadata works on every
//! platform (best effort); module and driver enumeration only exist on
//! Windows, where psapi reports them.

use serde::Serialize;

/// Version-resource metadata and elevation of one process
#[derive(Serialize, Debug)]
pub struct ProcessMetadata {
    /// Full path of the executable image
    pub exe_path: String,
    /// Company name from the version resource
    pub company: String,
    /// File description from the version resource
    pub description: String,
    /// File version from the version resource (or the fixed version block)
    pub version: String,
    /// Whether the process runs with elevated privileges; processes that
    /// cannot be opened (protected) are reported as not elevated
    pub elevated: bool,
    /// True when the executable image path is known but the file no longer
    /// exists on disk (deleted after launch / removed by a cleaner)
    pub binary_missing: bool,
    /// Authenticode signature check of the executable: Some(true) signed
    /// and trusted, Some(false) definitively unsigned, None when the
    /// check could not run (non-Windows, unreadable or deleted binary)
    pub signed: Option<bool>,
}

/// A module (DLL) loaded by a process
#[derive(Serialize, Debug)]
pub struct ModuleInfo {
    /// File name of the module, e.g. "kernel32.dll"
    pub name: String,
    /// Full path of the module on disk
    pub path: String,
    /// Size of the mapped image in bytes
    pub size: u64,
    /// Base address of the mapped image (decimal; the frontend renders hex)
    pub base_address: u64,
}

/// A kernel-mode driver loaded on the system
#[derive(Serialize, Debug)]
pub struct DriverInfo {
    /// Base name of the driver image, e.g. "ntoskrnl.exe"
    pub name: String,
    /// Full path of the driver image
    pub path: String,
    /// Base address of the loaded image (decimal; the frontend renders hex)
    pub base_address: u64,
}

/// Reads the metadata (company, description, version, elevation) of a process
pub fn get_metadata(pid: u32) -> Result<ProcessMetadata, String> {
    platform::get_metadata(pid)
}

/// Reads the process token's mandatory integrity level and maps it to a
/// stable token for localization ("low", "medium", "high", "system",
/// "protected", "untrusted"); "unknown" on other platforms
pub fn get_process_integrity(pid: u32) -> Result<String, String> {
    platform::get_integrity(pid)
}

/// Lists the modules (DLLs) loaded by a process
pub fn list_modules(pid: u32) -> Result<Vec<ModuleInfo>, String> {
    platform::list_modules(pid)
}

/// Lists the kernel-mode drivers loaded on the system
pub fn list_drivers() -> Result<Vec<DriverInfo>, String> {
    platform::list_drivers()
}

#[cfg(windows)]
mod platform {
    use super::{DriverInfo, ModuleInfo, ProcessMetadata};
    use crate::monitoring::winbuf;
    use std::ffi::c_void;
    use windows_sys::Win32::Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_INSUFFICIENT_BUFFER, GetLastError, HANDLE,
        TRUST_E_NOSIGNATURE,
    };
    use windows_sys::core::GUID;
    use windows_sys::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
        TOKEN_ELEVATION, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenElevation,
        TokenIntegrityLevel,
    };
    use windows_sys::Win32::Security::WinTrust::{
        WinVerifyTrust, WINTRUST_ACTION_GENERIC_VERIFY_V2, WINTRUST_DATA, WINTRUST_DATA_0,
        WINTRUST_FILE_INFO, WTD_CHOICE_FILE, WTD_REVOKE_NONE, WTD_STATEACTION_CLOSE,
        WTD_STATEACTION_VERIFY, WTD_UI_NONE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
    };
    use windows_sys::Win32::System::ProcessStatus::{
        EnumDeviceDrivers, EnumProcessModulesEx, GetDeviceDriverBaseNameW, GetDeviceDriverFileNameW,
        GetModuleFileNameExW, GetModuleInformation, LIST_MODULES_ALL, MODULEINFO,
    };
    use windows_sys::Win32::System::Threading::{
        OpenProcess, OpenProcessToken, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ,
    };

    /// Initial buffer size for image path queries, doubled while the API
    /// reports the buffer is too small
    const INITIAL_PATH_LEN: u32 = 1024;

    /// Upper bound for path buffer growth; Windows paths cannot exceed this
    const MAX_PATH_LEN: u32 = 32768;

    /// How many times a psapi enumeration is retried with a larger buffer
    /// (the module/driver list can change between calls)
    const MAX_BUFFER_GROWTH_RETRIES: u32 = 3;

    pub fn get_metadata(pid: u32) -> Result<ProcessMetadata, String> {
        // The limited query right is enough for the image path and the
        // process token, and succeeds for far more processes than
        // PROCESS_QUERY_INFORMATION (protected/system processes included)
        let handle = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        // SAFETY: handle is valid with PROCESS_QUERY_LIMITED_INFORMATION
        // access; every call below only writes into our own buffers
        let exe_path = query_image_path(handle);
        let elevated = query_elevation(handle);
        // SAFETY: handle is the valid process handle opened above
        unsafe { CloseHandle(handle) };

        let (company, description, version) = exe_path
            .as_deref()
            .map(read_version_strings)
            .unwrap_or_default();
        let binary_missing = exe_path
            .as_deref()
            .is_some_and(|path| !std::path::Path::new(path).exists());
        // Only verify binaries that actually exist on disk; a deleted
        // image would fail the trust call for the wrong reason
        let signed = exe_path
            .as_deref()
            .filter(|path| std::path::Path::new(path).exists())
            .and_then(verify_signature);
        Ok(ProcessMetadata {
            exe_path: exe_path.unwrap_or_default(),
            company,
            description,
            version,
            elevated,
            binary_missing,
            signed,
        })
    }

    pub fn list_modules(pid: u32) -> Result<Vec<ModuleInfo>, String> {
        // psapi module enumeration walks the target's loader data, which
        // needs both query rights and memory read access
        let handle = open_process(pid, PROCESS_QUERY_INFORMATION | PROCESS_VM_READ)?;
        let enum_result = enumerate_growing(
            &format!("modules of process {}", pid),
            |buffer, byte_len, needed| unsafe {
                // SAFETY: buffer is null or holds byte_len bytes of module
                // handle slots (see enumerate_growing); handle stays valid
                // for the following calls
                EnumProcessModulesEx(handle, buffer, byte_len, needed, LIST_MODULES_ALL)
            },
        );
        let modules = match enum_result {
            Ok(modules) => modules,
            Err(message) => {
                // SAFETY: handle is the valid process handle opened above
                unsafe { CloseHandle(handle) };
                return Err(message);
            }
        };

        let mut result = Vec::with_capacity(modules.len());
        for module in &modules {
            // A module unloaded between the two calls reads back empty; skip
            // it instead of failing the whole list
            let Some(path) = query_module_path(handle, *module) else {
                continue;
            };
            // SAFETY: handle is valid and module came from the enumeration
            let mut info: MODULEINFO = unsafe { std::mem::zeroed() };
            let known = unsafe {
                GetModuleInformation(
                    handle,
                    *module,
                    &mut info as *mut MODULEINFO,
                    std::mem::size_of::<MODULEINFO>() as u32,
                )
            } != 0;
            result.push(ModuleInfo {
                name: file_name_of(&path),
                path,
                size: if known { info.SizeOfImage as u64 } else { 0 },
                base_address: info.lpBaseOfDll as usize as u64,
            });
        }
        // SAFETY: handle is the valid process handle opened above; the
        // module queries in the loop above were its last users
        unsafe { CloseHandle(handle) };
        Ok(result)
    }

    pub fn list_drivers() -> Result<Vec<DriverInfo>, String> {
        let bases = enumerate_growing("device drivers", |buffer, byte_len, needed| unsafe {
            // SAFETY: buffer is null or holds byte_len bytes of driver base
            // slots (see enumerate_growing); needed is a plain out-parameter
            EnumDeviceDrivers(buffer, byte_len, needed)
        })?;

        let mut drivers = Vec::with_capacity(bases.len());
        for base in &bases {
            if base.is_null() {
                continue;
            }
            let path = query_driver_image_path(*base);
            // The base name query is the fallback when the full path is
            // unreadable (some drivers restrict their image section)
            let mut name = file_name_of(&path);
            if name.is_empty() {
                name = query_driver_base_name(*base);
            }
            drivers.push(DriverInfo {
                name,
                path,
                base_address: *base as usize as u64,
            });
        }
        Ok(drivers)
    }

    /// Runs a psapi-style enumeration (buffer pointer, byte size, byte count
    /// out-parameter) into a growing buffer of handle slots and returns the
    /// slots the API actually wrote. The buffer passed to `enumerate` is
    /// null when its byte size is 0, otherwise it holds that many bytes of
    /// zeroed slots. The first call probes the required size with an empty
    /// buffer; the API reports it in the count out-parameter, either by
    /// succeeding with a size larger than the one passed (the too-small
    /// signal documented for EnumProcessModulesEx) or by failing with a
    /// too-small buffer error, and psapi documents re-calling with that
    /// size because the list can change between calls. `what` names the
    /// list in error messages, which keep the original error code.
    fn enumerate_growing(
        what: &str,
        enumerate: impl Fn(*mut *mut c_void, u32, *mut u32) -> i32,
    ) -> Result<Vec<*mut c_void>, String> {
        let mut slots: Vec<*mut c_void> = Vec::new();
        for _ in 0..MAX_BUFFER_GROWTH_RETRIES {
            let mut needed: u32 = 0;
            let byte_len = (slots.len() * std::mem::size_of::<*mut c_void>()) as u32;
            let ok = enumerate(
                if slots.is_empty() {
                    std::ptr::null_mut()
                } else {
                    slots.as_mut_ptr()
                },
                byte_len,
                &mut needed,
            );
            if ok != 0 && needed <= byte_len {
                // On success needed is the number of bytes written; cut the
                // buffer down to the slots it actually holds
                slots.truncate(needed as usize / std::mem::size_of::<*mut c_void>());
                return Ok(slots);
            }
            if ok == 0 {
                let error = unsafe { GetLastError() };
                if !winbuf::is_buffer_too_small(error) {
                    return Err(format!(
                        "Failed to enumerate the {} (Windows error {})",
                        what, error
                    ));
                }
            }
            // Either the call succeeded with a too-small buffer (needed is
            // greater than the size passed) or it failed with a too-small
            // error: needed now holds the required size in bytes
            slots =
                vec![std::ptr::null_mut(); needed as usize / std::mem::size_of::<*mut c_void>()];
        }
        Err(format!(
            "Failed to enumerate the {}: the list kept changing between calls",
            what
        ))
    }

    /// Opens a process handle, mapping access-denied to a message the
    /// frontend recognizes to offer a privileged relaunch
    fn open_process(pid: u32, access: u32) -> Result<HANDLE, String> {
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
                _ => format!("Failed to open process {} (Windows error {})", pid, error),
            });
        }
        Ok(handle)
    }

    /// Reads the executable path of the process; None when the query fails
    fn query_image_path(handle: HANDLE) -> Option<String> {
        let mut len = INITIAL_PATH_LEN;
        while len <= MAX_PATH_LEN {
            let mut buffer = vec![0u16; len as usize];
            let mut size = len;
            // SAFETY: buffer holds `size` UTF-16 slots and size points at
            // the same length; handle is valid with the limited query right
            let ok = unsafe {
                QueryFullProcessImageNameW(
                    handle,
                    PROCESS_NAME_WIN32,
                    buffer.as_mut_ptr(),
                    &mut size,
                )
            };
            if ok != 0 {
                return Some(wide_to_string(&buffer));
            }
            let error = unsafe { GetLastError() };
            if error != ERROR_INSUFFICIENT_BUFFER {
                // Fallback for processes that reject the Win32 path query;
                // the module query may still be allowed for this handle
                return query_module_path(handle, std::ptr::null_mut());
            }
            len *= 2;
        }
        None
    }

    /// Reads the path of a module (the executable when module is null);
    /// None when it was unloaded or is unreadable
    fn query_module_path(handle: HANDLE, module: *mut c_void) -> Option<String> {
        let mut buffer = vec![0u16; INITIAL_PATH_LEN as usize];
        // SAFETY: handle is valid, module comes from EnumProcessModulesEx
        // (or null for the executable) and buffer holds INITIAL_PATH_LEN
        // UTF-16 slots
        let written =
            unsafe { GetModuleFileNameExW(handle, module, buffer.as_mut_ptr(), INITIAL_PATH_LEN) };
        if written == 0 {
            return None;
        }
        Some(wide_to_string(&buffer))
    }

    /// Reads whether the process token has the elevation flag set
    fn query_elevation(handle: HANDLE) -> bool {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: handle is valid with PROCESS_QUERY_LIMITED_INFORMATION
        // and token receives a new handle we own and close below
        let opened = unsafe { OpenProcessToken(handle, TOKEN_QUERY, &mut token) };
        if opened == 0 {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned: u32 = 0;
        // SAFETY: token is valid and elevation is a correctly sized,
        // zero-initialized out-parameter
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                &mut elevation as *mut TOKEN_ELEVATION as *mut c_void,
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            )
        };
        // SAFETY: token is the valid handle from OpenProcessToken
        unsafe { CloseHandle(token) };
        ok != 0 && elevation.TokenIsElevated != 0
    }

    /// Reads the mandatory integrity level of the process token and maps
    /// it to a stable token the frontend localizes ("low", "medium",
    /// "high", "system", "protected", "untrusted")
    pub fn get_integrity(pid: u32) -> Result<String, String> {
        let handle = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
        let outcome = query_integrity(handle);
        // SAFETY: handle is the valid process handle opened above
        unsafe { CloseHandle(handle) };
        outcome
    }

    /// SAFETY: handle is valid with PROCESS_QUERY_LIMITED_INFORMATION;
    /// the token opened here is owned and closed on every path
    fn query_integrity(handle: HANDLE) -> Result<String, String> {
        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: see above
        let opened = unsafe { OpenProcessToken(handle, TOKEN_QUERY, &mut token) };
        if opened == 0 {
            return Err("Failed to open the process token".to_string());
        }
        let mut needed: u32 = 0;
        // SAFETY: the null buffer makes the API report the required size
        let _ = unsafe {
            GetTokenInformation(
                token,
                TokenIntegrityLevel,
                std::ptr::null_mut(),
                0,
                &mut needed,
            )
        };
        let mut buffer: Vec<u8> = vec![0; needed as usize];
        // SAFETY: buffer is sized to `needed` bytes as reported above
        let ok = unsafe {
            GetTokenInformation(
                token,
                TokenIntegrityLevel,
                buffer.as_mut_ptr() as *mut c_void,
                needed,
                &mut needed,
            )
        };
        // SAFETY: token is the valid handle from OpenProcessToken
        unsafe { CloseHandle(token) };
        if ok == 0 {
            return Err("Failed to query the integrity level".to_string());
        }
        if (buffer.len() as usize) < std::mem::size_of::<TOKEN_MANDATORY_LABEL>() {
            return Err("Integrity buffer too small".to_string());
        }
        // SAFETY: the buffer now holds a TOKEN_MANDATORY_LABEL whose SID
        // the API owns for the lifetime of the buffer
        let label = unsafe { *(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL) };
        let sid = label.Label.Sid;
        // SAFETY: sid is a valid SID inside the buffer
        let count = unsafe { *GetSidSubAuthorityCount(sid) } as usize;
        if count == 0 {
            return Err("Empty integrity SID".to_string());
        }
        // The RID is the last sub-authority of the integrity SID
        // SAFETY: index is within the SID's sub-authority count
        let rid = unsafe { *GetSidSubAuthority(sid, (count - 1) as u32) };
        Ok(integrity_name(rid))
    }

    fn integrity_name(rid: u32) -> String {
        match rid {
            0x0000 => "untrusted",
            0x1000 => "low",
            0x2000 | 0x2100 => "medium",
            0x3000 => "high",
            0x4000 => "system",
            0x5000..=0x9000 => "protected",
            _ => "medium",
        }
        .to_string()
    }

    /// Verifies the Authenticode signature of an executable against the
    /// system trust stores with WinVerifyTrust (no UI, no revocation
    /// check). Returns Some(true) when the signature chains to a trusted
    /// root, Some(false) when the file definitively has no signature, and
    /// None when the outcome is unknown (file unreadable, provider or
    /// policy error) so the frontend can hide the badge instead of
    /// claiming the binary is unsafe.
    fn verify_signature(path: &str) -> Option<bool> {
        let path_wide = to_wide(path);
        // SAFETY: file_info borrows path_wide, which outlives both trust
        // calls below; the structs are correctly sized PODs the API only
        // reads during each call
        let mut file_info = WINTRUST_FILE_INFO {
            cbStruct: std::mem::size_of::<WINTRUST_FILE_INFO>() as u32,
            pcwszFilePath: path_wide.as_ptr(),
            hFile: std::ptr::null_mut(),
            pgKnownSubject: std::ptr::null_mut(),
        };
        let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
        let mut data = WINTRUST_DATA {
            cbStruct: std::mem::size_of::<WINTRUST_DATA>() as u32,
            pPolicyCallbackData: std::ptr::null_mut(),
            pSIPClientData: std::ptr::null_mut(),
            dwUIChoice: WTD_UI_NONE,
            fdwRevocationChecks: WTD_REVOKE_NONE,
            dwUnionChoice: WTD_CHOICE_FILE,
            Anonymous: WINTRUST_DATA_0 {
                pFile: &mut file_info,
            },
            dwStateAction: WTD_STATEACTION_VERIFY,
            hWVTStateData: std::ptr::null_mut(),
            pwszURLReference: std::ptr::null_mut(),
            dwProvFlags: 0,
            dwUIContext: 0,
            pSignatureSettings: std::ptr::null_mut(),
        };
        // SAFETY: null hwnd means no parent for the (disabled) UI; data
        // and action are valid for the duration of the call
        let result = unsafe {
            WinVerifyTrust(
                std::ptr::null_mut(),
                &mut action,
                &mut data as *mut WINTRUST_DATA as *mut c_void,
            )
        };
        // Every WTD_STATEACTION_VERIFY pass must be closed again or the
        // policy provider leaks its state handle; run the close pass on
        // the same structure
        data.dwStateAction = WTD_STATEACTION_CLOSE;
        // SAFETY: data still carries the state opened by the verify pass
        unsafe {
            WinVerifyTrust(
                std::ptr::null_mut(),
                &mut action,
                &mut data as *mut WINTRUST_DATA as *mut c_void,
            )
        };
        match result {
            0 => Some(true),
            TRUST_E_NOSIGNATURE => Some(false),
            _ => None,
        }
    }

    /// Reads the company, description and version strings from the version
    /// resource of an executable file; missing fields come back empty
    fn read_version_strings(path: &str) -> (String, String, String) {
        let path_wide = to_wide(path);
        // SAFETY: path_wide is a NUL-terminated buffer and the out-parameter
        // is a plain u32
        let size = unsafe { GetFileVersionInfoSizeW(path_wide.as_ptr(), std::ptr::null_mut()) };
        if size == 0 {
            return Default::default();
        }
        let mut data = vec![0u8; size as usize];
        // SAFETY: data is sized to `size` bytes as the API requires
        let ok = unsafe {
            GetFileVersionInfoW(path_wide.as_ptr(), 0, size, data.as_mut_ptr() as *mut c_void)
        };
        if ok == 0 {
            return Default::default();
        }
        let version = query_fixed_version(&data).unwrap_or_default();
        let mut company = String::new();
        let mut description = String::new();
        for language in query_translations(&data) {
            if company.is_empty() {
                company = query_version_string(&data, language, "CompanyName");
            }
            if description.is_empty() {
                description = query_version_string(&data, language, "FileDescription");
            }
            if !company.is_empty() && !description.is_empty() {
                break;
            }
        }
        (company, description, version)
    }

    /// Formats the four version numbers from the fixed version block
    fn query_fixed_version(data: &[u8]) -> Option<String> {
        let mut info: *mut VS_FIXEDFILEINFO = std::ptr::null_mut();
        let mut len: u32 = 0;
        let root = to_wide("\\");
        // SAFETY: data holds the version buffer for the lifetime of the
        // call; info and len are out-parameters validated below
        let ok = unsafe {
            VerQueryValueW(
                data.as_ptr() as *const c_void,
                root.as_ptr(),
                &mut info as *mut *mut VS_FIXEDFILEINFO as *mut *mut c_void,
                &mut len,
            )
        };
        if ok == 0 || info.is_null() {
            return None;
        }
        // SAFETY: on success the API points info at a VS_FIXEDFILEINFO
        // inside the data buffer, which outlives this read
        let info = unsafe { *info };
        Some(format!(
            "{}.{}.{}.{}",
            info.dwFileVersionMS >> 16,
            info.dwFileVersionMS & 0xFFFF,
            info.dwFileVersionLS >> 16,
            info.dwFileVersionLS & 0xFFFF
        ))
    }

    /// Reads the (language id, codepage) pairs from the translation block,
    /// packed as (lang << 16) | codepage
    fn query_translations(data: &[u8]) -> Vec<u32> {
        let mut pointer: *mut c_void = std::ptr::null_mut();
        let mut len: u32 = 0;
        let key = to_wide("\\VarFileInfo\\Translation");
        // SAFETY: data holds the version buffer for the lifetime of the call
        let ok = unsafe {
            VerQueryValueW(
                data.as_ptr() as *const c_void,
                key.as_ptr(),
                &mut pointer,
                &mut len,
            )
        };
        if ok == 0 || pointer.is_null() || len < 4 {
            return Vec::new();
        }
        let pairs = len as usize / 4;
        // SAFETY: on success the API points at `len` bytes of
        // (language id, codepage) u16 pairs inside the data buffer
        let raw = unsafe { std::slice::from_raw_parts(pointer as *const u16, pairs * 2) };
        raw.chunks_exact(2)
            .map(|pair| ((pair[0] as u32) << 16) | pair[1] as u32)
            .collect()
    }

    /// Reads one string value (company name, file description, ...) for the
    /// given translation; empty when the value does not exist
    fn query_version_string(data: &[u8], language: u32, field: &str) -> String {
        let lang = language >> 16;
        let codepage = language & 0xFFFF;
        let key = to_wide(&format!(
            "\\StringFileInfo\\{:04x}{:04x}\\{}",
            lang, codepage, field
        ));
        let mut pointer: *mut c_void = std::ptr::null_mut();
        let mut len: u32 = 0;
        // SAFETY: data holds the version buffer for the lifetime of the call
        let ok = unsafe {
            VerQueryValueW(
                data.as_ptr() as *const c_void,
                key.as_ptr(),
                &mut pointer,
                &mut len,
            )
        };
        if ok == 0 || pointer.is_null() || len < 2 {
            return String::new();
        }
        // SAFETY: on success the API points at a NUL-terminated UTF-16
        // string of at most `len` units inside the data buffer
        let units = unsafe { std::slice::from_raw_parts(pointer as *const u16, len as usize) };
        wide_to_string(units)
    }

    /// Reads the full path of a loaded driver image; empty when unreadable
    fn query_driver_image_path(base: *mut c_void) -> String {
        let mut buffer = vec![0u16; INITIAL_PATH_LEN as usize];
        // SAFETY: base comes from EnumDeviceDrivers and buffer holds
        // INITIAL_PATH_LEN UTF-16 slots
        let written =
            unsafe { GetDeviceDriverFileNameW(base, buffer.as_mut_ptr(), INITIAL_PATH_LEN) };
        if written == 0 {
            return String::new();
        }
        wide_to_string(&buffer)
    }

    /// Reads the base name of a loaded driver image; empty when unreadable
    fn query_driver_base_name(base: *mut c_void) -> String {
        let mut buffer = vec![0u16; INITIAL_PATH_LEN as usize];
        // SAFETY: base comes from EnumDeviceDrivers and buffer holds
        // INITIAL_PATH_LEN UTF-16 slots
        let written =
            unsafe { GetDeviceDriverBaseNameW(base, buffer.as_mut_ptr(), INITIAL_PATH_LEN) };
        if written == 0 {
            return String::new();
        }
        wide_to_string(&buffer)
    }

    /// Extracts the file name from a path, tolerating both separators
    fn file_name_of(path: &str) -> String {
        path.rsplit(['\\', '/'])
            .next()
            .unwrap_or("")
            .to_string()
    }

    /// Reads a NUL-terminated UTF-16 buffer into a String
    fn wide_to_string(units: &[u16]) -> String {
        let len = units
            .iter()
            .position(|unit| *unit == 0)
            .unwrap_or(units.len());
        String::from_utf16_lossy(&units[..len])
    }

    /// Encodes a string as a NUL-terminated UTF-16 buffer for Win32 APIs
    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(not(windows))]
mod platform {
    use super::ProcessMetadata;

    /// Best-effort metadata on other platforms: the executable path can be
    /// read from /proc on Linux; version resources and elevation are not
    /// portable, so they come back empty/false
    pub fn get_integrity(_pid: u32) -> Result<String, String> {
        Ok("unknown".to_string())
    }

    pub fn get_metadata(pid: u32) -> Result<ProcessMetadata, String> {
        let exe_path = std::fs::read_link(format!("/proc/{}/exe", pid))
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default();
        let binary_missing =
            !exe_path.is_empty() && !std::path::Path::new(&exe_path).exists();
        Ok(ProcessMetadata {
            exe_path,
            company: String::new(),
            description: String::new(),
            version: String::new(),
            elevated: false,
            binary_missing,
            signed: None,
        })
    }

    pub fn list_modules(_pid: u32) -> Result<Vec<super::ModuleInfo>, String> {
        Err("Process module enumeration is only supported on Windows".to_string())
    }

    pub fn list_drivers() -> Result<Vec<super::DriverInfo>, String> {
        Err("Driver enumeration is only supported on Windows".to_string())
    }
}
