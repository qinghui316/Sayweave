//! Installed compute payload identity; no inference, worker or model-file ownership.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::Read,
    path::Path,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeFile {
    pub name: String,
    pub bytes: u64,
    pub sha256: String,
}
impl RuntimeFile {
    pub fn valid(&self) -> bool {
        !self.name.is_empty()
            && self.name.len() <= 80
            && self
                .name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-_.".contains(&b))
            && sayweave_local_asr_protocol::runtime_file_names().contains(&self.name)
            && self.bytes > 0
            && self.bytes <= 128 * 1024 * 1024
            && self.sha256.len() == 64
            && self.sha256.bytes().all(|b| b.is_ascii_hexdigit())
    }
}
pub fn embedded() -> Result<Vec<RuntimeFile>, &'static str> {
    #[derive(Deserialize)]
    struct Identity {
        bytes: u64,
        sha256: String,
    }
    #[derive(Deserialize)]
    struct Manifest {
        version: u32,
        engine: String,
        files: BTreeMap<String, Identity>,
    }
    let value: Manifest =
        serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/local-asr.json")))
            .map_err(|_| "localRuntimeMismatch")?;
    let files = value
        .files
        .into_iter()
        .map(|(name, v)| RuntimeFile {
            name,
            bytes: v.bytes,
            sha256: v.sha256,
        })
        .collect::<Vec<_>>();
    if value.version != 1
        || value.engine != "0.1.3"
        || files.len() > 24
        || (!files.is_empty()
            && files.len() != sayweave_local_asr_protocol::runtime_file_names().len())
        || files.iter().any(|f| !f.valid())
    {
        return Err("localRuntimeMismatch");
    }
    Ok(files)
}
pub fn hold(program: &Path, inventory: &[RuntimeFile]) -> Result<Vec<File>, &'static str> {
    if !program.is_absolute()
        || program.file_name() != Some(std::ffi::OsStr::new("sayweave-local-asr-worker.exe"))
        || inventory.is_empty()
        || inventory.len() > 24
        || inventory.iter().any(|f| !f.valid())
        || inventory
            .iter()
            .filter(|f| f.name == "sayweave-local-asr-worker.exe")
            .count()
            != 1
    {
        return Err("localRuntimeMissing");
    }
    let directory = program.parent().ok_or("localRuntimeMissing")?;
    for ancestor in directory.ancestors() {
        let metadata = std::fs::symlink_metadata(ancestor).map_err(|_| "localRuntimeMissing")?;
        if metadata.file_type().is_symlink() {
            return Err("localRuntimeMismatch");
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err("localRuntimeMismatch");
            }
        }
    }
    let mut files = Vec::new();
    for entry in inventory {
        let path = directory.join(&entry.name);
        if std::fs::symlink_metadata(&path)
            .map_err(|_| "localRuntimeMissing")?
            .file_type()
            .is_symlink()
        {
            return Err("localRuntimeMismatch");
        }
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(1);
        }
        let mut file = options.open(&path).map_err(|_| "localRuntimeMissing")?;
        if file.metadata().map_err(|_| "localRuntimeMissing")?.len() != entry.bytes {
            return Err("localRuntimeMismatch");
        }
        let mut hash = Sha256::new();
        let mut bytes = [0; 65536];
        loop {
            let n = file.read(&mut bytes).map_err(|_| "localRuntimeMissing")?;
            if n == 0 {
                break;
            }
            hash.update(&bytes[..n]);
        }
        if format!("{:x}", hash.finalize()) != entry.sha256 {
            return Err("localRuntimeMismatch");
        }
        files.push(file);
    }
    for entry in std::fs::read_dir(directory).map_err(|_| "localRuntimeMissing")? {
        let name = entry
            .map_err(|_| "localRuntimeMissing")?
            .file_name()
            .to_string_lossy()
            .into_owned();
        if (name.to_ascii_lowercase().ends_with(".dll")
            || name.to_ascii_lowercase().ends_with(".exe"))
            && !inventory.iter().any(|f| f.name == name)
        {
            return Err("localRuntimeMismatch");
        }
    }
    Ok(files)
}
