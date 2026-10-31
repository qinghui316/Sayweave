//! Fault-only receipt facts. Persist results, never replay native input.
use crate::contracts::dictionary::DataError;
use sayweave_text_service::lifecycle::{ReceiptKey, ReceiptOwner, ReceiptRecovery};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

const MAX_FILE: u64 = 1024 * 1024;
const MAX_TOTAL: u64 = 64 * 1024 * 1024;
#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Fact {
    schema: u32,
    owner: ReceiptOwner,
    receipt: ReceiptKey,
    frame: Vec<u8>,
    // Receiving an original result never confirms the original host's write.
    effect: String,
}
fn regular(path: &Path, directory: bool) -> Result<(), DataError> {
    let meta = fs::symlink_metadata(path).map_err(|_| DataError::new("unavailable"))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return Err(DataError::new("forbidden"));
        }
    }
    if meta.file_type().is_symlink()
        || meta.is_dir() != directory
        || (!directory && !meta.is_file())
    {
        return Err(DataError::new("forbidden"));
    }
    Ok(())
}
fn directory(root: &Path) -> Result<PathBuf, DataError> {
    regular(root, true)?;
    let path = root
        .canonicalize()
        .map_err(|_| DataError::new("unavailable"))?
        .join("native-receipt-recovery");
    fs::create_dir_all(&path).map_err(|_| DataError::new("unavailable"))?;
    regular(&path, true)?;
    Ok(path)
}
pub(super) fn preserve(
    root: &Path,
    offer: &ReceiptRecovery,
    frame: &[u8],
) -> Result<(), DataError> {
    preserve_with_budget(root, offer, frame, MAX_TOTAL)
}
fn preserve_with_budget(
    root: &Path,
    offer: &ReceiptRecovery,
    frame: &[u8],
    budget: u64,
) -> Result<(), DataError> {
    if !offer.verifies(frame)
        || offer.owner.process == 0
        || offer.owner.created == 0
        || offer.owner.capability_sha256.len() != 64
        || !offer
            .owner
            .capability_sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit())
    {
        return Err(DataError::new("invalid"));
    }
    let fact = Fact {
        schema: 1,
        owner: offer.owner.clone(),
        receipt: offer.receipt.clone(),
        frame: frame.to_vec(),
        effect: "unconfirmed".into(),
    };
    let bytes = serde_json::to_vec(&fact).map_err(|_| DataError::new("invalid"))?;
    if bytes.len() as u64 > MAX_FILE {
        return Err(DataError::new("invalid"));
    }
    let dir = directory(root)?;
    let name = offer
        .receipt
        .sha256
        .as_ref()
        .ok_or_else(|| DataError::new("invalid"))?;
    let path = dir.join(format!("{name}.json"));
    if path.exists() {
        regular(&path, false)?;
        if fs::metadata(&path)
            .map_err(|_| DataError::new("unavailable"))?
            .len()
            > MAX_FILE
        {
            return Err(DataError::new("invalid"));
        }
        return if fs::read(&path).map_err(|_| DataError::new("unavailable"))? == bytes {
            Ok(())
        } else {
            Err(DataError::new("conflict"))
        };
    }
    // Remove only our own unpublished partial before counting the budget.
    let temporary = dir.join(format!("{name}.pending"));
    if temporary.exists() {
        regular(&temporary, false)?;
        fs::remove_file(&temporary).map_err(|_| DataError::new("unavailable"))?;
    }
    let mut total = 0u64;
    for entry in fs::read_dir(&dir).map_err(|_| DataError::new("unavailable"))? {
        let entry = entry.map_err(|_| DataError::new("unavailable"))?;
        regular(&entry.path(), false)?;
        total = total.saturating_add(
            entry
                .metadata()
                .map_err(|_| DataError::new("unavailable"))?
                .len(),
        );
    }
    if total.saturating_add(bytes.len() as u64) > budget {
        return Err(DataError::new("storageFull"));
    }
    // The DataWorker lease/journal lock serializes this namespace. Only the
    // unpublished temporary file may be replaced after an interrupted write.
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        publish(&temporary, &path)?;
        Ok::<_, std::io::Error>(())
    })();
    if result.is_err() && temporary.exists() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|_| DataError::new("unavailable"))?;
    // An ACK can follow only the complete durable bytes, not a queued write.
    if fs::read(&path).map_err(|_| DataError::new("unavailable"))? != bytes {
        return Err(DataError::new("conflict"));
    }
    Ok(())
}

