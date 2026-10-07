pub mod files;
pub mod playback;
#[cfg(test)]
mod tests;
pub mod types;
use crate::data::{DataWorker, Operation, Output};
use std::{
    collections::HashSet,
    path::Path,
    sync::{Arc, Mutex},
};
use types::*;
#[derive(Clone)]
pub struct HistoryService {
    pub data: DataWorker,
    pub files: Result<files::AudioFiles, Error>,
    active: Arc<Mutex<HashSet<String>>>,
    records: Arc<Mutex<HashSet<String>>>,
}
pub struct AudioLease {
    pub audio: Audio,
    _reservation: AudioReservation,
}
pub(crate) struct RecordLease {
    id: String,
    records: Arc<Mutex<HashSet<String>>>,
}
impl Drop for RecordLease {
    fn drop(&mut self) {
        if let Ok(mut records) = self.records.lock() {
            records.remove(&self.id);
        }
    }
}
struct AudioReservation {
    id: String,
    active: Arc<Mutex<HashSet<String>>>,
}
impl Drop for AudioReservation {
    fn drop(&mut self) {
        if let Ok(mut active) = self.active.lock() {
            active.remove(&self.id);
        }
    }
}
impl HistoryService {
    pub(crate) fn hold_record(&self, id: &str) -> Result<RecordLease, Error> {
        let mut records = self.records.lock().map_err(|_| Error::new("unavailable"))?;
        if !records.insert(id.into()) {
            return Err(Error::new("busy"));
        }
        Ok(RecordLease {
            id: id.into(),
            records: self.records.clone(),
        })
    }
    pub fn new(data: DataWorker, root: &Path) -> Self {
        Self {
            data,
            files: files::AudioFiles::new(root),
            active: Arc::new(Mutex::new(HashSet::new())),
            records: Arc::new(Mutex::new(HashSet::new())),
        }
    }
    pub async fn db(&self, op: DbOperation) -> Result<DbOutput, Error> {
        match self.data.request_admitted(Operation::History(op)).await? {
            Output::History(v) => Ok(v),
            _ => Err(Error::new("unavailable")),
        }
    }
    pub async fn preferences(&self) -> Result<Preferences, Error> {
        match self.db(DbOperation::Preferences).await? {
            DbOutput::Preferences(v) => Ok(v),
            _ => Err(Error::new("unavailable")),
        }
    }
    pub async fn detail(&self, id: &str) -> Result<Record, Error> {
        match self.db(DbOperation::Detail(id.into())).await? {
            DbOutput::Record(v) => Ok(v),
            _ => Err(Error::new("unavailable")),
        }
    }
    pub async fn asset(&self, id: &str) -> Result<Audio, Error> {
        match self.db(DbOperation::Audio(id.into())).await? {
            DbOutput::Audio(v) => Ok(v),
            _ => Err(Error::new("unavailable")),
        }
    }
    pub async fn lease(&self, id: &str) -> Result<AudioLease, Error> {
        let reservation = self.reserve(id)?;
        match self.asset(id).await {
            Ok(audio) if audio.status == "ready" => Ok(AudioLease {
                audio,
                _reservation: reservation,
            }),
            result => Err(result.err().unwrap_or(Error::new("fileUnavailable"))),
        }
    }
    fn reserve(&self, id: &str) -> Result<AudioReservation, Error> {
        let mut active = self.active.lock().map_err(|_| Error::new("unavailable"))?;
        if !active.insert(id.into()) {
            return Err(Error::new("busy"));
        }
        Ok(AudioReservation {
            id: id.into(),
            active: self.active.clone(),
        })
    }
    pub async fn remove(&self, id: &str, revision: i64) -> Result<(), Error> {
        let _record = self.hold_record(id)?;
        let record = self.detail(id).await?;
        let audio = record.audio;
        let _lease = if let Some(a) = &audio {
            Some(self.reserve(&a.id)?)
        } else {
            None
        };
        self.db(DbOperation::MarkDelete {
            id: id.into(),
            revision,
        })
        .await?;
        if let Some(a) = audio {
            let files = self.files.clone()?;
            tokio::task::spawn_blocking(move || files.remove(&a.file_key))
                .await
                .map_err(|_| Error::new("deleteFailed"))??;
        }
        self.db(DbOperation::DeleteRecord(id.into())).await?;
        Ok(())
    }
    pub async fn recover(&self) -> Result<(), Error> {
        self.clean_discarded_audio().await?;
        self.data
            .request_admitted(Operation::Processing(
                crate::contracts::processing::DbOperation::Recover,
            ))
            .await?;
        self.db(DbOperation::RecoverDelivery).await?;
        let assets = match self.db(DbOperation::Pending).await? {
            DbOutput::Assets(v) => v,
            _ => return Err(Error::new("unavailable")),
        };
        for a in assets {
            let files = self.files.clone()?;
            let key = a.file_key.clone();
            if a.status == "prepared" {
                let result = tokio::task::spawn_blocking(move || files.recover(&key))
                    .await
                    .map_err(|_| Error::new("fileUnavailable"))?;
                let bytes = result.as_ref().copied().unwrap_or(0);
                self.db(DbOperation::AudioState {
                    id: a.id,
                    status: if result.is_ok() { "ready" } else { "corrupt" }.into(),
                    bytes,
                })
                .await?;
            } else if tokio::task::spawn_blocking(move || files.remove(&key))
                .await
                .map_err(|_| Error::new("deleteFailed"))?
                .is_ok()
            {
                if a.status == "expire_pending" {
                    self.db(DbOperation::AudioState {
                        id: a.id,
                        status: "expired".into(),
                        bytes: 0,
                    })
                    .await?;
                } else {
                    self.db(DbOperation::DeleteRecord(a.record_id)).await?;
                }
            }
        }
        // Also finish tombstones for text-only records.
        self.db(DbOperation::Interrupted).await?;
        self.cleanup().await
    }
    pub(crate) async fn discard_audio(&self, record: &str) -> Result<(), Error> {
        self.db(DbOperation::DiscardAudio(record.into())).await?;
        self.clean_discarded_audio().await
    }
    async fn clean_discarded_audio(&self) -> Result<(), Error> {
        let DbOutput::Assets(assets) = self.db(DbOperation::DiscardedAudio).await? else {
            return Err(Error::new("unavailable"));
        };
        for asset in assets {
            let _lease = self.reserve(&asset.id)?;
            let files = self.files.clone()?;
            let key = asset.file_key;
            tokio::task::spawn_blocking(move || files.remove(&key))
                .await
                .map_err(|_| Error::new("deleteFailed"))??;
            self.db(DbOperation::DeleteDiscardedAudio(asset.id)).await?;
        }
        Ok(())
    }
    pub async fn cleanup(&self) -> Result<(), Error> {
        let assets = match self
            .db(DbOperation::Expired(crate::data::repository::now()))
            .await?
        {
            DbOutput::Assets(v) => v,
            _ => return Err(Error::new("unavailable")),
        };
        for a in assets {
            let _lease = match self.reserve(&a.id) {
                Ok(held) => held,
                Err(e) if e.code == "busy" => continue,
                Err(e) => return Err(e),
            };
            self.db(DbOperation::AudioState {
                id: a.id.clone(),
                status: "expire_pending".into(),
                bytes: a.bytes,
            })
            .await?;
            let files = self.files.clone()?;
            let key = a.file_key.clone();
            if tokio::task::spawn_blocking(move || files.remove(&key))
                .await
                .map_err(|_| Error::new("deleteFailed"))?
                .is_ok()
            {
                self.db(DbOperation::AudioState {
                    id: a.id,
                    status: "expired".into(),
                    bytes: 0,
                })
                .await?;
            }
        }
        Ok(())
    }
}

impl HistoryService {
    pub async fn validate_asset(&self, audio: &Audio) -> Result<std::path::PathBuf, Error> {
        let files = self.files.clone()?;
        let key = audio.file_key.clone();
        let result = tokio::task::spawn_blocking(move || {
            let path = files.path(&key, false).map_err(|e| (e, "corrupt"))?;
            let status = if path.exists() { "corrupt" } else { "missing" };
            match files::validate(&path) {
                Ok(_) => Ok(path),
                Err(e) => Err((e, status)),
            }
        })
        .await
        .map_err(|_| Error::new("fileUnavailable"))?;
        match result {
            Ok(path) => Ok(path),
            Err((e, status)) => {
                let _ = self
                    .db(DbOperation::AudioState {
                        id: audio.id.clone(),
                        status: status.into(),
                        bytes: audio.bytes,
                    })
                    .await;
                Err(e)
            }
        }
    }
}
