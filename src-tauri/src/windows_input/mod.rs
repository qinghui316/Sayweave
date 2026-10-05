//! Native target evidence is never serialized to the renderer.
use crate::history::types::Error;
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
    mpsc,
};
use tokio::sync::oneshot;
#[cfg(feature = "desktop-acceptance")]
mod acceptance;
#[cfg(windows)]
mod native;
#[cfg(feature = "desktop-acceptance")]
pub fn native_checks(fixture: &std::path::Path) -> Result<(), String> {
    acceptance::run(fixture)
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
    Confirmed,
    Unavailable,
    Mismatch,
    Timeout,
}
impl Delivery {
    pub fn not_submitted(code: &str) -> Self {
        Self {
            state: "copy".into(),
            error: Some(code.into()),
            restore_error: false,
            elapsed_ms: 0,
            submission: Submission::NotSubmitted,
            verification: Verification::NotAttempted,
            stage: "capture".into(),
            submitted_events: 0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct Anchor {
    pub window: usize,
    pub focus: usize,
    pub process: u32,
    pub lifetime: u64,
    pub generation: u64,
    pub captured_tick: u32,
    pub activity: u64,
}
#[derive(Default)]
pub struct DeliveryGate(AtomicU8);
impl DeliveryGate {
    pub fn cancel(&self) -> bool {
        self.0
            .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub fn cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire) == 1
    }
    pub fn commit(&self) -> bool {
        self.0
            .compare_exchange(0, 2, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
    pub fn committed(&self) -> bool {
        self.0.load(Ordering::Acquire) == 2
    }
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
    Deliver(String, String, Arc<DeliveryGate>, oneshot::Sender<Delivery>),
    Copy(
        String,
        Option<Arc<DeliveryGate>>,
        oneshot::Sender<Result<(), Error>>,
    ),
    Restore(oneshot::Sender<Result<(), Error>>),
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
                        Request::Deliver(_, _, _, r) => {
                            let _ = r.send(Delivery::not_submitted("targetUnsupported"));
                        }
                        Request::Copy(_, _, r) => {
                            let _ = r.send(Err(Error::new("copyFailed")));
                        }
                        Request::Restore(r) => {
                            let _ = r.send(Ok(()));
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
    pub fn anchor() -> Option<Anchor> {
        #[cfg(windows)]
        {
            native::start_anchor().ok()
        }
        #[cfg(not(windows))]
        {
            None
        }
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
    pub async fn deliver(&self, id: String, text: String, cancel: Arc<DeliveryGate>) -> Delivery {
        let (tx, rx) = oneshot::channel();
        if self
            .send(Request::Deliver(id, text, cancel.clone(), tx))
            .is_err()
        {
            return Delivery::not_submitted("busy");
        }
        rx.await.unwrap_or_else(|_| {
            let mut result = Delivery::not_submitted("inputWorkerLost");
            result.stage = "worker".into();
            if cancel.committed() {
                result.submission = Submission::Uncertain;
                result.state = "unconfirmed".into();
            }
            result
        })
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
        let _ = self.send(Request::Forget(id));
    }
    pub async fn restore(&self) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        self.send(Request::Restore(tx))?;
        rx.await.map_err(|_| Error::new("clipboardRestore"))?
    }
    pub fn shutdown(&self) {
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
}
impl Default for WindowsInputService {
    fn default() -> Self {
        Self::new()
    }
}

pub(crate) fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}
pub(crate) fn expected_text(before: &str, start: usize, end: usize, text: &str) -> Option<String> {
    let mut units: Vec<u16> = before.encode_utf16().collect();
    if start > end || end > units.len() {
        return None;
    }
    let insert: Vec<u16> = text.encode_utf16().collect();
    units.splice(start..end, insert);
    String::from_utf16(&units).ok().map(|s| normalize(&s))
}
pub(crate) fn insertion_prefix(before: &str, start: usize, text: &str) -> Option<String> {
    let units: Vec<u16> = before.encode_utf16().collect();
    let mut prefix = units.get(..start)?.to_vec();
    prefix.extend(text.encode_utf16());
    String::from_utf16(&prefix).ok().map(|s| normalize(&s))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selection_semantics() {
        assert_eq!(expected_text("abc", 1, 2, "中文"), Some("a中文c".into()));
        assert_eq!(expected_text("a😀b", 3, 3, "\nZ"), Some("a😀\nZb".into()));
        assert_eq!(expected_text("a😀b", 2, 2, "Z"), None);
        assert_eq!(expected_text("a😀b", 1, 3, "好"), Some("a好b".into()));
        assert_eq!(
            expected_text("a\r\nb", 4, 4, "\r\nc"),
            Some("a\nb\nc".into())
        );
        assert_eq!(expected_text("abc", 0, 3, "abc"), Some("abc".into()));
    }
    #[test]
    fn readback_caret_uses_actual_newline_representation_and_utf16_boundaries() {
        assert_eq!(
            insertion_prefix("a😀b", 3, "中文，\n🙂"),
            Some("a😀中文，\n🙂".into())
        );
        assert_eq!(
            insertion_prefix("a😀中文，\r\n🙂b", 10, ""),
            Some("a😀中文，\n🙂".into())
        );
        assert_eq!(insertion_prefix("a😀b", 2, ""), None);
        assert_ne!(
            insertion_prefix("a😀中文，\r\n🙂b", 11, ""),
            Some("a😀中文，\n🙂".into())
        );
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
