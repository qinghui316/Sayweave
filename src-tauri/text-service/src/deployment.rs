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
    let old = registration::owned_module()?;
    let module = prepare_installer(bytes)?;
    match old {
        Some(old) => registration::replace_machine(&module, &old),
        None => registration::register_machine(&module),
    }
}

/// Stage only fixed embedded bytes. No user SID/profile state is changed here.
pub fn prepare_installer(bytes: &[u8]) -> windows::core::Result<String> {
    registration::administrator()?;
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
    Ok(module.to_string_lossy().into_owned())
}
pub fn authorize_installer() -> windows::core::Result<()> {
    registration::administrator()
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
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Removal {
    schema: u32,
    initiator: String,
    updater: crate::transport::Identity,
    registration: Option<registration::InstallerRegistration>,
    descriptor: Option<crate::lifecycle::CurrentEngine>,
    complete: bool,
}
fn removal_file() -> windows::core::Result<PathBuf> {
    Ok(program_files()?
        .join("Sayweave")
        .join("TextService")
        .join("component-removal.json"))
}
fn read_removal() -> windows::core::Result<Option<Removal>> {
    let file = removal_file()?;
    if !present(&file)? {
        return Ok(None);
    }
    for path in [
        &program_files()?,
        file.parent().unwrap().parent().unwrap(),
        file.parent().unwrap(),
        &file,
    ] {
        protected(path)?;
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(file)
        .map_err(|_| fail("componentRemovalRead"))?
        .take(128 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| fail("componentRemovalRead"))?;
    if bytes.len() > 128 * 1024 {
        return Err(fail("componentRemovalLimit"));
    }
    let record: Removal =
        serde_json::from_slice(&bytes).map_err(|_| fail("componentRemovalInvalid"))?;
    if record.schema != 1
        || record.initiator.is_empty()
        || record.initiator.len() > 256
        || record.descriptor.as_ref().is_some_and(|d| !d.valid())
    {
        return Err(fail("componentRemovalInvalid"));
    }
    if let Some(old) = &record.registration {
        owned_location(&old.module)?;
    }
    Ok(Some(record))
}
fn save_removal(record: &Removal) -> windows::core::Result<()> {
    let file = removal_file()?;
    let base = file.parent().unwrap();
    protected(base)?;
    if present(&file)? {
        protected(&file)?;
    }
    let bytes = serde_json::to_vec(record).map_err(|_| fail("componentRemovalInvalid"))?;
    let temp = base.join(format!("removal-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut out = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|_| fail("componentRemovalWrite"))?;
        out.write_all(&bytes)
            .map_err(|_| fail("componentRemovalWrite"))?;
        out.sync_all().map_err(|_| fail("componentRemovalWrite"))?;
        drop(out);
        protected(&temp)?;
        std::fs::rename(&temp, &file).map_err(|_| fail("componentRemovalWrite"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
fn finish_removal(record: &mut Removal) -> windows::core::Result<()> {
    let descriptor = current_engine()?;
    if !registration::removal_matches(record.registration.as_ref())?
        || descriptor
            .as_ref()
            .is_some_and(|now| record.descriptor.as_ref() != Some(now))
    {
        return Err(fail("componentRemovalChanged"));
    }
    registration::resume_installer_removal(record.registration.as_ref())?;
    publish_engine(descriptor.as_ref(), None)?;
    if let Some(mut update) = read_core_update()? {
        // Removal is a durable terminal intent, never a request to restore old payloads.
        update.phase = "removed".into();
        save_core_update(&update)?;
    }
    record.complete = true;
    save_removal(record)
}
fn recover_removal(expected_sid: &str) -> windows::core::Result<()> {
    if let Some(mut record) = read_removal()?
        && !record.complete
    {
        if record.initiator != expected_sid {
            return Err(fail("componentRemovalOwner"));
        }
        finish_removal(&mut record)?;
    }
    Ok(())
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
    if !present(&base)? {
        registration::unregister_machine()?;
        return Ok(());
    }
    let mut record = match read_removal()? {
        Some(record) if !record.complete => record,
        _ => Removal {
            schema: 1,
            initiator: expected_sid.into(),
            updater: crate::transport::identity(std::process::id())?,
            registration: registration::snapshot_installer_machine()?,
            descriptor: current_engine()?,
            complete: false,
        },
    };
    if record.initiator != expected_sid {
        return Err(fail("componentRemovalOwner"));
    }
    // Persist intent before touching registration or the current descriptor.
    save_removal(&record)?;
    finish_removal(&mut record)?;
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
/// One protected descriptor; no path from a caller, frontend or registry is trusted.
pub fn current_engine() -> windows::core::Result<Option<crate::lifecycle::CurrentEngine>> {
    let base = program_files()?.join("Sayweave").join("TextService");
    let file = base.join("current-engine.json");
    if !present(&file)? {
        return Ok(None);
    }
    for path in [&program_files()?, base.parent().unwrap(), &base, &file] {
        protected(path)?;
    }
    let mut data = Vec::new();
    use std::io::Read;
    std::fs::File::open(&file)
        .map_err(|_| fail("descriptorRead"))?
        .take(4097)
        .read_to_end(&mut data)
        .map_err(|_| fail("descriptorRead"))?;
    if data.len() > 4096 {
        return Err(fail("descriptorLimit"));
    }
    let descriptor: crate::lifecycle::CurrentEngine =
        serde_json::from_slice(&data).map_err(|_| fail("descriptorInvalid"))?;
    if !descriptor.valid() {
        return Err(fail("descriptorInvalid"));
    }
    Ok(Some(descriptor))
}
pub fn verified_core(
    descriptor: &crate::lifecycle::CurrentEngine,
) -> windows::core::Result<PathBuf> {
    if !descriptor.valid() {
        return Err(fail("descriptorInvalid"));
    }
    let proxy = registration::owned_module()?.ok_or_else(|| fail("proxyMissing"))?;
    if installer_module_hash(&proxy)? != descriptor.proxy_sha256 {
        return Err(fail("proxyIdentity"));
    }
    let path = program_files()?
        .join("Sayweave")
        .join("TextService")
        .join(&descriptor.core_sha256)
        .join("sayweave_text_service.dll");
    let path = owned_path(&path.to_string_lossy())?;
    if installer_module_hash(&path.to_string_lossy())? != descriptor.core_sha256 {
        return Err(fail("coreIdentity"));
    }
    Ok(path)
}
pub fn verify_runtime_location(path: &Path) -> windows::core::Result<()> {
    owned_path(&path.to_string_lossy()).map(|_| ())
}
pub fn matches_bundle(proxy: &[u8], core: &[u8]) -> windows::core::Result<Option<bool>> {
    let Some(matched) = matches_payload(proxy)? else {
        return Ok(None);
    };
    if core.is_empty() {
        return Ok(Some(false));
    }
    let Some(descriptor) = current_engine()? else {
        return Ok(Some(false));
    };
    Ok(Some(
        matched
            && descriptor.input_protocol == crate::wire::PROTOCOL
            && descriptor.proxy_sha256 == version(proxy)
            && descriptor.core_sha256 == version(core)
            && verified_core(&descriptor).is_ok(),
    ))
}
pub fn publish_engine(
    expected: Option<&crate::lifecycle::CurrentEngine>,
    target: Option<&crate::lifecycle::CurrentEngine>,
) -> windows::core::Result<()> {
    registration::administrator()?;
    if current_engine()?.as_ref() != expected {
        return Err(fail("descriptorChanged"));
    }
    let base = program_files()?.join("Sayweave").join("TextService");
    protected(&base)?;
    let file = base.join("current-engine.json");
    if let Some(target) = target {
        verified_core(target)?;
        let bytes = serde_json::to_vec(target).map_err(|_| fail("descriptorInvalid"))?;
        let temp = base.join(format!("engine-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut out = std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp)
                .map_err(|_| fail("descriptorWrite"))?;
            out.write_all(&bytes).map_err(|_| fail("descriptorWrite"))?;
            out.sync_all().map_err(|_| fail("descriptorWrite"))?;
            drop(out);
            protected(&temp)?;
            if current_engine()?.as_ref() != expected {
                return Err(fail("descriptorChanged"));
            }
            std::fs::rename(&temp, &file).map_err(|_| fail("descriptorWrite"))?;
            if current_engine()?.as_ref() != Some(target) {
                return Err(fail("descriptorConfirmation"));
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(temp);
        }
        result
    } else {
        if present(&file)? {
            protected(&file)?;
            std::fs::remove_file(&file).map_err(|_| fail("descriptorRemove"))?;
        }
        Ok(())
    }
}
pub fn prepare_engine(
    proxy: &[u8],
    core: &[u8],
) -> windows::core::Result<(String, crate::lifecycle::CurrentEngine)> {
    let module = prepare_installer(proxy)?;
    prepare_installer(core)?;
    let descriptor = crate::lifecycle::CurrentEngine {
        schema_version: 1,
        revision: uuid::Uuid::new_v4().to_string(),
        proxy_sha256: version(proxy),
        core_sha256: version(core),
        abi: 1,
        input_protocol: crate::wire::PROTOCOL,
        platform: "x86_64-pc-windows-msvc".into(),
    };
    Ok((module, descriptor))
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CoreOrigin {
    registration: Option<registration::InstallerRegistration>,
    hash: Option<String>,
    engine: Option<crate::lifecycle::CurrentEngine>,
}
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CoreUpdate {
    schema: u32,
    phase: String,
    initiator: String,
    updater: crate::transport::Identity,
    before_registration: Option<registration::InstallerRegistration>,
    before_hash: Option<String>,
    before_engine: Option<crate::lifecycle::CurrentEngine>,
    #[serde(default)]
    origin: Option<CoreOrigin>,
    target: crate::lifecycle::CurrentEngine,
    module: String,
    error: Option<String>,
}
fn core_update_file() -> windows::core::Result<PathBuf> {
    Ok(program_files()?
        .join("Sayweave")
        .join("TextService")
        .join("component-update.json"))
}
fn read_core_update() -> windows::core::Result<Option<CoreUpdate>> {
    let file = core_update_file()?;
    if !present(&file)? {
        return Ok(None);
    }
    for path in [
        &program_files()?,
        file.parent().unwrap().parent().unwrap(),
        file.parent().unwrap(),
        &file,
    ] {
        protected(path)?;
    }
    let mut data = Vec::new();
    use std::io::Read;
    std::fs::File::open(&file)
        .map_err(|_| fail("componentUpdateRead"))?
        .take(128 * 1024 + 1)
        .read_to_end(&mut data)
        .map_err(|_| fail("componentUpdateRead"))?;
    if data.len() > 128 * 1024 {
        return Err(fail("componentUpdateLimit"));
    }
    let record: CoreUpdate =
        serde_json::from_slice(&data).map_err(|_| fail("componentUpdateInvalid"))?;
    if record.schema != 1
        || !record.target.valid()
        || ![
            "prepared",
            "committed",
            "rollbackPending",
            "rolledBack",
            "removed",
        ]
        .contains(&record.phase.as_str())
        || record.initiator.is_empty()
        || record.initiator.len() > 256
    {
        return Err(fail("componentUpdateInvalid"));
    }
    owned_location(&record.module)?;
    if let Some(old) = &record.before_registration {
        owned_location(&old.module)?;
    }
    if let Some(origin) = &record.origin {
        if origin.registration.is_some() != origin.hash.is_some()
            || origin.engine.as_ref().is_some_and(|d| !d.valid())
            || origin
                .hash
                .as_ref()
                .is_some_and(|h| h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            return Err(fail("componentOriginInvalid"));
        }
        if let Some(old) = &origin.registration {
            owned_location(&old.module)?;
        }
    }
    Ok(Some(record))
}
fn save_core_update(record: &CoreUpdate) -> windows::core::Result<()> {
    let file = core_update_file()?;
    let base = file
        .parent()
        .ok_or_else(|| fail("componentUpdateInvalid"))?;
    protected(base)?;
    if present(&file)? {
        protected(&file)?;
    }
    let data = serde_json::to_vec(record).map_err(|_| fail("componentUpdateInvalid"))?;
    if data.len() > 128 * 1024 {
        return Err(fail("componentUpdateLimit"));
    }
    let temp = base.join(format!("component-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut out = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|_| fail("componentUpdateWrite"))?;
        out.write_all(&data)
            .map_err(|_| fail("componentUpdateWrite"))?;
        out.sync_all().map_err(|_| fail("componentUpdateWrite"))?;
        drop(out);
        protected(&temp)?;
        std::fs::rename(&temp, &file).map_err(|_| fail("componentUpdateWrite"))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}
pub fn core_update_pending() -> windows::core::Result<bool> {
    Ok(read_removal()?.is_some_and(|r| !r.complete)
        || read_core_update()?
            .is_some_and(|r| !["committed", "rolledBack", "removed"].contains(&r.phase.as_str())))
}
pub fn can_repair_bundle(proxy: &[u8], core: &[u8]) -> windows::core::Result<bool> {
    if let Some(record) = read_removal()?
        && !record.complete
    {
        let descriptor = current_engine()?;
        return Ok(record.initiator == crate::transport::current_user_sid()?
            && !crate::transport::same_process_alive(record.updater)?
            && registration::removal_matches(record.registration.as_ref())?
            && descriptor
                .as_ref()
                .is_none_or(|now| record.descriptor.as_ref() == Some(now)));
    }
    let Some(record) = read_core_update()? else {
        return Ok(false);
    };
    if record.phase == "committed"
        || record.phase == "rolledBack"
        || record.phase == "removed"
        || record.initiator != crate::transport::current_user_sid()?
        || record.target.proxy_sha256 != version(proxy)
        || record.target.core_sha256 != version(core)
        || crate::transport::same_process_alive(record.updater)?
    {
        return Ok(false);
    }
    let descriptor = current_engine()?;
    let registered = registration::owned_module()?;
    Ok((descriptor.as_ref() == record.before_engine.as_ref()
        || descriptor.as_ref() == Some(&record.target))
        && (registered.as_deref() == Some(record.module.as_str())
            || registered.as_deref()
                == record
                    .before_registration
                    .as_ref()
                    .map(|r| r.module.as_str())))
}
fn rollback_core_update(record: &mut CoreUpdate) -> windows::core::Result<()> {
    let current = current_engine()?;
    if current.as_ref() != record.before_engine.as_ref() && current.as_ref() != Some(&record.target)
    {
        return Err(fail("descriptorChanged"));
    }
    if let (Some(old), Some(hash)) = (&record.before_registration, &record.before_hash)
        && installer_module_hash(&old.module)? != *hash
    {
        return Err(fail("rollbackCoreIdentity"));
    }
    record.phase = "rollbackPending".into();
    save_core_update(record)?;
    registration::restore_installer_machine(record.before_registration.as_ref(), &record.module)?;
    if current.as_ref() != record.before_engine.as_ref() {
        publish_engine(current.as_ref(), record.before_engine.as_ref())?;
    }
    record.phase = "rolledBack".into();
    save_core_update(record)
}
/// The caller holds the existing machine maintenance lock. This extends its
/// protected component transaction, never user/HKCU files or arbitrary paths.
pub fn proxy_restart_required(proxy: &[u8], core: &[u8]) -> windows::core::Result<bool> {
    if let Some(record) = read_core_update()?
        && record.phase == "committed"
        && record.target.proxy_sha256 == version(proxy)
        && record.target.core_sha256 == version(core)
    {
        return Ok(record
            .before_registration
            .as_ref()
            .is_none_or(|before| before.module != record.module));
    }
    Ok(!matches!(matches_payload(proxy), Ok(Some(true))))
}
fn update_origin(
    prior: Option<CoreUpdate>,
    target: &crate::lifecycle::CurrentEngine,
    before: CoreOrigin,
) -> CoreOrigin {
    prior
        .filter(|r| r.phase == "committed" && r.target.proxy_sha256 == target.proxy_sha256)
        .and_then(|r| r.origin)
        .unwrap_or(before)
}
pub fn install_bundle(proxy: &[u8], core: &[u8], expected_sid: &str) -> windows::core::Result<()> {
    registration::administrator()?;
    if expected_sid != crate::transport::current_user_sid()? {
        return Err(fail("initiatingUserChanged"));
    }
    recover_removal(expected_sid)?;
    if let Some(mut prior) = read_core_update()?
        && prior.phase != "committed"
        && prior.phase != "rolledBack"
        && prior.phase != "removed"
    {
        if prior.initiator != expected_sid {
            return Err(fail("componentUpdateOwner"));
        }
        if prior.target.proxy_sha256 == version(proxy)
            && prior.target.core_sha256 == version(core)
            && current_engine()?.as_ref() == Some(&prior.target)
            && matches!(matches_bundle(proxy, core), Ok(Some(true)))
        {
            prior.phase = "committed".into();
            save_core_update(&prior)?;
            return Ok(());
        }
        rollback_core_update(&mut prior)?;
    }
    let before_registration = registration::snapshot_installer_machine()?;
    let before_hash = before_registration
        .as_ref()
        .map(|old| installer_module_hash(&old.module))
        .transpose()?;
    let before_engine = current_engine()?;
    let (module, target) = prepare_engine(proxy, core)?;
    // Carry one original rollback anchor across compatible core-only updates.
    // A proxy migration creates a fresh anchor; normal failure rollback still uses immediate before.
    let origin = update_origin(
        read_core_update()?,
        &target,
        CoreOrigin {
            registration: before_registration.clone(),
            hash: before_hash.clone(),
            engine: before_engine.clone(),
        },
    );
    let mut record = CoreUpdate {
        schema: 1,
        phase: "prepared".into(),
        initiator: expected_sid.into(),
        updater: crate::transport::identity(std::process::id())?,
        before_registration,
        before_hash,
        before_engine,
        origin: Some(origin),
        target,
        module,
        error: None,
    };
    save_core_update(&record)?;
    let result = (|| {
        match record
            .before_registration
            .as_ref()
            .map(|r| r.module.as_str())
        {
            Some(old) if old != record.module => {
                registration::replace_machine(&record.module, old)?
            }
            None => registration::register_machine(&record.module)?,
            _ => (),
        }
        publish_engine(record.before_engine.as_ref(), Some(&record.target))?;
        if !matches!(matches_bundle(proxy, core), Ok(Some(true))) {
            return Err(fail("componentConfirmation"));
        }
        record.phase = "committed".into();
        save_core_update(&record)
    })();
    if let Err(cause) = result {
        record.error = Some(format!("{:08x}", cause.code().0 as u32));
        let _ = save_core_update(&record);
        if rollback_core_update(&mut record).is_err() {
            return Err(fail("componentRollbackPending"));
        }
        return Err(cause);
    }
    Ok(())
}

/// Fixed rollback entry; the caller holds the original machine maintenance lock.
/// No paths or replacement bytes are supplied by the frontend.
pub fn rollback_bundle(proxy: &[u8], core: &[u8], expected_sid: &str) -> windows::core::Result<()> {
    registration::administrator()?;
    if expected_sid != crate::transport::current_user_sid()? {
        return Err(fail("initiatingUserChanged"));
    }
    let mut record = read_core_update()?.ok_or_else(|| fail("componentRollbackMissing"))?;
    if record.phase != "committed"
        || record.initiator != expected_sid
        || record.target.proxy_sha256 != version(proxy)
        || record.target.core_sha256 != version(core)
        || !matches!(matches_bundle(proxy, core), Ok(Some(true)))
    {
        return Err(fail("componentRollbackChanged"));
    }
    let origin = record
        .origin
        .clone()
        .ok_or_else(|| fail("componentRollbackMissing"))?;
    if let (Some(old), Some(hash)) = (&origin.registration, &origin.hash)
        && installer_module_hash(&old.module)? != *hash
    {
        return Err(fail("componentRollbackIdentity"));
    }
    record.before_registration = origin.registration;
    record.before_hash = origin.hash;
    record.before_engine = origin.engine;
    record.updater = crate::transport::identity(std::process::id())?;
    record.error = Some("ownerRollback".into());
    rollback_core_update(&mut record)
}

/// Read/replace the one protected deployment record, never a caller-selected path.
pub fn installer_record() -> windows::core::Result<Option<Vec<u8>>> {
    let base = program_files()?.join("Sayweave").join("TextService");
    let file = base.join("installer.json");
    if !present(&file)? {
        return Ok(None);
    }
    protected(&program_files()?)?;
    protected(base.parent().unwrap())?;
    protected(&base)?;
    protected(&file)?;
    if std::fs::metadata(&file)
        .map_err(|_| fail("installerReadFailed"))?
        .len()
        > 128 * 1024
    {
        return Err(fail("installerRecordLimit"));
    }
    Ok(Some(
        std::fs::read(file).map_err(|_| fail("installerReadFailed"))?,
    ))
}
pub fn write_installer_record(bytes: &[u8]) -> windows::core::Result<()> {
    registration::administrator()?;
    if bytes.is_empty() || bytes.len() > 128 * 1024 {
        return Err(fail("installerRecordLimit"));
    }
    let app = program_files()?.join("Sayweave");
    ensure_directory(&app)?;
    let base = app.join("TextService");
    ensure_directory(&base)?;
    let target = base.join("installer.json");
    if present(&target)? {
        protected(&target)?;
    }
    let temp = base.join(format!("installer-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|_| fail("installerWriteFailed"))?;
        file.write_all(bytes)
            .map_err(|_| fail("installerWriteFailed"))?;
        file.sync_all().map_err(|_| fail("installerWriteFailed"))?;
        drop(file);
        protected(&temp)?;
        unsafe {
            MoveFileExW(
                PCWSTR(wide(&temp.to_string_lossy()).as_ptr()),
                PCWSTR(wide(&target.to_string_lossy()).as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )?;
        }
        protected(&target)
    })();
    if result.is_err() && present(&temp).unwrap_or(false) {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
pub fn installer_module_hash(module: &str) -> windows::core::Result<String> {
    let file = owned_path(module)?;
    let meta = std::fs::metadata(&file).map_err(|_| fail("payloadReadFailed"))?;
    if !(1024..=32 * 1024 * 1024).contains(&meta.len()) {
        return Err(fail("invalidPayload"));
    }
    let bytes = std::fs::read(&file).map_err(|_| fail("payloadReadFailed"))?;
    if !bytes.starts_with(b"MZ") {
        return Err(fail("invalidPayload"));
    }
    Ok(version(&bytes))
}
pub fn commit_installer(
    module: &str,
    expected: Option<&str>,
    language: u16,
) -> windows::core::Result<()> {
    registration::administrator()?;
    let hash = installer_module_hash(module)?;
    if owned_location(module)?
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        != Some(hash.as_str())
    {
        return Err(fail("payloadMismatch"));
    }
    let current = registration::owned_module()?;
    if current.as_deref() != expected {
        return Err(fail("registrationChanged"));
    }
    match current {
        Some(old) if old == module => Ok(()),
        Some(old) => registration::replace_machine(module, &old),
        None => registration::register_installer_machine(module, language),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn original_rollback_anchor_survives_two_core_updates_but_not_proxy_migration() {
        let descriptor = |core: char, proxy: char| crate::lifecycle::CurrentEngine {
            schema_version: 1,
            revision: uuid::Uuid::new_v4().to_string(),
            proxy_sha256: proxy.to_string().repeat(64),
            core_sha256: core.to_string().repeat(64),
            abi: 1,
            input_protocol: 9,
            platform: "x86_64-pc-windows-msvc".into(),
        };
        let a = descriptor('a', 'd');
        let b = descriptor('b', 'd');
        let c = descriptor('c', 'd');
        let empty = CoreOrigin {
            registration: None,
            hash: None,
            engine: None,
        };
        let make = |target, origin| CoreUpdate {
            schema: 1,
            phase: "committed".into(),
            initiator: "owner".into(),
            updater: crate::transport::Identity {
                pid: 1,
                session: 1,
                created: 1,
            },
            before_registration: None,
            before_hash: None,
            before_engine: None,
            origin: Some(origin),
            target,
            module: "unused".into(),
            error: None,
        };
        let first = update_origin(
            Some(make(a.clone(), empty)),
            &b,
            CoreOrigin {
                registration: None,
                hash: None,
                engine: Some(a),
            },
        );
        let second = update_origin(
            Some(make(b.clone(), first)),
            &c,
            CoreOrigin {
                registration: None,
                hash: None,
                engine: Some(b),
            },
        );
        assert!(second.engine.is_none());
        let migration = update_origin(
            Some(make(c.clone(), second)),
            &descriptor('a', 'e'),
            CoreOrigin {
                registration: None,
                hash: None,
                engine: Some(c.clone()),
            },
        );
        assert_eq!(migration.engine, Some(c));
    }
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
