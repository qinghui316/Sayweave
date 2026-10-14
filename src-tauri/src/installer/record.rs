use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Component, Path},
};

pub type Result<T> = std::result::Result<T, String>;
pub const MAX_RECORD: u64 = 128 * 1024;
pub const MAX_FILE: u64 = 128 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    BackedUp,
    Prepared,
    ProgramInstalled,
    MachineCommitted,
    UserInitialized,
    Committed,
    RolledBack,
    Uninstalled,
    Uninstalling,
    Recovery,
}
impl Phase {
    pub fn terminal(self) -> bool {
        matches!(self, Self::Committed | Self::RolledBack | Self::Uninstalled)
    }
    pub fn advance(self, next: Self) -> Result<Self> {
        if self == next {
            return Ok(self);
        }
        if matches!(
            (self, next),
            (Self::BackedUp, Self::Prepared)
                | (Self::Prepared, Self::ProgramInstalled)
                | (Self::ProgramInstalled, Self::MachineCommitted)
                | (Self::MachineCommitted, Self::UserInitialized)
                | (Self::UserInitialized, Self::Committed)
        ) {
            return Ok(next);
        }
        if !self.terminal() && matches!(next, Self::Recovery | Self::RolledBack) {
            return Ok(next);
        }
        Err("installerPhaseConflict".into())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Target {
    pub version: String,
    pub commit: String,
    pub product: String,
    pub payload_hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core_hash: Option<String>,
    pub main_hash: String,
    pub helper_hash: String,
    pub uninstall_hash: String,
}
impl Target {
    pub fn validate(&self) -> Result<()> {
        if !hex(&self.commit, 40)
            || !hex(&self.payload_hash, 64)
            || self.core_hash.as_ref().is_some_and(|h| !hex(h, 64))
            || !hex(&self.main_hash, 64)
            || !hex(&self.helper_hash, 64)
            || !hex(&self.uninstall_hash, 64)
            || self.version.is_empty()
            || self.version.len() > 64
            || self.product.is_empty()
            || self.product.len() > 128
            || self.product.contains(['\\', '/', '\0'])
        {
            return Err("installerTargetInvalid".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Process {
    pub pid: u32,
    pub session: u32,
    pub created: u64,
    pub sid: String,
    pub image_hash: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FileBackup {
    pub path: String,
    pub backup: String,
    pub before_hash: Option<String>,
    pub installed_hash: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Value {
    pub name: String,
    pub kind: u32,
    pub bytes: Vec<u8>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyBackup {
    pub path: String,
    pub existed: bool,
    pub values: Vec<Value>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UserRecord {
    pub schema_version: u8,
    pub transaction: String,
    pub phase: Phase,
    pub target: Target,
    pub installer: Process,
    pub server: Option<Process>,
    pub install_dir: String,
    pub language: u16,
    pub user_choice: Option<bool>,
    #[serde(default)]
    pub user_initialization_started: bool,
    pub user_initialized: bool,
    pub files: Vec<FileBackup>,
    pub keys: Vec<KeyBackup>,
    pub error: Option<String>,
    #[serde(default)]
    pub last_failure: Option<Failure>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Failure {
    pub phase: Phase,
    pub code: String,
}
pub fn hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_hexdigit())
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn read(path: &Path, limit: u64) -> Result<Vec<u8>> {
    reject_links(path)?;
    let meta = fs::metadata(path).map_err(|_| "installerReadFailed")?;
    if !meta.is_file() || meta.len() > limit {
        return Err("installerSizeLimit".into());
    }
    fs::read(path).map_err(|_| "installerReadFailed".into())
}
pub fn reject_links(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|p| matches!(p, Component::ParentDir | Component::CurDir))
    {
        return Err("installerPathInvalid".into());
    }
    for part in path.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(part) {
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err("installerReparsePath".into());
                }
            }
            if meta.file_type().is_symlink() {
                return Err("installerLinkedPath".into());
            }
        }
    }
    Ok(())
}
pub fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    reject_links(path)?;
    let parent = path.parent().ok_or("installerPathInvalid")?;
    fs::create_dir_all(parent).map_err(|_| "installerWriteFailed")?;
    reject_links(parent)?;
    let temp = parent.join(format!("installer-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|_| "installerWriteFailed")?;
        file.write_all(bytes).map_err(|_| "installerWriteFailed")?;
        file.sync_all().map_err(|_| "installerWriteFailed")?;
        drop(file);
        #[cfg(windows)]
        {
            use windows::{
                Win32::Storage::FileSystem::{
                    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
                },
                core::PCWSTR,
            };
            let wide = |p: &Path| {
                p.as_os_str()
                    .to_string_lossy()
                    .encode_utf16()
                    .chain(Some(0))
                    .collect::<Vec<_>>()
            };
            unsafe {
                MoveFileExW(
                    PCWSTR(wide(&temp).as_ptr()),
                    PCWSTR(wide(path).as_ptr()),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
                .map_err(|_| "installerWriteFailed")?;
            }
        }
        #[cfg(not(windows))]
        {
            fs::rename(&temp, path).map_err(|_| "installerWriteFailed")?;
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
#[cfg(windows)]
pub fn write_receipt(path: &Path, bytes: &[u8]) -> Result<()> {
    reject_links(path)?;
    let parent = path.parent().ok_or("installerPathInvalid")?;
    fs::create_dir_all(parent).map_err(|e| format!("installerWriteFailed:parent.create:{e}"))?;
    sayweave_text_service::atomic_record::write(
        path,
        bytes,
        sayweave_text_service::atomic_record::budget(),
        |p| {
            reject_links(p).map_err(|e| {
                windows::core::Error::new(windows::Win32::Foundation::E_ACCESSDENIED, e)
            })?;
            for owned in [p.parent().unwrap(), p] {
                sayweave_text_service::atomic_record::verify_user_owner(owned)?;
            }
            Ok(())
        },
    )
    .map_err(|e| format!("installerWriteFailed:{e}"))
}
impl UserRecord {
    pub fn validate(&self) -> Result<()> {
        self.target.validate()?;
        if self.schema_version != 1
            || uuid::Uuid::parse_str(&self.transaction).is_err()
            || self.language == 0
            || self.files.len() > 5
            || self.keys.len() != 2
            || self.installer.pid == 0
            || !hex(&self.installer.image_hash, 64)
            || self.user_initialized && !self.user_initialization_started
        {
            return Err("installerRecordInvalid".into());
        }
        if self.files.iter().any(|f| {
            !matches!(
                f.backup.as_str(),
                "main" | "uninstall" | "helper" | "desktop" | "startMenu"
            ) || f.before_hash.as_ref().is_some_and(|v| !hex(v, 64))
                || f.installed_hash.as_ref().is_some_and(|v| !hex(v, 64))
        }) {
            return Err("installerRecordInvalid".into());
        }
        Ok(())
    }
}
/// A new process may resume an abandoned transaction, never a live installer.
pub fn recovery_owner(before: &Process, current: &Process, before_live: bool) -> Result<()> {
    if before.sid != current.sid {
        return Err("installerInitiator".into());
    }
    if before_live && before != current {
        return Err("installerBusy".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transaction_cannot_skip_user_or_component_verification() {
        assert!(Phase::Prepared.advance(Phase::Committed).is_err());
        assert!(Phase::ProgramInstalled.advance(Phase::Committed).is_err());
        assert_eq!(
            Phase::UserInitialized.advance(Phase::Committed).unwrap(),
            Phase::Committed
        );
        assert!(Phase::Committed.advance(Phase::RolledBack).is_err());
        assert_eq!(
            Phase::MachineCommitted.advance(Phase::Recovery).unwrap(),
            Phase::Recovery
        );
    }
    #[test]
    fn durable_backup_identity_and_bounded_reads_reject_corruption() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("receipt");
        write(&path, b"original").unwrap();
        assert_eq!(hash(&read(&path, 8).unwrap()), hash(b"original"));
        assert!(read(&path, 7).is_err());
        write(&path, b"replacement").unwrap();
        assert_eq!(read(&path, 32).unwrap(), b"replacement");
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[cfg(windows)]
    #[test]
    fn ordinary_owner_receipt_preserves_owner_and_confirms_replacement() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("receipt.json");
        write_receipt(&path, b"old").unwrap();
        write_receipt(&path, b"new").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"new");
        sayweave_text_service::atomic_record::verify_user_owner(&path).unwrap();
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[test]
    fn live_installer_identity_fences_receipt_recovery_including_pid_reuse() {
        let original = Process {
            pid: 100,
            session: 1,
            created: 10,
            sid: "S-1-5-21-1000".into(),
            image_hash: "a".repeat(64),
        };
        let mut second = original.clone();
        second.created = 20;
        assert_eq!(
            recovery_owner(&original, &second, true).unwrap_err(),
            "installerBusy"
        );
        assert!(recovery_owner(&original, &original, true).is_ok());
        assert!(recovery_owner(&original, &second, false).is_ok());
        second.sid = "S-1-5-21-2000".into();
        assert_eq!(
            recovery_owner(&original, &second, false).unwrap_err(),
            "installerInitiator"
        );
    }
}
