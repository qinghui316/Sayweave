//! Per-user logon registration. Only the ordinary, committed installer writes it.
use std::path::Path;

pub(crate) const MARKER: &str = "LoginStartupInitialized";
pub(crate) const WARNING: &str = "installerStartupWarning";
const ARGUMENT: &str = "--autostart";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum LaunchReason {
    #[default]
    Manual,
    Autostart,
}
impl LaunchReason {
    pub(crate) fn current() -> Self {
        Self::from_arguments(std::env::args().skip(1))
    }
    fn from_arguments(args: impl IntoIterator<Item = String>) -> Self {
        if args.into_iter().any(|arg| arg == ARGUMENT) {
            Self::Autostart
        } else {
            Self::Manual
        }
    }
    pub(crate) fn main_visible(self) -> bool {
        self == Self::Manual
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use crate::installer::metadata;
    use windows::{
        Win32::{Foundation::*, System::Registry::*, UI::Shell::CommandLineToArgvW},
        core::{PCWSTR, PWSTR},
    };
    type Result<T> = std::result::Result<T, String>;
    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    struct Key(HKEY);
    impl Drop for Key {
        fn drop(&mut self) {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }
    fn error(stage: &str, code: WIN32_ERROR) -> String {
        format!("startup{stage}:{}", code.0)
    }
    fn open(path: &str, write: bool) -> Result<Option<Key>> {
        let mut raw = HKEY::default();
        let access = if write {
            KEY_QUERY_VALUE | KEY_SET_VALUE
        } else {
            KEY_QUERY_VALUE
        };
        let result = unsafe {
            RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(metadata::wide(path).as_ptr()),
                Some(0),
                access,
                &mut raw,
            )
        };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        result.ok().map_err(|_| error("Open", result))?;
        Ok(Some(Key(raw)))
    }
    fn create(path: &str) -> Result<Key> {
        let mut raw = HKEY::default();
        let result = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(metadata::wide(path).as_ptr()),
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                None,
                &mut raw,
                None,
            )
        };
        result.ok().map_err(|_| error("Create", result))?;
        Ok(Key(raw))
    }
    fn query(path: &str, name: &str) -> Result<Option<(REG_VALUE_TYPE, Vec<u8>)>> {
        let Some(key) = open(path, false)? else {
            return Ok(None);
        };
        let mut kind = REG_VALUE_TYPE::default();
        let mut size = 0;
        let result = unsafe {
            RegQueryValueExW(
                key.0,
                PCWSTR(metadata::wide(name).as_ptr()),
                None,
                Some(&mut kind),
                None,
                Some(&mut size),
            )
        };
        if result == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        result.ok().map_err(|_| error("Read", result))?;
        if size > 4096 {
            return Err("startupValueLimit".into());
        }
        let mut bytes = vec![0; size as usize];
        let mut actual = size;
        let first_kind = kind;
        let result = unsafe {
            RegQueryValueExW(
                key.0,
                PCWSTR(metadata::wide(name).as_ptr()),
                None,
                Some(&mut kind),
                Some(bytes.as_mut_ptr()),
                Some(&mut actual),
            )
        };
        result.ok().map_err(|_| error("Read", result))?;
        if actual != size || kind != first_kind {
            return Err("startupValueChanged".into());
        }
        Ok(Some((kind, bytes)))
    }
    fn set(path: &str, name: &str, kind: REG_VALUE_TYPE, bytes: &[u8]) -> Result<()> {
        let key = create(path)?;
        let result = unsafe {
            RegSetValueExW(
                key.0,
                PCWSTR(metadata::wide(name).as_ptr()),
                Some(0),
                kind,
                Some(bytes),
            )
        };
        result.ok().map_err(|_| error("Write", result))
    }
    fn decode_string(value: (REG_VALUE_TYPE, Vec<u8>)) -> Result<String> {
        if value.0 != REG_SZ || !value.1.len().is_multiple_of(2) {
            return Err("startupValueType".into());
        }
        let mut wide = value
            .1
            .as_chunks::<2>()
            .0
            .iter()
            .map(|v| u16::from_le_bytes([v[0], v[1]]))
            .collect::<Vec<_>>();
        if wide.pop() != Some(0) || wide.contains(&0) {
            return Err("startupValueType".into());
        }
        String::from_utf16(&wide).map_err(|_| "startupValueType".into())
    }
    fn command(directory: &Path) -> Result<String> {
        let exe = directory.join("sayweave.exe");
        let path = exe.to_str().ok_or("startupPath")?;
        if !exe.is_absolute() || path.contains(['"', '\0', '\r', '\n']) {
            return Err("startupPath".into());
        }
        let command = format!("\"{path}\" {ARGUMENT}");
        if command.encode_utf16().count() > 260 {
            return Err("startupCommandLimit".into());
        }
        Ok(command)
    }
    fn owns(value: &str, directory: &Path) -> bool {
        if value.contains('\0') {
            return false;
        }
        unsafe {
            let mut count = 0;
            let args = CommandLineToArgvW(PCWSTR(metadata::wide(value).as_ptr()), &mut count);
            if args.is_null() {
                return false;
            }
            let result = if count == 1 || count == 2 {
                let values = std::slice::from_raw_parts(args, count as usize);
                let startup_argument = count == 1
                    || PWSTR(values[1].0)
                        .to_string()
                        .is_ok_and(|arg| arg == ARGUMENT);
                match PWSTR(values[0].0).to_string() {
                    Ok(exe) => {
                        startup_argument
                            && metadata::same_path(
                                &exe,
                                &directory.join("sayweave.exe").to_string_lossy(),
                            )
                    }
                    _ => false,
                }
            } else {
                false
            };
            let _ = LocalFree(Some(HLOCAL(args.cast())));
            result
        }
    }
    trait Store {
        fn initialized(&mut self) -> Result<bool>;
        fn entry(&mut self) -> Result<Option<String>>;
        fn write_entry(&mut self, command: &str) -> Result<()>;
        fn mark(&mut self) -> Result<()>;
        fn remove(&mut self) -> Result<()>;
    }
    fn initialize(store: &mut impl Store, directory: &Path) -> Result<()> {
        // A completed marker also preserves a user-removed entry. Never resurrect it.
        if store.initialized()? {
            return Ok(());
        }
        match store.entry()? {
            Some(value) if owns(&value, directory) => {}
            Some(_) => return Err("startupConflict".into()),
            None => {
                let expected = command(directory)?;
                store.write_entry(&expected)?;
                if store.entry()?.as_deref() != Some(&expected) {
                    return Err("startupWriteUnconfirmed".into());
                }
            }
        }
        store.mark()
    }
    fn remove_owned(store: &mut impl Store, directory: &Path) -> Result<()> {
        if let Some(value) = store.entry()?
            && owns(&value, directory)
        {
            store.remove()?;
            if store.entry()?.is_some() {
                return Err("startupRemoveUnconfirmed".into());
            }
        }
        Ok(())
    }
    struct Registry<'a> {
        run: &'a str,
        product: &'a str,
        name: &'a str,
    }
    impl Store for Registry<'_> {
        fn initialized(&mut self) -> Result<bool> {
            match query(self.product, MARKER)? {
                None => Ok(false),
                Some((REG_DWORD, bytes)) if bytes == 1u32.to_le_bytes() => Ok(true),
                _ => Err("startupMarkerInvalid".into()),
            }
        }
        fn entry(&mut self) -> Result<Option<String>> {
            query(self.run, self.name)?.map(decode_string).transpose()
        }
        fn write_entry(&mut self, value: &str) -> Result<()> {
            let bytes = metadata::wide(value)
                .iter()
                .flat_map(|v| v.to_le_bytes())
                .collect::<Vec<_>>();
            set(self.run, self.name, REG_SZ, &bytes)
        }
        fn mark(&mut self) -> Result<()> {
            set(self.product, MARKER, REG_DWORD, &1u32.to_le_bytes())?;
            if !self.initialized()? {
                return Err("startupMarkerUnconfirmed".into());
            }
            Ok(())
        }
        fn remove(&mut self) -> Result<()> {
            let Some(key) = open(self.run, true)? else {
                return Ok(());
            };
            let result =
                unsafe { RegDeleteValueW(key.0, PCWSTR(metadata::wide(self.name).as_ptr())) };
            if result == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            result.ok().map_err(|_| error("Delete", result))
        }
    }
    fn production() -> bool {
        crate::build_info::current().channel == "stable"
            && env!("SAYWEAVE_BUILD_PRODUCT") == "Sayweave"
    }
    pub(crate) fn install(directory: &Path, product: &str) -> Result<()> {
        if !production() {
            return Ok(());
        }
        // The caller already verified committed installation, initiator and product namespace.
        initialize(
            &mut Registry {
                run: RUN,
                product,
                name: "Sayweave",
            },
            directory,
        )
    }
    pub(crate) fn uninstall(directory: &Path, product: &str) -> Result<()> {
        if !production() {
            return Ok(());
        }
        remove_owned(
            &mut Registry {
                run: RUN,
                product,
                name: "Sayweave",
            },
            directory,
        )
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[derive(Default)]
        struct Memory {
            marker: bool,
            entry: Option<String>,
            writes: usize,
            deleted: usize,
            fail_write: bool,
            fail_mark: bool,
        }
        impl Store for Memory {
            fn initialized(&mut self) -> Result<bool> {
                Ok(self.marker)
            }
            fn entry(&mut self) -> Result<Option<String>> {
                Ok(self.entry.clone())
            }
            fn write_entry(&mut self, value: &str) -> Result<()> {
                if self.fail_write {
                    return Err("accessDenied:5".into());
                }
                self.entry = Some(value.into());
                self.writes += 1;
                Ok(())
            }
            fn mark(&mut self) -> Result<()> {
                if self.fail_mark {
                    return Err("markerDenied:5".into());
                }
                self.marker = true;
                Ok(())
            }
            fn remove(&mut self) -> Result<()> {
                self.entry = None;
                self.deleted += 1;
                Ok(())
            }
        }
        #[test]
        fn first_install_and_upgrade_initialize_once() {
            let dir = Path::new(r"C:\自有测试 目录\Sayweave");
            let mut store = Memory::default();
            initialize(&mut store, dir).unwrap();
            assert!(store.marker);
            assert_eq!(store.writes, 1);
            initialize(&mut store, dir).unwrap();
            assert_eq!(store.writes, 1);
            store.entry = None;
            initialize(&mut store, dir).unwrap();
            assert!(store.entry.is_none());
            assert_eq!(store.writes, 1);
        }
        #[test]
        fn interrupted_marker_only_is_repaired_without_rewriting_command() {
            let dir = Path::new(r"C:\Sayweave");
            let mut store = Memory {
                fail_mark: true,
                ..Default::default()
            };
            assert_eq!(initialize(&mut store, dir).unwrap_err(), "markerDenied:5");
            assert_eq!(store.writes, 1);
            assert!(!store.marker);
            store.fail_mark = false;
            initialize(&mut store, dir).unwrap();
            assert_eq!(store.writes, 1);
            assert!(store.marker);
        }
        #[test]
        fn preexisting_plain_launch_preserves_command_and_only_marks() {
            let dir = Path::new(r"C:\Sayweave");
            let entry = format!("\"{}\"", dir.join("sayweave.exe").display());
            let mut store = Memory {
                entry: Some(entry.clone()),
                ..Default::default()
            };
            initialize(&mut store, dir).unwrap();
            assert!(store.marker);
            assert_eq!(store.entry.as_deref(), Some(entry.as_str()));
            assert_eq!(store.writes, 0);
            remove_owned(&mut store, dir).unwrap();
            assert_eq!(store.deleted, 1);
        }
        #[test]
        fn conflicting_command_and_denied_write_are_preserved() {
            let dir = Path::new(r"C:\Sayweave");
            let foreign = "\"C:\\Other\\app.exe\" --autostart";
            let mut store = Memory {
                entry: Some(foreign.into()),
                ..Default::default()
            };
            assert_eq!(initialize(&mut store, dir).unwrap_err(), "startupConflict");
            assert_eq!(store.writes, 0);
            assert!(!store.marker);
            remove_owned(&mut store, dir).unwrap();
            assert_eq!(store.deleted, 0);
            let mut denied = Memory {
                fail_write: true,
                ..Default::default()
            };
            assert_eq!(initialize(&mut denied, dir).unwrap_err(), "accessDenied:5");
            assert!(!denied.marker);
            assert!(denied.entry.is_none());
        }
        #[test]
        fn uninstall_removes_only_exact_command_owner_and_argument() {
            let dir = Path::new(r"C:\Sayweave");
            let mut store = Memory {
                entry: Some(command(dir).unwrap()),
                ..Default::default()
            };
            remove_owned(&mut store, dir).unwrap();
            assert_eq!(store.deleted, 1);
            store.entry = Some(format!("{} --other", command(dir).unwrap()));
            remove_owned(&mut store, dir).unwrap();
            assert_eq!(store.deleted, 1);
        }
        #[test]
        fn paths_are_quoted_and_limits_not_truncated() {
            let dir = Path::new(r"C:\中文 空格\Sayweave");
            let value = command(dir).unwrap();
            assert!(owns(&value, dir));
            assert!(value.starts_with('"'));
            assert!(command(Path::new("relative")).is_err());
            assert!(command(Path::new(&format!(r"C:\{}", "a".repeat(260)))).is_err());
        }
        #[test]
        fn native_registry_roundtrip_keeps_unrelated_values_and_validates_marker() {
            let id = uuid::Uuid::new_v4();
            let root = format!(r"Software\SayweaveTests\LoginStartup\{id}");
            struct Cleanup(String);
            impl Drop for Cleanup {
                fn drop(&mut self) {
                    unsafe {
                        let _ = RegDeleteTreeW(
                            HKEY_CURRENT_USER,
                            PCWSTR(metadata::wide(&self.0).as_ptr()),
                        );
                    }
                }
            }
            let _cleanup = Cleanup(root.clone());
            let run = format!(r"{root}\Run");
            let product = format!(r"{root}\Product");
            let mut store = Registry {
                run: &run,
                product: &product,
                name: "OwnedTest",
            };
            let dir = Path::new(r"C:\自有启动 测试");
            set(&run, "Unrelated", REG_DWORD, &7u32.to_le_bytes()).unwrap();
            initialize(&mut store, dir).unwrap();
            assert!(store.initialized().unwrap());
            assert!(owns(&store.entry().unwrap().unwrap(), dir));
            remove_owned(&mut store, dir).unwrap();
            initialize(&mut store, dir).unwrap();
            assert!(store.entry().unwrap().is_none());
            assert_eq!(
                query(&run, "Unrelated").unwrap().unwrap().1,
                7u32.to_le_bytes()
            );
            set(&product, MARKER, REG_DWORD, &2u32.to_le_bytes()).unwrap();
            assert!(store.initialized().is_err());
            set(&run, "OwnedTest", REG_BINARY, &[1, 2, 3]).unwrap();
            assert!(store.entry().is_err());
            set(&product, MARKER, REG_DWORD, &1u32.to_le_bytes()).unwrap();
            // Completion must not even reinterpret a user-replaced/removed startup value.
            initialize(&mut store, dir).unwrap();
            assert_eq!(query(&run, "OwnedTest").unwrap().unwrap().1, vec![1, 2, 3]);
        }
    }
}

#[cfg(windows)]
pub(super) use native::{install, uninstall};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_exact_private_flag_selects_hidden_launch() {
        assert!(LaunchReason::from_arguments(Vec::<String>::new()).main_visible());
        assert!(!LaunchReason::from_arguments(["--autostart".into()]).main_visible());
        assert!(LaunchReason::from_arguments(["--autostart-other".into()]).main_visible());
    }
}
