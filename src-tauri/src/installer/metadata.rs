//! Only the ordinary user's two fixed NSIS namespaces and owned app links are backed up.
use super::record::{KeyBackup, Result, Value};
use std::path::PathBuf;
use windows::{
    Win32::{
        Foundation::*,
        System::{Com::CoTaskMemFree, Registry::*},
        UI::Shell::*,
    },
    core::{PCWSTR, PWSTR},
};

pub fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}
pub fn folder(id: &windows::core::GUID) -> Result<PathBuf> {
    unsafe {
        let path =
            SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).map_err(|_| "installerUserFolder")?;
        let value = path.to_string().map_err(|_| "installerUserFolder");
        CoTaskMemFree(Some(path.0.cast()));
        Ok(PathBuf::from(value?))
    }
}
pub fn keys(product: &str, product_key: &str) -> Result<[String; 2]> {
    let parts = product_key.split('\\').collect::<Vec<_>>();
    if parts.len() != 3 || parts[0] != "Software" || parts[1] != "sayweave" || parts[2] != product {
        return Err("installerRegistryNamespace".into());
    }
    Ok([
        product_key.into(),
        format!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{product}"),
    ])
}
fn open(path: &str, write: bool) -> Result<Option<Key>> {
    let mut key = HKEY::default();
    let result = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(wide(path).as_ptr()),
            Some(0),
            if write {
                KEY_READ | KEY_WRITE
            } else {
                KEY_READ
            },
            &mut key,
        )
    };
    if result == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    result.ok().map_err(|_| "installerRegistryRead")?;
    Ok(Some(Key(key)))
}
pub fn snapshot(path: &str) -> Result<KeyBackup> {
    let Some(key) = open(path, false)? else {
        return Ok(KeyBackup {
            path: path.into(),
            existed: false,
            values: Vec::new(),
        });
    };
    let mut child = [0u16; 256];
    let mut child_len = child.len() as u32;
    let child_result = unsafe {
        RegEnumKeyExW(
            key.0,
            0,
            Some(PWSTR(child.as_mut_ptr())),
            &mut child_len,
            None,
            None,
            None,
            None,
        )
    };
    if child_result != ERROR_NO_MORE_ITEMS {
        return Err("installerRegistrySubkey".into());
    }
    let mut values = Vec::new();
    for index in 0..64 {
        let mut name = [0u16; 256];
        let mut length = name.len() as u32;
        let mut kind = REG_VALUE_TYPE::default();
        let mut size = 0;
        let result = unsafe {
            RegEnumValueW(
                key.0,
                index,
                Some(PWSTR(name.as_mut_ptr())),
                &mut length,
                None,
                Some(&mut kind.0),
                None,
                Some(&mut size),
            )
        };
        if result == ERROR_NO_MORE_ITEMS {
            return Ok(KeyBackup {
                path: path.into(),
                existed: true,
                values,
            });
        }
        result.ok().map_err(|_| "installerRegistryRead")?;
        if size > 16384 || !matches!(kind, REG_SZ | REG_DWORD | REG_EXPAND_SZ) {
            return Err("installerRegistryValue".into());
        }
        let name =
            String::from_utf16(&name[..length as usize]).map_err(|_| "installerRegistryValue")?;
        let mut bytes = vec![0; size as usize];
        let mut actual = size;
        unsafe {
            RegQueryValueExW(
                key.0,
                PCWSTR(wide(&name).as_ptr()),
                None,
                Some(&mut kind),
                Some(bytes.as_mut_ptr()),
                Some(&mut actual),
            )
            .ok()
            .map_err(|_| "installerRegistryRead")?;
        }
        if actual != size {
            return Err("installerRegistryChanged".into());
        }
        values.push(Value {
            name,
            kind: kind.0,
            bytes,
        });
    }
    Err("installerRegistryLimit".into())
}
pub fn install_location(key: &KeyBackup) -> Result<Option<String>> {
    let Some(value) = key.values.iter().find(|v| v.name == "InstallLocation") else {
        return Ok(None);
    };
    if value.kind != REG_SZ.0 || value.bytes.len() % 2 != 0 {
        return Err("installerRegistryValue".into());
    }
    let chars = value
        .bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|v| u16::from_le_bytes([v[0], v[1]]))
        .collect::<Vec<_>>();
    Ok(Some(
        String::from_utf16(&chars)
            .map_err(|_| "installerRegistryValue")?
            .trim_end_matches('\0')
            .trim_matches('"')
            .into(),
    ))
}
pub fn restore(snapshot: &KeyBackup, install_dir: &str) -> Result<()> {
    let current = self::snapshot(&snapshot.path)?;
    let before_names = snapshot
        .values
        .iter()
        .map(|v| v.name.as_str())
        .collect::<Vec<_>>();
    let allowed = [
        "",
        "DisplayName",
        "DisplayIcon",
        "DisplayVersion",
        "Publisher",
        "InstallLocation",
        "UninstallString",
        "NoModify",
        "NoRepair",
        "EstimatedSize",
        "URLInfoAbout",
        "URLUpdateInfo",
        "HelpLink",
        "MainBinaryName",
        "Installer Language",
        "currentUser",
        "perMachine",
    ];
    if current
        .values
        .iter()
        .any(|v| !allowed.contains(&v.name.as_str()) && !before_names.contains(&v.name.as_str()))
    {
        return Err("installerRegistryChanged".into());
    }
    if let Some(location) = install_location(&current)?
        && !same_path(&location, install_dir)
    {
        return Err("installerRegistryChanged".into());
    }
    let mut raw = HKEY::default();
    unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(wide(&snapshot.path).as_ptr()),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE,
            None,
            &mut raw,
            None,
        )
        .ok()
        .map_err(|_| "installerRegistryWrite")?;
    }
    let key = Key(raw);
    for value in &current.values {
        if !snapshot.values.iter().any(|v| v.name == value.name) {
            unsafe {
                RegDeleteValueW(key.0, PCWSTR(wide(&value.name).as_ptr()))
                    .ok()
                    .map_err(|_| "installerRegistryWrite")?;
            }
        }
    }
    for value in &snapshot.values {
        unsafe {
            RegSetValueExW(
                key.0,
                PCWSTR(wide(&value.name).as_ptr()),
                Some(0),
                REG_VALUE_TYPE(value.kind),
                Some(&value.bytes),
            )
            .ok()
            .map_err(|_| "installerRegistryWrite")?;
        }
    }
    drop(key);
    if !snapshot.existed {
        unsafe {
            RegDeleteKeyW(HKEY_CURRENT_USER, PCWSTR(wide(&snapshot.path).as_ptr()))
                .ok()
                .map_err(|_| "installerRegistryWrite")?;
        }
    }
    let after = self::snapshot(&snapshot.path)?;
    if after.existed != snapshot.existed
        || after.values.len() != snapshot.values.len()
        || snapshot.values.iter().any(|v| {
            !after
                .values
                .iter()
                .any(|a| a.name == v.name && a.kind == v.kind && a.bytes == v.bytes)
        })
    {
        return Err("installerRegistryRestore".into());
    }
    Ok(())
}
pub fn same_path(a: &str, b: &str) -> bool {
    let equal = |a: &str, b: &str| {
        a.trim_start_matches("\\\\?\\")
            .trim_end_matches('\\')
            .eq_ignore_ascii_case(b.trim_start_matches("\\\\?\\").trim_end_matches('\\'))
    };
    if equal(a, b) {
        return true;
    }
    // ShellLink can return an 8.3 parent on Windows runner volumes. Resolve
    // existing directory identities, including when the owned EXE is absent.
    let normalized = |value: &str| -> Option<PathBuf> {
        let path = std::path::Path::new(value);
        super::record::reject_links(path).ok()?;
        std::fs::canonicalize(path).ok().or_else(|| {
            Some(
                std::fs::canonicalize(path.parent()?)
                    .ok()?
                    .join(path.file_name()?),
            )
        })
    };
    match (normalized(a), normalized(b)) {
        (Some(a), Some(b)) => equal(&a.to_string_lossy(), &b.to_string_lossy()),
        _ => false,
    }
}
pub fn normalized_directory(path: &std::path::Path) -> Result<PathBuf> {
    super::record::reject_links(path)?;
    if path.exists() {
        if !path.is_dir() {
            return Err("installerDirectory".into());
        }
        return std::fs::canonicalize(path).map_err(|_| "installerDirectory".into());
    }
    let parent = normalized_directory(path.parent().ok_or("installerDirectory")?)?;
    Ok(parent.join(path.file_name().ok_or("installerDirectory")?))
}
/// Shortcut bytes are shell-generated. After a crash, bind their semantic target
/// before removing/restoring only the two enumerated product link paths.
pub fn shortcut_owned(path: &std::path::Path, directory: &str) -> Result<bool> {
    use windows::{Win32::System::Com::*, core::Interface};
    super::record::read(path, 64 * 1024)?;
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
            .map_err(|_| "installerShortcut")?;
        let persist: IPersistFile = link.cast().map_err(|_| "installerShortcut")?;
        persist
            .Load(PCWSTR(wide(&path.to_string_lossy()).as_ptr()), STGM_READ)
            .map_err(|_| "installerShortcut")?;
        let mut target = vec![0u16; 32768];
        let mut args = vec![0u16; 32768];
        link.GetPath(&mut target, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
            .map_err(|_| "installerShortcut")?;
        link.GetArguments(&mut args)
            .map_err(|_| "installerShortcut")?;
        let text = |value: &[u16]| {
            String::from_utf16_lossy(
                &value[..value.iter().position(|v| *v == 0).unwrap_or(value.len())],
            )
        };
        Ok(text(&args).is_empty()
            && same_path(
                &text(&target),
                &PathBuf::from(directory)
                    .join("sayweave.exe")
                    .to_string_lossy(),
            ))
    }
}
pub fn remove_owned_key(before: &KeyBackup, install_dir: &str) -> Result<()> {
    let empty = KeyBackup {
        path: before.path.clone(),
        existed: false,
        values: Vec::new(),
    };
    restore(&empty, install_dir)
}
pub fn current_language() -> Result<u16> {
    unsafe {
        sayweave_text_service::registration::profiles()
            .map_err(|_| "componentQueryFailed")?
            .GetCurrentLanguage()
            .map_err(|_| "componentQueryFailed".into())
    }
}
pub fn initialize_apartment() -> Result<()> {
    unsafe {
        windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_APARTMENTTHREADED,
        )
        .ok()
        .map_err(|_| "componentApartment".into())
    }
}
pub fn user_sid() -> Result<String> {
    sayweave_text_service::transport::current_user_sid().map_err(|_| "installerUserIdentity".into())
}

