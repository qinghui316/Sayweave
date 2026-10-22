//! Current-block capability, independent of optional language-learning watches.
//! All ranges and subscriptions stay on the owning apartment.
use super::*;
#[path = "block_boundary.rs"]
mod boundary;
use boundary::BoundaryProof;
#[path = "block_scope.rs"]
mod scope;
use scope::BlockScope;
#[path = "block_content.rs"]
mod content;
#[path = "block_evidence.rs"]
mod evidence;

pub(super) struct Track {
    pub context: ITfContext,
    client: u32,
    range: ITfRange,
    field: BlockScope,
    task: String,
    data: Rc<RefCell<Data>>,
    cookie: u32,
    manager_source: ITfSource,
    manager_cookie: u32,
    notifications: Rc<TextNotifications>,
    boundary: Option<Rc<BoundaryProof>>,
    paragraph: Rc<RefCell<Option<Rc<initial_selection::ParagraphProbe>>>>,
    projection: Rc<RefCell<Option<Rc<BoundaryProof>>>>,
    selection_sync: Rc<RefCell<Option<Rc<SelectionSync>>>>,
    origin: Option<Rc<initial_selection::ParagraphOrigin>>,
    source_follow: RefCell<Option<SourceFollow>>,
    first_source: RefCell<Option<Rc<FirstSourceReceipt>>>,
    first_source_state: Cell<FirstReceiptState>,
    first_source_events: RefCell<Vec<&'static str>>,
    first_source_polled: Cell<bool>,
    first_source_note: Cell<&'static str>,
    first_source_stage: Cell<&'static str>,
    first_host_source: Cell<(&'static str, &'static str)>,
    first_source_read_result: Cell<Option<(u32, Option<u32>, bool)>>,
    #[cfg(feature = "acceptance")]
    first_source_result: RefCell<Option<serde_json::Value>>,
    _lease: Lease,
}
#[derive(Clone)]
struct SourceFollow {
    source: Rc<initial_selection::ParagraphProbe>,
    predecessor: u64,
    version: u64,
    text: String,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum FirstReceiptState {
    Unseeded,
    Unpaired,
    Claimed,
    ReadonlyCommitted,
    Retired,
}
pub(super) struct FirstSourceReceipt {
    request: Request,
    connection: u64,
    generation: u64,
    epoch: u64,
    // The live State retains this exact interface. Epoch/generation fence its
    // lifetime; no COM reference is released after the final publication CAS.
    composition: usize,
    block_epoch: u64,
    version: u64,
    lease: u64,
    position: RangePosition,
    text: String,
    // Captured by bind_owned only after this Update's successful mutation and
    // native readback. A later same-text field cannot manufacture this origin.
    origin: Rc<initial_selection::ParagraphOrigin>,
}
impl FirstSourceReceipt {
    pub(super) fn proves_origin(
        &self,
        origin: &initial_selection::ParagraphOrigin,
        pending: &(u64, String),
        version: u64,
    ) -> bool {
        std::ptr::eq(self.origin.as_ref(), origin)
            && self.request.action == Action::Update
            && self.request.operation == pending.0
            && self.text == pending.1
            && !self.text.is_empty()
            && self.version == version
    }
    fn current(&self, shared: &Shared, track: &Rc<Track>) -> bool {
        self.current_at(shared, track, self.generation, false, self.lease)
    }
    fn current_at(
        &self,
        shared: &Shared,
        track: &Rc<Track>,
        generation: u64,
        cleanup: bool,
        lease: u64,
    ) -> bool {
        let s = shared.borrow();
        let d = track.data.borrow();
        matches!(
            track.first_source_state.get(),
            FirstReceiptState::Unpaired | FirstReceiptState::Claimed
        ) && s.active
            && s.connected
            && s.cleanup == cleanup
            && !s.key_failure
            && s.generation == generation
            && s.connection == self.connection
            && s.epoch == self.epoch
            && s.composition
                .as_ref()
                .is_some_and(|value| value.as_raw() as usize == self.composition)
            && s.context.as_ref() == Some(&track.context)
            && s.version.owned != cleanup
            && s.version.task == self.request.task
            && s.version.turn == self.request.turn
            && s.version.operation == self.request.operation
            && s.version.revision == self.request.revision
            && s.pending.is_none()
            && s.expected == self.text
            && s.block.as_ref().is_some_and(|v| Rc::ptr_eq(v, track))
            && d.valid
            && !d.writing
            && d.origin_clean
            && d.notification_depth == 0
            && d.epoch == self.block_epoch
            && d.version == self.version
            && d.lease == lease
            && d.operation == self.request.operation
            && d.position == self.position
            && d.text == self.text
            && track.paragraph.borrow().is_none()
            && track.source_follow.borrow().is_none()
            && track.projection.borrow().is_none()
            && track.selection_sync.borrow().is_none()
    }
}
// A single local claim owns an unsettled receipt. Attempting the optional READ
// is separate from settling it; only a typed unpublished-text result can return
// the claim to this same Track. Claims and settlement perform no provider call.
pub(super) struct FirstSourceClaim {
    track: Rc<Track>,
    receipt: Rc<FirstSourceReceipt>,
}
pub(super) fn claim_first_source(shared: &Shared) -> Option<FirstSourceClaim> {
    let track = shared.borrow().block.clone()?;
    take_first_source(shared, &track, "claimed.handoff")
}
fn take_first_source(
    shared: &Shared,
    track: &Rc<Track>,
    note: &'static str,
) -> Option<FirstSourceClaim> {
    if track.first_source_state.get() != FirstReceiptState::Unpaired {
        return None;
    }
    let receipt = track.first_source.borrow().clone()?;
    if !receipt.current(shared, track) {
        retire_first_source(track, "superseded");
        return None;
    }
    let taken = track.first_source.borrow_mut().take()?;
    debug_assert!(Rc::ptr_eq(&receipt, &taken));
    track.first_source_state.set(FirstReceiptState::Claimed);
    track.first_source_note.set(note);
    first_source_event(track, note);
    Some(FirstSourceClaim {
        track: track.clone(),
        receipt,
    })
}
fn retire_first_source(track: &Track, note: &'static str) {
    if matches!(
        track.first_source_state.get(),
        FirstReceiptState::Unpaired | FirstReceiptState::Claimed
    ) {
        track.first_source_state.set(FirstReceiptState::Retired);
        track.first_source_note.set(note);
        first_source_event(track, note);
        track.first_source.borrow_mut().take();
    }
}
fn first_source_event(track: &Track, event: &'static str) {
    let mut events = track.first_source_events.borrow_mut();
    if events.len() < 12 {
        events.push(event);
    }
}
impl Drop for FirstSourceClaim {
    fn drop(&mut self) {
        if self.track.first_source_state.get() == FirstReceiptState::Claimed {
            retire_first_source(&self.track, "unsettled");
        }
    }
}
impl FirstSourceClaim {
    pub(super) fn after_revoke(&self, shared: &Shared) -> bool {
        self.receipt.lease.checked_add(1).is_some_and(|lease| {
            self.receipt
                .current_at(shared, &self.track, self.receipt.generation, false, lease)
        })
    }
    pub(super) fn bind(self, generation: u64, cleanup_id: u64) -> Rc<FirstSourceHandoff> {
        Rc::new(FirstSourceHandoff {
            claim: self,
            cleanup: Some((generation, cleanup_id)),
        })
    }
}
pub(super) struct FirstSourceHandoff {
    claim: FirstSourceClaim,
    cleanup: Option<(u64, u64)>,
}
impl FirstSourceHandoff {
    fn current(&self, shared: &Shared) -> bool {
        let Some((generation, cleanup_id)) = self.cleanup else {
            return self.claim.receipt.current(shared, &self.claim.track);
        };
        shared.borrow().cleanup_id == cleanup_id
            && self
                .claim
                .receipt
                .lease
                .checked_add(1)
                .is_some_and(|lease| {
                    self.claim.receipt.current_at(
                        shared,
                        &self.claim.track,
                        generation,
                        true,
                        lease,
                    )
                })
    }
}
// A host callback already owns its native edit cookie. Unlike a keyboard
// handoff it has not revoked a lease or entered cleanup; retain that exact
// active receipt fence rather than inventing either transition.
pub(super) fn claim_first_host_source(shared: &Shared) -> Option<Rc<FirstSourceHandoff>> {
    let track = {
        let state = shared.borrow();
        if state.cleanup {
            return None;
        }
        state.block.clone()?
    };
    let available = track.first_source.borrow().is_some();
    let claim = take_first_source(shared, &track, "claimed.host");
    track.first_host_source.set((
        if claim.is_some() {
            "claimed"
        } else if available {
            "stale"
        } else {
            "missing"
        },
        "entry",
    ));
    claim.map(|claim| {
        Rc::new(FirstSourceHandoff {
            claim,
            cleanup: None,
        })
    })
}
#[implement(ITfEditSession)]
struct FirstSourceEdit {
    shared: Shared,
    track: Rc<Track>,
    receipt: Rc<FirstSourceReceipt>,
    proof: Rc<RefCell<Option<initial_selection::ParagraphProbe>>>,
    notification: Rc<Cell<Option<u64>>>,
    entered: Rc<Cell<bool>>,
    outcome: Rc<Cell<SourceAttempt>>,
    _lease: Lease,
}
impl ITfEditSession_Impl for FirstSourceEdit_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        let _callback = Callback::enter();
        self.entered.set(true);
        let (shared, track, receipt) = (
            self.shared.clone(),
            self.track.clone(),
            self.receipt.clone(),
        );
        let guard: MutationGuard = Rc::new(move || {
            authorized(&shared, &receipt.request, receipt.connection)?;
            if !receipt.current(&shared, &track) {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let manager = shared
                .borrow()
                .manager
                .clone()
                .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
            let native = unsafe { edit_context::retained(&shared, &manager)? };
            if native.context != track.context || !native.focus_matches() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            native.permits()?;
            if !receipt.current(&shared, &track) {
                return Err(failure(TF_E_DISCONNECTED));
            }
            Ok(())
        });
        self.receipt.origin.handoff_stage("poll.authority");
        guard()?;
        let serial = self.track.data.borrow().notification;
        let outcome = unsafe {
            follow_client_write_inner(
                &self.shared,
                &self.track.context,
                ec,
                &guard,
                SourcePurpose::FirstReceipt,
                Some(&self.proof),
                Some(&self.receipt),
            )?
        };
        self.receipt.origin.handoff_stage("poll.final");
        if self.track.data.borrow().notification != serial
            || !self.receipt.current(&self.shared, &self.track)
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        self.notification.set(Some(serial));
        self.outcome.set(outcome);
        Ok(())
    }
}
// At most one optional synchronous READ. A specific unpublished-text result may
// leave its still-current receipt for the already-existing key handoff, but it
// never rearms this poll or changes ordinary input/Trail2.
pub(super) fn poll_first_source(shared: &Shared) {
    {
        let s = shared.borrow();
        if CALLBACKS.load(Ordering::Acquire) != 0
            || s.text_notifications.depth.get() != 0
            || s.pending.is_some()
            || first_entry_pending(&s)
        {
            return;
        }
    }
    read_first_source_once(shared, "attempted");
}
// KeyTrace only accounts for the first key transaction. Source pairing uses
// this existing optional poll or the legal native handoff, never the Up sink.
fn read_first_source_once(shared: &Shared, entry: &'static str) {
    let track = shared.borrow().block.clone();
    let Some(track) = track else {
        return;
    };
    if track.first_source_polled.replace(true) {
        return;
    }
    let Some(claim) = take_first_source(shared, &track, entry) else {
        return;
    };
    let receipt = claim.receipt.clone();
    let trace = receipt.origin.source_call(
        &receipt.request.task,
        receipt.request.operation,
        receipt.request.revision,
        "poll",
    );
    let proof = Rc::new(RefCell::new(None));
    let notification = Rc::new(Cell::new(None));
    let entered = Rc::new(Cell::new(false));
    let outcome = Rc::new(Cell::new(SourceAttempt::Unchanged));
    let edit: ITfEditSession = FirstSourceEdit {
        shared: shared.clone(),
        track: track.clone(),
        receipt: receipt.clone(),
        proof: proof.clone(),
        notification: notification.clone(),
        entered: entered.clone(),
        outcome: outcome.clone(),
        _lease: Lease::new(),
    }
    .into();
    #[cfg(feature = "acceptance")]
    let read_started = unsafe { GetTickCount64() };
    let result = unsafe {
        track
            .context
            .RequestEditSession(track.client, &edit, TF_ES_SYNC | TF_ES_READ)
    };
    #[cfg(feature = "acceptance")]
    let read_finished = unsafe { GetTickCount64() };
    drop(edit);
    track.first_source_read_result.set(Some((
        result
            .as_ref()
            .err()
            .map_or(0, |error| error.code().0 as u32),
        result.as_ref().ok().map(|hr| hr.0 as u32),
        entered.get(),
    )));
    let candidate = proof.borrow_mut().take();
    let trace_error = result
        .as_ref()
        .err()
        .map(|error| error.code())
        .or_else(|| result.as_ref().ok().filter(|hr| hr.is_err()).copied());
    trace.finish(
        if trace_error.is_some() {
            "error"
        } else if !entered.get() {
            "notEntered"
        } else {
            source_outcome(outcome.get())
        },
        trace_error,
    );
    #[cfg(feature = "acceptance")]
    {
        let d = track.data.borrow();
        *track.first_source_result.borrow_mut() = Some(serde_json::json!({
            "tick":unsafe { GetTickCount64() },
            "readStarted":read_started,"readFinished":read_finished,"entered":entered.get(),
            "requestHr":result.as_ref().err().map(|e|e.code().0 as u32),
            "sessionHr":result.as_ref().ok().map(|hr|hr.0 as u32),
            "candidate":candidate.is_some(),"current":receipt.current(shared,&track),
            "notification":notification.get(),"currentNotification":d.notification,
            "receiptVersion":receipt.version,"version":d.version,"lease":d.lease,
            "origin":track.origin.as_ref().map(|origin|origin.diagnostic()),
        }));
    }
    drop(proof);
    settle_first_source(
        &claim,
        outcome.get(),
        candidate,
        true,
        "committed",
        Some(&trace),
        || {
            result.as_ref().is_ok_and(|hr| hr.is_ok())
                && authorized(shared, &receipt.request, receipt.connection).is_ok()
                && receipt.current(shared, &track)
                && notification.get() == Some(track.data.borrow().notification)
        },
    );
}

// The three callers supply their real final fence. All candidate COM holders
// from their query must have left before this pure, once-only publication CAS.
// A committed readonly source still grants no native write lease.
fn settle_first_source(
    claim: &FirstSourceClaim,
    outcome: SourceAttempt,
    candidate: Option<initial_selection::ParagraphProbe>,
    may_wait: bool,
    committed_note: &'static str,
    trace: Option<&initial_selection::SourceCall>,
    current: impl Fn() -> bool,
) -> bool {
    let track = &claim.track;
    let receipt = &claim.receipt;
    track.first_source_stage.set(trace.map_or_else(
        || {
            track
                .origin
                .as_ref()
                .map_or("noOrigin", |origin| origin.source_stage())
        },
        |trace| trace.stage_name(),
    ));
    if track.first_source_state.get() != FirstReceiptState::Claimed {
        if let Some(trace) = trace {
            trace.settlement("notClaimed");
        }
        return false;
    }
    first_source_event(
        track,
        match outcome {
            SourceAttempt::Committed => "outcome.committed",
            SourceAttempt::AwaitPublication => "outcome.awaitPublication",
            SourceAttempt::Unchanged => "outcome.unchanged",
        },
    );
    if outcome == SourceAttempt::AwaitPublication && candidate.is_none() && may_wait {
        if current()
            && track.origin.as_ref().is_some_and(|origin| {
                origin.awaiting_write(receipt.request.operation, &receipt.text)
            })
            && track.first_source.borrow().is_none()
        {
            *track.first_source.borrow_mut() = Some(receipt.clone());
            track.first_source_state.set(FirstReceiptState::Unpaired);
            track.first_source_stage.set("awaitHandoff");
            track.first_source_note.set("awaitHandoff");
            first_source_event(track, "awaitHandoff");
            if let Some(trace) = trace {
                trace.settlement("awaitHandoff");
            }
        } else {
            retire_first_source(track, "superseded");
            if let Some(trace) = trace {
                trace.settlement("superseded");
            }
        }
        return false;
    }
    if outcome == SourceAttempt::Committed
        && let Some(candidate) = candidate
        && let Ok(candidate) = candidate.commit_source(&receipt.text, receipt.version)
    {
        // Dropping an unaccepted local proof may itself reenter, so never
        // retire a newer owner on its behalf. Publication is pure after current.
        if current()
            && track.first_source_state.get() == FirstReceiptState::Claimed
            && track.first_source.borrow().is_none()
            && track.origin.as_ref().is_some_and(|origin| {
                origin.awaiting_write(receipt.request.operation, &receipt.text)
            })
        {
            *track.paragraph.borrow_mut() = Some(Rc::new(candidate));
            track
                .first_source_state
                .set(FirstReceiptState::ReadonlyCommitted);
            track.first_source_note.set(committed_note);
            first_source_event(track, committed_note);
            if let Some(trace) = trace {
                trace.settlement(committed_note);
            }
            return true;
        }
        drop(candidate);
    }
    retire_first_source(track, "unavailable.candidate");
    if let Some(trace) = trace {
        trace.settlement("retired");
    }
    false
}

