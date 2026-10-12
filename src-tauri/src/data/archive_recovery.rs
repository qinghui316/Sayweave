//! Ordinary archive recovery. Only persistence facts are replayed; never input or providers.
use super::{
    Operation,
    repository::{Repository, db_error},
};
use crate::contracts::{dictionary::DataError, history as h, processing as p};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct Stamp {
    epoch: u64,
    revision: u64,
}
pub(super) fn stamp() -> Stamp {
    static EPOCH: std::sync::LazyLock<u64> =
        std::sync::LazyLock::new(|| uuid::Uuid::new_v4().as_u128() as u64);
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    Stamp {
        epoch: *EPOCH,
        revision: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1,
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
enum Fact {
    Delivery {
        task: String,
        attempt: String,
        state: String,
        error: Option<String>,
    },
    Prepare {
        value: h::Prepare,
        audio_key: Option<String>,
    },
    Retry(h::Attempt),
    Stage {
        attempt_id: String,
        status: String,
    },
    Checkpoint {
        attempt_id: String,
        text: String,
    },
    Finish {
        attempt_id: String,
        status: String,
        text: String,
        error: Option<String>,
        duration_ms: u64,
        elapsed_ms: u64,
    },
    Audio {
        id: String,
        status: String,
        bytes: u64,
    },
    Run(p::RunMetadata),
    Unit(p::UnitRecord),
    FinishRun {
        id: String,
        text: String,
        state: String,
        error: Option<String>,
    },
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    schema: u32,
    key: String,
    created: i64,
    fact: Fact,
    #[serde(default)]
    stamp: Stamp,
}
impl Fact {
    fn capture(op: &Operation) -> Option<Self> {
        Some(match op {
            Operation::History(h::DbOperation::Delivery {
                task,
                attempt,
                state,
                error,
            }) => Self::Delivery {
                task: task.clone(),
                attempt: attempt.clone(),
                state: state.clone(),
                error: error.clone(),
            },
            Operation::History(h::DbOperation::Prepare(value)) => Self::Prepare {
                value: value.clone(),
                audio_key: value.audio.as_ref().map(|a| a.file_key.clone()),
            },
            Operation::History(h::DbOperation::Retry(a)) => Self::Retry(a.clone()),
            Operation::History(h::DbOperation::Stage { attempt_id, status }) => Self::Stage {
                attempt_id: attempt_id.clone(),
                status: status.clone(),
            },
            Operation::History(h::DbOperation::Checkpoint { attempt_id, text }) => {
                Self::Checkpoint {
                    attempt_id: attempt_id.clone(),
                    text: text.clone(),
                }
            }
            Operation::History(h::DbOperation::Finish {
                attempt_id,
                status,
                text,
                error,
                duration_ms,
                elapsed_ms,
            }) => Self::Finish {
                attempt_id: attempt_id.clone(),
                status: status.clone(),
                text: text.clone(),
                error: error.clone(),
                duration_ms: *duration_ms,
                elapsed_ms: *elapsed_ms,
            },
            Operation::History(h::DbOperation::AudioState { id, status, bytes }) => Self::Audio {
                id: id.clone(),
                status: status.clone(),
                bytes: *bytes,
            },
            Operation::Processing(p::DbOperation::Prepare(r)) => Self::Run(r.clone()),
            Operation::Processing(p::DbOperation::SaveUnit(u)) => Self::Unit(u.clone()),
            Operation::Processing(p::DbOperation::Finish {
                id,
                text,
                state,
                error,
            }) => Self::FinishRun {
                id: id.clone(),
                text: text.clone(),
                state: state.clone(),
                error: error.clone(),
            },
            _ => return None,
        })
    }
    fn compatible(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Prepare { .. }, Self::Prepare { .. })
            | (Self::Retry(_), Self::Retry(_))
            | (Self::Run(_), Self::Run(_))
            | (Self::Finish { .. }, Self::Finish { .. })
            | (Self::FinishRun { .. }, Self::FinishRun { .. }) => {
                serde_json::to_vec(self).ok() == serde_json::to_vec(other).ok()
            }
            (Self::Unit(a), Self::Unit(b)) => {
                a.id == b.id
                    && a.run_id == b.run_id
                    && a.turn == b.turn
                    && a.source_start == b.source_start
                    && a.source_end == b.source_end
                    && a.source_refs == b.source_refs
                    && a.source_text == b.source_text
            }
            (
                Self::Delivery {
                    task: a,
                    attempt: b,
                    ..
                },
                Self::Delivery {
                    task: c,
                    attempt: d,
                    ..
                },
            ) => a == c && b == d,
            _ => {
                std::mem::discriminant(self) == std::mem::discriminant(other)
                    && self.identity() == other.identity()
            }
        }
    }
    fn identity(&self) -> (&str, u8) {
        match self {
            Self::Delivery { task, .. } => (task, 8),
            Self::Prepare { value, .. } => (&value.attempt.id, 0),
            Self::Retry(a) => (&a.id, 0),
            Self::Run(r) => (&r.id, 1),
            Self::Stage { attempt_id, .. } => (attempt_id, 2),
            Self::Checkpoint { attempt_id, .. } => (attempt_id, 3),
            Self::Unit(u) => (&u.id, 4),
            Self::Audio { id, .. } => (id, 5),
            Self::Finish { attempt_id, .. } => (attempt_id, 6),
            Self::FinishRun { id, .. } => (id, 7),
        }
    }
    fn operation(self) -> Operation {
        match self {
            Self::Delivery {
                task,
                attempt,
                state,
                error,
            } => Operation::History(h::DbOperation::Delivery {
                task,
                attempt,
                state,
                error,
            }),
            Self::Prepare {
                mut value,
                audio_key,
            } => {
                if let Some(a) = &mut value.audio {
                    a.file_key = audio_key.unwrap_or_default();
                }
                Operation::History(h::DbOperation::Prepare(value))
            }
            Self::Retry(a) => Operation::History(h::DbOperation::Retry(a)),
            Self::Stage { attempt_id, status } => {
                Operation::History(h::DbOperation::Stage { attempt_id, status })
            }
            Self::Checkpoint { attempt_id, text } => {
                Operation::History(h::DbOperation::Checkpoint { attempt_id, text })
            }
            Self::Finish {
                attempt_id,
                status,
                text,
                error,
                duration_ms,
                elapsed_ms,
            } => Operation::History(h::DbOperation::Finish {
                attempt_id,
                status,
                text,
                error,
                duration_ms,
                elapsed_ms,
            }),
            Self::Audio { id, status, bytes } => {
                Operation::History(h::DbOperation::AudioState { id, status, bytes })
            }
            Self::Run(r) => Operation::Processing(p::DbOperation::Prepare(r)),
            Self::Unit(u) => Operation::Processing(p::DbOperation::SaveUnit(u)),
            Self::FinishRun {
                id,
                text,
                state,
                error,
            } => Operation::Processing(p::DbOperation::Finish {
                id,
                text,
                state,
                error,
            }),
        }
    }
}
fn regular(path: &Path, dir: bool) -> Result<(), DataError> {
    let m = fs::symlink_metadata(path).map_err(|_| DataError::new("unavailable"))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(DataError::new("forbidden"));
        }
    }
    if m.file_type().is_symlink() || dir != m.is_dir() || !dir && !m.is_file() {
        return Err(DataError::new("forbidden"));
    }
    Ok(())
}
fn directory(root: &Path) -> Result<PathBuf, DataError> {
    regular(root, true)?;
    let path = root
        .canonicalize()
        .map_err(|_| DataError::new("unavailable"))?
        .join("archive-recovery");
    fs::create_dir_all(&path).map_err(|_| DataError::new("unavailable"))?;
    regular(&path, true)?;
    Ok(path)
}
fn key(fact: &Fact) -> Result<String, DataError> {
    let (id, rank) = fact.identity();
    uuid::Uuid::parse_str(id).map_err(|_| DataError::new("invalid"))?;
    Ok(format!("{id}-{rank}"))
}
#[derive(Default)]
pub(super) struct Memory {
    facts: std::collections::BTreeMap<String, (Operation, Vec<u8>, Stamp)>,
    bytes: usize,
}
impl Memory {
    pub fn retain(&mut self, op: &Operation) {
        self.retain_stamped(op, stamp());
    }
    pub fn retain_stamped(&mut self, op: &Operation, ticket: Stamp) {
        let Some(fact) = Fact::capture(op) else {
            return;
        };
        let (Ok(key), Ok(body)) = (key(&fact), serde_json::to_vec(&fact)) else {
            return;
        };
        if let Some((old, _, version)) = self.facts.get(&key) {
            if version.epoch == ticket.epoch && version.revision > ticket.revision {
                return;
            }
            if Fact::capture(old).is_some_and(|old| !old.compatible(&fact)) {
                log::warn!(target:"sayweave","operation=archive_memory result=conflict");
                return;
            }
        }
        let old = self.facts.get(&key).map_or(0, |(_, b, _)| b.len());
        if self.bytes - old + body.len() > 2 * 1024 * 1024
            || !self.facts.contains_key(&key) && self.facts.len() >= 64
        {
            log::warn!(target:"sayweave","operation=archive_memory result=unavailable");
            return;
        }
        self.bytes = self.bytes - old + body.len();
        self.facts.insert(key, (op.clone(), body, ticket));
    }
    pub fn forget(&mut self, op: &Operation, ticket: Stamp) {
        let Some(fact) = Fact::capture(op) else {
            return;
        };
        let Ok(key) = key(&fact) else {
            return;
        };
        if self
            .facts
            .get(&key)
            .is_some_and(|(_, _, old)| old.epoch == ticket.epoch && old.revision <= ticket.revision)
            && let Some((_, body, _)) = self.facts.remove(&key)
        {
            self.bytes -= body.len();
        }
    }
    #[cfg(test)]
    pub fn invalidate(&mut self, repo: &Repository, record: &str) -> Result<(), DataError> {
        let mut attempts = Vec::new();
        let mut runs = Vec::new();
        let mut audio = Vec::new();
        let mut query = repo
            .connection
            .prepare("SELECT id FROM speech_attempts WHERE record_id=?1")
            .map_err(db_error)?;
        for id in query
            .query_map([record], |r| r.get::<_, String>(0))
            .map_err(db_error)?
        {
            attempts.push(id.map_err(db_error)?);
        }
        for (op, _, _) in self.facts.values() {
            match Fact::capture(op) {
                Some(Fact::Prepare { value, .. }) if value.record_id == record => {
                    attempts.push(value.attempt.id);
                    if let Some(a) = value.audio {
                        audio.push(a.id);
                    }
                }
                Some(Fact::Retry(a)) if a.record_id == record => attempts.push(a.id),
                _ => {}
            }
        }
        for (op, _, _) in self.facts.values() {
            if let Some(Fact::Run(r)) = Fact::capture(op)
                && attempts.contains(&r.attempt_id)
            {
                runs.push(r.id);
            }
        }
        let mut keys = Vec::new();
        for (key, (op, _, _)) in &self.facts {
            let mapped = match Fact::capture(op) {
                Some(Fact::Unit(u)) => runs.contains(&u.run_id),
                Some(Fact::FinishRun { id, .. }) => runs.contains(&id),
                Some(Fact::Audio { id, .. }) => audio.contains(&id),
                Some(Fact::Delivery { attempt, .. }) => attempts.contains(&attempt),
                _ => false,
            };
            if mapped || record_owner(repo, op)?.as_deref() == Some(record) {
                keys.push(key.clone());
            }
        }
        for key in keys {
            if let Some((_, body, _)) = self.facts.remove(&key) {
                self.bytes -= body.len();
            }
        }
        Ok(())
    }
    pub fn flush(&mut self, root: &Path) {
        let ready: Vec<_> = self
            .facts
            .iter()
            .filter_map(
                |(key, (op, _, ticket))| match stage_stamped(root, op, *ticket) {
                    Ok(Some(_)) => Some(key.clone()),
                    Err(e) if e.code == "staleArchive" => Some(key.clone()),
                    _ => None,
                },
            )
            .collect();
        for key in ready {
            if let Some((_, body, _)) = self.facts.remove(&key) {
                self.bytes -= body.len();
            }
        }
    }
}

