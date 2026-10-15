//! Native target evidence is never serialized to the renderer.
use crate::history::types::Error;
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU64, AtomicUsize, Ordering},
    mpsc,
};
use tokio::sync::oneshot;
#[cfg(feature = "desktop-acceptance")]
mod acceptance;
#[cfg(windows)]
mod application;
#[cfg(windows)]
pub(crate) mod composition;
#[cfg(windows)]
mod native;
#[cfg(windows)]
mod runtime_module;
pub async fn application_context(
    anchor: Option<Anchor>,
) -> crate::contracts::language::ApplicationContext {
    #[cfg(windows)]
    if let Some(anchor) = anchor {
        return tokio::task::spawn_blocking(move || application::resolve(&anchor))
            .await
            .unwrap_or_default();
    }
    #[cfg(not(windows))]
    let _ = anchor;
    Default::default()
}
#[cfg(windows)]
pub(crate) use native::{composition_target, reanchor_turn};
#[cfg(feature = "desktop-acceptance")]
pub fn native_checks(fixture: &std::path::Path) -> Result<(), String> {
    acceptance::run(fixture)
}
#[cfg(feature = "desktop-acceptance")]
pub fn composition_checks(fixture: &std::path::Path) -> Result<(), String> {
    acceptance::composition_run(fixture)
}
#[cfg(feature = "desktop-acceptance")]
pub fn observation_checks(process: u32) -> Result<(), String> {
    acceptance::observation_run(process)
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub state: String,
    pub error: Option<String>,
    pub restore_error: bool,
    pub elapsed_ms: u64,
    pub submission: Submission,
    pub verification: Verification,
    pub stage: String,
    pub submitted_events: u32,
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Submission {
    NotSubmitted,
    Submitted,
    Uncertain,
}
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Verification {
    NotAttempted,
    Unavailable,
}
impl Delivery {
    pub fn not_submitted(code: &str) -> Self {
        Self {
            error: Some(code.into()),
            stage: "capture".into(),
            ..Self::result_only()
        }
    }
    pub(crate) fn result_only() -> Self {
        Self {
            state: "copy".into(),
            error: None,
            restore_error: false,
            elapsed_ms: 0,
            submission: Submission::NotSubmitted,
            verification: Verification::NotAttempted,
            stage: "result".into(),
            submitted_events: 0,
        }
    }
}
/// Only positive target-nature evidence selects recording without a write owner.
/// Probe failures still defer to TSF; component/permission failures are not absence.
pub(crate) fn result_only_target(result: &Result<(), Error>) -> bool {
    result
        .as_ref()
        .err()
        .is_some_and(|error| matches!(error.code.as_str(), "targetReadonly" | "targetNotEditable"))
}
/// Lack of UIA readback is not a rejection of TSF. Positive password/readonly/target-change
/// evidence, however, is shared by every input strategy and cannot be overridden by another probe.
pub(crate) fn draft_rejection(result: &Result<(), Error>) -> Option<String> {
    result
        .as_ref()
        .err()
        .filter(|e| {
            matches!(
                e.code.as_str(),
                "targetSafetyUnknown"
                    | "targetPassword"
                    | "targetReadonly"
                    | "targetPermission"
                    | "inputComposition"
                    | "targetChanged"
            )
        })
        .map(|e| e.code.clone())
}
#[derive(Clone, Debug)]
pub struct Anchor {
    pub capture_id: u64,
    pub window: usize,
    pub focus: usize,
    pub process: u32,
    pub lifetime: u64,
    pub generation: u64,
    pub captured_tick: u32,
    pub activity: u64,
}
#[derive(Default)]
pub struct DeliveryGate {
    state: AtomicU8,
    preparation_deadline: AtomicU64,
}
impl DeliveryGate {
    pub fn cancel(&self) -> bool {
        self.state
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub fn cancelled(&self) -> bool {
        self.state.load(Ordering::Acquire) == 1
    }
    pub fn commit(&self) -> bool {
        self.state
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub fn set_preparation_budget(&self, budget: std::time::Duration) {
        #[cfg(windows)]
        self.preparation_deadline.store(
            unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
                .saturating_add(budget.as_millis().min(15_000) as u64),
            Ordering::Release,
        );
        #[cfg(not(windows))]
        let _ = budget;
    }
    #[cfg(windows)]
    pub(crate) fn preparation_deadline(&self) -> u64 {
        let value = self.preparation_deadline.load(Ordering::Acquire);
        if value == 0 {
            unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
                .saturating_add(15_000)
        } else {
            value
        }
    }
}
pub(crate) fn observe_pointer(window: usize, event_tick: u32) {
    #[cfg(windows)]
    native::observe_pointer(window, event_tick);
    #[cfg(not(windows))]
    let _ = (window, event_tick);
}
pub(crate) fn observe(event: u32, window: usize, object: i32, event_tick: u32) {
    #[cfg(windows)]
    native::observe(event, window, object, event_tick);
}
fn event_after_capture(event_tick: u32, captured_tick: u32) -> bool {
    event_tick.wrapping_sub(captured_tick) <= i32::MAX as u32
}
enum Request {
    Capture(String, Anchor, oneshot::Sender<Result<(), Error>>),
    Copy(
        String,
        Option<Arc<DeliveryGate>>,
        oneshot::Sender<Result<(), Error>>,
    ),
    Forget(String),
    Stop,
}
struct Envelope {
    request: Request,
    _pending: Pending,
}
struct Pending(Arc<AtomicUsize>);
impl Drop for Pending {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
#[derive(Clone)]
pub struct WindowsInputService {
    sender: mpsc::SyncSender<Envelope>,
    owner: Arc<AtomicUsize>,
    stopping: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
    finished: Arc<AtomicBool>,
    pending: Arc<AtomicUsize>,
    admission: Arc<std::sync::Mutex<()>>,
}
impl WindowsInputService {
    pub fn new() -> Self {
        let (sender, receiver) = mpsc::sync_channel(64);
        let owner = Arc::new(AtomicUsize::new(0));
        let stopping = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicBool::new(false));
        let pending = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        #[cfg(windows)]
        {
            let (o, s, b, f) = (
                owner.clone(),
                stopping.clone(),
                busy.clone(),
                finished.clone(),
            );
            std::thread::Builder::new()
                .name("sayweave-input-mta".into())
                .spawn(move || {
                    native::worker(receiver, o, s, b);
                    f.store(true, Ordering::Release);
                })
                .expect("native input worker");
        }
        #[cfg(not(windows))]
        {
            let f = finished.clone();
            std::thread::spawn(move || {
                while let Ok(envelope) = receiver.recv() {
                    match envelope.request {
                        Request::Capture(_, _, r) => {
                            let _ = r.send(Err(Error::new("targetUnsupported")));
                        }
                        Request::Copy(_, _, r) => {
                            let _ = r.send(Err(Error::new("copyFailed")));
                        }
                        Request::Stop => break,
                        Request::Forget(_) => {}
                    }
                }
                f.store(true, Ordering::Release);
            });
        }
        Self {
            sender,
            owner,
            stopping,
            busy,
            pending,
            finished,
            admission: Arc::new(std::sync::Mutex::new(())),
        }
    }
    pub fn set_owner(&self, window: usize) {
        self.owner.store(window, Ordering::Release);
    }
    pub(crate) fn start_target() -> Result<Option<Anchor>, Error> {
        #[cfg(windows)]
        {
            native::start_anchor()
        }
        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }
    #[cfg(any(test, feature = "desktop-acceptance"))]
    pub fn anchor() -> Option<Anchor> {
        Self::start_target().ok().flatten()
    }
    pub(crate) fn bind_target(
        &self,
        id: &str,
        anchor: &Anchor,
        overlay: Option<crate::voice_window::TaskWindow>,
    ) {
        #[cfg(windows)]
        if let Err(error) = native::bind_watch(id, anchor, overlay) {
            log::info!(target: "sayweave", "operation=input_binding task_id={id} error={}", error.code);
        }
        #[cfg(not(windows))]
        let _ = (id, anchor, overlay);
    }
    fn send(&self, request: Request) -> Result<(), Error> {
        let _guard = self
            .admission
            .lock()
            .map_err(|_| Error::new("unavailable"))?;
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::new("stopping"));
        }
        self.pending.fetch_add(1, Ordering::AcqRel);
        self.sender
            .try_send(Envelope {
                request,
                _pending: Pending(self.pending.clone()),
            })
            .map_err(|_| Error::new("busy"))
    }
    pub async fn capture(&self, id: String, anchor: Anchor) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.send(Request::Capture(id, anchor, tx))?;
        rx.await.map_err(|_| Error::new("targetProbeFailed"))?
    }
    pub async fn copy(&self, text: String) -> Result<(), Error> {
        if text.len() > 1024 * 1024 {
            return Err(Error::new("invalid"));
        }
        let (tx, rx) = oneshot::channel();
        self.send(Request::Copy(text, None, tx))?;
        rx.await.map_err(|_| Error::new("copyFailed"))?
    }
    pub async fn copy_bound(&self, text: String, gate: Arc<DeliveryGate>) -> Result<(), Error> {
        if text.len() > 1024 * 1024 {
            return Err(Error::new("invalid"));
        }
        let (tx, rx) = oneshot::channel();
        self.send(Request::Copy(text, Some(gate), tx))?;
        rx.await.map_err(|_| Error::new("copyFailed"))?
    }
    pub fn forget(&self, id: String) {
        #[cfg(windows)]
        native::release_watch(&id);
        let _ = self.send(Request::Forget(id));
    }
    pub fn shutdown(&self) {
        // The application exit owner retires observation before calling shutdown.
        #[cfg(windows)]
        tauri::async_runtime::spawn(async {
            let _ = composition::stop_observer().await;
        });
        let _guard = self.admission.lock().ok();
        self.stopping.store(true, Ordering::Release);
        self.pending.fetch_add(1, Ordering::AcqRel);
        let _ = self.sender.try_send(Envelope {
            request: Request::Stop,
            _pending: Pending(self.pending.clone()),
        });
    }
    pub async fn drain(&self) {
        while !self.finished.load(Ordering::Acquire)
            || self.pending.load(Ordering::Acquire) > 0
            || self.busy.load(Ordering::Acquire)
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Acquire) || self.pending.load(Ordering::Acquire) > 0
    }
}
impl Default for WindowsInputService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_positive_target_nature_selects_copy_recording() {
        assert!(!result_only_target(&Ok(())));
        for code in ["targetReadonly", "targetNotEditable"] {
            assert!(result_only_target(&Err(Error::new(code))));
        }
        for code in [
            "targetProbeFailed",
            "targetEditabilityUnknown",
            "targetSafetyUnknown",
            "targetPassword",
            "targetPermission",
            "inputComposition",
            "targetChanged",
            "compositionRuntimeUnavailable",
            "componentVersionMismatch",
            "compositionCleanup",
        ] {
            assert!(!result_only_target(&Err(Error::new(code))), "{code}");
        }
    }
    #[test]
    fn deliberate_copy_is_not_an_input_failure_or_a_submission() {
        let result = Delivery::result_only();
        assert_eq!(result.state, "copy");
        assert_eq!(result.error, None);
        assert_eq!(result.submission, Submission::NotSubmitted);
        assert_eq!(result.verification, Verification::NotAttempted);
        assert_eq!(result.submitted_events, 0);
        let failure = Delivery::not_submitted("compositionRejected");
        assert_eq!(failure.error.as_deref(), Some("compositionRejected"));
        assert_eq!(failure.stage, "capture");
    }
    #[test]
    fn delayed_events_before_capture_do_not_invalidate_a_new_target() {
        assert!(!event_after_capture(99, 100));
        assert!(event_after_capture(100, 100));
        assert!(event_after_capture(101, 100));
        assert!(event_after_capture(3, u32::MAX - 2));
        assert!(!event_after_capture(u32::MAX - 2, 3));
    }
    #[test]
    fn cancellation_has_a_single_commit_boundary() {
        for _ in 0..500 {
            let gate = Arc::new(DeliveryGate::default());
            let copy = gate.clone();
            let cancellation = std::thread::spawn(move || copy.cancel());
            let committed = gate.commit();
            assert_ne!(cancellation.join().unwrap(), committed);
            assert!(!gate.commit());
        }
    }
    #[test]
    fn admitted_queue_is_counted_until_drop() {
        let count = Arc::new(AtomicUsize::new(2));
        let first = Pending(count.clone());
        let second = Pending(count.clone());
        drop(first);
        assert_eq!(count.load(Ordering::Acquire), 1);
        drop(second);
        assert_eq!(count.load(Ordering::Acquire), 0);
    }
}
