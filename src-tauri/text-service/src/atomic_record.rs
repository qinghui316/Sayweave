//! Bounded replacement of a durable journal. Callers own path policy and transaction locks.
use std::{
    io::Write,
    path::Path,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Security::{Authorization::*, *},
        Storage::FileSystem::*,
        System::Threading::*,
    },
    core::{Error, HRESULT, PCWSTR, Result},
};

pub const RETRY_DELAYS: [u64; 4] = [50, 100, 200, 400];
pub fn budget() -> Instant {
    Instant::now() + Duration::from_secs(1)
}
pub fn stage<T>(name: &str, result: Result<T>) -> Result<T> {
    result.map_err(|e| {
        Error::new(
            e.code(),
            format!("journal.{name}:{}:{}", e.code().0 as u32, e.message()),
        )
    })
}
fn io<T>(name: &str, result: std::io::Result<T>) -> Result<T> {
    stage(
        name,
        result.map_err(|e| {
            Error::new(
                HRESULT::from_win32(e.raw_os_error().unwrap_or(1) as u32),
                "nativeIo",
            )
        }),
    )
}
fn fail(name: &str) -> Error {
    Error::new(E_ACCESSDENIED, name)
}
fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(Some(0))
        .collect()
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
struct Descriptor(PSECURITY_DESCRIPTOR);
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0.0)));
        }
    }
}

// Get security and file identity through the same handle, then close it before rename.
// AccessCheck distinguishes a real DACL denial from sharing failures, without needing
// DELETE access to open an already shared destination.
#[derive(PartialEq, Eq)]
struct Stamp {
    id: (u32, u32, u32),
    attributes: u32,
    security: Vec<u8>,
}
fn stamp(path: &Path, desired: u32) -> Result<Option<Stamp>> {
    unsafe {
        let handle = match CreateFileW(
            PCWSTR(wide(path).as_ptr()),
            READ_CONTROL.0 | FILE_READ_ATTRIBUTES.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            None,
        ) {
            Ok(h) => Handle(h),
            Err(e)
                if [
                    ERROR_FILE_NOT_FOUND.to_hresult(),
                    ERROR_PATH_NOT_FOUND.to_hresult(),
                ]
                .contains(&e.code()) =>
            {
                return Ok(None);
            }
            Err(e) => return Err(e),
        };
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        GetFileInformationByHandle(handle.0, &mut info)?;
        if info.dwFileAttributes & (FILE_ATTRIBUTE_REPARSE_POINT.0 | FILE_ATTRIBUTE_READONLY.0) != 0
        {
            return Err(fail("journalPathAttributes"));
        }
        let mut sd = PSECURITY_DESCRIPTOR::default();
        GetSecurityInfo(
            handle.0,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            Some(&mut sd),
        )
        .ok()?;
        let sd = Descriptor(sd);
        let mut primary = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_QUERY | TOKEN_DUPLICATE,
            &mut primary,
        )?;
        let primary = Handle(primary);
        let mut token = HANDLE::default();
        DuplicateToken(primary.0, SecurityImpersonation, &mut token)?;
        let token = Handle(token);
        let mapping = GENERIC_MAPPING {
            GenericRead: FILE_GENERIC_READ.0,
            GenericWrite: FILE_GENERIC_WRITE.0,
            GenericExecute: FILE_GENERIC_EXECUTE.0,
            GenericAll: FILE_ALL_ACCESS.0,
        };
        // The requested file rights do not consume privileges. Keep a bounded aligned buffer
        // nevertheless; insufficient privilege-buffer space fails closed.
        let mut privileges = [0u64; 128];
        let mut length = std::mem::size_of_val(&privileges) as u32;
        let mut granted = 0;
        let mut allowed = windows::core::BOOL(0);
        AccessCheck(
            sd.0,
            token.0,
            desired,
            &mapping,
            Some(privileges.as_mut_ptr().cast()),
            &mut length,
            &mut granted,
            &mut allowed,
        )?;
        if !allowed.as_bool() {
            return Err(fail("journalPermissionDenied"));
        }
        let security = std::slice::from_raw_parts(
            sd.0.0.cast::<u8>(),
            GetSecurityDescriptorLength(sd.0) as usize,
        )
        .to_vec();
        Ok(Some(Stamp {
            id: (
                info.dwVolumeSerialNumber,
                info.nFileIndexHigh,
                info.nFileIndexLow,
            ),
            attributes: info.dwFileAttributes,
            security,
        }))
    }
}
/// Ordinary receipt files must remain owned by the initiating user.
pub fn verify_user_owner(path: &Path) -> Result<()> {
    if !path
        .try_exists()
        .map_err(|e| Error::new(E_ACCESSDENIED, e.to_string()))?
    {
        return Ok(());
    }
    unsafe {
        let mut owner = PSID::default();
        let mut sd = PSECURITY_DESCRIPTOR::default();
        GetNamedSecurityInfoW(
            PCWSTR(wide(path).as_ptr()),
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            None,
            None,
            &mut sd,
        )
        .ok()?;
        let _sd = Descriptor(sd);
        let mut name = windows::core::PWSTR::null();
        ConvertSidToStringSidW(owner, &mut name)?;
        let text = name.to_string();
        let _ = LocalFree(Some(HLOCAL(name.0.cast())));
        if text? != crate::transport::current_user_sid()? {
            return Err(fail("journalOwnerChanged"));
        }
        Ok(())
    }
}
fn read_journal(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    io("read.open", std::fs::File::open(path))?
        .take(128 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| {
            Error::new(
                HRESULT::from_win32(e.raw_os_error().unwrap_or(1) as u32),
                "journal.read",
            )
        })?;
    if bytes.len() > 128 * 1024 {
        return Err(fail("journalSizeLimit"));
    }
    Ok(bytes)
}
fn contention(e: &Error) -> bool {
    [
        ERROR_ACCESS_DENIED,
        ERROR_SHARING_VIOLATION,
        ERROR_LOCK_VIOLATION,
    ]
    .iter()
    .any(|code| code.to_hresult() == e.code())
}