pub fn uninstall_owned_location(product: &str) -> Result<Option<String>> {
    install_location(&snapshot(&format!(
        "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{product}"
    ))?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shell_generated_shortcut_requires_the_owned_target_and_no_arguments() {
        use windows::{Win32::System::Com::*, core::Interface};
        let root = tempfile::tempdir().unwrap();
        let executable = root.path().join("sayweave.exe");
        std::fs::write(&executable, b"MZfixture").unwrap();
        let path = root.path().join("product.lnk");
        initialize_apartment().unwrap();
        unsafe {
            let link: IShellLinkW =
                CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).unwrap();
            link.SetPath(PCWSTR(wide(&executable.to_string_lossy()).as_ptr()))
                .unwrap();
            let persist: IPersistFile = link.cast().unwrap();
            persist
                .Save(PCWSTR(wide(&path.to_string_lossy()).as_ptr()), true)
                .unwrap();
            assert!(shortcut_owned(&path, &root.path().to_string_lossy()).unwrap());
            let mut short = vec![0u16; 32768];
            let length = windows::Win32::Storage::FileSystem::GetShortPathNameW(
                PCWSTR(wide(&root.path().to_string_lossy()).as_ptr()),
                Some(&mut short),
            );
            assert!(length > 0 && length < (short.len() as u32));
            let short_root = PathBuf::from(String::from_utf16(&short[..length as usize]).unwrap());
            assert_eq!(
                normalized_directory(&short_root).unwrap(),
                normalized_directory(root.path()).unwrap()
            );
            link.SetPath(PCWSTR(
                wide(&short_root.join("sayweave.exe").to_string_lossy()).as_ptr(),
            ))
            .unwrap();
            persist
                .Save(PCWSTR(wide(&path.to_string_lossy()).as_ptr()), true)
                .unwrap();
            assert!(shortcut_owned(&path, &root.path().to_string_lossy()).unwrap());
            link.SetArguments(PCWSTR(wide("--foreign-operation").as_ptr()))
                .unwrap();
            persist
                .Save(PCWSTR(wide(&path.to_string_lossy()).as_ptr()), true)
                .unwrap();
            assert!(!shortcut_owned(&path, &root.path().to_string_lossy()).unwrap());
            drop(persist);
            drop(link);
            CoUninitialize();
        }
    }
    #[test]
    fn registry_backup_namespace_cannot_point_to_a_different_product_or_publisher() {
        assert!(keys("Sayweave", "Software\\sayweave\\Sayweave").is_ok());
        assert!(keys("Sayweave", "Software\\Microsoft\\Sayweave").is_err());
        assert!(keys("Sayweave", "Software\\sayweave\\Other").is_err());
    }
}
