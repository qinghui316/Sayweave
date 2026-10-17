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
pub(crate) struct Stamp {
    epoch: u64,
    revision: u64,
}
pub(crate) fn stamp() -> Stamp {
    static EPOCH: std::sync::LazyLock<u64> =
        std::sync::LazyLock::new(|| uuid::Uuid::new_v4().as_u128() as u64);
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    Stamp {
        epoch: *EPOCH,
        revision: NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1,
    }
}
#[derive(Default)]
pub(super) struct Causal {
    keys: std::sync::Mutex<std::collections::HashMap<String, (usize, Option<Stamp>)>>,
}
struct LiveKey {
    owner: std::sync::Arc<Causal>,
    key: String,
}
impl Drop for LiveKey {
    fn drop(&mut self) {
        if let Ok(mut keys) = self.owner.keys.lock()
            && let Some((live, _)) = keys.get_mut(&self.key)
        {
            *live -= 1;
            if *live == 0 {
                keys.remove(&self.key);
            }
        }
    }
}
#[derive(Clone)]
pub(crate) struct Ticket {
    pub(super) stamp: Stamp,
    lease: Option<std::sync::Arc<LiveKey>>,
}
pub(crate) fn untracked_ticket() -> Ticket {
    Ticket {
        stamp: stamp(),
        lease: None,
    }
}
impl Causal {
    pub fn ticket(self: &std::sync::Arc<Self>, op: &Operation, version: Stamp) -> Ticket {
        let key = Fact::capture(op).and_then(|f| key(&f).ok());
        let lease = key.and_then(|key| {
            self.keys.lock().ok().map(|mut keys| {
                keys.entry(key.clone()).or_default().0 += 1;
                std::sync::Arc::new(LiveKey {
                    owner: self.clone(),
                    key,
                })
            })
        });
        Ticket {
            stamp: version,
            lease,
        }
    }
    pub fn superseded(&self, ticket: &Ticket) -> Result<bool, DataError> {
        let keys = self
            .keys
            .lock()
            .map_err(|_| DataError::new("unavailable"))?;
        Ok(ticket
            .lease
            .as_ref()
            .and_then(|l| keys.get(&l.key))
            .and_then(|(_, s)| *s)
            .is_some_and(|s| s.epoch == ticket.stamp.epoch && s.revision > ticket.stamp.revision))
    }
    fn applied(&self, ticket: &Ticket) -> Result<(), DataError> {
        let mut keys = self
            .keys
            .lock()
            .map_err(|_| DataError::new("unavailable"))?;
        if let Some(lease) = &ticket.lease
            && let Some((_, known)) = keys.get_mut(&lease.key)
            && known.is_none_or(|s| {
                s.epoch != ticket.stamp.epoch || s.revision <= ticket.stamp.revision
            })
        {
            *known = Some(ticket.stamp);
        }
        Ok(())
    }
}
fn done(op: &Operation) -> Result<super::Output, DataError> {
    match op {
        Operation::History(_) => Ok(super::Output::History(h::DbOutput::Done)),
        Operation::Processing(_) => Ok(super::Output::Processing(p::DbOutput::Done)),
        _ => Err(DataError::new("invalid")),
    }
}
pub(super) fn superseded_output(op: &Operation) -> Result<super::Output, DataError> {
    done(op)
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
    Draft(crate::contracts::draft::Checkpoint),
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
            Operation::Processing(p::DbOperation::SaveDraft(d)) => Self::Draft(d.clone()),
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
            (Self::Draft(a), Self::Draft(b)) => {
                a.run_id == b.run_id && a.source.block == b.source.block && a.order == b.order
            }
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
            Self::Draft(d) => (&d.source.block, 9),
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
            Self::Draft(d) => Operation::Processing(p::DbOperation::SaveDraft(d)),
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
    if let Fact::Delivery { task, .. } = fact
        && let Some(copy) = task.strip_prefix("copy:")
    {
        let parsed = uuid::Uuid::parse_str(copy).map_err(|_| DataError::new("invalid"))?;
        if parsed.to_string() != copy {
            return Err(DataError::new("invalid"));
        }
        // A closed delivery namespace, not a path supplied by the caller.
        // Keep ':' out of Windows filenames and retain the native UUID rule
        // for every other kind of archive fact.
        return Ok(format!("copy-{parsed}-{rank}"));
    }
    uuid::Uuid::parse_str(id).map_err(|_| DataError::new("invalid"))?;
    Ok(format!("{id}-{rank}"))
}
#[derive(Default)]
pub(super) struct Memory {
    facts: std::collections::BTreeMap<String, (Operation, Vec<u8>, Ticket)>,
    bytes: usize,
}
impl Memory {
    #[cfg(test)]
    pub fn retain(&mut self, op: &Operation) {
        self.retain_stamped(op, untracked_ticket());
    }
    pub fn retain_stamped(&mut self, op: &Operation, ticket: Ticket) {
        let Some(fact) = Fact::capture(op) else {
            return;
        };
        let (Ok(key), Ok(body)) = (key(&fact), serde_json::to_vec(&fact)) else {
            return;
        };
        if let Some((old, _, version)) = self.facts.get(&key) {
            if version.stamp.epoch == ticket.stamp.epoch
                && version.stamp.revision > ticket.stamp.revision
            {
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
    pub fn forget(&mut self, op: &Operation, ticket: &Ticket) {
        let Some(fact) = Fact::capture(op) else {
            return;
        };
        let Ok(key) = key(&fact) else {
            return;
        };
        if self.facts.get(&key).is_some_and(|(_, _, old)| {
            old.stamp.epoch == ticket.stamp.epoch && old.stamp.revision <= ticket.stamp.revision
        }) && let Some((_, body, _)) = self.facts.remove(&key)
        {
            self.bytes -= body.len();
        }
    }
    pub fn pending_run(&self, run: &str) -> bool {
        self.facts.values().any(|(op, _, _)| {
            matches!(op, Operation::Processing(p::DbOperation::SaveUnit(u)) if u.run_id == run)
                || matches!(op, Operation::Processing(p::DbOperation::SaveDraft(d)) if d.run_id == run)
                || matches!(op, Operation::Processing(p::DbOperation::Prepare(r)) if r.id == run)
        })
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
                Some(Fact::Draft(d)) => runs.contains(&d.run_id),
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
                |(key, (op, _, ticket))| match stage_stamped(root, op, ticket.stamp) {
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
        schema: 2,
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
    if !matches!(entry.schema, 1 | 2)
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
        Operation::Processing(p::DbOperation::SaveDraft(d))=>repo.connection.query_row("SELECT a.record_id FROM processing_runs p JOIN speech_attempts a ON a.id=p.attempt_id WHERE p.id=?1", [&d.run_id], |r| r.get::<_, String>(0)).optional().map_err(db_error)?,
        _ => None,
    })
}
/// Repository-state checks make retries safe even after a lost acknowledgement.
#[cfg(test)]
pub(super) fn apply(repo: &mut Repository, op: Operation) -> Result<super::Output, DataError> {
    apply_stamped(repo, op, stamp())
}
pub(super) fn apply_stamped(
    repo: &mut Repository,
    op: Operation,
    version: Stamp,
) -> Result<super::Output, DataError> {
    let ticket = repo.archive_causal.ticket(&op, version);
    if repo.archive_causal.superseded(&ticket)? {
        return done(&op);
    }
    // A terminal run cannot seal out its still-pending attempt/checkpoint
    // facts. File names and replay cursor order do not establish causality.
    if let Operation::Processing(p::DbOperation::Finish { id, .. }) = &op
        && pending_run(&repo.archive_root, id)?
    {
        return Err(DataError::new("busy"));
    }
    repo.connection
        .busy_timeout(std::time::Duration::ZERO)
        .map_err(db_error)?;
    let result = apply_inner(repo, op);
    if result.is_ok() {
        repo.archive_causal.applied(&ticket)?;
    }
    if let Err(e) = repo
        .connection
        .busy_timeout(std::time::Duration::from_secs(5))
    {
        log::warn!(target:"sayweave","operation=archive_timeout_restore result={:?}",e.sqlite_error_code());
    }
    result
}
fn pending_run(root: &Path, run: &str) -> Result<bool, DataError> {
    for path in paths(root)? {
        let (_, entry) = match read_entry(&path) {
            Ok(value) => value,
            Err(error) => {
                // An unparseable retained fact has no authenticated run owner.
                // Keep it for recovery diagnostics; do not block unrelated runs.
                log::warn!(target:"sayweave", "operation=archive_dependency result=unattributed error={}", error.code);
                continue;
            }
        };
        if matches!(&entry.fact, Fact::Run(r) if r.id == run)
            || matches!(&entry.fact, Fact::Unit(u) if u.run_id == run)
            || matches!(&entry.fact, Fact::Draft(d) if d.run_id == run)
        {
            return Ok(true);
        }
    }
    Ok(false)
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
        Operation::Processing(p::DbOperation::SaveDraft(d)) => {
            if !d.valid() {
                return Err(DataError::new("invalid"));
            }
            let run: Option<String> = repo
                .connection
                .query_row(
                    "SELECT state FROM processing_runs WHERE id=?1",
                    [&d.run_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db_error)?;
            if run.as_deref().is_some_and(|s| s != "running") && d.state == "preview" {
                return Ok(super::Output::Processing(p::DbOutput::Done));
            }
            let known:Option<(String,i64,String,String)>=repo.connection.query_row("SELECT run_id,visible_revision,visible_text,state FROM processing_draft_blocks WHERE id=?1",[&d.source.block],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional().map_err(db_error)?;
            if let Some((run, revision, text, state)) = known {
                if run != d.run_id {
                    return Err(DataError::new("conflict"));
                }
                if revision as u64 > d.visible_revision
                    || state != "preview" && d.state == "preview"
                {
                    return Ok(super::Output::Processing(p::DbOutput::Done));
                }
                if state != "preview"
                    && (text != d.visible_text
                        || state != d.state
                        || revision as u64 != d.visible_revision)
                {
                    return Err(DataError::new("conflict"));
                }
            }
        }
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
            let same:Option<bool>=repo.connection.query_row("SELECT attempt_id=?2 AND task_id=?3 AND config_id=?4 AND config_name=?5 AND model_id=?6 AND config_revision=?7 AND prompt_version=?8 AND settings_revision=?9 AND dictionary_version=?10 AND mode=?11 FROM processing_runs WHERE id=?1",rusqlite::params![v.id,v.attempt_id,v.task_id,v.config_id,v.config_name,v.model_id,v.config_revision,v.prompt_version,v.settings_revision,v.dictionary_version,v.mode],|r|r.get(0)).optional().map_err(db_error)?;
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
            apply_stamped(repo, entry.fact.operation(), entry.stamp)
                .and_then(|_| acknowledge(&(path.clone(), body)))
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
            Fact::Draft(d) => self.runs.contains(&d.run_id),
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
    #[test]
    fn clipboard_delivery_stages_and_replays_without_overwriting_native_receipt() {
        let temp = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(temp.path()).unwrap();
        let mut prepared = prepare();
        prepared.attempt.record_id = prepared.record_id.clone();
        apply(
            &mut repo,
            Operation::History(h::DbOperation::Prepare(prepared.clone())),
        )
        .unwrap();
        let native = uuid::Uuid::new_v4().to_string();
        apply(
            &mut repo,
            Operation::History(h::DbOperation::Delivery {
                task: native.clone(),
                attempt: prepared.attempt.id.clone(),
                state: "unconfirmed".into(),
                error: Some("compositionRejected".into()),
            }),
        )
        .unwrap();
        let copy = |state: &str, error: Option<&str>| {
            Operation::History(h::DbOperation::Delivery {
                task: format!("copy:{}", uuid::Uuid::new_v4()),
                attempt: prepared.attempt.id.clone(),
                state: state.into(),
                error: error.map(str::to_owned),
            })
        };
        let success = copy("confirmed", None);
        let receipt = stage(temp.path(), &success).unwrap().unwrap();
        assert!(
            receipt
                .0
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("copy-")
        );
        let causal = std::sync::Arc::new(Causal::default());
        assert!(causal.ticket(&success, stamp()).lease.is_some());
        let failure = copy("failed", Some("copyFailed"));
        let mut memory = Memory::default();
        memory.retain(&failure);
        assert_eq!(memory.facts.len(), 1);
        memory.flush(temp.path());
        assert!(memory.facts.is_empty());
        let pending = entries(temp.path()).unwrap().len();
        assert_eq!(pending, 2);
        drop(repo);
        let mut repo = Repository::open(temp.path()).unwrap();
        // Startup replay is bounded. Continue the same maintenance path without
        // requiring both receipts to fit in its first 25ms pass on every disk.
        for _ in 0..pending {
            replay_batch(&mut repo, temp.path(), false, std::time::Duration::ZERO, 1).unwrap();
        }
        assert!(entries(temp.path()).unwrap().is_empty());
        let rows: Vec<(String,Option<String>)>=repo.connection.prepare("SELECT state,error_code FROM delivery_attempts WHERE task_id LIKE 'copy:%' ORDER BY state").unwrap().query_map([],|row|Ok((row.get(0)?,row.get(1)?))).unwrap().collect::<Result<_,_>>().unwrap();
        assert_eq!(
            rows,
            vec![
                ("confirmed".into(), None),
                ("failed".into(), Some("copyFailed".into()))
            ]
        );
        let original: (String, Option<String>) = repo
            .connection
            .query_row(
                "SELECT state,error_code FROM delivery_attempts WHERE task_id=?1",
                [native],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            original,
            ("unconfirmed".into(), Some("compositionRejected".into()))
        );
        for task in [
            "copy:../outside".into(),
            format!("other:{}", uuid::Uuid::new_v4()),
            format!("copy:{}:extra", uuid::Uuid::new_v4()),
        ] {
            let invalid = Fact::Delivery {
                task,
                attempt: prepared.attempt.id.clone(),
                state: "confirmed".into(),
                error: None,
            };
            assert!(key(&invalid).is_err());
        }
    }

    #[tokio::test]
    async fn clipboard_receipt_uses_the_real_data_worker_archive_route() {
        let temp = tempfile::tempdir().unwrap();
        let mut prepared = prepare();
        prepared.attempt.record_id = prepared.record_id.clone();
        let mut repo = Repository::open(temp.path()).unwrap();
        apply(
            &mut repo,
            Operation::History(h::DbOperation::Prepare(prepared.clone())),
        )
        .unwrap();
        drop(repo);
        let worker = super::super::DataWorker::start(temp.path().to_owned()).unwrap();
        let copy = format!("copy:{}", uuid::Uuid::new_v4());
        let op = Operation::History(h::DbOperation::Delivery {
            task: copy.clone(),
            attempt: prepared.attempt.id,
            state: "confirmed".into(),
            error: None,
        });
        let ticket = worker.archive_ticket(&op);
        assert!(
            ticket.lease.is_some(),
            "clipboard receipt must have the ordinary causal archive lease"
        );
        worker.request_archive_stamped(op, ticket).await.unwrap();
        worker.shutdown().await;
        let repo = Repository::open(temp.path()).unwrap();
        let state: String = repo
            .connection
            .query_row(
                "SELECT state FROM delivery_attempts WHERE task_id=?1",
                [copy],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "confirmed");
        assert!(entries(temp.path()).unwrap().is_empty());
    }
    #[test]
    fn draft_recovery_cannot_seal_run_before_pending_attempt_and_visible_checkpoint() {
        use crate::contracts::draft::{Association, BaselineKind, Checkpoint, Snapshot};
        let temp = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(temp.path()).unwrap();
        let mut prepared = prepare();
        prepared.attempt.record_id = prepared.record_id.clone();
        apply(
            &mut repo,
            Operation::History(h::DbOperation::Prepare(prepared.clone())),
        )
        .unwrap();
        let run = uuid::Uuid::from_u128(1).to_string();
        apply(
            &mut repo,
            Operation::Processing(p::DbOperation::Prepare(p::RunMetadata {
                mode: "native".into(),
                id: run.clone(),
                attempt_id: prepared.attempt.id,
                task_id: uuid::Uuid::new_v4().to_string(),
                config_id: String::new(),
                config_name: "口述结果".into(),
                model_id: String::new(),
                config_revision: 1,
                prompt_version: "native-v1".into(),
                settings_revision: 1,
                dictionary_version: "terms".into(),
            })),
        )
        .unwrap();
        let source = Snapshot {
            block: uuid::Uuid::from_u128(300).to_string(),
            version: 2,
            source_epoch: 2,
            baseline_kind: BaselineKind::EditorSnapshot,
            text: "周四开会".into(),
            speech_end: 0,
            insertion_end: 4,
        };
        let association = Association::attempt(1, source.clone());
        let unit = p::UnitRecord {
            id: uuid::Uuid::from_u128(200).to_string(),
            run_id: run.clone(),
            turn: 1,
            source_start: 0,
            source_end: 4,
            source_refs: serde_json::to_string(&association).unwrap(),
            source_text: source.text.clone(),
            candidate_text: None,
            generation_state: "skipped".into(),
            application_state: "notApplied".into(),
            source_committed_end: None,
            error_code: None,
            prompt_tokens: None,
            completion_tokens: None,
            elapsed_ms: 0,
        };
        stage(
            temp.path(),
            &Operation::Processing(p::DbOperation::SaveUnit(unit)),
        )
        .unwrap();
        let checkpoint = Checkpoint {
            run_id: run.clone(),
            order: 1,
            source,
            visible_revision: 3,
            visible_text: "周四开会".into(),
            state: "stopped".into(),
        };
        stage(
            temp.path(),
            &Operation::Processing(p::DbOperation::SaveDraft(checkpoint)),
        )
        .unwrap();
        let finish = Operation::Processing(p::DbOperation::Finish {
            id: run.clone(),
            text: "周四开会".into(),
            state: "success".into(),
            error: None,
        });
        stage(temp.path(), &finish).unwrap();
        assert_eq!(apply(&mut repo, finish).unwrap_err().code, "busy");
        for _ in 0..12 {
            replay_batch(
                &mut repo,
                temp.path(),
                false,
                std::time::Duration::from_secs(1),
                1,
            )
            .unwrap();
        }
        assert!(entries(temp.path()).unwrap().is_empty());
        let result:(String,i64,String) = repo.connection.query_row("SELECT state,(SELECT count(*) FROM processing_units WHERE run_id=?1),(SELECT visible_text FROM processing_draft_blocks WHERE run_id=?1) FROM processing_runs WHERE id=?1", [&run], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
        assert_eq!(result, ("success".into(), 1, "周四开会".into()));
        replay(&mut repo, temp.path(), false).unwrap();
        invalidate_record(&repo, temp.path(), &prepared.record_id).unwrap();
    }
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
    fn unrelated_corrupt_recovery_file_is_retained_without_blocking_normal_run_finish() {
        let temp = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(temp.path()).unwrap();
        let mut prepared = prepare();
        prepared.attempt.record_id = prepared.record_id.clone();
        apply(
            &mut repo,
            Operation::History(h::DbOperation::Prepare(prepared.clone())),
        )
        .unwrap();
        let run = uuid::Uuid::new_v4().to_string();
        apply(
            &mut repo,
            Operation::Processing(p::DbOperation::Prepare(p::RunMetadata {
                mode: "native".into(),
                id: run.clone(),
                attempt_id: prepared.attempt.id,
                task_id: uuid::Uuid::new_v4().to_string(),
                config_id: String::new(),
                config_name: "口述结果".into(),
                model_id: String::new(),
                config_revision: 1,
                prompt_version: "native-v1".into(),
                settings_revision: 1,
                dictionary_version: "terms".into(),
            })),
        )
        .unwrap();
        let unknown = directory(temp.path())
            .unwrap()
            .join(format!("{}-4.json", uuid::Uuid::new_v4()));
        fs::write(&unknown, b"damaged unrelated fact").unwrap();
        apply(
            &mut repo,
            Operation::Processing(p::DbOperation::Finish {
                id: run.clone(),
                text: "正常保留结果".into(),
                state: "success".into(),
                error: None,
            }),
        )
        .unwrap();
        assert_eq!(
            repo.connection
                .query_row(
                    "SELECT text FROM processing_runs WHERE id=?1",
                    [&run],
                    |r| r.get::<_, String>(0)
                )
                .unwrap(),
            "正常保留结果"
        );
        assert!(unknown.exists());
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
                mode: "model".into(),
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
                    mode: "model".into(),
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
        let a = untracked_ticket();
        let b = untracked_ticket();
        let mut memory = Memory::default();
        memory.retain_stamped(&old, a.clone());
        stage_stamped(temp.path(), &new, b.stamp).unwrap();
        apply(&mut repo, new.clone()).unwrap();
        memory.forget(&new, &b);
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
            stage_stamped(temp.path(), &old, a.stamp).unwrap_err().code,
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
                mode: "model".into(),
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