/// All preparation errors are immediate. Only replacement is retried, with the same temp.
/// `deadline` is the enclosing phase deadline, never extended by a retry.
pub fn write(
    path: &Path,
    bytes: &[u8],
    deadline: Instant,
    validate: impl Fn(&Path) -> Result<()>,
) -> Result<()> {
    if bytes.is_empty() || bytes.len() > 128 * 1024 {
        return Err(fail("journalSizeLimit"));
    }
    let parent = path.parent().ok_or_else(|| fail("journalParent"))?;
    stage("path.validate", validate(path))?;
    let temp = parent.join(format!("journal-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = io(
            "temp.create",
            std::fs::OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(&temp),
        )?;
        io("temp.write", file.write_all(bytes))?;
        io("temp.flush", file.sync_all())?;
        drop(file);
        stage("temp.validate", validate(&temp))?;
        let parent_before = stage(
            "parent.permission",
            stamp(parent, FILE_ADD_FILE.0 | FILE_DELETE_CHILD.0),
        )?
        .ok_or_else(|| fail("journalParentMissing"))?;
        let target_before = stage("target.permission", stamp(path, DELETE.0))?;
        let temp_before = stage("temp.permission", stamp(&temp, DELETE.0))?
            .ok_or_else(|| fail("journalTempMissing"))?;
        let original = if target_before.is_some() {
            Some(stage("target.read", read_journal(path))?)
        } else {
            None
        };
        let validate_replace = || -> Result<()> {
            validate(path)?;
            validate(&temp)?;
            if stamp(parent, FILE_ADD_FILE.0 | FILE_DELETE_CHILD.0)?.as_ref()
                != Some(&parent_before)
                || stamp(path, DELETE.0)? != target_before
                || stamp(&temp, DELETE.0)?.as_ref() != Some(&temp_before)
                || stage("temp.read", read_journal(&temp))? != bytes
                || (target_before.is_some()
                    && Some(stage("target.read", read_journal(path))?) != original)
            {
                return Err(fail("journalIdentityChanged"));
            }
            Ok(())
        };
        stage("replace.validate", validate_replace())?;
        if Instant::now() >= deadline {
            return Err(Error::new(
                ERROR_TIMEOUT.to_hresult(),
                "journal.target.replace.deadline",
            ));
        }
        for delay in RETRY_DELAYS
            .into_iter()
            .map(Some)
            .chain(std::iter::once(None))
        {
            let replace = unsafe {
                MoveFileExW(
                    PCWSTR(wide(&temp).as_ptr()),
                    PCWSTR(wide(path).as_ptr()),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            };
            match replace {
                Ok(()) => {
                    stage("result.validate", validate(path))?;
                    if stage("result.read", read_journal(path))? != bytes {
                        return Err(fail("journalResultUnconfirmed"));
                    }
                    return Ok(());
                }
                Err(e) => {
                    if !contention(&e) || delay.is_none() {
                        return stage("target.replace", Err(e));
                    }
                    stage("retry.validate", validate_replace())?;
                    let wait = Duration::from_millis(delay.expect("checked retry"));
                    if Instant::now()
                        .checked_add(wait)
                        .is_none_or(|next| next > deadline)
                    {
                        return stage("target.replace.deadline", Err(e));
                    }
                    std::thread::sleep(wait);
                    stage("retry.validate", validate_replace())?;
                    if Instant::now() > deadline {
                        return stage("target.replace.deadline", Err(e));
                    }
                }
            }
        }
        unreachable!()
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{os::windows::fs::OpenOptionsExt, sync::mpsc};
    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("sayweave-journal-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn file(&self) -> std::path::PathBuf {
            self.0.join("receipt.json")
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn validate(path: &Path) -> Result<()> {
        for ancestor in path.ancestors() {
            if let Ok(meta) = std::fs::symlink_metadata(ancestor) {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
                    return Err(fail("linkedFixture"));
                }
            }
        }
        Ok(())
    }
    #[test]
    fn transient_reader_releases_same_durable_candidate_and_confirms_bytes() {
        let f = Fixture::new();
        let p = f.file();
        std::fs::write(&p, b"old").unwrap();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .open(&p)
            .unwrap();
        let (tx, rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            tx.send(()).unwrap();
            write(
                &p,
                b"next",
                Instant::now() + Duration::from_secs(2),
                validate,
            )
        });
        rx.recv().unwrap();
        std::thread::sleep(Duration::from_millis(90));
        drop(held);
        writer.join().unwrap().unwrap();
        assert_eq!(std::fs::read(f.file()).unwrap(), b"next");
        assert_eq!(std::fs::read_dir(&f.0).unwrap().count(), 1);
    }
    #[test]
    fn permanent_reader_and_phase_deadline_preserve_old_record() {
        let f = Fixture::new();
        let p = f.file();
        std::fs::write(&p, b"old").unwrap();
        let _held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0)
            .open(&p)
            .unwrap();
        let began = Instant::now();
        let e = write(&p, b"next", began + Duration::from_millis(40), validate).unwrap_err();
        assert!(e.message().contains("deadline"), "{e}");
        assert!(began.elapsed() < Duration::from_millis(250));
        let began = Instant::now();
        let e = write(&p, b"next", began + Duration::from_secs(2), validate).unwrap_err();
        assert!(e.message().contains("target.replace"), "{e}");
        assert!(began.elapsed() >= Duration::from_millis(750));
        assert!(began.elapsed() < Duration::from_secs(2));
        assert_eq!(std::fs::read(&p).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(&f.0).unwrap().count(), 1);
    }
    #[test]
    fn readonly_and_validation_failure_are_not_retried() {
        let f = Fixture::new();
        let p = f.file();
        std::fs::write(&p, b"old").unwrap();
        let original_permissions = std::fs::metadata(&p).unwrap().permissions();
        let mut permissions = original_permissions.clone();
        permissions.set_readonly(true);
        std::fs::set_permissions(&p, permissions.clone()).unwrap();
        let began = Instant::now();
        let e = write(&p, b"next", began + Duration::from_secs(2), validate).unwrap_err();
        assert!(e.message().contains("permission"), "{e}");
        assert!(began.elapsed() < Duration::from_millis(250));
        std::fs::set_permissions(&p, original_permissions).unwrap();
        let calls = std::cell::Cell::new(0);
        assert!(
            write(&p, b"next", budget(), |_| {
                calls.set(calls.get() + 1);
                Err(fail("denied"))
            })
            .is_err()
        );
        assert_eq!(calls.get(), 1);
        assert_eq!(std::fs::read(&p).unwrap(), b"old");
    }
    #[test]
    fn retry_rejects_changed_destination_instead_of_overwriting_new_record() {
        let f = Fixture::new();
        let p = f.file();
        std::fs::write(&p, b"old").unwrap();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
            .open(&p)
            .unwrap();
        let calls = std::cell::Cell::new(0);
        let result = write(&p, b"next", budget(), |candidate| {
            validate(candidate)?;
            calls.set(calls.get() + 1);
            if calls.get() == 5 {
                std::fs::write(&p, b"external").unwrap();
            }
            Ok(())
        });
        drop(held);
        assert!(result.unwrap_err().message().contains("IdentityChanged"));
        assert_eq!(std::fs::read(&p).unwrap(), b"external");
    }
}
