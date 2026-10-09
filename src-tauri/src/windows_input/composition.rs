//! Session-owned host. TSF ranges stay in the target's apartment, never in this worker.
use super::{Anchor, Delivery, DeliveryGate, Submission, Verification};
use crate::history::types::Error;
use sayweave_text_service::{
    client::ProfileLease,
    registration,
    transport::Pipe,
    wire::{Action, Mutation, OutputRegion, Prefix, Reply, Request},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
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
    Observe(Arc<ObservationSink>, oneshot::Sender<bool>),
    Begin(u64, Anchor, oneshot::Sender<Result<(), Error>>),
    Finish(
        u64,
        String,
        Finish,
        Option<String>,
        bool,
        oneshot::Sender<FinishOutcome>,
    ),
    Close(oneshot::Sender<Result<(), Error>>),
    Replace(
        u64,
        String,
        Option<String>,
        oneshot::Sender<Result<(), Error>>,
    ),
    Settle(
        u64,
        String,
        String,
        Option<(String, String)>,
        Arc<AtomicBool>,
        oneshot::Sender<PrefixOutcome>,
    ),
}
pub(crate) struct ObservationSink {
    pub snapshot: crate::contracts::language::Snapshot,
    pub control: Arc<crate::language_policy::LearningControl>,
    pub emit: Arc<dyn Fn(crate::contracts::language::EditEvidence) + Send + Sync>,
}
impl ObservationSink {
    fn allowed(&self) -> bool {
        self.control
            .live(&crate::contracts::language::LearningEligibility::capture(
                &self.snapshot.settings,
            ))
            .any()
    }
}
struct SourceReceipt {
    operation: u64,
    revision: u64,
    raw: String,
}
static RETAINED: Mutex<Option<Arc<Connection>>> = Mutex::new(None);
pub(crate) async fn stop_observer() -> Result<(), Error> {
    let old = RETAINED
        .lock()
        .map_err(|_| Error::new("compositionWorker"))?
        .take();
    if let Some(old) = old
        && let Err(error) = old.close().await
    {
        log::info!(target:"sayweave","operation=observation_retire result=failed error={}",error.code);
    }
    Ok(())
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
        // Retire the previous read-only lease before acquiring the exclusive
        // activation. An old observer can never deactivate the next session.
        stop_observer().await?;
        let (sender, commands) = mpsc::sync_channel(4);
        let connection = Arc::new(Connection { sender });
        let weak_connection = Arc::downgrade(&connection);
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
                        Ok(Command::Observe(sink,reply))=>{let ready=host.observe(sink);let _=reply.send(ready);},
                        Ok(Command::Replace(turn,text,raw,reply))=>{
                            let result=if turn==host.turn {host.update(&text)}else{Err(Error::new("staleSession"))};
                            if result.is_ok(){host.remember_tail(raw);}
                            let _=reply.send(result);
                        },
                        Ok(Command::Settle(turn,prefix,tail,source,permit,reply))=>{
                            let outcome=if turn!=host.turn {PrefixOutcome {prefix:Mutation::NotAttempted,tail_owned:false,error:Some("staleSession".into())}}
                            else if !permit.load(Ordering::Acquire) {PrefixOutcome {prefix:Mutation::NotAttempted,tail_owned:false,error:Some("staleSnapshot".into())}}
                            else {host.settle(&prefix,&tail,&permit)};
                            if outcome.prefix==Mutation::Changed&&let Some((prefix_source,tail_source))=source {host.remember_receipt(OutputRegion::Prefix,prefix_source);host.remember_tail(Some(tail_source));}
                            let _=reply.send(outcome);
                        },
                        Ok(Command::Begin(turn,anchor,reply))=>{let _=reply.send(host.begin_turn(turn,anchor));},
                        Ok(Command::Close(reply))=>{let result=host.close();drop(host);let _=reply.send(result);return;},
                        Ok(Command::Finish(turn,text,mode,raw,keep,reply))=>{
                            if turn!=host.turn {let _=reply.send(FinishOutcome::rejected("staleSession"));continue;}
                            let mut delivery=host.finish(&text,mode);
                            if delivery.committed || delivery.handed_off {host.remember_receipt(OutputRegion::Tail,raw.or_else(||host.raw_tail.as_ref().map(|v|v.raw.clone())).unwrap_or_default());}
                            if keep {let _=reply.send(delivery);continue;}
                            if host.sink.is_some() && let Some(connection)=weak_connection.upgrade() {
                                host.retained_at=Some(Instant::now());
                                if let Ok(mut retained)=RETAINED.lock(){*retained=Some(connection);}
                                let _=reply.send(delivery);continue;
                            }
                            delivery.record_cleanup(host.close().is_err());
                            drop(host);let _=reply.send(delivery);return;
                        },
                        Err(mpsc::RecvTimeoutError::Disconnected)=>return,
                        Err(mpsc::RecvTimeoutError::Timeout)=>{
                            if let Some(at)=host.retained_at {
                                let alive=host.poll_observer();
                                if !alive || at.elapsed()>=Duration::from_secs(60){let _=host.close();if let Ok(mut retained)=RETAINED.lock()&&retained.as_ref().is_some_and(|c|std::sync::Weak::ptr_eq(&weak_connection,&Arc::downgrade(c))){retained.take();}return;}
                            }
                        },
                    }
                }
            })
            .map_err(|_| Error::new("compositionWorker"))?;
        result
            .await
            .map_err(|_| Error::new("compositionWorker"))??;
        Ok(Arc::new(Self {
            connection,
            turn,
            closed: AtomicBool::new(false),
            source_sequence: AtomicU64::new(0),
        }))
    }
    pub(crate) fn connection(&self) -> Arc<Connection> {
        self.connection.clone()
    }
    #[cfg(feature = "desktop-acceptance")]
    pub async fn replace(&self, sequence: u64, text: String) -> Result<(), Error> {
        self.replace_with_source(sequence, text, None).await
    }
    pub async fn observe(&self, sink: Arc<ObservationSink>) -> bool {
        let (reply, result) = oneshot::channel();
        if self
            .connection
            .sender
            .try_send(Command::Observe(sink, reply))
            .is_err()
        {
            return false;
        }
        result.await.unwrap_or(false)
    }
    pub async fn replace_with_source(
        &self,
        sequence: u64,
        text: String,
        raw: Option<String>,
    ) -> Result<(), Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Error::new("staleSession"));
        }
        if sequence == 0 || sequence <= self.source_sequence.fetch_max(sequence, Ordering::AcqRel) {
            return Err(Error::new("staleSnapshot"));
        }
        let (reply, result) = oneshot::channel();
        self.connection
            .sender
            .try_send(Command::Replace(self.turn, text, raw, reply))
            .map_err(|_| Error::new("compositionWorker"))?;
        result.await.map_err(|_| Error::new("compositionWorker"))?
    }
    #[cfg(feature = "desktop-acceptance")]
    pub async fn settle(
        &self,
        sequence: u64,
        prefix: String,
        tail: String,
        permit: Arc<AtomicBool>,
    ) -> PrefixOutcome {
        self.settle_with_source(sequence, prefix, tail, None, permit)
            .await
    }
    pub async fn settle_with_source(
        &self,
        sequence: u64,
        prefix: String,
        tail: String,
        source: Option<(String, String)>,
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
            .try_send(Command::Settle(
                self.turn, prefix, tail, source, permit, reply,
            ))
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
    #[cfg(feature = "desktop-acceptance")]
    pub async fn finish_receipt(&self, text: String, mode: Finish) -> FinishOutcome {
        self.finish_range(text, mode, false).await
    }
    pub async fn finish_receipt_with_source(
        &self,
        text: String,
        mode: Finish,
        raw: String,
    ) -> FinishOutcome {
        self.finish_range_source(text, mode, Some(raw), false).await
    }

    pub async fn handoff_receipt(&self) -> FinishOutcome {
        self.finish_range(String::new(), Finish::Handoff("editing".into()), true)
            .await
    }
    async fn finish_range(&self, text: String, mode: Finish, keep: bool) -> FinishOutcome {
        self.finish_range_source(text, mode, None, keep).await
    }
    async fn finish_range_source(
        &self,
        text: String,
        mode: Finish,
        raw: Option<String>,
        keep: bool,
    ) -> FinishOutcome {
        if self.closed.swap(true, Ordering::AcqRel) {
            return FinishOutcome::rejected("staleSession");
        }
        let (reply, result) = oneshot::channel();
        if self
            .connection
            .sender
            .try_send(Command::Finish(self.turn, text, mode, raw, keep, reply))
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
    supports_observation: bool,
    abandoned_optional: Vec<(u64, u64)>,
    optional_disabled: bool,
    observer_reversal: u64,
    sink: Option<Arc<ObservationSink>>,
    raw_tail: Option<SourceReceipt>,
    receipts: HashMap<(u64, u64, OutputRegion), SourceReceipt>,
    retained_at: Option<Instant>,
    poll_at: Instant,
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
    fn remember_tail(&mut self, raw: Option<String>) {
        self.raw_tail = raw.map(|raw| SourceReceipt {
            operation: self.operation,
            revision: self.revision,
            raw,
        });
    }
    fn remember_receipt(&mut self, region: OutputRegion, raw: String) {
        if raw.is_empty() || self.receipts.len() >= 16 {
            return;
        }
        self.receipts.insert(
            (self.turn, self.operation, region),
            SourceReceipt {
                operation: self.operation,
                revision: self.revision,
                raw,
            },
        );
    }
    fn observe(&mut self, sink: Arc<ObservationSink>) -> bool {
        if !sink.allowed() {
            return false;
        }
        self.observer_reversal = crate::input_state::reversal_epoch();
        if !self.supports_observation
            || !crate::contracts::language::LearningEligibility::capture(&sink.snapshot.settings)
                .any()
            || sink.snapshot.application.key == "unknown"
        {
            return false;
        }
        if self.exchange(Action::ObserveStart, "", true).is_ok() {
            self.sink = Some(sink);
            true
        } else {
            false
        }
    }
    fn observation_records(&mut self, reply: Reply) {
        let Some(observation) = reply.edit_observation else {
            return;
        };
        let Some(sink) = &self.sink else {
            return;
        };
        if !sink.allowed() || self.observer_reversal != crate::input_state::reversal_epoch() {
            return;
        }
        for value in observation.records {
            if !value.ended
                || value.composing
                || value.task != self.task
                || value.before == value.after
            {
                continue;
            }
            let source = self
                .receipts
                .get(&(value.turn, value.operation, value.region))
                .or_else(|| {
                    self.raw_tail.as_ref().filter(|r| {
                        value.region == OutputRegion::Tail
                            && value.turn == self.turn
                            && r.operation == value.operation
                    })
                });
            let Some(source) = source.filter(|r| r.revision == value.revision) else {
                continue;
            };
            let Some(expires_at) = edit_expiry(
                unsafe { GetTickCount64() },
                value.changed_tick,
                crate::data::repository::now(),
            ) else {
                continue;
            };
            (sink.emit)(crate::contracts::language::EditEvidence {
                id: format!(
                    "{}:{}:{}:{:?}:{}",
                    value.task, value.turn, value.operation, value.region, value.sequence
                ),
                task: value.task,
                turn: value.turn,
                operation: value.operation,
                sequence: value.sequence,
                region: if value.region == OutputRegion::Prefix {
                    "prefix"
                } else {
                    "tail"
                }
                .into(),
                expires_at,
                application: sink.snapshot.application.clone(),
                eligibility: crate::contracts::language::LearningEligibility::capture(
                    &sink.snapshot.settings,
                ),
                raw: source.raw.clone(),
                before: value.before,
                after: value.after,
            });
            self.receipts
                .remove(&(value.turn, value.operation, value.region));
        }
    }
    fn stop_observation(&mut self) {
        if self.sink.is_some()
            && let Ok(reply) = self.exchange_reply(Action::ObserveStop, "", None, true, None)
        {
            self.observation_records(reply);
        }
    }
    fn poll_observer(&mut self) -> bool {
        if self.sink.as_ref().is_some_and(|s| !s.allowed())
            || self.observer_reversal != crate::input_state::reversal_epoch()
        {
            self.stop_observation();
            self.sink = None;
            return false;
        }
        if self.poll_at.elapsed() < Duration::from_millis(250) {
            return true;
        }
        self.poll_at = Instant::now();
        if let Ok(reply) = self.exchange_reply(Action::ObservePoll, "", None, true, None) {
            if let Some(value) = &reply.edit_observation {
                log::info!(target:"sayweave","operation=observation_poll task_id={} turn={} active={} records={}",self.task,self.turn,value.active,value.records.len());
            }
            let active = reply
                .edit_observation
                .as_ref()
                .is_some_and(|v| v.active > 0);
            self.observation_records(reply);
            active
                || self
                    .retained_at
                    .is_some_and(|at| at.elapsed() < Duration::from_secs(1))
        } else {
            false
        }
    }
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
            supports_observation: false,
            abandoned_optional: Vec::new(),
            optional_disabled: false,
            observer_reversal: crate::input_state::reversal_epoch(),
            sink: None,
            raw_tail: None,
            receipts: HashMap::new(),
            retained_at: None,
            poll_at: Instant::now(),
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
        self.stop_observation();
        self.anchor = anchor;
        self.turn = turn;
        self.revision = 0;
        self.settled_empty_revision = None;
        self.text.clear();
        self.attempted = false;
        self.raw_tail = None;
        self.receipts.clear();
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
        if let Some(sink) = self.sink.clone() {
            let _ = self.observe(sink);
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
        let optional = matches!(
            action,
            Action::ObserveStart | Action::ObservePoll | Action::ObserveStop
        );
        if optional && self.optional_disabled {
            return Err(Error::new("observationUnavailable"));
        }
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
                // Only a timed-out optional read may finish after its caller
                // abandoned it. Its exact receipt must not poison the next
                // write. Unknown/core mismatches remain protocol failures.
                if discard_optional_reply(&mut self.abandoned_optional, reply.operation, reply.revision) {
                    continue;
                }
                if reply.operation != request.operation || reply.revision != request.revision {
                    return Err(Error::new("compositionProtocol"));
                }
                if action == Action::Inspect {
                    self.supports_observation=reply.edit_observation.as_ref().is_some_and(|v|v.capability==2);
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
                if optional {
                    self.abandoned_optional
                        .push((request.operation, request.revision));
                    // Never evict an outstanding identity to make room for
                    // another read. Stop optional work at the fixed bound.
                    self.optional_disabled = self.abandoned_optional.len() >= 16;
                }
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
        let mut cleanup_failed = false;
        let mut handed_off = false;
        let mut already_settled = false;
        let result = if aborted {
            let settled =
                completed_prefix(self.settled_empty_revision, self.revision, &self.text, "");
            if settled {
                let result = self.release_empty_write_lease();
                cleanup_failed = result.is_err();
                result
            } else {
                let result = self.exchange(Action::Abort, "", true);
                rollback_failed = result.is_err();
                result
            }
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
                let accepted = self.gate.commit();
                if settled {
                    // Keep the read-only receipts, but retire the write-side
                    // insertion lease and its sink before post-delivery edits.
                    // Handoff releases the lease without another text commit.
                    let released = self.release_empty_write_lease();
                    cleanup_failed = released.is_err();
                    if !accepted {
                        cancelled = true;
                        return released.and(Err(Error::new("cancelled")));
                    }
                    already_settled = true;
                    return released;
                }
                if !accepted {
                    cancelled = true;
                    let result = self.exchange(Action::Abort, "", true);
                    rollback_failed = result.is_err();
                    return result.and(Err(Error::new("cancelled")));
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
        if cleanup_failed {
            delivery.error = Some("compositionCleanup".into());
        }
        FinishOutcome {
            delivery,
            committed,
            handed_off,
            cancelled,
            rollback_failed,
        }
    }
    fn release_empty_write_lease(&mut self) -> Result<(), Error> {
        let result = self.exchange(Action::Handoff, "", true);
        if result.is_ok() {
            self.range_open = false;
        }
        result
    }
    fn close(&mut self) -> Result<(), Error> {
        self.stop_observation();
        self.sink = None;
        self.profile
            .close()
            .map_err(|_| Error::new("compositionCleanup"))
    }
}
fn discard_optional_reply(abandoned: &mut Vec<(u64, u64)>, operation: u64, revision: u64) -> bool {
    if let Some(index) = abandoned
        .iter()
        .position(|value| *value == (operation, revision))
    {
        abandoned.swap_remove(index);
        true
    } else {
        false
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
fn edit_expiry(now_tick: u64, changed_tick: u64, wall: i64) -> Option<i64> {
    if changed_tick == 0 || changed_tick > now_tick {
        return None;
    }
    let elapsed = now_tick - changed_tick;
    (elapsed < 600000).then(|| wall + 600000 - elapsed as i64)
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

#[cfg(test)]
mod tests {
    use super::*;
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
    #[test]
    fn observation_off_on_does_not_reactivate_a_frozen_settings_revision() {
        use crate::contracts::language::*;
        let first = LanguageSettings {
            automatic_terms: Settings {
                enabled: true,
                revision: 1,
            },
            personalization: Settings {
                enabled: true,
                revision: 1,
            },
        };
        let control = Arc::new(crate::language_policy::LearningControl::new(first.clone()));
        let mut snapshot = Snapshot {
            settings: first.clone(),
            application: ApplicationContext::default(),
            profile_version: 0,
            shared_version: 0,
            preferences: vec![],
        };
        let old = ObservationSink {
            snapshot: snapshot.clone(),
            control: control.clone(),
            emit: Arc::new(|_| {}),
        };
        assert!(old.allowed());
        control.publish(&LanguageSettings {
            automatic_terms: first.automatic_terms,
            personalization: Settings {
                enabled: false,
                revision: 2,
            },
        });
        assert!(old.allowed());
        control.publish(&LanguageSettings {
            automatic_terms: Settings {
                enabled: false,
                revision: 2,
            },
            personalization: Settings {
                enabled: false,
                revision: 2,
            },
        });
        assert!(!old.allowed());
        control.publish(&LanguageSettings {
            automatic_terms: Settings {
                enabled: true,
                revision: 3,
            },
            personalization: Settings {
                enabled: false,
                revision: 2,
            },
        });
        assert!(!old.allowed());
        snapshot.settings = control.snapshot();
        let new = ObservationSink {
            snapshot,
            control,
            emit: Arc::new(|_| {}),
        };
        assert!(new.allowed());
    }
    #[test]
    fn edit_expiry_uses_native_change_time_instead_of_late_poll_time() {
        assert_eq!(edit_expiry(120000, 100000, 1000000), Some(1580000));
        assert!(edit_expiry(700000, 100000, 1000000).is_none());
        assert!(edit_expiry(100, 101, 1000000).is_none());
        assert!(edit_expiry(100, 0, 1000000).is_none());
    }
    #[test]
    fn late_optional_receipt_is_discarded_without_accepting_unknown_write_receipts() {
        // ObserveStart(2,0) exceeded its budget; Update(3,1) is now waiting.
        let mut abandoned = vec![(2, 0)];
        let frames = [(2, 0), (3, 1)];
        let mut accepted = vec![];
        for (operation, revision) in frames {
            if !discard_optional_reply(&mut abandoned, operation, revision) {
                assert_eq!((operation, revision), (3, 1));
                accepted.push((operation, revision));
            }
        }
        assert_eq!(accepted, vec![(3, 1)]);
        assert!(abandoned.is_empty());
        // A duplicate or a wrong revision is not silently swallowed.
        assert!(!discard_optional_reply(&mut abandoned, 2, 0));
        abandoned.push((4, 1));
        assert!(!discard_optional_reply(&mut abandoned, 4, 2));
        assert_eq!(abandoned, vec![(4, 1)]);
    }
}
