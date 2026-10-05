//! Session-owned host. TSF ranges stay in the target's apartment, never in this worker.
use super::{Anchor, Delivery, DeliveryGate, Submission, Verification};
use crate::history::types::Error;
use sayweave_text_service::{
    client::ProfileLease,
    registration,
    transport::Pipe,
    wire::{Action, Mutation, Reply, Request},
};
use std::{
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use windows::Win32::{
    Foundation::*,
    System::{Com::*, SystemInformation::GetTickCount64},
    UI::WindowsAndMessaging::*,
};

#[derive(Default)]
struct Desired {
    revision: u64,
    text: String,
    closed: bool,
    notified: bool,
    frozen: bool,
    failure: Option<String>,
}
enum Command {
    Wake,
    Finish(String, Finish, oneshot::Sender<Delivery>),
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finish {
    Commit,
    Handoff(String),
    Abort,
}
pub struct Draft {
    turn: u64,
    sender: mpsc::SyncSender<Command>,
    desired: Arc<Mutex<Desired>>,
}
impl Draft {
    pub async fn begin(
        task: String,
        turn: u64,
        anchor: Anchor,
        gate: Arc<DeliveryGate>,
    ) -> Result<Arc<Self>, Error> {
        let (sender, commands) = mpsc::sync_channel(4);
        let desired = Arc::new(Mutex::new(Desired::default()));
        let (ready, result) = oneshot::channel();
        let shared = desired.clone();
        std::thread::Builder::new()
            .name("sayweave-composition-sta".into())
            .spawn(move || {
                let began = Instant::now();
                let diagnostic_task = task.clone();
                let mut host = match Host::open(task, turn, anchor, gate) {
                    Ok(host) => host,
                    Err(error) => {
                        log::info!(target:"sayweave", "operation=draft_admission task_id={} turn={} result=rejected error={} elapsed_ms={}", diagnostic_task,turn,error.code,began.elapsed().as_millis());
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                log::info!(target:"sayweave", "operation=draft_admission task_id={} turn={} result=ready elapsed_ms={}", diagnostic_task,turn,began.elapsed().as_millis());
                if ready.send(Ok(())).is_err() {
                    return;
                }
                let mut applied = 0;
                let mut last_update: Option<Instant> = None;
                loop {
                    pump();
                    match commands.recv_timeout(Duration::from_millis(5)) {
                        Ok(Command::Finish(text, mode, reply)) => {
                            let mut delivery = host.finish(&text, mode);
                            if host.close().is_err() {
                                delivery.error = Some("compositionCleanup".into());
                            }
                            let _ = reply.send(delivery);
                            break;
                        }
                        Ok(Command::Wake) => {
                            if let Ok(mut desired) = shared.lock() {
                                desired.notified = false;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    let latest = shared.lock().ok().and_then(|d| {
                        (!d.closed && !d.frozen && d.revision > applied)
                            .then(|| (d.revision, d.text.clone()))
                    });
                    if let Some((revision, text)) = latest
                        && last_update.is_none_or(|at| at.elapsed() >= Duration::from_millis(50))
                    {
                        if host.failure.is_none() {
                            let _ = host.update(&text);
                        }
                        if let Ok(mut desired) = shared.lock() {
                            desired.failure = host.failure.clone();
                        }
                        applied = revision;
                        last_update = Some(Instant::now());
                    }
                }
            })
            .map_err(|_| Error::new("compositionWorker"))?;
        result
            .await
            .map_err(|_| Error::new("compositionWorker"))??;
        Ok(Arc::new(Self {
            turn,
            sender,
            desired,
        }))
    }
    /// One replaceable snapshot; a blocked target cannot create an unbounded edit queue.
    pub fn update(&self, turn: u64, sequence: u64, text: &str) {
        let Ok(mut desired) = self.desired.lock() else {
            return;
        };
        if turn != self.turn || desired.closed || desired.frozen || sequence <= desired.revision {
            return;
        }
        desired.revision = sequence;
        desired.text = text.into();
        if !desired.notified {
            desired.notified = true;
            if self.sender.try_send(Command::Wake).is_err() {
                desired.notified = false;
            }
        }
    }
    pub fn freeze(&self) {
        if let Ok(mut desired) = self.desired.lock() {
            desired.frozen = true;
        }
    }
    pub fn failure(&self) -> Option<String> {
        self.desired.lock().ok().and_then(|d| d.failure.clone())
    }
    pub async fn finish(&self, text: String, mode: Finish) -> Delivery {
        if let Ok(mut desired) = self.desired.lock() {
            if desired.closed {
                return Delivery::not_submitted("staleSession");
            }
            desired.closed = true;
        }
        let (reply, result) = oneshot::channel();
        if self
            .sender
            .try_send(Command::Finish(text, mode, reply))
            .is_err()
        {
            let mut outcome = Delivery::not_submitted("compositionWorker");
            outcome.submission = Submission::Uncertain;
            outcome.state = "unconfirmed".into();
            return outcome;
        }
        result.await.unwrap_or_else(|_| {
            let mut outcome = Delivery::not_submitted("compositionWorker");
            outcome.submission = Submission::Uncertain;
            outcome.state = "unconfirmed".into();
            outcome
        })
    }
}
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
struct Host {
    task: String,
    turn: u64,
    anchor: Anchor,
    gate: Arc<DeliveryGate>,
    pipe: Pipe,
    capability: String,
    operation: u64,
    revision: u64,
    text: String,
    attempted: bool,
    visible: bool,
    failure: Option<String>,
    profile: ProfileLease,
    _lease: registration::HostLease,
    _apartment: Apartment,
}
impl Host {
    fn open(
        task: String,
        turn: u64,
        anchor: Anchor,
        gate: Arc<DeliveryGate>,
    ) -> Result<Self, Error> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|_| Error::new("compositionApartment"))?;
        }
        let apartment = Apartment;
        let capability = uuid::Uuid::new_v4().to_string();
        let lease = registration::host_capability(std::process::id(), &capability)
            .map_err(|_| Error::new("compositionBusy"))?;
        crate::text_service::require_ready()?;
        let profile = ProfileLease::open().map_err(|error| {
            log::info!(target:"sayweave", "operation=draft_activation task_id={} turn={} hresult={:08x}", task, turn, error.code().0 as u32);
            Error::new("compositionActivationFailed")
        })?;
        if gate.cancelled() {
            return Err(Error::new("cancelled"));
        }
        let mut pid = 0;
        let thread =
            unsafe { GetWindowThreadProcessId(HWND(anchor.window as *mut _), Some(&mut pid)) };
        if pid != anchor.process {
            return Err(Error::new("targetChanged"));
        }
        let started = Instant::now();
        let pipe = loop {
            target_ready(&task, &anchor, &gate)?;
            if let Ok(pipe) = Pipe::client(pid, thread) {
                break pipe;
            }
            if started.elapsed() > Duration::from_millis(500) {
                return Err(Error::new("compositionUnavailable"));
            }
            pump();
            std::thread::sleep(Duration::from_millis(5));
        };
        let mut host = Self {
            task,
            turn,
            anchor,
            gate,
            pipe,
            capability,
            operation: 0,
            revision: 0,
            text: String::new(),
            attempted: false,
            visible: false,
            failure: None,
            profile,
            _lease: lease,
            _apartment: apartment,
        };
        super::native::composition_begin_target(&host.task, &host.anchor)?;
        host.exchange(Action::Begin, "", false)?;
        // Begin itself does not change text. Do not publish a writable draft if input moved
        // while the target's asynchronous edit session was being scheduled/executed.
        super::native::composition_begin_target(&host.task, &host.anchor)?;
        Ok(host)
    }
    fn exchange(&mut self, action: Action, text: &str, cleanup: bool) -> Result<(), Error> {
        if !cleanup {
            target_ready(&self.task, &self.anchor, &self.gate)?;
        }
        if action == Action::Begin {
            super::native::composition_begin_target(&self.task, &self.anchor)?;
        }
        self.operation += 1;
        if action == Action::Update {
            self.revision += 1;
        }
        let request = Request {
            protocol: 1,
            capability: self.capability.clone(),
            task: self.task.clone(),
            turn: self.turn,
            operation: self.operation,
            revision: self.revision,
            deadline_tick: unsafe { GetTickCount64() } + 2000,
            action,
            text: text.into(),
        };
        if !request.valid() {
            return Err(Error::new("compositionTextLimit"));
        }
        if action == Action::Update {
            self.attempted = true;
        }
        self.pipe
            .send(&serde_json::to_vec(&request).map_err(|_| Error::new("compositionProtocol"))?)
            .map_err(|_| Error::new("compositionDisconnected"))?;
        let at = Instant::now();
        loop {
            pump();
            if let Some(frame) = self
                .pipe
                .receive()
                .map_err(|_| Error::new("compositionDisconnected"))?
            {
                let reply: Reply = serde_json::from_slice(&frame)
                    .map_err(|_| Error::new("compositionProtocol"))?;
                if reply.operation != request.operation || reply.revision != request.revision {
                    return Err(Error::new("compositionProtocol"));
                }
                if reply.mutation == Mutation::Changed {
                    self.visible = true;
                }
                log::info!(target:"sayweave", "operation=composition_edit task_id={} turn={} revision={} action={:?} applied={} code={} mutation={:?} elapsed_ms={}", self.task, self.turn, request.revision, action, reply.applied, reply.code, reply.mutation, at.elapsed().as_millis());
                return if reply.applied {
                    Ok(())
                } else {
                    Err(Error::new(reply_error(&reply.code)))
                };
            }
            if at.elapsed() >= Duration::from_secs(2) {
                return Err(Error::new("compositionTimeout"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn update(&mut self, text: &str) -> Result<(), Error> {
        if text == self.text {
            return Ok(());
        }
        let result = self.exchange(Action::Update, text, false);
        if result.is_ok() {
            self.text = text.into();
        } else {
            self.failure = result.as_ref().err().map(|e| e.code.clone());
        }
        result
    }
    fn finish(&mut self, text: &str, mode: Finish) -> Delivery {
        let began = Instant::now();
        let aborted = mode == Finish::Abort;
        let result = if aborted {
            self.exchange(Action::Abort, "", true)
        } else if let Finish::Handoff(reason) = &mode {
            self.exchange(Action::Handoff, "", true)
                .and(Err(Error::new(reason)))
        } else if self.failure.is_some() {
            self.exchange(Action::Handoff, "", true)
                .and(Err(Error::new(self.failure.as_deref().unwrap())))
        } else {
            self.update(text).and_then(|_| {
                if !self.gate.commit() {
                    return self
                        .exchange(Action::Abort, "", true)
                        .and(Err(Error::new("cancelled")));
                }
                self.exchange(Action::Commit, "", false)
            })
        };
        let confirmed = result.is_ok() && mode == Finish::Commit;
        let mut delivery = Delivery::not_submitted(
            result
                .as_ref()
                .err()
                .map(|e| e.code.as_str())
                .unwrap_or(if aborted { "cancelled" } else { "none" }),
        );
        if aborted && result.is_err() {
            delivery.error = Some("compositionRollback".into());
        }
        delivery.stage = "composition".into();
        delivery.elapsed_ms = began.elapsed().as_millis() as u64;
        if confirmed {
            delivery.state = "confirmed".into();
            delivery.error = None;
            delivery.submission = Submission::Submitted;
            delivery.verification = Verification::Confirmed;
        } else if self.visible && !aborted || self.attempted && result.is_err() {
            delivery.state = "unconfirmed".into();
            delivery.submission = if self.visible {
                Submission::Submitted
            } else {
                Submission::Uncertain
            };
            delivery.verification = Verification::Unavailable;
        }
        delivery
    }
    fn close(&mut self) -> Result<(), Error> {
        self.profile
            .close()
            .map_err(|_| Error::new("compositionCleanup"))
    }
}
fn reply_error(code: &str) -> &'static str {
    if code
        == format!(
            "editLease:{:08x}",
            windows::Win32::UI::TextServices::TF_E_READONLY.0 as u32
        )
    {
        // An acknowledged read-only TSF context has no writable range. This is
        // distinct from a missing/disabled component or a failed edit request.
        return "targetReadonlyContext";
    }
    let edit_lock = ["edit:", "requestEdit:", "requestEditResult:"]
        .iter()
        .any(|prefix| code.starts_with(prefix))
        && code.rsplit(':').next()
            == Some(
                format!(
                    "{:08x}",
                    windows::Win32::UI::TextServices::TF_E_LOCKED.0 as u32
                )
                .as_str(),
            );
    if edit_lock {
        "inputComposition"
    } else {
        "compositionRejected"
    }
}
#[cfg(test)]
mod tests {
    use super::reply_error;
    #[test]
    fn foreign_ime_lock_remains_temporary_at_each_edit_boundary() {
        assert_eq!(
            reply_error(&format!(
                "editLease:{:08x}",
                windows::Win32::UI::TextServices::TF_E_READONLY.0 as u32
            )),
            "targetReadonlyContext"
        );
        let locked = format!(
            "{:08x}",
            windows::Win32::UI::TextServices::TF_E_LOCKED.0 as u32
        );
        for phase in [
            "edit",
            "edit:beginProperties",
            "requestEdit",
            "requestEditResult",
        ] {
            assert_eq!(
                reply_error(&format!("{phase}:{locked}")),
                "inputComposition"
            );
        }
        assert_eq!(
            reply_error("edit:updateText:80070057"),
            "compositionRejected"
        );
        assert_eq!(
            reply_error(&format!("editLease:{locked}")),
            "compositionRejected"
        );
        assert_eq!(
            reply_error("draftOwnership:80070057"),
            "compositionRejected"
        );
    }
}
fn target_ready(task: &str, anchor: &Anchor, gate: &DeliveryGate) -> Result<(), Error> {
    let began = Instant::now();
    loop {
        if gate.cancelled() {
            return Err(Error::new("cancelled"));
        }
        if crate::input_state::activity() != anchor.activity {
            return Err(Error::new("editing"));
        }
        if super::native::composition_target(task, anchor)? {
            return Ok(());
        }
        if began.elapsed() >= Duration::from_millis(500) {
            return Err(Error::new("targetChanged"));
        }
        pump();
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn pump() {
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}
