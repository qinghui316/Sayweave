use crate::{
    coordinator::Phase,
    history::types::{DbOperation, Error},
    shortcuts::{ShortcutService, TaskStamp, types::Mode},
    speech::{NativeStart, Snapshot as SpeechSnapshot, Source as SpeechSource, SpeechService},
    windows_input::{Anchor, Delivery, DeliveryGate, WindowsInputService},
};
use serde::Serialize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tauri::{Emitter, Manager};
use tokio::sync::{oneshot, watch};
#[cfg(windows)]
pub(crate) mod drafts;
mod presentation;
#[cfg(windows)]
pub(crate) mod processing_context;
#[cfg(windows)]
use crate::windows_input::composition::{Draft, Finish};
pub use presentation::Presentation;
use presentation::{final_presentation, speech_presentation};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InputRoute {
    #[default]
    None,
    Live,
    ResultOnly,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum InputMode {
    #[default]
    Revisable,
    Caret,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Admission {
    #[default]
    Pending,
    Ready,
    Rejected,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub input_route: InputRoute,
    pub input_mode: InputMode,
    pub admission: Admission,
    pub sequence: u64,
    pub task_id: Option<String>,
    pub source: String,
    pub stage: String,
    pub text: String,
    pub delivery: Option<Delivery>,
    pub error: Option<String>,
    pub presentation_id: Option<String>,
    pub presentation: Presentation,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            input_route: InputRoute::None,
            input_mode: InputMode::Revisable,
            admission: Admission::Pending,
            sequence: 1,
            task_id: None,
            source: "trial".into(),
            stage: "idle".into(),
            text: String::new(),
            delivery: None,
            error: None,
            presentation_id: None,
            presentation: Presentation::Idle,
        }
    }
}
#[derive(Clone)]
struct Active {
    id: String,
    source: String,
    epoch: u64,
    origin: Option<TaskStamp>,
    cancel: watch::Sender<u8>,
    explicit_end: Arc<AtomicBool>,
    gate: Arc<DeliveryGate>,
    overlay_revision: u64,
    began: std::time::Instant,
    continuous: Option<crate::speech::continuous::Control>,
    #[cfg(windows)]
    draft: Option<Arc<drafts::SessionDraft>>,
}
struct Reservation {
    id: String,
    expected_task: Option<String>,
    cancel: watch::Sender<u8>,
    gate: Arc<DeliveryGate>,
}
pub struct DictationCoordinator {
    active: Mutex<Option<Active>>,
    snapshot: Mutex<Snapshot>,
    stopping: AtomicBool,
    copy_gate: Mutex<Option<Arc<DeliveryGate>>>,
    component_maintenance: Arc<AtomicBool>,
}
pub struct ComponentMaintenance(Arc<AtomicBool>);
impl Drop for ComponentMaintenance {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
#[cfg(test)]
mod maintenance_tests {
    use super::*;
    #[test]
    fn component_maintenance_has_one_guard_and_releases_admission_on_drop() {
        let coordinator = DictationCoordinator::default();
        assert!(!coordinator.busy());
        let guard = coordinator.component_maintenance().unwrap();
        assert!(coordinator.busy());
        assert_eq!(
            coordinator.component_maintenance().err().unwrap().code,
            "busy"
        );
        drop(guard);
        assert!(!coordinator.busy());
        coordinator.stopping.store(true, Ordering::Release);
        assert_eq!(
            coordinator.component_maintenance().err().unwrap().code,
            "stopping"
        );
    }
}
impl Default for DictationCoordinator {
    fn default() -> Self {
        Self {
            active: Mutex::new(None),
            snapshot: Mutex::new(Snapshot::default()),
            stopping: AtomicBool::new(false),
            copy_gate: Mutex::new(None),
            component_maintenance: Arc::new(AtomicBool::new(false)),
        }
    }
}
impl DictationCoordinator {
    pub fn observe_transcript(&self, transcript: &crate::speech::NativeTranscript) {
        #[cfg(windows)]
        if let Ok(active) = self.active.lock()
            && let Some(session) = active.as_ref().filter(|s| {
                s.id == transcript.task_id && s.source == "external" && !s.gate.cancelled()
            })
            && let Some(draft) = &session.draft
        {
            draft.update(transcript);
        }
        #[cfg(not(windows))]
        let _ = transcript;
    }
    pub async fn retry_displayed(
        &self,
        app: tauri::AppHandle,
        record: String,
        task: String,
        source: SpeechSource,
    ) -> Result<SpeechSnapshot, Error> {
        let source = source.retry_source().as_str();
        let mut reserved = self.reserve(&app, source, 0, None, None)?;
        reserved.expected_task = Some(task);
        self.start_reserved(app, source, Ok(None), Some(record), None, reserved)
            .await
    }
    pub fn busy(&self) -> bool {
        self.component_maintenance.load(Ordering::Acquire)
            || self.active.lock().map(|s| s.is_some()).unwrap_or(true)
    }
    pub fn component_maintenance(&self) -> Result<ComponentMaintenance, Error> {
        let active = self.active.lock().map_err(|_| Error::new("unavailable"))?;
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::new("stopping"));
        }
        if active.is_some() || self.component_maintenance.swap(true, Ordering::AcqRel) {
            return Err(Error::new("busy"));
        }
        Ok(ComponentMaintenance(self.component_maintenance.clone()))
    }
    pub fn configure_capture<T>(
        &self,
        begin: bool,
        operation: impl FnOnce() -> Result<T, Error>,
    ) -> Result<T, Error> {
        let active = self.active.lock().map_err(|_| Error::new("unavailable"))?;
        if begin && active.is_some() {
            return Err(Error::new("busy"));
        }
        operation()
    }
    pub fn snapshot(&self) -> Result<Snapshot, Error> {
        self.snapshot
            .lock()
            .map(|s| s.clone())
            .map_err(|_| Error::new("unavailable"))
    }
    fn update(&self, app: &tauri::AppHandle, id: &str, change: impl FnOnce(&mut Snapshot)) {
        if let Ok(mut s) = self.snapshot.lock()
            && s.task_id.as_deref() == Some(id)
        {
            let old_stage = s.stage.clone();
            let old_presentation = s.presentation;
            let old_recovery = s.delivery.as_ref().is_some_and(|d| d.restore_error);
            change(&mut s);
            let layout = old_stage != s.stage
                || old_presentation != s.presentation
                || old_recovery != s.delivery.as_ref().is_some_and(|d| d.restore_error);
            s.sequence += 1;
            let copy = s.clone();
            drop(s);
            if layout {
                crate::refresh_voice(app);
            }
            let _ = app.emit_to("main", "dictation:snapshot", &copy);
            let _ = app.emit_to("voice-overlay", "dictation:snapshot", &copy);
        }
    }
    pub fn observe_speech(&self, app: &tauri::AppHandle, speech: &SpeechSnapshot) {
        let Some(id) = speech.task_id.as_deref() else {
            return;
        };
        let Ok(current) = self.snapshot() else {
            return;
        };
        if current.task_id.as_deref() != Some(id)
            || current.source != "external"
            || !matches!(current.stage.as_str(), "preparing" | "recognizing")
        {
            return;
        }
        let Ok(active) = self.active.lock() else {
            return;
        };
        let Some(task) = active.as_ref().filter(|a| a.id == id) else {
            return;
        };
        let Some(next) = speech_presentation(speech.phase, *task.cancel.borrow()) else {
            return;
        };
        #[cfg(windows)]
        let next = if next == Presentation::Recording {
            task.draft.as_ref().map_or(next, |d| d.presentation())
        } else {
            next
        };
        drop(active);
        if next != current.presentation {
            self.update(app, id, |s| {
                if matches!(s.stage.as_str(), "preparing" | "recognizing") {
                    s.presentation = next;
                }
            });
        }
    }
    pub fn key(
        &self,
        app: &tauri::AppHandle,
        epoch: u64,
        pressed: bool,
        mode: Mode,
        anchor: Result<Option<Anchor>, Error>,
        task: TaskStamp,
    ) {
        let active = {
            let Ok(active) = self.active.lock() else {
                return;
            };
            if !key_matches_task(
                active.as_ref().map(|a| a.id.as_str()),
                active.as_ref().and_then(|a| a.origin.as_ref()),
                &task,
                app.state::<ShortcutService>().task_is_current(&task),
            ) {
                return;
            }
            active
                .as_ref()
                .map(|a| (a.id.clone(), a.epoch, a.source.clone()))
        };
        if active.is_some() {
            if let Some((id, current, source)) = active
                && source == "external"
                && (mode == Mode::Toggle && pressed
                    || mode == Mode::Hold && !pressed && epoch == current)
            {
                let phase = app
                    .state::<SpeechService>()
                    .snapshot()
                    .ok()
                    .filter(|s| s.task_id.as_deref() == Some(&id))
                    .map(|s| s.phase);
                if phase == Some(Phase::Recording) {
                    self.finish_bound(app, &id).ok();
                } else if matches!(phase, Some(Phase::Connecting))
                    || self.snapshot().is_ok_and(|s| s.stage == "preparing")
                {
                    self.cancel_bound(Some(&id)).ok();
                }
            }
            return;
        }
        if !pressed {
            return;
        }
        let Ok(reserved) = self.reserve(
            app,
            "external",
            epoch,
            Some(&task),
            anchor.as_ref().ok().and_then(Option::as_ref),
        ) else {
            return;
        };
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = app
                .state::<Self>()
                .start_reserved(app.clone(), "external", anchor, None, None, reserved)
                .await;
        });
    }
    pub async fn start(
        &self,
        app: tauri::AppHandle,
        source: &str,
        epoch: u64,
        anchor: Option<Anchor>,
        retry: Option<String>,
        expected: Option<(String, i64)>,
    ) -> Result<SpeechSnapshot, Error> {
        self.start_target(app, source, epoch, Ok(anchor), retry, expected)
            .await
    }
    async fn start_target(
        &self,
        app: tauri::AppHandle,
        source: &str,
        epoch: u64,
        anchor: Result<Option<Anchor>, Error>,
        retry: Option<String>,
        expected: Option<(String, i64)>,
    ) -> Result<SpeechSnapshot, Error> {
        let reserved = self.reserve(
            &app,
            source,
            epoch,
            None,
            anchor.as_ref().ok().and_then(Option::as_ref),
        )?;
        self.start_reserved(app, source, anchor, retry, expected, reserved)
            .await
    }
    pub fn pause_shortcuts(&self, service: &ShortcutService) -> Result<bool, Error> {
        let active = self.active.lock().map_err(|_| Error::new("unavailable"))?;
        if service.is_capturing() {
            return Err(Error::new("busy"));
        }
        service.block_triggers(true);
        Ok(active.is_some())
    }
    fn reserve(
        &self,
        app: &tauri::AppHandle,
        source: &str,
        epoch: u64,
        event: Option<&TaskStamp>,
        anchor: Option<&Anchor>,
    ) -> Result<Reservation, Error> {
        let _admission = app
            .state::<crate::lifecycle::Lifecycle>()
            .enter()
            .map_err(Error::new)?;
        if let Some(updates) = app.try_state::<crate::update::UpdateService>() {
            updates.foreground(app);
        }
        SpeechSource::try_from(source)?;
        // Application admission includes the DEV demonstration session; the
        // speech service itself owns no rendering Runtime or window state.
        if crate::read(app).is_ok_and(|r| r.demo_busy()) {
            return Err(Error::new("busy"));
        }
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, _) = watch::channel(0u8);
        let gate = Arc::new(DeliveryGate::default());
        {
            let mut a = self.active.lock().map_err(|_| Error::new("unavailable"))?;
            if self.stopping.load(Ordering::Acquire) {
                return Err(Error::new("stopping"));
            }
            if a.is_some()
                || self.component_maintenance.load(Ordering::Acquire)
                || app.state::<ShortcutService>().triggers_paused()
            {
                return Err(Error::new("busy"));
            }
            if event.is_some_and(|event| !app.state::<ShortcutService>().task_is_current(event)) {
                return Err(Error::new("staleSession"));
            }
            self.invalidate_copy()?;
            *a = Some(Active {
                id: id.clone(),
                source: source.into(),
                epoch,
                origin: event.cloned(),
                cancel: tx.clone(),
                explicit_end: Arc::new(AtomicBool::new(false)),
                gate: gate.clone(),
                overlay_revision: crate::OVERLAY_VISIBILITY.load(Ordering::Acquire),
                began: std::time::Instant::now(),
                continuous: None,
                #[cfg(windows)]
                draft: None,
            });
        }
        app.state::<ShortcutService>().active_task(Some(id.clone()));
        app.state::<crate::language_learning::LanguageLearning>()
            .interrupt(app);
        let sequence = self.snapshot()?.sequence + 1;
        *self
            .snapshot
            .lock()
            .map_err(|_| Error::new("unavailable"))? = Snapshot {
            task_id: Some(id.clone()),
            source: source.into(),
            sequence,
            stage: "preparing".into(),
            presentation_id: Some(uuid::Uuid::new_v4().to_string()),
            presentation: Presentation::Preparing,
            ..Snapshot::default()
        };
        if source == "external" {
            let overlay = crate::voice_window::bind(&id);
            if let Some(anchor) = anchor {
                app.state::<WindowsInputService>()
                    .bind_target(&id, anchor, overlay);
            }
        }
        self.update(app, &id, |_| {});
        if source == "external" {
            // Keydown owns the view before model, credential or provider preparation begins.
            let _ = crate::window_action_internal(app, "overlay", "show");
            log::info!(target:"sayweave","operation=dictation_phase task_id={} phase=preparing elapsed_ms=0",id);
        }
        Ok(Reservation {
            id,
            expected_task: None,
            cancel: tx,
            gate,
        })
    }
    async fn start_reserved(
        &self,
        app: tauri::AppHandle,
        source: &str,
        anchor: Result<Option<Anchor>, Error>,
        retry: Option<String>,
        expected: Option<(String, i64)>,
        reserved: Reservation,
    ) -> Result<SpeechSnapshot, Error> {
        let speech_source = SpeechSource::try_from(source)?;
        let Reservation {
            id,
            expected_task,
            cancel: tx,
            gate,
        } = reserved;
        app.state::<ShortcutService>().active_task(Some(id.clone()));
        self.update(&app, &id, |_| {});
        let input = app.state::<WindowsInputService>();
        let mut anchor = match anchor {
            Ok(value) => value,
            Err(error) => {
                self.reject_preparation(&app, &id, speech_source, &error, &gate)
                    .await;
                return Err(error);
            }
        };
        let preparation_deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        gate.set_preparation_budget(std::time::Duration::from_secs(15));
        let optional = async {
            let dictionary = processing_context::dictionary(&app).await.unwrap_or_else(|error| {
            log::info!(target:"sayweave", "operation=dictionary_prepare task_id={} result=unavailable error={}", id,error.code);
            Arc::new(crate::processing::prompt::DictionaryIndex::new(vec![]))
        });
            let application = crate::windows_input::application_context(anchor.clone()).await;
            let language = if source == "external" {
                crate::language_learning::snapshot(
                    &app.state::<crate::data::DataWorker>(),
                    application,
                )
                .await
                .ok()
            } else {
                None
            };
            (dictionary, language)
        };
        let (dictionary, language) = match crate::task_stop::optional_read(
            optional,
            tx.subscribe(),
            std::time::Duration::from_secs(2),
        )
        .await
        {
            crate::task_stop::OptionalRead::Ready(value) => value,
            crate::task_stop::OptionalRead::Unavailable => {
                log::info!(target:"sayweave","operation=language_prepare task_id={} result=unavailable",id);
                (
                    Arc::new(crate::processing::prompt::DictionaryIndex::new(vec![])),
                    None,
                )
            }
            crate::task_stop::OptionalRead::Stopped => {
                let error = Error::new("cancelled");
                self.reject_preparation(&app, &id, speech_source, &error, &gate)
                    .await;
                return Err(error);
            }
        };
        let hotwords = dictionary.hotwords();
        #[cfg(windows)]
        let processing = if source == "external" {
            match processing_context::Context::prepare(
                &app,
                tx.subscribe(),
                &id,
                dictionary,
                language.clone(),
                preparation_deadline,
            )
            .await
            {
                Ok(value) => value,
                Err(error) if error.code == "cancelled" => {
                    let error = Error::new("cancelled");
                    self.reject_preparation(&app, &id, speech_source, &error, &gate)
                        .await;
                    return Err(error);
                }
                Err(error) => {
                    log::info!(target:"sayweave","operation=processing_prepare task_id={} result=original error={}",id,error.code);
                    None
                }
            }
        } else {
            None
        };
        self.update(&app, &id, |s| {
            s.input_route = if source != "external" {
                InputRoute::None
            } else if anchor.is_some() {
                InputRoute::Live
            } else {
                InputRoute::ResultOnly
            };
        });
        // Assess shared password/read-only/identity evidence before acquiring our own range.
        // UIA readback limitations do not select another writing strategy.
        if source == "external"
            && let Some(target) = anchor.clone()
        {
            let assessment = input.capture(id.clone(), target).await;
            if crate::windows_input::result_only_target(&assessment) {
                // No native range was acquired. Forget the provisional target
                // before audio starts; this task cannot follow later focus.
                input.forget(id.clone());
                anchor = None;
            } else if let Some(code) = crate::windows_input::draft_rejection(&assessment) {
                let error = Error::new(&code);
                self.reject_preparation(&app, &id, speech_source, &error, &gate)
                    .await;
                return Err(error);
            }
        }
        #[cfg(windows)]
        let raw_draft = if source == "external"
            && let Some(target) = anchor.clone()
        {
            match Draft::begin(id.clone(), 1, target, gate.clone()).await {
                Ok(draft) => Some(draft),
                Err(error) if error.code == "targetReadonlyContext" => {
                    input.forget(id.clone());
                    anchor = None;
                    None
                }
                Err(error) => {
                    self.reject_preparation(&app, &id, speech_source, &error, &gate)
                        .await;
                    return Err(error);
                }
            }
        } else {
            None
        };
        #[cfg(windows)]
        if let (Some(native), Some(language)) = (
            &raw_draft,
            language.as_ref().filter(|s| {
                crate::contracts::language::LearningEligibility::capture(&s.settings).any()
            }),
        ) {
            app.state::<crate::language_learning::LanguageLearning>()
                .observation_control()
                .publish(&language.settings);
            let handle = app.clone();
            let _ = native
                .observe(Arc::new(
                    crate::windows_input::composition::ObservationSink {
                        snapshot: language.clone(),
                        control: app
                            .state::<crate::language_learning::LanguageLearning>()
                            .observation_control(),
                        emit: Arc::new(move |e| {
                            handle
                                .state::<crate::language_learning::LanguageLearning>()
                                .edit(e)
                        }),
                    },
                ))
                .await;
        }
        let continuous = (source == "external" && anchor.is_some()).then(|| {
            let flow =
                crate::speech::continuous::Control::new(crate::input_state::activity_evidence());
            flow.resume(1, anchor.as_ref().unwrap().activity);
            flow.defer_cancel_archive();
            flow
        });
        if let Ok(mut active) = self.active.lock()
            && let Some(active) = active.as_mut().filter(|s| s.id == id)
        {
            active.continuous = continuous.clone();
        }
        let session_anchor = anchor.clone();
        let session_cancel = tx.clone();
        self.update(&app, &id, |s| {
            s.input_route = if source != "external" {
                InputRoute::None
            } else if anchor.is_some() {
                InputRoute::Live
            } else {
                InputRoute::ResultOnly
            }
        });
        // Live input requires acknowledged Begin; result-only has no write owner.
        self.update(&app, &id, |s| s.admission = Admission::Ready);
        let (terminal_tx, terminal_rx) = oneshot::channel();
        let result = app
            .state::<SpeechService>()
            .start_native(
                app.clone(),
                retry,
                expected,
                NativeStart {
                    id: id.clone(),
                    source: speech_source,
                    expected_task,
                    cancel: tx,
                    terminal: terminal_tx,
                    continuous: continuous.clone(),
                    hotwords,
                },
            )
            .await;
        if result.is_err() {
            #[cfg(windows)]
            if let Some(draft) = raw_draft {
                let _ = draft.finish(String::new(), Finish::Abort).await;
            }
            input.forget(id.clone());
            self.update(&app, &id, |s| {
                s.stage = "idle".into();
                s.error = result.as_ref().err().map(|e| e.code.clone());
                s.presentation = Presentation::Recognizing;
            });
            if !gate.cancelled()
                && let Err(error) = &result
                && error.code != "cancelled"
                && error.code != "staleSession"
            {
                app.state::<SpeechService>()
                    .start_error(&id, speech_source, error);
            }
            self.release(&app, &id).await;
            return result;
        }
        #[cfg(windows)]
        let shared_record = app.state::<SpeechService>().task_record(&id);
        self.update(&app, &id, |s| s.stage = "recognizing".into());
        let source = source.to_string();
        tauri::async_runtime::spawn(async move {
            let coordinator = app.state::<Self>();
            let journal_queue = crate::archive::Archive::new(
                app.state::<crate::data::DataWorker>().inner().clone(),
                id.clone(),
            )
            .ok();
            #[cfg(windows)]
            if raw_draft.is_some()
                && let Some(s) = app
                    .state::<SpeechService>()
                    .snapshot()
                    .ok()
                    .filter(|s| s.task_id.as_deref() == Some(&id))
                && s.record_id.is_some()
                && let Some(attempt) = s.attempt_id
            {
                for state in ["pending", "delivering"] {
                    if let Some(queue) = &journal_queue {
                        queue.enqueue(
                            state.into(),
                            crate::data::Operation::History(DbOperation::Delivery {
                                task: id.clone(),
                                attempt: attempt.clone(),
                                state: state.into(),
                                error: None,
                            }),
                        );
                    }
                }
            }
            #[cfg(windows)]
            let mut processing = processing;
            #[cfg(windows)]
            let record = if raw_draft.is_some() {
                let current = app
                    .state::<SpeechService>()
                    .snapshot()
                    .ok()
                    .filter(|s| s.task_id.as_deref() == Some(&id));
                if !gate.cancelled()
                    && let (Some(context), Some(current)) = (&processing, &current)
                    && context
                        .attach(current.record_id.as_deref(), current.attempt_id.as_deref())
                        .await
                        .is_err()
                {
                    processing = None;
                }
                shared_record
            } else {
                None
            };
            #[cfg(windows)]
            let draft = raw_draft.map(|native| {
                drafts::SessionDraft::new(
                    id.clone(),
                    session_anchor.clone().unwrap(),
                    native,
                    continuous.clone().unwrap(),
                    session_cancel.clone(),
                    processing,
                    record,
                )
            });
            #[cfg(windows)]
            if let Some(draft) = &draft {
                if !gate.cancelled() {
                    draft.activate();
                    if let Ok(mut active) = coordinator.active.lock()
                        && let Some(session) = active.as_mut().filter(|s| s.id == id)
                    {
                        session.draft = Some(draft.clone());
                    }
                    let latest = app
                        .state::<SpeechService>()
                        .subscribe_transcripts()
                        .borrow()
                        .clone();
                    coordinator.observe_transcript(&latest);
                }
                // Every admitted range has a cleanup owner, including the
                // cancelled/failed-journal path which may never write text.
                let manager = draft.clone();
                let projection = draft.clone();
                let handle = app.clone();
                let task_id = id.clone();
                tauri::async_runtime::spawn(async move {
                    manager
                        .maintain(move |presentation| {
                            handle
                                .state::<DictationCoordinator>()
                                .update(&handle, &task_id, |s| {
                                    s.presentation = presentation;
                                    s.input_mode = projection.input_mode();
                                    if projection.result_only() {
                                        s.input_route = InputRoute::ResultOnly;
                                    }
                                });
                        })
                        .await;
                });
            }
            let mut recognition_archive = None;
            let mut recognized_snapshot = None;
            let mut delivery_records = Vec::new();
            if let Ok(terminal) = terminal_rx.await {
                recognized_snapshot = Some(terminal.snapshot.clone());
                coordinator.update(&app, &id, |s| {
                    s.text = terminal.text.clone();
                    s.stage = "result".into();
                    s.error = s.error.take().or_else(|| terminal.error.clone());
                    s.presentation = Presentation::Recognizing;
                });
                if source == "external" && !terminal.text.is_empty() {
                    #[cfg(windows)]
                    let inline = match &draft {
                        Some(draft) => {
                            let mode = composition_finish(terminal.phase, true, gate.cancelled());
                            let latest = terminal.transcript.clone();
                            Some(draft.finish(&latest, mode).await)
                        }
                        None => None,
                    };
                    #[cfg(not(windows))]
                    let inline: Option<Delivery> = None;
                    let delivery = inline.unwrap_or_else(|| {
                        if gate.cancelled() {
                            Delivery::not_submitted("cancelled")
                        } else if session_anchor.is_none() {
                            Delivery::result_only()
                        } else {
                            Delivery::not_submitted("targetUnavailable")
                        }
                    });
                    if terminal.persistent {
                        if session_anchor.is_none() {
                            delivery_records.push((
                                terminal.attempt_id.clone(),
                                "pending".to_string(),
                                None,
                            ));
                        }
                        delivery_records.push((
                            terminal.attempt_id.clone(),
                            delivery.state.clone(),
                            delivery.error.clone(),
                        ));
                    }
                    coordinator.update(&app, &id, |s| {
                        log::info!(target:"sayweave", "operation=dictation_delivery task_id={} state={} elapsed_ms={} error={}", id, delivery.state, delivery.elapsed_ms, delivery.error.as_deref().unwrap_or("none"));
                        s.delivery = Some(delivery);
                        s.stage = "result".into();
                        s.presentation = Presentation::Delivering;
                    });
                }
                coordinator.update(&app, &id, |s| {
                    s.presentation = Presentation::Recognizing;
                });
                #[cfg(windows)]
                if terminal.text.is_empty()
                    && let Some(draft) = &draft
                {
                    let mode = if terminal.phase == Phase::Cancelled {
                        Finish::Abort
                    } else {
                        Finish::Handoff(
                            terminal.error.clone().unwrap_or_else(|| "emptyText".into()),
                        )
                    };
                    let latest = terminal.transcript.clone();
                    let delivery = draft.finish(&latest, mode).await;
                    let partial = terminal.transcript.clone();
                    if terminal.record_id.is_some() {
                        delivery_records.push((
                            terminal.attempt_id.clone(),
                            delivery.state.clone(),
                            delivery.error.clone(),
                        ));
                    }
                    coordinator.update(&app, &id, |s| {
                        if delivery.submission != crate::windows_input::Submission::NotSubmitted
                            && partial.task_id == id
                        {
                            s.text = partial.text;
                        }
                        s.delivery = Some(delivery);
                    });
                }
                recognition_archive = Some(terminal.archived);
            } else {
                #[cfg(windows)]
                let abandoned_delivery = if let Some(draft) = &draft {
                    let latest = app
                        .state::<SpeechService>()
                        .subscribe_transcripts()
                        .borrow()
                        .clone();
                    Some(
                        draft
                            .finish(&latest, Finish::Handoff("interrupted".into()))
                            .await,
                    )
                } else {
                    None
                };
                let partial = app
                    .state::<SpeechService>()
                    .snapshot()
                    .ok()
                    .filter(|s| s.task_id.as_deref() == Some(&id))
                    .map(|s| s.text)
                    .unwrap_or_default();
                coordinator.update(&app, &id, |s| {
                    s.error = Some("interrupted".into());
                    s.stage = "result".into();
                    s.text = partial;
                    if source == "external" && !s.text.is_empty() {
                        s.delivery = Some(Delivery::not_submitted("recognitionFailed"));
                    }
                    #[cfg(windows)]
                    if let Some(delivery) = abandoned_delivery {
                        s.delivery = Some(delivery);
                    }
                    s.presentation = Presentation::Recognizing;
                });
            }
            let mut automatic_result_copy = false;
            #[cfg(windows)]
            if let Some(report) = draft.as_ref().and_then(|d| d.report()) {
                automatic_result_copy = report.result_only
                    && !report.cancelled
                    && !report.result_raw.trim().is_empty()
                    && !gate.cancelled()
                    && coordinator.active.lock().is_ok_and(|a| {
                        a.as_ref()
                            .is_some_and(|a| a.id == id && a.explicit_end.load(Ordering::Acquire))
                    })
                    && recognized_snapshot
                        .as_ref()
                        .is_some_and(|s| s.phase == Phase::Completed);
                coordinator.update(&app, &id, |s| {
                    if report.result_only && !report.cancelled {
                        s.text = report.result_raw;
                        s.input_route = InputRoute::ResultOnly;
                    } else if !report.text.is_empty() || report.cancelled {
                        s.text = report.text;
                    }
                });
            }
            // InputSettled: release admission and presentation without awaiting
            // ordinary persistence. The finalizer retains record/range reports.
            #[cfg(windows)]
            let input_recovery = draft
                .as_ref()
                .and_then(|d| d.report())
                .is_some_and(|r| r.rollback_failed);
            #[cfg(not(windows))]
            let input_recovery = false;
            let mut archive_error = false;
            #[cfg(windows)]
            let cancel_prune = if let (Some(draft), Some(current)) = (&draft, &recognized_snapshot)
                && let Some(report) = draft.report()
                && report.cancelled
                && current.record_id.is_some()
                && let Some(attempt) = &current.attempt_id
            {
                // Durable intent first; ordinary SQL below remains ordered after
                // recognition and processing queues. No external input is replayed.
                archive_error |= draft.stage_cancel().await.is_err();
                let text = report.cancellation_raw().to_owned();
                let operation = DbOperation::PruneCancelled {
                    attempt_id: attempt.clone(),
                    text,
                };
                archive_error |= app
                    .state::<crate::data::DataWorker>()
                    .stage_cancel(&crate::data::Operation::History(operation.clone()))
                    .await
                    .is_err();
                Some(operation)
            } else {
                None
            };
            let copy_attempt = recognized_snapshot
                .as_ref()
                .and_then(|s| s.attempt_id.clone());
            let (copy_settled, copy_outcome) = oneshot::channel::<Option<DbOperation>>();
            let archive_app = app.clone();
            let archive_id = id.clone();
            let language_source = recognized_snapshot.clone();
            crate::archive::spawn(async move {
                let app = archive_app;
                let id = archive_id;
                let coordinator = app.state::<DictationCoordinator>();
                let mut saved = false;
                archive_error |= match journal_queue {
                    Some(queue) => !queue.finish().await,
                    None => true,
                };
                if let Some(archive) = recognition_archive {
                    match archive.await {
                        Ok(result) => {
                            saved = result.saved;
                            archive_error |= result.save_error;
                        }
                        Err(_) => archive_error = true,
                    }
                }
                for (attempt, state, error) in delivery_records {
                    if coordinator
                        .journal(&app, &id, attempt.as_deref(), &state, error)
                        .await
                        .is_err()
                    {
                        archive_error = true;
                    }
                }
                match copy_outcome.await {
                    Ok(Some(operation)) => {
                        archive_error |= app
                            .state::<SpeechService>()
                            .history
                            .db(operation)
                            .await
                            .is_err()
                    }
                    Ok(None) => (),
                    Err(_) => archive_error = true,
                }
                #[cfg(windows)]
                if let Some(draft) = &draft {
                    let final_report = draft.report();
                    match draft.archive().await {
                        Ok(Some((_report, _persisted))) => {
                            let _ = app.emit_to("main", "processing:archived", &id);
                        }
                        Err(error) => {
                            log::info!(target:"sayweave","operation=processing_archive task_id={} result=failed error={}",id,error.code);
                            archive_error = true;
                        }
                        _ => {}
                    }
                    let current = recognized_snapshot;
                    if final_report.as_ref().is_some_and(|r| r.cancelled) {
                        app.state::<SpeechService>().invalidate_task_audio(&id);
                    }
                    if let (Some(report), Some(current)) = (&final_report, &current)
                        && report.cancelled
                        && let (Some(record), Some(_attempt)) =
                            (&current.record_id, &current.attempt_id)
                    {
                        let service = app.state::<SpeechService>();
                        if service.history.discard_audio(record).await.is_err() {
                            archive_error = true;
                            log::info!(target:"sayweave","operation=cancel_audio task_id={} result=pending",id);
                        }
                        let recovery = report.units.iter().any(|u| u.application == "uncertain")
                            || input_recovery;
                        if let Some(operation) = cancel_prune {
                            archive_error |= service.history.db(operation).await.is_err();
                        }
                        draft.release_record();
                        if report.raw.is_empty() && !recovery {
                            match service.history.detail(record).await {
                                Ok(row) => {
                                    archive_error |=
                                        service.history.remove(record, row.revision).await.is_err()
                                }
                                Err(_) => archive_error = true,
                            }
                            saved = false;
                            service.forget_audio(record);
                        }
                        let _ = app.emit_to("main", "processing:archived", &id);
                    } else {
                        draft.release_record();
                    }
                }
                if saved
                    && !archive_error
                    && let (Some(language), Some(recognition)) = (&language, &language_source)
                    && language.settings.personalization.enabled
                    && recognition.phase == Phase::Completed
                    && let Some(attempt) = &recognition.attempt_id
                {
                    app.state::<crate::language_learning::LanguageLearning>()
                        .speech(crate::contracts::language::StyleObservation {
                            id: format!("speech:{id}"),
                            task: id.clone(),
                            application: language.application.clone(),
                            settings_revision: language.settings.personalization.revision,
                            language: crate::contracts::language::WritingLanguage::of(
                                &recognition.text,
                            ),
                            kind: "speech".into(),
                            summary: String::new(),
                            summary_complete: false,
                            source_ref: Some(attempt.clone()),
                        });
                }
                app.state::<SpeechService>()
                    .archive_settled(crate::speech::ArchiveSettlement {
                        task_id: id,
                        saved: saved && !archive_error,
                        save_error: archive_error,
                        attempt_id: language_source.as_ref().and_then(|s| s.attempt_id.clone()),
                        revision: 0,
                        state: String::new(),
                    });
                !archive_error
            });
            app.state::<WindowsInputService>().forget(id.clone());
            coordinator.release(&app, &id).await;
            let mut result_copy = None;
            if automatic_result_copy
                && let Ok(snapshot) = coordinator.snapshot()
                && snapshot.task_id.as_deref() == Some(&id)
                && let Some(presentation) = snapshot.presentation_id
                && let Ok((text, copy_gate)) = coordinator.begin_copy(&id, &presentation)
            {
                // Admission is released; this task/presentation gate still
                // lets dismissal or a newer task cancel the clipboard effect.
                let result = app
                    .state::<WindowsInputService>()
                    .copy_bound(text, copy_gate)
                    .await;
                result_copy = copy_receipt(&id, copy_attempt.as_deref(), &result);
                if result
                    .as_ref()
                    .is_err_and(|e| e.code != "cancelled" && e.code != "staleSession")
                {
                    coordinator.update(&app, &id, |s| {
                        if s.presentation_id.as_deref() == Some(&presentation) {
                            s.error.get_or_insert_with(|| "copyFailed".into());
                        }
                    });
                }
            }
            let _ = copy_settled.send(result_copy);
        });
        result
    }
    async fn reject_preparation(
        &self,
        app: &tauri::AppHandle,
        id: &str,
        source: SpeechSource,
        error: &Error,
        gate: &DeliveryGate,
    ) {
        if gate.cancelled() || matches!(error.code.as_str(), "cancelled" | "staleSession") {
            self.update(app, id, |s| {
                s.stage = "idle".into();
                s.error = Some("cancelled".into());
            });
            app.state::<WindowsInputService>().forget(id.into());
            self.release(app, id).await;
            return;
        }
        log::info!(target:"sayweave", "operation=dictation_admission task_id={id} result=rejected error={}", error.code);
        self.update(app, id, |s| {
            s.admission = Admission::Rejected;
            s.stage = "result".into();
            s.error = Some(error.code.clone());
        });
        app.state::<WindowsInputService>().forget(id.into());
        app.state::<SpeechService>().start_error(id, source, error);
        self.release(app, id).await;
    }
    async fn journal(
        &self,
        app: &tauri::AppHandle,
        id: &str,
        attempt: Option<&str>,
        state: &str,
        error: Option<String>,
    ) -> Result<(), Error> {
        let attempt = attempt.ok_or_else(|| Error::new("saveFailed"))?;
        app.state::<SpeechService>()
            .history
            .db(DbOperation::Delivery {
                task: id.into(),
                attempt: attempt.into(),
                state: state.into(),
                error,
            })
            .await
            .map(|_| ())
    }
    async fn release(&self, app: &tauri::AppHandle, id: &str) {
        log::info!(target:"sayweave", "operation=dictation_cleanup task_id={id} step=begin");
        let old = {
            let Ok(a) = self.active.lock() else {
                return;
            };
            if !a.as_ref().is_some_and(|a| a.id == id) {
                return;
            }
            a.clone()
        };
        app.state::<ShortcutService>().settle(app).await;
        log::info!(target:"sayweave", "operation=dictation_cleanup task_id={id} step=shortcuts_settled");
        crate::voice_window::release(id);
        {
            let Ok(mut a) = self.active.lock() else {
                return;
            };
            if !a.as_ref().is_some_and(|a| a.id == id) {
                return;
            }
            app.state::<ShortcutService>().active_task(None);
            *a = None;
            log::info!(target:"sayweave", "operation=dictation_cleanup task_id={id} step=admission_released");
        }
        // Projection can schedule window work. Never retain admission across it;
        // update checks the task identity again if a new task won the boundary.
        self.update(app, id, |s| {
            let explicit_end = old
                .as_ref()
                .is_some_and(|a| a.explicit_end.load(Ordering::Acquire));
            if s.admission == Admission::Rejected && !explicit_end {
                s.presentation = Presentation::Error;
                return;
            }
            s.presentation = final_presentation(
                s.input_route == InputRoute::ResultOnly,
                &s.text,
                s.delivery.as_ref(),
                s.error.as_deref(),
                explicit_end,
            );
        });
        if let Some(old) = old
            && old.source == "external"
        {
            log::info!(target:"sayweave","operation=dictation_phase task_id={} phase=released elapsed_ms={}",id,old.began.elapsed().as_millis());
            if let Ok(s) = self.snapshot()
                && let Some(ms) = s.presentation.hide_after()
                && let Some(presentation) = s.presentation_id
            {
                self.schedule_hide(app, id, presentation, old.overlay_revision, ms);
            }
        }
    }
    fn schedule_hide(
        &self,
        app: &tauri::AppHandle,
        id: &str,
        presentation: String,
        visibility: u64,
        ms: u64,
    ) {
        let app = app.clone();
        let id = id.to_string();
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
            if visibility == crate::OVERLAY_VISIBILITY.load(Ordering::Acquire)
                && !app.state::<Self>().busy()
            {
                let _ = app.state::<Self>().dismiss(&app, &id, &presentation);
            }
        });
    }
    pub fn dismiss(
        &self,
        app: &tauri::AppHandle,
        id: &str,
        presentation: &str,
    ) -> Result<(), Error> {
        // Hold admission until the bound view has been hidden, so it cannot hide a new task.
        let active = self.active.lock().map_err(|_| Error::new("unavailable"))?;
        let current = self.snapshot()?;
        if current.task_id.as_deref() != Some(id)
            || current.presentation_id.as_deref() != Some(presentation)
        {
            return Err(Error::new("staleSession"));
        }
        self.invalidate_copy()?;
        crate::window_action(app, "overlay", "hide").map_err(|_| Error::new("unavailable"))?;
        if active.is_none() {
            self.update(app, id, |s| {
                s.presentation = Presentation::Idle;
                s.presentation_id = None;
            });
        }
        log::info!(target:"sayweave","operation=dictation_phase task_id={} phase=hidden",id);
        Ok(())
    }
    fn invalidate_copy(&self) -> Result<(), Error> {
        if let Some(gate) = self
            .copy_gate
            .lock()
            .map_err(|_| Error::new("unavailable"))?
            .take()
        {
            gate.cancel();
        }
        Ok(())
    }
    fn begin_copy(
        &self,
        task: &str,
        presentation: &str,
    ) -> Result<(String, Arc<DeliveryGate>), Error> {
        let active = self.active.lock().map_err(|_| Error::new("unavailable"))?;
        let snapshot = self.snapshot()?;
        if active.is_some()
            || snapshot.task_id.as_deref() != Some(task)
            || snapshot.presentation_id.as_deref() != Some(presentation)
            || snapshot.presentation != Presentation::Result
        {
            return Err(Error::new("staleSession"));
        }
        if snapshot.text.is_empty() {
            return Err(Error::new("invalid"));
        }
        self.invalidate_copy()?;
        let gate = Arc::new(DeliveryGate::default());
        *self
            .copy_gate
            .lock()
            .map_err(|_| Error::new("unavailable"))? = Some(gate.clone());
        Ok((snapshot.text, gate))
    }
    pub fn finish_bound(&self, app: &tauri::AppHandle, id: &str) -> Result<(), Error> {
        let active = self.active.lock().map_err(|_| Error::new("unavailable"))?;
        let a = active
            .as_ref()
            .filter(|a| a.id == id)
            .ok_or_else(|| Error::new("staleSession"))?;
        // Task identity is the authority. Editing/resuming can temporarily
        // change the speech phase while this same gesture is still active.
        a.explicit_end.store(true, Ordering::Release);
        if let Some(flow) = &a.continuous {
            flow.request_finish();
        }
        // A closed receiver means this active task is already settling. Finish
        // is idempotent; it must not create a new error during that boundary.
        let _ = crate::task_stop::request(&a.cancel, crate::contracts::speech::StopReason::Finish);
        let elapsed = a.began.elapsed().as_millis();
        drop(active);
        self.update(app, id, |s| {
            s.presentation = s.presentation.finish_requested()
        });
        log::info!(target:"sayweave","operation=dictation_phase task_id={} phase=finish_requested elapsed_ms={}",id,elapsed);
        Ok(())
    }
    pub fn cancel_bound(&self, id: Option<&str>) -> Result<(), Error> {
        let active = self.active.lock().map_err(|_| Error::new("unavailable"))?;
        let a = active
            .as_ref()
            .filter(|a| Some(a.id.as_str()) == id)
            .ok_or_else(|| Error::new("staleSession"))?;
        if !a.gate.cancel() {
            return Err(Error::new("inputSubmitted"));
        }
        a.explicit_end.store(true, Ordering::Release);
        if let Some(flow) = &a.continuous {
            flow.request_finish();
        }
        let _ = crate::task_stop::request(&a.cancel, crate::contracts::speech::StopReason::Cancel);
        Ok(())
    }
    pub fn interrupt_bound(&self, id: Option<&str>) {
        if let Ok(a) = self.active.lock()
            && let Some(a) = a.as_ref().filter(|a| Some(a.id.as_str()) == id)
        {
            a.gate.cancel();
            if let Some(flow) = &a.continuous {
                flow.request_finish();
            }
            let _ = crate::task_stop::request(
                &a.cancel,
                crate::contracts::speech::StopReason::Interrupted,
            );
        }
    }
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        let _ = self.invalidate_copy();
        if let Ok(a) = self.active.lock()
            && let Some(a) = a.as_ref()
        {
            a.gate.cancel();
            if let Some(flow) = &a.continuous {
                flow.request_finish();
            }
            let _ = crate::task_stop::request(
                &a.cancel,
                crate::contracts::speech::StopReason::Interrupted,
            );
        }
    }
    pub async fn drain(&self) {
        while self.busy() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}
