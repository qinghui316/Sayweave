//! Fixed component deployment. No renderer-supplied path, script or DLL is accepted.
use crate::registration;
use std::{
    io::Write,
    path::{Path, PathBuf},
};
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        Storage::FileSystem::*,
        System::{
            Com::CoTaskMemFree,
            SystemServices::{ACCESS_ALLOWED_ACE_TYPE, ACCESS_DENIED_ACE_TYPE},
        },
        UI::Shell::*,
    },
    core::{PCWSTR, PWSTR},
};

fn fail(code: &str) -> windows::core::Error {
    windows::core::Error::new(E_ACCESSDENIED, code)
}
fn wide(v: &str) -> Vec<u16> {
    v.encode_utf16().chain(Some(0)).collect()
}
fn present(path: &Path) -> windows::core::Result<bool> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(fail("protectedPathUnavailable")),
    }
}
fn local_path(value: &str) -> windows::core::Result<PathBuf> {
    // std::fs::canonicalize stores Win32 verbatim disk paths in COM registration.
    let value = value.strip_prefix("\\\\?\\").unwrap_or(value);
    let path = PathBuf::from(value);
    if !matches!(path.components().next(),Some(std::path::Component::Prefix(p)) if matches!(p.kind(),std::path::Prefix::Disk(_)))
        || !path.has_root()
        || path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(fail("invalidModulePath"));
    }
    Ok(path)
}
pub fn version(bytes: &[u8]) -> String {
    use sha2::Digest;
    format!("{:x}", sha2::Sha256::digest(bytes))
}
fn program_files() -> windows::core::Result<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(&FOLDERID_ProgramFiles, KF_FLAG_DEFAULT, None)?;
        let result = p.to_string().map(PathBuf::from);
        CoTaskMemFree(Some(p.0.cast()));
        Ok(result?)
    }
}
fn sid(sid: PSID) -> windows::core::Result<String> {
    unsafe {
        let mut value = PWSTR::null();
        ConvertSidToStringSidW(sid, &mut value)?;
        let result = value.to_string();
        let _ = LocalFree(Some(HLOCAL(value.0.cast())));
        Ok(result?)
    }
}
fn privileged(v: &str) -> bool {
    matches!(
        v,
        "S-1-5-18"
            | "S-1-5-32-544"
            | "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464"
    )
}
fn protected(path: &Path) -> windows::core::Result<()> {
    unsafe {
        let name = wide(&path.to_string_lossy());
        let attrs = GetFileAttributesW(PCWSTR(name.as_ptr()));
        if attrs == INVALID_FILE_ATTRIBUTES || attrs & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
            return Err(fail("invalidProtectedPath"));
        }
        let mut owner = PSID::default();
        let mut dacl = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        GetNamedSecurityInfoW(
            PCWSTR(name.as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            None,
            &mut descriptor,
        )
        .ok()?;
        let result = (|| {
            if !privileged(&sid(owner)?) || dacl.is_null() {
                return Err(fail("unprotectedOwner"));
            }
            let mut info = ACL_SIZE_INFORMATION::default();
            GetAclInformation(
                dacl,
                (&mut info as *mut ACL_SIZE_INFORMATION).cast(),
                std::mem::size_of_val(&info) as u32,
                AclSizeInformation,
            )?;
            for n in 0..info.AceCount {
                let mut ptr = std::ptr::null_mut();
                GetAce(dacl, n, &mut ptr)?;
                let header = &*ptr.cast::<ACE_HEADER>();
                if header.AceFlags & INHERIT_ONLY_ACE.0 as u8 != 0 {
                    continue;
                }
                if header.AceType as u32 == ACCESS_DENIED_ACE_TYPE {
                    continue;
                }
                if header.AceType as u32 != ACCESS_ALLOWED_ACE_TYPE {
                    return Err(fail("unsupportedProtectedAcl"));
                }
                let ace = &*ptr.cast::<ACCESS_ALLOWED_ACE>();
                // File write/append/EA/attributes, delete(child), ACL/owner and generic write/all.
                let writes = 0x4000_0000u32 | 0x1000_0000 | 0x000d_0156;
                if ace.Mask & writes != 0
                    && !privileged(&sid(PSID((&ace.SidStart as *const u32).cast_mut().cast()))?)
                {
                    return Err(fail("writableProtectedPath"));
                }
            }
            Ok(())
        })();
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        result
    }
}
fn ensure_directory(path: &Path) -> windows::core::Result<()> {
    if !present(path)? {
        unsafe {
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(
                    wide("O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;0x1200a9;;;BU)")
                        .as_ptr(),
                ),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )?;
            let attr = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: false.into(),
            };
            let result =
                CreateDirectoryW(PCWSTR(wide(&path.to_string_lossy()).as_ptr()), Some(&attr));
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
            result?;
        }
    }
    protected(path)
}
fn owned_location(path: &str) -> windows::core::Result<PathBuf> {
    let base = program_files()?.join("Sayweave").join("TextService");
    let candidate = local_path(path)?;
    let folder = candidate
        .parent()
        .ok_or_else(|| fail("invalidModulePath"))?;
    if candidate.file_name() != Some(std::ffi::OsStr::new("sayweave_text_service.dll"))
        || folder.parent() != Some(base.as_path())
    {
        return Err(fail("invalidModulePath"));
    }
    let name = folder
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| fail("invalidModulePath"))?;
    if name != "stage-a" && !(name.len() == 64 && name.bytes().all(|b| b.is_ascii_hexdigit())) {
        return Err(fail("invalidModuleVersion"));
    }
    Ok(candidate)
}
fn owned_path(path: &str) -> windows::core::Result<PathBuf> {
    let candidate = owned_location(path)?;
    protected(&program_files()?)?;
    let base = program_files()?.join("Sayweave").join("TextService");
    for p in [
        base.parent().unwrap(),
        base.as_path(),
        candidate.parent().unwrap(),
        candidate.as_path(),
    ] {
        if present(p)? {
            protected(p)?;
        }
    }
    Ok(candidate)
}
/// Called only by the application's explicit elevated helper with its embedded bytes.
pub fn install(bytes: &[u8], expected_sid: &str) -> windows::core::Result<()> {
    registration::administrator()?;
    if expected_sid != crate::transport::current_user_sid()? {
        return Err(fail("initiatingUserChanged"));
    }
    if bytes.len() < 1024 || bytes.len() > 32 * 1024 * 1024 || !bytes.starts_with(b"MZ") {
        return Err(fail("invalidPayload"));
    }
    let old = registration::owned_module()?;
    if let Some(old) = &old {
        owned_path(old)?;
    }
    let root = program_files()?;
    protected(&root)?;
    let app = root.join("Sayweave");
    ensure_directory(&app)?;
    let base = app.join("TextService");
    ensure_directory(&base)?;
    let folder = base.join(version(bytes));
    ensure_directory(&folder)?;
    let module = folder.join("sayweave_text_service.dll");
    let created = !present(&module)?;
    if created {
        write_payload(&module, bytes)?;
    }
    struct PendingModule {
        path: PathBuf,
        created: bool,
        durable: bool,
    }
    impl Drop for PendingModule {
        fn drop(&mut self) {
            if self.created && !self.durable && protected(&self.path).is_ok() {
                let _ = std::fs::remove_file(&self.path);
                if let Some(parent) = self.path.parent() {
                    let _ = std::fs::remove_dir(parent);
                }
            }
        }
    }
    let mut pending = PendingModule {
        path: module.clone(),
        created,
        durable: false,
    };
    owned_path(&module.to_string_lossy())?;
    if std::fs::read(&module).map_err(|_| fail("payloadReadFailed"))? != bytes {
        return Err(fail("payloadMismatch"));
    }
    let mut managed = managed_modules(&base)?;
    if let Some(old) = &old
        && !managed.contains(old)
    {
        managed.push(old.clone());
    }
    let new = module.to_string_lossy().into_owned();
    if !managed.contains(&new) {
        managed.push(new);
    }
    if managed.len() > 64 {
        return Err(fail("managedVersionLimit"));
    }
    write_managed(&base, &managed)?;
    pending.durable = true;
    match old {
        Some(old) => registration::replace_machine(&module.to_string_lossy(), &old),
        None => registration::register_machine(&module.to_string_lossy()),
    }
    // Keep old protected modules: applications may still have those versions mapped.
}
fn write_payload(module: &Path, bytes: &[u8]) -> windows::core::Result<()> {
    let temp = module.with_file_name(format!("payload-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|_| fail("payloadWriteFailed"))?;
        file.write_all(bytes)
            .map_err(|_| fail("payloadWriteFailed"))?;
        file.sync_all().map_err(|_| fail("payloadWriteFailed"))?;
        drop(file);
        protected(&temp)?;
        if std::fs::read(&temp).map_err(|_| fail("payloadReadFailed"))? != bytes {
            return Err(fail("payloadMismatch"));
        }
        std::fs::rename(&temp, module).map_err(|_| fail("payloadWriteFailed"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
pub fn remove(expected_sid: &str) -> windows::core::Result<()> {
    registration::administrator()?;
    if expected_sid != crate::transport::current_user_sid()? {
        return Err(fail("initiatingUserChanged"));
    }
    let base = program_files()?.join("Sayweave").join("TextService");
    let mut managed = managed_modules(&base)?;
    if let Some(path) = registration::owned_module()? {
        owned_path(&path)?;
        if !managed.contains(&path) {
            managed.push(path);
        }
    }
    for path in &managed {
        owned_path(path)?;
    }
    // Registry/profile removal comes first; a mapped DLL can remain for a later exact cleanup.
    registration::unregister_machine()?;
    if !present(&base)? {
        return Ok(());
    }
    let mut pending = Vec::new();
    for path in managed {
        let module = Path::new(&path);
        if present(module)? && std::fs::remove_file(module).is_err() {
            pending.push(path);
            continue;
        }
        if let Some(folder) = module.parent() {
            let _ = std::fs::remove_dir(folder);
        }
    }
    write_managed(&base, &pending)?;
    Ok(())
}
fn managed_modules(base: &Path) -> windows::core::Result<Vec<String>> {
    if !present(base)? {
        return Ok(Vec::new());
    }
    protected(&program_files()?)?;
    protected(base.parent().ok_or_else(|| fail("invalidProtectedPath"))?)?;
    protected(base)?;
    let file = base.join("managed.json");
    if !present(&file)? {
        return Ok(Vec::new());
    }
    protected(&file)?;
    if std::fs::metadata(&file)
        .map_err(|_| fail("managedReadFailed"))?
        .len()
        > 128 * 1024
    {
        return Err(fail("managedVersionLimit"));
    }
    let value: Vec<String> =
        serde_json::from_slice(&std::fs::read(&file).map_err(|_| fail("managedReadFailed"))?)
            .map_err(|_| fail("managedInvalid"))?;
    if value.len() > 64 || value.iter().any(|v| v.len() > 4096) {
        return Err(fail("managedVersionLimit"));
    }
    // Validate the immutable fixed path shape even for a previously deleted module.
    for path in &value {
        owned_location(path)?;
    }
    Ok(value)
}
fn write_managed(base: &Path, paths: &[String]) -> windows::core::Result<()> {
    let target = base.join("managed.json");
    if present(&target)? {
        protected(&target)?;
    }
    let temp = base.join(format!("managed-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let data = serde_json::to_vec(paths).map_err(|_| fail("managedInvalid"))?;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|_| fail("managedWriteFailed"))?;
        file.write_all(&data)
            .map_err(|_| fail("managedWriteFailed"))?;
        file.sync_all().map_err(|_| fail("managedWriteFailed"))?;
        drop(file);
        protected(&temp)?;
        std::fs::rename(&temp, &target).map_err(|_| fail("managedWriteFailed"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
pub fn cleanup_pending() -> windows::core::Result<bool> {
    let base = program_files()?.join("Sayweave").join("TextService");
    if registration::owned_module()?.is_some() {
        return Ok(false);
    }
    for path in managed_modules(&base)? {
        if present(Path::new(&path))? {
            return Ok(true);
        }
    }
    Ok(false)
}
pub fn matches_payload(bytes: &[u8]) -> windows::core::Result<Option<bool>> {
    let Some(path) = registration::owned_module()? else {
        return Ok(None);
    };
    let module = owned_path(&path)?;
    let data = std::fs::read(module).map_err(|_| fail("payloadReadFailed"))?;
    Ok(Some(!bytes.is_empty() && version(&data) == version(bytes)))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn current_install_has_protected_identity_without_registry_writes() {
        // Read-only: this succeeds without an install on contributor machines as well.
        if let Some(path) = registration::owned_module().unwrap() {
            assert!(owned_path(&path).is_ok());
        }
        assert!(owned_path("C:\\Users\\Public\\sayweave_text_service.dll").is_err());
        assert_eq!(local_path("\\\\?\\C:\\Program Files\\Sayweave\\TextService\\stage-a\\sayweave_text_service.dll").unwrap(),PathBuf::from("C:\\Program Files\\Sayweave\\TextService\\stage-a\\sayweave_text_service.dll"));
        assert!(local_path("\\\\?\\UNC\\server\\share\\module.dll").is_err());
        assert!(local_path("C:\\Program Files\\..\\module.dll").is_err());
        let absent = program_files()
            .unwrap()
            .join("Sayweave")
            .join("TextService")
            .join("a".repeat(64))
            .join("sayweave_text_service.dll");
        if std::fs::symlink_metadata(&absent).is_err() {
            // An owned fixed location remains repairable after its file/folder disappears.
            assert!(owned_path(&absent.to_string_lossy()).is_ok());
        }
        assert!(
            owned_location("C:\\Program Files\\Sayweave\\TextService\\stage-a\\other.dll").is_err()
        );
    }
    #[test]
    fn versions_bind_payload_bytes() {
        assert_eq!(
            version(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_ne!(version(b"abc"), version(b"abd"));
    }
}
