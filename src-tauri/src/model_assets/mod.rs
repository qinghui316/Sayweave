//! Model bytes and read leases; no recording, inference or credential ownership.
pub mod catalog;
mod service;
#[cfg(test)]
#[path = "tests.rs"]
mod service_tests;
pub use service::Assets;
pub use service::Status as AssetStatus;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const MODEL_ID: &str = "nemotron-3.5-asr-streaming-0.6b-q4";
pub const MODEL_FILE: &str = "nemotron-3.5-asr-streaming-0.6b-Q4_K_M.gguf";
pub const MODEL_BYTES: u64 = 495_831_520;
pub const MODEL_SHA256: &str = "41c99fa5fb6f3d35f68e79adc3e755eca2232a8d921178bd647b71194792b8fd";

pub struct ModelLease {
    pub path: PathBuf,
    pub sha256: String,
    // The handle stays live until the last worker/task reference has settled.
    _file: File,
}
impl ModelLease {
    pub fn acquire(
        path: &Path,
        expected_bytes: u64,
        sha256: &str,
    ) -> Result<Arc<Self>, &'static str> {
        validate_path(path)?;
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Other readers (the worker) are allowed; writers/deletion are not.
            options.share_mode(1);
        }
        let mut file = options.open(path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                "localModelMissing"
            } else {
                "localModelUnreadable"
            }
        })?;
        if file.metadata().map_err(|_| "localModelUnreadable")?.len() != expected_bytes {
            return Err("localModelCorrupt");
        }
        let mut digest = Sha256::new();
        let mut buffer = [0; 64 * 1024];
        loop {
            let n = file.read(&mut buffer).map_err(|_| "localModelUnreadable")?;
            if n == 0 {
                break;
            }
            digest.update(&buffer[..n]);
        }
        if format!("{:x}", digest.finalize()) != sha256 {
            return Err("localModelCorrupt");
        }
        let path = std::fs::canonicalize(path).map_err(|_| "localModelMissing")?;
        Ok(Arc::new(Self {
            path,
            sha256: sha256.into(),
            _file: file,
        }))
    }
}

// Model files belong to our asset directory. Never follow a redirected owned
// file or directory during verification, download, rename or removal.
pub(super) fn validate_path(path: &Path) -> Result<(), &'static str> {
    if !path.is_absolute()
        || path.components().any(|p| {
            matches!(
                p,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err("localFileAccess");
    }
    for part in path.ancestors() {
        match std::fs::symlink_metadata(part) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err("localFileAccess");
                }
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    if meta.file_attributes() & 0x400 != 0 {
                        return Err("localFileAccess");
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("localFileAccess"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lease_verifies_actual_bytes_and_retains_the_file() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("fixture.gguf");
        std::fs::write(&file, b"fixture").unwrap();
        let hash = format!("{:x}", Sha256::digest(b"fixture"));
        let lease = ModelLease::acquire(&file, 7, &hash).unwrap();
        assert_eq!(lease.sha256, hash);
        assert!(ModelLease::acquire(&file, 8, &hash).is_err());
        assert!(ModelLease::acquire(&file, 7, MODEL_SHA256).is_err());
        #[cfg(windows)]
        assert!(std::fs::remove_file(&file).is_err());
        drop(lease);
        std::fs::remove_file(file).unwrap();
    }
}
