//! Auxiliary cancellation journal. Only retained recognition data is persisted;
//! replay changes storage, never an external input target.
use super::{Operation, repository::Repository};
use crate::contracts::{dictionary::DataError, history, processing};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};
const LIMIT: usize = 16 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Entry {
    Processing {
        id: String,
        text: String,
        raw: String,
        units: Vec<processing::UnitRecord>,
    },
    History {
        attempt_id: String,
        text: String,
    },
}
impl Entry {
    fn id(&self) -> &str {
        match self {
            Self::Processing { id, .. } => id,
            Self::History { attempt_id, .. } => attempt_id,
        }
    }
    fn capture(operation: &Operation) -> Option<Self> {
        match operation {
            Operation::Processing(processing::DbOperation::Cancel {
                id,
                text,
                raw,
                units,
            }) => Some(Self::Processing {
                id: id.clone(),
                text: text.clone(),
                raw: raw.clone(),
                units: units.clone(),
            }),
            Operation::History(history::DbOperation::PruneCancelled { attempt_id, text }) => {
                Some(Self::History {
                    attempt_id: attempt_id.clone(),
                    text: text.clone(),
                })
            }
            _ => None,
        }
    }
    fn apply(self, repository: &mut Repository) -> Result<(), DataError> {
        match self {
            Self::Processing {
                id,
                text,
                raw,
                units,
            } => {
                let settled = repository.connection.query_row("SELECT p.state='cancelled' AND p.text=?2 AND a.text=?3 FROM processing_runs p JOIN speech_attempts a ON a.id=p.attempt_id WHERE p.id=?1",rusqlite::params![id,text,raw],|r|r.get::<_,bool>(0)).optional().map_err(super::repository::db_error)?.unwrap_or(false);
                if !settled {
                    repository.processing_operation(processing::DbOperation::Cancel {
                        id,
                        text,
                        raw,
                        units,
                    })?;
                }
            }
            Self::History { attempt_id, text } => {
                repository
                    .history_operation(history::DbOperation::PruneCancelled { attempt_id, text })?;
            }
        }
        Ok(())
    }
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
        || (directory && !meta.is_dir())
        || (!directory && !meta.is_file())
    {
        return Err(DataError::new("forbidden"));
    }
    Ok(())
}
fn directory(root: &Path) -> Result<PathBuf, DataError> {
    regular(root, true)?;
    let root = root
        .canonicalize()
        .map_err(|_| DataError::new("unavailable"))?;
    let path = root.join("cancel-recovery");
    fs::create_dir_all(&path).map_err(|_| DataError::new("unavailable"))?;
    regular(&path, true)?;
    Ok(path)
}
/// Must complete before admitting the cancellation to the bounded worker queue.
pub(super) fn stage(root: &Path, operation: &Operation) -> Result<bool, DataError> {
    let Some(entry) = Entry::capture(operation) else {
        return Ok(false);
    };
    if uuid::Uuid::parse_str(entry.id()).is_err() {
        return Err(DataError::new("invalid"));
    }
    let bytes = serde_json::to_vec(&entry).map_err(|_| DataError::new("invalid"))?;
    if bytes.len() > LIMIT {
        return Err(DataError::new("invalid"));
    }
    let path = directory(root)?.join(format!("{}.json", entry.id()));
    if path.exists() {
        regular(&path, false)?;
        if fs::metadata(&path)
            .map_err(|_| DataError::new("unavailable"))?
            .len()
            > LIMIT as u64
        {
            return Err(DataError::new("unavailable"));
        }
        if fs::read(&path).map_err(|_| DataError::new("unavailable"))? == bytes {
            return Ok(true);
        }
        return Err(DataError::new("conflict"));
    }
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| DataError::new("unavailable"))?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| DataError::new("unavailable"))?;
    Ok(true)
}
/// Read paths are blocked until every registered intent is settled. A malformed
/// or partial intent is an explicit recovery error, never permission to expose
/// pre-cancellation rows or to guess which data should be deleted.
pub(super) fn replay(repository: &mut Repository, root: &Path) -> Result<(), DataError> {
    let path = directory(root)?;
    let mut paths = fs::read_dir(path)
        .map_err(|_| DataError::new("unavailable"))?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| DataError::new("unavailable"))?;
    paths.sort();
    let mut entries = Vec::new();
    for path in paths {
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        regular(&path, false)?;
        if fs::metadata(&path)
            .map_err(|_| DataError::new("unavailable"))?
            .len()
            > LIMIT as u64
        {
            return Err(DataError::new("unavailable"));
        }
        let entry: Entry =
            serde_json::from_slice(&fs::read(&path).map_err(|_| DataError::new("unavailable"))?)
                .map_err(|_| DataError::new("unavailable"))?;
        if uuid::Uuid::parse_str(entry.id()).is_err()
            || path.file_stem().and_then(|s| s.to_str()) != Some(entry.id())
        {
            return Err(DataError::new("forbidden"));
        }
        entries.push((path, entry));
    }
    entries.sort_by_key(|(_, entry)| match entry {
        Entry::Processing { .. } => 0,
        Entry::History { .. } => 1,
    });
    for (path, entry) in entries {
        entry.apply(repository)?;
        fs::remove_file(path).map_err(|_| DataError::new("unavailable"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contracts::history::{Attempt, Prepare};
    use crate::data::repository::now;
    fn prepared(repo: &mut Repository) -> (String, String) {
        let id = uuid::Uuid::new_v4().to_string();
        let attempt = uuid::Uuid::new_v4().to_string();
        repo.history_operation(history::DbOperation::Prepare(Prepare {
            record_id: id.clone(),
            audio: None,
            attempt: Attempt {
                id: attempt.clone(),
                record_id: id.clone(),
                kind: "live".into(),
                config_id: "fixture".into(),
                config_name: "fixture".into(),
                config_revision: 1,
                model_id: "fixture".into(),
                service_host: "example.invalid".into(),
                created_at: now(),
                ended_at: None,
                status: "preparing".into(),
                text: String::new(),
                error_code: None,
                elapsed_ms: 0,
            },
        }))
        .unwrap();
        repo.history_operation(history::DbOperation::Finish {
            attempt_id: attempt.clone(),
            status: "cancelled".into(),
            text: "前段。未提交尾部。".into(),
            error: Some("cancelled".into()),
            duration_ms: 0,
            elapsed_ms: 1,
        })
        .unwrap();
        (id, attempt)
    }
    #[test]
    fn raw_cancel_survives_sqlite_failure_and_replays_before_reopen_reads() {
        let root = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(root.path()).unwrap();
        let (record, attempt) = prepared(&mut repo);
        let intent = Operation::History(history::DbOperation::PruneCancelled {
            attempt_id: attempt.clone(),
            text: "前段。".into(),
        });
        stage(root.path(), &intent).unwrap();
        repo.connection
            .pragma_update(None, "query_only", true)
            .unwrap();
        assert!(replay(&mut repo, root.path()).is_err());
        assert!(
            directory(root.path())
                .unwrap()
                .join(format!("{attempt}.json"))
                .exists()
        );
        drop(repo);
        let mut reopened = Repository::open(root.path()).unwrap();
        let history::DbOutput::Record(row) = reopened
            .history_operation(history::DbOperation::Detail(record))
            .unwrap()
        else {
            panic!("record expected")
        };
        assert_eq!(row.attempts[0].text, "前段。");
        assert!(
            !directory(root.path())
                .unwrap()
                .join(format!("{attempt}.json"))
                .exists()
        );
    }
    #[test]
    fn incomplete_journal_blocks_reopen_instead_of_exposing_old_cancelled_text() {
        let root = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(root.path()).unwrap();
        let (_, attempt) = prepared(&mut repo);
        drop(repo);
        fs::write(
            directory(root.path())
                .unwrap()
                .join(format!("{attempt}.json")),
            b"{\"History\":",
        )
        .unwrap();
        assert!(Repository::open(root.path()).is_err());
    }
    #[test]
    fn precise_cancel_receipt_wins_over_broader_raw_fallback_on_restart() {
        let root = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(root.path()).unwrap();
        let (record, attempt) = prepared(&mut repo);
        let run = processing::RunMetadata {
            id: uuid::Uuid::new_v4().to_string(),
            attempt_id: attempt.clone(),
            task_id: uuid::Uuid::new_v4().to_string(),
            config_id: uuid::Uuid::new_v4().to_string(),
            config_name: "fixture".into(),
            model_id: "fixture".into(),
            config_revision: 1,
            prompt_version: "faithful-v1".into(),
            settings_revision: 1,
            dictionary_version: "snapshot".into(),
        };
        repo.processing_operation(processing::DbOperation::Prepare(run.clone()))
            .unwrap();
        let unit = processing::UnitRecord {
            id: uuid::Uuid::new_v4().to_string(),
            run_id: run.id.clone(),
            turn: 1,
            source_start: 0,
            source_end: 3,
            source_refs: "[]".into(),
            source_text: "前段。".into(),
            candidate_text: Some("前段。".into()),
            generation_state: "success".into(),
            application_state: "uncertain".into(),
            source_committed_end: None,
            error_code: Some("compositionUncertain".into()),
            prompt_tokens: None,
            completion_tokens: None,
            elapsed_ms: 1,
        };
        repo.processing_operation(processing::DbOperation::SaveUnit(unit.clone()))
            .unwrap();
        // Intents coexist after a failed queue/write; creation order deliberately
        // starts with the broader fallback. Restart still prefers exact receipts.
        stage(
            root.path(),
            &Operation::History(history::DbOperation::PruneCancelled {
                attempt_id: attempt,
                text: "前段。未提交尾部。".into(),
            }),
        )
        .unwrap();
        stage(
            root.path(),
            &Operation::Processing(processing::DbOperation::Cancel {
                id: run.id,
                text: String::new(),
                raw: String::new(),
                units: vec![unit],
            }),
        )
        .unwrap();
        drop(repo);
        let mut reopened = Repository::open(root.path()).unwrap();
        let history::DbOutput::Record(row) = reopened
            .history_operation(history::DbOperation::Detail(record))
            .unwrap()
        else {
            panic!("record expected")
        };
        assert_eq!(row.attempts[0].text, "");
        assert_eq!(row.processing[0].units.len(), 1);
        assert_eq!(row.processing[0].units[0].source_text, "前段。");
        assert_eq!(row.processing[0].units[0].application_state, "uncertain");
        assert!(!format!("{:?}", row).contains("未提交尾部"));
    }
    #[tokio::test]
    async fn full_queue_keeps_cancel_intent_and_next_reader_settles_it() {
        use std::sync::{Arc, Mutex, atomic::AtomicBool};
        use tokio::sync::{mpsc, oneshot};
        let root = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(root.path()).unwrap();
        let (record, attempt) = prepared(&mut repo);
        drop(repo);
        let worker = super::super::DataWorker::start(root.path().to_path_buf()).unwrap();
        while worker.status().unwrap().state == "preparing" {
            tokio::task::yield_now().await;
        }
        let (sender, _receiver) = mpsc::channel(64);
        let fake = super::super::DataWorker {
            sender,
            status: worker.status.clone(),
            stopping: Arc::new(AtomicBool::new(false)),
            dispatch: Arc::new(Mutex::new(())),
            root: worker.root.clone(),
            journal_io: worker.journal_io.clone(),
            lease_active: worker.lease_active.clone(),
        };
        for _ in 0..64 {
            let (response, _) = oneshot::channel();
            assert!(
                fake.sender
                    .try_send(super::super::Request {
                        operation: Operation::Retry,
                        response,
                        queued: std::time::Instant::now(),
                    })
                    .is_ok()
            );
        }
        let error = fake
            .request(
                "main",
                Operation::History(history::DbOperation::PruneCancelled {
                    attempt_id: attempt.clone(),
                    text: "前段。".into(),
                }),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, "busy");
        assert!(
            directory(root.path())
                .unwrap()
                .join(format!("{attempt}.json"))
                .exists()
        );
        let super::super::Output::History(history::DbOutput::Record(row)) = worker
            .request(
                "main",
                Operation::History(history::DbOperation::Detail(record)),
            )
            .await
            .unwrap()
        else {
            panic!("record expected")
        };
        assert_eq!(row.attempts[0].text, "前段。");
        assert!(
            !directory(root.path())
                .unwrap()
                .join(format!("{attempt}.json"))
                .exists()
        );
    }
}