#[cfg(windows)]
fn composition_finish(phase: Phase, journaled: bool, interrupted: bool) -> Finish {
    if phase == Phase::Cancelled {
        Finish::Abort
    } else if !journaled {
        Finish::Handoff("saveFailed".into())
    } else if phase != Phase::Completed {
        Finish::Handoff("recognitionFailed".into())
    } else if interrupted {
        Finish::Handoff("interrupted".into())
    } else {
        Finish::Commit
    }
}
// Each clipboard effect has its own delivery identity and shares the immutable
// recognition attempt. It must never overwrite the original input receipt.
fn copy_receipt(
    task: &str,
    attempt: Option<&str>,
    result: &Result<(), Error>,
) -> Option<DbOperation> {
    let operation = format!("copy:{}", uuid::Uuid::new_v4());
    log::info!(target:"sayweave", "operation=dictation_result_copy task_id={task} delivery_id={operation} result={} error={}", if result.is_ok(){"copied"}else{"failed"}, result.as_ref().err().map_or("none",|e|e.code.as_str()));
    attempt.map(|attempt| DbOperation::Delivery {
        task: operation,
        attempt: attempt.into(),
        state: if result.is_ok() {
            "confirmed"
        } else {
            "failed"
        }
        .into(),
        error: result.as_ref().err().map(|e| e.code.clone()),
    })
}
fn key_matches_task(
    id: Option<&str>,
    origin: Option<&TaskStamp>,
    event: &TaskStamp,
    current: bool,
) -> bool {
    current && id == event.id.as_deref()
        || id.is_some() && event.id.is_none() && origin == Some(event)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn copied_and_failed_clipboard_receipts_survive_reopen_without_replacing_native_input() {
        use crate::{contracts::history::DbOutput, data::repository::Repository};
        let root = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(root.path()).unwrap();
        repo.connection
            .execute(
                "INSERT INTO speech_records(id,created_at,status) VALUES('r',1,'success')",
                [],
            )
            .unwrap();
        repo.connection.execute("INSERT INTO speech_attempts(id,record_id,kind,config_id,config_name,model_id,service_host,config_revision,created_at,status) VALUES('a','r','live','c','test','model','host',1,1,'success')",[]).unwrap();
        repo.history_operation(DbOperation::Delivery {
            task: "native-task".into(),
            attempt: "a".into(),
            state: "unconfirmed".into(),
            error: Some("compositionRejected".into()),
        })
        .unwrap();
        repo.history_operation(copy_receipt("native-task", Some("a"), &Ok(())).unwrap())
            .unwrap();
        repo.history_operation(
            copy_receipt("native-task", Some("a"), &Err(Error::new("copyFailed"))).unwrap(),
        )
        .unwrap();
        drop(repo);
        let mut repo = Repository::open(root.path()).unwrap();
        repo.history_operation(DbOperation::RecoverDelivery)
            .unwrap();
        let copies: Vec<(String,String,Option<String>)> = repo.connection.prepare("SELECT attempt_id,state,error_code FROM delivery_attempts WHERE task_id LIKE 'copy:%' ORDER BY state").unwrap().query_map([],|row| Ok((row.get(0)?,row.get(1)?,row.get(2)?))).unwrap().collect::<Result<_,_>>().unwrap();
        assert_eq!(
            copies,
            vec![
                ("a".into(), "confirmed".into(), None),
                ("a".into(), "failed".into(), Some("copyFailed".into()))
            ]
        );
        let DbOutput::Deliveries(rows) = repo
            .history_operation(DbOperation::Deliveries("r".into()))
            .unwrap()
        else {
            panic!("input projection")
        };
        assert_eq!(
            rows.len(),
            1,
            "clipboard confirmation cannot hide native input uncertainty"
        );
        assert_eq!(rows[0].state, "unconfirmed");
        assert_eq!(rows[0].error.as_deref(), Some("compositionRejected"));
        assert!(copy_receipt("unsaved-task", None, &Ok(())).is_none());
    }
    #[cfg(windows)]
    #[test]
    fn required_journal_and_recognition_guard_commit_independently_of_archive() {
        assert_eq!(
            composition_finish(Phase::Completed, false, false),
            Finish::Handoff("saveFailed".into())
        );
        assert_eq!(
            composition_finish(Phase::Failed, true, false),
            Finish::Handoff("recognitionFailed".into())
        );
        assert_eq!(
            composition_finish(Phase::Completed, true, true),
            Finish::Handoff("interrupted".into())
        );
        assert_eq!(
            composition_finish(Phase::Cancelled, false, true),
            Finish::Abort
        );
        assert_eq!(
            composition_finish(Phase::Completed, true, false),
            Finish::Commit
        );
    }
    #[test]
    fn queued_copy_is_invalidated_by_dismiss_or_new_task_before_commit() {
        let coordinator = DictationCoordinator::default();
        *coordinator.snapshot.lock().unwrap() = Snapshot {
            task_id: Some("A".into()),
            presentation_id: Some("view-A".into()),
            presentation: Presentation::Result,
            text: "文字".into(),
            ..Snapshot::default()
        };
        let (_, gate) = coordinator.begin_copy("A", "view-A").unwrap();
        coordinator.invalidate_copy().unwrap();
        assert!(!gate.commit());
        assert!(coordinator.begin_copy("A", "old-view").is_err());
        let (_, old) = coordinator.begin_copy("A", "view-A").unwrap();
        let (_, latest) = coordinator.begin_copy("A", "view-A").unwrap();
        assert!(!old.commit());
        assert!(latest.commit());
    }
    #[test]
    fn queued_release_and_second_toggle_can_only_cancel_the_task_they_started() {
        let initial = TaskStamp {
            id: None,
            sequence: 0,
        };
        // Both presses/releases can be enqueued before the first press creates A.
        assert!(key_matches_task(Some("A"), Some(&initial), &initial, false));
        assert!(!key_matches_task(None, None, &initial, false));
        let next_idle = TaskStamp {
            id: None,
            sequence: 2,
        };
        assert!(!key_matches_task(
            Some("B"),
            Some(&next_idle),
            &initial,
            false
        ));
        let a = TaskStamp {
            id: Some("A".into()),
            sequence: 1,
        };
        assert!(!key_matches_task(Some("B"), Some(&next_idle), &a, false));
        assert!(key_matches_task(None, None, &next_idle, true));
    }
}
#[tauri::command]
pub fn dictation_snapshot(
    window: tauri::WebviewWindow,
    service: tauri::State<'_, DictationCoordinator>,
) -> Result<Snapshot, Error> {
    caller(window.label())?;
    service.snapshot()
}
fn caller(label: &str) -> Result<(), Error> {
    if ["main", "voice-overlay"].contains(&label) {
        Ok(())
    } else {
        Err(Error::new("forbidden"))
    }
}
#[tauri::command]
pub async fn dictation_start(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    service: tauri::State<'_, DictationCoordinator>,
) -> Result<SpeechSnapshot, Error> {
    caller(window.label())?;
    service
        .start_target(
            app.clone(),
            "external",
            0,
            WindowsInputService::start_target(),
            None,
            None,
        )
        .await
}
#[tauri::command]
pub fn dictation_control(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    service: tauri::State<'_, DictationCoordinator>,
    action: String,
    task_id: String,
) -> Result<(), Error> {
    caller(window.label())?;
    match action.as_str() {
        "finish" => service.finish_bound(&app, &task_id),
        "cancel" => service.cancel_bound(Some(&task_id)),
        _ => Err(Error::new("invalid")),
    }
}
#[tauri::command]
pub async fn dictation_copy(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    service: tauri::State<'_, DictationCoordinator>,
    task_id: String,
    presentation_id: String,
) -> Result<(), Error> {
    caller(window.label())?;
    let (text, gate) = service.begin_copy(&task_id, &presentation_id)?;
    let attempt = app
        .state::<SpeechService>()
        .snapshot()
        .ok()
        .filter(|s| s.task_id.as_deref() == Some(&task_id))
        .and_then(|s| s.attempt_id);
    let result = app
        .state::<WindowsInputService>()
        .copy_bound(text, gate)
        .await;
    if let Some(receipt) = copy_receipt(&task_id, attempt.as_deref(), &result)
        && let Err(error) = app.state::<SpeechService>().history.db(receipt).await
    {
        log::error!(target:"sayweave", "operation=dictation_copy_archive task_id={task_id} error={}",error.code);
    }
    result?;
    service.update(&app, &task_id, |s| {
        if s.presentation_id.as_deref() == Some(&presentation_id)
            && s.error.as_deref() == Some("copyFailed")
        {
            s.error = None;
        }
    });
    let visibility = crate::OVERLAY_VISIBILITY.load(Ordering::Acquire);
    service.schedule_hide(&app, &task_id, presentation_id, visibility, 600);
    Ok(())
}
#[tauri::command]
pub fn dictation_dismiss(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    service: tauri::State<'_, DictationCoordinator>,
    task_id: String,
    presentation_id: String,
) -> Result<(), Error> {
    caller(window.label())?;
    service.dismiss(&app, &task_id, &presentation_id)
}

#[tauri::command]
pub fn dictation_settings(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    service: tauri::State<'_, DictationCoordinator>,
    task_id: String,
    presentation_id: String,
) -> Result<(), Error> {
    caller(window.label())?;
    let current = service.snapshot()?;
    if current.task_id.as_deref() != Some(&task_id)
        || current.presentation_id.as_deref() != Some(&presentation_id)
        || current.admission != Admission::Rejected
    {
        return Err(Error::new("staleSession"));
    }
    crate::window_action(&app, "main", "show").map_err(|_| Error::new("unavailable"))?;
    app.emit_to("main", "dictation:settings", ())
        .map_err(|_| Error::new("unavailable"))?;
    service.dismiss(&app, &task_id, &presentation_id)
}