pub(super) fn binding(shared: &Shared) -> Option<edit_context::Binding> {
    shared
        .borrow()
        .block
        .as_ref()
        .map(|track| track.field.binding())
}
struct Data {
    id: String,
    epoch: u64,
    version: u64,
    operation: u64,
    text: String,
    position: RangePosition,
    valid: bool,
    invalid_reason: String,
    focus_diagnostic: Option<serde_json::Value>,
    writable: bool,
    lease: u64,
    writing: bool,
    evidence: evidence::Record,
    semantic_boundary: Option<initial_selection::SemanticBoundary>,
    pending_proof: bool,
    paragraph_pending: bool,
    notification: u64,
    notification_depth: u32,
    origin_clean: bool,
}
struct SelectionSync {
    paragraph: Rc<initial_selection::ParagraphProbe>,
    _source_range: windows::Win32::UI::Accessibility::ITextRangeProvider,
}
struct EditNotification {
    data: Rc<RefCell<Data>>,
    serial: u64,
    version: u64,
    lease: Cell<u64>,
}
impl EditNotification {
    fn enter(data: &Rc<RefCell<Data>>) -> Self {
        let mut state = data.borrow_mut();
        state.notification += 1;
        state.notification_depth += 1;
        Self {
            data: data.clone(),
            serial: state.notification,
            version: state.version,
            lease: Cell::new(state.lease),
        }
    }
    fn current(&self) -> bool {
        let state = self.data.borrow();
        state.valid
            && state.notification == self.serial
            && state.version == self.version
            && state.lease == self.lease.get()
    }
    fn commit_outside(&self) -> bool {
        if !self.current() {
            return false;
        }
        // Called only after the boundary has finished all COM reads. This
        // lease revocation and the outside evidence commit have no reentry
        // point between them; a later superseding notification cannot undo it.
        let mut state = self.data.borrow_mut();
        state.writable = false;
        state.lease += 1;
        self.lease.set(state.lease);
        true
    }
}
impl Drop for EditNotification {
    fn drop(&mut self) {
        self.data.borrow_mut().notification_depth -= 1;
    }
}
impl Drop for Track {
    fn drop(&mut self) {
        self.data.borrow_mut().valid = false;
        self.notifications.retire(self.context.clone(), self.cookie);
        self.notifications
            .retire_source(self.manager_source.clone(), self.manager_cookie);
    }
}

