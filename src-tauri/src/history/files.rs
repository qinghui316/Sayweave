use super::types::Error;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
#[derive(Clone)]
pub struct AudioFiles {
    root: PathBuf,
}
impl AudioFiles {
    pub fn new(profile: &Path) -> Result<Self, Error> {
        let root = profile.join("audio");
        fs::create_dir_all(&root).map_err(|_| Error::new("saveFailed"))?;
        let meta = fs::symlink_metadata(&root).map_err(|_| Error::new("fileUnavailable"))?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if meta.file_attributes() & 0x400 != 0 {
                return Err(Error::new("forbidden"));
            }
        }
        if meta.file_type().is_symlink() {
            return Err(Error::new("forbidden"));
        }
        Ok(Self {
            root: root
                .canonicalize()
                .map_err(|_| Error::new("fileUnavailable"))?,
        })
    }
    pub fn path(&self, key: &str, part: bool) -> Result<PathBuf, Error> {
        let root_meta =
            fs::symlink_metadata(&self.root).map_err(|_| Error::new("fileUnavailable"))?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if root_meta.file_attributes() & 0x400 != 0 {
                return Err(Error::new("forbidden"));
            }
        }
        if root_meta.file_type().is_symlink() {
            return Err(Error::new("forbidden"));
        }
        if uuid::Uuid::parse_str(key).is_err() {
            return Err(Error::new("forbidden"));
        }
        let path = self
            .root
            .join(format!("{key}.{}", if part { "part" } else { "wav" }));
        if let Ok(meta) = fs::symlink_metadata(&path) {
            // On Windows junctions and other reparse points must not be followed.
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if meta.file_attributes() & 0x400 != 0 {
                    return Err(Error::new("forbidden"));
                }
            }
            if meta.file_type().is_symlink() || !meta.is_file() {
                return Err(Error::new("forbidden"));
            }
        }
        Ok(path)
    }
    pub fn writer(&self, key: &str) -> Result<hound::WavWriter<std::io::BufWriter<File>>, Error> {
        let f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.path(key, true)?)
            .map_err(|_| Error::new("saveFailed"))?;
        hound::WavWriter::new(
            std::io::BufWriter::new(f),
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .map_err(|_| Error::new("saveFailed"))
    }
    pub fn complete(&self, key: &str) -> Result<u64, Error> {
        let part = self.path(key, true)?;
        validate(&part)?;
        OpenOptions::new()
            .read(true)
            .write(true)
            .open(&part)
            .and_then(|f| f.sync_all())
            .map_err(|_| Error::new("saveFailed"))?;
        fs::rename(part, self.path(key, false)?).map_err(|_| Error::new("saveFailed"))?;
        validate(&self.path(key, false)?)
    }
    pub fn recover(&self, key: &str) -> Result<u64, Error> {
        let ready = self.path(key, false)?;
        if ready.exists() {
            return validate(&ready);
        }
        let part = self.path(key, true)?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&part)
            .map_err(|_| Error::new("fileUnavailable"))?;
        let length = file
            .metadata()
            .map_err(|_| Error::new("fileUnavailable"))?
            .len();
        let mut header = [0u8; 44];
        file.read_exact(&mut header)
            .map_err(|_| Error::new("fileUnavailable"))?;
        if &header[0..4] != b"RIFF"
            || &header[8..12] != b"WAVE"
            || &header[12..16] != b"fmt "
            || &header[36..40] != b"data"
            || !(46..=20 * 1024 * 1024).contains(&length)
        {
            return Err(Error::new("fileUnavailable"));
        }
        let samples = (length - 44) / 2 * 2;
        file.set_len(samples + 44)
            .map_err(|_| Error::new("saveFailed"))?;
        file.seek(SeekFrom::Start(4))
            .and_then(|_| file.write_all(&((samples + 36) as u32).to_le_bytes()))
            .map_err(|_| Error::new("saveFailed"))?;
        file.seek(SeekFrom::Start(40))
            .and_then(|_| file.write_all(&(samples as u32).to_le_bytes()))
            .map_err(|_| Error::new("saveFailed"))?;
        file.sync_all().map_err(|_| Error::new("saveFailed"))?;
        drop(file);
        self.complete(key)
    }
    pub fn remove(&self, key: &str) -> Result<(), Error> {
        for part in [true, false] {
            let path = self.path(key, part)?;
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error::new("deleteFailed")),
            }
        }
        Ok(())
    }
    pub fn read(&self, key: &str) -> Result<Vec<i16>, Error> {
        let path = self.path(key, false)?;
        validate(&path)?;
        hound::WavReader::open(path)
            .map_err(|_| Error::new("fileUnavailable"))?
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Error::new("fileUnavailable"))
    }
}
pub fn validate(path: &Path) -> Result<u64, Error> {
    let mut r = hound::WavReader::open(path).map_err(|_| Error::new("fileUnavailable"))?;
    let s = r.spec();
    if s.channels != 1
        || s.sample_rate != 16000
        || s.bits_per_sample != 16
        || s.sample_format != hound::SampleFormat::Int
        || r.len() == 0
        || r.len() > 16000 * 600
    {
        return Err(Error::new("fileUnavailable"));
    }
    let samples = r
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| Error::new("fileUnavailable"))?;
    Ok(samples.len() as u64 * 2)
}
