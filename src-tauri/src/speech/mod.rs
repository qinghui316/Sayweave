pub mod audio;
pub mod commands;
pub mod continuous;
mod doubao;
mod hotwords;
#[cfg(all(test, windows))]
mod live_continuation;
#[cfg(all(test, windows))]
mod live_hotwords;
mod local;
pub mod qwen;
mod recognition;
#[cfg(all(test, windows))]
pub(crate) use live_hotwords::learned_term_followup;
#[cfg(test)]
mod tests;
mod transport;
use crate::{
    contracts::speech::Phase,
    data::DataWorker,
    history::{HistoryService, playback::Playback, types::*},
    models::ModelService,
};
use serde::Serialize;
use std::{
    sync::{
        Arc, Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::Manager;
use tokio::sync::{Mutex as AsyncMutex, watch};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    Trial,
    External,
    History,
}
impl TryFrom<&str> for Source {
    type Error = Error;
    fn try_from(value: &str) -> Result<Self, Error> {
        match value {
            "trial" => Ok(Self::Trial),
            "external" => Ok(Self::External),
            "history" => Ok(Self::History),
            _ => Err(Error::new("invalid")),
        }
    }
}
impl Source {
    pub fn retry_source(self) -> Self {
        match self {
            Self::Trial => Self::Trial,
            Self::External | Self::History => Self::History,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trial => "trial",
            Self::External => "external",
            Self::History => "history",
        }
    }
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(skip)]
    pub source_record_id: Option<String>,
    pub protocol: u8,
    pub sequence: u64,
    pub lifecycle: u64,
    pub task_id: Option<String>,
    pub source: Option<Source>,
    pub attempt_id: Option<String>,
    pub record_id: Option<String>,
    pub phase: Phase,
    pub text: String,
    pub segments: Vec<recognition::Segment>,
    pub model_name: Option<String>,
    pub device_name: Option<String>,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub save_error: bool,
    pub saved: bool,
    pub retry: bool,
    pub can_retry: bool,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            source_record_id: None,
            protocol: 1,
            sequence: 1,
            lifecycle: 1,
            task_id: None,
            source: None,
            attempt_id: None,
            record_id: None,
            phase: Phase::Idle,
            text: String::new(),
            segments: vec![],
            model_name: None,
            device_name: None,
            duration_ms: 0,
            error: None,
            save_error: false,
            saved: false,
            retry: false,
            can_retry: false,
        }
    }
}
#[derive(Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Meter {
    pub(crate) task_id: String,
    sequence: u64,
    level: f32,
    energies: [f32; 15],
    bars: [f32; 15],
    capture_sequence: u64,
    duration_ms: u64,
}
#[derive(Clone)]
pub(crate) enum NativeEvent {
    Snapshot(Snapshot),
    Meter(Meter),
    ArchiveSettled(ArchiveSettlement),
}
type OutputObserver = Arc<dyn Fn(NativeEvent) + Send + Sync>;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ArchiveSettlement {
    pub task_id: String,
    pub saved: bool,
    pub save_error: bool,
    pub attempt_id: Option<String>,
    pub revision: u64,
    pub state: String,
}
struct Task {
    id: String,
    record_id: String,
    attempt_id: String,
    persistent: bool,
    retry: bool,
    audio: Option<Audio>,
    preferences: Preferences,
    replay: Option<Vec<i16>>,
    continuous: Option<continuous::Control>,
    hotwords: Vec<String>,
    archive: crate::archive::Archive,
    record_lease: Mutex<Option<Arc<crate::history::RecordLease>>>,
    audio_archive: Mutex<Option<(tokio::sync::oneshot::Receiver<bool>, u64)>>,
}
pub struct NativeTerminal {
    pub snapshot: Snapshot,
    pub transcript: NativeTranscript,
    pub(crate) archived: tokio::sync::oneshot::Receiver<ArchiveSettlement>,
    pub persistent: bool,
}
impl std::ops::Deref for NativeTerminal {
    type Target = Snapshot;
    fn deref(&self) -> &Snapshot {
        &self.snapshot
    }
}
pub struct NativeStart {
    pub purpose: crate::contracts::translation::Purpose,
    pub id: String,
    pub source: Source,
    pub expected_task: Option<String>,
    pub cancel: watch::Sender<u8>,
    pub terminal: tokio::sync::oneshot::Sender<NativeTerminal>,
    pub continuous: Option<continuous::Control>,
    pub hotwords: Vec<String>,
    pub configuration: Option<crate::models::SpeechConfiguration>,
}
/// Native cumulative recognition stream; separate from renderer and lifecycle projection.
#[derive(Clone, Default)]
pub struct NativeTranscript {
    pub task_id: String,
    pub turn: u64,
    pub sequence: u64,
    pub text: String,
    pub segments: Vec<crate::contracts::processing::SourceSegment>,
}
pub struct SpeechService {
    pub history: HistoryService,
    pub playback: Playback,
    snapshot: Mutex<Snapshot>,
    record_lease: Mutex<Option<(String, Weak<crate::history::RecordLease>)>>,
    states: watch::Sender<Snapshot>,
    transcripts: watch::Sender<NativeTranscript>,
    output: OnceLock<OutputObserver>,
    active: Mutex<Option<(String, watch::Sender<u8>)>>,
    gate: AsyncMutex<()>,
    stopping: AtomicBool,
    preparing: AtomicBool,
    ready: AtomicBool,
    init_gate: AsyncMutex<()>,
    last_audio: Mutex<Option<(String, Vec<i16>)>>,
    operation_gate: Mutex<()>,
    operations: Arc<AtomicUsize>,
    archive_revision: std::sync::atomic::AtomicU64,
}
pub struct FreshOperation(Arc<AtomicUsize>);
impl Drop for FreshOperation {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
impl SpeechService {
    pub fn new(history: HistoryService) -> Self {
        let (states, _) = watch::channel(Snapshot::default());
        let (transcripts, _) = watch::channel(NativeTranscript::default());
        Self {
            history,
            playback: Playback::new(),
            snapshot: Mutex::new(Snapshot::default()),
            record_lease: Mutex::new(None),
            states,
            transcripts,
            output: OnceLock::new(),
            active: Mutex::new(None),
            gate: AsyncMutex::new(()),
            stopping: AtomicBool::new(false),
            preparing: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            init_gate: AsyncMutex::new(()),
            last_audio: Mutex::new(None),
            operation_gate: Mutex::new(()),
            operations: Arc::new(AtomicUsize::new(0)),
            archive_revision: std::sync::atomic::AtomicU64::new(0),
        }
    }
    pub fn admit(&self) -> Result<FreshOperation, Error> {
        let _guard = self
            .operation_gate
            .lock()
            .map_err(|_| Error::new("unavailable"))?;
        if self.is_stopping() {
            return Err(Error::new("stopping"));
        }
        self.operations.fetch_add(1, Ordering::AcqRel);
        Ok(FreshOperation(self.operations.clone()))
    }
    pub(crate) fn task_record(&self, id: &str) -> Option<Arc<crate::history::RecordLease>> {
        self.record_lease
            .lock()
            .ok()?
            .as_ref()
            .filter(|(task, _)| task == id)?
            .1
            .upgrade()
    }
    pub fn snapshot(&self) -> Result<Snapshot, Error> {
        self.snapshot
            .lock()
            .map(|s| s.clone())
            .map_err(|_| Error::new("unavailable"))
    }
    pub fn subscribe(&self) -> watch::Receiver<Snapshot> {
        self.states.subscribe()
    }
    pub fn subscribe_transcripts(&self) -> watch::Receiver<NativeTranscript> {
        self.transcripts.subscribe()
    }
    pub(crate) fn set_output(&self, observer: OutputObserver) -> Result<(), Error> {
        self.output
            .set(observer)
            .map_err(|_| Error::new("conflict"))
    }
    fn publish(&self, event: NativeEvent) {
        if let Some(observer) = self.output.get() {
            observer(event);
        }
    }
    pub(crate) fn archive_settled(&self, mut result: ArchiveSettlement) {
        result.revision = self.archive_revision.fetch_add(1, Ordering::AcqRel) + 1;
        result.state = if result.saved {
            "saved"
        } else if result.save_error {
            if result
                .attempt_id
                .as_ref()
                .is_some_and(|id| self.history.data.pending_archive(id))
            {
                "pending"
            } else {
                "unavailable"
            }
        } else {
            "disabled"
        }
        .into();
        self.update(&result.task_id, |s| {
            s.saved = result.saved;
            s.save_error = result.save_error;
        });
        self.publish(NativeEvent::ArchiveSettled(result));
    }
    pub(crate) fn acknowledge_terminal(&self, id: &str, lifecycle: u64) {
        self.update_if(
            id,
            |s| s.lifecycle == lifecycle && matches!(s.phase, Phase::Completed | Phase::Cancelled),
            |s| s.phase = Phase::Idle,
        );
    }
    fn publish_transcript(&self, task: &str, turn: u64, transcript: recognition::Transcript) {
        let previous = self.transcripts.borrow().clone();
        let sequence = previous.sequence + 1;
        let segments = transcript
            .segments
            .iter()
            .map(|s| {
                let old = (previous.task_id == task && previous.turn == turn)
                    .then(|| previous.segments.iter().find(|p| p.id == s.id))
                    .flatten();
                if s.confirmed && old.is_none_or(|p| !p.confirmed || p.text != s.text) {
                    log::info!(target:"sayweave", "operation=speech_confirmed task_id={} turn={} segment_id={} sequence={} source_chars={}",task,turn,s.id,sequence,s.text.chars().count());
                }
                crate::contracts::processing::SourceSegment {
                    id: s.id,
                    text: s.text.clone(),
                    confirmed: s.confirmed,
                    revision: old
                        .map(|p| p.revision + u64::from(p.text != s.text))
                        .unwrap_or(1),
                }
            })
            .collect();
        self.transcripts.send_replace(NativeTranscript {
            task_id: task.into(),
            turn,
            sequence,
            text: transcript.text,
            segments,
        });
    }
    pub fn is_active(&self) -> bool {
        self.preparing.load(Ordering::Acquire)
            || self.active.lock().map(|s| s.is_some()).unwrap_or(true)
    }
    pub async fn initialize(&self) -> Result<(), Error> {
        let _operation = self.admit()?;
        let _guard = self.init_gate.lock().await;
        if !self.ready.load(Ordering::Acquire) {
            self.history.recover().await?;
            self.ready.store(true, Ordering::Release);
        }
        Ok(())
    }
    pub async fn maintain(&self) -> Result<(), Error> {
        // Hold admission across the whole cycle, including file cleanup after initialization.
        // The daily sleep belongs to the caller and must not keep shutdown waiting.
        let _operation = self.admit()?;
        self.initialize().await?;
        self.history.cleanup().await
    }
    fn update(&self, id: &str, change: impl FnOnce(&mut Snapshot)) {
        self.update_if(id, |_| true, change);
    }
    fn update_if(
        &self,
        id: &str,
        predicate: impl FnOnce(&Snapshot) -> bool,
        change: impl FnOnce(&mut Snapshot),
    ) {
        let update = if let Ok(mut s) = self.snapshot.lock() {
            if s.task_id.as_deref() != Some(id) || !predicate(&s) {
                return;
            }
            let phase = s.phase;
            change(&mut s);
            s.sequence += 1;
            if phase != s.phase {
                s.lifecycle += 1;
            }
            // The watch projection is published under this facts lock so a
            // delayed older clone cannot replace a newer native task.
            let copy = s.clone();
            self.states.send_replace(copy.clone());
            Some(copy)
        } else {
            None
        };
        if let Some(s) = update {
            // The application sink receives every transition, including a Trial
            // terminal before an External start. UI-specific projection is there.
            self.publish(NativeEvent::Snapshot(s));
        }
    }
    pub async fn start_native(
        &self,
        app: tauri::AppHandle,
        retry_id: Option<String>,
        expected_config: Option<(String, i64)>,
        context: NativeStart,
    ) -> Result<Snapshot, Error> {
        self.start_bound(
            app,
            retry_id,
            expected_config,
            context.expected_task.clone(),
            Some(context),
        )
        .await
    }
    async fn start_bound(
        &self,
        app: tauri::AppHandle,
        retry_id: Option<String>,
        expected_config: Option<(String, i64)>,
        expected_task: Option<String>,
        context: Option<NativeStart>,
    ) -> Result<Snapshot, Error> {
        let _gate = self.gate.try_lock().map_err(|_| Error::new("busy"))?;
        let _admission = app
            .state::<crate::lifecycle::Lifecycle>()
            .enter()
            .map_err(Error::new)?;
        let _operation = self.admit()?;
        if expected_task.is_some() && self.snapshot()?.task_id != expected_task {
            return Err(Error::new("staleSession"));
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::new("stopping"));
        }
        if self.is_active() {
            return Err(Error::new("busy"));
        }
        self.preparing.store(true, Ordering::Release);
        struct Preparing<'a>(&'a AtomicBool);
        impl Drop for Preparing<'_> {
            fn drop(&mut self) {
                self.0.store(false, Ordering::Release);
            }
        }
        let _preparing = Preparing(&self.preparing);
        if context.as_ref().is_some_and(|c| *c.cancel.borrow() >= 2) {
            return Err(Error::new("cancelled"));
        }
        self.initialize().await?;
        let cfg =
            if let Some(configuration) = context.as_ref().and_then(|c| c.configuration.as_ref()) {
                configuration.clone()
            } else {
                app.state::<ModelService>()
                    .speech_config()
                    .await
                    .map_err(|e| Error::new(&e.code))?
            };
        if expected_config
            .as_ref()
            .is_some_and(|(id, revision)| id != cfg.id() || *revision != cfg.revision())
        {
            return Err(Error::new("conflict"));
        }
        if context.as_ref().is_some_and(|c| *c.cancel.borrow() >= 2) {
            return Err(Error::new("cancelled"));
        }
        let mut preferences = self.history.preferences().await?;
        self.playback.control("stop", None).await?;
        let devices = if retry_id.is_none() {
            tokio::task::spawn_blocking(audio::devices)
                .await
                .map_err(|_| Error::new("deviceAccess"))??
        } else {
            vec![]
        };
        let device = match &preferences.device_id {
            Some(id) => devices.iter().find(|d| &d.id == id),
            None => devices.iter().find(|d| d.is_default),
        };
        if retry_id.is_none() && device.is_none() {
            return Err(Error::new("deviceMissing"));
        }
        if let Some(device) = device {
            preferences.device_id = Some(device.id.clone());
        }
        let task_id = context
            .as_ref()
            .map(|c| c.id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        if context.as_ref().is_some_and(|c| *c.cancel.borrow() >= 2) {
            return Err(Error::new("cancelled"));
        }
        let attempt_id = uuid::Uuid::new_v4().to_string();
        let record_id = retry_id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let mut lease = None;
        let (replay, persistent, audio) = if retry_id.is_some() {
            let record = match self.history.detail(&record_id).await {
                Ok(record) => Some(record),
                Err(e) if e.code == "notFound" => None,
                Err(e) => return Err(e),
            };
            if record
                .as_ref()
                .is_some_and(|r| r.status == "delete_pending")
            {
                return Err(Error::new("busy"));
            }
            let resource = record.as_ref().and_then(|r| r.audio.clone());
            let pcm = if let Some(a) = &resource {
                let held = self.history.lease(&a.id).await?;
                self.history.validate_asset(a).await?;
                let files = self.history.files.clone()?;
                let key = a.file_key.clone();
                let pcm = tokio::task::spawn_blocking(move || files.read(&key))
                    .await
                    .map_err(|_| Error::new("fileUnavailable"))??;
                lease = Some(held);
                pcm
            } else {
                self.last_audio
                    .lock()
                    .map_err(|_| Error::new("unavailable"))?
                    .as_ref()
                    .filter(|(id, _)| id == &record_id)
                    .map(|(_, pcm)| pcm.clone())
                    .ok_or(Error::new("fileUnavailable"))?
            };
            (Some(pcm), record.is_some(), resource)
        } else {
            let a = if preferences.history_enabled && preferences.audio_enabled {
                Some(Audio {
                    id: uuid::Uuid::new_v4().to_string(),
                    record_id: record_id.clone(),
                    file_key: uuid::Uuid::new_v4().to_string(),
                    status: "prepared".into(),
                    bytes: 0,
                    expires_at: preferences
                        .retention_days
                        .map(|d| crate::data::repository::now() + d as i64 * 86400000),
                })
            } else {
                None
            };
            (None, preferences.history_enabled, a)
        };
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::new("stopping"));
        }
        let attempt = Attempt {
            purpose: context.as_ref().map_or_default(|c| c.purpose),
            target_language: None,
            config_revision: cfg.revision(),
            id: attempt_id.clone(),
            record_id: record_id.clone(),
            kind: if retry_id.is_some() { "retry" } else { "live" }.into(),
            config_id: cfg.id().into(),
            config_name: cfg.name().into(),
            model_id: cfg.model_id().into(),
            service_host: cfg.service_host(),
            source: cfg.source(),
            created_at: crate::data::repository::now(),
            ended_at: None,
            status: "preparing".into(),
            text: String::new(),
            error_code: None,
            elapsed_ms: 0,
        };
        // Reserve the shared network slot before starting persistence or capture.
        let mut admission = app
            .state::<ModelService>()
            .requests
            .register(&task_id)
            .map_err(|e| Error::new(&e.code))?;
        let archive = crate::archive::Archive::new(self.history.data.clone(), task_id.clone())?;
        let record_lease = if persistent {
            Some(Arc::new(self.history.hold_record(&record_id)?))
        } else {
            None
        };
        if persistent {
            archive.enqueue(
                "prepare".into(),
                crate::data::Operation::History(if retry_id.is_some() {
                    DbOperation::Retry(Box::new(attempt))
                } else {
                    DbOperation::Prepare(Box::new(Prepare {
                        record_id: record_id.clone(),
                        attempt,
                        audio: audio.clone(),
                    }))
                }),
            );
        }
        let continuous = context.as_ref().and_then(|c| c.continuous.clone());
        let hotwords = if let Some(context) = &context {
            context.hotwords.clone()
        } else {
            match tokio::time::timeout(
                Duration::from_secs(2),
                app.state::<crate::data::DataWorker>().request_admitted(
                    crate::data::Operation::Processing(
                        crate::contracts::processing::DbOperation::DictionarySnapshot,
                    ),
                ),
            )
            .await
            {
                Ok(Ok(crate::data::Output::Processing(
                    crate::contracts::processing::DbOutput::Dictionary(records),
                ))) => crate::processing::prompt::DictionaryIndex::new(records).hotwords(),
                _ => vec![],
            }
        };
        let source = context.as_ref().map_or(
            if retry_id.is_some() {
                Source::History
            } else {
                Source::Trial
            },
            |context| context.source,
        );
        let (tx, rx, terminal) = if let Some(c) = context {
            let rx = c.cancel.subscribe();
            (c.cancel, rx, Some(c.terminal))
        } else {
            let (tx, rx) = watch::channel(if self.stopping.load(Ordering::Acquire) {
                3u8
            } else {
                0u8
            });
            (tx, rx, None)
        };
        *self.active.lock().map_err(|_| Error::new("unavailable"))? = Some((task_id.clone(), tx));
        let old = self.snapshot()?;
        *self
            .snapshot
            .lock()
            .map_err(|_| Error::new("unavailable"))? = Snapshot {
            source_record_id: Some(record_id.clone()),
            task_id: Some(task_id.clone()),
            source: Some(source),
            attempt_id: Some(attempt_id.clone()),
            record_id: if persistent {
                Some(record_id.clone())
            } else {
                None
            },
            sequence: old.sequence + 1,
            lifecycle: old.lifecycle + 1,
            model_name: Some(cfg.model_id().into()),
            device_name: device.map(|d| d.name.clone()),
            retry: retry_id.is_some(),
            ..Snapshot::default()
        };
        if retry_id.is_none() {
            *self
                .last_audio
                .lock()
                .map_err(|_| Error::new("unavailable"))? = None;
        }
        self.update(&task_id, |s| s.phase = Phase::Connecting);
        *self
            .record_lease
            .lock()
            .map_err(|_| Error::new("unavailable"))? = record_lease
            .as_ref()
            .map(|held| (task_id.clone(), Arc::downgrade(held)));
        let task = Task {
            record_lease: Mutex::new(record_lease),
            audio_archive: Mutex::new(None),
            archive,
            id: task_id,
            record_id,
            attempt_id,
            persistent,
            retry: retry_id.is_some(),
            audio,
            preferences,
            replay,
            continuous,
            hotwords,
        };
        tauri::async_runtime::spawn(async move {
            let service = app.state::<SpeechService>();
            let _lease = lease;
            // 10-minute capture guard plus setup/finalization allowance. Never a 30-second replay timeout.
            let id = task.id.clone();
            let mut cancel = rx;
            let result = service
                .run(&app, &task, &cfg, &mut cancel, &mut admission)
                .await;
            if let Err(e) = result {
                service.update(&id, |s| {
                    s.error = Some(e.code);
                    s.phase = Phase::Failed;
                });
            }
            let (archived, archive_done) = tokio::sync::oneshot::channel();
            let coordinated = terminal.is_some();
            if let Some(terminal) = terminal
                && let Ok(snapshot) = service.snapshot()
            {
                let _ = terminal.send(NativeTerminal {
                    snapshot,
                    transcript: {
                        let value = service.transcripts.borrow();
                        if value.task_id == id {
                            value.clone()
                        } else {
                            NativeTranscript {
                                task_id: id.clone(),
                                ..NativeTranscript::default()
                            }
                        }
                    },
                    archived: archive_done,
                    persistent: task.persistent,
                });
            }
            task.archive.seal_with(None);
            let archive = task.archive.clone();
            let handle = app.clone();
            let archive_id = id.clone();
            let archived_snapshot = service
                .snapshot()
                .ok()
                .filter(|s| s.task_id.as_deref() == Some(&id));
            let retained = task.persistent
                && archived_snapshot
                    .as_ref()
                    .is_some_and(|s| s.record_id.is_some());
            let prior_save_error = archived_snapshot.as_ref().is_some_and(|s| s.save_error);
            let record_lease = task.record_lease.lock().ok().and_then(|mut r| r.take());
            let audio_archive = task.audio_archive.lock().ok().and_then(|mut a| a.take());
            let audio_id = task
                .audio
                .as_ref()
                .filter(|_| task.persistent)
                .map(|a| a.id.clone());
            service.operations.fetch_add(1, Ordering::AcqRel);
            let operation = FreshOperation(service.operations.clone());
            tauri::async_runtime::spawn(async move {
                let _operation = operation;
                let mut success = archive.finish().await;
                if let Some((completion, bytes)) = audio_archive {
                    let audio_saved = completion.await.unwrap_or(false);
                    success &= audio_saved;
                    if let Some(audio_id) = audio_id {
                        success &= handle
                            .state::<DataWorker>()
                            .request_archive(crate::data::Operation::History(
                                DbOperation::AudioState {
                                    id: audio_id,
                                    status: if audio_saved { "ready" } else { "corrupt" }.into(),
                                    bytes,
                                },
                            ))
                            .await
                            .is_ok();
                    }
                }
                drop(record_lease);
                drop(_lease);
                let saved = retained && success && !prior_save_error;
                handle.state::<SpeechService>().update(&archive_id, |s| {
                    s.save_error |= !success;
                    s.saved = saved && !s.save_error;
                });
                let result = ArchiveSettlement {
                    task_id: archive_id,
                    saved,
                    save_error: prior_save_error || !success,
                    attempt_id: archived_snapshot
                        .as_ref()
                        .and_then(|s| s.attempt_id.clone()),
                    revision: 0,
                    state: String::new(),
                };
                if !coordinated {
                    handle
                        .state::<SpeechService>()
                        .archive_settled(result.clone());
                }
                let _ = archived.send(result);
            });
            if let Ok(mut a) = service.active.lock()
                && a.as_ref().is_some_and(|(current, _)| current == &id)
            {
                *a = None;
            }
        });
        self.snapshot()
    }
    async fn run(
        &self,
        app: &tauri::AppHandle,
        task: &Task,
        cfg: &crate::models::SpeechConfiguration,
        cancel: &mut watch::Receiver<u8>,
        admission: &mut crate::network::RequestGuard,
    ) -> Result<(), Error> {
        let began = Instant::now();
        let mut transcript = recognition::Transcript::default();
        let mut capture: Option<audio::Capture> = None;
        let mut meter_task: Option<tokio::task::JoinHandle<()>> = None;
        let mut duration = 0;
        let mut save_error = false;
        let mut abort = cancel.clone();
        let work = async {
            let mut session = continuous::Stream::open(
                cfg,
                &task.id,
                task.continuous.clone(),
                app.state::<ModelService>().requests.clone(),
                &task.hotwords,
            )
            .await?;
            if *cancel.borrow() >= 2 {
                return Err(Error::new("cancelled"));
            }
            if task.replay.is_none() {
                capture = Some(audio::Capture::prepare(
                    task.preferences.device_id.clone(),
                    task.audio
                        .as_ref()
                        .map(|a| self.history.files.clone().map(|f| (f, a.file_key.clone())))
                        .transpose()?,
                    cancel.clone(),
                    task.continuous.as_ref().map(|c| c.audio.clone()),
                )?);
                // Publish the cleanup owner before readiness can suspend or fail.
                // The common post-network drain owns file completion on every exit.
                capture.as_mut().unwrap().wait_ready().await?;
            }
            self.update(&task.id, |s| {
                s.phase = if task.retry {
                    Phase::Processing
                } else {
                    Phase::Recording
                }
            });
            if let Some(c) = capture.as_ref() {
                let handle = app.clone();
                let id = task.id.clone();
                let mut sequence = 0;
                meter_task = Some(audio::publish_meter(
                    c.meter.clone(),
                    cancel.clone(),
                    move |sample| {
                        sequence += 1;
                        let meter = Meter {
                            task_id: id.clone(),
                            sequence,
                            level: sample.level,
                            energies: sample.energies,
                            bars: sample.bars,
                            capture_sequence: sample.capture_sequence,
                            duration_ms: sample.duration_ms,
                        };
                        if handle
                            .state::<SpeechService>()
                            .snapshot()
                            .is_ok_and(|s| s.task_id.as_deref() == Some(&id))
                        {
                            handle
                                .state::<SpeechService>()
                                .publish(NativeEvent::Meter(meter));
                        }
                    },
                ));
            }
            log::info!(target:"sayweave","operation=speech_phase task_id={} phase=ready elapsed_ms={}",task.id,began.elapsed().as_millis());
            if task.persistent {
                task.archive.enqueue(
                    "stage".into(),
                    crate::data::Operation::History(DbOperation::Stage {
                        attempt_id: task.attempt_id.clone(),
                        status: if task.retry {
                            "processing"
                        } else {
                            "recording"
                        }
                        .into(),
                    }),
                );
            }
            let mut replay_at = 0;
            let mut sending = true;
            let mut final_at = None;
            let mut tick = tokio::time::interval(Duration::from_millis(50));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut published = Instant::now();
            let mut checkpoint = Instant::now();
            let mut dirty = false;
            loop {
                if *cancel.borrow() >= 2 {
                    return Err(Error::new("cancelled"));
                }
                if final_at.is_some_and(|at: Instant| at.elapsed() > Duration::from_secs(30)) {
                    return Err(Error::new("timeout"));
                }
                tokio::select! {
                                   biased;
                                   _=cancel.changed()=>{if *cancel.borrow()==1 {if let Some(c)=&capture{c.stop();}log::info!(target:"sayweave","operation=speech_phase task_id={} phase=capture_stopped elapsed_ms={}",task.id,began.elapsed().as_millis());self.update(&task.id,|s|s.phase=Phase::Processing);if task.persistent { task.archive.enqueue("stage".into(),crate::data::Operation::History(DbOperation::Stage{attempt_id:task.attempt_id.clone(),status:"processing".into()})); }}},
                                   message=session.next_event()=>{match message? {
                                       continuous::StreamEvent::Update(value)=>{
                                           transcript=value.raw;
                                           dirty=true;
                                           if let Some((turn,text))=value.draft {self.publish_transcript(&task.id,turn,text);}
                                           if !transcript.confirmed_text.is_empty()&&task.persistent&&checkpoint.elapsed()>Duration::from_secs(1){task.archive.enqueue("checkpoint".into(),crate::data::Operation::History(DbOperation::Checkpoint{attempt_id:task.attempt_id.clone(),text:transcript.text(true)}));checkpoint=Instant::now();}
                                       },
                                       continuous::StreamEvent::Finished=>{if sending{return Err(Error::new("protocol"));}
                if transcript.text(true).trim().is_empty(){return Err(Error::new("emptyText"));}log::info!(target:"sayweave","operation=speech_phase task_id={} phase=final_received elapsed_ms={}",task.id,began.elapsed().as_millis());break;},
                                       continuous::StreamEvent::Heartbeat=>{}
                                   }},
                                   _=tick.tick()=>{
                                       if began.elapsed()>Duration::from_secs(650){return Err(Error::new("timeout"));}
                                       if sending {
                                           if let Some(pcm)=&task.replay {
                                               let end=(replay_at+800).min(pcm.len());if replay_at<end {session.send_pcm(1,&pcm[replay_at..end]).await?;replay_at=end;duration=replay_at as u64/16;}else{sending=false;}
                                           } else if let Some(c)=capture.as_mut(){
                                               for _ in 0..16 {let Ok(packet)=c.packets.try_recv() else {break};duration=packet.duration_ms;session.send_pcm(packet.turn,&packet.pcm).await?;}
                                               if c.packets.is_closed()&&c.packets.is_empty(){sending=false;}
                                           }
                                           if !sending {session.finish().await?;final_at=Some(Instant::now());log::info!(target:"sayweave","operation=speech_phase task_id={} phase=audio_drained elapsed_ms={}",task.id,began.elapsed().as_millis());self.update(&task.id,|s|s.phase=Phase::Processing);if task.persistent { task.archive.enqueue("stage".into(),crate::data::Operation::History(DbOperation::Stage{attempt_id:task.attempt_id.clone(),status:"processing".into()})); }}
                                       }
                                       if published.elapsed()>=Duration::from_millis(100)&&(dirty||sending){self.update(&task.id,|s|{s.text=transcript.text(false);s.segments=transcript.segments();s.duration_ms=duration;});published=Instant::now();dirty=false;}
                                   }
                               }
            }
            Ok::<_, Error>(())
        };
        // The request coordinator gives speech and model tests the same four slots and shutdown signal.
        let mut outcome=admission.run(660,async {tokio::select!{
            result=work=>result.map_err(|e|crate::models::types::ModelError::new(&e.code)),
            _=async {loop{if *abort.borrow()>=2{break;}
 if abort.changed().await.is_err(){break;}}}=>Err(crate::models::types::ModelError::new("cancelled"))
        }}).await.map_err(|e|Error::new(&e.code));
        if let Some(task) = meter_task {
            task.abort();
        }
        // Stop and finalize outside the cancelled network future; never abandon an admitted file journal.
        if let Some(c) = capture.as_mut() {
            c.stop();
            if let Ok(result) = (&mut c.done).await {
                duration = result.pcm.len() as u64 / 16;
                save_error |= result.save_error;
                if let Some(error) = result.error
                    && *cancel.borrow() < 2
                {
                    outcome = Err(Error::new(&error));
                }
                if let Some(completion) = result.archived {
                    if let Ok(mut archive) = task.audio_archive.lock() {
                        *archive = Some((completion, result.bytes));
                    }
                } else if let Some(a) = task.audio.as_ref().filter(|_| task.persistent) {
                    task.archive.enqueue(
                        "audio".into(),
                        crate::data::Operation::History(DbOperation::AudioState {
                            id: a.id.clone(),
                            status: if save_error { "corrupt" } else { "missing" }.into(),
                            bytes: result.bytes,
                        }),
                    );
                }
                if let Ok(mut last) = self.last_audio.lock() {
                    *last = Some((task.record_id.clone(), result.pcm));
                }
                log::info!(target:"sayweave","SAYWEAVE_SPEECH operation=capture peak_frames={} duration_ms={} result={}",result.peak,duration,if outcome.is_ok(){"ok"}else{"failed"});
            }
        }
        if capture.is_none()
            && !task.retry
            && task.persistent
            && let Some(audio) = &task.audio
        {
            task.archive.enqueue(
                "audio".into(),
                crate::data::Operation::History(DbOperation::AudioState {
                    id: audio.id.clone(),
                    status: "missing".into(),
                    bytes: 0,
                }),
            );
        }
        if *cancel.borrow() >= 2 {
            outcome = Err(Error::new(if *cancel.borrow() == 2 {
                "cancelled"
            } else {
                "interrupted"
            }));
        }
        if let Some(raw) = task.continuous.as_ref().and_then(|c| c.latest_raw()) {
            transcript = raw;
        }
        let cancelled = outcome
            .as_ref()
            .err()
            .is_some_and(|e| e.code == "cancelled");
        if cancelled
            && !task.retry
            && !task
                .continuous
                .as_ref()
                .is_some_and(|c| c.preserve_history() || c.has_cancel_archive_owner())
        {
            let prune = DbOperation::PruneCancelled {
                attempt_id: task.attempt_id.clone(),
                text: String::new(),
            };
            if task.persistent {
                save_error |= self
                    .history
                    .data
                    .stage_cancel(&crate::data::Operation::History(prune.clone()))
                    .await
                    .is_err();
                task.archive.seal_with(Some(crate::data::Operation::History(
                    DbOperation::Finish {
                        attempt_id: task.attempt_id.clone(),
                        status: "cancelled".into(),
                        text: String::new(),
                        error: Some("cancelled".into()),
                        duration_ms: duration,
                        elapsed_ms: began.elapsed().as_millis() as u64,
                    },
                )));
            }
            save_error |= !task.archive.finish().await;
            let audio_archive = task.audio_archive.lock().ok().and_then(|mut a| a.take());
            if let Some((completion, _)) = audio_archive {
                save_error |= !completion.await.unwrap_or(false);
            }
            if task.persistent {
                save_error |= self.history.db(prune).await.is_err();
            }
            if let Ok(mut lease) = task.record_lease.lock() {
                lease.take();
            }
            save_error |= self.discard_new(&task.record_id, task.persistent).await;
            self.update(&task.id, |s| {
                s.text.clear();
                s.segments.clear();
                s.record_id = None;
                s.source_record_id = None;
                s.save_error = save_error;
                s.phase = Phase::Cancelled;
            });
            return Ok(());
        }
        let text = if outcome.is_ok() {
            transcript.text(true)
        } else {
            transcript.text(false)
        };
        if task.persistent {
            let status = if cancelled {
                "cancelled"
            } else if outcome
                .as_ref()
                .err()
                .is_some_and(|e| e.code == "interrupted")
            {
                "interrupted"
            } else if outcome.is_ok() {
                "success"
            } else {
                "failed"
            };
            task.archive
                .seal_with(Some(crate::data::Operation::History(DbOperation::Finish {
                    attempt_id: task.attempt_id.clone(),
                    status: status.into(),
                    text: text.clone(),
                    error: outcome.as_ref().err().map(|e| e.code.clone()),
                    duration_ms: duration,
                    elapsed_ms: began.elapsed().as_millis() as u64,
                })));
        }
        self.update(&task.id, |s| {
            s.text = text;
            s.segments = transcript.segments();
            s.duration_ms = duration;
            s.save_error = save_error;
            s.saved = false; // ArchiveSettled updates only this task after persistence.
            s.can_retry = self.last_audio.lock().is_ok_and(|last| {
                last.as_ref()
                    .is_some_and(|(id, pcm)| id == &task.record_id && !pcm.is_empty())
            }) || task.audio.as_ref().is_some_and(|a| a.status == "ready");
            s.error = outcome.as_ref().err().map(|e| e.code.clone());
            s.phase = if cancelled {
                Phase::Cancelled
            } else if outcome.is_ok() {
                Phase::Completed
            } else {
                Phase::Failed
            };
        });
        Ok(())
    }
    async fn discard_new(&self, record_id: &str, persistent: bool) -> bool {
        let failed = persistent
            && async {
                let record = self.history.detail(record_id).await?;
                self.history.remove(record_id, record.revision).await
            }
            .await
            .is_err();
        if let Ok(mut last) = self.last_audio.lock()
            && last.as_ref().is_some_and(|(id, _)| id == record_id)
        {
            *last = None;
        }
        failed
    }
    pub fn forget_audio(&self, record_id: &str) {
        if let Ok(mut last) = self.last_audio.lock()
            && last.as_ref().is_some_and(|(id, _)| id == record_id)
        {
            *last = None;
        }
        if let Ok(snapshot) = self.snapshot()
            && snapshot.record_id.as_deref() == Some(record_id)
            && let Some(task_id) = snapshot.task_id
        {
            self.update(&task_id, |s| {
                s.can_retry = false;
                s.saved = false;
                s.record_id = None;
            });
        }
    }
    pub(crate) fn invalidate_task_audio(&self, task_id: &str) {
        let Ok(snapshot) = self.snapshot() else {
            return;
        };
        if snapshot.task_id.as_deref() != Some(task_id) {
            return;
        }
        if let Ok(mut last) = self.last_audio.lock()
            && last
                .as_ref()
                .is_some_and(|(id, _)| snapshot.source_record_id.as_ref() == Some(id))
        {
            *last = None;
        }
        self.update(task_id, |s| {
            s.can_retry = false;
            s.source_record_id = None;
        });
    }
    pub fn start_error(&self, id: &str, source: Source, error: &Error) {
        let Ok(_gate) = self.gate.try_lock() else {
            return;
        };
        if self.is_active() || self.is_stopping() || error.code == "busy" {
            return;
        }
        if let Ok(mut s) = self.snapshot.lock() {
            let sequence = s.sequence + 1;
            *s = Snapshot::default();
            s.task_id = Some(id.into());
            s.source = Some(source);
            s.sequence = sequence;
        }
        self.update(id, |s| {
            s.error = Some(error.code.clone());
            s.phase = Phase::Failed;
        });
    }
    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }
    pub fn shutdown(&self) {
        let _guard = self.operation_gate.lock().ok();
        self.stopping.store(true, Ordering::Release);
        if let Ok(snapshot) = self.snapshot() {
            self.states.send_replace(snapshot);
        }
        if let Ok(a) = self.active.lock()
            && let Some((_, tx)) = a.as_ref()
        {
            let _ =
                crate::task_stop::request(tx, crate::contracts::speech::StopReason::Interrupted);
        }
    }
    pub async fn drain(&self) -> Result<(), Error> {
        while self.is_active() || self.operations.load(Ordering::Acquire) > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        self.playback.control("stop", None).await.map(|_| ())
    }
    pub async fn play(&self, id: &str) -> Result<crate::history::playback::PlaybackState, Error> {
        let _operation = self.admit()?;
        let _guard = self.gate.try_lock().map_err(|_| Error::new("busy"))?;
        if self.is_active() {
            return Err(Error::new("busy"));
        }
        let lease = self.history.lease(id).await?;
        let path = self.history.validate_asset(&lease.audio).await?;
        self.playback.start(path, lease).await
    }
}