fn publish(temporary: &Path, path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
        let from: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // No REPLACE_EXISTING: a published original fact is immutable.
        unsafe {
            MoveFileExW(
                windows::core::PCWSTR(from.as_ptr()),
                windows::core::PCWSTR(to.as_ptr()),
                MOVEFILE_WRITE_THROUGH,
            )
            .map_err(std::io::Error::other)
        }
    }
    #[cfg(not(windows))]
    {
        fs::rename(temporary, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, ReceiptRecovery, Vec<u8>) {
        let root = std::env::temp_dir().join(format!("sayweave-receipt-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let frame = serde_json::to_vec(&serde_json::json!({"engine":{"proxy_instance":uuid::Uuid::new_v4().to_string(),"engine_generation":2},"task":uuid::Uuid::new_v4().to_string(),"turn":1,"operation":3,"revision":1,"mutation":"changed","text":"原结果"})).unwrap();
        let offer = ReceiptRecovery {
            control_protocol: 1,
            task: uuid::Uuid::new_v4().to_string(),
            turn: 1,
            operation: 1,
            code: "receiptRecovery".into(),
            receipt: ReceiptKey::from_frame(&frame).unwrap(),
            owner: ReceiptOwner {
                process: 42,
                session: 1,
                created: 9,
                capability_sha256: "a".repeat(64),
            },
        };
        (root, offer, frame)
    }
    #[test]
    fn exact_fact_is_durable_idempotent_and_keeps_original_unknown_effect() {
        let (root, mut offer, frame) = fixture();
        preserve(&root, &offer, &frame).unwrap();
        offer.task = uuid::Uuid::new_v4().to_string(); // A different receiver does not rewrite original ownership.
        preserve_with_budget(&root, &offer, &frame, 0).unwrap();
        let path = directory(&root)
            .unwrap()
            .join(format!("{}.json", offer.receipt.sha256.as_ref().unwrap()));
        let fact: Fact = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(fact.owner, offer.owner);
        assert_eq!(fact.frame, frame);
        assert_eq!(fact.effect, "unconfirmed");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn changed_bytes_owner_or_existing_corruption_cannot_be_acknowledged() {
        let (root, mut offer, mut frame) = fixture();
        preserve(&root, &offer, &frame).unwrap();
        offer.owner.created += 1;
        assert_eq!(
            preserve(&root, &offer, &frame).unwrap_err().code,
            "conflict"
        );
        frame.push(b' ');
        assert_eq!(preserve(&root, &offer, &frame).unwrap_err().code, "invalid");
        let path = directory(&root)
            .unwrap()
            .join(format!("{}.json", offer.receipt.sha256.as_ref().unwrap()));
        fs::write(path, b"partial").unwrap();
        frame.pop();
        assert_eq!(
            preserve(&root, &offer, &frame).unwrap_err().code,
            "conflict"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn full_store_retains_proxy_result_and_an_unpublished_partial_can_retry() {
        let (root, offer, frame) = fixture();
        assert_eq!(
            preserve_with_budget(&root, &offer, &frame, 0)
                .unwrap_err()
                .code,
            "storageFull"
        );
        let dir = directory(&root).unwrap();
        let pending = dir.join(format!(
            "{}.pending",
            offer.receipt.sha256.as_ref().unwrap()
        ));
        fs::write(&pending, b"interrupted").unwrap();
        let size = serde_json::to_vec(&Fact {
            schema: 1,
            owner: offer.owner.clone(),
            receipt: offer.receipt.clone(),
            frame: frame.clone(),
            effect: "unconfirmed".into(),
        })
        .unwrap()
        .len() as u64;
        preserve_with_budget(&root, &offer, &frame, size).unwrap();
        assert!(!pending.exists());
        fs::remove_dir_all(root).unwrap();
    }
}
