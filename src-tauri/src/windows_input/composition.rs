//! Session-owned host. TSF ranges stay in the target's apartment, never in this worker.
use super::{Anchor, Delivery, DeliveryGate, Submission, Verification};
use crate::history::types::Error;
use sayweave_text_service::{
    client::ProfileLease,
    lifecycle::{ControlAction, ControlReply, ControlRequest, ReceiptKey},
    registration,
    transport::Pipe,
    wire::{
        Action, BlockAction, BlockCommand, BlockSnapshot, EngineBinding, Mutation, OutputRegion,
        Prefix, Reply, Request, Revoke,
    },
};
use std::{
    collections::{HashMap, VecDeque},
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
mod retirement;
use retirement::{Retirement, WorkerCompletion};

pub(crate) enum Command {
    ReplaceTracked(
        u64,
        String,
        Option<String>,
        String,
        Arc<AtomicBool>,
        oneshot::Sender<Result<BlockSnapshot, WriteFailure>>,
    ),
    Acquire(
        u64,
        BlockCommand,
        Arc<AtomicBool>,
        u64,
        oneshot::Sender<Result<Reply, Error>>,
    ),
    Block(
        u64,
        Option<Anchor>,
        BlockCommand,
        String,
        Arc<AtomicBool>,
        oneshot::Sender<Result<Reply, Error>>,
    ),
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
    Replace(
        u64,
        String,
        Option<String>,
        Option<Arc<AtomicBool>>,
        oneshot::Sender<Result<(), WriteFailure>>,
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
pub(crate) fn retain_failed(connection: &Arc<Connection>) {
    if let Ok(mut slot) = RETAINED.lock() {
        slot.get_or_insert_with(|| connection.clone());
    }
}
pub(crate) async fn stop_observer() -> Result<(), Error> {
    stop_observer_for(None).await
}
async fn stop_observer_for(gate: Option<&DeliveryGate>) -> Result<(), Error> {
    let old = RETAINED
        .lock()
        .map_err(|_| Error::new("compositionWorker"))?
        .clone();
    if let Some(old) = old {
        let began = Instant::now();
        let result = wait_retirement(&old, gate).await;
        log::info!(target:"sayweave","operation=observation_retire task_id={} turn={} result={} error={} elapsed_ms={}",old.retirement.task,old.retirement.turn,if result.is_ok(){"complete"}else{"failed"},result.as_ref().err().map_or("none",|e|e.code.as_str()),began.elapsed().as_millis());
        result?;
        clear_retained(&RETAINED, &old.retirement);
    }
    Ok(())
}
async fn wait_retirement(old: &Connection, gate: Option<&DeliveryGate>) -> Result<(), Error> {
    let remaining = gate.map_or(15_000, |gate| {
        gate.preparation_deadline()
            .saturating_sub(unsafe { GetTickCount64() })
    });
    let cancelled = async {
        loop {
            if gate.is_some_and(DeliveryGate::cancelled) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    };
    tokio::select! {
        biased;
        _ = cancelled => Err(Error::new("cancelled")),
        result = tokio::time::timeout(Duration::from_millis(remaining), old.close()) =>
            result.unwrap_or_else(|_| Err(Error::new("compositionTimeout"))),
    }
}
fn clear_retained(slot: &Mutex<Option<Arc<Connection>>>, retirement: &Arc<Retirement>) {
    if let Ok(mut retained) = slot.lock()
        && retained
            .as_ref()
            .is_some_and(|c| Arc::ptr_eq(&c.retirement, retirement))
    {
        retained.take();
    }
}
pub(crate) struct PrefixOutcome {
    pub prefix: Mutation,
    pub tail_owned: bool,
    pub error: Option<String>,
}
#[derive(Debug)]
pub(crate) struct WriteFailure {
    pub error: Error,
    pub mutation: Mutation,
    /// The body write completed; only its independent tracking failed.
    pub tracking_failed: bool,
    /// A settled refusal of the old range, not permission for a fresh target.
    pub scope_unavailable: bool,
}
impl WriteFailure {
    fn not_attempted(code: &str) -> Self {
        Self {
            error: Error::new(code),
            mutation: Mutation::NotAttempted,
            tracking_failed: false,
            scope_unavailable: false,
        }
    }
    fn uncertain(error: Error) -> Self {
        Self {
            error,
            mutation: Mutation::Uncertain,
            tracking_failed: false,
            scope_unavailable: false,
        }
    }
}
fn update_receipt(
    reply: Reply,
    text: &str,
    block_id: Option<&str>,
) -> Result<Option<BlockSnapshot>, WriteFailure> {
    if reply.applied {
        if let Some(id) = block_id
            && reply
                .block
                .as_ref()
                .is_none_or(|b| b.id != id || b.epoch == 0 || b.text != text || b.writable)
        {
            return Err(WriteFailure {
                error: Error::new("compositionPartial"),
                mutation: reply.mutation,
                tracking_failed: true,
                scope_unavailable: false,
            });
        }
        return Ok(reply.block);
    }
    let tracking_failed = block_id.is_some() && reply.code.starts_with("edit:writeTracking:");
    let error = if reply.mutation != Mutation::NotAttempted {
        "compositionPartial"
    } else {
        reply_error(&reply.code)
    };
    log::info!(target:"sayweave", "operation=composition_update_refused task_id={} turn={} mutation={:?} tracking_failed={tracking_failed} native_code={}",reply.task,reply.turn,reply.mutation,reply.code);
    Err(WriteFailure {
        error: Error::new(error),
        mutation: reply.mutation,
        tracking_failed,
        scope_unavailable: reply.mutation == Mutation::NotAttempted
            && reply.code.starts_with("draftOwnershipTerminated:"),
    })
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
pub(crate) enum ReacquireOutcome {
    Ready {
        draft: Arc<Draft>,
        snapshot: BlockSnapshot,
    },
    AwaitingProof,
    Unavailable,
    Invalidated,
    Uncertain,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BlockRefusal {
    AwaitingProof,
    Unavailable,
    Invalidated,
    Uncertain,
}
pub(crate) fn block_refusal(reply: &Reply) -> BlockRefusal {
    if reply.mutation != Mutation::NotAttempted {
        BlockRefusal::Uncertain
    } else if reply.code.starts_with("blockAwaitingRepresentation:")
        || reply.code.starts_with("blockInputMethodBusy:")
        || reply.code.starts_with("blockSnapshotChanged:")
        || reply.code.ends_with(":80040500")
    {
        BlockRefusal::AwaitingProof
    } else if reply.code.starts_with("blockUnavailable:") {
        BlockRefusal::Unavailable
    } else {
        BlockRefusal::Invalidated
    }
}
/// One apartment, activation lease and authenticated pipe for the entire session.
/// Editing retires only the range; it does not toggle the process-wide TSF service.
pub(crate) struct Connection {
    sender: mpsc::SyncSender<Command>,
    retirement: Arc<Retirement>,
}
impl Connection {
    pub(crate) async fn reacquire_block(
        self: &Arc<Self>,
        turn: u64,
        snapshot: BlockSnapshot,
        permit: Arc<AtomicBool>,
        remaining: Duration,
    ) -> Result<ReacquireOutcome, Error> {
        let request = BlockCommand {
            action: BlockAction::Acquire,
            id: snapshot.id.clone(),
            epoch: snapshot.epoch,
            version: snapshot.version,
            lease: snapshot.lease,
            expected: snapshot.text.clone(),
            selection_start: snapshot.selection_start,
            selection_end: snapshot.selection_end,
        };
        let deadline = unsafe { GetTickCount64() }
            .saturating_add(remaining.as_millis().min(u64::MAX as u128) as u64);
        let (send, receive) = oneshot::channel();
        self.sender
            .try_send(Command::Acquire(turn, request, permit, deadline, send))
            .map_err(|_| Error::new("compositionWorker"))?;
        let reply = receive
            .await
            .map_err(|_| Error::new("compositionWorker"))??;
        if !reply.applied {
            return Ok(match block_refusal(&reply) {
                BlockRefusal::AwaitingProof => ReacquireOutcome::AwaitingProof,
                BlockRefusal::Unavailable => ReacquireOutcome::Unavailable,
                BlockRefusal::Invalidated => ReacquireOutcome::Invalidated,
                BlockRefusal::Uncertain => ReacquireOutcome::Uncertain,
            });
        }
        let snapshot = reply
            .block
            .filter(|b| b.writable)
            .ok_or_else(|| Error::new("compositionProtocol"))?;
        Ok(ReacquireOutcome::Ready {
            draft: Arc::new(Draft {
                connection: self.clone(),
                turn,
                closed: AtomicBool::new(false),
                source_sequence: AtomicU64::new(0),
            }),
            snapshot,
        })
    }
    pub(crate) async fn block(
        &self,
        turn: u64,
        anchor: Option<Anchor>,
        command: BlockCommand,
        text: String,
        permit: Arc<AtomicBool>,
    ) -> Result<Reply, Error> {
        let (reply, result) = oneshot::channel();
        self.sender
            .try_send(Command::Block(turn, anchor, command, text, permit, reply))
            .map_err(|_| Error::new("compositionWorker"))?;
        result.await.map_err(|_| Error::new("compositionWorker"))?
    }
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
        self.retirement.close().await
    }
}
impl Draft {
    #[cfg(test)]
    pub(crate) fn fixture() -> (Arc<Self>, mpsc::Receiver<Command>) {
        let (sender, receiver) = mpsc::sync_channel(4);
        (
            Arc::new(Self {
                connection: Arc::new(Connection {
                    sender,
                    retirement: Retirement::new("fixture".into(), 1),
                }),
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
        stop_observer_for(Some(&gate)).await?;
        if gate.cancelled() {
            return Err(Error::new("cancelled"));
        }
        if unsafe { GetTickCount64() } >= gate.preparation_deadline() {
            return Err(Error::new("compositionTimeout"));
        }
        let (sender, commands) = mpsc::sync_channel(4);
        let retirement = Retirement::new(task.clone(), turn);
        let connection = Arc::new(Connection {
            sender,
            retirement: retirement.clone(),
        });
        let weak_connection = Arc::downgrade(&connection);
        let (ready, result) = oneshot::channel();
        std::thread::Builder::new()
            .name("sayweave-composition-sta".into())
            .spawn(move || {
                // Declared before Host: unwinding drops all apartment-owned
                // resources before the terminal failure becomes observable.
                let mut completion = WorkerCompletion::new(retirement);
                let began = Instant::now();
                let diagnostic_task = task.clone();
                let mut host = match Host::open(task, turn, anchor, gate) {
                    Ok(host) => host,
                    Err(error) => {
                        log::info!(target:"sayweave", "operation=draft_admission task_id={} turn={} result=rejected error={} elapsed_ms={}", diagnostic_task,turn,error.code,began.elapsed().as_millis());
                        let _ = ready.send(Err(error));
                        completion.finish(Err(Error::new("compositionWorker")), "openFailed");
                        return;
                    }
                };
                log::info!(target:"sayweave", "operation=draft_admission task_id={} turn={} result=ready elapsed_ms={}", diagnostic_task,turn,began.elapsed().as_millis());
                if ready.send(Ok(())).is_err() {
                    let _ = host.retire(&mut completion, "admissionAbandoned");
                    return;
                }
                loop {
                    if completion.requested() {
                        let _ = host.retire(&mut completion, "closeRequested");
                        return;
                    }
                    pump();
                    match commands.recv_timeout(Duration::from_millis(5)) {
                        Ok(Command::Block(turn,anchor,command,text,permit,reply))=>{
                            let result=host.block(turn,anchor,command,&text,&permit,None);
                            let _=reply.send(result);
                        },
                        Ok(Command::Acquire(turn,command,permit,deadline,reply))=>{
                            let result=host.block(turn,None,command,"",&permit,Some(deadline));
                            let _=reply.send(result);
                        },
                        Ok(Command::Observe(sink,reply))=>{let ready=host.observe(sink);let _=reply.send(ready);},
                        Ok(Command::Replace(turn,text,raw,permit,reply))=>{
                            let result=if turn==host.turn {host.update_guarded(&text,permit.as_deref())}else{Err(WriteFailure::not_attempted("staleSession"))};
                            if result.is_ok(){host.remember_tail(raw);}
                            let _=reply.send(result);
                        },
                        Ok(Command::ReplaceTracked(turn,text,raw,id,permit,reply))=>{
                            let result=if turn==host.turn {
                                host.update_scoped(&text,Some(&permit),Some(id))
                                    .and_then(|snapshot|snapshot.ok_or_else(||WriteFailure::uncertain(Error::new("compositionPartial"))))
                            }else{Err(WriteFailure::not_attempted("staleSession"))};
                            if result.is_ok() || result.as_ref().err().is_some_and(|e|e.mutation==Mutation::Changed&&e.tracking_failed){host.remember_tail(raw);}
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
                            delivery.record_cleanup(host.retire(&mut completion,"finished").is_err());
                            let _=reply.send(delivery);return;
                        },
                        Err(mpsc::RecvTimeoutError::Disconnected)=>{let _=host.retire(&mut completion,"disconnected");return;},
                        Err(mpsc::RecvTimeoutError::Timeout)=>{
                            if let Some(at)=host.retained_at {
                                let alive=host.poll_observer();
                                if !alive || at.elapsed()>=Duration::from_secs(60){let _=host.retire(&mut completion,if alive{"observationExpired"}else{"observationEnded"});return;}
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
        self.replace_guarded(sequence, text, raw, None)
            .await
            .map_err(|e| e.error)
    }
    pub(crate) async fn replace_guarded(
        &self,
        sequence: u64,
        text: String,
        raw: Option<String>,
        permit: Option<Arc<AtomicBool>>,
    ) -> Result<(), WriteFailure> {
        if self.closed.load(Ordering::Acquire) {
            return Err(WriteFailure::not_attempted("staleSession"));
        }
        if sequence == 0 || sequence <= self.source_sequence.fetch_max(sequence, Ordering::AcqRel) {
            return Err(WriteFailure::not_attempted("staleSnapshot"));
        }
        let (reply, result) = oneshot::channel();
        self.connection
            .sender
            .try_send(Command::Replace(self.turn, text, raw, permit, reply))
            .map_err(|_| WriteFailure::not_attempted("compositionWorker"))?;
        result
            .await
            .map_err(|_| WriteFailure::uncertain(Error::new("compositionWorker")))?
    }
    pub(crate) async fn replace_tracked(
        &self,
        sequence: u64,
        text: String,
        raw: Option<String>,
        id: String,
        permit: Arc<AtomicBool>,
    ) -> Result<BlockSnapshot, WriteFailure> {
        if self.closed.load(Ordering::Acquire) {
            return Err(WriteFailure::not_attempted("staleSession"));
        }
        if sequence == 0 || sequence <= self.source_sequence.fetch_max(sequence, Ordering::AcqRel) {
            return Err(WriteFailure::not_attempted("staleSnapshot"));
        }
        let (reply, result) = oneshot::channel();
        self.connection
            .sender
            .try_send(Command::ReplaceTracked(
                self.turn, text, raw, id, permit, reply,
            ))
            .map_err(|_| WriteFailure::not_attempted("compositionWorker"))?;
        result
            .await
            .map_err(|_| WriteFailure::uncertain(Error::new("compositionWorker")))?
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
    revoking_primary: Option<(u64, u64, u64)>,
    block: Option<BlockSnapshot>,
    supports_observation: bool,
    abandoned_optional: Vec<(u64, u64, u64)>,
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
    engine: EngineBinding,
    operation: u64,
    revision: u64,
    range_epoch: u64,
    transport_attempted: bool,
    retained_ack: Option<ReceiptKey>,
    receipt_inbox: VecDeque<Vec<u8>>,
    acknowledged_receipts: VecDeque<ReceiptKey>,
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
    fn block(
        &mut self,
        turn: u64,
        anchor: Option<Anchor>,
        command: BlockCommand,
        text: &str,
        permit: &AtomicBool,
        deadline: Option<u64>,
    ) -> Result<Reply, Error> {
        if let Some(anchor) = anchor {
            if self.range_open
                || turn < self.turn
                || command.action != BlockAction::Read
                || anchor.process != self.anchor.process
                || anchor.lifetime != self.anchor.lifetime
                || anchor.editor != self.anchor.editor
                || anchor.window != self.anchor.window
                || anchor.capture_id != self.anchor.capture_id
            {
                return Err(Error::new("staleSession"));
            }
            // Consume genuine editor changes under the old observation identity
            // before a new source turn or a model replacement can supersede it.
            self.stop_observation();
            self.anchor = anchor;
            self.turn = turn;
        }
        if turn != self.turn {
            return Err(Error::new("staleSession"));
        }
        let action = command.action;
        let reply = self.exchange_block_reply(
            Action::Inspect,
            text,
            None,
            matches!(action, BlockAction::Read | BlockAction::Release),
            Some(permit),
            Some((command, deadline)),
        )?;
        if reply.applied {
            if action == BlockAction::Release {
                self.block = None;
            } else {
                self.block = reply.block.clone();
            }
            if action == BlockAction::Apply {
                self.text = text.into();
                self.visible = true;
            }
            if action == BlockAction::Acquire {
                self.text = reply
                    .block
                    .as_ref()
                    .ok_or_else(|| Error::new("compositionProtocol"))?
                    .text
                    .clone();
                self.range_open = true;
                self.visible = true;
                self.failure = None;
                self.settled_empty_revision = None;
            }
        }
        Ok(reply)
    }
    fn retire(
        mut self,
        completion: &mut WorkerCompletion,
        reason: &'static str,
    ) -> Result<(), Error> {
        let result = self.close();
        drop(self);
        completion.finish(result.clone(), reason);
        result
    }
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
            if value.engine != self.engine {
                continue;
            }
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
        let (pid, thread) = editor_endpoint(&anchor)?;
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
            engine: EngineBinding::default(),
            operation: 0,
            revision: 0,
            range_epoch: 0,
            revoking_primary: None,
            block: None,
            transport_attempted: false,
            retained_ack: None,
            receipt_inbox: VecDeque::new(),
            acknowledged_receipts: VecDeque::new(),
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
        host.prepare_engine()?;
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
        // prepare_engine already verified the actual proxy and executing core.
        super::native::composition_begin_target(&host.task, &host.anchor)?;
        if let Err(error) = host.exchange(Action::Begin, "", false) {
            if error.code == "targetReadonlyContext" {
                // This is the only refused Begin that can admit recording
                // without a target. Check activation/pipe cleanup explicitly;
                // Drop alone cannot report a failed profile release.
                host.close()?;
            }
            return Err(error);
        }
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
            || anchor.editor != self.anchor.editor
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
    fn prepare_engine(&mut self) -> Result<(), Error> {
        let deadline = self.gate.preparation_deadline();
        let mut operation = 0;
        let expected = crate::installer::package_hash(crate::text_service::core_payload());
        loop {
            target_ready(&self.task, &self.anchor, &self.gate)?;
            let now = unsafe { GetTickCount64() };
            if now >= deadline {
                return Err(Error::new("compositionTimeout"));
            }
            operation += 1;
            let request = ControlRequest {
                control_protocol: 1,
                capability: self.capability.clone(),
                task: self.task.clone(),
                turn: self.turn,
                operation,
                deadline_tick: deadline.min(now + 15_000),
                action: ControlAction::PrepareEngine,
                expected_core: expected.clone(),
                receipt: None,
            };
            self.pipe
                .send(&serde_json::to_vec(&request).map_err(|_| Error::new("compositionProtocol"))?)
                .map_err(|_| Error::new("compositionRuntimeUnavailable"))?;
            let reply = loop {
                pump();
                target_ready(&self.task, &self.anchor, &self.gate)?;
                if let Some(frame) = self
                    .pipe
                    .receive()
                    .map_err(|_| Error::new("compositionDisconnected"))?
                {
                    let reply: ControlReply = serde_json::from_slice(&frame)
                        .map_err(|_| Error::new("compositionTargetVersion"))?;
                    if reply.control_protocol != 1
                        || reply.task != self.task
                        || reply.turn != self.turn
                        || reply.operation != operation
                    {
                        return Err(Error::new("compositionProtocol"));
                    }
                    break reply;
                }
                if unsafe { GetTickCount64() } >= deadline {
                    return Err(Error::new("compositionTimeout"));
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            if !reply.ready {
                if reply.code == "engineBusy" {
                    std::thread::sleep(Duration::from_millis(25));
                    continue;
                }
                log::info!(target:"sayweave","operation=engine_prepare task_id={} result=rejected reason={}",self.task,reply.code);
                return Err(Error::new("compositionRuntimeUnavailable"));
            }
            let binding = reply
                .engine
                .ok_or_else(|| Error::new("compositionProtocol"))?;
            let runtime = reply
                .runtime
                .ok_or_else(|| Error::new("compositionProtocol"))?;
            let (process, thread) = editor_endpoint(&self.anchor)?;
            if !binding.valid()
                || runtime.engine != binding
                || runtime.protocol != sayweave_text_service::wire::PROTOCOL
                || runtime.process != self.anchor.editor.process
                || runtime.thread != thread
            {
                return Err(Error::new("compositionTargetVersion"));
            }
            super::runtime_module::verify(process, &runtime)?;
            self.engine = binding;
            log::info!(target:"sayweave","operation=engine_prepare task_id={} result=ready process={} lifetime={} thread={} proxy_instance={} engine_generation={} core_hash={}",self.task,process,self.anchor.editor.lifetime,thread,self.engine.proxy_instance,self.engine.engine_generation,expected);
            return Ok(());
        }
    }
    fn receipt_request(
        &self,
        action: ControlAction,
        key: ReceiptKey,
        deadline: u64,
    ) -> ControlRequest {
        ControlRequest {
            control_protocol: 1,
            capability: self.capability.clone(),
            task: key.task.clone(),
            turn: key.turn,
            operation: key.operation,
            deadline_tick: deadline,
            action,
            expected_core: crate::installer::package_hash(crate::text_service::core_payload()),
            receipt: Some(key),
        }
    }
    fn recover_receipt(&mut self, request: &Request) -> Result<Vec<u8>, Error> {
        // Read-only recovery has a short bound. It never retries the original edit.
        let deadline = unsafe { GetTickCount64() } + 500;
        let key = ReceiptKey {
            engine: request.engine.clone(),
            task: request.task.clone(),
            turn: request.turn,
            operation: request.operation,
            revision: request.revision,
            sha256: None,
        };
        let control = self.receipt_request(ControlAction::ReadRetired, key.clone(), deadline);
        self.pipe = Pipe(HANDLE::default());
        let (pid, thread) =
            editor_endpoint(&self.anchor).map_err(|_| Error::new("compositionDisconnected"))?;
        loop {
            if unsafe { GetTickCount64() } >= deadline {
                return Err(Error::new("compositionDisconnected"));
            }
            pump();
            if let Ok(pipe) = Pipe::client(pid, thread) {
                self.pipe = pipe;
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        self.pipe
            .send(&serde_json::to_vec(&control).map_err(|_| Error::new("compositionProtocol"))?)
            .map_err(|_| Error::new("compositionDisconnected"))?;
        loop {
            pump();
            if let Some(bytes) = self
                .pipe
                .receive()
                .map_err(|_| Error::new("compositionDisconnected"))?
            {
                let receipt = ReceiptKey::from_frame(&bytes)
                    .ok_or_else(|| Error::new("compositionDisconnected"))?;
                if !key.matches(&receipt) {
                    return Err(Error::new("compositionProtocol"));
                }
                self.retained_ack = Some(receipt);
                return Ok(bytes);
            }
            if unsafe { GetTickCount64() } >= deadline {
                return Err(Error::new("compositionDisconnected"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn receive_business(&mut self) -> Result<Option<Vec<u8>>, windows::core::Error> {
        if let Some(frame) = self.receipt_inbox.pop_front() {
            return Ok(Some(frame));
        }
        loop {
            let Some(frame) = self.pipe.receive()? else {
                return Ok(None);
            };
            let value: serde_json::Value = serde_json::from_slice(&frame)
                .map_err(|_| windows::core::Error::from_hresult(E_INVALIDARG))?;
            if value.get("control_protocol").is_none() {
                return Ok(Some(frame));
            }
            let reply: ControlReply = serde_json::from_value(value)
                .map_err(|_| windows::core::Error::from_hresult(E_INVALIDARG))?;
            let known = self
                .acknowledged_receipts
                .iter()
                .chain(self.retained_ack.iter())
                .any(|key| {
                    reply.control_protocol == 1
                        && reply.task == key.task
                        && reply.turn == key.turn
                        && reply.operation == key.operation
                        && reply.engine.as_ref() == Some(&key.engine)
                        && reply.ready
                        && reply.code == "receiptAcknowledged"
                });
            if !known {
                return Err(windows::core::Error::from_hresult(E_ACCESSDENIED));
            }
        }
    }
    fn acknowledge_receipt(&mut self) -> Result<(), Error> {
        let Some(key) = self.retained_ack.clone() else {
            return Ok(());
        };
        let deadline = unsafe { GetTickCount64() } + 500;
        let request =
            self.receipt_request(ControlAction::AcknowledgeRetired, key.clone(), deadline);
        self.pipe
            .send(&serde_json::to_vec(&request).map_err(|_| Error::new("compositionProtocol"))?)
            .map_err(|_| Error::new("compositionDisconnected"))?;
        loop {
            pump();
            if let Some(bytes) = self
                .pipe
                .receive()
                .map_err(|_| Error::new("compositionDisconnected"))?
            {
                let value: serde_json::Value = serde_json::from_slice(&bytes)
                    .map_err(|_| Error::new("compositionProtocol"))?;
                if value.get("control_protocol").is_none() {
                    let reply: Reply = serde_json::from_value(value)
                        .map_err(|_| Error::new("compositionProtocol"))?;
                    let identity = ReceiptKey::from_frame(&bytes)
                        .ok_or_else(|| Error::new("compositionProtocol"))?;
                    if key.matches(&identity) {
                        continue;
                    } // Duplicate of the receipt already assigned to its caller.
                    if !pending_business_receipt(
                        &reply,
                        &self.engine,
                        &self.task,
                        self.turn,
                        (self.operation, self.revision),
                        &self.abandoned_optional,
                    ) && self.revoking_primary
                        != Some((reply.turn, reply.operation, reply.revision))
                        || self.receipt_inbox.len() >= 4
                    {
                        return Err(Error::new("compositionProtocol"));
                    }
                    self.receipt_inbox.push_back(bytes);
                    continue;
                }
                let reply: ControlReply =
                    serde_json::from_value(value).map_err(|_| Error::new("compositionProtocol"))?;
                let matches = |old: &ReceiptKey| {
                    reply.control_protocol == 1
                        && reply.task == old.task
                        && reply.turn == old.turn
                        && reply.operation == old.operation
                        && reply.engine.as_ref() == Some(&old.engine)
                        && reply.ready
                        && reply.code == "receiptAcknowledged"
                };
                if !matches(&key) {
                    if self.acknowledged_receipts.iter().any(matches) {
                        continue;
                    }
                    return Err(Error::new("compositionProtocol"));
                }
                if self.acknowledged_receipts.len() >= 16 {
                    self.acknowledged_receipts.pop_front();
                }
                self.acknowledged_receipts.push_back(key.clone());
                self.retained_ack = None;
                return Ok(());
            }
            if unsafe { GetTickCount64() } >= deadline {
                return Err(Error::new("compositionDisconnected"));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
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
        self.exchange_block_reply(action, text, prefix, cleanup, permission, None)
    }
    fn exchange_block_reply(
        &mut self,
        action: Action,
        text: &str,
        prefix: Option<Prefix>,
        cleanup: bool,
        permission: Option<&AtomicBool>,
        block: Option<(BlockCommand, Option<u64>)>,
    ) -> Result<Reply, Error> {
        let (block, deadline) = block.map_or((None, None), |(command, deadline)| {
            (Some(command), deadline)
        });
        self.transport_attempted = false;
        self.acknowledge_receipt()?;
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
        if matches!(action, Action::Update | Action::Settle)
            || block
                .as_ref()
                .is_some_and(|b| b.action == BlockAction::Apply)
        {
            self.revision += 1;
        }
        let request = Request {
            revoke: None,
            block: block.clone(),
            engine: self.engine.clone(),
            protocol: sayweave_text_service::wire::PROTOCOL,
            capability: self.capability.clone(),
            task: self.task.clone(),
            turn: self.turn,
            operation: self.operation,
            revision: self.revision,
            deadline_tick: deadline
                .unwrap_or(u64::MAX)
                .min(unsafe { GetTickCount64() } + 2000),
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
        let mut revocation: Option<Request> = None;
        let mut revoked = false;
        let mut primary: Option<Reply> = None;
        loop {
            if primary.is_some() && (revocation.is_none() || revoked) {
                self.revoking_primary = None;
                return Ok(primary.take().unwrap());
            }
            pump();
            let frame = match self.receive_business() {
                Ok(frame) => frame,
                Err(error) => {
                    if matches!(error.code(), E_ACCESSDENIED | E_INVALIDARG) {
                        return Err(Error::new("compositionProtocol"));
                    }
                    log::info!(target:"sayweave", "operation=draft_transport task_id={} turn={} action={:?} phase=receive hresult={:08x}", self.task,self.turn,action,error.code().0 as u32);
                    Some(self.recover_receipt(&request)?)
                }
            };
            if let Some(frame) = frame {
                let reply: Reply = serde_json::from_slice(&frame)
                    .map_err(|_| Error::new("compositionProtocol"))?;
                if reply.engine != self.engine || reply.task != self.task {
                    return Err(Error::new("compositionProtocol"));
                }
                // Only a timed-out optional read may finish after its caller
                // abandoned it. Its exact receipt must not poison the next
                // write. Unknown/core mismatches remain protocol failures.
                if discard_optional_reply(
                    &mut self.abandoned_optional,
                    reply.turn,
                    reply.operation,
                    reply.revision,
                ) {
                    self.retained_ack = ReceiptKey::from_frame(&frame);
                    self.acknowledge_receipt()?;
                    continue;
                }
                if reply.turn != self.turn {
                    return Err(Error::new("compositionProtocol"));
                }
                if revocation
                    .as_ref()
                    .is_some_and(|r| r.operation == reply.operation && r.revision == reply.revision)
                {
                    self.retained_ack = ReceiptKey::from_frame(&frame);
                    self.acknowledge_receipt()?;
                    if !reply.applied {
                        return Err(Error::new("compositionProtocol"));
                    }
                    revoked = true;
                    continue;
                }
                if reply.operation != request.operation || reply.revision != request.revision {
                    return Err(Error::new("compositionProtocol"));
                }
                self.retained_ack = ReceiptKey::from_frame(&frame);
                // Exact receipt is parsed and assigned to this request before acknowledgement.
                if self.retained_ack.is_some()
                    && let Err(error) = self.acknowledge_receipt()
                {
                    log::info!(target:"sayweave", "operation=retired_receipt_ack task_id={} result=pending error={}", self.task,error.code);
                }
                if action == Action::Inspect && block.is_none() {
                    self.supports_observation = reply
                        .edit_observation
                        .as_ref()
                        .is_some_and(|v| v.capability == 2);
                    let (pid, thread) = editor_endpoint(&self.anchor)?;
                    if !reply.compatible_runtime(pid, thread) {
                        return Err(Error::new("compositionTargetVersion"));
                    }
                    log::info!(target:"sayweave", "operation=draft_runtime task_id={} turn={} result=ready protocol={} process={} thread={}", self.task,self.turn,sayweave_text_service::wire::PROTOCOL,pid,thread);
                }
                if reply.mutation == Mutation::Changed {
                    self.visible = true;
                }
                if reply.applied {
                    self.range_epoch = reply.range_epoch;
                }
                log::info!(target:"sayweave", "operation=composition_edit task_id={} turn={} revision={} action={:?} applied={} code={} mutation={:?} elapsed_ms={}", self.task, self.turn, request.revision, action, reply.applied, reply.code, reply.mutation, at.elapsed().as_millis());
                primary = Some(reply);
                continue;
            }
            if revocation.is_none()
                && primary.is_none()
                && permission.is_some_and(|p| !p.load(Ordering::Acquire))
            {
                self.operation += 1;
                let revoke = Request {
                    revoke: Some(Revoke {
                        operation: request.operation,
                        revision: request.revision,
                    }),
                    block: None,
                    engine: self.engine.clone(),
                    protocol: request.protocol,
                    capability: self.capability.clone(),
                    task: self.task.clone(),
                    turn: self.turn,
                    operation: self.operation,
                    revision: request.revision,
                    deadline_tick: unsafe { GetTickCount64() } + 2000,
                    action: Action::Revoke,
                    text: String::new(),
                    prefix: None,
                };
                self.revoking_primary = Some((request.turn, request.operation, request.revision));
                self.pipe
                    .send(
                        &serde_json::to_vec(&revoke)
                            .map_err(|_| Error::new("compositionProtocol"))?,
                    )
                    .map_err(|_| Error::new("compositionDisconnected"))?;
                revocation = Some(revoke);
            }
            if at.elapsed() >= Duration::from_secs(2) {
                if let Some(reply) = primary.take() {
                    if let Some(revoke) = revocation {
                        if self.abandoned_optional.len() >= 16 {
                            return Err(Error::new("compositionProtocol"));
                        }
                        self.abandoned_optional.push((
                            revoke.turn,
                            revoke.operation,
                            revoke.revision,
                        ));
                    }
                    self.revoking_primary = None;
                    return Ok(reply);
                }
                if optional {
                    self.abandoned_optional.push((
                        request.turn,
                        request.operation,
                        request.revision,
                    ));
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
        self.update_guarded(text, None).map_err(|e| e.error)
    }
    fn update_guarded(
        &mut self,
        text: &str,
        permit: Option<&AtomicBool>,
    ) -> Result<(), WriteFailure> {
        self.update_scoped(text, permit, None).map(|_| ())
    }
    fn update_scoped(
        &mut self,
        text: &str,
        permit: Option<&AtomicBool>,
        block_id: Option<String>,
    ) -> Result<Option<BlockSnapshot>, WriteFailure> {
        if permit.is_some_and(|p| !p.load(Ordering::Acquire)) {
            return Err(WriteFailure::not_attempted("staleSnapshot"));
        }
        if text == self.text && block_id.is_none() {
            return Ok(None);
        }
        self.settled_empty_revision = None;
        let block = block_id.as_ref().map(|id| {
            (
                BlockCommand {
                    id: id.clone(),
                    action: BlockAction::Bind,
                    epoch: 0,
                    version: 0,
                    lease: 0,
                    expected: String::new(),
                    selection_start: 0,
                    selection_end: 0,
                },
                None,
            )
        });
        let result = self
            .exchange_block_reply(Action::Update, text, None, false, permit, block)
            .map_err(|error| {
                if self.transport_attempted {
                    WriteFailure::uncertain(error)
                } else {
                    WriteFailure {
                        error,
                        mutation: Mutation::NotAttempted,
                        tracking_failed: false,
                        scope_unavailable: false,
                    }
                }
            })
            .and_then(|r| update_receipt(r, text, block_id.as_deref()));
        if result.is_ok()
            || result
                .as_ref()
                .err()
                .is_some_and(|e| e.mutation == Mutation::Changed && e.tracking_failed)
        {
            self.text = text.into();
            if let Ok(Some(snapshot)) = &result {
                log::info!(target:"sayweave", "operation=draft_block_bound task_id={} turn={} block={} epoch={}", self.task,self.turn,snapshot.id,snapshot.epoch);
            }
        }
        if result.as_ref().err().is_some_and(|e| {
            !e.scope_unavailable
                && !matches!(
                    e.error.code.as_str(),
                    "staleSnapshot" | "editing" | "inputComposition" | "selectionMapping"
                )
        }) {
            self.failure = result.as_ref().err().map(|e| e.error.code.clone());
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
        let mut unwritten_retired = false;
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
        } else if !self.attempted && !self.visible && self.text.is_empty() && text.is_empty() {
            // Retire a prepared range without claiming a text commit. No focused
            // editability/readback is required because no write has been attempted.
            cancelled = !self.gate.commit();
            let released = self.release_empty_write_lease();
            unwritten_retired = released.is_ok();
            cleanup_failed = released.is_err();
            if cancelled {
                released.and(Err(Error::new("cancelled")))
            } else {
                released
            }
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
        let committed =
            result.is_ok() && mode == Finish::Commit && !already_settled && !unwritten_retired;
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
        let receipt = self.acknowledge_receipt();
        let block = if let Some(snapshot) = self.block.clone() {
            let command = BlockCommand {
                id: snapshot.id,
                action: BlockAction::Release,
                epoch: snapshot.epoch,
                version: snapshot.version,
                lease: snapshot.lease,
                expected: String::new(),
                selection_start: 0,
                selection_end: 0,
            };
            self.block(self.turn, None, command, "", &AtomicBool::new(true), None)
                .and_then(|r| {
                    if r.applied {
                        Ok(())
                    } else {
                        Err(Error::new("compositionCleanup"))
                    }
                })
        } else {
            Ok(())
        };
        self.stop_observation();
        self.sink = None;
        let closed = self
            .profile
            .close()
            .map_err(|_| Error::new("compositionCleanup"));
        receipt.and(block).and(closed)
    }
}
fn pending_business_receipt(
    reply: &Reply,
    engine: &EngineBinding,
    task: &str,
    turn: u64,
    current: (u64, u64),
    abandoned: &[(u64, u64, u64)],
) -> bool {
    reply.engine == *engine
        && reply.task == task
        && (reply.turn == turn && (reply.operation, reply.revision) == current
            || abandoned.contains(&(reply.turn, reply.operation, reply.revision)))
}
fn discard_optional_reply(
    abandoned: &mut Vec<(u64, u64, u64)>,
    turn: u64,
    operation: u64,
    revision: u64,
) -> bool {
    if let Some(index) = abandoned
        .iter()
        .position(|value| *value == (turn, operation, revision))
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
    // Native diagnostic metadata follows the stable business code.
    let code = code.split_whitespace().next().unwrap_or(code);
    if code.starts_with("edit:selectionMapping:") {
        return "selectionMapping";
    }
    if [
        "edit:expected:",
        "edit:selection:",
        "edit:lease:",
        "edit:updateLease:",
    ]
    .iter()
    .any(|prefix| code.starts_with(prefix))
    {
        return "editing";
    }
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
fn editor_endpoint(anchor: &Anchor) -> Result<(u32, u32), Error> {
    super::native::editor_endpoint(anchor)
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
    fn write_effect_and_tracking_refusal_are_independent_receipt_facts() {
        let reply = |phase: &str, mutation: Mutation| -> Reply {
            serde_json::from_value(serde_json::json!({
                "engine":EngineBinding { proxy_instance: uuid::Uuid::new_v4().to_string(), engine_generation: 1 },
                "task":"receipt-test","turn":1,"operation":1,"revision":1,
                "applied":false,"code":phase,"mutation":mutation
            }))
            .unwrap()
        };
        let written = update_receipt(
            reply("edit:writeTracking:80070005", Mutation::Changed),
            "正文",
            Some("block"),
        )
        .unwrap_err();
        assert_eq!(written.mutation, Mutation::Changed);
        assert!(written.tracking_failed);
        assert!(
            !written.scope_unavailable,
            "security refusals cannot grant a fresh range"
        );
        let mut malformed = reply("applied", Mutation::Changed);
        malformed.applied = true;
        let malformed = update_receipt(malformed, "正文", Some("block")).unwrap_err();
        assert_eq!(malformed.mutation, Mutation::Changed);
        assert!(malformed.tracking_failed);
        assert!(
            !malformed.scope_unavailable,
            "a missing snapshot is a protocol fault"
        );
        let rejected = update_receipt(
            reply("edit:updateLease:80070057", Mutation::NotAttempted),
            "正文",
            Some("block"),
        )
        .unwrap_err();
        assert_eq!(rejected.mutation, Mutation::NotAttempted);
        assert!(!rejected.tracking_failed);
        assert_eq!(rejected.error.code, "editing");
        let uncertain = update_receipt(
            reply("edit:updateText:80070057", Mutation::Uncertain),
            "正文",
            Some("block"),
        )
        .unwrap_err();
        assert_eq!(uncertain.mutation, Mutation::Uncertain);
        assert!(!uncertain.tracking_failed);
    }
    #[test]
    fn block_refusals_keep_wait_boundary_and_uncertain_effect_separate() {
        let mut reply: Reply = serde_json::from_value(serde_json::json!({
            "engine":{"proxy_instance":uuid::Uuid::new_v4().to_string(),"engine_generation":1},
            "task":uuid::Uuid::new_v4().to_string(),"turn":1,"operation":1,"revision":1,
            "applied":false,"code":"blockAwaitingRepresentation:80040500","mutation":"notAttempted"
        }))
        .unwrap();
        for phase in [
            "blockAwaitingRepresentation",
            "blockInputMethodBusy",
            "blockSnapshotChanged",
        ] {
            reply.code = format!("{phase}:80040500:diagnostic");
            assert_eq!(block_refusal(&reply), BlockRefusal::AwaitingProof);
        }
        reply.code = "blockScopeInvalid:800401fd".into();
        assert_eq!(block_refusal(&reply), BlockRefusal::Invalidated);
        reply.code = "blockSnapshotChanged:80040500".into();
        reply.mutation = Mutation::Uncertain;
        assert_eq!(block_refusal(&reply), BlockRefusal::Uncertain);
    }

    #[test]
    fn unavailable_old_range_is_distinct_from_a_changed_target() {
        let mut reply: Reply = serde_json::from_value(serde_json::json!({
            "engine":{"proxy_instance":uuid::Uuid::new_v4().to_string(),"engine_generation":1},
            "task":"scope","turn":1,"operation":1,"revision":1,
            "applied":false,"code":"blockEditEmpty:80040200","mutation":"notAttempted"
        }))
        .unwrap();
        for code in [
            "blockUnavailable:blockEditEmpty:80040200",
            "blockUnavailable:blockProtectedAnchor:80040200",
        ] {
            reply.code = code.into();
            assert_ne!(block_refusal(&reply), BlockRefusal::Invalidated, "{code}");
        }
        reply.code = "draftOwnershipTerminated:80070057".into();
        let failure = update_receipt(reply.clone(), "周三", None).unwrap_err();
        assert!(failure.scope_unavailable);
        assert_eq!(failure.mutation, Mutation::NotAttempted);
        reply.mutation = Mutation::Uncertain;
        assert!(
            !update_receipt(reply, "周三", None)
                .unwrap_err()
                .scope_unavailable
        );
    }
    #[test]
    fn ack_wait_preserves_current_write_and_late_optional_identity() {
        let engine = EngineBinding {
            proxy_instance: uuid::Uuid::new_v4().to_string(),
            engine_generation: 1,
        };
        let task = uuid::Uuid::new_v4().to_string();
        let mut reply: Reply =
            serde_json::from_value(serde_json::json!({"engine":engine,"task":task,"turn":1,
            "operation":11,"revision":3,"applied":true,"code":"ok","mutation":"changed"}))
            .unwrap();
        assert!(pending_business_receipt(
            &reply,
            &engine,
            &task,
            1,
            (11, 3),
            &[(1, 10, 2)]
        ));
        reply.operation = 10;
        reply.revision = 2;
        assert!(pending_business_receipt(
            &reply,
            &engine,
            &task,
            1,
            (11, 3),
            &[(1, 10, 2)]
        ));
        reply.operation = 12;
        assert!(!pending_business_receipt(
            &reply,
            &engine,
            &task,
            1,
            (11, 3),
            &[(1, 10, 2)]
        ));
        reply.operation = 11;
        reply.revision = 3;
        reply.engine.engine_generation += 1;
        assert!(!pending_business_receipt(
            &reply,
            &engine,
            &task,
            1,
            (11, 3),
            &[(1, 10, 2)]
        ));
    }

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
        assert_eq!(
            reply_error(&format!(
                "editLease:{:08x} nativeContext=focus/carrierStatus",
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
            assert_eq!(
                reply_error(&format!(
                    "{phase}:{locked} nativeContext=focus/documentStatus"
                )),
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
        let mut abandoned = vec![(1, 2, 0)];
        let frames = [(2, 0), (3, 1)];
        let mut accepted = vec![];
        for (operation, revision) in frames {
            if !discard_optional_reply(&mut abandoned, 1, operation, revision) {
                assert_eq!((operation, revision), (3, 1));
                accepted.push((operation, revision));
            }
        }
        assert_eq!(accepted, vec![(3, 1)]);
        assert!(abandoned.is_empty());
        // A duplicate or a wrong revision is not silently swallowed.
        assert!(!discard_optional_reply(&mut abandoned, 1, 2, 0));
        abandoned.push((1, 4, 1));
        assert!(!discard_optional_reply(&mut abandoned, 2, 4, 1));
        assert!(!discard_optional_reply(&mut abandoned, 1, 4, 2));
        assert_eq!(abandoned, vec![(1, 4, 1)]);
    }
}
