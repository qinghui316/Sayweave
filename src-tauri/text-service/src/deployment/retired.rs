//! Runs only inside the caller's machine-maintenance lock. No directory enumeration.
use super::*;
use std::{
    collections::BTreeSet,
    io::Read,
    os::windows::{fs::OpenOptionsExt, io::AsRawHandle},
    time::Instant,
};

/// Installer-owned recovery identities, validated before deployment may collect anything.
#[derive(Default)]
pub struct RetainedComponents {
    paths: BTreeSet<PathBuf>,
    hashes: BTreeSet<String>,
}
impl RetainedComponents {
    pub fn module(&mut self, path: &str, hash: Option<&str>) -> windows::core::Result<()> {
        let path = owned_location(path)?;
        if let Some(hash) = hash {
            self.hash(hash)?;
        }
        // Pin missing entries as well: recovery, not GC, decides whether their absence is valid.
        self.paths.insert(path);
        Ok(())
    }
    fn hash(&mut self, hash: &str) -> windows::core::Result<()> {
        if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(fail("retentionIdentityInvalid"));
        }
        self.hashes.insert(hash.to_ascii_lowercase());
        Ok(())
    }
    pub fn engine(&mut self, value: &crate::lifecycle::CurrentEngine) -> windows::core::Result<()> {
        if !value.valid() {
            return Err(fail("retentionDescriptorInvalid"));
        }
        self.hash(&value.proxy_sha256)?;
        self.hash(&value.core_sha256)
    }
    fn contains(&self, path: &Path) -> bool {
        self.paths.iter().any(|p| {
            p.to_string_lossy()
                .eq_ignore_ascii_case(&path.to_string_lossy())
        }) || path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|p| p.to_str())
            .is_some_and(|h| self.hashes.contains(&h.to_ascii_lowercase()))
    }
}
fn retain_local(pins: &mut RetainedComponents) -> windows::core::Result<()> {
    if let Some(path) = registration::owned_module()? {
        pins.module(&path, None)?;
    }
    if let Some(engine) = current_engine()? {
        pins.engine(&engine)?;
    }
    if let Some(record) = read_core_update()? {
        pins.module(&record.module, Some(&record.target.proxy_sha256))?;
        pins.engine(&record.target)?;
        if let Some(old) = &record.before_registration {
            pins.module(&old.module, record.before_hash.as_deref())?;
        }
        if let Some(engine) = &record.before_engine {
            pins.engine(engine)?;
        }
        if let Some(origin) = &record.origin {
            if let Some(old) = &origin.registration {
                pins.module(&old.module, origin.hash.as_deref())?;
            }
            if let Some(engine) = &origin.engine {
                pins.engine(engine)?;
            }
        }
    }
    if let Some(record) = read_removal()?
        && !record.complete
    {
        if let Some(old) = &record.registration {
            pins.module(&old.module, None)?;
        }
        if let Some(engine) = &record.descriptor {
            pins.engine(engine)?;
        }
    }
    Ok(())
}
// Hash and delete through one exclusive handle. A mapped image is rejected by Windows'
// disposition operation even when its original loader handle has already been closed.
fn delete_verified(path: &Path, expected: &str) -> windows::core::Result<bool> {
    protected(path.parent().ok_or_else(|| fail("retiredParent"))?)?;
    protected(path)?;
    let mut file = match std::fs::OpenOptions::new()
        .access_mode(GENERIC_READ.0 | DELETE.0)
        .share_mode(0)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0)
        .open(path)
    {
        Ok(file) => file,
        Err(_) => return Ok(false), // Busy or otherwise unprovable. Keep the managed entry.
    };
    let handle = HANDLE(file.as_raw_handle());
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(handle, &mut info)?;
    }
    if info.nNumberOfLinks != 1
        || info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT.0 | FILE_ATTRIBUTE_DIRECTORY.0)
            != 0
        || info.nFileSizeHigh != 0
        || !(1024..=32 * 1024 * 1024).contains(&info.nFileSizeLow)
    {
        return Ok(false);
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| fail("retiredRead"))?;
    if !bytes.starts_with(b"MZ") || version(&bytes) != expected {
        return Ok(false);
    }
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    if unsafe {
        SetFileInformationByHandle(
            handle,
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            std::mem::size_of_val(&disposition) as u32,
        )
    }
    .is_err()
    {
        return Ok(false);
    }
    drop(file);
    Ok(!present(path)?)
}
fn net_size(managed: &[String], required: &[String]) -> usize {
    managed
        .iter()
        .chain(required)
        .map(|p| p.to_ascii_lowercase())
        .collect::<BTreeSet<_>>()
        .len()
}
/// `base` is fixed in production. Isolated tests use a protected fixture and the same collector.
fn collect(
    base: &Path,
    mut managed: Vec<String>,
    required: &[String],
    pins: &RetainedComponents,
    deadline: Instant,
) -> windows::core::Result<Vec<String>> {
    protected(base)?;
    let mut kept = Vec::new();
    for value in managed.drain(..) {
        let path = local_path(&value)?;
        // Explicitly enforce containment again at the deletion boundary.
        if path.parent().and_then(|p| p.parent()) != Some(base)
            || path.file_name() != Some(std::ffi::OsStr::new("sayweave_text_service.dll"))
        {
            return Err(fail("retiredPathInvalid"));
        }
        if pins.contains(&path) || Instant::now() >= deadline {
            kept.push(value);
            continue;
        }
        let Some(hash) = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
        else {
            return Err(fail("retiredPathInvalid"));
        };
        if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            kept.push(value);
            continue;
        }
        // A missing file is a valid interrupted-delete outcome, only under a verified parent.
        if present(path.parent().unwrap())? {
            protected(path.parent().unwrap())?;
        }
        let removed = !present(&path)? || delete_verified(&path, &hash.to_ascii_lowercase())?;
        if !removed {
            kept.push(value);
        } else {
            // Remove only an empty, already validated hash directory, never its other contents.
            let _ = std::fs::remove_dir(path.parent().unwrap());
        }
    }
    // Deletion is already durable. If this write fails, old manifest entries remain and are
    // safely reconciled on the next maintenance; files are never unlisted before deletion.
    write_managed_until(base, &kept, deadline)?;
    if net_size(&kept, required) > 64 {
        return Err(fail("managedVersionLimit:retainedOrBusy"));
    }
    Ok(kept)
}
pub(super) fn prepare(
    proxy: &[u8],
    core: &[u8],
    mut pins: RetainedComponents,
    deadline: Instant,
) -> windows::core::Result<()> {
    registration::administrator()?;
    for bytes in [proxy, core] {
        if bytes.len() < 1024 || bytes.len() > 32 * 1024 * 1024 || !bytes.starts_with(b"MZ") {
            return Err(fail("invalidPayload"));
        }
        pins.hash(&version(bytes))?;
    }
    // Complete retention validation BEFORE any deletion.
    retain_local(&mut pins)?;
    let root = program_files()?;
    protected(&root)?;
    let app = root.join("Sayweave");
    ensure_directory(&app)?;
    let base = app.join("TextService");
    ensure_directory(&base)?;
    let mut required = [proxy, core]
        .map(|bytes| {
            base.join(version(bytes))
                .join("sayweave_text_service.dll")
                .to_string_lossy()
                .into_owned()
        })
        .to_vec();
    if let Some(old) = registration::owned_module()? {
        required.push(local_path(&old)?.to_string_lossy().into_owned());
    }
    collect(&base, managed_modules(&base)?, &required, &pins, deadline)?;
    if Instant::now() >= deadline {
        return Err(fail("componentPrepareDeadline"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paired_capacity_counts_only_net_additions() {
        let list = (0..64).map(|n| format!("module{n}")).collect::<Vec<_>>();
        assert_eq!(net_size(&list, &["module0".into(), "MODULE1".into()]), 64);
        assert_eq!(net_size(&list, &["proxy".into(), "core".into()]), 66);
        assert_eq!(net_size(&list[..62], &["proxy".into(), "core".into()]), 64);
        assert_eq!(net_size(&list, &["proxy".into(), "proxy".into()]), 65);
    }
    #[test]
    fn retention_is_typed_and_keeps_descriptor_and_path_identities() {
        let mut pins = RetainedComponents::default();
        assert!(pins.hash("not-a-hash").is_err());
        assert!(
            pins.module("C:\\Users\\Public\\sayweave_text_service.dll", None)
                .is_err()
        );
        pins.engine(&crate::lifecycle::CurrentEngine {
            schema_version: 1,
            revision: uuid::Uuid::new_v4().to_string(),
            proxy_sha256: "a".repeat(64),
            core_sha256: "b".repeat(64),
            abi: 1,
            input_protocol: 10,
            platform: "x86_64-pc-windows-msvc".into(),
        })
        .unwrap();
        let base = program_files()
            .unwrap()
            .join("Sayweave")
            .join("TextService");
        assert!(pins.contains(&base.join("a".repeat(64)).join("sayweave_text_service.dll")));
        assert!(pins.contains(&base.join("b".repeat(64)).join("sayweave_text_service.dll")));
    }
    #[test]
    #[ignore = "explicit elevated isolated deployment regression; no system registration or real journal changes"]
    fn protected_collection_and_journal_contention() {
        registration::administrator().unwrap();
        let app = program_files().unwrap().join("Sayweave");
        ensure_directory(&app).unwrap();
        let base = app.join(format!("MaintenanceTest-{}", uuid::Uuid::new_v4()));
        ensure_directory(&base).unwrap();
        println!("isolated_fixture={}", base.display());
        let payload = |n: u8| {
            let mut b = vec![n; 2048];
            b[..2].copy_from_slice(b"MZ");
            b
        };
        let mut entries = Vec::new();
        for n in 0..64 {
            let bytes = payload(n);
            let dir = base.join(version(&bytes));
            ensure_directory(&dir).unwrap();
            let p = dir.join("sayweave_text_service.dll");
            std::fs::write(&p, bytes).unwrap();
            entries.push(p.to_string_lossy().into_owned());
        }
        write_managed(&base, &entries).unwrap();
        let mut pins = RetainedComponents::default();
        for entry in entries.iter().take(4) {
            pins.paths.insert(PathBuf::from(entry));
        }
        let all_pins = RetainedComponents {
            paths: entries.iter().map(PathBuf::from).collect(),
            hashes: BTreeSet::new(),
        };
        let no_room = collect(
            &base,
            entries.clone(),
            &["new-proxy".into(), "new-core".into()],
            &all_pins,
            Instant::now() + std::time::Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(no_room.message().contains("managedVersionLimit"));
        assert!(entries.iter().all(|p| Path::new(p).exists()));
        assert!(
            collect(
                &base,
                vec![app.join("outside.dll").to_string_lossy().into_owned()],
                &[],
                &pins,
                Instant::now() + std::time::Duration::from_secs(5)
            )
            .is_err()
        );
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .open(&entries[4])
            .unwrap();
        // A manifest hash mismatch must remain managed, never be deleted.
        std::fs::write(&entries[5], payload(90)).unwrap();
        // Missing-file crash recovery; the directory remains protected.
        std::fs::remove_file(&entries[6]).unwrap();
        let required = vec![
            base.join(version(&payload(80)))
                .join("sayweave_text_service.dll")
                .to_string_lossy()
                .into_owned(),
            base.join(version(&payload(81)))
                .join("sayweave_text_service.dll")
                .to_string_lossy()
                .into_owned(),
        ];
        let kept = collect(
            &base,
            entries.clone(),
            &required,
            &pins,
            Instant::now() + std::time::Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(kept.len(), 6);
        for p in &entries[..6] {
            assert!(kept.contains(p));
            assert!(Path::new(p).exists());
        }
        assert_eq!(net_size(&kept, &required), 8);
        drop(held);
        let kept = collect(
            &base,
            kept,
            &required,
            &pins,
            Instant::now() + std::time::Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(kept.len(), 5);
        // Use a real PE image but never invoke DllMain or alter registration.
        let bytes = std::fs::read(std::env::current_exe().unwrap()).unwrap();
        assert!(bytes.len() < 32 * 1024 * 1024);
        let dir = base.join(version(&bytes));
        ensure_directory(&dir).unwrap();
        let mapped = dir.join("sayweave_text_service.dll");
        std::fs::write(&mapped, &bytes).unwrap();
        use windows::Win32::System::Memory::*;
        let file = std::fs::File::open(&mapped).unwrap();
        unsafe {
            let mapping = CreateFileMappingW(
                HANDLE(file.as_raw_handle()),
                None,
                PAGE_READONLY | SEC_IMAGE,
                0,
                0,
                PCWSTR::null(),
            )
            .unwrap();
            let view = MapViewOfFile(mapping, FILE_MAP_READ, 0, 0, 0);
            assert!(!view.Value.is_null());
            drop(file);
            CloseHandle(mapping).unwrap();
            assert!(!delete_verified(&mapped, &version(&bytes)).unwrap());
            assert!(mapped.exists());
            UnmapViewOfFile(view).unwrap();
        }
        assert!(delete_verified(&mapped, &version(&bytes)).unwrap());
        println!(
            "manifest64=true paired_reservation=true pins_retained=true opened_retained=true mapped_retained=true hash_mismatch_retained=true interrupted_delete_reconciled=true"
        );
        // Same production protected atomic writer under a held reader, released once.
        let journal = base.join("installer.json");
        std::fs::write(&journal, b"old").unwrap();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .open(&journal)
            .unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(100));
                drop(held);
            });
            crate::atomic_record::write(
                &journal,
                b"new",
                Instant::now() + std::time::Duration::from_secs(2),
                validate_journal_path,
            )
            .unwrap();
        });
        assert_eq!(std::fs::read(&journal).unwrap(), b"new");
        println!(
            "protected_reader_released=true replacement_bytes_verified=true real_installer_untouched=true"
        );
        let set_acl = |sddl: &str| unsafe {
            let mut descriptor = PSECURITY_DESCRIPTOR::default();
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(wide(sddl).as_ptr()),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
            .unwrap();
            let mut present_acl = windows::core::BOOL(0);
            let mut defaulted = windows::core::BOOL(0);
            let mut acl = std::ptr::null_mut();
            GetSecurityDescriptorDacl(descriptor, &mut present_acl, &mut acl, &mut defaulted)
                .unwrap();
            SetNamedSecurityInfoW(
                PCWSTR(wide(&journal.to_string_lossy()).as_ptr()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(acl),
                None,
            )
            .ok()
            .unwrap();
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        };
        set_acl("D:P(A;;GR;;;BA)(A;;GA;;;SY)");
        let began = Instant::now();
        let denied = crate::atomic_record::write(
            &journal,
            b"denied",
            began + std::time::Duration::from_secs(2),
            validate_journal_path,
        )
        .unwrap_err();
        assert!(denied.message().contains("PermissionDenied"), "{denied}");
        assert!(began.elapsed() < std::time::Duration::from_millis(250));
        set_acl("D:P(A;;GA;;;BA)(A;;GA;;;SY)(A;;GR;;;BU)");
        assert_eq!(std::fs::read(&journal).unwrap(), b"new");
        println!("permission_denied_without_retry=true old_bytes_retained=true");
        // Only this generated, containment-checked fixture is removed.
        assert_eq!(base.parent(), Some(app.as_path()));
        assert!(
            base.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("MaintenanceTest-")
        );
        std::fs::remove_dir_all(base).unwrap();
    }
}
