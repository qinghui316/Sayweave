//! Registration is separate from activation; only the speech category is activated.
use windows::{
    Win32::{
        Foundation::*,
        Security::*,
        System::{Com::*, Registry::*, Threading::*},
        UI::{Input::KeyboardAndMouse::HKL, Shell::*, TextServices::*},
    },
    core::{GUID, Interface, PCWSTR, PWSTR, w},
};
pub const CLASS: GUID = GUID::from_u128(0xcb473cf0_9b4c_482c_bfc2_5527d2379aca);
pub const PROFILE: GUID = GUID::from_u128(0x6cd26440_e770_471e_8212_b5a565e816f5);
const COM_ROOT: &str = "Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}";
const COM_PATH: &str =
    "Software\\Classes\\CLSID\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}\\InprocServer32";
const OWNER: &str = "SayweaveStageA-v1";
const TIP_ROOT: &str = "Software\\Microsoft\\CTF\\TIP\\{CB473CF0-9B4C-482C-BFC2-5527D2379ACA}";
const PROFILE_KEY: &str = "{6CD26440-E770-471E-8212-B5A565E816F5}";
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
    let bytes = string_bytes(value);
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
    publication: Option<crate::host_identity::Publication>,
    mutex: HANDLE,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
thread_local! { static HOST_HELD: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
impl Drop for HostLease {
    fn drop(&mut self) {
        unsafe {
            drop(self.publication.take());
            let _ = ReleaseMutex(self.mutex);
            let _ = CloseHandle(self.mutex);
            HOST_HELD.set(false);
        }
    }
}
pub fn host_capability(pid: u32, capability: &str) -> windows::core::Result<HostLease> {
    unsafe {
        if HOST_HELD.get() {
            return Err(windows::core::Error::new(
                E_ACCESSDENIED,
                "hostAlreadyActive",
            ));
        }
        let name = wide(&format!(
            "Local\\Sayweave.TSF.Host.{}",
            crate::transport::current_user_sid()?
        ));
        let security = crate::host_identity::UserSecurity::new()?;
        let attributes = security.attributes();
        let mutex = CreateMutexW(Some(&attributes), false, PCWSTR(name.as_ptr()))?;
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
        HOST_HELD.set(true);
        let mut lease = HostLease {
            publication: None,
            mutex,
            _thread: std::marker::PhantomData,
        };
        lease.publication = Some(crate::host_identity::Publication::new(pid, capability)?);
        Ok(lease)
    }
}
pub fn read_host() -> Option<(u32, String)> {
    crate::host_identity::read()
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
pub(crate) fn administrator() -> windows::core::Result<()> {
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
/// Owned machine registration only. A per-user shadow registration is a conflict.
pub fn owned_module() -> windows::core::Result<Option<String>> {
    if exists(HKEY_CURRENT_USER, COM_ROOT)? {
        return Err(windows::core::Error::new(
            E_ACCESSDENIED,
            "userRegistrationConflict",
        ));
    }
    if !exists(HKEY_LOCAL_MACHINE, COM_ROOT)? {
        return Ok(None);
    }
    let read = |name: &str| -> windows::core::Result<String> {
        unsafe {
            let mut value = [0u16; 32768];
            let mut bytes = std::mem::size_of_val(&value) as u32;
            RegGetValueW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(wide(COM_PATH).as_ptr()),
                PCWSTR(wide(name).as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(value.as_mut_ptr().cast()),
                Some(&mut bytes),
            )
            .ok()?;
            let end = value
                .iter()
                .position(|c| *c == 0)
                .ok_or_else(|| windows::core::Error::from_hresult(E_INVALIDARG))?;
            String::from_utf16(&value[..end])
                .map_err(|_| windows::core::Error::from_hresult(E_INVALIDARG))
        }
    };
    if read("SayweaveOwner")? != OWNER {
        return Err(windows::core::Error::new(
            E_ACCESSDENIED,
            "registrationOwnerMismatch",
        ));
    }
    Ok(Some(read("")?))
}
/// Repair owned display metadata without resetting TSF/profile/default relationships.
pub(crate) fn replace_machine(path: &str, expected: &str) -> windows::core::Result<()> {
    replace_metadata(path, expected, || Ok(()))
}
#[cfg(feature = "acceptance")]
pub fn acceptance_replace_machine(
    path: &str,
    expected: &str,
    reject_confirmation: bool,
) -> windows::core::Result<()> {
    replace_metadata(path, expected, || {
        if reject_confirmation {
            Err(windows::core::Error::new(
                E_FAIL,
                "acceptanceReplacementFailure",
            ))
        } else {
            Ok(())
        }
    })
}
fn replace_metadata(
    path: &str,
    expected: &str,
    confirm: impl FnOnce() -> windows::core::Result<()>,
) -> windows::core::Result<()> {
    administrator()?;
    if owned_module()?.as_deref() != Some(expected) {
        return Err(windows::core::Error::new(
            E_ACCESSDENIED,
            "registrationChanged",
        ));
    }
    let k = open_existing(HKEY_LOCAL_MACHINE, COM_PATH, KEY_READ | KEY_WRITE)?
        .ok_or_else(preference_error)?;
    let mut metadata = Vec::new();
    for language in owned_languages(&profiles()?)? {
        let name = format!("{TIP_ROOT}\\LanguageProfile\\0x{language:08X}\\{PROFILE_KEY}");
        let key = open_existing(HKEY_LOCAL_MACHINE, &name, KEY_READ | KEY_WRITE)?
            .ok_or_else(preference_error)?;
        metadata.push((
            language,
            raw_string(&key, "Description")?,
            raw_string(&key, "IconFile")?,
            key,
        ));
    }
    if metadata.is_empty() {
        return Err(preference_error());
    }
    let mut attempted = vec![[false; 2]; metadata.len()];
    let mut com_attempted = false;
    let result = (|| {
        for (index, (_, before_description, before_icon, key)) in metadata.iter().enumerate() {
            if owned_module()?.as_deref() != Some(expected)
                || raw_string(key, "Description")? != *before_description
                || raw_string(key, "IconFile")? != *before_icon
            {
                return Err(preference_error());
            }
            attempted[index][0] = true;
            string(key, "Description", "Sayweave Voice")?;
            if raw_string(key, "IconFile")? != *before_icon {
                return Err(preference_error());
            }
            attempted[index][1] = true;
            string(key, "IconFile", path)?;
        }
        if owned_module()?.as_deref() != Some(expected) {
            return Err(preference_error());
        }
        com_attempted = true;
        string(&k, "", path)?;
        confirm()?;
        if owned_module()?.as_deref() != Some(path) {
            return Err(preference_error());
        }
        let profile = profiles()?;
        for (language, _, _, key) in &metadata {
            let description =
                unsafe { profile.GetLanguageProfileDescription(&CLASS, *language, &PROFILE)? };
            if description != "Sayweave Voice"
                || raw_string(key, "Description")? != string_bytes("Sayweave Voice")
                || raw_string(key, "IconFile")? != string_bytes(path)
            {
                return Err(windows::core::Error::new(E_FAIL, "replacementNotConfirmed"));
            }
        }
        Ok(())
    })();
    if let Err(error) = result {
        let restore = (|| {
            let current = owned_module()?;
            if current.as_deref() != Some(expected)
                && !(com_attempted && current.as_deref() == Some(path))
            {
                return Err(preference_error());
            }
            for (index, (_, description, icon, key)) in metadata.iter().enumerate() {
                for (field, name, before, after) in [
                    (
                        0,
                        "Description",
                        description,
                        string_bytes("Sayweave Voice"),
                    ),
                    (1, "IconFile", icon, string_bytes(path)),
                ] {
                    if attempted[index][field] {
                        restore_string(key, name, before, &after)?;
                    }
                }
                if raw_string(key, "Description")? != *description
                    || raw_string(key, "IconFile")? != *icon
                {
                    return Err(preference_error());
                }
            }
            if com_attempted {
                restore_string(&k, "", &string_bytes(expected), &string_bytes(path))?;
            }
            if owned_module()?.as_deref() != Some(expected) {
                return Err(preference_error());
            }
            Ok::<(), windows::core::Error>(())
        })();
        if restore.is_err() {
            return Err(windows::core::Error::new(
                E_FAIL,
                "registrationRollbackIncomplete",
            ));
        }
        return Err(error);
    }
    Ok(())
}
fn restore_string(
    key: &Key,
    name: &str,
    before: &[u8],
    written: &[u8],
) -> windows::core::Result<()> {
    let current = raw_string(key, name)?;
    if current == before {
        return Ok(());
    }
    // An unrelated write invalidates rollback ownership; never overwrite it.
    if !rollback_value_known(&current, before, written) {
        return Err(preference_error());
    }
    unsafe {
        RegSetValueExW(
            key.0,
            PCWSTR(wide(name).as_ptr()),
            None,
            REG_SZ,
            Some(before),
        )
        .ok()
    }
}
fn rollback_value_known(current: &[u8], before: &[u8], written: &[u8]) -> bool {
    current == before || current == written
}
fn string_bytes(value: &str) -> Vec<u8> {
    wide(value).iter().flat_map(|c| c.to_le_bytes()).collect()
}
fn raw_string(key: &Key, name: &str) -> windows::core::Result<Vec<u8>> {
    unsafe {
        let name = wide(name);
        let mut kind = REG_VALUE_TYPE::default();
        let mut size = 0;
        RegQueryValueExW(
            key.0,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            None,
            Some(&mut size),
        )
        .ok()?;
        if kind != REG_SZ || !(2..=65536).contains(&size) || size % 2 != 0 {
            return Err(preference_error());
        }
        let mut bytes = vec![0; size as usize];
        RegQueryValueExW(
            key.0,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut kind),
            Some(bytes.as_mut_ptr()),
            Some(&mut size),
        )
        .ok()?;
        if kind != REG_SZ || size as usize != bytes.len() {
            return Err(preference_error());
        }
        Ok(bytes)
    }
}
fn register_scope(path: &str, root: HKEY) -> windows::core::Result<()> {
    register_profile(path, root, None, true, || Ok(()))
}
/// Machine registration must never enable the administrator's user profile.
pub fn register_installer_machine(path: &str, language: u16) -> windows::core::Result<()> {
    administrator()?;
    if language == 0 {
        return Err(preference_error());
    }
    register_profile(path, HKEY_LOCAL_MACHINE, Some(language), false, || Ok(()))
}
#[cfg(feature = "acceptance")]
pub fn acceptance_registration_rollback(path: &str) -> windows::core::Result<()> {
    register_profile(path, HKEY_CURRENT_USER, None, true, || {
        Err(windows::core::Error::new(
            E_FAIL,
            "acceptanceRegistrationFailure",
        ))
    })
}
fn register_profile(
    path: &str,
    root: HKEY,
    requested_language: Option<u16>,
    enable_user: bool,
    confirm: impl FnOnce() -> windows::core::Result<()>,
) -> windows::core::Result<()> {
    unsafe {
        // Fresh-install only: never overwrite or roll back an existing installation.
        let roots: &[HKEY] = if enable_user {
            &[HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE]
        } else {
            &[HKEY_LOCAL_MACHINE]
        };
        for &root in roots {
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
        if !owned_languages(&profile)?.is_empty() || owned_category_count()? != 0 {
            return Err(windows::core::Error::new(
                E_ACCESSDENIED,
                "existingRegistration",
            ));
        }
        let category: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        let language = requested_language.unwrap_or(profile.GetCurrentLanguage()?);
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
            // Keep terminated backing for compatibility while preserving explicit counts.
            let desc = wide("Sayweave Voice");
            let icon = wide(path);
            profile
                .AddLanguageProfile(
                    &CLASS,
                    language,
                    &PROFILE,
                    &desc[..desc.len() - 1],
                    &icon[..icon.len() - 1],
                    0,
                )
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
            if enable_user {
                profile
                    .EnableLanguageProfile(&CLASS, language, &PROFILE, true)
                    .map_err(|e| windows::core::Error::new(e.code(), "enableProfile"))?;
            }
            confirm()
        })();
        if result.is_err() {
            if enable_user && user_preferences().is_err() {
                return Err(windows::core::Error::new(
                    E_FAIL,
                    "registrationRollbackIncomplete",
                ));
            }
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
                remove_registered_profiles(&profile, enable_user)
            } else {
                Ok(())
            };
            if category_cleanup.is_err()
                || profile_cleanup.is_err()
                || enable_user && clear_user_preferences().is_err()
                || confirm_registry_removal(enable_user).is_err()
                || !com_anchor_matches(root, &string_bytes(OWNER), &string_bytes(path))?
            {
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

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct InstallerRegistration {
    pub module: String,
    module_bytes: Vec<u8>,
    profiles: Vec<InstallerProfile>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
struct InstallerProfile {
    language: u16,
    description: Vec<u8>,
    icon: Vec<u8>,
}
pub fn snapshot_installer_machine() -> windows::core::Result<Option<InstallerRegistration>> {
    let Some(module) = owned_module()? else {
        return Ok(None);
    };
    let key =
        open_existing(HKEY_LOCAL_MACHINE, COM_PATH, KEY_READ)?.ok_or_else(preference_error)?;
    if raw_string(&key, "SayweaveOwner")? != string_bytes(OWNER) {
        return Err(preference_error());
    }
    let module_bytes = raw_string(&key, "")?;
    let mut snapshots = Vec::new();
    owned_category_count()?;
    for language in owned_languages(&profiles()?)? {
        let path = format!("{TIP_ROOT}\\LanguageProfile\\0x{language:08X}\\{PROFILE_KEY}");
        let key =
            open_existing(HKEY_LOCAL_MACHINE, &path, KEY_READ)?.ok_or_else(preference_error)?;
        snapshots.push(InstallerProfile {
            language,
            description: raw_string(&key, "Description")?,
            icon: raw_string(&key, "IconFile")?,
        });
    }
    if snapshots.is_empty() {
        return Err(preference_error());
    }
    Ok(Some(InstallerRegistration {
        module,
        module_bytes,
        profiles: snapshots,
    }))
}
pub fn restore_installer_machine(
    snapshot: Option<&InstallerRegistration>,
    expected: &str,
) -> windows::core::Result<()> {
    administrator()?;
    let Some(before) = snapshot else {
        if !installer_fresh_anchor_known(expected)? {
            if exists(HKEY_LOCAL_MACHINE, TIP_ROOT)? || owned_category_count()? != 0 {
                return Err(preference_error());
            }
            return Ok(());
        }
        // Fresh registration can crash between the three COM values. There was
        // no prior namespace: accept only the exact subset this transaction writes.
        if owned_category_count()? != 0 {
            let category: ITfCategoryMgr =
                unsafe { CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)? };
            unsafe {
                category.UnregisterCategory(&CLASS, &GUID_TFCAT_TIP_SPEECH, &CLASS)?;
            }
        }
        remove_registered_profiles(&profiles()?, false)?;
        confirm_registry_removal(false)?;
        if !installer_fresh_anchor_known(expected)? {
            return Err(preference_error());
        }
        unsafe {
            RegDeleteTreeW(HKEY_LOCAL_MACHINE, PCWSTR(wide(COM_ROOT).as_ptr())).ok()?;
        }
        return Ok(());
    };
    let current = owned_module()?;
    if current.as_deref() != Some(before.module.as_str()) && current.as_deref() != Some(expected) {
        return Err(preference_error());
    }
    owned_category_count()?;
    let languages = owned_languages(&profiles()?)?;
    if languages
        != before
            .profiles
            .iter()
            .map(|p| p.language)
            .collect::<std::collections::BTreeSet<_>>()
    {
        return Err(preference_error());
    }
    // Validate the entire snapshot first. A crash can leave COM old with only
    // one profile field new, or COM new with some fields already restored.
    let com_key = open_existing(HKEY_LOCAL_MACHINE, COM_PATH, KEY_READ | KEY_WRITE)?
        .ok_or_else(preference_error)?;
    if raw_string(&com_key, "SayweaveOwner")? != string_bytes(OWNER)
        || !rollback_value_known(
            &raw_string(&com_key, "")?,
            &before.module_bytes,
            &string_bytes(expected),
        )
    {
        return Err(preference_error());
    }
    let mut profile_keys = Vec::new();
    for profile in &before.profiles {
        let path = format!(
            "{TIP_ROOT}\\LanguageProfile\\0x{:08X}\\{PROFILE_KEY}",
            profile.language
        );
        let key = open_existing(HKEY_LOCAL_MACHINE, &path, KEY_READ | KEY_WRITE)?
            .ok_or_else(preference_error)?;
        if !rollback_value_known(
            &raw_string(&key, "Description")?,
            &profile.description,
            &string_bytes("Sayweave Voice"),
        ) || !rollback_value_known(
            &raw_string(&key, "IconFile")?,
            &profile.icon,
            &string_bytes(expected),
        ) {
            return Err(preference_error());
        }
        profile_keys.push(key);
    }
    // Every write requires the precise value produced by this transaction.
    for (profile, key) in before.profiles.iter().zip(profile_keys) {
        restore_string(
            &key,
            "Description",
            &profile.description,
            &string_bytes("Sayweave Voice"),
        )?;
        restore_string(&key, "IconFile", &profile.icon, &string_bytes(expected))?;
    }
    restore_string(&com_key, "", &before.module_bytes, &string_bytes(expected))?;
    verify_installer_snapshot(before)
}
fn installer_fresh_anchor_known(expected: &str) -> windows::core::Result<bool> {
    if exists(HKEY_CURRENT_USER, COM_ROOT)? {
        return Err(preference_error());
    }
    let Some(root) = open_existing(HKEY_LOCAL_MACHINE, COM_ROOT, KEY_READ)? else {
        return Ok(false);
    };
    if !names(&root, false)?.is_empty() || names(&root, true)?.iter().any(|v| v != "InprocServer32")
    {
        return Err(preference_error());
    }
    if let Some(key) = open_existing(HKEY_LOCAL_MACHINE, COM_PATH, KEY_READ)? {
        if !names(&key, true)?.is_empty() {
            return Err(preference_error());
        }
        let values = names(&key, false)?;
        for name in &values {
            let value = match name.as_str() {
                "" => expected,
                "SayweaveOwner" => OWNER,
                "ThreadingModel" => "Apartment",
                _ => return Err(preference_error()),
            };
            if raw_string(&key, name)? != string_bytes(value) {
                return Err(preference_error());
            }
        }
        if !values.is_empty() && !values.iter().any(|v| v == "SayweaveOwner") {
            return Err(preference_error());
        }
    }
    Ok(true)
}
fn verify_installer_snapshot(snapshot: &InstallerRegistration) -> windows::core::Result<()> {
    let current = snapshot_installer_machine()?.ok_or_else(preference_error)?;
    if current.module_bytes != snapshot.module_bytes
        || current.profiles.len() != snapshot.profiles.len()
    {
        return Err(preference_error());
    }
    for (a, b) in current.profiles.iter().zip(&snapshot.profiles) {
        if a.language != b.language || a.description != b.description || a.icon != b.icon {
            return Err(preference_error());
        }
    }
    Ok(())
}
/// The ordinary initiating account owns this choice; missing and explicit false differ.
pub fn installer_user_choice(language: u16) -> windows::core::Result<Option<bool>> {
    let target = format!("{TIP_ROOT}\\LanguageProfile\\0x{language:08X}\\{PROFILE_KEY}");
    Ok(user_preferences()?
        .into_iter()
        .find(|(p, _)| p == &target)
        .and_then(|(_, v)| v.map(|v| v == 1)))
}
pub fn initialize_installer_user(
    language: u16,
    before: Option<bool>,
) -> windows::core::Result<bool> {
    let p = profiles()?;
    unsafe {
        if p.GetCurrentLanguage()? != language || installer_user_choice(language)? != before {
            return Err(preference_error());
        }
        let enabled = before.unwrap_or(true);
        if before.is_none() {
            p.EnableLanguageProfile(&CLASS, language, &PROFILE, enabled)?;
        }
        if p.IsEnabledLanguageProfile(&CLASS, language, &PROFILE)?
            .as_bool()
            != enabled
        {
            return Err(preference_error());
        }
        Ok(enabled)
    }
}
pub fn restore_installer_user(language: u16, before: Option<bool>) -> windows::core::Result<()> {
    let current = installer_user_choice(language)?;
    if current == before {
        return Ok(());
    }
    if before.is_some() || current != Some(true) {
        return Err(preference_error());
    }
    // First enablement wrote only this known DWORD. Never clear another language/choice.
    let leaf = format!("{TIP_ROOT}\\LanguageProfile\\0x{language:08X}\\{PROFILE_KEY}");
    let key = open_existing(HKEY_CURRENT_USER, &leaf, KEY_READ | KEY_WRITE)?
        .ok_or_else(preference_error)?;
    unsafe {
        RegDeleteValueW(key.0, w!("Enable")).ok()?;
    }
    drop(key);
    for path in [
        &leaf,
        &format!("{TIP_ROOT}\\LanguageProfile\\0x{language:08X}"),
        &format!("{TIP_ROOT}\\LanguageProfile"),
        TIP_ROOT,
    ] {
        let value = unsafe { SHDeleteEmptyKeyW(HKEY_CURRENT_USER, PCWSTR(wide(path).as_ptr())) };
        if value != ERROR_SUCCESS && value != ERROR_FILE_NOT_FOUND && value != ERROR_DIR_NOT_EMPTY {
            return Err(preference_error());
        }
    }
    Ok(())
}
pub fn unregister() -> windows::core::Result<()> {
    unregister_scope(HKEY_CURRENT_USER, true)
}
pub fn unregister_machine() -> windows::core::Result<()> {
    administrator()?;
    unregister_scope(HKEY_LOCAL_MACHINE, true)
}
fn unregister_scope(root: HKEY, cleanup_user: bool) -> windows::core::Result<()> {
    unsafe {
        if root == HKEY_LOCAL_MACHINE && exists(HKEY_CURRENT_USER, COM_ROOT)? {
            return Err(windows::core::Error::new(
                E_ACCESSDENIED,
                "userRegistrationConflict",
            ));
        }
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
        let anchor = open_existing(root, COM_PATH, KEY_READ)?.ok_or_else(preference_error)?;
        let expected_owner = raw_string(&anchor, "SayweaveOwner")?;
        let expected_module = raw_string(&anchor, "")?;
        drop(anchor);
        if expected_owner != string_bytes(OWNER) {
            return Err(windows::core::Error::new(
                E_ACCESSDENIED,
                "registrationOwnerMismatch",
            ));
        }
        // Refuse an unknown preference tree while the owned COM anchor is retryable.
        if cleanup_user {
            user_preferences()?;
        }
        owned_languages(&profiles()?)?;
        let category_count = owned_category_count()?;
        // Attempt every cleanup even when one COM service is unavailable.
        let category = (|| {
            if category_count == 0 {
                return Ok(());
            }
            let category: ITfCategoryMgr =
                CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
            category.UnregisterCategory(&CLASS, &GUID_TFCAT_TIP_SPEECH, &CLASS)
        })();
        let profile = profiles().and_then(|p| remove_registered_profiles(&p, cleanup_user));
        // Keep COM registration until TSF removal succeeds, so cleanup can be retried.
        category?;
        profile?;
        if cleanup_user {
            clear_user_preferences()?;
        }
        confirm_registry_removal(cleanup_user)?;
        if !com_anchor_matches(root, &expected_owner, &expected_module)? {
            return Err(preference_error());
        }
        let status = RegDeleteTreeW(root, PCWSTR(wide(COM_ROOT).as_ptr()));
        if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
            status.ok()?;
        }
        Ok(())
    }
}
fn com_anchor_matches(root: HKEY, owner: &[u8], module: &[u8]) -> windows::core::Result<bool> {
    let Some(key) = open_existing(root, COM_PATH, KEY_READ)? else {
        return Ok(false);
    };
    Ok(raw_string(&key, "SayweaveOwner")? == owner && raw_string(&key, "")? == module)
}
fn confirm_registry_removal(cleanup_user: bool) -> windows::core::Result<()> {
    let machine = exists(HKEY_LOCAL_MACHINE, TIP_ROOT)?;
    let user = cleanup_user && exists(HKEY_CURRENT_USER, TIP_ROOT)?;
    if machine || user {
        return Err(windows::core::Error::new(
            E_FAIL,
            format!("profileRemovalIncomplete machine={machine} user={user}"),
        ));
    }
    // TSF category enumeration can retain this process's pre-removal inventory.
    // Successful native removals and absent persisted roots establish cleanup;
    // acceptance separately confirms profiles/categories in a new process.
    Ok(())
}

fn open_existing(
    root: HKEY,
    path: &str,
    access: REG_SAM_FLAGS,
) -> windows::core::Result<Option<Key>> {
    unsafe {
        let mut handle = HKEY::default();
        let status = RegOpenKeyExW(root, PCWSTR(wide(path).as_ptr()), None, access, &mut handle);
        if matches!(status, ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND) {
            return Ok(None);
        }
        status.ok()?;
        Ok(Some(Key(handle)))
    }
}
fn names(key: &Key, children: bool) -> windows::core::Result<Vec<String>> {
    unsafe {
        let mut names = Vec::new();
        for index in 0..=32 {
            let mut name = [0u16; 256];
            let mut count = name.len() as u32;
            let status = if children {
                RegEnumKeyExW(
                    key.0,
                    index,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut count,
                    None,
                    None,
                    None,
                    None,
                )
            } else {
                RegEnumValueW(
                    key.0,
                    index,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut count,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if status == ERROR_NO_MORE_ITEMS {
                return Ok(names);
            }
            status.ok()?;
            if index == 32 || count as usize >= name.len() {
                return Err(windows::core::Error::new(
                    E_ACCESSDENIED,
                    "preferenceInventoryExceeded",
                ));
            }
            names.push(
                String::from_utf16(&name[..count as usize])
                    .map_err(|_| windows::core::Error::from_hresult(E_INVALIDARG))?,
            );
        }
        unreachable!()
    }
}
fn preference_error() -> windows::core::Error {
    windows::core::Error::new(E_ACCESSDENIED, "preferenceIdentityChanged")
}
fn user_preferences() -> windows::core::Result<Vec<(String, Option<u32>)>> {
    let mut paths = Vec::new();
    let Some(root) = open_existing(HKEY_CURRENT_USER, TIP_ROOT, KEY_READ)? else {
        return Ok(paths);
    };
    if !names(&root, false)?.is_empty() {
        return Err(preference_error());
    }
    let children = names(&root, true)?;
    if children.is_empty() {
        return Ok(vec![(TIP_ROOT.into(), None)]);
    }
    if children != ["LanguageProfile"] {
        return Err(preference_error());
    }
    let language_root = format!("{TIP_ROOT}\\LanguageProfile");
    let key =
        open_existing(HKEY_CURRENT_USER, &language_root, KEY_READ)?.ok_or_else(preference_error)?;
    if !names(&key, false)?.is_empty() {
        return Err(preference_error());
    }
    for language in names(&key, true)? {
        if language.len() != 10
            || !language.starts_with("0x")
            || !language[2..].bytes().all(|b| b.is_ascii_hexdigit())
            || !matches!(u32::from_str_radix(&language[2..], 16), Ok(1..=65535))
        {
            return Err(preference_error());
        }
        let group = format!("{language_root}\\{language}");
        let key =
            open_existing(HKEY_CURRENT_USER, &group, KEY_READ)?.ok_or_else(preference_error)?;
        if !names(&key, false)?.is_empty() {
            return Err(preference_error());
        }
        let children = names(&key, true)?;
        if !children.is_empty() {
            if children != [PROFILE_KEY] {
                return Err(preference_error());
            }
            let leaf = format!("{group}\\{PROFILE_KEY}");
            let key =
                open_existing(HKEY_CURRENT_USER, &leaf, KEY_READ)?.ok_or_else(preference_error)?;
            if !names(&key, true)?.is_empty() {
                return Err(preference_error());
            }
            let values = names(&key, false)?;
            let value = if values.is_empty() {
                None
            } else {
                if values != ["Enable"] {
                    return Err(preference_error());
                }
                let mut enabled = 0u32;
                let mut size = 4;
                unsafe {
                    RegGetValueW(
                        key.0,
                        PCWSTR::null(),
                        w!("Enable"),
                        RRF_RT_REG_DWORD,
                        None,
                        Some((&mut enabled as *mut u32).cast()),
                        Some(&mut size),
                    )
                    .ok()?;
                }
                if size != 4 || enabled > 1 {
                    return Err(preference_error());
                }
                Some(enabled)
            };
            paths.push((leaf, value));
        }
        paths.push((group, None));
    }
    paths.push((language_root, None));
    paths.push((TIP_ROOT.into(), None));
    Ok(paths)
}
fn clear_user_preferences() -> windows::core::Result<()> {
    // API removal can leave Enable=0. Delete only that known value and empty keys.
    // SHDeleteEmptyKey refuses concurrent values as well as subkeys.
    for (path, value) in user_preferences()? {
        let Some(key) = open_existing(HKEY_CURRENT_USER, &path, KEY_READ | KEY_WRITE)? else {
            continue;
        };
        if let Some(expected) = value {
            if names(&key, false)? != ["Enable"] || !names(&key, true)?.is_empty() {
                return Err(preference_error());
            }
            let mut actual = 0u32;
            let mut size = 4;
            unsafe {
                RegGetValueW(
                    key.0,
                    PCWSTR::null(),
                    w!("Enable"),
                    RRF_RT_REG_DWORD,
                    None,
                    Some((&mut actual as *mut u32).cast()),
                    Some(&mut size),
                )
                .ok()?;
                if size != 4 || actual != expected {
                    return Err(preference_error());
                }
                RegDeleteValueW(key.0, w!("Enable")).ok()?;
            }
        }
        drop(key);
        unsafe {
            SHDeleteEmptyKeyW(HKEY_CURRENT_USER, PCWSTR(wide(&path).as_ptr())).ok()?;
        }
    }
    if exists(HKEY_CURRENT_USER, TIP_ROOT)? {
        return Err(preference_error());
    }
    Ok(())
}
fn owned_category_count() -> windows::core::Result<usize> {
    unsafe {
        let category: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        let enumerator = category.EnumCategoriesInItem(&CLASS)?;
        let mut count = 0usize;
        loop {
            let mut batch = [GUID::default(); 16];
            let mut fetched = 0;
            enumerator.Next(&mut batch, Some(&mut fetched)).ok()?;
            if fetched as usize > batch.len() || count + fetched as usize > 512 {
                return Err(windows::core::Error::new(
                    E_FAIL,
                    "categoryInventoryExceeded",
                ));
            }
            count += fetched as usize;
            if batch[..fetched as usize]
                .iter()
                .any(|category| *category != GUID_TFCAT_TIP_SPEECH)
            {
                return Err(windows::core::Error::new(
                    E_ACCESSDENIED,
                    "categoryIdentityChanged",
                ));
            }
            if fetched == 0 {
                return Ok(count);
            }
        }
    }
}
fn owned_languages(
    profile: &ITfInputProcessorProfiles,
) -> windows::core::Result<std::collections::BTreeSet<u16>> {
    unsafe {
        let manager: ITfInputProcessorProfileMgr = profile.cast()?;
        let enumerator = manager.EnumProfiles(0)?;
        let mut owned = std::collections::BTreeSet::new();
        let mut count = 0usize;
        loop {
            let mut batch = [TF_INPUTPROCESSORPROFILE::default(); 16];
            let mut fetched = 0;
            enumerator.Next(&mut batch, &mut fetched)?;
            if fetched as usize > batch.len() || count + fetched as usize > 512 {
                return Err(windows::core::Error::new(
                    E_FAIL,
                    "profileInventoryExceeded",
                ));
            }
            count += fetched as usize;
            for profile in &batch[..fetched as usize] {
                if profile.clsid == CLASS {
                    if profile.guidProfile != PROFILE {
                        return Err(windows::core::Error::new(
                            E_ACCESSDENIED,
                            "profileIdentityChanged",
                        ));
                    }
                    owned.insert(profile.langid);
                }
            }
            if fetched == 0 {
                return Ok(owned);
            }
        }
    }
}

fn remove_registered_profiles(
    profile: &ITfInputProcessorProfiles,
    cleanup_user: bool,
) -> windows::core::Result<()> {
    unsafe {
        for language in owned_languages(profile)? {
            if cleanup_user {
                profile.EnableLanguageProfile(&CLASS, language, &PROFILE, false)?;
            }
            profile.RemoveLanguageProfile(&CLASS, language, &PROFILE)?;
        }
        if exists(HKEY_LOCAL_MACHINE, TIP_ROOT)? {
            profile.Unregister(&CLASS)?;
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

#[cfg(test)]
mod installer_snapshot_tests {
    use super::*;
    #[test]
    fn half_written_metadata_is_restorable_only_from_exact_known_values() {
        let old = string_bytes("original.dll");
        let next = string_bytes("new.dll");
        assert!(rollback_value_known(&old, &old, &next));
        assert!(rollback_value_known(&next, &old, &next));
        assert!(!rollback_value_known(
            &string_bytes("third-party.dll"),
            &old,
            &next
        ));
        assert!(!rollback_value_known(b"malformed", &old, &next));
    }
}