#[cfg(test)]
pub(super) fn stage(root: &Path, op: &Operation) -> Result<Option<(PathBuf, Vec<u8>)>, DataError> {
    stage_stamped(root, op, stamp())
}
pub(super) fn stage_stamped(
    root: &Path,
    op: &Operation,
    ticket: Stamp,
) -> Result<Option<(PathBuf, Vec<u8>)>, DataError> {
    let Some(fact) = Fact::capture(op) else {
        return Ok(None);
    };
    let key = key(&fact)?;
    let body = serde_json::to_vec(&Entry {
        schema: 1,
        key: key.clone(),
        created: super::repository::now(),
        fact: fact.clone(),
        stamp: ticket,
    })
    .map_err(|_| DataError::new("invalid"))?;
    let dir = directory(root)?;
    let path = dir.join(format!("{key}.json"));
    if path.exists() {
        let (_, old) = read_entry(&path)?;
        if !old.fact.compatible(&fact) {
            return Err(DataError::new("conflict"));
        }
        if old.stamp.epoch == ticket.epoch && old.stamp.revision > ticket.revision {
            return Err(DataError::new("staleArchive"));
        }
    }
    let temp = dir.join(format!("{key}-{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|_| DataError::new("unavailable"))?;
    file.write_all(&body)
        .and_then(|_| file.sync_all())
        .map_err(|_| DataError::new("unavailable"))?;
    drop(file);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            Win32::Storage::FileSystem::{
                MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
            },
            core::PCWSTR,
        };
        let from: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
        let to: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        unsafe {
            MoveFileExW(
                PCWSTR(from.as_ptr()),
                PCWSTR(to.as_ptr()),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        }
        .map_err(|_| DataError::new("unavailable"))?;
    }
    #[cfg(not(windows))]
    fs::rename(&temp, &path).map_err(|_| DataError::new("unavailable"))?;
    Ok(Some((path, body)))
}
pub(super) fn acknowledge(receipt: &(PathBuf, Vec<u8>)) -> Result<(), DataError> {
    if !receipt.0.exists() {
        return Ok(());
    }
    regular(&receipt.0, false)?;
    if fs::read(&receipt.0).map_err(|_| DataError::new("unavailable"))? == receipt.1 {
        fs::remove_file(&receipt.0).map_err(|_| DataError::new("unavailable"))?;
    }
    Ok(())
}
pub(super) fn pending_attempt(root: &Path, attempt: &str) -> bool {
    let Ok(paths) = paths(root) else {
        return false;
    };
    for path in paths {
        if let Ok((_, e)) = read_entry(&path)
            && match &e.fact {
                Fact::Prepare { value, .. } => value.attempt.id == attempt,
                Fact::Retry(a) => a.id == attempt,
                Fact::Delivery { attempt: id, .. } => id == attempt,
                Fact::Run(r) => r.attempt_id == attempt,
                Fact::Stage { attempt_id, .. }
                | Fact::Checkpoint { attempt_id, .. }
                | Fact::Finish { attempt_id, .. } => attempt_id == attempt,
                _ => false,
            }
        {
            return true;
        }
    }
    false
}
fn paths(root: &Path) -> Result<Vec<PathBuf>, DataError> {
    let mut out = Vec::new();
    for entry in fs::read_dir(directory(root)?).map_err(|_| DataError::new("unavailable"))? {
        let path = entry.map_err(|_| DataError::new("unavailable"))?.path();
        if path.extension().is_some_and(|s| s == "json") {
            out.push(path);
        }
    }
    out.sort();
    Ok(out)
}
fn read_entry(path: &Path) -> Result<(Vec<u8>, Entry), DataError> {
    regular(path, false)?;
    if fs::metadata(path)
        .map_err(|_| DataError::new("unavailable"))?
        .len()
        > 16 * 1024 * 1024
    {
        return Err(DataError::new("invalid"));
    }
    let body = fs::read(path).map_err(|_| DataError::new("unavailable"))?;
    let entry: Entry = serde_json::from_slice(&body).map_err(|_| DataError::new("invalid"))?;
    if entry.schema != 1
        || key(&entry.fact)? != entry.key
        || path.file_stem().is_none_or(|s| s != entry.key.as_str())
    {
        return Err(DataError::new("invalid"));
    }
    Ok((body, entry))
}
#[cfg(test)]
fn entries(root: &Path) -> Result<Vec<(PathBuf, Vec<u8>, Entry)>, DataError> {
    paths(root)?
        .into_iter()
        .map(|p| read_entry(&p).map(|(b, e)| (p, b, e)))
        .collect()
}
fn record_owner(repo: &Repository, op: &Operation) -> Result<Option<String>, DataError> {
    Ok(match op {
        Operation::History(h::DbOperation::Delivery { attempt, .. }) => repo.connection.query_row("SELECT record_id FROM speech_attempts WHERE id=?1", [attempt], |r| r.get::<_, String>(0)).optional().map_err(db_error)?,
        Operation::History(h::DbOperation::Prepare(v)) => Some(v.record_id.clone()),
        Operation::History(h::DbOperation::Retry(a)) => Some(a.record_id.clone()),
        Operation::History(h::DbOperation::Stage { attempt_id, .. } | h::DbOperation::Checkpoint { attempt_id, .. } | h::DbOperation::Finish { attempt_id, .. }) => repo.connection.query_row("SELECT record_id FROM speech_attempts WHERE id=?1", [attempt_id], |r| r.get::<_, String>(0)).optional().map_err(db_error)?,
        Operation::History(h::DbOperation::AudioState { id, .. }) => repo.connection.query_row("SELECT record_id FROM audio_assets WHERE id=?1", [id], |r| r.get::<_, String>(0)).optional().map_err(db_error)?,
        Operation::Processing(p::DbOperation::Prepare(r)) => repo.connection.query_row("SELECT record_id FROM speech_attempts WHERE id=?1", [&r.attempt_id], |r| r.get::<_, String>(0)).optional().map_err(db_error)?,
        Operation::Processing(p::DbOperation::SaveUnit(p::UnitRecord { run_id: id, .. }) | p::DbOperation::Finish { id, .. }) => repo.connection.query_row("SELECT a.record_id FROM processing_runs p JOIN speech_attempts a ON a.id=p.attempt_id WHERE p.id=?1", [id], |r| r.get::<_, String>(0)).optional().map_err(db_error)?,
        _ => None,
    })
}
/// Repository-state checks make retries safe even after a lost acknowledgement.
pub(super) fn apply(repo: &mut Repository, op: Operation) -> Result<super::Output, DataError> {
    repo.connection
        .busy_timeout(std::time::Duration::ZERO)
        .map_err(db_error)?;
    let result = apply_inner(repo, op);
    if let Err(e) = repo
        .connection
        .busy_timeout(std::time::Duration::from_secs(5))
    {
        log::warn!(target:"sayweave","operation=archive_timeout_restore result={:?}",e.sqlite_error_code());
    }
    result
}
fn known_attempt(repo: &Repository, a: &h::Attempt) -> Result<Option<bool>, DataError> {
    repo.connection.query_row("SELECT record_id=?2 AND kind=?3 AND config_id=?4 AND config_name=?5 AND model_id=?6 AND service_host=?7 AND config_revision=?8 AND created_at=?9 FROM speech_attempts WHERE id=?1",rusqlite::params![a.id,a.record_id,a.kind,a.config_id,a.config_name,a.model_id,a.service_host,a.config_revision,a.created_at],|r|r.get(0)).optional().map_err(db_error)
}
fn apply_inner(repo: &mut Repository, op: Operation) -> Result<super::Output, DataError> {
    if let Operation::History(h::DbOperation::Delivery {
        task,
        attempt,
        state,
        ..
    }) = &op
    {
        let existing: Option<(String, String)> = repo
            .connection
            .query_row(
                "SELECT attempt_id,state FROM delivery_attempts WHERE task_id=?1",
                [task],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .map_err(db_error)?;
        if let Some((prior, old)) = existing {
            if prior != *attempt {
                return Err(DataError::new("conflict"));
            }
            let rank = |s: &str| match s {
                "pending" => 0,
                "delivering" => 1,
                "submitted" => 2,
                _ => 3,
            };
            if rank(&old) > rank(state) {
                return Ok(super::Output::History(h::DbOutput::Done));
            }
        }
    }
    let record = record_owner(repo, &op)?;
    if record.is_none() && !matches!(op, Operation::History(h::DbOperation::Prepare(_))) {
        return Err(DataError::new("notFound"));
    }
    if let Operation::History(h::DbOperation::AudioState { id, status, .. }) = &op {
        let protected: bool = repo.connection.query_row("SELECT EXISTS(SELECT 1 FROM audio_assets a WHERE a.id=?1 AND (a.status IN ('expired','expire_pending','delete_pending') OR EXISTS(SELECT 1 FROM discarded_audio d WHERE d.id=a.id)))", [id], |r| r.get(0)).map_err(db_error)?;
        if protected && status != "expired" {
            return Ok(super::Output::History(h::DbOutput::Done));
        }
    }
    if let Operation::History(h::DbOperation::Retry(a)) = &op
        && let Some(same) = known_attempt(repo, a)?
    {
        return if same {
            Ok(super::Output::History(h::DbOutput::Done))
        } else {
            Err(DataError::new("conflict"))
        };
    }
    if let Some(record) = record {
        let deleting: bool = repo.connection.query_row("SELECT EXISTS(SELECT 1 FROM speech_records WHERE id=?1 AND status='delete_pending')", [record], |r| r.get(0)).map_err(db_error)?;
        if deleting {
            return match op {
                Operation::History(_) => Ok(super::Output::History(h::DbOutput::Done)),
                _ => Ok(super::Output::Processing(p::DbOutput::Done)),
            };
        }
    }
    match &op {
        Operation::History(h::DbOperation::Prepare(v)) => {
            if v.attempt.record_id != v.record_id {
                return Err(DataError::new("invalid"));
            }
            if let Some(same) = known_attempt(repo, &v.attempt)? {
                return if same {
                    Ok(super::Output::History(h::DbOutput::Done))
                } else {
                    Err(DataError::new("conflict"))
                };
            }
        }
        Operation::Processing(p::DbOperation::Prepare(v)) => {
            let same:Option<bool>=repo.connection.query_row("SELECT attempt_id=?2 AND task_id=?3 AND config_id=?4 AND config_name=?5 AND model_id=?6 AND config_revision=?7 AND prompt_version=?8 AND settings_revision=?9 AND dictionary_version=?10 FROM processing_runs WHERE id=?1",rusqlite::params![v.id,v.attempt_id,v.task_id,v.config_id,v.config_name,v.model_id,v.config_revision,v.prompt_version,v.settings_revision,v.dictionary_version],|r|r.get(0)).optional().map_err(db_error)?;
            if let Some(same) = same {
                return if same {
                    Ok(super::Output::Processing(p::DbOutput::Done))
                } else {
                    Err(DataError::new("conflict"))
                };
            }
        }
        Operation::History(h::DbOperation::Finish {
            attempt_id,
            status,
            text,
            error,
            ..
        }) => {
            let row: Option<(String, String, Option<String>)> = repo
                .connection
                .query_row(
                    "SELECT status,text,error_code FROM speech_attempts WHERE id=?1",
                    [attempt_id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(db_error)?;
            if let Some((old, body, cause)) = row
                && !["preparing", "recording", "processing"].contains(&old.as_str())
            {
                return if old == "cancelled" || old == *status && body == *text && cause == *error {
                    Ok(super::Output::History(h::DbOutput::Done))
                } else {
                    Err(DataError::new("conflict"))
                };
            }
        }
        Operation::Processing(p::DbOperation::Finish {
            id,
            state,
            text,
            error,
        }) => {
            let row: Option<(String, String, Option<String>)> = repo
                .connection
                .query_row(
                    "SELECT state,text,error_code FROM processing_runs WHERE id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()
                .map_err(db_error)?;
            if let Some((old, body, cause)) = row
                && old != "running"
            {
                return if old == "cancelled" || old == *state && body == *text && cause == *error {
                    Ok(super::Output::Processing(p::DbOutput::Done))
                } else {
                    Err(DataError::new("conflict"))
                };
            }
        }
        Operation::Processing(p::DbOperation::SaveUnit(u)) => {
            let state: Option<String> = repo
                .connection
                .query_row(
                    "SELECT state FROM processing_runs WHERE id=?1",
                    [&u.run_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db_error)?;
            if state.as_deref() == Some("cancelled") {
                return Ok(super::Output::Processing(p::DbOutput::Done));
            }
            if state.as_deref().is_some_and(|s| s != "running") {
                let same: bool = repo.connection.query_row("SELECT EXISTS(SELECT 1 FROM processing_units WHERE id=?1 AND run_id=?2 AND source_text=?3 AND source_refs=?4 AND candidate_text IS ?5 AND generation_state=?6 AND application_state=?7 AND error_code IS ?8)", rusqlite::params![u.id, u.run_id, u.source_text, u.source_refs, u.candidate_text, u.generation_state, u.application_state, u.error_code], |r| r.get(0)).map_err(db_error)?;
                return if same {
                    Ok(super::Output::Processing(p::DbOutput::Done))
                } else {
                    Err(DataError::new("conflict"))
                };
            }
        }
        _ => {}
    }
    match op {
        Operation::History(op) => repo.history_operation(op).map(super::Output::History),
        Operation::Processing(op) => repo.processing_operation(op).map(super::Output::Processing),
        _ => Err(DataError::new("invalid")),
    }
}
pub(super) fn replay(
    repo: &mut Repository,
    root: &Path,
    prepare_only: bool,
) -> Result<(), DataError> {
    replay_batch(
        repo,
        root,
        prepare_only,
        std::time::Duration::from_millis(25),
        64,
    )
}
fn replay_batch(
    repo: &mut Repository,
    root: &Path,
    prepare_only: bool,
    budget: std::time::Duration,
    limit: usize,
) -> Result<(), DataError> {
    let paths: Vec<_> = paths(root)?
        .into_iter()
        .filter(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.rsplit('-').next())
                .and_then(|s| s.parse::<u8>().ok())
                .is_some_and(|rank| (rank <= 1) == prepare_only)
        })
        .collect();
    if paths.is_empty() {
        return Ok(());
    }
    let phase = usize::from(!prepare_only);
    let start = repo.archive_cursor[phase] % paths.len();
    let began = std::time::Instant::now();
    let mut visited = 0;
    for path in paths
        .iter()
        .cycle()
        .skip(start)
        .take(paths.len().min(limit))
    {
        visited += 1;
        let result = read_entry(path).and_then(|(body, entry)| {
            apply(repo, entry.fact.operation()).and_then(|_| acknowledge(&(path.clone(), body)))
        });
        if let Err(e) = result {
            log::warn!(target:"sayweave","operation=archive_recovery result={}",e.code);
            if e.code == "busy" {
                break;
            }
        }
        if began.elapsed() > budget {
            break;
        }
    }
    repo.archive_cursor[phase] = (start + visited) % paths.len();
    Ok(())
}
struct Bindings {
    record: String,
    attempts: Vec<String>,
    runs: Vec<String>,
    audio: Vec<String>,
}
impl Bindings {
    fn new(repo: &Repository, record: &str) -> Result<Self, DataError> {
        let mut value = Self {
            record: record.into(),
            attempts: Vec::new(),
            runs: Vec::new(),
            audio: Vec::new(),
        };
        for (sql, target) in [
            (
                "SELECT id FROM speech_attempts WHERE record_id=?1",
                &mut value.attempts,
            ),
            (
                "SELECT id FROM processing_runs WHERE attempt_id IN (SELECT id FROM speech_attempts WHERE record_id=?1)",
                &mut value.runs,
            ),
            (
                "SELECT id FROM audio_assets WHERE record_id=?1",
                &mut value.audio,
            ),
        ] {
            let mut query = repo.connection.prepare(sql).map_err(db_error)?;
            for id in query
                .query_map([record], |r| r.get::<_, String>(0))
                .map_err(db_error)?
            {
                target.push(id.map_err(db_error)?);
            }
        }
        Ok(value)
    }
    fn seed(&mut self, fact: &Fact) {
        match fact {
            Fact::Prepare { value, .. } if value.record_id == self.record => {
                self.attempts.push(value.attempt.id.clone());
                if let Some(a) = &value.audio {
                    self.audio.push(a.id.clone());
                }
            }
            Fact::Retry(a) if a.record_id == self.record => self.attempts.push(a.id.clone()),
            _ => {}
        }
    }
    fn run(&mut self, fact: &Fact) {
        if let Fact::Run(r) = fact
            && self.attempts.contains(&r.attempt_id)
        {
            self.runs.push(r.id.clone());
        }
    }
    fn owns(&self, fact: &Fact) -> bool {
        match fact {
            Fact::Prepare { value, .. } => value.record_id == self.record,
            Fact::Retry(a) => a.record_id == self.record,
            Fact::Run(r) => self.attempts.contains(&r.attempt_id),
            Fact::Unit(u) => self.runs.contains(&u.run_id),
            Fact::FinishRun { id, .. } => self.runs.contains(id),
            Fact::Delivery { attempt, .. } => self.attempts.contains(attempt),
            Fact::Audio { id, .. } => self.audio.contains(id),
            Fact::Stage { attempt_id, .. }
            | Fact::Checkpoint { attempt_id, .. }
            | Fact::Finish { attempt_id, .. } => self.attempts.contains(attempt_id),
        }
    }
}
/// Build one ownership graph before clearing either medium. SQL delete_pending
/// remains the recovery anchor if any I/O prevents complete invalidation.
pub(super) fn invalidate_all(
    repo: &Repository,
    root: &Path,
    record: &str,
    memory: &mut Memory,
) -> Result<(), DataError> {
    let files = paths(root)?;
    let mut bindings = Bindings::new(repo, record)?;
    for (op, _, _) in memory.facts.values() {
        if let Some(fact) = Fact::capture(op) {
            bindings.seed(&fact);
        }
    }
    for path in &files {
        let (_, e) = read_entry(path)?;
        bindings.seed(&e.fact);
    }
    for (op, _, _) in memory.facts.values() {
        if let Some(fact) = Fact::capture(op) {
            bindings.run(&fact);
        }
    }
    for path in &files {
        let (_, e) = read_entry(path)?;
        bindings.run(&e.fact);
    }
    let keys: Vec<_> = memory
        .facts
        .iter()
        .filter_map(|(key, (op, _, _))| {
            Fact::capture(op)
                .filter(|f| bindings.owns(f))
                .map(|_| key.clone())
        })
        .collect();
    // Keep the graph until all disk facts are removed; no partially cleared
    // parent mapping can strand a child in the other medium.
    for path in files {
        let (body, e) = read_entry(&path)?;
        if bindings.owns(&e.fact) {
            acknowledge(&(path, body))?;
        }
    }
    for key in keys {
        if let Some((_, body, _)) = memory.facts.remove(&key) {
            memory.bytes -= body.len();
        }
    }
    Ok(())
}
#[cfg(test)]
fn invalidate_record(repo: &Repository, root: &Path, record: &str) -> Result<(), DataError> {
    invalidate_all(repo, root, record, &mut Memory::default())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn prepare() -> h::Prepare {
        let id = || uuid::Uuid::new_v4().to_string();
        h::Prepare {
            record_id: id(),
            attempt: h::Attempt {
                id: id(),
                record_id: String::new(),
                kind: "live".into(),
                config_id: id(),
                config_name: "fixture".into(),
                model_id: "fixture".into(),
                service_host: "localhost".into(),
                config_revision: 1,
                created_at: super::super::repository::now(),
                ended_at: None,
                status: "preparing".into(),
                text: String::new(),
                error_code: None,
                elapsed_ms: 0,
            },
            audio: None,
        }
    }
    #[test]
    fn lost_ack_and_restart_replay_one_result_without_duplicates() {
        let temp = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(temp.path()).unwrap();
        let mut p = prepare();
        p.attempt.record_id = p.record_id.clone();
        let op = Operation::History(h::DbOperation::Prepare(p.clone()));
        stage(temp.path(), &op).unwrap();
        apply(&mut repo, op).unwrap();
        let end = Operation::History(h::DbOperation::Finish {
            attempt_id: p.attempt.id.clone(),
            status: "success".into(),
            text: "保留正文".into(),
            error: None,
            duration_ms: 10,
            elapsed_ms: 20,
        });
        stage(temp.path(), &end).unwrap();
        drop(repo);
        let repo = Repository::open(temp.path()).unwrap();
        let text: String = repo
            .connection
            .query_row(
                "SELECT text FROM speech_attempts WHERE id=?1",
                [&p.attempt.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(text, "保留正文");
        assert!(entries(temp.path()).unwrap().is_empty());
        let count: u32 = repo
            .connection
            .query_row("SELECT count(*) FROM speech_records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
    #[test]
    fn delete_invalidates_pending_creation_and_terminal_facts() {
        let temp = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(temp.path()).unwrap();
        let mut p = prepare();
        p.attempt.record_id = p.record_id.clone();
        let op = Operation::History(h::DbOperation::Prepare(p.clone()));
        apply(&mut repo, op.clone()).unwrap();
        stage(temp.path(), &op).unwrap();
        let end = Operation::History(h::DbOperation::Finish {
            attempt_id: p.attempt.id.clone(),
            status: "success".into(),
            text: "不能恢复已删除文字".into(),
            error: None,
            duration_ms: 0,
            elapsed_ms: 0,
        });
        stage(temp.path(), &end).unwrap();
        invalidate_record(&repo, temp.path(), &p.record_id).unwrap();
        repo.connection
            .execute("DELETE FROM speech_records WHERE id=?1", [&p.record_id])
            .unwrap();
        drop(repo);
        let repo = Repository::open(temp.path()).unwrap();
        assert!(entries(temp.path()).unwrap().is_empty());
        assert_eq!(
            repo.connection
                .query_row("SELECT count(*) FROM speech_records", [], |r| r
                    .get::<_, u32>(0))
                .unwrap(),
            0
        );
    }
    #[test]
    fn checkpoint_coalesces_and_corrupt_recovery_is_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let _repo = Repository::open(temp.path()).unwrap();
        let attempt = uuid::Uuid::new_v4().to_string();
        let old = stage(
            temp.path(),
            &Operation::History(h::DbOperation::Checkpoint {
                attempt_id: attempt.clone(),
                text: "旧".into(),
            }),
        )
        .unwrap()
        .unwrap();
        let new = stage(
            temp.path(),
            &Operation::History(h::DbOperation::Checkpoint {
                attempt_id: attempt,
                text: "新".into(),
            }),
        )
        .unwrap()
        .unwrap();
        acknowledge(&old).unwrap();
        assert!(new.0.exists());
        fs::write(&new.0, b"invalid").unwrap();
        assert!(entries(temp.path()).is_err());
        assert!(new.0.exists());
    }
    #[test]
    fn missing_parent_preserves_checkpoint_and_unrelated_database_reads() {
        let temp = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(temp.path()).unwrap();
        let op = Operation::History(h::DbOperation::Checkpoint {
            attempt_id: uuid::Uuid::new_v4().to_string(),
            text: "待恢复正文".into(),
        });
        let receipt = stage(temp.path(), &op).unwrap().unwrap();
        assert_eq!(apply(&mut repo, op).unwrap_err().code, "notFound");
        replay(&mut repo, temp.path(), false).unwrap();
        assert!(receipt.0.exists());
        repo.history_operation(h::DbOperation::Preferences).unwrap();
    }
    #[test]
    fn retry_and_cancelled_audio_lost_ack_are_safe_to_replay() {
        let temp = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(temp.path()).unwrap();
        let mut p = prepare();
        p.attempt.record_id = p.record_id.clone();
        let aid = uuid::Uuid::new_v4().to_string();
        p.audio = Some(h::Audio {
            id: aid.clone(),
            record_id: p.record_id.clone(),
            file_key: uuid::Uuid::new_v4().to_string(),
            status: "prepared".into(),
            bytes: 0,
            expires_at: None,
        });
        apply(
            &mut repo,
            Operation::History(h::DbOperation::Prepare(p.clone())),
        )
        .unwrap();
        let mut attempt = p.attempt.clone();
        attempt.id = uuid::Uuid::new_v4().to_string();
        attempt.kind = "retry".into();
        let op = Operation::History(h::DbOperation::Retry(attempt));
        stage(temp.path(), &op).unwrap();
        apply(&mut repo, op.clone()).unwrap();
        apply(&mut repo, op).unwrap();
        let ready = Operation::History(h::DbOperation::AudioState {
            id: aid.clone(),
            status: "ready".into(),
            bytes: 100,
        });
        stage(temp.path(), &ready).unwrap();
        repo.history_operation(h::DbOperation::DiscardAudio(p.record_id))
            .unwrap();
        replay(&mut repo, temp.path(), true).unwrap();
        replay(&mut repo, temp.path(), false).unwrap();
        assert_ne!(
            repo.connection
                .query_row("SELECT status FROM audio_assets WHERE id=?1", [aid], |r| {
                    r.get::<_, String>(0)
                })
                .unwrap(),
            "ready"
        );
        assert!(entries(temp.path()).unwrap().is_empty());
    }
    #[test]
    fn volatile_checkpoint_survives_failed_file_stage_then_flushes_without_duplication() {
        let temp = tempfile::tempdir().unwrap();
        let mut memory = Memory::default();
        let id = uuid::Uuid::new_v4().to_string();
        fs::write(
            temp.path().join("archive-recovery"),
            b"unwritable directory fixture",
        )
        .unwrap();
        let old = Operation::History(h::DbOperation::Checkpoint {
            attempt_id: id.clone(),
            text: "旧".into(),
        });
        let new = Operation::History(h::DbOperation::Checkpoint {
            attempt_id: id,
            text: "新".into(),
        });
        memory.retain(&old);
        memory.retain(&new);
        memory.flush(temp.path());
        assert_eq!(memory.facts.len(), 1);
        fs::remove_file(temp.path().join("archive-recovery")).unwrap();
        memory.flush(temp.path());
        assert!(memory.facts.is_empty());
        assert_eq!(entries(temp.path()).unwrap().len(), 1);
    }
    #[test]
    fn deletion_removes_volatile_children_even_when_run_parent_was_never_saved() {
        let temp = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(temp.path()).unwrap();
        let mut p = prepare();
        p.attempt.record_id = p.record_id.clone();
        apply(
            &mut repo,
            Operation::History(h::DbOperation::Prepare(p.clone())),
        )
        .unwrap();
        let run = uuid::Uuid::new_v4().to_string();
        let mut memory = Memory::default();
        memory.retain(&Operation::Processing(p::DbOperation::Prepare(
            p::RunMetadata {
                id: run.clone(),
                attempt_id: p.attempt.id,
                task_id: uuid::Uuid::new_v4().to_string(),
                config_id: uuid::Uuid::new_v4().to_string(),
                config_name: "fixture".into(),
                model_id: "fixture".into(),
                config_revision: 1,
                prompt_version: "v1".into(),
                settings_revision: 1,
                dictionary_version: "v1".into(),
            },
        )));
        memory.retain(&Operation::Processing(p::DbOperation::SaveUnit(
            p::UnitRecord {
                id: uuid::Uuid::new_v4().to_string(),
                run_id: run.clone(),
                turn: 1,
                source_start: 0,
                source_end: 1,
                source_refs: "[]".into(),
                source_text: "删除正文".into(),
                candidate_text: None,
                generation_state: "pending".into(),
                application_state: "pending".into(),
                source_committed_end: None,
                error_code: None,
                prompt_tokens: None,
                completion_tokens: None,
                elapsed_ms: 0,
            },
        )));
        memory.retain(&Operation::Processing(p::DbOperation::Finish {
            id: run,
            text: "删除正文".into(),
            state: "success".into(),
            error: None,
        }));
        memory.invalidate(&repo, &p.record_id).unwrap();
        assert!(memory.facts.is_empty());
        memory.flush(temp.path());
        assert!(entries(temp.path()).unwrap().is_empty());
    }

    #[test]
    fn partial_batches_advance_each_phase_without_starving_parent_facts() {
        let temp = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(temp.path()).unwrap();
        for n in 0..3 {
            stage(
                temp.path(),
                &Operation::Processing(p::DbOperation::Prepare(p::RunMetadata {
                    id: format!("00000000-0000-4000-8000-{n:012}"),
                    attempt_id: uuid::Uuid::new_v4().to_string(),
                    task_id: uuid::Uuid::new_v4().to_string(),
                    config_id: uuid::Uuid::new_v4().to_string(),
                    config_name: "fixture".into(),
                    model_id: "fixture".into(),
                    config_revision: 1,
                    prompt_version: "v1".into(),
                    settings_revision: 1,
                    dictionary_version: "v1".into(),
                })),
            )
            .unwrap();
            let mut p = prepare();
            p.attempt.id = format!("ffffffff-0000-4000-8000-{n:012}");
            p.attempt.record_id = p.record_id.clone();
            stage(temp.path(), &Operation::History(h::DbOperation::Prepare(p))).unwrap();
        }
        for _ in 0..12 {
            replay_batch(&mut repo, temp.path(), true, std::time::Duration::ZERO, 64).unwrap();
            replay_batch(&mut repo, temp.path(), false, std::time::Duration::ZERO, 64).unwrap();
        }
        assert_eq!(
            repo.connection
                .query_row("SELECT count(*) FROM speech_records", [], |r| r
                    .get::<_, u32>(0))
                .unwrap(),
            3
        );
        assert_eq!(entries(temp.path()).unwrap().len(), 3);
    }
    #[test]
    fn successful_newer_checkpoint_retires_old_volatile_without_reverting_text() {
        let temp = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(temp.path()).unwrap();
        let mut p = prepare();
        p.attempt.record_id = p.record_id.clone();
        apply(
            &mut repo,
            Operation::History(h::DbOperation::Prepare(p.clone())),
        )
        .unwrap();
        let old = Operation::History(h::DbOperation::Checkpoint {
            attempt_id: p.attempt.id.clone(),
            text: "旧正文".into(),
        });
        let new = Operation::History(h::DbOperation::Checkpoint {
            attempt_id: p.attempt.id.clone(),
            text: "新的正文".into(),
        });
        let a = stamp();
        let b = stamp();
        let mut memory = Memory::default();
        memory.retain_stamped(&old, a);
        stage_stamped(temp.path(), &new, b).unwrap();
        apply(&mut repo, new.clone()).unwrap();
        memory.forget(&new, b);
        memory.flush(temp.path());
        assert!(memory.facts.is_empty());
        assert_eq!(
            repo.connection
                .query_row(
                    "SELECT text FROM speech_attempts WHERE id=?1",
                    [p.attempt.id],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "新的正文"
        );
        assert_eq!(
            stage_stamped(temp.path(), &old, a).unwrap_err().code,
            "staleArchive"
        );
    }
    #[test]
    fn deletion_joins_disk_and_volatile_parent_graphs_before_clearing_either() {
        for parent_in_memory in [true, false] {
            let temp = tempfile::tempdir().unwrap();
            let mut repo = Repository::open(temp.path()).unwrap();
            let mut p = prepare();
            p.attempt.record_id = p.record_id.clone();
            apply(
                &mut repo,
                Operation::History(h::DbOperation::Prepare(p.clone())),
            )
            .unwrap();
            let run = uuid::Uuid::new_v4().to_string();
            let parent = Operation::Processing(p::DbOperation::Prepare(p::RunMetadata {
                id: run.clone(),
                attempt_id: p.attempt.id,
                task_id: uuid::Uuid::new_v4().to_string(),
                config_id: uuid::Uuid::new_v4().to_string(),
                config_name: "fixture".into(),
                model_id: "fixture".into(),
                config_revision: 1,
                prompt_version: "v1".into(),
                settings_revision: 1,
                dictionary_version: "v1".into(),
            }));
            let child = Operation::Processing(p::DbOperation::Finish {
                id: run,
                text: "用户已删除的正文".into(),
                state: "success".into(),
                error: None,
            });
            let mut memory = Memory::default();
            if parent_in_memory {
                memory.retain(&parent);
                stage(temp.path(), &child).unwrap();
            } else {
                stage(temp.path(), &parent).unwrap();
                memory.retain(&child);
            }
            invalidate_all(&repo, temp.path(), &p.record_id, &mut memory).unwrap();
            assert!(memory.facts.is_empty());
            assert!(entries(temp.path()).unwrap().is_empty());
        }
    }
}
