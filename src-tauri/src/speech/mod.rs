pub mod audio;
pub mod commands;
mod doubao;
pub mod qwen;
mod recognition;
#[cfg(test)]
mod tests;
mod transport;
use crate::{
    coordinator::Phase,
    history::{HistoryService, playback::Playback, types::*},
    models::ModelService,
};
use serde::Serialize;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager};
use tokio::sync::{Mutex as AsyncMutex, watch};
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(skip)]
    pub runtime_id: u64,
    #[serde(skip)]
    pub source_record_id: Option<String>,
    pub protocol: u8,
    pub sequence: u64,
    pub lifecycle: u64,
    pub task_id: Option<String>,
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
            runtime_id: 0,
            source_record_id: None,
            protocol: 1,
            sequence: 1,
            lifecycle: 1,
            task_id: None,
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
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct Meter {
    task_id: String,
    sequence: u64,
    level: f32,
    duration_ms: u64,
}
struct Task {
    id: String,
    runtime_id: u64,
    record_id: String,
    attempt_id: String,
    persistent: bool,
    retry: bool,
    audio: Option<Audio>,
    preferences: Preferences,
    replay: Option<Vec<i16>>,
}
pub struct SpeechService {
    pub history: HistoryService,
    pub playback: Playback,
    snapshot: Mutex<Snapshot>,
    active: Mutex<Option<(String, watch::Sender<u8>)>>,
    gate: AsyncMutex<()>,
    stopping: AtomicBool,
    preparing: AtomicBool,
    ready: AtomicBool,
    init_gate: AsyncMutex<()>,
    last_audio: Mutex<Option<(String, Vec<i16>)>>,
    operation_gate: Mutex<()>,
    operations: Arc<AtomicUsize>,
}
pub struct FreshOperation(Arc<AtomicUsize>);
impl Drop for FreshOperation {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
impl SpeechService {
    pub fn new(history: HistoryService) -> Self {
        Self {
            history,
            playback: Playback::new(),
            snapshot: Mutex::new(Snapshot::default()),
            active: Mutex::new(None),
            gate: AsyncMutex::new(()),
            stopping: AtomicBool::new(false),
            preparing: AtomicBool::new(false),
            ready: AtomicBool::new(false),
            init_gate: AsyncMutex::new(()),
            last_audio: Mutex::new(None),
            operation_gate: Mutex::new(()),
            operations: Arc::new(AtomicUsize::new(0)),
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
    pub fn snapshot(&self) -> Result<Snapshot, Error> {
        self.snapshot
            .lock()
            .map(|s| s.clone())
            .map_err(|_| Error::new("unavailable"))
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
    fn update(&self, app: &tauri::AppHandle, id: &str, change: impl FnOnce(&mut Snapshot)) {
        let update = if let Ok(mut s) = self.snapshot.lock() {
            if s.task_id.as_deref() != Some(id) {
                return;
            }
            let phase = s.phase;
            change(&mut s);
            s.sequence += 1;
            if phase != s.phase {
                s.lifecycle += 1;
            }
            Some((s.clone(), phase != s.phase))
        } else {
            None
        };
        if let Some((s, phase_changed)) = update {
            let _ = app.emit_to("main", "speech:snapshot", &s);
            let _ = app.emit_to("voice-overlay", "speech:snapshot", &s);
            if phase_changed
                && let Some(runtime) = app.try_state::<crate::Runtime>()
                && let Ok(mut r) = runtime.0.lock()
            {
                if r.session.id != s.runtime_id {
                    return;
                }
                r.session.phase = s.phase;
                r.revision += 1;
                let copy = r.clone();
                drop(r);
                crate::publish(app, copy);
            }
        }
    }
    pub fn control(&self, id: &str, action: &str) -> Result<(), Error> {
        let active = self.active.lock().map_err(|_| Error::new("unavailable"))?;
        let (current, tx) = active.as_ref().ok_or(Error::new("staleSession"))?;
        if current != id {
            return Err(Error::new("staleSession"));
        }
        let value = match action {
            "finish" if self.snapshot()?.phase == Phase::Recording => 1,
            "cancel" => 2,
            _ => return Err(Error::new("invalid")),
        };
        if *tx.borrow() >= 2 || (value == 1 && *tx.borrow() != 0) {
            return Err(Error::new("busy"));
        }
        tx.send(value).map_err(|_| Error::new("staleSession"))
    }
    pub async fn start(
        &self,
        app: tauri::AppHandle,
        retry_id: Option<String>,
        expected_config: Option<(String, i64)>,
    ) -> Result<Snapshot, Error> {
        self.start_bound(app, retry_id, expected_config, None).await
    }
    async fn start_bound(
        &self,
        app: tauri::AppHandle,
        retry_id: Option<String>,
        expected_config: Option<(String, i64)>,
        expected_task: Option<String>,
    ) -> Result<Snapshot, Error> {
        let _gate = self.gate.try_lock().map_err(|_| Error::new("busy"))?;
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
        self.initialize().await?;
        let cfg = app
            .state::<ModelService>()
            .speech_config()
            .await
            .map_err(|e| Error::new(&e.code))?;
        if expected_config
            .as_ref()
            .is_some_and(|(id, revision)| id != &cfg.id || *revision != cfg.revision)
        {
            return Err(Error::new("conflict"));
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
        let task_id = uuid::Uuid::new_v4().to_string();
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
            config_revision: cfg.revision,
            id: attempt_id.clone(),
            record_id: record_id.clone(),
            kind: if retry_id.is_some() { "retry" } else { "live" }.into(),
            config_id: cfg.id.clone(),
            config_name: cfg.input.name.clone(),
            model_id: cfg.input.model_id.clone(),
            service_host: reqwest::Url::parse(&cfg.input.base_url)
                .ok()
                .and_then(|u| u.host_str().map(String::from))
                .unwrap_or_default(),
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
        let runtime_id = {
            let runtime = app.state::<crate::Runtime>();
            let mut r = runtime.0.lock().map_err(|_| Error::new("unavailable"))?;
            if matches!(
                r.session.phase,
                Phase::Connecting | Phase::Recording | Phase::Processing
            ) {
                return Err(Error::new("busy"));
            }
            r.session.id += 1;
            r.session.phase = Phase::Connecting;
            r.revision += 1;
            let copy = r.clone();
            drop(r);
            let id = copy.session.id;
            crate::publish(&app, copy);
            id
        };
        if persistent
            && let Err(e) = self
                .history
                .db(if retry_id.is_some() {
                    DbOperation::Retry(attempt)
                } else {
                    DbOperation::Prepare(Prepare {
                        record_id: record_id.clone(),
                        attempt,
                        audio: audio.clone(),
                    })
                })
                .await
        {
            if let Ok(mut r) = app.state::<crate::Runtime>().0.lock()
                && r.session.id == runtime_id
            {
                r.session.phase = Phase::Idle;
                r.revision += 1;
                let copy = r.clone();
                drop(r);
                crate::publish(&app, copy);
            }
            return Err(e);
        }
        let (tx, rx) = watch::channel(if self.stopping.load(Ordering::Acquire) {
            3u8
        } else {
            0u8
        });
        *self.active.lock().map_err(|_| Error::new("unavailable"))? = Some((task_id.clone(), tx));
        let old = self.snapshot()?;
        *self
            .snapshot
            .lock()
            .map_err(|_| Error::new("unavailable"))? = Snapshot {
            runtime_id,
            source_record_id: Some(record_id.clone()),
            task_id: Some(task_id.clone()),
            attempt_id: Some(attempt_id.clone()),
            record_id: if persistent {
                Some(record_id.clone())
            } else {
                None
            },
            sequence: old.sequence + 1,
            lifecycle: old.lifecycle + 1,
            model_name: Some(cfg.input.model_id.clone()),
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
        self.update(&app, &task_id, |s| s.phase = Phase::Connecting);
        let task = Task {
            id: task_id,
            runtime_id,
            record_id,
            attempt_id,
            persistent,
            retry: retry_id.is_some(),
            audio,
            preferences,
            replay,
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
            drop(_lease);
            if let Err(e) = result {
                service.update(&app, &id, |s| {
                    s.error = Some(e.code);
                    s.phase = Phase::Failed;
                });
            }
            if let Ok(mut a) = service.active.lock()
                && a.as_ref().is_some_and(|(current, _)| current == &id)
            {
                *a = None;
            }
            if service
                .snapshot()
                .is_ok_and(|s| matches!(s.phase, Phase::Completed | Phase::Cancelled))
            {
                tokio::time::sleep(Duration::from_millis(1000)).await;
                if crate::read(&app).is_ok_and(|r| r.session.id == task.runtime_id) {
                    service.update(&app, &id, |s| s.phase = Phase::Idle);
                }
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
        let mut duration = 0;
        let mut save_error = false;
        let mut abort = cancel.clone();
        let work = async {
            let mut session = recognition::RecognitionSession::open(cfg, &task.id).await?;
            if *cancel.borrow() >= 2 {
                return Err(Error::new("cancelled"));
            }
            if task.replay.is_none() {
                capture = Some(
                    audio::Capture::start(
                        task.preferences.device_id.clone(),
                        task.audio
                            .as_ref()
                            .map(|a| self.history.files.clone().map(|f| (f, a.file_key.clone())))
                            .transpose()?,
                    )
                    .await?,
                );
            }
            self.update(app, &task.id, |s| {
                s.phase = if task.retry {
                    Phase::Processing
                } else {
                    Phase::Recording
                }
            });
            if task.persistent
                && self
                    .history
                    .db(DbOperation::Stage {
                        attempt_id: task.attempt_id.clone(),
                        status: if task.retry {
                            "processing"
                        } else {
                            "recording"
                        }
                        .into(),
                    })
                    .await
                    .is_err()
            {
                save_error = true;
            }
            let mut replay_at = 0;
            let mut sending = true;
            let mut final_at = None;
            let mut tick = tokio::time::interval(Duration::from_millis(50));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut published = Instant::now();
            let mut checkpoint = Instant::now();
            let mut meter_seq = 0;
            let mut dirty = false;
            loop {
                if *cancel.borrow() >= 2 {
                    return Err(Error::new("cancelled"));
                }
                if final_at.is_some_and(|at: Instant| at.elapsed() > Duration::from_secs(30)) {
                    return Err(Error::new("timeout"));
                }
                tokio::select! {
                                   _=cancel.changed()=>{if *cancel.borrow()==1 {if let Some(c)=&capture{c.stop();}self.update(app,&task.id,|s|s.phase=Phase::Processing);if task.persistent&&self.history.db(DbOperation::Stage{attempt_id:task.attempt_id.clone(),status:"processing".into()}).await.is_err(){save_error=true;}}},
                                   message=session.next_event()=>{match message? {
                                       recognition::Event::Update(value)=>{transcript=value;dirty=true;if !transcript.confirmed_text.is_empty()&&task.persistent&&checkpoint.elapsed()>Duration::from_secs(1){if self.history.db(DbOperation::Checkpoint{attempt_id:task.attempt_id.clone(),text:transcript.text(true)}).await.is_err(){save_error=true;}checkpoint=Instant::now();}},
                                       recognition::Event::Finished=>{if sending{return Err(Error::new("protocol"));}
                if transcript.text(true).trim().is_empty(){return Err(Error::new("emptyText"));}break;},
                                       recognition::Event::Heartbeat=>{}
                                   }},
                                   _=tick.tick()=>{
                                       if began.elapsed()>Duration::from_secs(650){return Err(Error::new("timeout"));}
                                       let mut level=0.0;
                                       if sending {
                                           if let Some(pcm)=&task.replay {
                                               let end=(replay_at+800).min(pcm.len());if replay_at<end {session.send_pcm(&pcm[replay_at..end]).await?;replay_at=end;duration=replay_at as u64/16;}else{sending=false;}
                                           } else if let Some(c)=capture.as_mut(){
                                               while let Ok(packet)=c.packets.try_recv(){duration=packet.duration_ms;level=packet.level;session.send_pcm(&packet.pcm).await?;}
                                               if c.packets.is_closed()&&c.packets.is_empty(){sending=false;}
                                           }
                                           if !sending {session.finish().await?;final_at=Some(Instant::now());self.update(app,&task.id,|s|s.phase=Phase::Processing);if task.persistent&&self.history.db(DbOperation::Stage{attempt_id:task.attempt_id.clone(),status:"processing".into()}).await.is_err(){save_error=true;}}
                                       }
                                       meter_seq+=1;let meter=Meter{task_id:task.id.clone(),sequence:meter_seq,level,duration_ms:duration};let _=app.emit_to("main","speech:meter",&meter);let _=app.emit_to("voice-overlay","speech:meter",&meter);
                                       if published.elapsed()>=Duration::from_millis(100)&&(dirty||sending){self.update(app,&task.id,|s|{s.text=transcript.text(false);s.segments=transcript.segments();s.duration_ms=duration;});published=Instant::now();dirty=false;}
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
                if let Some(a) = task.audio.as_ref().filter(|_| task.persistent)
                    && self
                        .history
                        .db(DbOperation::AudioState {
                            id: a.id.clone(),
                            status: if save_error { "corrupt" } else { "ready" }.into(),
                            bytes: result.bytes,
                        })
                        .await
                        .is_err()
                {
                    save_error = true;
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
            && self
                .history
                .db(DbOperation::AudioState {
                    id: audio.id.clone(),
                    status: "missing".into(),
                    bytes: 0,
                })
                .await
                .is_err()
        {
            save_error = true;
        }
        if *cancel.borrow() >= 2 {
            outcome = Err(Error::new("cancelled"));
        }
        let cancelled = outcome
            .as_ref()
            .err()
            .is_some_and(|e| e.code == "cancelled");
        if cancelled && !task.retry {
            save_error |= self.discard_new(&task.record_id, task.persistent).await;
            self.update(app, &task.id, |s| {
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
            } else if outcome.is_ok() {
                "success"
            } else {
                "failed"
            };
            if self
                .history
                .db(DbOperation::Finish {
                    attempt_id: task.attempt_id.clone(),
                    status: status.into(),
                    text: text.clone(),
                    error: outcome.as_ref().err().map(|e| e.code.clone()),
                    duration_ms: duration,
                    elapsed_ms: began.elapsed().as_millis() as u64,
                })
                .await
                .is_err()
            {
                save_error = true;
            }
        }
        self.update(app, &task.id, |s| {
            s.text = text;
            s.segments = transcript.segments();
            s.duration_ms = duration;
            s.save_error = save_error;
            s.saved = task.persistent && !save_error;
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
    pub fn forget_audio(&self, app: &tauri::AppHandle, record_id: &str) {
        if let Ok(mut last) = self.last_audio.lock()
            && last.as_ref().is_some_and(|(id, _)| id == record_id)
        {
            *last = None;
        }
        if let Ok(snapshot) = self.snapshot()
            && snapshot.record_id.as_deref() == Some(record_id)
            && let Some(task_id) = snapshot.task_id
        {
            self.update(app, &task_id, |s| {
                s.can_retry = false;
                s.saved = false;
                s.record_id = None;
            });
        }
    }
    pub fn start_error(&self, app: &tauri::AppHandle, error: &Error) {
        let Ok(_gate) = self.gate.try_lock() else {
            return;
        };
        if self.is_active() || self.is_stopping() || error.code == "busy" {
            return;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let runtime_id = if let Ok(mut r) = app.state::<crate::Runtime>().0.lock() {
            r.session.id += 1;
            r.session.id
        } else {
            return;
        };
        if let Ok(mut s) = self.snapshot.lock() {
            s.task_id = Some(id.clone());
            s.runtime_id = runtime_id;
            s.sequence += 1;
        }
        self.update(app, &id, |s| {
            s.error = Some(error.code.clone());
            s.phase = Phase::Failed;
        });
    }
    pub async fn retry_current(&self, app: tauri::AppHandle) -> Result<Snapshot, Error> {
        let current = self.snapshot()?;
        if !current.can_retry {
            return Err(Error::new("fileUnavailable"));
        }
        let id = current
            .source_record_id
            .ok_or(Error::new("fileUnavailable"))?;
        // Persistent replay and temporary audio must both refer to the displayed record.
        // start_bound verifies the task under the admission gate before reading its resource.
        self.start_bound(app, Some(id), None, current.task_id).await
    }
    pub fn is_stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }
    pub fn shutdown(&self) {
        let _guard = self.operation_gate.lock().ok();
        self.stopping.store(true, Ordering::Release);
        if let Ok(a) = self.active.lock()
            && let Some((_, tx)) = a.as_ref()
        {
            let _ = tx.send(3);
        }
    }
    pub async fn drain(&self) {
        while self.is_active() || self.operations.load(Ordering::Acquire) > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let _ = self.playback.control("stop", None).await;
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
