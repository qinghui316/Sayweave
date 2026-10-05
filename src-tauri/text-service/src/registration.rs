//! Registration is separate from activation; only the speech category is activated.
use windows::{
    Win32::{
        Foundation::*,
        Security::*,
        System::{Com::*, Registry::*, Threading::*},
        UI::{Input::KeyboardAndMouse::HKL, Shell::*, TextServices::*},
    },
    core::{GUID, Interface, PCWSTR, w},
};
pub const CLASS: GUID = GUID::from_u128(0xcb473cf0_9b4c_482c_bfc2_5527d2379aca);
pub const PROFILE: GUID = GUID::from_u128(0x6cd26440_e770_471e_8212_b5a565e816f5);
const COM_ROOT: &str = "Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}";
const COM_PATH: &str =
    "Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}\\InprocServer32";
const CONTROL: &str = "Software\\Sayweave\\TextService\\Probe";
const OWNER: &str = "SayweaveStageA-v1";
fn wide(value: &str) -> Vec<u16> {
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
fn key(root: HKEY, path: &str) -> windows::core::Result<Key> {
    unsafe {
        let mut k = HKEY::default();
        RegCreateKeyExW(
            root,
            PCWSTR(wide(path).as_ptr()),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_READ | KEY_WRITE,
            None,
            &mut k,
            None,
        )
        .ok()?;
        Ok(Key(k))
    }
}
fn string(k: &Key, name: &str, value: &str) -> windows::core::Result<()> {
    let bytes: Vec<u8> = wide(value).iter().flat_map(|c| c.to_le_bytes()).collect();
    unsafe {
        RegSetValueExW(
            k.0,
            PCWSTR(wide(name).as_ptr()),
            Some(0),
            REG_SZ,
            Some(&bytes),
        )
        .ok()
    }
}
/// A live lease serializes probes and cannot clear a newer host on drop.
pub struct HostLease {
    pid: u32,
    capability: String,
    mutex: HANDLE,
}
impl Drop for HostLease {
    fn drop(&mut self) {
        unsafe {
            if read_host() == Some((self.pid, self.capability.clone())) {
                let _ = RegDeleteTreeW(HKEY_CURRENT_USER, PCWSTR(wide(CONTROL).as_ptr()));
            }
            let _ = ReleaseMutex(self.mutex);
            let _ = CloseHandle(self.mutex);
        }
    }
}
pub fn host_capability(pid: u32, capability: &str) -> windows::core::Result<HostLease> {
    unsafe {
        let name = wide(&format!(
            "Local\\Sayweave.TSF.Host.{}",
            crate::transport::current_user_sid()?
        ));
        let mutex = CreateMutexW(None, false, PCWSTR(name.as_ptr()))?;
        if !matches!(
            WaitForSingleObject(mutex, 0),
            WAIT_OBJECT_0 | WAIT_ABANDONED
        ) {
            let _ = CloseHandle(mutex);
            return Err(windows::core::Error::new(
                E_ACCESSDENIED,
                "hostAlreadyActive",
            ));
        }
        let lease = HostLease {
            pid,
            capability: capability.into(),
            mutex,
        };
        let k = key(HKEY_CURRENT_USER, CONTROL)?;
        let value = serde_json::to_vec(&(pid, capability))
            .map_err(|_| windows::core::Error::from_hresult(E_INVALIDARG))?;
        RegSetValueExW(k.0, w!("Host"), Some(0), REG_BINARY, Some(&value)).ok()?;
        Ok(lease)
    }
}
pub fn read_host() -> Option<(u32, String)> {
    unsafe {
        let mut data = [0u8; 128];
        let mut size = data.len() as u32;
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(wide(CONTROL).as_ptr()),
            w!("Host"),
            RRF_RT_REG_BINARY,
            None,
            Some(data.as_mut_ptr().cast()),
            Some(&mut size),
        )
        .ok()
        .ok()?;
        let value: (u32, String) = serde_json::from_slice(&data[..size as usize]).ok()?;
        (value.0 != 0 && uuid::Uuid::parse_str(&value.1).is_ok()).then_some(value)
    }
}
pub fn profiles() -> windows::core::Result<ITfInputProcessorProfiles> {
    unsafe { CoCreateInstance(&CLSID_TF_InputProcessorProfiles, None, CLSCTX_INPROC_SERVER) }
}
pub fn register(path: &str) -> windows::core::Result<()> {
    register_scope(path, HKEY_CURRENT_USER)
}
/// Explicit operator-only installation; never called by the renderer or during startup.
pub fn register_machine(path: &str) -> windows::core::Result<()> {
    administrator()?;
    let canonical = std::fs::canonicalize(path)
        .map_err(|_| windows::core::Error::new(E_INVALIDARG, "missingModule"))?;
    let protected = unsafe {
        let value = SHGetKnownFolderPath(&FOLDERID_ProgramFiles, KF_FLAG_DEFAULT, None)?;
        let path = value.to_string();
        CoTaskMemFree(Some(value.0.cast()));
        std::path::PathBuf::from(path?)
            .join("Sayweave")
            .join("TextService")
    };
    let protected = std::fs::canonicalize(protected)
        .map_err(|_| windows::core::Error::new(E_INVALIDARG, "missingProtectedDirectory"))?;
    if !canonical.starts_with(protected) {
        return Err(windows::core::Error::new(
            E_ACCESSDENIED,
            "unprotectedModulePath",
        ));
    }
    register_scope(&canonical.to_string_lossy(), HKEY_LOCAL_MACHINE)
}
fn administrator() -> windows::core::Result<()> {
    unsafe {
        let mut sid = [0usize; SECURITY_MAX_SID_SIZE as usize / std::mem::size_of::<usize>() + 1];
        let mut size = std::mem::size_of_val(&sid) as u32;
        CreateWellKnownSid(
            WinBuiltinAdministratorsSid,
            None,
            Some(PSID(sid.as_mut_ptr().cast())),
            &mut size,
        )?;
        let mut member = windows::core::BOOL::default();
        CheckTokenMembership(None, PSID(sid.as_ptr() as *mut _), &mut member)?;
        if !member.as_bool() {
            return Err(windows::core::Error::new(
                E_ACCESSDENIED,
                "installationRequiresAdministrator",
            ));
        }
        Ok(())
    }
}
fn register_scope(path: &str, root: HKEY) -> windows::core::Result<()> {
    unsafe {
        // Fresh-install only: never overwrite or roll back an existing installation.
        for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for path in [
                COM_ROOT,
                "Software\\Microsoft\\CTF\\TIP\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}",
            ] {
                if exists(root, path)? {
                    return Err(windows::core::Error::new(
                        E_ACCESSDENIED,
                        "existingRegistration",
                    ));
                }
            }
        }
        let profile = profiles()?;
        let category: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        let language = profile.GetCurrentLanguage()?;
        let mut registered = false;
        let mut category_registered = false;
        let result = (|| {
            let k = key(root, COM_PATH)?;
            string(&k, "SayweaveOwner", OWNER)?;
            string(&k, "", path)?;
            string(&k, "ThreadingModel", "Apartment")?;
            profile
                .Register(&CLASS)
                .map_err(|e| windows::core::Error::new(e.code(), "profilesRegister"))?;
            registered = true;
            let desc: Vec<u16> = "Sayweave Voice".encode_utf16().collect();
            let icon: Vec<u16> = path.encode_utf16().collect();
            profile
                .AddLanguageProfile(&CLASS, language, &PROFILE, &desc, &icon, 0)
                .map_err(|e| windows::core::Error::new(e.code(), "addLanguageProfile"))?;
            category
                .RegisterCategory(&CLASS, &GUID_TFCAT_TIP_SPEECH, &CLASS)
                .map_err(|e| windows::core::Error::new(e.code(), "speechCategory"))?;
            category_registered = true;
            if root == HKEY_LOCAL_MACHINE {
                profile
                    .EnableLanguageProfileByDefault(&CLASS, language, &PROFILE, false)
                    .map_err(|e| windows::core::Error::new(e.code(), "disableDefaultEnablement"))?;
            }
            profile
                .EnableLanguageProfile(&CLASS, language, &PROFILE, true)
                .map_err(|e| windows::core::Error::new(e.code(), "enableProfile"))?;
            Ok(())
        })();
        if result.is_err() {
            let category_cleanup = if category_registered {
                category.UnregisterCategory(&CLASS, &GUID_TFCAT_TIP_SPEECH, &CLASS)
            } else {
                Ok(())
            };
            let profile_cleanup = if registered
                || exists(
                    HKEY_LOCAL_MACHINE,
                    "Software\\Microsoft\\CTF\\TIP\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}",
                )? {
                profile.Unregister(&CLASS)
            } else {
                Ok(())
            };
            if category_cleanup.is_err() || profile_cleanup.is_err() {
                // Keep the owned COM path and protected binary for an explicit retry.
                return Err(windows::core::Error::new(
                    E_FAIL,
                    "registrationRollbackIncomplete",
                ));
            }
            let status = RegDeleteTreeW(root, PCWSTR(wide(COM_ROOT).as_ptr()));
            if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
                return Err(windows::core::Error::new(
                    status.to_hresult(),
                    "registrationRollbackFailed",
                ));
            }
        }
        result
    }
}
pub fn activate(session: bool) -> windows::core::Result<u16> {
    unsafe {
        let p = profiles()?;
        let language = p.GetCurrentLanguage()?;
        let manager: ITfInputProcessorProfileMgr = p.cast()?;
        manager.ActivateProfile(
            TF_PROFILETYPE_INPUTPROCESSOR,
            language,
            &CLASS,
            &PROFILE,
            HKL::default(),
            if session {
                TF_IPPMF_FORSESSION
            } else {
                TF_IPPMF_FORPROCESS
            },
        )?;
        Ok(language)
    }
}
pub fn deactivate(language: u16, session: bool) -> windows::core::Result<()> {
    unsafe {
        let manager: ITfInputProcessorProfileMgr = profiles()?.cast()?;
        manager.DeactivateProfile(
            TF_PROFILETYPE_INPUTPROCESSOR,
            language,
            &CLASS,
            &PROFILE,
            HKL::default(),
            if session {
                TF_IPPMF_FORSESSION
            } else {
                TF_IPPMF_FORPROCESS
            },
        )
    }
}
pub fn unregister() -> windows::core::Result<()> {
    unregister_scope(HKEY_CURRENT_USER)
}
pub fn unregister_machine() -> windows::core::Result<()> {
    administrator()?;
    unregister_scope(HKEY_LOCAL_MACHINE)
}
fn unregister_scope(root: HKEY) -> windows::core::Result<()> {
    unsafe {
        if !exists(root, COM_ROOT)? {
            return Ok(());
        }
        let mut owner = [0u16; 64];
        let mut bytes = std::mem::size_of_val(&owner) as u32;
        RegGetValueW(
            root,
            PCWSTR(wide(COM_PATH).as_ptr()),
            w!("SayweaveOwner"),
            RRF_RT_REG_SZ,
            None,
            Some(owner.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
        .ok()?;
        let end = owner
            .iter()
            .position(|c| *c == 0)
            .ok_or_else(|| windows::core::Error::from_hresult(E_ACCESSDENIED))?;
        if String::from_utf16_lossy(&owner[..end]) != OWNER {
            return Err(windows::core::Error::new(
                E_ACCESSDENIED,
                "registrationOwnerMismatch",
            ));
        }
        // Attempt every cleanup even when one COM service is unavailable.
        let category = (|| {
            let category: ITfCategoryMgr =
                CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
            category.UnregisterCategory(&CLASS, &GUID_TFCAT_TIP_SPEECH, &CLASS)
        })();
        let profile = profiles().and_then(|p| p.Unregister(&CLASS));
        // Keep COM registration until TSF removal succeeds, so cleanup can be retried.
        category?;
        profile?;
        let status = RegDeleteTreeW(root, PCWSTR(wide(COM_ROOT).as_ptr()));
        if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
            status.ok()?;
        }
        Ok(())
    }
}

fn exists(root: HKEY, path: &str) -> windows::core::Result<bool> {
    unsafe {
        let mut key = HKEY::default();
        let status = RegOpenKeyExW(root, PCWSTR(wide(path).as_ptr()), None, KEY_READ, &mut key);
        match status {
            ERROR_SUCCESS => {
                let _ = RegCloseKey(key);
                Ok(true)
            }
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => Ok(false),
            _ => {
                status.ok()?;
                Ok(false)
            }
        }
    }
}
