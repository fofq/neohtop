//! Startup item discovery and control
//!
//! Covers the three places a Windows machine actually autostarts things
//! from: the registry Run/RunOnce keys (HKCU, HKLM, HKLM's 32-bit view),
//! the per-user and common Startup folders, and scheduled tasks stored
//! under System32\Tasks (non-Microsoft only — the OS's own tasks are
//! noise for this view).
//!
//! Enable/disable follows Task Manager's own convention: a 12-byte
//! REG_BINARY value under HKCU Explorer\StartupApproved\<bucket> whose
//! first bit marks the item disabled, so NeoHtop's toggles stay in sync
//! with what Task Manager shows. Scheduled tasks are toggled through
//! schtasks. Deletion is a hard remove (registry value / file / task)
//! and always goes through a frontend confirmation first.

use serde::{Deserialize, Serialize};

/// One autostart entry shown in the startup-items panel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartupItem {
    /// Stable identity used for enable/disable/delete, prefixed by kind:
    /// "registry|<root>|<subkey>|<value>", "folder|<path>" or "task|<path>"
    pub id: String,
    /// "registry" | "folder" | "task"
    pub kind: String,
    pub name: String,
    /// Registry value data, folder file name, or task exec command
    pub command: String,
    /// Where the item comes from (key path / folder / task path)
    pub location: String,
    pub enabled: bool,
    /// Raw trigger token for tasks ("logon", "boot", ...), else empty
    pub detail: String,
}