// The composition and the independent tracking clone are different native
// ranges. Refresh from the proven writer, never from offsets or a text search.
pub(super) struct OwnedUpdate {
    track: Rc<Track>,
    version: u64,
    armed: std::cell::Cell<bool>,
}
impl Drop for OwnedUpdate {
    fn drop(&mut self) {
        if self.armed.get() {
            self.track.data.borrow_mut().writing = false;
        }
    }
}
pub(super) unsafe fn owned_update(
    shared: &Shared,
    context: &ITfContext,
    range: &ITfRange,
    ec: u32,
) -> windows::core::Result<Option<OwnedUpdate>> {
    let track = {
        let state = shared.borrow();
        let Some(track) = state.block.clone() else {
            return Ok(None);
        };
        if state.context.as_ref() != Some(context)
            || state.composition.is_none()
            || state.version.task != track.task
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        track
    };
    if !track.data.borrow().valid || track.data.borrow().writing {
        return Err(failure(TF_E_DISCONNECTED));
    }
    let matched = (|| unsafe {
        let mut phase = "blockOwnedBaseline".into();
        if let Some(boundary) = &track.boundary()
            && !boundary.matches(context, &track.range, ec, &mut phase)?
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let raw = track.data.borrow().text.clone();
        if track.range.CompareStart(ec, range, TF_ANCHOR_START)? != 0
            || track.range.CompareEnd(ec, range, TF_ANCHOR_END)? != 0
            || range_text(range, ec)? != raw
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        Ok::<_, windows::core::Error>(())
    })();
    let mut data = track.data.borrow_mut();
    if matched.is_err() || !data.valid {
        if data.valid {
            data.invalid_reason = "blockOwnedBaseline".into();
        }
        data.valid = false;
        data.writable = false;
        return Err(failure(TF_E_DISCONNECTED));
    }
    let version = data.version;
    drop(data);
    Ok(Some(OwnedUpdate {
        track,
        version,
        armed: std::cell::Cell::new(false),
    }))
}
impl OwnedUpdate {
    pub(super) fn arm(&self) -> bool {
        let mut data = self.track.data.borrow_mut();
        if !data.valid || data.version != self.version || data.writing {
            return false;
        }
        data.writing = true;
        data.writable = false;
        data.lease += 1;
        self.armed.set(true);
        true
    }
    #[cfg(feature = "acceptance")]
    pub(super) unsafe fn stale_fixture(&self, ec: u32) -> windows::core::Result<()> {
        unsafe {
            if block_faults::stale_tracking_range() {
                self.track.range.Collapse(ec, TF_ANCHOR_START)?;
            }
            Ok(())
        }
    }
    pub(super) unsafe fn follow(
        &self,
        context: &ITfContext,
        range: &ITfRange,
        ec: u32,
        authority: impl Fn() -> windows::core::Result<()>,
    ) -> windows::core::Result<()> {
        let notification = self.track.data.borrow().notification;
        let result = (|| unsafe {
            #[cfg(feature = "acceptance")]
            if block_faults::reject_owned_follow() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            authority()?;
            let valid = self.track.data.borrow().valid;
            if !valid || !self.track.field.matches()? {
                return Err(failure(TF_E_DISCONNECTED));
            }
            if let Some(boundary) = &self.track.boundary() {
                boundary.follow_owned_write(context, range, ec)?;
            }
            authority()?;
            self.track
                .range
                .ShiftStartToRange(ec, range, TF_ANCHOR_START)?;
            self.track.range.ShiftEndToRange(ec, range, TF_ANCHOR_END)?;
            let text = range_text(&self.track.range, ec)?;
            relative_selection(context, &self.track.range, ec)?;
            authority()?;
            if !self.track.field.matches()? {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let position = range_position(context, &self.track.range, ec)?;
            let mut data = self.track.data.borrow_mut();
            if !data.valid || data.notification != notification {
                return Err(failure(TF_E_DISCONNECTED));
            }
            if data.text != text {
                if data.origin_clean
                    && self.track.projection.borrow().is_none()
                    && let Some(source) = self.track.paragraph.borrow().clone()
                {
                    let pending = self.track.source_follow.borrow().clone();
                    let predecessor = if let Some(pending) = pending {
                        (Rc::ptr_eq(&pending.source, &source)
                            && pending.version == data.version
                            && pending.text == data.text
                            && source.committed_at(pending.predecessor))
                        .then_some(pending.predecessor)
                    } else {
                        source.committed_at(data.version).then_some(data.version)
                    };
                    // Losing optional provider continuity cannot turn a proven
                    // native write into a failed/unknown write. Its stale proof
                    // stays unusable at the version gate; never advance it.
                    if let Some(predecessor) = predecessor {
                        *self.track.source_follow.borrow_mut() = Some(SourceFollow {
                            source,
                            predecessor,
                            version: data.version + 1,
                            text: text.clone(),
                        });
                    }
                }
                data.text = text;
                data.version += 1;
            }
            data.position = position;
            Ok::<_, windows::core::Error>(())
        })();
        if result.is_err() {
            let mut data = self.track.data.borrow_mut();
            if data.valid {
                data.invalid_reason = "blockOwnedFollow".into();
            }
            data.valid = false;
            data.writable = false;
        }
        let error = result.as_ref().err().map(|e| e.code());
        unsafe {
            evidence::capture(
                context,
                &self.track.range,
                &self.track.field,
                &self.track.data,
                ec,
                ("ownedFollow", "blockOwnedFollow", error),
            );
        }
        // The writer has already changed text. A failed post-write guard must
        // reach the receipt as an uncertain/changed failure, never success.
        result?;
        if !self.track.data.borrow().valid || self.track.data.borrow().notification != notification
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        Ok(())
    }
}

pub(super) struct EndGuard {
    track: Rc<Track>,
    shared: Shared,
}
pub(super) fn end_guard(shared: &Shared) -> Option<EndGuard> {
    shared
        .borrow()
        .block
        .clone()
        .filter(|track| {
            track
                .boundary()
                .is_some_and(|boundary| boundary.has_protected())
                || track.projection.borrow().is_some()
        })
        .map(|track| EndGuard {
            track,
            shared: shared.clone(),
        })
}
impl EndGuard {
    pub(super) unsafe fn check(&self, ec: u32) -> windows::core::Result<()> {
        unsafe {
            let track = &self.track;
            let current = || {
                self.shared
                    .borrow()
                    .block
                    .as_ref()
                    .is_some_and(|active| Rc::ptr_eq(active, track))
            };
            let serial = track.data.borrow().notification;
            let mut phase = "blockProtectedEnd".into();
            if !current()
                || !track.data.borrow().valid
                || !track
                    .boundary()
                    .ok_or_else(|| failure(TF_E_DISCONNECTED))?
                    .matches(&track.context, &track.range, ec, &mut phase)?
                || !track.data.borrow().valid
                || track.data.borrow().notification != serial
                || !current()
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            Ok(())
        }
    }
}

unsafe fn relative_selection(
    context: &ITfContext,
    range: &ITfRange,
    ec: u32,
) -> windows::core::Result<(u32, u32)> {
    unsafe {
        let selected = selection(context, ec)?;
        if selected.CompareStart(ec, range, TF_ANCHOR_START)? < 0
            || selected.CompareEnd(ec, range, TF_ANCHOR_END)? > 0
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let prefix = range.Clone()?;
        prefix.ShiftEndToRange(ec, &selected, TF_ANCHOR_START)?;
        let start = range_text(&prefix, ec)?.encode_utf16().count() as u32;
        prefix.ShiftEndToRange(ec, &selected, TF_ANCHOR_END)?;
        let end = range_text(&prefix, ec)?.encode_utf16().count() as u32;
        Ok((start, end))
    }
}

// Runs inside the owning write lock, before its Update receipt can escape.
// Effect and original scope are separate facts supplied by the same writer.
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn bind_owned(
    shared: &Shared,
    context: &ITfContext,
    r: &Request,
    connection: u64,
    generation: u64,
    ec: u32,
    origin: Option<Rc<initial_selection::ParagraphOrigin>>,
    mutation: Mutation,
) -> windows::core::Result<BlockSnapshot> {
    unsafe {
        let epoch = shared.borrow().epoch;
        let manager = shared
            .borrow()
            .manager
            .clone()
            .ok_or_else(|| failure(E_FAIL))?;
        if edit_context::focused(shared, &manager)? != *context {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let mut track = Track::bind(shared, context, r, ec)?;
        track.origin = origin;
        let track = Rc::new(track);
        // Binding records an already-owned write, not an external
        // editor baseline. Its provenance is the active composition
        // and this writer's exact text/position map; UIA may still be
        // publishing the previous renderer snapshot here.
        let (composition, written) = {
            let state = shared.borrow();
            (
                state
                    .composition
                    .clone()
                    .ok_or_else(|| failure(TF_E_DISCONNECTED))?,
                state
                    .text
                    .clone()
                    .ok_or_else(|| failure(TF_E_DISCONNECTED))?,
            )
        };
        let owned = composition.GetRange()?;
        if owned.CompareStart(ec, &track.range, TF_ANCHOR_START)? != 0
            || owned.CompareEnd(ec, &track.range, TF_ANCHOR_END)? != 0
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let actual = range_text(&owned, ec)?;
        let (start, end) = relative_selection(context, &owned, ec)?;
        let start = written
            .position(&actual, start, false)
            .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
        let end = written
            .position(&actual, end, false)
            .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
        let snapshot = {
            let data = track.data.borrow();
            if !data.valid || data.text != actual {
                return Err(failure(TF_E_DISCONNECTED));
            }
            BlockSnapshot {
                id: data.id.clone(),
                epoch: data.epoch,
                version: data.version,
                lease: data.lease,
                text: written.semantic,
                selection_start: start,
                selection_end: end,
                writable: false,
            }
        };
        if edit_context::focused(shared, &manager)? != *context || !track.field.matches()? {
            return Err(failure(TF_E_DISCONNECTED));
        }
        edit_authorized(shared, r, connection, generation)?;
        let mut state = shared.borrow_mut();
        let tracked = track.data.borrow();
        if state.epoch != epoch
            || !state.version.owned
            || state.version.task != r.task
            || state.version.turn != r.turn
            || state.block.is_some()
            || state.composition.as_ref() != Some(&composition)
            || !tracked.valid
            || tracked.version != snapshot.version
            || tracked.lease != snapshot.lease
            || state.expected != tracked.text
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        drop(tracked);
        if mutation == Mutation::Changed
            && r.action == Action::Update
            && let Some(origin) = track
                .origin
                .as_ref()
                .filter(|origin| origin.awaiting_write(r.operation, &track.data.borrow().text))
        {
            let d = track.data.borrow();
            *track.first_source.borrow_mut() = Some(Rc::new(FirstSourceReceipt {
                request: r.clone(),
                connection,
                generation,
                epoch,
                composition: composition.as_raw() as usize,
                block_epoch: d.epoch,
                version: d.version,
                lease: d.lease,
                position: d.position,
                text: d.text.clone(),
                origin: origin.clone(),
            }));
            track.first_source_note.set("seeded");
            track.first_source_state.set(FirstReceiptState::Unpaired);
            first_source_event(&track, "seeded");
        }
        state.block = Some(track);
        Ok(snapshot)
    }
}

impl Track {
    pub(super) fn source_trace(&self) -> (&'static str, &'static str) {
        (self.first_source_note.get(), self.first_source_stage.get())
    }
    pub(super) fn host_source_trace(&self) -> (&'static str, &'static str) {
        self.first_host_source.get()
    }
    pub(super) fn receipt_events(&self) -> Vec<&'static str> {
        self.first_source_events.borrow().clone()
    }
    unsafe fn synchronize_selection(
        &self,
        ec: u32,
        phase: &mut String,
    ) -> windows::core::Result<()> {
        unsafe {
            let Some(pending) = self.selection_sync.borrow().clone() else {
                return Ok(());
            };
            let (id, epoch, operation, version, notification, lease, text, position) = {
                let data = self.data.borrow();
                if !data.valid || data.writing || data.notification_depth != 0 {
                    *phase = "blockSnapshotChanged".into();
                    return Err(failure(TF_E_LOCKED));
                }
                (
                    data.id.clone(),
                    data.epoch,
                    data.operation,
                    data.version,
                    data.notification,
                    data.lease,
                    data.text.clone(),
                    data.position,
                )
            };
            let current = || {
                let data = self.data.borrow();
                data.valid
                    && !data.writing
                    && !data.writable
                    && data.id == id
                    && data.epoch == epoch
                    && data.operation == operation
                    && data.version == version
                    && data.notification == notification
                    && data.lease == lease
                    && data.notification_depth == 0
                    && data.text == text
                    && data.position == position
            };
            *phase = "blockSelectionSyncScope".into();
            let proof = self
                .projection
                .borrow()
                .clone()
                .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
            if !self.field.matches()?
                || !proof.matches(&self.context, &self.range, ec, phase)?
                || range_position(&self.context, &self.range, ec)? != position
                || range_text(&self.range, ec)? != text
                || !current()
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let document = self.context.GetStart(ec)?;
            document.ShiftEndToRange(ec, &self.context.GetEnd(ec)?, TF_ANCHOR_END)?;
            let native = range_text(&document, ec)?;
            let mut read_phase = String::new();
            let observed = pending
                .paragraph
                .synchronize(&native, &mut read_phase)
                .inspect_err(|_| *phase = format!("blockSelectionSyncRejected:{read_phase}"))?;
            let selected = relative_selection(&self.context, &self.range, ec)?;
            let owner: ITfContextComposition = self.context.cast()?;
            let mut compositions = [None];
            let mut fetched = 0;
            owner
                .EnumCompositions()?
                .Next(&mut compositions, &mut fetched)?;
            let paired = observed.as_ref().is_some_and(|observed| {
                observed.text == text && observed.selected == [selected.0 as i32, selected.1 as i32]
            });
            #[cfg(feature = "acceptance")]
            pending.paragraph.record_handoff("selection.sync",serde_json::json!({
                "version":version,"notification":notification,"lease":lease,"paired":paired,
                "observed":observed,"nativeSelection":[selected.0,selected.1],"trackingCommitted":true,
            }));
            if fetched != 0 {
                *phase = "blockInputMethodBusy".into();
                return Err(failure(TF_E_LOCKED));
            }
            if !proof.matches(&self.context, &self.range, ec, phase)?
                || !self.field.matches()?
                || range_position(&self.context, &self.range, ec)? != position
                || range_text(&self.range, ec)? != text
                || relative_selection(&self.context, &self.range, ec)? != selected
                || !current()
            {
                *phase = "blockSnapshotChanged".into();
                return Err(failure(TF_E_LOCKED));
            }
            if !paired {
                *phase = "blockAwaitingSelectionSync".into();
                return Err(failure(TF_E_LOCKED));
            }
            // All COM work is complete. Clear only this exact pending proof;
            // no source admission, range movement or writer is performed here.
            let retired = {
                let mut slot = self.selection_sync.borrow_mut();
                if !current()
                    || !slot
                        .as_ref()
                        .is_some_and(|value| Rc::ptr_eq(value, &pending))
                {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                slot.take()
            };
            let source = self.paragraph.borrow_mut().take();
            drop(retired);
            drop(source);
            drop(pending);
            if !current() || self.selection_sync.borrow().is_some() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            Ok(())
        }
    }
    #[cfg(feature = "acceptance")]
    pub(super) fn origin_diagnostic(&self) -> serde_json::Value {
        let mut value = self
            .origin
            .as_ref()
            .map(|origin| origin.diagnostic())
            .unwrap_or(serde_json::Value::Null);
        let paragraph = self.paragraph.borrow().clone();
        if let Some(proof) = paragraph
            && let Some(object) = value.as_object_mut()
        {
            object.insert(
                "savedRangeComparisons".into(),
                proof.comparison_diagnostic(),
            );
        }
        if let Some(object) = value.as_object_mut() {
            object.insert("firstReceipt".into(), self.first_source_note.get().into());
            object.insert(
                "firstReceiptResult".into(),
                self.first_source_result.borrow().clone().into(),
            );
            let data = self.data.borrow();
            object.insert("tracking".into(),serde_json::json!({
                "id":data.id,"epoch":data.epoch,"version":data.version,"lease":data.lease,
                "notification":data.notification,"valid":data.valid,"writable":data.writable,
                "position":[data.position.start,data.position.end],
                "installed":self.projection.borrow().is_some(),"selectionPending":self.selection_sync.borrow().is_some(),
            }));
        }
        value
    }
    fn boundary(&self) -> Option<Rc<BoundaryProof>> {
        if let Some(proof) = self.projection.borrow().clone() {
            return Some(proof);
        }
        self.boundary.clone()
    }
    unsafe fn bind(
        shared: &Shared,
        context: &ITfContext,
        request: &Request,
        ec: u32,
    ) -> windows::core::Result<Self> {
        unsafe {
            let command = request
                .block
                .as_ref()
                .ok_or_else(|| failure(E_INVALIDARG))?;
            let (composition, expected, client, notifications) = {
                let state = shared.borrow();
                if state.context.as_ref() != Some(context)
                    || !state.version.owned
                    || state.version.task != request.task
                    || state.version.turn != request.turn
                    || state.block.is_some()
                {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                (
                    state
                        .composition
                        .clone()
                        .ok_or_else(|| failure(TF_E_DISCONNECTED))?,
                    state.expected.clone(),
                    state.client,
                    state.text_notifications.clone(),
                )
            };
            let range = composition.GetRange()?.Clone()?;
            if range_text(&range, ec)? != expected || expected.is_empty() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let manager = shared
                .borrow()
                .manager
                .clone()
                .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
            let field =
                BlockScope::capture(&manager, context, edit_context::retained(shared, &manager)?)?;
            range.SetGravity(ec, TF_GRAVITY_BACKWARD, TF_GRAVITY_FORWARD)?;
            // Unavailable/oversized outside evidence keeps the original strict
            // notification rule. It does not block ordinary composition input.
            let mut boundary_reason = "blockBoundaryUnavailable".into();
            let boundary = BoundaryProof::capture(context, &range, ec, &mut boundary_reason)
                .inspect_err(|error| {
                    #[cfg(feature = "acceptance")]
                    eprintln!(
                        "block boundary capture phase={boundary_reason} hr={:08x}",
                        error.code().0 as u32
                    );
                    #[cfg(not(feature = "acceptance"))]
                    let _ = error;
                })
                .ok()
                .map(Rc::new);
            let data = Rc::new(RefCell::new(Data {
                id: command.id.clone(),
                epoch: COMPOSITION_EPOCH.fetch_add(1, Ordering::AcqRel) + 1,
                version: 1,
                operation: request.operation,
                text: expected,
                position: range_position(context, &range, ec)?,
                valid: true,
                invalid_reason: "blockInvalid".into(),
                focus_diagnostic: None,
                writable: false,
                lease: 0,
                writing: false,
                evidence: evidence::Record::default(),
                semantic_boundary: None,
                pending_proof: false,
                paragraph_pending: false,
                notification: 0,
                notification_depth: 0,
                origin_clean: true,
            }));
            let paragraph = Rc::new(RefCell::new(None));
            let projection = Rc::new(RefCell::new(None));
            let selection_sync = Rc::new(RefCell::new(None));
            let sink: ITfTextEditSink = Sink {
                field: field.clone(),
                range: range.clone(),
                client,
                data: data.clone(),
                notifications: notifications.clone(),
                boundary: boundary.clone(),
                paragraph: paragraph.clone(),
                projection: projection.clone(),
                selection_sync: selection_sync.clone(),
                boundary_reason,
                _lease: Lease::new(),
            }
            .into();
            let source: ITfSource = context.cast()?;
            let manager = shared
                .borrow()
                .manager
                .clone()
                .ok_or_else(|| failure(E_FAIL))?;
            let manager_source: ITfSource = manager.cast()?;
            let focus: ITfThreadMgrEventSink = FocusSink {
                context: context.clone(),
                native: field.binding(),
                data: data.clone(),
                notifications: notifications.clone(),
                _lease: Lease::new(),
            }
            .into();
            let cookie = source.AdviseSink(&ITfTextEditSink::IID, &sink)?;
            let manager_cookie =
                match manager_source.AdviseSink(&ITfThreadMgrEventSink::IID, &focus) {
                    Ok(cookie) => cookie,
                    Err(error) => {
                        notifications.retire(context.clone(), cookie);
                        return Err(error);
                    }
                };
            let track = Self {
                context: context.clone(),
                client,
                range,
                origin: None,
                source_follow: RefCell::new(None),
                first_source: RefCell::new(None),
                first_source_state: Cell::new(FirstReceiptState::Unseeded),
                first_source_events: RefCell::new(Vec::new()),
                first_source_polled: Cell::new(false),
                first_source_note: Cell::new("unseeded"),
                first_source_stage: Cell::new("initial"),
                first_host_source: Cell::new(("notEntered", "initial")),
                first_source_read_result: Cell::new(None),
                #[cfg(feature = "acceptance")]
                first_source_result: RefCell::new(None),
                field,
                task: request.task.clone(),
                data,
                cookie,
                manager_source,
                manager_cookie,
                notifications,
                boundary,
                paragraph,
                projection,
                selection_sync,
                _lease: Lease::new(),
            };
            // Capture outside semantic evidence only while the original owned
            // range is known. A later failed read cannot invent this baseline.
            if let Ok((_, boundary)) =
                content::capture(context, &track.range, &track.field, None, ec, 1)
            {
                track.data.borrow_mut().semantic_boundary = Some(boundary);
            }
            evidence::capture(
                context,
                &track.range,
                &track.field,
                &track.data,
                ec,
                ("bind", "block", None),
            );
            Ok(track)
        }
    }

    unsafe fn verified_content(
        &self,
        ec: u32,
        phase: &mut String,
    ) -> windows::core::Result<content::VerifiedBlockContent> {
        unsafe {
            self.synchronize_selection(ec, phase)?;
            let (version, original) = {
                let data = self.data.borrow();
                (data.version, data.semantic_boundary.clone())
            };
            if let Some(boundary) = &self.boundary()
                && !boundary.anchors_match(&self.context, &self.range, ec, phase)?
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            // Native scope and edit continuity, not whole-field UIA equality,
            // authorize this exact inline text. The outside suffix is never
            // read into the baseline. Paragraph/object representations require
            // the separate semantic proof below; no character is trimmed.
            let native = range_text(&self.range, ec)?;
            let position = range_position(&self.context, &self.range, ec)?;
            let continuous = {
                let data = self.data.borrow();
                data.valid
                    && data.notification_depth == 0
                    && data.version == version
                    && data.text == native
                    && data.position == position
            };
            if !continuous {
                *phase = "blockSnapshotChanged".into();
                return Err(failure(TF_E_LOCKED));
            }
            if content::inline_identity(&native)
                && (self.boundary().is_some() || self.field.has_native_origin())
            {
                if let Some(boundary) = &self.boundary()
                    && !boundary.matches(&self.context, &self.range, ec, phase)?
                {
                    return Err(failure(TF_E_LOCKED));
                }
                let (selection_start, selection_end) =
                    relative_selection(&self.context, &self.range, ec)?;
                return Ok(content::VerifiedBlockContent {
                    semantic_text: native.clone(),
                    native_text: native,
                    native_end: position.end,
                    selection_start,
                    selection_end,
                    content_version: version,
                });
            }
            if let Some(original) = original {
                // UIA has no revision shared with TSF. Semantic agreement
                // cannot replace original native outside fingerprints.
                if let Some(boundary) = &self.boundary()
                    && !boundary.matches(&self.context, &self.range, ec, phase)?
                {
                    return Err(failure(TF_E_LOCKED));
                }
                *phase = "blockAwaitingRepresentation".into();
                let (verified, _) = content::capture(
                    &self.context,
                    &self.range,
                    &self.field,
                    Some(&original),
                    ec,
                    version,
                )
                .inspect_err(|error| {
                    if error.code() == TF_E_DISCONNECTED {
                        *phase = "blockScopeInvalid".into();
                        let mut data = self.data.borrow_mut();
                        data.valid = false;
                        data.writable = false;
                        data.invalid_reason.clone_from(phase);
                    }
                })?;
                if self.data.borrow().version != version || !self.data.borrow().valid {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                return Ok(verified);
            }
            // Native controls without a semantic provider retain strict raw
            // boundaries. Missing semantic evidence never relaxes a mismatch.
            if !self.field.native_identity_text() {
                *phase = "blockAwaitingRepresentation".into();
                return Err(failure(TF_E_LOCKED));
            }
            if let Some(boundary) = &self.boundary()
                && !boundary.matches(&self.context, &self.range, ec, phase)?
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let native_text = range_text(&self.range, ec)?;
            let (selection_start, selection_end) =
                relative_selection(&self.context, &self.range, ec)?;
            Ok(content::VerifiedBlockContent {
                semantic_text: native_text.clone(),
                native_text,
                native_end: range_position(&self.context, &self.range, ec)?.end,
                selection_start,
                selection_end,
                content_version: version,
            })
        }
    }

    // Explicit capability transition before producing a fresh snapshot. The
    // removed interval is retained as native protected content, not decoded.
    unsafe fn contract_unproven_tail(
        &self,
        ec: u32,
        phase: &mut String,
    ) -> windows::core::Result<()> {
        unsafe {
            *phase = "blockScopeContraction".into();
            let (version, lease, notification, native, position) = {
                let data = self.data.borrow();
                if !data.valid || data.writing || data.notification_depth != 0 {
                    return Err(failure(TF_E_LOCKED));
                }
                (
                    data.version,
                    data.lease,
                    data.notification,
                    data.text.clone(),
                    data.position,
                )
            };
            if self.field.native_identity_text() {
                return Err(failure(TF_E_LOCKED));
            }
            let owner: ITfContextComposition = self.context.cast()?;
            let mut compositions = [None];
            let mut fetched = 0;
            owner
                .EnumCompositions()?
                .Next(&mut compositions, &mut fetched)?;
            if fetched != 0 {
                *phase = "blockInputMethodBusy".into();
                return Err(failure(TF_E_LOCKED));
            }
            let units =
                content::protected_prefix_units(&native).ok_or_else(|| failure(TF_E_LOCKED))?;
            let boundary = self.boundary().ok_or_else(|| failure(TF_E_LOCKED))?;
            if !self.field.matches()?
                || range_text(&self.range, ec)? != native
                || range_position(&self.context, &self.range, ec)? != position
            {
                return Err(failure(TF_E_LOCKED));
            }
            let candidate = self.range.Clone()?;
            candidate.Collapse(ec, TF_ANCHOR_START)?;
            let mut moved = 0;
            candidate.ShiftEnd(ec, units, &mut moved, std::ptr::null())?;
            if moved != units {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let candidate_text = range_text(&candidate, ec)?;
            if !content::inline_identity(&candidate_text)
                || candidate_text.encode_utf16().count() != units as usize
                || !native.starts_with(&candidate_text)
            {
                return Err(failure(TF_E_LOCKED));
            }
            // Selection outside the candidate cannot be silently moved into it.
            relative_selection(&self.context, &candidate, ec)?;
            let interval =
                boundary.prepare_contraction(&self.context, &self.range, &candidate, ec, phase)?;
            if !boundary.matches(&self.context, &self.range, ec, phase)?
                || range_text(&self.range, ec)? != native
                || range_text(&candidate, ec)? != candidate_text
                || !self.field.matches()?
            {
                return Err(failure(TF_E_LOCKED));
            }
            relative_selection(&self.context, &candidate, ec)?;
            {
                let mut data = self.data.borrow_mut();
                if !data.valid
                    || data.version != version
                    || data.lease != lease
                    || data.notification != notification
                    || data.notification_depth != 0
                {
                    return Err(failure(TF_E_LOCKED));
                }
                // Fence reentrant callbacks until the new range and its guard
                // have both been installed. Partial failure stays invalid.
                data.valid = false;
                data.writable = false;
                data.lease += 1;
                data.invalid_reason = "blockScopeContraction".into();
            }
            self.range.ShiftEndToRange(ec, &candidate, TF_ANCHOR_END)?;
            boundary.commit_contraction(interval);
            if !boundary.matches(&self.context, &self.range, ec, phase)?
                || range_text(&self.range, ec)? != candidate_text
                || !self.field.matches()?
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let position = range_position(&self.context, &self.range, ec)?;
            relative_selection(&self.context, &self.range, ec)?;
            let mut data = self.data.borrow_mut();
            if data.version != version
                || data.lease != lease + 1
                || data.notification != notification
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            data.version += 1;
            data.text = candidate_text;
            data.position = position;
            data.pending_proof = false;
            data.paragraph_pending = false;
            data.valid = true;
            Ok(())
        }
    }

    fn snapshot_field(&self) -> windows::core::Result<()> {
        let before = {
            let data = self.data.borrow();
            (
                data.id.clone(),
                data.epoch,
                data.version,
                data.lease,
                data.notification,
                data.notification_depth,
            )
        };
        // This one scope query releases its temporary COM references before
        // the pure state fence. Errors and a stale focus flag retain their
        // original refusal semantics; only a proven identity change retires.
        match self.field.match_state()? {
            initial_selection::FocusMatch::Current => Ok(()),
            initial_selection::FocusMatch::Unfocused => Err(failure(TF_E_DISCONNECTED)),
            initial_selection::FocusMatch::DifferentIdentity => {
                let mut data = self.data.borrow_mut();
                let unchanged = before
                    == (
                        data.id.clone(),
                        data.epoch,
                        data.version,
                        data.lease,
                        data.notification,
                        data.notification_depth,
                    );
                let retire = data.valid && unchanged;
                if retire {
                    data.invalid_reason = "blockScopeInvalid".into();
                    data.valid = false;
                    data.writable = false;
                    data.lease += 1;
                }
                drop(data);
                if retire {
                    retire_first_source(self, "fieldChanged");
                }
                Err(failure(TF_E_DISCONNECTED))
            }
        }
    }

    unsafe fn snapshot(&self, ec: u32, phase: &mut String) -> windows::core::Result<BlockSnapshot> {
        unsafe {
            let mut version = self.data.borrow().version;
            if !self.data.borrow().valid {
                *phase = self.data.borrow().invalid_reason.clone();
                return Err(failure(TF_E_DISCONNECTED));
            }
            *phase = "blockSnapshotField".into();
            self.snapshot_field()?;
            *phase = "blockCarrierAccess".into();
            self.field.check_carrier(self.client).inspect_err(|error| {
                if error.code() == TF_E_LOCKED {
                    *phase = "blockInputMethodBusy".into();
                }
            })?;
            *phase = "blockSnapshotForeground".into();
            let foreground = GetForegroundWindow();
            *phase = "blockSnapshotView".into();
            let view = self.context.GetActiveView()?.GetWnd()?;
            // BlockScope already checks the local focus HWND and retained
            // native view. A missing optional view HWND does not turn the
            // outer application process into the editing process.
            if !view.0.is_null() && GetAncestor(view, GA_ROOT) != foreground {
                return Err(failure(TF_E_DISCONNECTED));
            }
            *phase = "blockSnapshotAccess".into();
            ensure_enabled_context(&self.context)?;
            if self.context.GetStatus()?.dwDynamicFlags & TF_SD_READONLY != 0
                || password(&self.context, &self.range, ec)?
            {
                return Err(failure(E_ACCESSDENIED));
            }
            if self.data.borrow().pending_proof && self.paragraph.borrow().is_some() {
                *phase = "blockAwaitingRepresentation".into();
                let attempt = (ProjectionProbe {
                    field: &self.field,
                    boundary: self.boundary(),
                    range: &self.range,
                    data: &self.data,
                    paragraph: &self.paragraph,
                    projection: &self.projection,
                    selection_sync: &self.selection_sync,
                })
                .probe_projection(&self.context, ec, phase);
                match attempt {
                    Ok(ProjectionAttempt::AwaitSnapshot) => return Err(failure(TF_E_LOCKED)),
                    Ok(_) => {}
                    Err(error) => {
                        if !phase.starts_with("blockProviderRangeRetired:") {
                            *phase = format!(
                                "blockParagraphProjection:{phase}:{:08x}",
                                error.code().0 as u32
                            );
                        }
                        let mut data = self.data.borrow_mut();
                        if data.pending_proof && self.projection.borrow().is_none() {
                            data.valid = false;
                            data.pending_proof = false;
                            data.paragraph_pending = false;
                            data.writable = false;
                            data.lease += 1;
                            data.invalid_reason.clone_from(phase);
                        }
                        return Err(failure(TF_E_DISCONNECTED));
                    }
                }
                version = self.data.borrow().version;
            }
            let verified = match self.verified_content(ec, phase) {
                Err(error)
                    if error.code() == TF_E_LOCKED && phase == "blockAwaitingRepresentation" =>
                {
                    self.contract_unproven_tail(ec, phase)
                        .inspect_err(|error| {
                            if error.code() == TF_E_LOCKED && phase != "blockInputMethodBusy" {
                                *phase = "blockAwaitingRepresentation".into();
                            }
                        })?;
                    version = self.data.borrow().version;
                    self.verified_content(ec, phase)?
                }
                value => value?,
            };
            let text = verified.native_text;
            let selection_start = verified.selection_start;
            let selection_end = verified.selection_end;
            *phase = "blockSnapshotField".into();
            self.snapshot_field()?;
            *phase = "blockSnapshotVersion".into();
            let mut data = self.data.borrow_mut();
            if !data.valid {
                *phase = data.invalid_reason.clone();
                return Err(failure(TF_E_DISCONNECTED));
            }
            if data.version != version || verified.semantic_text.is_empty() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            if data.text != text {
                data.text = text;
                data.version += 1;
                data.writable = false;
                data.lease += 1;
            }
            data.pending_proof = false;
            data.paragraph_pending = false;
            *phase = "block".into();
            Ok(BlockSnapshot {
                lease: data.lease,
                id: data.id.clone(),
                epoch: data.epoch,
                version: data.version,
                text: verified.semantic_text,
                selection_start,
                selection_end,
                writable: data.writable,
            })
        }
    }
}

#[implement(ITfTextEditSink)]
struct Sink {
    field: BlockScope,
    range: ITfRange,
    client: u32,
    data: Rc<RefCell<Data>>,
    notifications: Rc<TextNotifications>,
    boundary: Option<Rc<BoundaryProof>>,
    paragraph: Rc<RefCell<Option<Rc<initial_selection::ParagraphProbe>>>>,
    projection: Rc<RefCell<Option<Rc<BoundaryProof>>>>,
    selection_sync: Rc<RefCell<Option<Rc<SelectionSync>>>>,
    boundary_reason: String,
    _lease: Lease,
}
impl Sink {
    fn boundary(&self) -> Option<Rc<BoundaryProof>> {
        if let Some(proof) = self.projection.borrow().clone() {
            return Some(proof);
        }
        self.boundary.clone()
    }
}
#[derive(Clone, Copy)]
enum ProjectionAttempt {
    Unchanged,
    AwaitSnapshot,
    Applied,
}
struct ProjectionProbe<'a> {
    field: &'a BlockScope,
    boundary: Option<Rc<BoundaryProof>>,
    range: &'a ITfRange,
    data: &'a Rc<RefCell<Data>>,
    paragraph: &'a Rc<RefCell<Option<Rc<initial_selection::ParagraphProbe>>>>,
    projection: &'a Rc<RefCell<Option<Rc<BoundaryProof>>>>,
    selection_sync: &'a Rc<RefCell<Option<Rc<SelectionSync>>>>,
}
impl ProjectionProbe<'_> {
    unsafe fn probe_projection(
        &self,
        context: &ITfContext,
        ec: u32,
        failure_phase: &mut String,
    ) -> windows::core::Result<ProjectionAttempt> {
        unsafe {
            if self.projection.borrow().is_some() {
                return Ok(ProjectionAttempt::Unchanged);
            }
            let Some(paragraph) = self.paragraph.borrow().clone() else {
                return Ok(ProjectionAttempt::Unchanged);
            };
            let (id, epoch, operation, version, serial, lease, depth, previous, paragraph_pending) = {
                let data = self.data.borrow();
                (
                    data.id.clone(),
                    data.epoch,
                    data.operation,
                    data.version,
                    data.notification,
                    data.lease,
                    data.notification_depth,
                    data.position,
                    data.paragraph_pending,
                )
            };
            let current = || {
                let data = self.data.borrow();
                data.valid
                    && !data.writing
                    && data.id == id
                    && data.epoch == epoch
                    && data.operation == operation
                    && data.version == version
                    && data.notification == serial
                    && data.lease == lease
                    && data.notification_depth == depth
                    && data.paragraph_pending == paragraph_pending
                    && self
                        .paragraph
                        .borrow()
                        .as_ref()
                        .is_some_and(|source| Rc::ptr_eq(source, &paragraph))
            };
            let folded = range_position(context, self.range, ec)?;
            // Ordinary edits retain their native interval and continue through
            // the existing tracker. Only a receipt-proven translated,
            // collapsed interval can enter this handoff.
            if folded == previous {
                let selected = range_position(context, &selection(context, ec)?, ec)?;
                if selected.start >= previous.start && selected.end <= previous.end {
                    return Ok(ProjectionAttempt::Unchanged);
                }
                // The client can refresh its selection before collapsing the
                // old cache interval. Preserve only exact, committed readonly
                // provenance while those two representations catch up.
                let text = self.data.borrow().text.clone();
                let boundary = self
                    .boundary
                    .as_ref()
                    .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
                *failure_phase = "paragraph.awaitCoordinates".into();
                if previous.start >= previous.end
                    || range_text(self.range, ec)? != text
                    || !self.field.matches()?
                    || !boundary.matches(context, self.range, ec, failure_phase)?
                {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                paragraph.pending_coordinates(&text, version, failure_phase)?;
                let owner: ITfContextComposition = context.cast()?;
                let mut entries = [None];
                let mut count = 0;
                owner.EnumCompositions()?.Next(&mut entries, &mut count)?;
                if count != 0
                    || !boundary.matches(context, self.range, ec, failure_phase)?
                    || !self.field.matches()?
                    || range_text(self.range, ec)? != text
                    || range_position(context, self.range, ec)? != previous
                    || range_position(context, &selection(context, ec)?, ec)? != selected
                    || !current()
                    || !self
                        .paragraph
                        .borrow()
                        .as_ref()
                        .is_some_and(|p| Rc::ptr_eq(p, &paragraph))
                {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                return Ok(ProjectionAttempt::AwaitSnapshot);
            }
            // After this exact source entered a coordinate wait, later user
            // keys can extend the old native interval before the cache pairs.
            // Continue the SAME projection validation instead of returning to
            // ordinary tracking with that obsolete interval. Other proof waits
            // and first-time noncollapsed intervals retain the original gate.
            if folded.start != folded.end && current() {
                evidence::range_geometry(self.data, previous, folded, false);
            }
            if folded.start != folded.end && !paragraph_pending
                || previous.start == folded.start
                || previous.start >= previous.end
            {
                if current() {
                    evidence::range_geometry(self.data, previous, folded, false);
                }
                return Ok(ProjectionAttempt::Unchanged);
            }
            let document = context.GetStart(ec)?;
            document.ShiftEndToRange(ec, &context.GetEnd(ec)?, TF_ANCHOR_END)?;
            let native = range_text(&document, ec)?;
            #[cfg(feature = "acceptance")]
            diagnose_properties(context, ec, "folded");
            let mut projection_phase = String::new();
            let observed = paragraph.projection(&native, version, &mut projection_phase);
            #[cfg(feature = "acceptance")]
            eprintln!(
                "paragraph_projection_probe {}",
                serde_json::json!({"previous":[previous.start,previous.end],"folded":range_position(context,self.range,ec).ok().map(|v|[v.start,v.end]),"nativeUnits":native.encode_utf16().collect::<Vec<_>>(),"projection":observed.as_ref().ok(),"error":observed.as_ref().err().map(|e|format!("{:08x}",e.code().0 as u32))})
            );
            let eligible = || -> windows::core::Result<()> {
                if !self.field.matches()? {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                let owner: ITfContextComposition = context.cast()?;
                let mut entries = [None];
                let mut count = 0;
                owner.EnumCompositions()?.Next(&mut entries, &mut count)?;
                if count != 0 || !current() {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                Ok(())
            };
            eligible().inspect_err(|_| *failure_phase = "paragraph.nativeEligibility".into())?;
            #[cfg(feature = "acceptance")]
            if observed.is_err() {
                let fence = serde_json::json!({
                    "version":version,"notification":serial,"lease":lease,
                    "depth":depth,"current":current(),"phase":projection_phase,
                });
                eprintln!("paragraph_projection_refusal_fence {fence}");
                projection_phase = format!("{projection_phase}:fence={fence}");
            }
            let Some(observed) = observed.inspect_err(|_| *failure_phase = projection_phase)?
            else {
                #[cfg(feature = "acceptance")]
                paragraph.record_handoff(
                    "await.body",
                    serde_json::json!({
                        "version":version,"notification":serial,"lease":lease,
                        "depth":depth,"current":current(),"trackingCommitted":false,
                        "nativeUnits":native.encode_utf16().collect::<Vec<_>>(),
                    }),
                );
                return Ok(ProjectionAttempt::AwaitSnapshot);
            };
            let selected = range_position(context, &selection(context, ec)?, ec)?;
            if !self.field.matches()? || !current() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            eligible()?;
            if observed.selected != [selected.start, selected.end] {
                #[cfg(feature = "acceptance")]
                paragraph.record_handoff(
                    "await.selection",
                    serde_json::json!({
                        "version":version,"notification":serial,"lease":lease,
                        "depth":depth,"current":current(),"trackingCommitted":false,
                        "observed":observed,"nativeSelection":[selected.start,selected.end],
                    }),
                );
            }
            let candidate = context.GetStart(ec)?;
            let mut moved = 0;
            let units = i32::try_from(observed.text.encode_utf16().count())
                .map_err(|_| failure(E_INVALIDARG))?;
            candidate.ShiftEnd(ec, units, &mut moved, std::ptr::null())?;
            if moved != units
                || range_text(&candidate, ec)? != observed.text
                || password(context, &candidate, ec)?
            {
                return Err(failure(E_ACCESSDENIED));
            }
            candidate.SetGravity(ec, TF_GRAVITY_BACKWARD, TF_GRAVITY_FORWARD)?;
            let candidate_selection = relative_selection(context, &candidate, ec)?;
            if candidate_selection != (selected.start as u32, selected.end as u32) {
                *failure_phase = "paragraph.nativeSelectionChanged".into();
                return Err(failure(TF_E_DISCONNECTED));
            }
            let mut phase = "paragraphProjectionGuard".into();
            let proof = Rc::new(BoundaryProof::capture(context, &candidate, ec, &mut phase)?);
            let owner: ITfContextComposition = context.cast()?;
            let mut compositions = [None];
            let mut fetched = 0;
            owner
                .EnumCompositions()?
                .Next(&mut compositions, &mut fetched)?;
            if fetched != 0
                || paragraph
                    .projection(&range_text(&document, ec)?, version, failure_phase)?
                    .as_ref()
                    != Some(&observed)
                || !proof.matches(context, &candidate, ec, &mut phase)?
                || !self.field.matches()?
                || !current()
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let position = range_position(context, &candidate, ec)?;
            if relative_selection(context, &candidate, ec)? != candidate_selection || !current() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let pending = Rc::new(SelectionSync {
                paragraph: paragraph.clone(),
                _source_range: observed.range.clone(),
            });
            {
                let mut data = self.data.borrow_mut();
                data.valid = false;
                data.writable = false;
                data.lease += 1;
                data.invalid_reason = "paragraphProjectionInstalling".into();
            }
            self.range
                .ShiftStartToRange(ec, &candidate, TF_ANCHOR_START)?;
            self.range.ShiftEndToRange(ec, &candidate, TF_ANCHOR_END)?;
            if !proof.matches(context, self.range, ec, &mut phase)?
                || range_text(self.range, ec)? != observed.text
                || !self.field.matches()?
                || paragraph
                    .projection(&range_text(&document, ec)?, version, failure_phase)?
                    .as_ref()
                    != Some(&observed)
                || relative_selection(context, self.range, ec)? != candidate_selection
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let mut data = self.data.borrow_mut();
            if data.valid
                || data.writing
                || data.id != id
                || data.epoch != epoch
                || data.operation != operation
                || data.notification != serial
                || data.notification_depth != depth
                || data.version != version
                || data.lease != lease + 1
                || data.paragraph_pending != paragraph_pending
                || !self
                    .paragraph
                    .borrow()
                    .as_ref()
                    .is_some_and(|source| Rc::ptr_eq(source, &paragraph))
                || self.projection.borrow().is_some()
                || self.selection_sync.borrow().is_some()
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            *self.projection.borrow_mut() = Some(proof);
            *self.selection_sync.borrow_mut() = Some(pending);
            data.text = observed.text;
            data.position = position;
            data.version += 1;
            data.pending_proof = false;
            data.paragraph_pending = false;
            data.valid = true;
            data.invalid_reason = "block".into();
            #[cfg(feature = "acceptance")]
            let version = data.version;
            drop(data);
            #[cfg(feature = "acceptance")]
            paragraph.record_handoff("tracking.committed",serde_json::json!({
                "version":version,"notification":serial,"lease":lease+1,"trackingCommitted":true,"writable":false,
                "selectionPending":true,"position":[position.start,position.end],
            }));
            #[cfg(feature = "acceptance")]
            eprintln!(
                "paragraph_projection_applied range={},{} version={}",
                position.start, position.end, version
            );
            Ok(ProjectionAttempt::Applied)
        }
    }
}

impl ITfTextEditSink_Impl for Sink_Impl {
    fn OnEndEdit(
        &self,
        context: Ref<ITfContext>,
        ec: u32,
        record: Ref<ITfEditRecord>,
    ) -> windows::core::Result<()> {
        let _callback = Callback::enter();
        let _notification = TextNotification::enter(self.notifications.clone());
        // Even an invalid/transitioning owner must observe reentry; otherwise
        // a contraction could mistake a nested edit for an unchanged version.
        let notification = EditNotification::enter(&self.data);
        if !self.data.borrow().valid {
            return Ok(());
        }
        let writing = self.data.borrow().writing;
        if writing {
            // Intermediate geometry is not a user edit. The writer's own
            // guard and final readback remain authoritative; focus/context
            // sinks still invalidate the capability during this interval.
            return Ok(());
        }
        let mut paragraph_pending = false;
        let mut phase = "blockEditContext".into();
        let result = (|| unsafe {
            let context = context.as_ref().ok_or_else(|| failure(E_POINTER))?;
            // Own composition updates remain subject to native text proof; they
            // update tracking but must never become user-edit learning evidence.
            let own = context.InWriteSession(self.client)?.as_bool();
            if !own {
                self.data.borrow_mut().origin_clean = false;
            }
            if self.paragraph.borrow().is_some() && self.projection.borrow().is_none() {
                // The retained whole-field provider range is the source while
                // the old native interval is folded. A projection must not
                // hide an unreadable or out-of-context current edit record.
                phase = "blockParagraphEditRecord".into();
                let record = record.as_ref().ok_or_else(|| failure(E_POINTER))?;
                let _ = record.GetSelectionStatus()?;
                let document = context.GetStart(ec)?;
                document.ShiftEndToRange(ec, &context.GetEnd(ec)?, TF_ANCHOR_END)?;
                if !updates_within(record, &document, ec)? {
                    return Err(failure(TF_E_DISCONNECTED));
                }
            }
            match (ProjectionProbe {
                field: &self.field,
                boundary: self.boundary(),
                range: &self.range,
                data: &self.data,
                paragraph: &self.paragraph,
                projection: &self.projection,
                selection_sync: &self.selection_sync,
            })
            .probe_projection(context, ec, &mut phase)?
            {
                ProjectionAttempt::Unchanged => {}
                ProjectionAttempt::Applied => {
                    return Ok((
                        own,
                        self.data.borrow().text.clone(),
                        true,
                        self.data.borrow().position,
                        false,
                    ));
                }
                ProjectionAttempt::AwaitSnapshot => {
                    paragraph_pending = true;
                    phase = "blockAwaitingParagraph".into();
                    return Err(failure(TF_E_LOCKED));
                }
            }
            #[cfg(feature = "acceptance")]
            if block_faults::native_scope_only()
                && let Some(record) = record.as_ref()
            {
                let ranges = (|| {
                    let updates = record.GetTextAndPropertyUpdates(TF_GTP_INCL_TEXT, &[])?;
                    let mut values = Vec::new();
                    for _ in 0..64 {
                        let mut item = [None];
                        let mut count = 0;
                        updates.Next(&mut item, &mut count)?;
                        if count == 0 {
                            break;
                        }
                        let item = item[0].as_ref().ok_or_else(|| failure(E_FAIL))?;
                        values.push(serde_json::json!({
                            "start": item.CompareStart(ec,&self.range,TF_ANCHOR_START)?,
                            "end": item.CompareEnd(ec,&self.range,TF_ANCHOR_END)?,
                            "units": range_text(item,ec).ok().map(|v|v.encode_utf16().count()),
                            "position": range_position(context,item,ec).ok().map(|v|[v.start,v.end]),
                        }));
                    }
                    Ok::<_, windows::core::Error>(values)
                })();
                eprintln!(
                    "native-scope edit-record {}",
                    serde_json::json!({
                        "within":updates_within(record,&self.range,ec).ok(),
                        "selectionChanged":record.GetSelectionStatus().ok().map(|v|v.as_bool()),
                        "selection":relative_selection(context,&self.range,ec).ok(),
                        "scope":self.field.matches().ok(),
                        "updates":ranges.ok(),
                    })
                );
            }
            phase = "blockEditField".into();
            if !self.field.matches()? {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let record = record.as_ref().ok_or_else(|| failure(E_POINTER))?;
            let (previous, previous_position) = {
                let data = self.data.borrow();
                (data.text.clone(), data.position)
            };
            let text = range_text(&self.range, ec)?;
            let position = range_position(context, &self.range, ec)?;
            if let Some(boundary) = &self.boundary()
                && boundary.has_protected()
                && !boundary.protected_matches(context, &self.range, ec, &mut phase)?
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            phase = "blockEditRangeDrift".into();
            if position.start != previous_position.start
                || position.end - position.start != text.encode_utf16().count() as i32
            {
                if notification.current() {
                    evidence::range_geometry(&self.data, previous_position, position, true);
                }
                return Err(failure(TF_E_DISCONNECTED));
            }
            phase = "blockEditSelection".into();
            relative_selection(context, &self.range, ec)?;
            phase = "blockEditUpdates".into();
            let within = updates_within(record, &self.range, ec)?;
            #[cfg(feature = "acceptance")]
            let within = {
                let previous = self.data.borrow().text.clone();
                let changed = range_text(&self.range, ec)? != previous;
                let coarse = block_faults::coarse_notification(changed);
                within && !coarse
            };
            let mut disjoint = false;
            if !within {
                phase = self.boundary_reason.clone();
                let boundary = self.boundary().ok_or_else(|| failure(TF_E_DISCONNECTED))?;
                if !boundary.matches(context, &self.range, ec, &mut phase)? {
                    disjoint = text == previous
                        && position == previous_position
                        && boundary.observe_disjoint_suffix(
                            context,
                            &self.range,
                            record,
                            ec,
                            &mut phase,
                            || notification.commit_outside(),
                        )?;
                } else {
                    // Coarse notifications are covered by unchanged outside
                    // bytes; this does not adopt a changed outside snapshot.
                    disjoint = false;
                }
                if !disjoint && !boundary.matches(context, &self.range, ec, &mut phase)? {
                    return Err(failure(TF_E_DISCONNECTED));
                }
            }
            phase = "blockEditSelection".into();
            relative_selection(context, &self.range, ec)?;
            phase = "blockEditTextRead".into();
            if text.is_empty() {
                phase = "blockEditEmpty".into();
                return Err(failure(TF_E_DISCONNECTED));
            }
            phase = "blockEditSelectionStatus".into();
            let changed = record.GetSelectionStatus()?.as_bool();
            Ok((own, text, changed, position, disjoint))
        })();
        let error = result.as_ref().err().map(|e| e.code());
        if !notification.current() {
            // A nested edit/focus callback owns the newer truth. Never revive
            // an invalid range or publish the outer callback's stale sample.
            return Ok(());
        }
        let mut data = self.data.borrow_mut();
        match result {
            Ok((own, text, selection_changed, position, disjoint)) => {
                data.position = position;
                let text_changed = data.text != text;
                if data.text != text {
                    data.text = text;
                    data.version += 1;
                }
                if disjoint || !own && (text_changed || selection_changed) {
                    data.writable = false;
                    data.lease += 1;
                }
            }
            Err(_) => {
                if data.valid {
                    data.invalid_reason = phase.clone();
                }
                let representation_pending = paragraph_pending
                    || data.semantic_boundary.is_some()
                        && (phase.starts_with("blockSuffixText:")
                            || phase.starts_with("blockPrefixText:"));
                data.pending_proof = representation_pending;
                data.paragraph_pending = paragraph_pending;
                data.valid = representation_pending;
                data.writable = false;
                data.lease += 1;
            }
        }
        drop(data);
        if let Some(context) = context.as_ref() {
            unsafe {
                evidence::capture(
                    context,
                    &self.range,
                    &self.field,
                    &self.data,
                    ec,
                    ("onEndEdit", &phase, error),
                );
            }
        }
        // Tracking refusal must not fail the user's edit.
        Ok(())
    }
}

pub(super) fn release(shared: &Shared) {
    let old = shared.borrow_mut().block.take();
    if let Some(track) = &old {
        retire_first_source(track, "released");
    }
    drop(old);
}
pub(super) fn writing(shared: &Shared) -> bool {
    shared
        .borrow()
        .block
        .as_ref()
        .is_some_and(|t| t.data.borrow().writing)
}
pub(super) fn revoke(shared: &Shared) {
    if let Some(track) = shared.borrow().block.as_ref() {
        if track.first_source_state.get() == FirstReceiptState::Unpaired {
            retire_first_source(track, "leaseRevoked");
        }
        let mut data = track.data.borrow_mut();
        data.lease += 1;
        data.writable = false;
    }
}
fn original_point(shared: &Shared) -> Option<Rc<initial_selection::ParagraphOrigin>> {
    let state = shared.borrow();
    state
        .prepared
        .as_ref()
        .and_then(|p| p.origin.clone())
        .or_else(|| state.block.as_ref().and_then(|b| b.origin.clone()))
}
pub(super) unsafe fn observe_client_coordinates(
    shared: &Shared,
    context: &ITfContext,
    ec: u32,
    authority: &MutationGuard,
) {
    let _ = unsafe { follow_client_write(shared, context, ec, authority, SourcePurpose::Observe) };
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum SourcePurpose {
    Observe,
    Handoff,
    FirstHandoff,
    FirstHostEnd,
    HostEnd,
    OwnWrite(u64),
    FirstReceipt,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum SourceAttempt {
    Unchanged,
    AwaitPublication,
    Committed,
}
fn source_outcome(outcome: SourceAttempt) -> &'static str {
    match outcome {
        SourceAttempt::Unchanged => "unchanged",
        SourceAttempt::AwaitPublication => "awaitPublication",
        SourceAttempt::Committed => "candidate",
    }
}
unsafe fn follow_client_write(
    shared: &Shared,
    context: &ITfContext,
    ec: u32,
    authority: &MutationGuard,
    purpose: SourcePurpose,
) -> windows::core::Result<bool> {
    let origin = {
        let state = shared.borrow();
        state.block.as_ref().and_then(|track| track.origin.clone())
    };
    let Some(origin) = origin.filter(|origin| origin.pending_write().is_some()) else {
        return Ok(false);
    };
    let trace = {
        let state = shared.borrow();
        origin.source_call(
            &state.version.task,
            state.version.operation,
            state.version.revision,
            "follow",
        )
    };
    let result =
        unsafe { follow_client_write_inner(shared, context, ec, authority, purpose, None, None) };
    trace.finish(
        result
            .as_ref()
            .map_or("error", |outcome| source_outcome(*outcome)),
        result.as_ref().err().map(|error| error.code()),
    );
    if result.is_err() && purpose != SourcePurpose::FirstReceipt {
        origin.retire();
    }
    result.map(|outcome| outcome == SourceAttempt::Committed)
}
unsafe fn follow_client_write_inner(
    shared: &Shared,
    context: &ITfContext,
    ec: u32,
    authority: &MutationGuard,
    purpose: SourcePurpose,
    first: Option<&RefCell<Option<initial_selection::ParagraphProbe>>>,
    first_native: Option<&FirstSourceReceipt>,
) -> windows::core::Result<SourceAttempt> {
    unsafe {
        let handoff = matches!(
            purpose,
            SourcePurpose::Handoff | SourcePurpose::FirstHandoff
        );
        let first_receipt = matches!(
            purpose,
            SourcePurpose::FirstReceipt | SourcePurpose::FirstHandoff | SourcePurpose::FirstHostEnd
        );
        let (
            track,
            composition,
            generation,
            epoch,
            connection,
            operation,
            revision,
            cleanup_id,
            task,
            turn,
            pending_native,
        ) = {
            let s = shared.borrow();
            let rejected = if !s.active {
                Some("handoff.inactive")
            } else if !s.connected {
                Some("handoff.disconnected")
            } else if s.key_failure {
                Some("handoff.keyFailure")
            } else if s.cleanup != handoff {
                Some("handoff.cleanup")
            } else if s.version.owned == handoff {
                Some("handoff.ownership")
            } else if s.context.as_ref() != Some(context) {
                Some("handoff.context")
            } else {
                None
            };
            if let Some(reason) = rejected {
                if let Some(receipt) = first_native {
                    receipt.origin.handoff_stage(reason);
                }
                return Err(failure(TF_E_DISCONNECTED));
            }
            (
                s.block.clone().ok_or_else(|| failure(TF_E_DISCONNECTED))?,
                s.composition
                    .clone()
                    .ok_or_else(|| failure(TF_E_DISCONNECTED))?,
                s.generation,
                s.epoch,
                s.connection,
                s.version.operation,
                s.version.revision,
                s.cleanup_id,
                s.version.task.clone(),
                s.version.turn,
                s.pending
                    .as_ref()
                    .map(|p| (p.task.clone(), p.turn, p.operation, p.revision)),
            )
        };
        if let Some(receipt) = first_native {
            receipt.origin.handoff_stage("handoff.origin");
        }
        let origin = track
            .origin
            .clone()
            .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
        origin.handoff_stage("handoff.pending");
        let pending = origin
            .pending_write()
            .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
        let source_operation = if let SourcePurpose::OwnWrite(completed) = purpose {
            if !pending_native
                .as_ref()
                .is_some_and(|p| p.0 == task && p.1 == turn && p.2 == completed)
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            completed
        } else {
            operation
        };
        let rejected = if pending.0 != source_operation {
            Some("handoff.sourceOperation")
        } else if track.context != *context {
            Some("handoff.trackContext")
        } else if track.task != task {
            Some("handoff.trackTask")
        } else if shared.borrow().expected != pending.1 {
            Some("handoff.expectedText")
        } else {
            None
        };
        if let Some(reason) = rejected {
            origin.handoff_stage(reason);
            return Err(failure(TF_E_DISCONNECTED));
        }
        // Track::bind allocates its own block epoch after the composition's
        // epoch. Both identities must remain stable, not equal each other.
        let (block_epoch, version, lease, notification, depth, block_operation) = {
            let d = track.data.borrow();
            let rejected = if !d.valid {
                Some("handoff.blockInvalid")
            } else if d.writing {
                Some("handoff.blockWriting")
            } else if !d.origin_clean {
                Some("handoff.blockOrigin")
            } else if d.text != pending.1 {
                Some("handoff.blockText")
            } else {
                None
            };
            if let Some(reason) = rejected {
                origin.handoff_stage(reason);
                return Err(failure(TF_E_DISCONNECTED));
            }
            (
                d.epoch,
                d.version,
                d.lease,
                d.notification,
                d.notification_depth,
                d.operation,
            )
        };
        origin.handoff_stage("handoff.previousSource");
        let source = track.paragraph.borrow().clone();
        let following = track.source_follow.borrow().clone();
        if let Some(source) = &source {
            if let Some(following) = &following {
                if !Rc::ptr_eq(&following.source, source)
                    || !source.committed_at(following.predecessor)
                    || following.version != version
                    || following.text != pending.1
                {
                    return Err(failure(TF_E_DISCONNECTED));
                }
            } else {
                return if source.committed_at(version) {
                    Ok(SourceAttempt::Committed)
                } else {
                    Err(failure(TF_E_DISCONNECTED))
                };
            }
        } else if following.is_some() {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let current = || {
            let s = shared.borrow();
            let d = track.data.borrow();
            s.active
                && s.connected
                && !s.key_failure
                && s.cleanup == handoff
                && s.version.owned != handoff
                && s.generation == generation
                && s.epoch == epoch
                && s.connection == connection
                && s.cleanup_id == cleanup_id
                && s.context.as_ref() == Some(context)
                && s.composition.as_ref() == Some(&composition)
                && s.version.task == task
                && s.version.turn == turn
                && s.version.operation == operation
                && s.version.revision == revision
                && s.expected == pending.1
                && s.pending
                    .as_ref()
                    .map(|p| (p.task.clone(), p.turn, p.operation, p.revision))
                    == pending_native
                && s.block.as_ref().is_some_and(|b| Rc::ptr_eq(b, &track))
                && d.valid
                && !d.writing
                && d.origin_clean
                && d.epoch == block_epoch
                && d.text == pending.1
                && d.version == version
                && d.lease == lease
                && d.notification == notification
                && d.notification_depth == depth
                && d.operation == block_operation
        };
        // The host may already have moved its selection when it relinquishes
        // our composition. Readonly provenance must fence that current view,
        // not require the old writer's end caret. Acquire still checks it
        // independently after the native/provider range handoff.
        let readonly_selection = if matches!(
            purpose,
            SourcePurpose::HostEnd | SourcePurpose::FirstHostEnd
        ) {
            origin.handoff_stage("native.selection.capture");
            let selected =
                selection(context, ec).and_then(|range| range_position(context, &range, ec));
            evidence::source_selection(
                &track.data,
                origin.source_call_id(),
                "capture",
                0,
                true,
                None,
                &selected
                    .as_ref()
                    .map(|position| [i64::from(position.start), i64::from(position.end)])
                    .map_err(|error| windows::core::Error::from_hresult(error.code())),
                [block_epoch, version, lease, notification],
                current(),
            );
            let selected = selected?;
            origin.source_check_passed();
            Some(selected)
        } else {
            None
        };
        let native_attempt = Cell::new(0u32);
        let native = || -> windows::core::Result<()> {
            native_attempt.set(native_attempt.get().saturating_add(1));
            origin.handoff_stage("native.current");
            if !current() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            origin.source_check_passed();
            origin.handoff_stage("native.range");
            let owned = composition.GetRange()?;
            let mut entries = [None, None];
            let mut count = 0;
            origin.source_check_passed();
            origin.handoff_stage("native.enumerate");
            context
                .cast::<ITfContextComposition>()?
                .EnumCompositions()?
                .Next(&mut entries, &mut count)?;
            let units = pending.1.encode_utf16().count() as u32;
            #[cfg(feature = "acceptance")]
            eprintln!("paragraph_handoff_native_count count={count} handoff={handoff}");
            origin.source_check_passed();
            origin.handoff_stage("native.compositionSet");
            if count != 1 || entries[0].as_ref() != Some(&composition.cast::<ITfCompositionView>()?)
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            origin.source_check_passed();
            origin.handoff_stage("native.field");
            if !track.field.matches()? {
                return Err(failure(TF_E_DISCONNECTED));
            }
            origin.source_check_passed();
            origin.handoff_stage("native.ownedText");
            if range_text(&owned, ec)? != pending.1 {
                return Err(failure(TF_E_DISCONNECTED));
            }
            origin.source_check_passed();
            origin.handoff_stage("native.trackedText");
            if range_text(&track.range, ec)? != pending.1 {
                return Err(failure(TF_E_DISCONNECTED));
            }
            origin.source_check_passed();
            origin.handoff_stage("native.rangeStart");
            if owned.CompareStart(ec, &track.range, TF_ANCHOR_START)? != 0 {
                return Err(failure(TF_E_DISCONNECTED));
            }
            origin.source_check_passed();
            origin.handoff_stage("native.rangeEnd");
            if owned.CompareEnd(ec, &track.range, TF_ANCHOR_END)? != 0 {
                return Err(failure(TF_E_DISCONNECTED));
            }
            origin.source_check_passed();
            origin.handoff_stage("native.selection");
            let expected_selected = readonly_selection.map_or([i64::from(units); 2], |position| {
                [i64::from(position.start), i64::from(position.end)]
            });
            let selected = if readonly_selection.is_some() {
                selection(context, ec)
                    .and_then(|range| range_position(context, &range, ec))
                    .map(|position| [i64::from(position.start), i64::from(position.end)])
            } else {
                relative_selection(context, &owned, ec)
                    .map(|(start, end)| [i64::from(start), i64::from(end)])
            };
            evidence::source_selection(
                &track.data,
                origin.source_call_id(),
                "compare",
                native_attempt.get(),
                readonly_selection.is_some(),
                Some(expected_selected),
                &selected,
                [block_epoch, version, lease, notification],
                current(),
            );
            if selected? != expected_selected {
                origin.source_check_failed();
                origin.handoff_stage("native.selection.changed");
                return Err(failure(TF_E_DISCONNECTED));
            }
            origin.source_check_passed();
            origin.handoff_stage("native.current");
            if !current() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            origin.source_check_passed();
            Ok(())
        };
        origin.handoff_stage("handoff.authority.initial");
        authority()?;
        origin.source_check_passed();
        native()?;
        if first_receipt
            && !first_native
                .is_some_and(|receipt| receipt.proves_origin(&origin, &pending, version))
        {
            origin.handoff_stage("firstReceipt.nativeReceipt");
            return Err(failure(TF_E_DISCONNECTED));
        }
        if !origin.followed(source_operation) {
            let (candidate, previous) = match origin.prepare_follow(&pending)? {
                initial_selection::FollowPreparation::Candidate(candidate, previous) => {
                    (candidate, previous)
                }
                initial_selection::FollowPreparation::AwaitText => {
                    native()?;
                    origin.handoff_stage("handoff.authority.awaitFollow");
                    authority()?;
                    origin.source_check_passed();
                    origin.handoff_stage("follow.awaitText");
                    return Ok(SourceAttempt::AwaitPublication);
                }
                initial_selection::FollowPreparation::Unavailable => {
                    return Ok(SourceAttempt::Unchanged);
                }
            };
            native()?;
            origin.handoff_stage("handoff.authority.follow");
            authority()?;
            origin.source_check_passed();
            if !current() {
                origin.handoff_stage("handoff.current.follow");
                return Err(failure(TF_E_DISCONNECTED));
            }
            if !origin.publish_follow(&pending, candidate, previous) {
                origin.handoff_stage("handoff.publishFollow");
                return Err(failure(TF_E_DISCONNECTED));
            }
            // Retired range release may reenter; a changed source cannot retain
            // optional observation authority even after that release.
            if !current() {
                origin.handoff_stage("handoff.current.afterFollow");
                return Err(failure(TF_E_DISCONNECTED));
            }
        }
        // Pair the read with the same native ownership fence on both sides.
        // A counted client value cannot survive a changed native owner.
        let observed = if source.is_some() || first_receipt {
            true
        } else {
            origin.observe_client()
        };
        native()?;
        origin.handoff_stage("handoff.authority.observe");
        authority()?;
        origin.source_check_passed();
        if !current() {
            origin.handoff_stage("handoff.current.observe");
            return Err(failure(TF_E_DISCONNECTED));
        }
        if observed && (source.is_some() || first_receipt || origin.source_ready(&pending.1)) {
            // Seal and publish under the same owner identity as the receipt
            // pairing. COM during seal/native/authority may retire this Track;
            // the last pure fence must still name THIS current owner.
            origin.handoff_stage("handoff.seal");
            let proof = if first_receipt {
                let Some(proof) = origin.seal_first_receipt(
                    &pending,
                    version,
                    first_native.ok_or_else(|| failure(E_UNEXPECTED))?,
                )?
                else {
                    native()?;
                    origin.handoff_stage("handoff.authority.awaitSeal");
                    authority()?;
                    origin.source_check_passed();
                    origin.handoff_stage("firstReceipt.awaitClientBody");
                    return Ok(SourceAttempt::AwaitPublication);
                };
                first_source_event(&track, "nativeReceipt.paired");
                match origin.last_pair_reason() {
                    Some("composition") => first_source_event(&track, "client.compositionDiffers"),
                    Some("target") => first_source_event(&track, "client.targetDiffers"),
                    _ => (),
                }
                proof
            } else if let Some(following) = &following {
                origin.seal_following(
                    &following.source,
                    following.predecessor,
                    &pending.1,
                    version,
                )?
            } else {
                origin.seal(&pending.1, version)?
            };
            native()?;
            origin.handoff_stage("handoff.authority.seal");
            authority()?;
            origin.source_check_passed();
            let same_source = match (&source, &*track.paragraph.borrow()) {
                (None, None) => true,
                (Some(before), Some(after)) => Rc::ptr_eq(before, after),
                _ => false,
            };
            let same_follow = match (&following, &*track.source_follow.borrow()) {
                (None, None) => true,
                (Some(before), Some(after)) => {
                    Rc::ptr_eq(&before.source, &after.source)
                        && before.predecessor == after.predecessor
                        && before.version == after.version
                        && before.text == after.text
                }
                _ => false,
            };
            if !current() || !same_source || !same_follow || track.projection.borrow().is_some() {
                origin.handoff_stage("handoff.current.seal");
                return Err(failure(TF_E_DISCONNECTED));
            }
            // Remove the predecessor before releasing its COM references. The
            // replacement remains local and uncommitted through every fallible
            // check: a failed release-side reentry must not leave a new source
            // available to ProjectionProbe.
            let old = track.paragraph.borrow_mut().take();
            let old_follow = track.source_follow.borrow_mut().take();
            drop(old_follow);
            drop(old);
            drop(following);
            drop(source);
            // Releasing the old provider proof may reenter. Do not overwrite a
            // source or projection installed by that reentry, even if the other
            // owner fields happen to remain unchanged.
            native()?;
            origin.handoff_stage("handoff.authority.release");
            authority()?;
            origin.source_check_passed();
            if !current()
                || track.paragraph.borrow().is_some()
                || track.source_follow.borrow().is_some()
                || track.projection.borrow().is_some()
            {
                origin.handoff_stage("handoff.current.release");
                return Err(failure(TF_E_DISCONNECTED));
            }
            // commit_source and installation perform no COM calls. The exact
            // owner fence above is the final publication boundary; this proof
            // still confers no native writing permission.
            if first_receipt {
                // Every first-receipt entry returns a local candidate. Only
                // its common settlement can publish it after the caller's
                // real final fence (and the outer READ result for poll).
                *first.ok_or_else(|| failure(E_UNEXPECTED))?.borrow_mut() = Some(proof);
                return Ok(SourceAttempt::Committed);
            }
            let proof = Rc::new(proof.commit_source(&pending.1, version)?);
            *track.paragraph.borrow_mut() = Some(proof);
            origin.handoff_stage("handoff.sealed");
            #[cfg(feature = "acceptance")]
            eprintln!(
                "paragraph_scope_captured units={} version={version}",
                pending.1.encode_utf16().count()
            );
        }
        Ok(if track.paragraph.borrow().is_some() {
            SourceAttempt::Committed
        } else {
            SourceAttempt::Unchanged
        })
    }
}
pub(super) fn remember_client_write(shared: &Shared, operation: u64) {
    if let Some(origin) = original_point(shared) {
        let text = shared.borrow().expected.clone();
        origin.remember_write(operation, &text);
    }
}
pub(super) unsafe fn complete_client_write(
    shared: &Shared,
    context: &ITfContext,
    ec: u32,
    operation: u64,
    authority: &MutationGuard,
) {
    // Only an already established source needs this final own-write step.
    // Ordinary first output never waits for auxiliary snapshot readiness.
    let pending = shared
        .borrow()
        .block
        .as_ref()
        .is_some_and(|track| track.source_follow.borrow().is_some());
    if pending {
        let _ = unsafe {
            follow_client_write(
                shared,
                context,
                ec,
                authority,
                SourcePurpose::OwnWrite(operation),
            )
        };
    }
}
#[cfg(feature = "acceptance")]
unsafe fn diagnose_properties(context: &ITfContext, ec: u32, label: &str) {
    let result = (|| unsafe {
        let document = context.GetStart(ec)?;
        document.ShiftEndToRange(ec, &context.GetEnd(ec)?, TF_ANCHOR_END)?;
        let native = range_text(&document, ec)?;
        let native_units = native.encode_utf16().collect::<Vec<_>>();
        let selection = range_position(context, &selection(context, ec)?, ec)?;
        let category: ITfCategoryMgr =
            CoCreateInstance(&CLSID_TF_CategoryMgr, None, CLSCTX_INPROC_SERVER)?;
        let mut properties = Vec::new();
        for (name, guid) in [
            ("owner", GUID_PROP_TEXTOWNER),
            ("composing", GUID_PROP_COMPOSING),
        ] {
            let property = context.GetProperty(&guid)?;
            let mut ranges = None;
            property.EnumRanges(ec, &mut ranges, &document)?;
            let ranges = ranges.ok_or_else(|| failure(E_POINTER))?;
            let mut entries = Vec::new();
            for _ in 0..16 {
                let mut next = [None];
                let mut fetched = 0;
                ranges.Next(&mut next, &mut fetched)?;
                if fetched == 0 {
                    break;
                }
                let range = next[0].as_ref().ok_or_else(|| failure(E_POINTER))?;
                let value = property.GetValue(ec, range)?;
                let integer = i32::try_from(&value).ok();
                let owner = if name == "owner" {
                    integer.and_then(|atom| category.GetGUID(atom as u32).ok())
                } else {
                    None
                };
                entries.push(serde_json::json!({
                    "position":range_position(context,range,ec).ok().map(|v|[v.start,v.end]),
                    "integer":integer,
                    "owner":owner.map(|guid|format!("{guid:?}")),
                }));
            }
            properties.push(serde_json::json!({"property":name,"possiblyTruncated":entries.len()==16,"entries":entries}));
        }
        Ok::<_, windows::core::Error>(serde_json::json!({
            "properties":properties,
            "nativeUnits":native_units.iter().take(256).collect::<Vec<_>>(),
            "nativeTruncated":native_units.len()>256,
            "selected":[selection.start,selection.end],
            "documentFeed":diagnose_document_feed(context),
        }))
    })();
    eprintln!(
        "paragraph_source_properties {}",
        serde_json::json!({"phase":label,"properties":result.as_ref().ok(),"error":result.as_ref().err().map(|e|format!("{:08x}",e.code().0 as u32))})
    );
}
#[cfg(feature = "acceptance")]
unsafe fn diagnose_document_feed(context: &ITfContext) -> serde_json::Value {
    use windows::Win32::System::Threading::{GetCurrentProcessId, GetCurrentThreadId};
    use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
    unsafe {
        let focus = GetFocus();
        let identity = match initial_selection::FocusIdentity::capture() {
            Ok(identity) => identity,
            Err(error) => {
                return serde_json::json!({"identityError":format!("{:08x}",error.code().0 as u32)});
            }
        };
        let view = context.GetActiveView().and_then(|view| view.GetWnd());
        let root = GetAncestor(focus, GA_ROOT);
        let mut windows = vec![focus, root];
        if let Ok(view) = view {
            windows.push(view);
        }
        let mut seen = Vec::new();
        let mut replies = Vec::new();
        for window in windows {
            if window.0.is_null() || seen.contains(&window) {
                continue;
            }
            seen.push(window);
            let current = || {
                if !identity.matches().unwrap_or(false)
                    || GetFocus() != focus
                    || window != focus && !IsChild(window, focus).as_bool()
                {
                    return false;
                }
                let mut process = 0;
                let thread = GetWindowThreadProcessId(window, Some(&mut process));
                process == GetCurrentProcessId() && thread == GetCurrentThreadId()
            };
            if !current() {
                replies.push(serde_json::json!({"window":window.0 as usize,"sameThread":false}));
                continue;
            }
            // IMR_DOCUMENTFEED is a read request. Both calls stay on the
            // original UI thread; no returned pointer is dereferenced.
            let size = SendMessageW(window, WM_IME_REQUEST, Some(WPARAM(7)), Some(LPARAM(0))).0;
            if !(32..=65536).contains(&size) || size % 2 != 0 {
                replies.push(
                    serde_json::json!({"window":window.0 as usize,"size":size,"supported":false}),
                );
                continue;
            }
            let mut buffer = vec![0u32; (size as usize).div_ceil(4)];
            buffer[0] = size as u32;
            if !current() {
                replies.push(serde_json::json!({"window":window.0 as usize,"raced":true,"stage":"beforeFill"}));
                continue;
            }
            let reply = SendMessageW(
                window,
                WM_IME_REQUEST,
                Some(WPARAM(7)),
                Some(LPARAM(buffer.as_mut_ptr() as isize)),
            );
            if !current() {
                replies.push(serde_json::json!({"window":window.0 as usize,"raced":true,"stage":"afterFill"}));
                continue;
            }
            let valid = reply.0 != 0
                && buffer[0] as usize <= buffer.len() * 4
                && buffer[0] >= 32
                && buffer[1] == 0
                && buffer[3] >= 32
                && buffer[3].is_multiple_of(2)
                && (buffer[3] as usize)
                    .checked_add(buffer[2] as usize * 2)
                    .is_some_and(|end| end <= buffer[0] as usize)
                && [4usize, 6].into_iter().all(|at| {
                    buffer[at + 1].is_multiple_of(2)
                        && (buffer[at + 1] / 2)
                            .checked_add(buffer[at])
                            .is_some_and(|end| end <= buffer[2])
                });
            let units = if valid {
                std::slice::from_raw_parts(
                    buffer
                        .as_ptr()
                        .cast::<u8>()
                        .add(buffer[3] as usize)
                        .cast::<u16>(),
                    buffer[2] as usize,
                )
                .iter()
                .take(256)
                .copied()
                .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            replies.push(serde_json::json!({
                "window":window.0 as usize,"size":size,"supported":true,"valid":valid,
                "header":&buffer[..8],"units":units,"truncated":buffer[2]>256,
            }));
        }
        serde_json::json!(replies)
    }
}
pub(super) unsafe fn probe_handoff(
    shared: &Shared,
    context: &ITfContext,
    composition: &ITfComposition,
    ec: u32,
    authority: &MutationGuard,
    first_source: Option<&Rc<FirstSourceHandoff>>,
) {
    let trace = {
        let state = shared.borrow();
        let origin = first_source
            .map(|source| source.claim.receipt.origin.clone())
            .or_else(|| state.block.as_ref().and_then(|track| track.origin.clone()));
        origin.map(|origin| {
            origin.source_call(
                &state.version.task,
                state.version.operation,
                state.version.revision,
                source_entry(
                    state.cleanup,
                    first_source.map(|source| source.cleanup.is_some()),
                ),
            )
        })
    };
    let result = (|| unsafe {
        if let Some(trace) = &trace {
            trace.stage("probe.entry");
        }
        let track = shared
            .borrow()
            .block
            .clone()
            .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
        if track.projection.borrow().is_some()
            || track.paragraph.borrow().is_some() && track.source_follow.borrow().is_none()
        {
            return Ok(());
        }
        if let Some(trace) = &trace {
            trace.stage("probe.composition");
        }
        if shared.borrow().composition.as_ref() != Some(composition) {
            return Err(failure(TF_E_DISCONNECTED));
        }
        // Receipt pairing, strict seal and read-only source publication share
        // the existing full current-owner fence; no detached Track can publish.
        if let Some(trace) = &trace {
            trace.stage("probe.receipt");
        }
        if first_source.is_some_and(|source| !source.current(shared)) {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let purpose = if let Some(source) = first_source {
            if source.cleanup.is_some() {
                SourcePurpose::FirstHandoff
            } else {
                SourcePurpose::FirstHostEnd
            }
        } else if shared.borrow().cleanup {
            SourcePurpose::Handoff
        } else {
            SourcePurpose::HostEnd
        };
        let (state, authority, source) = (shared.clone(), authority.clone(), first_source.cloned());
        let guarded = move || {
            authority()?;
            if source
                .as_ref()
                .is_some_and(|source| !source.current(&state))
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            Ok(())
        };
        // The borrowed transfer cannot escape this synchronous native lock.
        // Its pure identity fence runs around the original provider pairing.
        let guard: MutationGuard = Rc::new(guarded);
        if let Some(source) = first_source {
            let proof = RefCell::new(None);
            let serial = track.data.borrow().notification;
            let outcome = follow_client_write_inner(
                shared,
                context,
                ec,
                &guard,
                purpose,
                Some(&proof),
                Some(&source.claim.receipt),
            );
            if let Some(trace) = &trace {
                trace.finish(
                    outcome
                        .as_ref()
                        .map_or("error", |outcome| source_outcome(*outcome)),
                    outcome.as_ref().err().map(|error| error.code()),
                );
            }
            let candidate = proof.borrow_mut().take();
            drop(proof);
            if outcome.is_err()
                && let Some(origin) = &track.origin
            {
                origin.retire();
            }
            let ready = outcome.is_ok();
            let committed = settle_first_source(
                &source.claim,
                outcome.unwrap_or(SourceAttempt::Unchanged),
                candidate,
                false,
                "committed.handoff",
                trace.as_ref(),
                || ready && source.current(shared) && track.data.borrow().notification == serial,
            );
            if !committed {
                return Err(failure(TF_E_DISCONNECTED));
            }
        } else if !follow_client_write(shared, context, ec, &guard, purpose)? {
            return Err(failure(TF_E_DISCONNECTED));
        }
        Ok::<_, windows::core::Error>(())
    })();
    if let Some(trace) = &trace {
        trace.finish_if_unrun(result.as_ref().err().map(|error| error.code()));
        if let Some(source) = first_source {
            source
                .claim
                .track
                .first_source_stage
                .set(trace.stage_name());
        }
    }
    if let Some(source) = first_source.filter(|source| source.cleanup.is_none()) {
        source.claim.track.first_host_source.set((
            if result.is_ok() {
                "committed"
            } else {
                "refused"
            },
            trace.as_ref().map_or("notRun", |trace| trace.stage_name()),
        ));
    }
    if let Err(_error) = result {
        #[cfg(feature = "acceptance")]
        eprintln!(
            "paragraph_scope_capture_failed hr={:08x} facts={}",
            _error.code().0 as u32,
            shared
                .borrow()
                .block
                .as_ref()
                .and_then(|t| t.origin.as_ref())
                .map(|o| o.diagnostic())
                .unwrap_or(serde_json::Value::Null)
        );
    }
}
fn source_entry(cleanup: bool, transfer_cleanup: Option<bool>) -> &'static str {
    if transfer_cleanup.unwrap_or(cleanup) {
        "keyboard"
    } else {
        "host"
    }
}
#[cfg(feature = "acceptance")]
pub(super) unsafe fn sample_after_handoff(shared: &Shared) {
    let track = shared.borrow().block.clone();
    let proof = track.and_then(|track| track.paragraph.borrow().clone());
    if let Some(proof) = proof {
        let _ = unsafe { proof.sample_handoff() };
    }
}
pub(super) fn invalidate(shared: &Shared, reason: &'static str) {
    if let Some(track) = shared.borrow().block.as_ref() {
        retire_first_source(track, reason);
        let mut data = track.data.borrow_mut();
        if data.valid {
            data.invalid_reason = reason.into();
        }
        data.valid = false;
        data.writable = false;
        data.lease += 1;
    }
}

#[implement(ITfThreadMgrEventSink)]
struct FocusSink {
    context: ITfContext,
    native: edit_context::Binding,
    data: Rc<RefCell<Data>>,
    notifications: Rc<TextNotifications>,
    _lease: Lease,
}
impl FocusSink {
    fn invalidate(&self, reason: &'static str) {
        let mut data = self.data.borrow_mut();
        if data.valid {
            data.invalid_reason = reason.into();
        }
        data.valid = false;
        data.writable = false;
        data.lease += 1;
    }
}
impl ITfThreadMgrEventSink_Impl for FocusSink_Impl {
    fn OnInitDocumentMgr(&self, _: Ref<ITfDocumentMgr>) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnUninitDocumentMgr(&self, document: Ref<ITfDocumentMgr>) -> windows::core::Result<()> {
        let _callback = Callback::enter();
        let _notification = TextNotification::enter(self.notifications.clone());
        if document
            .as_ref()
            .is_some_and(|doc| self.native.owns_document(doc))
        {
            self.invalidate("blockDocumentRetired");
        }
        Ok(())
    }
    fn OnSetFocus(
        &self,
        document: Ref<ITfDocumentMgr>,
        previous: Ref<ITfDocumentMgr>,
    ) -> windows::core::Result<()> {
        let _callback = Callback::enter();
        let _notification = TextNotification::enter(self.notifications.clone());
        let matched = document
            .as_ref()
            .map(|d| unsafe { self.native.matches_document(d) });
        if !matched
            .as_ref()
            .is_some_and(|value| value.as_ref().is_ok_and(|value| *value))
            || !self.native.focus_matches()
        {
            let diagnostic =
                self.native
                    .focus_diagnostic(document.as_ref(), previous.as_ref(), &matched);
            if self.data.borrow().valid {
                self.data.borrow_mut().focus_diagnostic = Some(diagnostic);
            }
            self.invalidate("blockDocumentFocus");
        }
        Ok(())
    }
    fn OnPushContext(&self, context: Ref<ITfContext>) -> windows::core::Result<()> {
        let _callback = Callback::enter();
        let _notification = TextNotification::enter(self.notifications.clone());
        if let Some(context) = context.as_ref()
            && context != &self.context
        {
            let added = unsafe { context.GetDocumentMgr() };
            if added
                .as_ref()
                .map_or(true, |document| self.native.owns_document(document))
            {
                self.invalidate("blockContextPushed");
            }
        }
        Ok(())
    }
    fn OnPopContext(&self, context: Ref<ITfContext>) -> windows::core::Result<()> {
        if context
            .as_ref()
            .is_some_and(|context| self.native.owns_context(context))
        {
            self.invalidate("blockContextPopped");
        }
        Ok(())
    }
}

pub(super) fn authorized(
    shared: &Shared,
    request: &Request,
    connection: u64,
) -> windows::core::Result<()> {
    let state = shared.borrow();
    if !state.active
        || !state.connected
        || state.connection != connection
        || state.capability.as_ref() != Some(&request.capability)
        || state.binding != request.engine
        || !request.live(unsafe { GetTickCount64() })
        || state.cleanup
        || state.hosted && !hosted_channel_alive(&state, &request.capability)
    {
        return Err(failure(TF_E_DISCONNECTED));
    }
    let peer = state.peer.ok_or_else(|| failure(E_ACCESSDENIED))?;
    if state
        .pipe
        .as_ref()
        .is_some_and(|p| !p.alive() || p.client_pid().ok() != Some(peer.pid))
    {
        return Err(failure(TF_E_DISCONNECTED));
    }
    drop(state);
    if registration::read_host() != Some((peer.pid, request.capability.clone()))
        || identity(peer.pid).ok() != Some(peer)
    {
        return Err(failure(TF_E_DISCONNECTED));
    }
    Ok(())
}
fn edit_authorized(
    shared: &Shared,
    r: &Request,
    connection: u64,
    generation: u64,
) -> windows::core::Result<()> {
    authorized(shared, r, connection)?;
    let state = shared.borrow();
    if state.generation != generation
        || !state.pending.as_ref().is_some_and(|p| {
            p.task == r.task
                && p.turn == r.turn
                && p.operation == r.operation
                && p.revision == r.revision
                && p.connection == connection
                && p.capability == r.capability
        })
    {
        return Err(failure(TF_E_DISCONNECTED));
    }
    Ok(())
}

pub(super) fn dispatch(shared: &Shared, request: &Request, connection: u64) {
    let result = (|| unsafe {
        authorized(shared, request, connection)?;
        if shared.borrow().pending.is_some() {
            return Err(failure(TF_E_LOCKED));
        }
        let command = request
            .block
            .as_ref()
            .ok_or_else(|| failure(E_INVALIDARG))?;
        if command.action == BlockAction::Release {
            let matched = shared.borrow().block.as_ref().is_some_and(|track| {
                let data = track.data.borrow();
                track.task == request.task
                    && data.id == command.id
                    && data.epoch == command.epoch
                    && request.operation > data.operation
            });
            if !matched {
                return Err(failure(TF_E_DISCONNECTED));
            }
            release(shared);
            reply(
                shared,
                request,
                connection,
                true,
                "blockReleased",
                Mutation::NotAttempted,
            );
            return Ok(());
        }
        let (context, manager, client) = {
            let state = shared.borrow();
            let context = if command.action == BlockAction::Bind {
                state.context.clone()
            } else {
                state.block.as_ref().map(|track| track.context.clone())
            }
            .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
            (
                context,
                state.manager.clone().ok_or_else(|| failure(E_FAIL))?,
                state.client,
            )
        };
        if edit_context::focused(shared, &manager)? != context {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let edit: ITfEditSession = EditBlock {
            generation: shared.borrow().generation,
            shared: shared.clone(),
            request: request.clone(),
            connection,
            context: context.clone(),
            _lease: Lease::new(),
        }
        .into();
        shared.borrow_mut().pending = Some(Pending {
            request: Some(request.clone()),
            started: false,
            operation: request.operation,
            revision: request.revision,
            task: request.task.clone(),
            turn: request.turn,
            connection,
            capability: request.capability.clone(),
            settlement: None,
        });
        let flags = TF_ES_ASYNCDONTCARE
            | if command.action == BlockAction::Read {
                TF_ES_READ
            } else {
                TF_ES_READWRITE
            };
        let result = request_native_edit(&context, client, &edit, flags)?;
        if shared
            .borrow()
            .pending
            .as_ref()
            .is_some_and(|p| p.operation == request.operation)
        {
            result.ok()?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        reply_outcome(
            shared,
            request,
            connection,
            false,
            &format!("block:{:08x}", error.code().0 as u32),
            Mutation::NotAttempted,
            None,
        );
    }
}

#[implement(ITfCompositionSink)]
struct AcquisitionSink {
    delegate: ITfCompositionSink,
    attached: Rc<Cell<bool>>,
    terminated: Rc<Cell<bool>>,
    _lease: Lease,
}

#[cfg(test)]
mod acquisition_tests {
    use super::*;
    #[test]
    fn source_diagnostic_entry_matches_real_cleanup_without_a_first_receipt() {
        assert_eq!(source_entry(false, None), "host");
        assert_eq!(source_entry(true, None), "keyboard");
        assert_eq!(source_entry(false, Some(false)), "host");
        assert_eq!(source_entry(true, Some(true)), "keyboard");
    }
    fn tracked_state() -> Rc<RefCell<Data>> {
        Rc::new(RefCell::new(Data {
            id: "owned-test".into(),
            epoch: 1,
            version: 1,
            operation: 1,
            text: "字".into(),
            position: RangePosition { start: 0, end: 1 },
            valid: true,
            invalid_reason: "block".into(),
            focus_diagnostic: None,
            writable: true,
            lease: 5,
            writing: false,
            evidence: evidence::Record::default(),
            semantic_boundary: None,
            pending_proof: false,
            paragraph_pending: false,
            notification: 0,
            notification_depth: 0,
            origin_clean: false,
        }))
    }
    #[test]
    fn outside_commit_revokes_lease_before_a_nested_notification_supersedes_it() {
        let data = tracked_state();
        let outer = EditNotification::enter(&data);
        assert!(outer.commit_outside());
        assert_eq!(data.borrow().lease, 6);
        assert!(!data.borrow().writable);
        assert!(outer.current());
        let nested = EditNotification::enter(&data);
        assert!(!outer.current());
        drop(nested);
        drop(outer);
        assert_eq!(data.borrow().notification_depth, 0);
        assert_eq!(data.borrow().lease, 6);
        assert!(!data.borrow().writable);
    }
    #[test]
    fn nested_invalidation_cannot_commit_an_outer_boundary_sample() {
        let data = tracked_state();
        let outer = EditNotification::enter(&data);
        data.borrow_mut().valid = false;
        assert!(!outer.commit_outside());
        assert!(!outer.current());
        drop(outer);
        assert!(!data.borrow().valid);
        assert_eq!(data.borrow().notification_depth, 0);
    }
    #[implement(ITfCompositionSink)]
    struct Delegate(Rc<Cell<u32>>);
    impl ITfCompositionSink_Impl for Delegate_Impl {
        fn OnCompositionTerminated(
            &self,
            _: u32,
            _: Ref<ITfComposition>,
        ) -> windows::core::Result<()> {
            self.0.set(self.0.get() + 1);
            Ok(())
        }
    }
    #[test]
    fn synchronous_termination_is_retained_before_publication() {
        let callbacks = Rc::new(Cell::new(0));
        let attached = Rc::new(Cell::new(false));
        let terminated = Rc::new(Cell::new(false));
        let sink: ITfCompositionSink = AcquisitionSink {
            delegate: Delegate(callbacks.clone()).into(),
            attached: attached.clone(),
            terminated: terminated.clone(),
            _lease: Lease::new(),
        }
        .into();
        unsafe {
            sink.OnCompositionTerminated(0, None).unwrap();
        }
        assert!(terminated.get());
        assert_eq!(callbacks.get(), 0);
        // Attaching a failed candidate for retirement also enables terminal
        // notifications; this flag never grants a write lease.
        attached.set(true);
        unsafe {
            sink.OnCompositionTerminated(0, None).unwrap();
        }
        assert_eq!(callbacks.get(), 1);
    }
}
impl ITfCompositionSink_Impl for AcquisitionSink_Impl {
    fn OnCompositionTerminated(
        &self,
        ec: u32,
        composition: Ref<ITfComposition>,
    ) -> windows::core::Result<()> {
        let _callback = Callback::enter();
        if !self.attached.get() {
            // The provider can terminate the composition before returning its
            // object. No unregistered object may become a live Draft afterward.
            self.terminated.set(true);
            return Ok(());
        }
        unsafe {
            self.delegate
                .OnCompositionTerminated(ec, composition.as_ref())
        }
    }
}

struct AcquisitionOwner {
    epoch: u64,
    context: ITfContext,
    cookie: u32,
}
impl AcquisitionOwner {
    fn retire_failed(&self, shared: &Shared) {
        let has_composition = {
            let state = shared.borrow();
            // Reentrant Deactivate may revoke the generation without removing
            // this candidate's resources. Conversely, a newer epoch must never
            // be touched by the old request's cleanup.
            if state.epoch != self.epoch
                || state.context.as_ref() != Some(&self.context)
                || state.text_cookie != Some(self.cookie)
            {
                return;
            }
            state.composition.is_some()
        };
        if has_composition {
            end_owned(shared);
        } else {
            release_composition(shared, self.epoch, None);
        }
    }
}

unsafe fn acquire_composition(
    edit: &EditBlock_Impl,
    track: &Track,
    snapshot: &BlockSnapshot,
    ec: u32,
    mutation: &mut Mutation,
    phase: &mut String,
    acquired_owner: &mut Option<AcquisitionOwner>,
) -> windows::core::Result<()> {
    unsafe {
        let shared = &edit.shared;
        let r = &edit.request;
        let context = &edit.context;
        let verified = track.verified_content(ec, phase)?;
        if verified.semantic_text != snapshot.text
            || verified.content_version != snapshot.version
            || verified.selection_start != snapshot.selection_start
            || verified.selection_end != snapshot.selection_end
        {
            *phase = "blockSnapshotChanged".into();
            return Err(failure(TF_E_LOCKED));
        }
        let original = range_position(context, &track.range, ec)?;
        if verified.native_end != original.end {
            return Err(failure(TF_E_DISCONNECTED));
        }
        // Paired reads may confirm identity, never justify moving endpoints.
        if track.verified_content(ec, phase)? != verified {
            return Err(failure(TF_E_LOCKED));
        }
        edit_authorized(shared, r, edit.connection, edit.generation)?;
        let native = range_text(&track.range, ec)?;
        if native != verified.native_text {
            *phase = "blockSnapshotChanged".into();
            return Err(failure(TF_E_LOCKED));
        }
        let text = CompositionText::from_verified(&verified.semantic_text, &native)
            .ok_or_else(|| failure(TF_E_LOCKED))?;
        let selected = selection(context, ec)?;
        let selected_position = range_position(context, &selected, ec)?;
        edit_authorized(shared, r, edit.connection, edit.generation)?;
        let epoch = COMPOSITION_EPOCH.fetch_add(1, Ordering::AcqRel) + 1;
        let text_sink: ITfTextEditSink = TextSink {
            state: shared.clone(),
            epoch,
            _lease: Lease::new(),
        }
        .into();
        let source: ITfSource = context.cast()?;
        let cookie = source.AdviseSink(&ITfTextEditSink::IID, &text_sink)?;
        if let Err(error) = edit_authorized(shared, r, edit.connection, edit.generation) {
            let _ = source.UnadviseSink(cookie);
            return Err(error);
        }
        {
            let mut state = shared.borrow_mut();
            if state.composition.is_some() || state.text_cookie.is_some() || state.version.owned {
                drop(state);
                let _ = source.UnadviseSink(cookie);
                return Err(failure(TF_E_DISCONNECTED));
            }
            state.epoch = epoch;
            *acquired_owner = Some(AcquisitionOwner {
                epoch,
                context: context.clone(),
                cookie,
            });
            state.context = Some(context.clone());
            state.native_context = Some(track.field.binding());
            state.text_cookie = Some(cookie);
            state.expected = native.clone();
            state.text = Some(text);
            state.selection = Some(selected);
            state.backup = None; // Reacquisition can never erase the user's baseline.
            state.prepared = None;
            state.empty_range = None;
            state.native_position = None;
            state.own_change = None;
            state.ownership_loss = None;
            state.ownership_detail = None;
            state.key_handoff = false;
            state.materialize_allowed = true;
            state.preserved_attributes.clear();
            state.version = DraftVersion {
                task: r.task.clone(),
                turn: r.turn,
                revision: r.revision,
                operation: r.operation,
                owned: true,
            };
        }
        *phase = "blockStartComposition".into();
        let delegate: ITfCompositionSink = CompositionSink {
            state: shared.clone(),
            epoch,
            _lease: Lease::new(),
        }
        .into();
        let attached = Rc::new(Cell::new(false));
        let terminated = Rc::new(Cell::new(false));
        let sink: ITfCompositionSink = AcquisitionSink {
            delegate,
            attached: attached.clone(),
            terminated: terminated.clone(),
            _lease: Lease::new(),
        }
        .into();
        let owner: ITfContextComposition = context.cast()?;
        #[cfg(feature = "acceptance")]
        block_faults::acquisition_without_object(shared)
            .inspect_err(|_| *mutation = Mutation::Uncertain)?;
        *phase = "blockCarrierAccess".into();
        track
            .field
            .check_carrier(track.client)
            .inspect_err(|error| {
                if error.code() == TF_E_LOCKED {
                    *phase = "blockInputMethodBusy".into();
                }
            })?;
        edit_authorized(shared, r, edit.connection, edit.generation)?;
        // The Windows wrapper rejects a successful HRESULT with a null object.
        let (
            guard_shared,
            guard_request,
            data,
            start_range,
            connection,
            generation,
            version,
            lease,
        ) = (
            shared.clone(),
            r.clone(),
            track.data.clone(),
            track.range.clone(),
            edit.connection,
            edit.generation,
            snapshot.version,
            snapshot.lease,
        );
        let pending_selection = track.selection_sync.clone();
        let guard: MutationGuard = Rc::new(move || {
            edit_authorized(&guard_shared, &guard_request, connection, generation)?;
            if pending_selection.borrow().is_some() {
                return Err(failure(TF_E_LOCKED));
            }
            let data = data.borrow();
            if !data.valid || data.version != version || data.lease != lease {
                return Err(failure(TF_E_DISCONNECTED));
            }
            Ok(())
        });
        let composition = guarded_operation(shared, context, mutation, guard, move |attempted| {
            attempted.set(true);
            owner.StartComposition(ec, &start_range, &sink)
        })?;
        // StartComposition may synchronously terminate the candidate or reenter
        // the service. Never attach its returned object to a newer owner.
        let still_owned = {
            let state = shared.borrow();
            !terminated.get()
                && state.epoch == epoch
                && state.generation == edit.generation
                && state.version.owned
                && state.version.operation == r.operation
                && state.version.task == r.task
                && state.composition.is_none()
        };
        if !still_owned {
            // This object is ours, but it cannot authorize a write. Retain it
            // for the existing retirement path if ending it fails.
            if composition.EndComposition(ec).is_err() {
                let mut state = shared.borrow_mut();
                if state.epoch == epoch && state.composition.is_none() {
                    state.composition = Some(composition);
                    // It now belongs to the existing retirement owner. A later
                    // real termination must release that owner's resources even
                    // though this candidate never became writable.
                    attached.set(true);
                }
            }
            return Err(failure(TF_E_DISCONNECTED));
        }
        shared.borrow_mut().composition = Some(composition.clone());
        attached.set(true);
        edit_authorized(shared, r, edit.connection, edit.generation)?;
        let actual = composition.GetRange()?;
        if actual.CompareStart(ec, &track.range, TF_ANCHOR_START)? != 0
            || actual.CompareEnd(ec, &track.range, TF_ANCHOR_END)? != 0
            || range_text(&actual, ec)? != native
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let after_selection = range_position(context, &selection(context, ec)?, ec)?;
        if after_selection.start != selected_position.start
            || after_selection.end != selected_position.end
            || track.data.borrow().lease != snapshot.lease
        {
            *phase = "blockSnapshotChanged".into();
            *mutation = Mutation::NotAttempted;
            return Err(failure(TF_E_LOCKED));
        }
        edit_authorized(shared, r, edit.connection, edit.generation)?;
        if track.selection_sync.borrow().is_some() {
            *phase = "blockAwaitingSelectionSync".into();
            return Err(failure(TF_E_LOCKED));
        }
        let mut data = track.data.borrow_mut();
        if !data.valid || data.version != snapshot.version {
            return Err(failure(TF_E_DISCONNECTED));
        }
        data.text = native;
        data.lease += 1;
        data.writable = true;
        *mutation = Mutation::NotAttempted;
        Ok(())
    }
}

#[implement(ITfEditSession)]
struct EditBlock {
    generation: u64,
    shared: Shared,
    context: ITfContext,
    request: Request,
    connection: u64,
    _lease: Lease,
}
impl ITfEditSession_Impl for EditBlock_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        let _callback = Callback::enter();
        let mut mutation = Mutation::NotAttempted;
        let mut phase = "block".into();
        let mut acquired_owner = None;
        let result = (|| unsafe {
            let shared = &self.shared;
            let r = &self.request;
            edit_authorized(shared, r, self.connection, self.generation)?;
            let command = r.block.as_ref().ok_or_else(|| failure(E_INVALIDARG))?;
            let manager = shared
                .borrow()
                .manager
                .clone()
                .ok_or_else(|| failure(E_FAIL))?;
            if edit_context::focused(shared, &manager)? != self.context {
                return Err(failure(TF_E_DISCONNECTED));
            }
            if command.action == BlockAction::Bind {
                return bind_owned(
                    shared,
                    &self.context,
                    r,
                    self.connection,
                    self.generation,
                    ec,
                    None,
                    Mutation::NotAttempted,
                );
            }
            let track = shared
                .borrow()
                .block
                .clone()
                .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
            let snapshot = track.snapshot(ec, &mut phase)?;
            if track.task != r.task
                || snapshot.id != command.id
                || snapshot.epoch != command.epoch
                || r.operation <= track.data.borrow().operation
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            if matches!(command.action, BlockAction::Acquire | BlockAction::Apply) {
                phase = "blockSnapshotChanged".into();
                if snapshot.version != command.version
                    || snapshot.text != command.expected
                    || snapshot.selection_start != command.selection_start
                    || snapshot.selection_end != command.selection_end
                    || (command.action == BlockAction::Acquire
                        && (shared.borrow().version.owned || shared.borrow().composition.is_some()))
                {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                let owner: ITfContextComposition = self.context.cast()?;
                let mut compositions = [None];
                let mut fetched = 0;
                owner
                    .EnumCompositions()?
                    .Next(&mut compositions, &mut fetched)?;
                if command.action == BlockAction::Acquire && fetched != 0 {
                    phase = "blockInputMethodBusy".into();
                    return Err(failure(TF_E_LOCKED));
                }
            }
            edit_authorized(shared, r, self.connection, self.generation)?;
            if shared
                .borrow()
                .block
                .as_ref()
                .is_none_or(|current| !Rc::ptr_eq(current, &track))
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            track.data.borrow_mut().operation = r.operation;
            match command.action {
                BlockAction::Acquire => {
                    acquire_composition(
                        self,
                        &track,
                        &snapshot,
                        ec,
                        &mut mutation,
                        &mut phase,
                        &mut acquired_owner,
                    )?;
                    // Do not re-read UIA inside compositionstart. The exact native
                    // readback above confirms this acquisition; later writes
                    // still require the normal live Draft lease.
                    let data = track.data.borrow();
                    return Ok(BlockSnapshot {
                        lease: data.lease,
                        writable: true,
                        ..snapshot
                    });
                }
                BlockAction::Apply => {
                    if !track.data.borrow().writable
                        || snapshot.lease != command.lease
                        || r.text.is_empty()
                    {
                        return Err(failure(TF_E_DISCONNECTED));
                    }
                    let composition = shared
                        .borrow()
                        .composition
                        .clone()
                        .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
                    let range = composition.GetRange()?;
                    let epoch = shared.borrow().epoch;
                    let mut update = r.clone();
                    update.action = Action::Update;
                    update.block = None;
                    let mut writer_phase = "blockUpdate";
                    write_composing(
                        shared,
                        &self.context,
                        &range,
                        &update,
                        self.generation,
                        epoch,
                        ec,
                        &mut mutation,
                        &mut writer_phase,
                        &r.text,
                        true,
                    )
                    .inspect_err(|_| phase = writer_phase.into())?;
                    let data = track.data.borrow();
                    return Ok(BlockSnapshot {
                        text: r.text.clone(),
                        version: data.version,
                        lease: data.lease,
                        writable: false,
                        selection_start: r.text.encode_utf16().count() as u32,
                        selection_end: r.text.encode_utf16().count() as u32,
                        ..snapshot
                    });
                }
                BlockAction::Read => (),
                _ => return Err(failure(E_INVALIDARG)),
            }
            track.snapshot(ec, &mut phase)
        })();
        match &result {
            Ok(snapshot) => {
                self.shared.borrow_mut().block_reply = Some(snapshot.clone());
                complete(
                    &self.shared,
                    &self.request,
                    self.connection,
                    true,
                    "blockReady",
                    mutation,
                    None,
                );
            }
            Err(error) => {
                let track = self.shared.borrow().block.clone();
                let detail = if let Some(track) = track {
                    if error.code() == TF_E_DISCONNECTED
                        && self.request.block.as_ref().is_some_and(|command| {
                            let data = track.data.borrow();
                            command.id == data.id && command.epoch == data.epoch
                        })
                    {
                        evidence::source_receipt(
                            &track.data,
                            track.first_source_note.get(),
                            track.first_source_stage.get(),
                            track.origin.is_some(),
                            track.host_source_trace(),
                            track.receipt_events(),
                            track.first_source_read_result.get(),
                            track.origin.as_ref().map(|origin| origin.pair_diagnostic()),
                        );
                    }
                    unsafe {
                        evidence::capture(
                            &self.context,
                            &track.range,
                            &track.field,
                            &track.data,
                            ec,
                            ("request", &phase, Some(error.code())),
                        );
                    }
                    evidence::suffix(&track.data)
                } else {
                    String::new()
                };
                complete(
                    &self.shared,
                    &self.request,
                    self.connection,
                    false,
                    &refusal_code(&phase, error.code(), mutation, &detail),
                    mutation,
                    None,
                );
                if let Some(owner) = acquired_owner {
                    owner.retire_failed(&self.shared);
                }
            }
        }
        result.map(|_| ())
    }
}

// Classification describes only the retired range. Fresh input still requires
// a fully authenticated Begin at the session's original editing endpoint.
fn refusal_code(phase: &str, error: HRESULT, mutation: Mutation, detail: &str) -> String {
    let unavailable = mutation == Mutation::NotAttempted
        && error == TF_E_DISCONNECTED
        && (phase.starts_with("blockProviderRangeRetired:")
            || matches!(
                phase,
                "blockEditEmpty"
                    | "blockProtectedAnchor"
                    | "blockEditRangeDrift"
                    | "blockDocumentFocus"
                    | "blockEditSelection"
            ));
    format!(
        "{}{phase}:{:08x}{detail}",
        if unavailable { "blockUnavailable:" } else { "" },
        error.0 as u32
    )
}

#[cfg(test)]
mod native_lifetime_tests {
    use super::*;

    #[test]
    fn lost_range_classification_cannot_promote_security_or_uncertain_effects() {
        let retired = "blockProviderRangeRetired:paragraph.cacheRange.childCompare:80040201";
        assert!(
            refusal_code(retired, TF_E_DISCONNECTED, Mutation::NotAttempted, "")
                .starts_with("blockUnavailable:")
        );
        for effect in [Mutation::Changed, Mutation::Uncertain] {
            assert!(
                !refusal_code(retired, TF_E_DISCONNECTED, effect, "")
                    .starts_with("blockUnavailable:")
            );
        }
        for code in [E_ACCESSDENIED, E_NOINTERFACE, E_FAIL] {
            assert!(
                !refusal_code(retired, code, Mutation::NotAttempted, "")
                    .starts_with("blockUnavailable:")
            );
        }
        assert!(
            !refusal_code(
                "blockProviderRangeRetiredSecurity:other",
                TF_E_DISCONNECTED,
                Mutation::NotAttempted,
                ""
            )
            .starts_with("blockUnavailable:")
        );
        assert!(
            refusal_code(
                "blockEditEmpty",
                TF_E_DISCONNECTED,
                Mutation::NotAttempted,
                ""
            )
            .starts_with("blockUnavailable:blockEditEmpty:")
        );
        for phase in [
            "blockSnapshotAccess",
            "blockGuardProtected",
            "blockScopeInvalid",
            "blockInputMethodBusy",
        ] {
            assert!(
                !refusal_code(phase, E_ACCESSDENIED, Mutation::NotAttempted, "")
                    .starts_with("blockUnavailable:")
            );
        }
        for effect in [Mutation::Changed, Mutation::Uncertain] {
            assert!(
                !refusal_code("blockEditEmpty", TF_E_DISCONNECTED, effect, "")
                    .starts_with("blockUnavailable:")
            );
        }
        assert!(
            !refusal_code("blockEditEmpty", E_ACCESSDENIED, Mutation::NotAttempted, "")
                .starts_with("blockUnavailable:")
        );
        assert!(
            !refusal_code(
                "blockDocumentRetired",
                TF_E_DISCONNECTED,
                Mutation::NotAttempted,
                ""
            )
            .starts_with("blockUnavailable:")
        );
    }

    #[test]
    fn popped_context_cannot_revive_even_when_the_same_object_is_pushed_again() {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            let result = (|| -> windows::core::Result<()> {
                let manager: ITfThreadMgr =
                    CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
                let client = manager.Activate()?;
                let result = (|| -> windows::core::Result<()> {
                    for focus_loss in [false, true] {
                        let document = manager.CreateDocumentMgr()?;
                        let mut context = None;
                        let mut cookie = 0;
                        document.CreateContext(client, 0, None, &mut context, &mut cookie)?;
                        let context = context.ok_or_else(|| failure(E_FAIL))?;
                        document.Push(&context)?;
                        let data = Rc::new(RefCell::new(Data {
                            id: "owned-lifetime".into(),
                            epoch: 1,
                            version: 1,
                            operation: 1,
                            text: "周三开会".into(),
                            position: RangePosition { start: 0, end: 4 },
                            valid: true,
                            invalid_reason: String::new(),
                            focus_diagnostic: None,
                            writable: true,
                            lease: 1,
                            writing: false,
                            evidence: evidence::Record::default(),
                            semantic_boundary: None,
                            pending_proof: false,
                            paragraph_pending: false,
                            notification: 0,
                            notification_depth: 0,
                            origin_clean: false,
                        }));
                        let retained = edit_context::Binding::from_document(&document)?;
                        let sink: ITfThreadMgrEventSink = FocusSink {
                            context: context.clone(),
                            native: retained.clone(),
                            data: data.clone(),
                            notifications: Rc::new(TextNotifications::default()),
                            _lease: Lease::new(),
                        }
                        .into();
                        let source: ITfSource = manager.cast()?;
                        let subscription = source.AdviseSink(&ITfThreadMgrEventSink::IID, &sink)?;
                        let result = (|| -> windows::core::Result<()> {
                            assert!(data.borrow().valid && data.borrow().writable);
                            if focus_loss {
                                assert!(retained.matches_document(&document)?);
                                assert!(!retained.focus_matches());
                                sink.OnSetFocus(&document, None)?;
                                assert!(!data.borrow().valid);
                                assert_eq!(data.borrow().invalid_reason, "blockDocumentFocus");
                                let suffix = evidence::suffix(&data);
                                assert!(suffix.contains("\"document_is_carrier\":true"));
                                assert!(suffix.contains("\"document_matches\":true"));
                                assert!(suffix.contains("\"window_alive\":false"));
                            }
                            document.Pop(TF_POPF_ALL)?;
                            assert!(!data.borrow().valid && !data.borrow().writable);
                            assert_eq!(
                                data.borrow().invalid_reason,
                                if focus_loss {
                                    "blockDocumentFocus"
                                } else {
                                    "blockContextPopped"
                                }
                            );
                            document.Push(&context)?;
                            assert_eq!(document.GetTop()?, context);
                            assert!(!data.borrow().valid && !data.borrow().writable);
                            assert_eq!(data.borrow().text, "周三开会");
                            document.Pop(TF_POPF_ALL)?;
                            Ok(())
                        })();
                        source.UnadviseSink(subscription)?;
                        result?;
                    }
                    Ok(())
                })();
                manager.Deactivate()?;
                result
            })();
            CoUninitialize();
            result.unwrap();
        }
    }
}
