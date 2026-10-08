//! Session-owned host. TSF ranges stay in the target's apartment, never in this worker.
use super::{Anchor, Delivery, DeliveryGate, Submission, Verification};
use crate::history::types::Error;
use sayweave_text_service::{
    client::ProfileLease,
    registration,
    transport::Pipe,
    wire::{Action, Mutation, Prefix, Reply, Request},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};
use tokio::sync::oneshot;
use windows::Win32::{
    Foundation::*,
    System::{Com::*, SystemInformation::GetTickCount64},
    UI::WindowsAndMessaging::*,
};

pub(crate) enum Command {
    Begin(u64, Anchor, oneshot::Sender<Result<(), Error>>),
    Finish(u64, String, Finish, bool, oneshot::Sender<FinishOutcome>),
    Close(oneshot::Sender<Result<(), Error>>),
    Replace(u64, String, oneshot::Sender<Result<(), Error>>),
    Settle(
        u64,
        String,
        String,
        Arc<AtomicBool>,
        oneshot::Sender<PrefixOutcome>,
    ),
}
pub(crate) struct PrefixOutcome {
    pub prefix: Mutation,
    pub tail_owned: bool,
    pub error: Option<String>,
}
pub(crate) struct FinishOutcome {
    pub delivery: Delivery,
    /// Acknowledgement of this Commit; prior visible text is independent.
    pub committed: bool,
    pub handed_off: bool,
    /// Effective abort decision, independent of rollback or lease cleanup errors.
    pub cancelled: bool,
    pub rollback_failed: bool,
}
impl FinishOutcome {
    pub(crate) fn record_cleanup(&mut self, failed: bool) {
        if failed {
            self.delivery.error = Some("compositionCleanup".into());
        }
    }
    fn rejected(code: &str) -> Self {
        Self {
            delivery: Delivery::not_submitted(code),
            committed: false,
            handed_off: false,
            cancelled: false,
            rollback_failed: false,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Finish {
    Commit,
    Handoff(String),
    Abort,
}
pub struct Draft {
    connection: Arc<Connection>,
    turn: u64,
    closed: AtomicBool,
    source_sequence: AtomicU64,
}
/// One apartment, activation lease and authenticated pipe for the entire session.
/// Editing retires only the range; it does not toggle the process-wide TSF service.
pub(crate) struct Connection {
    sender: mpsc::SyncSender<Command>,
}
impl Connection {
    pub async fn begin_turn(
        self: &Arc<Self>,
        turn: u64,
        anchor: Anchor,
    ) -> Result<Arc<Draft>, Error> {
        let (reply, result) = oneshot::channel();
        self.sender
            .try_send(Command::Begin(turn, anchor, reply))
            .map_err(|_| Error::new("compositionWorker"))?;
        result
            .await
            .map_err(|_| Error::new("compositionWorker"))??;
        Ok(Arc::new(Draft {
            connection: self.clone(),
            turn,
            closed: AtomicBool::new(false),
            source_sequence: AtomicU64::new(0),
        }))
    }
    pub async fn close(&self) -> Result<(), Error> {
        let (reply, result) = oneshot::channel();
        self.sender
            .try_send(Command::Close(reply))
            .map_err(|_| Error::new("compositionWorker"))?;
        result.await.map_err(|_| Error::new("compositionWorker"))?
    }
}
impl Draft {
    #[cfg(test)]
    pub(crate) fn fixture() -> (Arc<Self>, mpsc::Receiver<Command>) {
        let (sender, receiver) = mpsc::sync_channel(4);
        (
            Arc::new(Self {
                connection: Arc::new(Connection { sender }),
                turn: 1,
                closed: AtomicBool::new(false),
                source_sequence: AtomicU64::new(0),
            }),
            receiver,
        )
    }
    pub async fn begin(
        task: String,
        turn: u64,
        anchor: Anchor,
        gate: Arc<DeliveryGate>,
    ) -> Result<Arc<Self>, Error> {
        let (sender, commands) = mpsc::sync_channel(4);
        let (ready, result) = oneshot::channel();
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
                loop {
                    pump();
                    match commands.recv_timeout(Duration::from_millis(5)) {
                        Ok(Command::Replace(turn,text,reply))=>{
                            let result=if turn==host.turn {host.update(&text)}else{Err(Error::new("staleSession"))};
                            let _=reply.send(result);
                        },
                        Ok(Command::Settle(turn,prefix,tail,permit,reply))=>{
                            let outcome=if turn!=host.turn {PrefixOutcome {prefix:Mutation::NotAttempted,tail_owned:false,error:Some("staleSession".into())}}
                            else if !permit.load(Ordering::Acquire) {PrefixOutcome {prefix:Mutation::NotAttempted,tail_owned:false,error:Some("staleSnapshot".into())}}
                            else {host.settle(&prefix,&tail,&permit)};
                            let _=reply.send(outcome);
                        },
                        Ok(Command::Begin(turn,anchor,reply))=>{let _=reply.send(host.begin_turn(turn,anchor));},
                        Ok(Command::Close(reply))=>{let result=host.close();drop(host);let _=reply.send(result);return;},
                        Ok(Command::Finish(turn,text,mode,keep,reply))=>{
                            if turn!=host.turn {let _=reply.send(FinishOutcome::rejected("staleSession"));continue;}
                            let mut delivery=host.finish(&text,mode);
                            if keep {let _=reply.send(delivery);continue;}
                            delivery.record_cleanup(host.close().is_err());
                            drop(host);let _=reply.send(delivery);return;
                        },
                        Err(mpsc::RecvTimeoutError::Disconnected)=>return,
                        Err(mpsc::RecvTimeoutError::Timeout)=>{},
                    }
                }
            })
            .map_err(|_| Error::new("compositionWorker"))?;
        result
            .await
            .map_err(|_| Error::new("compositionWorker"))??;
        Ok(Arc::new(Self {
            connection: Arc::new(Connection { sender }),
            turn,
            closed: AtomicBool::new(false),
            source_sequence: AtomicU64::new(0),
        }))
    }
    pub(crate) fn connection(&self) -> Arc<Connection> {
        self.connection.clone()
    }
    pub async fn replace(&self, sequence: u64, text: String) -> Result<(), Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::new("staleSession"));
        }
        if sequence == 0 || sequence <= self.source_sequence.fetch_max(sequence, Ordering::AcqRel) {
            return Err(Error::new("staleSnapshot"));
        }
        let (reply, result) = oneshot::channel();
        self.connection
            .sender
            .try_send(Command::Replace(self.turn, text, reply))
            .map_err(|_| Error::new("compositionWorker"))?;
        result.await.map_err(|_| Error::new("compositionWorker"))?
    }
    pub async fn settle(
        &self,
        sequence: u64,
        prefix: String,
        tail: String,
        permit: Arc<AtomicBool>,
    ) -> PrefixOutcome {
        let fail = |code: &str| PrefixOutcome {
            prefix: Mutation::NotAttempted,
            tail_owned: false,
            error: Some(code.into()),
        };
        if self.closed.load(Ordering::Acquire) {
            return fail("staleSession");
        }
        if sequence == 0 || sequence < self.source_sequence.fetch_max(sequence, Ordering::AcqRel) {
            return fail("staleSnapshot");
        }
        let (reply, result) = oneshot::channel();
        if self
            .connection
            .sender
            .try_send(Command::Settle(self.turn, prefix, tail, permit, reply))
            .is_err()
        {
            return fail("compositionWorker");
        }
        result.await.unwrap_or_else(|_| PrefixOutcome {
            prefix: Mutation::Uncertain,
            tail_owned: false,
            error: Some("compositionWorker".into()),
        })
    }
    pub async fn finish(&self, text: String, mode: Finish) -> Delivery {
        self.finish_range(text, mode, false).await.delivery
    }
    pub async fn finish_receipt(&self, text: String, mode: Finish) -> FinishOutcome {
        self.finish_range(text, mode, false).await
    }

    pub async fn handoff_receipt(&self) -> FinishOutcome {
        self.finish_range(String::new(), Finish::Handoff("editing".into()), true)
            .await
    }
    async fn finish_range(&self, text: String, mode: Finish, keep: bool) -> FinishOutcome {
        if self.closed.swap(true, Ordering::AcqRel) {
            return FinishOutcome::rejected("staleSession");
        }
        let (reply, result) = oneshot::channel();
        if self
            .connection
            .sender
            .try_send(Command::Finish(self.turn, text, mode, keep, reply))
            .is_err()
        {
            let mut outcome = Delivery::not_submitted("compositionWorker");
            outcome.submission = Submission::Uncertain;
            outcome.state = "unconfirmed".into();
            return FinishOutcome {
                delivery: outcome,
                committed: false,
                handed_off: false,
                cancelled: false,
                rollback_failed: false,
            };
        }
        result.await.unwrap_or_else(|_| {
            let mut outcome = Delivery::not_submitted("compositionWorker");
            outcome.submission = Submission::Uncertain;
            outcome.state = "unconfirmed".into();
            FinishOutcome {
                delivery: outcome,
                committed: false,
                handed_off: false,
                cancelled: false,
                rollback_failed: false,
            }
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
    range_epoch: u64,
    transport_attempted: bool,
    text: String,
    attempted: bool,
    visible: bool,
    failure: Option<String>,
    range_open: bool,
    /// Exact successful Settle that already ended an empty composition.
    settled_empty_revision: Option<u64>,
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
            range_epoch: 0,
            transport_attempted: false,
            text: String::new(),
            attempted: false,
            visible: false,
            failure: None,
            range_open: false,
            settled_empty_revision: None,
            profile,
            _lease: lease,
            _apartment: apartment,
        };
        // Inspect the component executing in this target, not the registered file.
        // An old loaded DLL rejects v2 before any edit; there is no downgrade.
        host.exchange(Action::Inspect, "", false).map_err(|error| {
            log::info!(target:"sayweave", "operation=draft_runtime task_id={} turn={} result=rejected error={}", host.task,host.turn,error.code);
            if matches!(error.code.as_str(), "compositionDisconnected" | "compositionTimeout" | "compositionProtocol") {
                Error::new("compositionRuntimeUnavailable")
            } else {
                error
            }
        })?;
        super::native::composition_begin_target(&host.task, &host.anchor)?;
        host.exchange(Action::Begin, "", false)?;
        host.range_open = true;
        // Begin itself does not change text. Do not publish a writable draft if input moved
        // while the target's asynchronous edit session was being scheduled/executed.
        super::native::composition_begin_target(&host.task, &host.anchor)?;
        Ok(host)
    }
    fn begin_turn(&mut self, turn: u64, anchor: Anchor) -> Result<(), Error> {
        if self.range_open
            || turn < self.turn
            || self.failure.is_some()
            || anchor.window != self.anchor.window
            || anchor.process != self.anchor.process
            || anchor.lifetime != self.anchor.lifetime
            || anchor.capture_id != self.anchor.capture_id
        {
            return Err(Error::new("staleSession"));
        }
        self.anchor = anchor;
        self.turn = turn;
        self.revision = 0;
        self.settled_empty_revision = None;
        self.text.clear();
        self.attempted = false;
        self.visible = false;
        self.exchange(Action::Begin, "", false)?;
        self.range_open = true;
        if let Err(error) = super::native::composition_begin_target(&self.task, &self.anchor) {
            // No text was written. Retire this unpublished range before a retry can
            // acquire another one; never leave an acknowledged Begin orphaned.
            self.exchange(Action::Handoff, "", true)?;
            self.range_open = false;
            return Err(error);
        }
        Ok(())
    }
    fn exchange(&mut self, action: Action, text: &str, cleanup: bool) -> Result<(), Error> {
        let reply = self.exchange_reply(action, text, None, cleanup, None)?;
        if reply.applied {
            Ok(())
        } else {
            Err(Error::new(reply_error(&reply.code)))
        }
    }
    fn exchange_reply(
        &mut self,
        action: Action,
        text: &str,
        prefix: Option<Prefix>,
        cleanup: bool,
        permission: Option<&AtomicBool>,
    ) -> Result<Reply, Error> {
        self.transport_attempted = false;
        if !cleanup {
            target_ready(&self.task, &self.anchor, &self.gate)?;
        }
        if action == Action::Begin {
            super::native::composition_begin_target(&self.task, &self.anchor)?;
        }
        self.operation += 1;
        if matches!(action, Action::Update | Action::Settle) {
            self.revision += 1;
        }
        let request = Request {
            protocol: sayweave_text_service::wire::PROTOCOL,
            capability: self.capability.clone(),
            task: self.task.clone(),
            turn: self.turn,
            operation: self.operation,
            revision: self.revision,
            deadline_tick: unsafe { GetTickCount64() } + 2000,
            action,
            text: text.into(),
            prefix,
        };
        if !request.valid() {
            return Err(Error::new("compositionTextLimit"));
        }
        if matches!(action, Action::Update | Action::Settle) {
            self.attempted = true;
        }
        if permission.is_some_and(|p| !p.load(Ordering::Acquire)) {
            return Err(Error::new("staleSnapshot"));
        }
        self.transport_attempted = true;
        self.pipe
            .send(&serde_json::to_vec(&request).map_err(|_| Error::new("compositionProtocol"))?)
            .map_err(|error| {
                log::info!(target:"sayweave", "operation=draft_transport task_id={} turn={} action={:?} phase=send hresult={:08x}", self.task,self.turn,action,error.code().0 as u32);
                Error::new("compositionDisconnected")
            })?;
        let at = Instant::now();
        loop {
            pump();
            if let Some(frame) = self
                .pipe
                .receive()
                .map_err(|error| {
                    log::info!(target:"sayweave", "operation=draft_transport task_id={} turn={} action={:?} phase=receive hresult={:08x}", self.task,self.turn,action,error.code().0 as u32);
                    Error::new("compositionDisconnected")
                })?
            {
                let reply: Reply = serde_json::from_slice(&frame)
                    .map_err(|_| Error::new("compositionProtocol"))?;
                if reply.operation != request.operation || reply.revision != request.revision {
                    return Err(Error::new("compositionProtocol"));
                }
                if action == Action::Inspect {
                    let mut pid = 0;
                    let thread = unsafe { GetWindowThreadProcessId(HWND(self.anchor.window as *mut _), Some(&mut pid)) };
                    if pid != self.anchor.process || !reply.compatible_runtime(pid, thread) {
                        return Err(Error::new("compositionTargetVersion"));
                    }
                    log::info!(target:"sayweave", "operation=draft_runtime task_id={} turn={} result=ready protocol={} process={} thread={}", self.task,self.turn,sayweave_text_service::wire::PROTOCOL,pid,thread);
                }
                if reply.mutation == Mutation::Changed {
                    self.visible = true;
                }
                if reply.applied {self.range_epoch=reply.range_epoch;}
                log::info!(target:"sayweave", "operation=composition_edit task_id={} turn={} revision={} action={:?} applied={} code={} mutation={:?} elapsed_ms={}", self.task, self.turn, request.revision, action, reply.applied, reply.code, reply.mutation, at.elapsed().as_millis());
                return Ok(reply);
            }
            if at.elapsed() >= Duration::from_secs(2) {
                return Err(Error::new("compositionTimeout"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn settle(&mut self, prefix: &str, tail: &str, permit: &AtomicBool) -> PrefixOutcome {
        self.settled_empty_revision = None;
        // Only this existing host's range/revision may settle a prefix. Never
        // choose a new target or replay a request after a possibly changed range.
        let expected = Prefix {
            text: prefix.into(),
            expected_revision: self.revision,
            range_epoch: self.range_epoch,
        };
        if !permit.load(Ordering::Acquire) {
            return PrefixOutcome {
                prefix: Mutation::NotAttempted,
                tail_owned: false,
                error: Some("staleSnapshot".into()),
            };
        }
        let result = self.exchange_reply(Action::Settle, tail, Some(expected), false, Some(permit));
        let outcome = match result {
            Ok(reply) => {
                let settlement = reply.settlement.unwrap_or_default();
                if settlement.prefix != Mutation::NotAttempted
                    && settlement.prefix_operation != reply.operation
                {
                    PrefixOutcome {
                        prefix: Mutation::Uncertain,
                        tail_owned: false,
                        error: Some("compositionProtocol".into()),
                    }
                } else {
                    PrefixOutcome {
                        prefix: settlement.prefix,
                        tail_owned: reply.applied && settlement.tail_owned,
                        error: (!reply.applied).then(|| reply_error(&reply.code).into()),
                    }
                }
            }
            Err(error) => PrefixOutcome {
                prefix: if self.transport_attempted {
                    Mutation::Uncertain
                } else {
                    Mutation::NotAttempted
                },
                tail_owned: false,
                error: Some(error.code),
            },
        };
        if outcome.tail_owned {
            self.text = tail.into();
        }
        if outcome.prefix == Mutation::Changed {
            self.visible = true;
        }
        if outcome.prefix == Mutation::Changed
            && outcome.tail_owned
            && outcome.error.is_none()
            && tail.is_empty()
        {
            self.settled_empty_revision = Some(self.revision);
        }
        if outcome.error.is_some()
            && !(outcome.prefix == Mutation::NotAttempted
                && outcome.error.as_deref() == Some("staleSnapshot"))
        {
            self.failure = outcome.error.clone();
        }
        outcome
    }
    fn update(&mut self, text: &str) -> Result<(), Error> {
        if text == self.text {
            return Ok(());
        }
        self.settled_empty_revision = None;
        let result = self.exchange(Action::Update, text, false);
        if result.is_ok() {
            self.text = text.into();
        } else {
            self.failure = result.as_ref().err().map(|e| e.code.clone());
        }
        result
    }
    fn finish(&mut self, text: &str, mode: Finish) -> FinishOutcome {
        let began = Instant::now();
        let aborted = mode == Finish::Abort;
        let mut cancelled = aborted;
        let mut rollback_failed = false;
        let mut handed_off = false;
        let mut already_settled = false;
        let result = if aborted {
            let result = self.exchange(Action::Abort, "", true);
            rollback_failed = result.is_err();
            result
        } else if let Finish::Handoff(reason) = &mode {
            let result = self.exchange(Action::Handoff, "", true);
            if result.is_ok() {
                handed_off = true;
                self.range_open = false;
            } else {
                self.failure = result.as_ref().err().map(|e| e.code.clone());
            }
            result.and(Err(Error::new(reason)))
        } else if self.failure.is_some() {
            self.exchange(Action::Handoff, "", true)
                .and(Err(Error::new(self.failure.as_deref().unwrap())))
        } else {
            self.update(text).and_then(|_| {
                // A successful empty-tail Settle already submitted the text
                // and ended its composition. Only release the remaining lease;
                // do not require an unchanged caret for a second empty Commit.
                let settled =
                    completed_prefix(self.settled_empty_revision, self.revision, &self.text, text);
                if !self.gate.commit() {
                    cancelled = true;
                    if settled {
                        return Err(Error::new("cancelled"));
                    }
                    let result = self.exchange(Action::Abort, "", true);
                    rollback_failed = result.is_err();
                    return result.and(Err(Error::new("cancelled")));
                }
                if settled {
                    already_settled = true;
                    return Ok(());
                }
                self.exchange(Action::Commit, "", false)
            })
        };
        // A TSF acknowledgement is submission, not renderer completion. Even a
        // matching fresh UIA call can return the final Update's pre-commit cache.
        // There is no provider-correlated completion fence in this protocol, so
        // do not promote an acknowledged commit to independently confirmed input.
        let committed = result.is_ok() && mode == Finish::Commit && !already_settled;
        log::info!(target:"sayweave", "operation=draft_proof task_id={} turn={} revision={} phase=confirm submitted={} already_settled={} confirmed=false completion_boundary=unavailable", self.task,self.turn,self.revision,committed || already_settled,already_settled);
        let mut delivery = Delivery::not_submitted(
            result
                .as_ref()
                .err()
                .map(|e| e.code.as_str())
                .unwrap_or(if aborted { "cancelled" } else { "none" }),
        );
        if rollback_failed {
            delivery.error = Some("compositionRollback".into());
        }
        delivery.stage = "composition".into();
        delivery.elapsed_ms = began.elapsed().as_millis() as u64;
        if committed || self.visible && !aborted || self.attempted && result.is_err() {
            delivery.state = "unconfirmed".into();
            if committed || already_settled {
                delivery.error = Some("inputUnconfirmed".into());
            }
            delivery.submission = if self.visible {
                Submission::Submitted
            } else {
                Submission::Uncertain
            };
            delivery.verification = Verification::Unavailable;
        }
        FinishOutcome {
            delivery,
            committed,
            handed_off,
            cancelled,
            rollback_failed,
        }
    }
    fn close(&mut self) -> Result<(), Error> {
        self.profile
            .close()
            .map_err(|_| Error::new("compositionCleanup"))
    }
}
fn completed_prefix(receipt: Option<u64>, revision: u64, stored: &str, requested: &str) -> bool {
    receipt == Some(revision) && stored.is_empty() && requested.is_empty()
}
fn reply_error(code: &str) -> &'static str {
    if code == "keyboardHandoffFailed" {
        return "compositionHandoffFailed";
    }
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
    fn empty_tail_completion_requires_the_exact_settlement_and_no_remaining_text() {
        assert!(super::completed_prefix(Some(9), 9, "", ""));
        assert!(!super::completed_prefix(None, 9, "", ""));
        assert!(!super::completed_prefix(Some(8), 9, "", ""));
        assert!(!super::completed_prefix(Some(9), 9, "尾部", ""));
        assert!(!super::completed_prefix(Some(9), 9, "", "\n"));
        assert!(!super::completed_prefix(Some(9), 9, "", "新尾部"));
    }
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