#[cfg(windows)]
mod platform {
    use super::StartupItem;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::os::windows::process::CommandExt;
    use std::path::{Path, PathBuf};
    use windows_sys::Win32::Foundation::{ERROR_MORE_DATA, ERROR_NO_MORE_ITEMS};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegEnumValueW, RegOpenKeyExW,
        RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
        KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE, KEY_WRITE, REG_BINARY, REG_EXPAND_SZ, REG_SZ,
    };

    /// Keeps schtasks console windows from flashing on every query
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// First-byte flag Task Manager writes for a disabled item
    const APPROVED_DISABLED: u8 = 0x03;
    /// First-byte flag Task Manager writes for an (re-)enabled item
    const APPROVED_ENABLED: u8 = 0x02;

    const APPROVED_PREFIX: &str =
        r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";

    /// One registry Run key to scan
    struct RunKey {
        root: HKEY,
        /// Short root label embedded in the item id; "HKLM32" marks the
        /// Wow6432Node view so its StartupApproved bucket is Run32
        root_label: &'static str,
        /// Full subkey path under the root
        sub: &'static str,
    }

    const RUN_KEYS: &[RunKey] = &[
        RunKey {
            root: HKEY_CURRENT_USER,
            root_label: "HKCU",
            sub: r"Software\Microsoft\Windows\CurrentVersion\Run",
        },
        RunKey {
            root: HKEY_CURRENT_USER,
            root_label: "HKCU",
            sub: r"Software\Microsoft\Windows\CurrentVersion\RunOnce",
        },
        RunKey {
            root: HKEY_LOCAL_MACHINE,
            root_label: "HKLM",
            sub: r"Software\Microsoft\Windows\CurrentVersion\Run",
        },
        RunKey {
            root: HKEY_LOCAL_MACHINE,
            root_label: "HKLM",
            sub: r"Software\Microsoft\Windows\CurrentVersion\RunOnce",
        },
        RunKey {
            root: HKEY_LOCAL_MACHINE,
            root_label: "HKLM32",
            sub: r"Software\Wow6432Node\Microsoft\Windows\CurrentVersion\Run",
        },
    ];

    fn to_wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(Some(0)).collect()
    }

    /// Reads a NUL-terminated UTF-16 Win32 string of at most `max` units
    fn from_wide(ptr: *const u16, max: usize) -> String {
        let mut len = 0usize;
        while len < max && unsafe { *ptr.add(len) } != 0 {
            len += 1;
        }
        let slice = unsafe { std::slice::from_raw_parts(ptr, len) };
        OsString::from_wide(slice).to_string_lossy().into_owned()
    }

    /// Interprets a registry value's raw bytes as a displayable command
    fn command_from_value(value_type: u32, data: &[u8]) -> String {
        if (value_type == REG_SZ || value_type == REG_EXPAND_SZ) && data.len() >= 2 {
            // Stop at the NUL terminator (a whole zero u16); pair indexing
            // avoids cross-type slice/array equality comparisons
            let units: Vec<u16> = data
                .chunks_exact(2)
                .take_while(|pair| pair[0] != 0 || pair[1] != 0)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            OsString::from_wide(&units).to_string_lossy().into_owned()
        } else {
            String::new()
        }
    }

    /// Opens a registry key or None (missing keys are normal)
    fn open_key(root: HKEY, sub: &str, access: u32) -> Option<HKEY> {
        let wide = to_wide(sub);
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: wide is NUL-terminated and key receives the handle
        let result = unsafe { RegOpenKeyExW(root, wide.as_ptr(), 0, access, &mut key) };
        if result == 0 {
            Some(key)
        } else {
            None
        }
    }

    /// Reads one registry value's raw bytes
    fn query_value(key: HKEY, name: &str) -> Option<(u32, Vec<u8>)> {
        let wide = to_wide(name);
        let mut value_type = 0u32;
        let mut data = [0u8; 64];
        let mut data_len = data.len() as u32;
        // SAFETY: buffers sized by the len arguments; lpreserved is
        // *const u32 and must be null()
        let result = unsafe {
            RegQueryValueExW(
                key,
                wide.as_ptr(),
                std::ptr::null(),
                &mut value_type,
                data.as_mut_ptr(),
                &mut data_len,
            )
        };
        if result != 0 {
            return None;
        }
        Some((value_type, data[..data_len as usize].to_vec()))
    }

    /// True when Task Manager's StartupApproved marks the item disabled
    fn approved_disabled(approved: HKEY, name: &str) -> bool {
        match query_value(approved, name) {
            Some((REG_BINARY, data)) => data.first().is_some_and(|b| b & 1 == 1),
            _ => false,
        }
    }

    /// Opens one per-item StartupApproved sub-bucket (Run / Run32 /
    /// StartupFolder). Task Manager stores the disable flags in these
    /// sub-buckets — never in the top-level StartupApproved key — so the
    /// read side must look in the same place write_approved writes to.
    fn open_bucket(bucket: &str) -> Option<HKEY> {
        open_key(
            HKEY_CURRENT_USER,
            &format!("{}\\{}", APPROVED_PREFIX, bucket),
            KEY_QUERY_VALUE,
        )
    }

    /// Writes the StartupApproved flag that toggles an item
    fn write_approved(bucket: &str, name: &str, enabled: bool) -> Result<(), String> {
        let path = format!("{}\\{}", APPROVED_PREFIX, bucket);
        let wide_path = to_wide(&path);
        let mut key: HKEY = std::ptr::null_mut();
        // SAFETY: wide_path is NUL-terminated; key receives the handle,
        // created if absent so first-time disabling works
        let result = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide_path.as_ptr(),
                0,
                std::ptr::null(),
                0,
                KEY_WRITE,
                std::ptr::null(),
                &mut key,
                std::ptr::null_mut(),
            )
        };
        if result != 0 {
            return Err(format!(
                "Failed to open StartupApproved key (error {})",
                result
            ));
        }
        let wide_name = to_wide(name);
        let flag = if enabled {
            APPROVED_ENABLED
        } else {
            APPROVED_DISABLED
        };
        let data = [flag, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        // SAFETY: key is valid; data outlives the call
        let result = unsafe {
            RegSetValueExW(
                key,
                wide_name.as_ptr(),
                0,
                REG_BINARY,
                data.as_ptr(),
                data.len() as u32,
            )
        };
        // SAFETY: key is the valid handle from RegCreateKeyExW
        unsafe { RegCloseKey(key) };
        if result != 0 {
            Err(format!(
                "Failed to write StartupApproved value (error {})",
                result
            ))
        } else {
            Ok(())
        }
    }

    fn list_registry(items: &mut Vec<StartupItem>) {
        for key_def in RUN_KEYS {
            let Some(key) = open_key(key_def.root, key_def.sub, KEY_READ) else {
                continue;
            };
            let mut index = 0u32;
            // Enumerate into a local batch so the StartupApproved overlay
            // for this key's bucket can be applied before it merges in.
            let mut batch: Vec<StartupItem> = Vec::new();
            loop {
                let mut name = [0u16; 512];
                let mut name_len = name.len() as u32;
                let mut data = [0u8; 8192];
                let mut data_len = data.len() as u32;
                let mut value_type = 0u32;
                // SAFETY: buffers sized by the len arguments; the
                // reserved parameter is *const u32 and must be null()
                let result = unsafe {
                    RegEnumValueW(
                        key,
                        index,
                        name.as_mut_ptr(),
                        &mut name_len,
                        std::ptr::null(),
                        &mut value_type,
                        data.as_mut_ptr(),
                        &mut data_len,
                    )
                };
                if result == ERROR_NO_MORE_ITEMS {
                    break;
                }
                if result == ERROR_MORE_DATA {
                    // Value larger than the buffer: skip rather than
                    // truncate a path into something misleading
                    index += 1;
                    continue;
                }
                if result != 0 {
                    break;
                }
                let value_name = from_wide(name.as_ptr(), name_len as usize);
                if value_name.is_empty() {
                    index += 1;
                    continue;
                }
                let tail = key_def.sub.rsplit('\\').next().unwrap_or(key_def.sub);
                batch.push(StartupItem {
                    id: format!(
                        "registry|{}|{}|{}",
                        key_def.root_label, key_def.sub, value_name
                    ),
                    kind: "registry".to_string(),
                    name: value_name,
                    command: command_from_value(value_type, &data[..data_len as usize]),
                    location: format!("{}\\…\\{}", key_def.root_label, tail),
                    enabled: true,
                    detail: String::new(),
                });
                index += 1;
            }
            // SAFETY: key is the valid handle from open_key
            unsafe { RegCloseKey(key) };

            // Overlay this key's disable flags. The bucket follows the
            // root exactly as set_enabled's write side does (Run32 for the
            // 32-bit view, Run otherwise); opening the top-level
            // StartupApproved key here would miss every stored flag.
            let bucket = if key_def.root_label == "HKLM32" {
                "Run32"
            } else {
                "Run"
            };
            if let Some(approved) = open_bucket(bucket) {
                for item in batch.iter_mut() {
                    item.enabled = !approved_disabled(approved, &item.name);
                }
                // SAFETY: approved is the valid handle from open_bucket
                unsafe { RegCloseKey(approved) };
            }
            items.extend(batch);
        }
    }

    fn startup_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        if let Ok(appdata) = std::env::var("APPDATA") {
            dirs.push(
                PathBuf::from(&appdata).join(r"Microsoft\Windows\Start Menu\Programs\Startup"),
            );
        }
        if let Ok(programdata) = std::env::var("PROGRAMDATA") {
            dirs.push(
                PathBuf::from(&programdata).join(r"Microsoft\Windows\Start Menu\Programs\Startup"),
            );
        }
        dirs
    }

    fn list_folders(items: &mut Vec<StartupItem>) {
        for dir in startup_dirs() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if name.is_empty() {
                    continue;
                }
                items.push(StartupItem {
                    id: format!("folder|{}", path.to_string_lossy()),
                    kind: "folder".to_string(),
                    name,
                    command: path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                    location: dir.to_string_lossy().into_owned(),
                    enabled: true,
                    detail: String::new(),
                });
            }
        }

        // The StartupFolder flags live in the StartupFolder sub-bucket
        // (keyed by file name); the top-level StartupApproved key holds
        // no per-item values, so querying it always read back as enabled.
        if let Some(approved) = open_bucket("StartupFolder") {
            for item in items.iter_mut() {
                if item.kind == "folder" {
                    // StartupFolder flags are keyed by the file's name
                    item.enabled = !approved_disabled(approved, &item.command);
                }
            }
            // SAFETY: approved is the valid handle from open_bucket
            unsafe { RegCloseKey(approved) };
        }
    }

    /// Decodes task XML: Task Scheduler stores tasks as UTF-16 (LE/BE)
    /// or UTF-8, with or without a BOM
    fn decode_task_file(bytes: &[u8]) -> String {
        if bytes.len() < 2 {
            return String::new();
        }
        if bytes[0] == 0xFF && bytes[1] == 0xFE {
            let units: Vec<u16> = bytes[2..]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            return String::from_utf16_lossy(&units);
        }
        if bytes[0] == 0xFE && bytes[1] == 0xFF {
            let units: Vec<u16> = bytes[2..]
                .chunks_exact(2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            return String::from_utf16_lossy(&units);
        }
        let payload = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
            &bytes[3..]
        } else {
            bytes
        };
        String::from_utf8_lossy(payload).into_owned()
    }

    /// First `<tag>value</tag>` occurrence, attributes tolerated
    fn xml_tag(text: &str, tag: &str) -> Option<String> {
        let open = format!("<{}>", tag);
        let close = format!("</{}>", tag);
        let start = text.find(&open)? + open.len();
        let end = text[start..].find(&close)? + start;
        Some(text[start..end].trim().to_string())
    }

    /// First trigger element name mapped to a raw token the frontend
    /// localizes (startup.trigger.*)
    fn xml_first_trigger(text: &str) -> String {
        const TRIGGERS: &[(&str, &str)] = &[
            ("LogonTrigger", "logon"),
            ("BootTrigger", "boot"),
            ("TimeTrigger", "time"),
            ("CalendarTrigger", "calendar"),
            ("IdleTrigger", "idle"),
            ("SessionStateChangeTrigger", "session"),
            ("EventTrigger", "event"),
            ("RegistrationTrigger", "registration"),
        ];
        for (element, token) in TRIGGERS {
            if text.contains(&format!("<{}", element)) {
                return (*token).to_string();
            }
        }
        String::new()
    }

    // schtasks console output decodes via winbuf::decode_console_output
    use crate::monitoring::winbuf::decode_console_output;

    /// Parses one CSV row ("a","b with ""quotes""",c) into fields
    fn parse_csv_line(line: &str) -> Vec<String> {
        let mut fields = Vec::new();
        let mut current = String::new();
        let mut in_quotes = false;
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if in_quotes {
                if c == '"' {
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        current.push('"');
                    } else {
                        in_quotes = false;
                    }
                } else {
                    current.push(c);
                }
            } else if c == '"' {
                in_quotes = true;
            } else if c == ',' {
                fields.push(current.clone());
                current.clear();
            } else {
                current.push(c);
            }
        }
        fields.push(current);
        fields
    }

    /// Enumerates every scheduled task through `schtasks /query`, which
    /// works without elevation and covers system tasks the raw task files
    /// cannot offer to a non-admin reader. Task files are still read on a
    /// best-effort basis for trigger/command details.
    fn list_tasks(items: &mut Vec<StartupItem>) {
        let system_root =
            std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
        let tasks_root = Path::new(&system_root).join(r"System32\Tasks");

        let output = match std::process::Command::new("schtasks")
            .args(["/query", "/fo", "csv", "/nh"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            Ok(output) => output,
            Err(_) => return,
        };
        let text = decode_console_output(&output.stdout);
        for line in text.lines() {
            let fields = parse_csv_line(line);
            if fields.is_empty() {
                continue;
            }
            // Column layout differs by Windows build: older schtasks emits
            // "host","taskname","next run","status" while Windows 11 24H2+
            // dropped the host column ("taskname","next run","status").
            // The task path is the field starting with a backslash in both
            // layouts, and the status is always the trailing field.
            let task_path = match fields
                .iter()
                .find(|field| field.trim_start().starts_with('\\'))
            {
                Some(field) => field.trim().trim_start_matches('\\').to_string(),
                None => continue,
            };
            if task_path.is_empty() {
                continue;
            }
            let status = fields.last().map(|f| f.trim()).unwrap_or("");
            let disabled_by_status = status.contains("已禁用")
                || status.to_lowercase().contains("disabled");

            // Best-effort XML details (requires a readable task file)
            let (enabled, command, detail) = match std::fs::read(
                tasks_root.join(&task_path),
            ) {
                Ok(bytes) => {
                    let xml = decode_task_file(&bytes);
                    if xml.is_empty() {
                        (!disabled_by_status, String::new(), String::new())
                    } else {
                        let xml_enabled = xml_tag(&xml, "Enabled")
                            .map(|v| v != "false")
                            .unwrap_or(!disabled_by_status);
                        let command = xml_tag(&xml, "Command").unwrap_or_default();
                        let arguments =
                            xml_tag(&xml, "Arguments").unwrap_or_default();
                        let command = if arguments.is_empty() {
                            command
                        } else {
                            format!("{} {}", command, arguments)
                        };
                        (xml_enabled, command, xml_first_trigger(&xml))
                    }
                }
                Err(_) => (!disabled_by_status, String::new(), String::new()),
            };

            let folder = task_path
                .rsplit_once('\\')
                .map(|(parent, _)| parent)
                .unwrap_or("");
            items.push(StartupItem {
                id: format!("task|\\{}", task_path),
                kind: "task".to_string(),
                name: task_path
                    .rsplit('\\')
                    .next()
                    .unwrap_or(&task_path)
                    .to_string(),
                command,
                location: if folder.is_empty() {
                    "\\".to_string()
                } else {
                    format!("\\{}", folder)
                },
                enabled,
                detail,
            });
        }
    }

    pub fn list() -> Vec<StartupItem> {
        let mut items = Vec::new();
        list_registry(&mut items);
        list_folders(&mut items);
        list_tasks(&mut items);
        list_services(&mut items);
        items
    }

    /// Auto-start services (start type "auto") as startup entries, plus
    /// services this panel disabled earlier in the session so they stay
    /// visible and re-enableable.
    fn list_services(items: &mut Vec<StartupItem>) {
        let Ok(services) = crate::monitoring::services::collect() else {
            return;
        };
        let disabled = crate::monitoring::services::disabled_by_panel()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for service in services.iter() {
            let tracked = disabled.contains(&service.name);
            if service.start_type != "auto" && !tracked {
                continue;
            }
            items.push(StartupItem {
                id: format!("service|{}", service.name),
                kind: "service".to_string(),
                name: if service.display_name.is_empty() {
                    service.name.clone()
                } else {
                    service.display_name.clone()
                },
                command: service.binary_path.clone(),
                location: service.name.clone(),
                enabled: service.start_type == "auto",
                detail: String::new(),
            });
        }
    }

    /// Runs schtasks without flashing a console window
    fn run_schtasks(args: &[&str]) -> Result<bool, String> {
        let status = std::process::Command::new("schtasks")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| format!("Failed to run schtasks: {}", e))?;
        Ok(status.success())
    }

    pub fn set_enabled(id: &str, enabled: bool) -> Result<bool, String> {
        let parts: Vec<&str> = id.splitn(4, '|').collect();
        match parts.first().copied() {
            Some("registry") => {
                let [_kind, _root, _sub, value] = parts.as_slice() else {
                    return Err("Invalid startup item id".to_string());
                };
                // The bucket follows the root: the Wow6432Node view is
                // flagged under Run32, everything else under Run
                let bucket = if *_root == "HKLM32" { "Run32" } else { "Run" };
                write_approved(bucket, value, enabled)?;
                Ok(true)
            }
            Some("folder") => {
                let [_kind, path] = parts.as_slice() else {
                    return Err("Invalid startup item id".to_string());
                };
                let file = Path::new(path)
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .ok_or_else(|| "Invalid startup item path".to_string())?;
                write_approved("StartupFolder", &file, enabled)?;
                Ok(true)
            }
            Some("task") => {
                let [_kind, task] = parts.as_slice() else {
                    return Err("Invalid startup item id".to_string());
                };
                let mode = if enabled { "/enable" } else { "/disable" };
                run_schtasks(&["/change", "/tn", task, mode])
            }
            Some("service") => {
                let [_kind, name] = parts.as_slice() else {
                    return Err("Invalid startup item id".to_string());
                };
                // Auto-start services toggle their start type; deletion is
                // not offered for them (uninstall instead)
                crate::monitoring::services::set_start_type(
                    name,
                    if enabled { "auto" } else { "disabled" },
                )
            }
            _ => Err("Invalid startup item id".to_string()),
        }
    }

    pub fn delete(id: &str) -> Result<bool, String> {
        let parts: Vec<&str> = id.splitn(4, '|').collect();
        match parts.first().copied() {
            Some("registry") => {
                let [_kind, root, sub, value] = parts.as_slice() else {
                    return Err("Invalid startup item id".to_string());
                };
                let root_key = match *root {
                    "HKCU" => HKEY_CURRENT_USER,
                    "HKLM" | "HKLM32" => HKEY_LOCAL_MACHINE,
                    _ => return Err("Invalid startup item root".to_string()),
                };
                // The id embeds the full subkey path, so deleting uses it
                // verbatim; HKLM keys need an elevated process
                let Some(key) = open_key(root_key, sub, KEY_SET_VALUE) else {
                    return Err("Startup key no longer exists".to_string());
                };
                let wide = to_wide(value);
                // SAFETY: key is valid, wide is NUL-terminated
                let result = unsafe { RegDeleteValueW(key, wide.as_ptr()) };
                // SAFETY: key is the valid handle from open_key
                unsafe { RegCloseKey(key) };
                if result == 0 {
                    Ok(true)
                } else {
                    Err(format!(
                        "Failed to delete registry value (error {})",
                        result
                    ))
                }
            }
            Some("folder") => {
                let [_kind, path] = parts.as_slice() else {
                    return Err("Invalid startup item id".to_string());
                };
                std::fs::remove_file(path)
                    .map(|_| true)
                    .map_err(|e| format!("Failed to delete file: {}", e))
            }
            Some("task") => {
                let [_kind, task] = parts.as_slice() else {
                    return Err("Invalid startup item id".to_string());
                };
                run_schtasks(&["/delete", "/tn", task, "/f"])
            }
            // Services are never deleted from the startup panel; toggling
            // the start type is the intended (and reversible) action
            Some("service") => Err(
                "Services cannot be deleted from the startup panel; disable them instead"
                    .to_string(),
            ),
            _ => Err("Invalid startup item id".to_string()),
        }
    }
}

#[cfg(not(windows))]
mod platform {
    use super::StartupItem;

    pub fn list() -> Vec<StartupItem> {
        Vec::new()
    }

    pub fn set_enabled(_id: &str, _enabled: bool) -> Result<bool, String> {
        Ok(false)
    }

    pub fn delete(_id: &str) -> Result<bool, String> {
        Ok(false)
    }
}

pub fn list() -> Vec<StartupItem> {
    platform::list()
}

pub fn set_enabled(id: &str, enabled: bool) -> Result<bool, String> {
    platform::set_enabled(id, enabled)
}

pub fn delete(id: &str) -> Result<bool, String> {
    platform::delete(id)
}
