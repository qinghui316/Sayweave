//! Current-block policy inside the existing serial actor. No second event loop.
use super::*;
use crate::contracts::draft::Association;
use crate::processing::draft::{
    BLOCK_LIMIT, Block, Snapshot, capacity_boundary, capacity_boundary_at, native_capacity_boundary,
};
use sayweave_text_service::wire::{BlockAction, BlockCommand, BlockSnapshot};

pub(super) struct State {
    pub(super) mode: InputMode,
    caret_pending: bool,
    block: Block,
    order: u64,
    offset: usize,
    sealed_source: String,
    snapshot: Option<BlockSnapshot>,
    bound: bool,
    read_needed: bool,
    capacity: bool,
    permit: Arc<AtomicBool>,
    sequence: u64,
    stopped: bool,
    final_recorded: bool,
    rollover: bool,
    cleanup_failed: bool,
    uncertain: bool,
    proof_deadline: Option<Instant>,
    edit_identity: Option<(String, u64, u64)>,
    pending_source: Option<NativeTranscript>,
    pending_coverage: Option<(u64, crate::processing::draft_source::Watermark)>,
    visible_coverage: Option<crate::processing::draft_source::Watermark>,
    absorbed_coverage: Option<crate::processing::draft_source::Watermark>,
}
impl State {
    pub fn configured() -> Option<Self> {
        // Internal rollback keeps the current schema-capable binary. It is
        // intentionally absent from the product's settings and frontend ports.
        (std::env::var("SAYWEAVE_DRAFT_STRATEGY").as_deref() != Ok("segmented")).then(Self::new)
    }
    pub fn new() -> Self {
        Self {
            mode: InputMode::Revisable,
            caret_pending: false,
            block: Block::new(uuid::Uuid::new_v4().to_string()),
            order: 1,
            offset: 0,
            sealed_source: String::new(),
            snapshot: None,
            bound: false,
            read_needed: false,
            capacity: false,
            permit: Arc::new(AtomicBool::new(true)),
            sequence: 0,
            stopped: false,
            final_recorded: false,
            rollover: false,
            cleanup_failed: false,
            uncertain: false,
            proof_deadline: None,
            edit_identity: None,
            pending_source: None,
            pending_coverage: None,
            visible_coverage: None,
            absorbed_coverage: None,
        }
    }
    pub fn handoff(&mut self) {
        self.permit.store(false, Ordering::Release);
        if !self.read_needed {
            self.block.handoff();
            self.proof_deadline
                .get_or_insert_with(|| Instant::now() + Duration::from_secs(15));
        }
        self.read_needed = true;
    }
    pub fn stop(&mut self) {
        self.permit.store(false, Ordering::Release);
        self.block.stop();
        self.stopped = true;
    }
    pub(super) fn uncertain(&self) -> bool {
        self.uncertain
    }
    pub fn expire(&mut self) {
        self.stop();
        // A read raced by continued user edits cannot extend the end budget.
        self.cleanup_failed |= self.read_needed;
    }
    fn reset_block(&mut self) {
        self.block = Block::new(uuid::Uuid::new_v4().to_string());
        self.order += 1;
        self.snapshot = None;
        self.bound = false;
        self.read_needed = false;
        self.proof_deadline = None;
        self.edit_identity = None;
        self.capacity = false;
        self.final_recorded = false;
        self.rollover = false;
        self.permit = Arc::new(AtomicBool::new(true));
    }
    fn seal_source(&mut self, source: &NativeTranscript) {
        self.sealed_source = units::slice(&source.text, 0..self.offset);
        self.absorbed_coverage =
            crate::processing::draft_source::Watermark::capture(&source.segments, self.offset);
    }
}
pub(super) enum Effect {
    Write {
        preview: Option<Snapshot>,
        revision: u64,
        text: String,
        raw: String,
    },
    Acquire,
    Read {
        activity: u64,
    },
    Handoff,
    Release {
        abandon: bool,
    },
    Split {
        raw: String,
        text: String,
    },
    Keep,
    Close,
}
fn command(snapshot: &BlockSnapshot, action: BlockAction) -> BlockCommand {
    BlockCommand {
        id: snapshot.id.clone(),
        action,
        epoch: snapshot.epoch,
        version: snapshot.version,
        lease: snapshot.lease,
        expected: snapshot.text.clone(),
        selection_start: snapshot.selection_start,
        selection_end: snapshot.selection_end,
    }
}
fn job(
    s: &mut super::State,
    effect: Effect,
    future: impl std::future::Future<Output = NativeValue> + Send + 'static,
) {
    s.native = Some(NativeJob {
        kind: NativeKind::Revisable(effect),
        job: tokio::spawn(future),
    });
}
fn scalar_position(text: &str, utf16: u32) -> Option<usize> {
    let mut units = 0;
    for (i, c) in text.chars().enumerate() {
        if units == utf16 {
            return Some(i);
        }
        units += c.len_utf16() as u32;
    }
    (units == utf16).then(|| text.chars().count())
}
pub(super) fn observe(
    actor: &SessionDraft,
    s: &mut super::State,
    r: &mut State,
    value: NativeTranscript,
) {
    if value.task_id != actor.task
        || value.turn != s.turn
        || value.sequence <= s.latest.sequence
        || r.stopped
    {
        return;
    }
    if s.handoff || r.read_needed || s.activity != input_state::activity() {
        if r.pending_source
            .as_ref()
            .is_none_or(|old| value.sequence > old.sequence)
        {
            r.pending_source = Some(value);
        }
        return;
    }
    if let Some(coverage) = &r.absorbed_coverage {
        let Some(offset) = coverage.offset(&value.segments) else {
            s.record_failure(&actor.task, "sourceChanged".into());
            return;
        };
        r.offset = offset;
    } else if !value.text.starts_with(&r.sealed_source) {
        s.record_failure(&actor.task, "sourceChanged".into());
        return;
    }
    let tail = units::slice(&value.text, r.offset..value.text.chars().count());
    // Paragraph separators stay in the following owned tail. Ending the
    // preceding composition on a separator can rebase legacy native ranges.
    let available = r.block.speech_capacity();
    let boundary = if available == BLOCK_LIMIT {
        capacity_boundary(&tail)
    } else {
        capacity_boundary_at(&tail, available)
    }
    .and_then(|end| {
        let prefix = units::slice(&tail, 0..end);
        let body = prefix.trim_end_matches(['\r', '\n']);
        (!body.is_empty()).then(|| body.chars().count())
    });
    let end = boundary.unwrap_or_else(|| tail.chars().count());
    // Indivisible long source is delivered literally; never sent beyond the
    // model budget. Native admission still enforces its independent capacity.
    let candidate = units::slice(&tail, 0..end);
    let transport_boundary =
        native_capacity_boundary(&candidate, r.block.native_speech_capacity(14 * 1024));
    let end = transport_boundary.unwrap_or(end);
    if end == 0 && !candidate.is_empty() && r.block.confirmed().1.is_empty() {
        // A single grapheme larger than the native hard bound cannot acquire
        // a range; do not loop through empty delivery blocks or split it.
        s.record_failure(&actor.task, "compositionTextLimit".into());
        return;
    }
    r.capacity = boundary.is_some() || transport_boundary.is_some();
    let text = units::slice(&tail, 0..end);
    if text.encode_utf16().count() > 14 * 1024 {
        s.record_failure(&actor.task, "compositionTextLimit".into());
        return;
    }
    let stable = value
        .segments
        .iter()
        .take_while(|v| v.confirmed)
        .map(|v| v.text.chars().count())
        .sum::<usize>()
        .saturating_sub(r.offset)
        .min(end);
    if let Err(error) = r.block.observe(text, stable) {
        s.record_failure(&actor.task, error.code.into());
        return;
    }
    if s.unit
        .as_ref()
        .and_then(|u| u.source().draft.as_ref())
        .is_some_and(|a| !r.block.matches(&a.source))
    {
        cancel(actor, s, r);
    }
    if r.block
        .snapshot(false)
        .is_some_and(|v| r.block.needs_generation(&v))
        && s.merge_at.is_none()
    {
        s.merge_at = Some(Instant::now());
    }
    s.latest = value;
}
fn unit(r: &State, state: &super::State, snapshot: Snapshot, delivery: bool) -> Unit {
    let mut association = Association::attempt(r.order, snapshot.clone());
    association.asr_turn = state.turn;
    let mut cursor = 0;
    for segment in &state.latest.segments {
        let end = cursor + segment.text.chars().count();
        let begin = cursor.max(r.offset);
        let finish = end.min(r.offset + snapshot.speech_end);
        if begin < finish {
            association.asr_sources.push(units::SourceSpan {
                segment: segment.id,
                revision: segment.revision,
                chars: begin - cursor..finish - cursor,
            });
        }
        cursor = end;
    }
    association.delivery = delivery;
    if delivery {
        association.visible_revision = Some(r.block.confirmed().0);
    }
    Unit {
        draft: Some(association),
        id: uuid::Uuid::new_v4().to_string(),
        turn: state.turn,
        chars: 0..snapshot.text.chars().count(),
        source: vec![],
        text: snapshot.text,
        preserve_raw: false,
        final_source: false,
    }
}
pub(super) fn start_generation(
    actor: &SessionDraft,
    s: &mut super::State,
    r: &mut State,
) -> Result<(), Error> {
    if r.mode == InputMode::Caret
        || r.caret_pending
        || r.stopped
        || (s.stopping || actor.control.stopping()) && s.ending.is_none()
        || r.read_needed
        || s.handoff
        || s.stop_error.is_some()
        || s.unit.is_some()
        || s.ending.as_ref().is_some_and(|e| e.mode != Finish::Commit)
    {
        return Ok(());
    }
    let Some(snapshot) = r.block.snapshot(s.ending.is_some()) else {
        return Ok(());
    };
    if !r.block.needs_generation(&snapshot) {
        return Ok(());
    }
    if s.ending.is_none()
        && !r.capacity
        && s.merge_at
            .is_none_or(|v| v.elapsed() < Duration::from_millis(150))
    {
        return Ok(());
    }
    r.block.attempted(snapshot.clone());
    s.merge_at = None;
    let request = unit(r, s, snapshot, false);
    if actor.context.as_ref().is_none_or(|c| !c.model_ready())
        || s.model_disabled
        || request.text.chars().count() > crate::processing::draft::BLOCK_LIMIT
        || s.ending.as_ref().is_some_and(|e| e.expired)
    {
        s.unit = Some(UnitSlot::Ready(
            request,
            Err(crate::models::types::ModelError::new("preserveRaw")),
        ));
        return Ok(());
    }
    let context = actor.context.as_ref().unwrap().clone();
    let previous = s.report.text.clone();
    let deadline = s
        .ending
        .as_ref()
        .map_or_else(|| Instant::now() + Duration::from_secs(15), |e| e.deadline);
    let frozen = request.clone();
    s.unit = Some(UnitSlot::Generating(Generating {
        unit: request,
        job: tokio::spawn(async move { context.generate(&frozen, &previous, deadline).await }),
    }));
    Ok(())
}
fn accept(
    actor: &SessionDraft,
    s: &mut super::State,
    r: &mut State,
    unit: Unit,
    result: Generation,
) {
    let mut record = UnitResult {
        unit: unit.clone(),
        candidate: None,
        generation: "failed".into(),
        application: "notApplied".into(),
        error: None,
        input_tokens: None,
        output_tokens: None,
        elapsed_ms: 0,
        source_committed_end: None,
        applied_text: None,
        retained_separator: String::new(),
    };
    match result {
        Ok(output) => {
            record.input_tokens = output.usage.input;
            record.output_tokens = output.usage.output;
            record.elapsed_ms = output.elapsed_ms;
            record.candidate = Some(output.text.clone());
            record.generation = "success".into();
            if let Some(source) = unit.draft.as_ref().map(|d| &d.source)
                && r.block.matches(source)
                && !r.stopped
                && !r.read_needed
            {
                match r.block.accept(source.clone(), output.text) {
                    Ok(()) => {
                        record.application = if r.block.confirmed().0 > 0
                            && crate::contracts::draft::equivalent_text(
                                &r.block.visible(),
                                r.block.confirmed().1,
                            ) {
                            "applied"
                        } else {
                            "pending"
                        }
                        .into();
                        if record.application == "applied" {
                            record.applied_text = record
                                .candidate
                                .as_ref()
                                .map(|c| quality::retain_boundaries(&unit.text, c));
                        }
                    }
                    Err(error) => record.error = Some(error.code.into()),
                }
            }
        }
        Err(error) => {
            if error.code == "auth" {
                s.model_disabled = true;
            }
            if error.code == "preserveRaw" {
                record.generation = "skipped".into();
            }
            record.error = Some(error.code);
        }
    }
    log::info!(target:"sayweave", "operation=draft_candidate task_id={} block={} version={} unit={} generation={} application={} elapsed_ms={}",
        actor.task,r.block.id(),unit.draft.as_ref().map_or(0,|d|d.source.version),unit.id,record.generation,record.application,record.elapsed_ms);
    s.report.units.push(record);
}
pub(super) fn cancel(actor: &SessionDraft, s: &mut super::State, r: &mut State) {
    if let Some(slot) = s.unit.take() {
        match slot {
            UnitSlot::Generating(g) => {
                if let Some(context) = &actor.context {
                    context.cancel(&g.unit);
                }
                let source = g.unit.clone();
                accept(
                    actor,
                    s,
                    r,
                    source,
                    Err(crate::models::types::ModelError::new("cancelled")),
                );
                if let Some(last) = s.report.units.last_mut() {
                    last.generation = "interrupted".into();
                }
            }
            UnitSlot::Ready(source, result) => accept(actor, s, r, source, result),
        }
    }
}
fn archive_block(
    actor: &SessionDraft,
    s: &mut super::State,
    r: &mut State,
    text: &str,
    certain: bool,
) {
    if r.final_recorded {
        return;
    }
    r.final_recorded = true;
    checkpoint(
        actor,
        r,
        if !certain || r.cleanup_failed {
            "uncertain"
        } else if r.stopped {
            "stopped"
        } else {
            "delivered"
        },
    );
    s.report.revisable = true;
    let source = r.block.state_snapshot();
    let raw = units::slice(
        &s.written_raw,
        r.offset.min(s.written_raw.chars().count())..s.written_raw.chars().count(),
    );
    if !raw.is_empty() {
        if !s.report.raw.is_empty() {
            s.report.raw.push('\n');
        }
        s.report.raw.push_str(&raw);
    }
    if certain {
        s.report.text.push_str(text);
    }
    if source.text.is_empty() {
        return;
    }
    let record = UnitResult {
        unit: unit(r, s, source, true),
        candidate: (text.len() <= 16 * 1024).then(|| text.into()),
        generation: "skipped".into(),
        application: if certain && !r.cleanup_failed {
            "applied"
        } else {
            "uncertain"
        }
        .into(),
        error: (!certain || r.cleanup_failed).then(|| "inputUnconfirmed".into()),
        input_tokens: None,
        output_tokens: None,
        elapsed_ms: 0,
        source_committed_end: None,
        applied_text: certain.then(|| text.into()),
        retained_separator: String::new(),
    };
    log::info!(target:"sayweave", "operation=draft_delivery task_id={} block={} certain={} visible_revision={}",actor.task,r.block.id(),certain,r.block.confirmed().0);
    s.report.units.push(record);
}
pub(super) fn archive_detached(actor: &SessionDraft, s: &mut super::State, r: &mut State) {
    let text = r.block.confirmed().1.to_owned();
    archive_block(actor, s, r, &text, !r.uncertain && !r.cleanup_failed);
}
fn checkpoint(actor: &SessionDraft, r: &State, state: &str) {
    if let Some(context) = &actor.context {
        let (visible_revision, visible_text) = r.block.confirmed();
        context.checkpoint(crate::contracts::draft::Checkpoint {
            run_id: String::new(),
            order: r.order,
            source: r.block.state_snapshot(),
            visible_revision,
            visible_text: visible_text.into(),
            state: state.into(),
        });
    }
}
fn release(actor: &SessionDraft, s: &mut super::State, r: &State, abandon: bool) {
    let connection = actor.connection.clone();
    let turn = s.turn;
    let request = command(r.snapshot.as_ref().unwrap(), BlockAction::Release);
    job(s, Effect::Release { abandon }, async move {
        NativeValue::block(
            connection
                .block(
                    turn,
                    None,
                    request,
                    String::new(),
                    Arc::new(AtomicBool::new(true)),
                )
                .await,
        )
    });
}
fn continue_at_caret(
    actor: &SessionDraft,
    s: &mut super::State,
    r: &mut State,
) -> Result<(), Error> {
    // Coverage is produced by an acknowledged write, never by reading or
    // searching the host body. A changed partial ASR boundary cannot replay.
    let coverage = r
        .visible_coverage
        .clone()
        .ok_or_else(|| Error::new("sourceChanged"))?;
    let latest = r.pending_source.clone().unwrap_or_else(|| s.latest.clone());
    let offset = coverage
        .offset(&latest.segments)
        .ok_or_else(|| Error::new("sourceChanged"))?;
    r.pending_source = None;
    let text = r.block.confirmed().1.to_owned();
    archive_block(actor, s, r, &text, false);
    cancel(actor, s, r);
    r.reset_block();
    r.mode = InputMode::Caret;
    r.caret_pending = false;
    r.offset = offset;
    r.absorbed_coverage = Some(coverage);
    r.pending_coverage = None;
    r.sealed_source = units::slice(&latest.text, 0..offset);
    s.latest.sequence = 0;
    s.activity = input_state::activity();
    s.handoff = false;
    s.merge_at = None;
    s.paused_at = Some(Instant::now());
    observe(actor, s, r, latest);
    log::info!(target:"sayweave", "operation=draft_input_mode task_id={} turn={} mode=caret source_end={offset}",actor.task,s.turn);
    Ok(())
}
fn handoff_complete(actor: &SessionDraft, s: &mut super::State, r: &mut State) {
    if r.mode == InputMode::Caret {
        if let Err(error) = continue_at_caret(actor, s, r) {
            s.record_failure(&actor.task, error.code);
        }
        return;
    }
    handed_off(s, r);
}
fn handed_off(s: &mut super::State, r: &mut State) {
    if r.snapshot.is_none() {
        // No tracked block exists to reacquire: the next native range really
        // is a new source turn, unlike a refresh of an existing block.
        s.turn += 1;
        s.latest = NativeTranscript::default();
        s.written_raw.clear();
        r.offset = 0;
        r.sealed_source.clear();
    }
    // Retain the recognition source. Only a confirmed editor baseline will
    // advance its absorbed watermark, once, after the read succeeds.
    s.activity = input_state::activity();
    s.merge_at = None;
    s.handoff = false;
}
pub(super) fn schedule(
    actor: &SessionDraft,
    s: &mut super::State,
    r: &mut State,
) -> Result<(), Error> {
    s.report.revisable = true;
    if s.native.is_some() {
        return Ok(());
    }
    if r.mode == InputMode::Caret
        && s.draft.is_none()
        && r.proof_deadline.is_some_and(|d| Instant::now() >= d)
    {
        s.record_failure(&actor.task, "inputComposition".into());
        return Ok(());
    }
    if r.cleanup_failed {
        if s.ending.is_none() {
            return Ok(());
        }
        let text = r.block.confirmed().1.to_owned();
        archive_block(actor, s, r, &text, !r.uncertain && !r.read_needed);
        let connection = actor.connection.clone();
        job(s, Effect::Close, async move {
            NativeValue::Closed(connection.close().await)
        });
        return Ok(());
    }
    if s.stop_error.is_some() {
        r.stop();
        cancel(actor, s, r);
    }
    if s.handoff {
        if let Some(draft) = s.draft.take() {
            job(s, Effect::Handoff, async move {
                NativeValue::HandedOff(draft.handoff_receipt().await)
            });
        } else {
            handoff_complete(actor, s, r);
        }
        return Ok(());
    }
    // Finish intent is synchronous; its final ASR/End envelope can arrive later.
    // Release an already handed-off range above, but do not arrange fresh writes.
    if (s.stopping || actor.control.stopping()) && s.ending.is_none() {
        return Ok(());
    }
    if s.ending.as_ref().is_some_and(|e| e.mode != Finish::Commit) {
        r.stop();
        cancel(actor, s, r);
    }
    if r.mode == InputMode::Caret
        && s.ending.is_some()
        && s.draft.is_none()
        && r.block.visible() != r.block.confirmed().1
    {
        // Finish may flush a held composition, but does not authorize a new
        // position for speech that never reached the host.
        s.record_failure(&actor.task, "inputUnconfirmed".into());
        return Ok(());
    }
    if s.draft.is_none()
        && r.proof_deadline.is_some_and(|d| Instant::now() >= d)
        && r.snapshot.is_some()
    {
        release(actor, s, r, true);
        return Ok(());
    }
    if r.read_needed
        && let Some(snapshot) = &r.snapshot
        && (s.ending.is_some()
            || s.paused_at
                .is_none_or(|a| a.elapsed() >= Duration::from_millis(300)))
        && (s.ending.is_some() || !input_state::editing_key_held())
    {
        let connection = actor.connection.clone();
        let turn = s.turn;
        let anchor = match windows_input::reanchor_turn(&actor.task, &actor.anchor) {
            Ok(anchor) => anchor,
            Err(error) => {
                r.cleanup_failed = true;
                return Err(error);
            }
        };
        let activity = anchor.activity;
        let request = command(snapshot, BlockAction::Read);
        job(s, Effect::Read { activity }, async move {
            NativeValue::block(
                connection
                    .block(
                        turn,
                        Some(anchor),
                        request,
                        String::new(),
                        Arc::new(AtomicBool::new(true)),
                    )
                    .await,
            )
        });
        return Ok(());
    }
    if r.read_needed && r.snapshot.is_none() {
        let visible = r.block.confirmed().1.to_owned();
        archive_block(actor, s, r, &visible, false);
        r.reset_block();
        r.offset = 0;
        r.sealed_source.clear();
    }
    if matches!(s.unit, Some(UnitSlot::Ready(..)))
        && let Some(UnitSlot::Ready(unit, result)) = s.unit.take()
    {
        accept(actor, s, r, unit, result);
    }
    // A ready result can cover an older append-only snapshot. Freeze the
    // actual final source again before deciding that an empty slot is terminal.
    if s.ending.is_some() {
        start_generation(actor, s, r)?;
    }
    // Never remove a running generator while checking its ready slot.
    if r.rollover && r.snapshot.is_some() {
        release(actor, s, r, false);
        return Ok(());
    }
    let finish_tail = r.mode != InputMode::Caret
        && s.ending
            .as_ref()
            .is_some_and(|e| e.mode == Finish::Commit && !e.expired)
        && !r.stopped
        && !crate::contracts::draft::equivalent_text(&r.block.visible(), r.block.confirmed().1);
    if s.draft.is_none()
        && r.snapshot.is_none()
        && (s.ending.is_none() && !s.stopping || finish_tail)
    {
        if s.paused_at
            .is_none_or(|a| a.elapsed() >= Duration::from_millis(300))
            && !input_state::editing_key_held()
        {
            if r.mode == InputMode::Caret {
                // The old block is already retired. Preparation starts after
                // editing releases the caret, not while the user still types.
                r.proof_deadline
                    .get_or_insert_with(|| Instant::now() + Duration::from_secs(15));
            }
            let anchor = windows_input::reanchor_turn(&actor.task, &actor.anchor)?;
            if anchor.activity != s.activity {
                actor.control.hold_input();
                r.handoff();
                s.handoff = true;
                s.paused_at = Some(Instant::now());
                return Ok(());
            }
            let connection = actor.connection.clone();
            let turn = s.turn;
            let activity = s.activity;
            s.native = Some(NativeJob {
                kind: NativeKind::Begin { turn, activity },
                job: tokio::spawn(async move {
                    NativeValue::Begun(connection.begin_turn(turn, anchor).await)
                }),
            });
        }
        return Ok(());
    }
    if !r.read_needed
        && !r.stopped
        && s.stop_error.is_none()
        && let Some((revision, text)) = r.block.write()
    {
        r.sequence += 1;
        let raw = units::slice(
            &s.latest.text,
            0..(r.offset + r.block.state_snapshot().speech_end).min(s.latest.text.chars().count()),
        );
        let body = text.clone();
        let source = raw.clone();
        if let Some(draft) = s.draft.clone() {
            r.pending_coverage = crate::processing::draft_source::Watermark::capture(
                &s.latest.segments,
                raw.chars().count(),
            )
            .map(|coverage| (revision, coverage));
            let sequence = r.sequence;
            let permit = r.permit.clone();
            let initial_block =
                (!r.bound && r.mode == InputMode::Revisable).then(|| r.block.id().to_owned());
            job(
                s,
                Effect::Write {
                    preview: r.block.preview_source(),
                    revision,
                    text,
                    raw,
                },
                async move {
                    if let Some(id) = initial_block {
                        return NativeValue::TrackedReplaced(
                            draft
                                .replace_tracked(sequence, body, Some(source), id, permit)
                                .await,
                        );
                    }
                    NativeValue::GuardedReplaced(
                        draft
                            .replace_guarded(sequence, body, Some(source), Some(permit))
                            .await,
                    )
                },
            );
        } else if let Some(snapshot) = r.snapshot.clone() {
            let connection = actor.connection.clone();
            let turn = s.turn;
            let permit = r.permit.clone();
            let remaining = r.proof_deadline.map_or(Duration::from_secs(15), |d| {
                d.saturating_duration_since(Instant::now())
            });
            let remaining = s.ending.as_ref().map_or(remaining, |e| {
                remaining.min(e.deadline.saturating_duration_since(Instant::now()))
            });
            job(s, Effect::Acquire, async move {
                NativeValue::Reacquired(
                    connection
                        .reacquire_block(turn, snapshot, permit, remaining)
                        .await,
                )
            });
        }
        return Ok(());
    }
    let covered = r.mode == InputMode::Caret
        || r.block
            .snapshot(true)
            .is_none_or(|snapshot| !r.block.needs_generation(&snapshot));
    let finishing = s.ending.is_some() && s.unit.is_none() && covered;
    let splitting = r.capacity
        && s.unit.is_none()
        && covered
        && !r.stopped
        && (s.ending.is_some() || r.block.stable_covers_all());
    if !finishing && !splitting {
        return Ok(());
    }
    if splitting && s.draft.is_some() && !r.read_needed {
        if r.snapshot.is_some() {
            release(actor, s, r, false);
            return Ok(());
        }
        let draft = s.draft.as_ref().unwrap().clone();
        let text = r.block.confirmed().1.to_owned();
        let raw = r.block.state_snapshot().text;
        let body = text.clone();
        r.sequence += 1;
        let sequence = r.sequence;
        let permit = r.permit.clone();
        job(s, Effect::Split { raw, text }, async move {
            NativeValue::Settled(
                draft
                    .settle_with_source(sequence, body, String::new(), None, permit)
                    .await,
            )
        });
        return Ok(());
    }
    if splitting && s.draft.is_none() {
        let text = r.block.confirmed().1.to_owned();
        archive_block(actor, s, r, &text, true);
        r.offset += r.block.state_snapshot().speech_end;
        r.seal_source(&s.latest);
        r.rollover = true;
        if r.snapshot.is_some() {
            release(actor, s, r, false);
        } else {
            r.reset_block();
        }
        return Ok(());
    }
    if let Some(draft) = s.draft.take() {
        job(s, Effect::Keep, async move {
            NativeValue::HandedOff(draft.handoff_receipt().await)
        });
        return Ok(());
    }
    if r.snapshot.is_some() {
        release(actor, s, r, false);
        return Ok(());
    }
    let text = r.block.confirmed().1.to_owned();
    archive_block(actor, s, r, &text, s.stop_error.is_none());
    let connection = actor.connection.clone();
    job(s, Effect::Close, async move {
        NativeValue::Closed(connection.close().await)
    });
    Ok(())
}

pub(super) fn native_done(
    actor: &SessionDraft,
    s: &mut super::State,
    r: &mut State,
    kind: &NativeKind,
    value: NativeValue,
) -> Option<Delivery> {
    match (kind, value) {
        (NativeKind::Begin { turn, activity }, NativeValue::Begun(result)) => match result {
            Ok(draft) => {
                s.draft = Some(draft);
                s.activity = *activity;
                if *activity != input_state::activity() {
                    s.handoff = true;
                    r.handoff();
                } else {
                    s.paused_at = None;
                    if r.mode == InputMode::Caret {
                        r.proof_deadline = None;
                        actor.control.resume_retained(*turn, *activity);
                    } else {
                        actor.control.resume(*turn, *activity);
                    }
                }
            }
            Err(error)
                if (s.stopping || actor.control.stopping())
                    && normal_preparation_refusal(&error.code) =>
            {
                r.stop();
                cancel(actor, s, r);
                log::info!(target:"sayweave", "operation=composition_prepare_retired task_id={} turn={} error={}", actor.task, turn, error.code);
            }
            Err(error) if error.code == "inputComposition" => {
                actor.control.hold_input();
                s.paused_at = Some(Instant::now());
                log::info!(target:"sayweave", "operation=composition_prepare_wait task_id={} turn={} reason=inputComposition", actor.task, turn);
            }
            Err(error) if error.code == "editing" => {
                actor.control.hold_input();
                r.handoff();
                cancel(actor, s, r);
                s.handoff = true;
                s.paused_at = Some(Instant::now());
            }
            Err(error) => s.record_failure(&actor.task, error.code),
        },
        (
            NativeKind::Revisable(Effect::Write {
                preview,
                revision,
                text,
                raw,
            }),
            value,
        ) => {
            let mut mutation = Mutation::Uncertain;
            let result = match value {
                NativeValue::TrackedReplaced(Err(failure))
                | NativeValue::GuardedReplaced(Err(failure)) => {
                    if failure.mutation == Mutation::Changed && failure.tracking_failed {
                        // SetText completed before tracking failed. Preserve
                        // that submission without granting a range or replay.
                        let _ = r.block.acknowledge(*revision, text.clone());
                        s.written_raw = raw.clone();
                        s.report.recovery_raw = s.latest.text.clone();
                        s.record_failure(&actor.task, failure.error.code);
                        return None;
                    }
                    mutation = failure.mutation;
                    Err(failure.error)
                }
                NativeValue::TrackedReplaced(Ok(snapshot)) => {
                    if snapshot.id != r.block.id() {
                        Err(Error::new("compositionProtocol"))
                    } else {
                        // Retain the native identity even if physical editing
                        // revoked the write permit while this receipt travelled.
                        r.bound = true;
                        r.snapshot = Some(snapshot);
                        Ok(())
                    }
                }
                NativeValue::Replaced(result) => result,
                NativeValue::GuardedReplaced(Ok(())) => Ok(()),
                _ => Err(Error::new("compositionProtocol")),
            };
            match result {
                Err(error) if error.code == "selectionMapping" => {
                    r.block.discard_preview();
                    if let Some(record) = s
                        .report
                        .units
                        .iter_mut()
                        .rev()
                        .find(|u| u.application == "pending")
                    {
                        record.application = "notApplied".into();
                        record.error = Some(error.code);
                    }
                }
                Ok(()) => {
                    let _ = r.block.acknowledge(*revision, text.clone());
                    if let Some((written_revision, coverage)) = r.pending_coverage.take()
                        && written_revision == *revision
                    {
                        r.visible_coverage = Some(coverage);
                    }
                    s.written_raw = raw.clone();
                    s.last_write = Some(Instant::now());
                    checkpoint(actor, r, "preview");
                    if let Some(preview) = preview
                        && let Some(record) =
                            s.report.units.iter_mut().find(|u| {
                                u.unit.draft.as_ref().is_some_and(|d| d.source == *preview)
                            })
                    {
                        record.application = "applied".into();
                        record.applied_text = record
                            .candidate
                            .as_ref()
                            .map(|c| quality::retain_boundaries(&record.unit.text, c));
                    }
                }
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "staleSnapshot" | "editing" | "cancelled"
                    ) && r.stopped => {}
                Err(error)
                    if matches!(
                        error.code.as_str(),
                        "staleSnapshot" | "editing" | "inputComposition"
                    ) =>
                {
                    actor.control.hold_input();
                    cancel(actor, s, r);
                    r.handoff();
                    s.handoff = true;
                    s.paused_at = Some(Instant::now());
                }
                Err(error) => {
                    r.uncertain |= mutation != Mutation::NotAttempted;
                    s.report.recovery_raw = s.latest.text.clone();
                    s.record_failure(&actor.task, error.code);
                }
            }
        }
        (NativeKind::Revisable(Effect::Acquire), NativeValue::Reacquired(result)) => {
            use windows_input::composition::ReacquireOutcome;
            match result {
                Ok(ReacquireOutcome::Ready { draft, snapshot }) => {
                    s.draft = Some(draft);
                    if !r.read_needed && !s.handoff && r.permit.load(Ordering::Acquire) {
                        r.proof_deadline = None;
                    }
                    r.snapshot = Some(snapshot);
                }
                Ok(ReacquireOutcome::AwaitingProof) => {
                    actor.control.hold_input();
                    r.handoff();
                    s.paused_at = Some(Instant::now());
                }
                Ok(ReacquireOutcome::Unavailable) => {
                    r.caret_pending = true;
                    release(actor, s, r, false);
                }
                Ok(ReacquireOutcome::Invalidated) => release(actor, s, r, true),
                Ok(ReacquireOutcome::Uncertain) => {
                    r.uncertain = true;
                    s.record_failure(&actor.task, "compositionPartial".into());
                }
                Err(error) => s.record_failure(&actor.task, error.code),
            }
        }
        (NativeKind::Revisable(Effect::Handoff), NativeValue::HandedOff(outcome)) => {
            let safe = outcome.handed_off;
            if !safe {
                s.record_failure(&actor.task, "compositionCleanup".into());
            }
            s.last_handoff = Some(outcome);
            if safe {
                handoff_complete(actor, s, r);
            }
        }
        (NativeKind::Revisable(Effect::Read { activity }), NativeValue::Block(result)) => {
            match result {
                Ok(reply) if reply.applied && *activity == input_state::activity() => {
                    if let Some(snapshot) = reply.block {
                        let range = scalar_position(&snapshot.text, snapshot.selection_start)
                            .zip(scalar_position(&snapshot.text, snapshot.selection_end));
                        if let Some((start, end)) = range {
                            // Stop keeps editor text but must not grant new model permission.
                            let identity = (snapshot.id.clone(), snapshot.epoch, snapshot.version);
                            let fresh = r.edit_identity.as_ref() != Some(&identity)
                                && snapshot.text != r.block.confirmed().1;
                            let source = r.pending_source.as_ref().unwrap_or(&s.latest);
                            let finish_visible = !fresh
                                && s.ending.as_ref().is_some_and(|e| e.mode == Finish::Commit)
                                && s.unit.is_none()
                                && source.task_id == actor.task
                                && source.turn == s.turn
                                && source.text == s.written_raw
                                && r.visible_coverage.as_ref().is_some_and(|coverage| {
                                    coverage.offset(&source.segments)
                                        == Some(source.text.chars().count())
                                });
                            let mut stopped = r.stopped;
                            let rebased = if fresh {
                                r.block.rebase_retained(snapshot.text.clone(), start..end)
                            } else if stopped {
                                // Ending unchanged editor text needs no source
                                // insertion mapping and grants no write permit.
                                Ok(())
                            } else {
                                r.block.resume_unchanged(start..end)
                            };
                            let rebased = match rebased {
                                Err(error) if error.code == "sourceSelection" && finish_visible => {
                                    // Final ASR contains only already displayed source.
                                    // Retain it without authorizing a new insertion.
                                    r.stop();
                                    stopped = true;
                                    Ok(())
                                }
                                result => result,
                            };
                            if rebased.is_ok() {
                                if fresh {
                                    r.offset = s.written_raw.chars().count();
                                    r.absorbed_coverage = r.visible_coverage.clone();
                                    r.sealed_source = s.written_raw.clone();
                                }
                                r.edit_identity = Some(identity);
                                r.snapshot = Some(snapshot);
                                r.read_needed = false;
                                r.permit = Arc::new(AtomicBool::new(!stopped));
                                checkpoint(actor, r, "preview");
                                if stopped {
                                    r.block.stop();
                                } else if r.block.confirmed().1.chars().count()
                                    > crate::processing::draft::BLOCK_LIMIT
                                    && s.ending.is_none()
                                {
                                    let text = r.block.confirmed().1.to_owned();
                                    archive_block(actor, s, r, &text, true);
                                    r.rollover = true;
                                } else if !s.stopping {
                                    s.paused_at = None;
                                    s.activity = *activity;
                                    actor.control.resume_retained(s.turn, *activity);
                                    let pending =
                                        r.pending_source.take().unwrap_or_else(|| s.latest.clone());
                                    s.latest.sequence = 0;
                                    observe(actor, s, r, pending);
                                }
                            } else if rebased.is_err_and(|e| e.code == "sourceSelection") {
                                // The native range is valid; only the ASR insertion
                                // mapping is unavailable while the user positions
                                // the caret to edit. Keep the read-only tracker,
                                // revoked permit and original source watermark.
                                // A later verified text edit can establish a new
                                // baseline. The existing proof deadline still wins.
                                r.snapshot = Some(snapshot);
                                s.paused_at = Some(Instant::now());
                                log::info!(target:"sayweave", "operation=draft_selection_wait task_id={} block={} reason=sourceSelection", actor.task,r.block.id());
                            } else {
                                r.cleanup_failed = true;
                                s.record_failure(&actor.task, "compositionProtocol".into());
                            }
                        } else {
                            r.cleanup_failed = true;
                            s.record_failure(&actor.task, "compositionProtocol".into());
                        }
                    } else {
                        r.cleanup_failed = true;
                        s.record_failure(&actor.task, "compositionProtocol".into());
                    }
                }
                Ok(reply) if reply.applied => {
                    r.snapshot = reply.block.or_else(|| r.snapshot.take());
                    s.activity = input_state::activity();
                    s.paused_at = Some(Instant::now());
                }
                Ok(reply)
                    if windows_input::composition::block_refusal(&reply)
                        == windows_input::composition::BlockRefusal::Unavailable =>
                {
                    r.caret_pending = true;
                    release(actor, s, r, false);
                }
                Ok(reply)
                    if windows_input::composition::block_refusal(&reply)
                        == windows_input::composition::BlockRefusal::AwaitingProof =>
                {
                    s.paused_at = Some(Instant::now());
                }
                Ok(reply) if reply.mutation == Mutation::NotAttempted => release(actor, s, r, true),
                Ok(_) => {
                    r.cleanup_failed = true;
                    s.record_failure(&actor.task, "compositionPartial".into());
                }
                Err(error) => {
                    r.cleanup_failed = true;
                    s.record_failure(&actor.task, error.code);
                }
            }
        }
        (NativeKind::Revisable(Effect::Release { abandon }), NativeValue::Block(result)) => {
            match result {
                Ok(reply) if reply.applied => {
                    r.snapshot = None;
                    if r.caret_pending {
                        if let Err(error) = continue_at_caret(actor, s, r) {
                            s.record_failure(&actor.task, error.code);
                        }
                    } else if r.rollover {
                        r.reset_block();
                        let latest = s.latest.clone();
                        s.latest.sequence = 0;
                        observe(actor, s, r, latest);
                    } else if *abandon {
                        // Original scope is retired. Keep this recognition
                        // source for ResultOnly; never begin at a new focus.
                        r.stop();
                        s.record_failure(&actor.task, "targetChanged".into());
                    }
                }
                _ => {
                    r.cleanup_failed = true;
                    s.record_failure(&actor.task, "compositionCleanup".into());
                }
            }
        }
        (NativeKind::Revisable(Effect::Split { raw, text }), NativeValue::Settled(outcome)) => {
            if outcome.prefix == Mutation::Changed && outcome.tail_owned {
                archive_block(actor, s, r, text, true);
                r.offset += raw.chars().count();
                r.seal_source(&s.latest);
                r.reset_block();
                let latest = s.latest.clone();
                s.latest.sequence = 0;
                observe(actor, s, r, latest);
            } else {
                s.record_failure(
                    &actor.task,
                    outcome.error.unwrap_or_else(|| "compositionPartial".into()),
                );
            }
        }
        (NativeKind::Revisable(Effect::Keep), NativeValue::HandedOff(outcome)) => {
            if !outcome.handed_off {
                s.record_failure(&actor.task, "compositionCleanup".into());
            }
            s.last_handoff = Some(outcome);
        }
        (NativeKind::Revisable(Effect::Close), NativeValue::Closed(result)) => {
            s.report.cancelled = s.ending.as_ref().is_some_and(|e| e.mode == Finish::Abort);
            // An acknowledged native write remains submitted even when a
            // released carrier cannot expose the user's final edited body.
            let submitted = !s.written_raw.is_empty() || !s.report.text.is_empty();
            let mut delivery = Delivery::not_submitted(if s.report.cancelled {
                "cancelled"
            } else if !submitted {
                "emptyText"
            } else {
                "inputUnconfirmed"
            });
            if submitted {
                delivery.submission = Submission::Submitted;
                delivery.verification = crate::windows_input::Verification::Unavailable;
                delivery.stage = "composition".into();
                delivery.state = "unconfirmed".into();
            }
            if result.is_err() {
                crate::windows_input::composition::retain_failed(&actor.connection);
                delivery.error = Some("compositionCleanup".into());
            }
            if r.uncertain {
                delivery.submission = Submission::Uncertain;
                delivery.verification = crate::windows_input::Verification::Unavailable;
                delivery.stage = "composition".into();
                delivery.state = "unconfirmed".into();
            }
            return Some(delivery);
        }
        _ => s.record_failure(&actor.task, "compositionProtocol".into()),
    }
    None
}

#[cfg(test)]
mod tests {
    use super::super::tests::{actor, block_reply, command};
    use super::*;
    use crate::windows_input::composition::Command;
    fn source(task: &str, sequence: u64, old: &str, new: &str) -> NativeTranscript {
        let mut segments = vec![crate::contracts::processing::SourceSegment {
            id: 1,
            revision: 1,
            text: old.into(),
            confirmed: true,
        }];
        if !new.is_empty() {
            segments.push(crate::contracts::processing::SourceSegment {
                id: 2,
                revision: sequence,
                text: new.into(),
                confirmed: false,
            });
        }
        NativeTranscript {
            task_id: task.into(),
            turn: 1,
            sequence,
            text: format!("{old}{new}"),
            segments,
        }
    }
    #[tokio::test]
    async fn caret_transition_seals_old_source_and_writes_only_new_tail_without_tracking() {
        let (actor, mut s, native) = actor(None);
        let mut r = State::new();
        baseline(&mut r, "周三开会");
        s.latest = source(&actor.task, 1, "周三开会", "");
        s.written_raw = s.latest.text.clone();
        r.visible_coverage =
            crate::processing::draft_source::Watermark::capture(&s.latest.segments, 4);
        r.pending_source = Some(source(&actor.task, 2, "周三开会", "地点在上海"));
        let old_permit = r.permit.clone();
        r.permit.store(false, Ordering::Release);
        continue_at_caret(&actor, &mut s, &mut r).unwrap();
        assert_eq!(s.turn, 1);
        assert_eq!(r.mode, InputMode::Caret);
        assert!(!old_permit.load(Ordering::Acquire));
        assert!(!Arc::ptr_eq(&old_permit, &r.permit));
        assert_eq!(r.block.visible(), "地点在上海");
        assert!(
            r.proof_deadline.is_none(),
            "editing has not released a new position yet"
        );
        assert!(
            s.report.text.is_empty(),
            "host user edits are not fabricated"
        );
        start_generation(&actor, &mut s, &mut r).unwrap();
        assert!(s.unit.is_none());
        schedule(&actor, &mut s, &mut r).unwrap();
        let Command::Replace(turn, text, _, _, reply) = command(&native).await else {
            panic!("shared untracked writer")
        };
        assert_eq!(turn, 1);
        assert_eq!(text, "地点在上海");
        reply.send(Ok(())).unwrap();
        let value = SessionDraft::native_value(&mut s.native).await.unwrap();
        let completed = s.native.take().unwrap();
        native_done(&actor, &mut s, &mut r, &completed.kind, value);
        assert!(s.stop_error.is_none());
        assert!(
            r.visible_coverage
                .as_ref()
                .is_some_and(|v| v.offset(&s.latest.segments) == Some(9))
        );
        let mut latest = s.latest.clone();
        latest.sequence += 1;
        latest.text.push_str("北京");
        latest
            .segments
            .push(crate::contracts::processing::SourceSegment {
                id: 3,
                revision: 1,
                text: "北京".into(),
                confirmed: true,
            });
        r.pending_source = Some(latest);
        r.handoff();
        continue_at_caret(&actor, &mut s, &mut r).unwrap();
        assert_eq!(s.turn, 1);
        assert_eq!(r.block.visible(), "北京");
        assert!(s.unit.is_none());
    }
    #[test]
    fn caret_boundary_change_is_refused_without_consuming_a_new_input_block() {
        let (actor, mut s, _) = actor(None);
        let mut r = State::new();
        baseline(&mut r, "开会");
        let mut prior = source(&actor.task, 1, "开会", "");
        prior.segments[0].confirmed = false;
        r.visible_coverage =
            crate::processing::draft_source::Watermark::capture(&prior.segments, 2);
        s.latest = prior;
        r.pending_source = Some(source(&actor.task, 2, "会议", "在周一"));
        let id = r.block.id().to_owned();
        assert_eq!(
            continue_at_caret(&actor, &mut s, &mut r).unwrap_err().code,
            "sourceChanged"
        );
        assert_eq!(r.block.id(), id);
        assert_eq!(r.mode, InputMode::Revisable);
        assert_eq!(r.pending_source.as_ref().unwrap().text, "会议在周一");
    }
    #[tokio::test]
    async fn unavailable_scope_needs_a_successful_release_before_caret_mode() {
        let (actor, mut s, native) = actor(None);
        s.draft = None;
        let mut r = State::new();
        baseline(&mut r, "原块");
        let mut refusal = block_reply(&actor.task, None, false);
        refusal.code = "blockRetainedScopeUnavailable:80040204".into();
        let activity = s.activity;
        native_done(
            &actor,
            &mut s,
            &mut r,
            &NativeKind::Revisable(Effect::Read { activity }),
            NativeValue::block(Ok(refusal)),
        );
        assert_eq!(r.mode, InputMode::Revisable);
        assert!(r.caret_pending);
        let Command::Block(_, _, request, text, _, reply) = command(&native).await else {
            panic!("release old scope")
        };
        assert_eq!(request.action, BlockAction::Release);
        assert!(text.is_empty());
        reply.send(Err(Error::new("compositionCleanup"))).unwrap();
        let value = SessionDraft::native_value(&mut s.native).await.unwrap();
        let completed = s.native.take().unwrap();
        native_done(&actor, &mut s, &mut r, &completed.kind, value);
        assert_eq!(r.mode, InputMode::Revisable);
        assert_eq!(s.stop_error.as_deref(), Some("compositionCleanup"));
    }
    #[test]
    fn acknowledged_scope_release_keeps_the_recognition_source_and_only_the_new_tail() {
        let (actor, mut s, _) = actor(None);
        s.draft = None;
        let mut r = State::new();
        baseline(&mut r, "周三开会");
        s.latest = source(&actor.task, 1, "周三开会", "");
        s.written_raw = s.latest.text.clone();
        r.visible_coverage =
            crate::processing::draft_source::Watermark::capture(&s.latest.segments, 4);
        r.pending_source = Some(source(&actor.task, 2, "周三开会", "地点在上海"));
        r.handoff();
        r.caret_pending = true;
        actor.control.hold_input();
        native_done(
            &actor,
            &mut s,
            &mut r,
            &NativeKind::Revisable(Effect::Release { abandon: false }),
            NativeValue::block(Ok(block_reply(&actor.task, None, true))),
        );
        assert_eq!(s.turn, 1);
        assert_eq!(r.mode, InputMode::Caret);
        assert_eq!(r.block.visible(), "地点在上海");
        assert!(r.snapshot.is_none());
        assert!(s.draft.is_none());
        assert!(s.stop_error.is_none());
        assert_eq!(
            actor.control.audio.current(),
            0,
            "new position is not ready"
        );
        assert!(s.report.text.is_empty());
    }
    #[test]
    fn caret_begin_receipt_cannot_reopen_audio_after_finish_intent() {
        let (actor, mut s, _) = actor(None);
        let draft = s.draft.take().unwrap();
        let mut r = State::new();
        r.mode = InputMode::Caret;
        actor.control.resume(s.turn, s.activity);
        actor.control.fixture_ready(s.turn, s.activity);
        actor.control.hold_input();
        actor.control.request_finish();
        let kind = NativeKind::Begin {
            turn: s.turn,
            activity: s.activity,
        };
        native_done(&actor, &mut s, &mut r, &kind, NativeValue::Begun(Ok(draft)));
        assert_eq!(actor.control.audio.current(), 0);
        assert!(s.stop_error.is_none());
    }
    #[test]
    fn caret_without_a_new_tail_keeps_the_prior_submission_fact_at_finish_or_cancel() {
        for mode in [Finish::Commit, Finish::Abort] {
            let (actor, mut s, _) = actor(None);
            let mut r = State::new();
            r.mode = InputMode::Caret;
            s.written_raw = "周三开会".into();
            ending(&mut s, false);
            s.ending.as_mut().unwrap().mode = mode.clone();
            let delivery = native_done(
                &actor,
                &mut s,
                &mut r,
                &NativeKind::Revisable(Effect::Close),
                NativeValue::Closed(Ok(())),
            )
            .unwrap();
            assert_eq!(delivery.submission, Submission::Submitted);
            assert_eq!(
                delivery.verification,
                crate::windows_input::Verification::Unavailable
            );
            assert_eq!(s.report.cancelled, mode == Finish::Abort);
            assert_ne!(delivery.error.as_deref(), Some("emptyText"));
            assert!(
                s.report.text.is_empty(),
                "committed user edits remain unknown"
            );
        }
    }
    #[test]
    fn caret_finish_never_opens_a_new_range_for_an_unwritten_tail() {
        let (actor, mut s, _) = actor(None);
        s.draft = None;
        let mut r = State::new();
        r.mode = InputMode::Caret;
        r.block.observe("未输入尾字".into(), 5).unwrap();
        ending(&mut s, false);
        schedule(&actor, &mut s, &mut r).unwrap();
        assert!(s.native.is_none());
        assert_eq!(s.stop_error.as_deref(), Some("inputUnconfirmed"));
    }
    #[test]
    fn invalid_scope_release_does_not_start_an_asr_turn_at_the_new_focus() {
        let (actor, mut s, _) = actor(None);
        let mut r = State::new();
        let body = "原块正文";
        let snapshot = baseline(&mut r, body);
        r.snapshot = Some(snapshot);
        s.latest.text = body.into();
        s.written_raw = body.into();
        actor.control.resume(s.turn, s.activity);
        actor.control.fixture_ready(s.turn, s.activity);
        let turn = s.turn;
        native_done(
            &actor,
            &mut s,
            &mut r,
            &NativeKind::Revisable(Effect::Release { abandon: true }),
            NativeValue::block(Ok(block_reply(&actor.task, None, true))),
        );
        assert_eq!(s.turn, turn);
        assert_eq!(s.latest.text, body);
        assert_eq!(s.written_raw, body);
        assert_eq!(actor.control.audio.current(), turn);
        assert_eq!(s.stop_error.as_deref(), Some("targetChanged"));
        assert!(r.stopped);
        assert!(r.snapshot.is_none());
    }
    #[test]
    fn resumed_ime_composition_waits_without_failing_and_can_recover_same_turn() {
        let (actor, mut s, _) = actor(None);
        let draft = s.draft.take().unwrap();
        let mut r = State::new();
        let turn = s.turn;
        let activity = s.activity;
        s.report.text = "已交付的正文".into();
        native_done(
            &actor,
            &mut s,
            &mut r,
            &NativeKind::Begin { turn, activity },
            NativeValue::Begun(Err(Error::new("inputComposition"))),
        );
        assert!(s.stop_error.is_none());
        assert!(s.paused_at.is_some());
        assert!(!actor.control.stopping());
        assert_eq!(s.turn, turn);
        assert_eq!(s.report.text, "已交付的正文");
        native_done(
            &actor,
            &mut s,
            &mut r,
            &NativeKind::Begin { turn, activity },
            NativeValue::Begun(Ok(draft)),
        );
        assert!(s.draft.is_some());
        assert!(s.paused_at.is_none());
        assert!(s.stop_error.is_none());
    }
    #[tokio::test]
    async fn rejected_resume_during_a_new_edit_revokes_the_old_source_before_retry() {
        let (actor, mut s, _) = actor(None);
        s.draft = None;
        let mut r = State::new();
        r.block.observe("旧尾部".into(), 3).unwrap();
        let old = r.block.snapshot(true).unwrap();
        let turn = s.turn;
        let activity = s.activity;
        native_done(
            &actor,
            &mut s,
            &mut r,
            &NativeKind::Begin { turn, activity },
            NativeValue::Begun(Err(Error::new("editing"))),
        );
        assert!(s.handoff);
        assert!(!r.block.matches(&old));
        assert!(!r.permit.load(Ordering::Acquire));
        assert!(s.stop_error.is_none());
        schedule(&actor, &mut s, &mut r).unwrap();
        assert_eq!(s.turn, turn + 1);
        assert!(s.latest.text.is_empty());
    }
    #[tokio::test]
    async fn finish_intent_blocks_revisable_writes_until_final_source_arrives() {
        let (actor, mut s, native) = actor(None);
        let mut r = State::new();
        r.block.observe("尚未显示的尾部".into(), 7).unwrap();
        actor.control.request_finish();
        start_generation(&actor, &mut s, &mut r).unwrap();
        schedule(&actor, &mut s, &mut r).unwrap();
        assert!(s.unit.is_none());
        assert!(s.native.is_none());
        assert!(native.try_recv().is_err());
        ending(&mut s, false);
        schedule(&actor, &mut s, &mut r).unwrap();
        assert!(matches!(
            s.native.as_ref().unwrap().kind,
            NativeKind::Revisable(Effect::Write { .. })
        ));
    }
    #[test]
    fn ending_busy_prepare_is_quiet_but_cleanup_failure_remains_real() {
        for code in [
            "inputComposition",
            "editing",
            "targetChanged",
            "compositionCleanup",
        ] {
            let (actor, mut s, _) = actor(None);
            let mut r = State::new();
            ending(&mut s, false);
            let turn = s.turn;
            let activity = s.activity;
            native_done(
                &actor,
                &mut s,
                &mut r,
                &NativeKind::Begin { turn, activity },
                NativeValue::Begun(Err(Error::new(code))),
            );
            assert_eq!(s.stop_error.is_some(), code == "compositionCleanup");
            if code != "compositionCleanup" {
                assert!(r.stopped);
            }
        }
    }
    #[test]
    fn native_receipt_classifies_known_and_uncertain_effects_as_unverified_display() {
        for uncertain in [false, true] {
            let (actor, mut s, _) = actor(None);
            let mut r = State::new();
            r.uncertain = uncertain;
            if !uncertain {
                s.report.text = "实际写入结果".into();
            }
            let delivery = native_done(
                &actor,
                &mut s,
                &mut r,
                &NativeKind::Revisable(Effect::Close),
                NativeValue::Closed(Ok(())),
            )
            .unwrap();
            assert_eq!(
                delivery.submission,
                if uncertain {
                    Submission::Uncertain
                } else {
                    Submission::Submitted
                }
            );
            assert_eq!(
                delivery.verification,
                crate::windows_input::Verification::Unavailable
            );
            assert_eq!(delivery.stage, "composition");
        }
    }

    fn ending(s: &mut super::super::State, expired: bool) {
        let (reply, _) = oneshot::channel();
        s.ending = Some(Ending {
            mode: Finish::Commit,
            reply,
            deadline: Instant::now() + Duration::from_secs(15),
            expired,
        });
        s.stopping = true;
    }
    fn baseline(r: &mut State, text: &str) -> BlockSnapshot {
        r.handoff();
        r.block
            .rebase_retained(text.into(), text.chars().count()..text.chars().count())
            .unwrap();
        r.read_needed = false;
        r.permit = Arc::new(AtomicBool::new(true));
        let snapshot = BlockSnapshot {
            id: r.block.id().into(),
            epoch: 1,
            version: 2,
            lease: 2,
            text: text.into(),
            selection_start: text.encode_utf16().count() as u32,
            selection_end: text.encode_utf16().count() as u32,
            writable: false,
        };
        r.snapshot = Some(snapshot.clone());
        snapshot
    }
    #[tokio::test]
    async fn oversized_unconfirmed_source_is_previewed_but_not_irreversibly_delivered() {
        let (actor, mut s, _) = actor(None);
        let mut r = State::new();
        let body = "甲。".repeat(1100);
        observe(
            &actor,
            &mut s,
            &mut r,
            NativeTranscript {
                task_id: actor.task.clone(),
                turn: 1,
                sequence: 1,
                text: body.clone(),
                segments: vec![crate::contracts::processing::SourceSegment {
                    id: 1,
                    revision: 1,
                    text: body,
                    confirmed: false,
                }],
            },
        );
        assert!(r.capacity);
        assert!(!r.block.stable_covers_all());
        start_generation(&actor, &mut s, &mut r).unwrap();
        assert!(s.unit.is_none());
        assert!(s.report.text.is_empty());
        assert_eq!(r.block.visible().chars().count(), 2000);
    }
    #[tokio::test]
    async fn native_pressure_delivers_literal_protected_source_in_bounded_chunks() {
        let (actor, mut s, _) = actor(None);
        let mut r = State::new();
        let body = format!("https://example.com/{}", "x".repeat(22000));
        observe(
            &actor,
            &mut s,
            &mut r,
            NativeTranscript {
                task_id: actor.task.clone(),
                turn: 1,
                sequence: 1,
                text: body.clone(),
                segments: vec![crate::contracts::processing::SourceSegment {
                    id: 1,
                    revision: 1,
                    text: body.clone(),
                    confirmed: true,
                }],
            },
        );
        assert!(r.capacity);
        assert!(s.stop_error.is_none());
        assert_eq!(r.block.visible(), body[..14 * 1024]);
        start_generation(&actor, &mut s, &mut r).unwrap();
        assert!(matches!(s.unit, Some(UnitSlot::Ready(_, Err(_)))));
    }
    #[tokio::test]
    async fn native_budget_includes_unicode_editor_baseline_and_excludes_replaced_selection() {
        let (actor, mut s, _) = actor(None);
        s.draft = None;
        let mut r = State::new();
        let text = "🙂".repeat(1900);
        baseline(&mut r, &text);
        let body = format!("https://example.com/{}", "x".repeat(22000));
        observe(
            &actor,
            &mut s,
            &mut r,
            NativeTranscript {
                task_id: actor.task.clone(),
                turn: 1,
                sequence: 1,
                text: body.clone(),
                segments: vec![crate::contracts::processing::SourceSegment {
                    id: 1,
                    revision: 1,
                    text: body.clone(),
                    confirmed: true,
                }],
            },
        );
        assert!(r.capacity);
        assert!(s.stop_error.is_none());
        assert_eq!(r.block.visible().encode_utf16().count(), 14 * 1024);
        assert!(r.block.visible().starts_with(&text));
        assert_eq!(r.block.state_snapshot().speech_end, 14 * 1024 - 3800);
        r.handoff();
        r.block.rebase_retained(text, 1800..1900).unwrap();
        assert_eq!(r.block.native_speech_capacity(14 * 1024), 14 * 1024 - 3600);
    }
    #[tokio::test]
    async fn exhausted_native_budget_keeps_baseline_and_replays_tail_during_finish() {
        for (base, tail) in [("甲".repeat(16384), "后"), ("甲".repeat(14335), "🙂")] {
            let (actor, mut s, native) = actor(None);
            s.draft = None;
            let mut r = State::new();
            baseline(&mut r, &base);
            ending(&mut s, false);
            observe(
                &actor,
                &mut s,
                &mut r,
                NativeTranscript {
                    task_id: actor.task.clone(),
                    turn: 1,
                    sequence: 1,
                    text: tail.into(),
                    segments: vec![crate::contracts::processing::SourceSegment {
                        id: 1,
                        revision: 1,
                        text: tail.into(),
                        confirmed: true,
                    }],
                },
            );
            assert!(s.stop_error.is_none());
            assert!(r.capacity);
            assert_eq!(r.block.visible(), base);
            assert_eq!(r.block.state_snapshot().speech_end, 0);
            schedule(&actor, &mut s, &mut r).unwrap();
            let Command::Block(_, _, request, text, _, reply) = command(&native).await else {
                panic!("release baseline")
            };
            assert_eq!(request.action, BlockAction::Release);
            assert!(text.is_empty());
            reply
                .send(Ok(block_reply(&actor.task, None, true)))
                .unwrap();
            let value = SessionDraft::native_value(&mut s.native).await.unwrap();
            let completed = s.native.take().unwrap();
            native_done(&actor, &mut s, &mut r, &completed.kind, value);
            assert_eq!(s.report.text, base);
            assert_eq!(r.offset, 0);
            assert_eq!(r.block.visible(), tail);
            assert!(s.stop_error.is_none());
        }
        assert_eq!(native_capacity_boundary("🙂", 1), Some(0));
        assert_eq!(native_capacity_boundary("后", 0), Some(0));
    }
    #[tokio::test]
    async fn tracked_capacity_rollover_replays_tail_even_when_finish_has_arrived() {
        let (actor, mut s, native) = actor(None);
        let draft = s.draft.take().unwrap();
        let mut r = State::new();
        let first = "甲".repeat(1900);
        baseline(&mut r, &first);
        let speech = "乙。".repeat(100);
        let value = NativeTranscript {
            task_id: actor.task.clone(),
            turn: s.turn,
            sequence: 1,
            text: speech.clone(),
            segments: vec![crate::contracts::processing::SourceSegment {
                id: 1,
                revision: 1,
                confirmed: true,
                text: speech,
            }],
        };
        observe(&actor, &mut s, &mut r, value);
        assert!(r.capacity);
        assert_eq!(r.block.state_snapshot().speech_end, 100);
        let (revision, text) = r.block.write().unwrap();
        native_done(
            &actor,
            &mut s,
            &mut r,
            &NativeKind::Revisable(Effect::Write {
                preview: None,
                revision,
                text,
                raw: "乙。".repeat(50),
            }),
            NativeValue::Replaced(Ok(())),
        );
        start_generation(&actor, &mut s, &mut r).unwrap();
        ending(&mut s, false);
        schedule(&actor, &mut s, &mut r).unwrap();
        let Command::Block(_, _, request, _, _, reply) = command(&native).await else {
            panic!("release prior block")
        };
        assert_eq!(request.action, BlockAction::Release);
        reply
            .send(Ok(block_reply(&actor.task, None, true)))
            .unwrap();
        let value = SessionDraft::native_value(&mut s.native).await.unwrap();
        let completed = s.native.take().unwrap();
        native_done(&actor, &mut s, &mut r, &completed.kind, value);
        assert_eq!(r.offset, 100);
        assert_eq!(r.block.visible(), "乙。".repeat(50));
        assert_eq!(s.report.text, format!("{first}{}", "乙。".repeat(50)));
        // Supply the accepted new-range receipt without a live foreground HWND.
        let activity = s.activity;
        native_done(
            &actor,
            &mut s,
            &mut r,
            &NativeKind::Begin { turn: 1, activity },
            NativeValue::Begun(Ok(draft)),
        );
        schedule(&actor, &mut s, &mut r).unwrap();
        let Command::ReplaceTracked(_, body, _, id, _, reply) = command(&native).await else {
            panic!("remaining tail")
        };
        assert_eq!(body, "乙。".repeat(50));
        let mut tracked = r
            .snapshot
            .clone()
            .unwrap_or_else(|| baseline(&mut State::new(), &body));
        tracked.id = id;
        tracked.text = body;
        tracked.writable = false;
        reply.send(Ok(tracked)).unwrap();
        let value = SessionDraft::native_value(&mut s.native).await.unwrap();
        let completed = s.native.take().unwrap();
        native_done(&actor, &mut s, &mut r, &completed.kind, value);
        assert_eq!(r.block.confirmed().1, "乙。".repeat(50));
    }
    #[tokio::test]
    async fn release_failure_keeps_known_editor_body_without_claiming_clean_delivery() {
        let (actor, mut s, _) = actor(None);
        s.draft = None;
        let mut r = State::new();
        baseline(&mut r, "用户改过的正文");
        native_done(
            &actor,
            &mut s,
            &mut r,
            &NativeKind::Revisable(Effect::Release { abandon: false }),
            NativeValue::block(Err(Error::new("compositionCleanup"))),
        );
        ending(&mut s, false);
        schedule(&actor, &mut s, &mut r).unwrap();
        assert_eq!(s.report.text, "用户改过的正文");
        assert_eq!(s.report.units.last().unwrap().application, "uncertain");
        assert!(matches!(
            s.native.as_ref().unwrap().kind,
            NativeKind::Revisable(Effect::Close)
        ));
    }

    #[tokio::test]
    async fn stopping_at_a_relocated_caret_needs_no_source_insertion_permission() {
        for (commit, pending_tail) in [(false, false), (true, false), (true, true)] {
            let (actor, mut s, _) = actor(None);
            let mut r = State::new();
            r.block.observe("周三开会".into(), 4).unwrap();
            let (revision, text) = r.block.write().unwrap();
            r.block.acknowledge(revision, text.clone()).unwrap();
            r.handoff();
            if commit {
                ending(&mut s, false);
                s.latest = NativeTranscript {
                    task_id: actor.task.clone(),
                    turn: s.turn,
                    sequence: 1,
                    text: text.clone(),
                    segments: vec![crate::contracts::processing::SourceSegment {
                        id: 1,
                        revision: 1,
                        text: text.clone(),
                        confirmed: true,
                    }],
                };
                s.written_raw = text.clone();
                r.visible_coverage =
                    crate::processing::draft_source::Watermark::capture(&s.latest.segments, 4);
                if pending_tail {
                    let mut pending = s.latest.clone();
                    pending.sequence += 1;
                    pending.text.push('后');
                    pending
                        .segments
                        .push(crate::contracts::processing::SourceSegment {
                            id: 2,
                            revision: 1,
                            text: "后".into(),
                            confirmed: true,
                        });
                    r.pending_source = Some(pending);
                }
            } else {
                r.stop();
            }
            let snapshot = BlockSnapshot {
                id: r.block.id().into(),
                epoch: 1,
                version: 1,
                lease: 2,
                text: text.clone(),
                selection_start: 1,
                selection_end: 1,
                writable: false,
            };
            r.snapshot = Some(snapshot.clone());
            let deadline = r.proof_deadline;
            native_done(
                &actor,
                &mut s,
                &mut r,
                &NativeKind::Revisable(Effect::Read {
                    activity: input_state::activity(),
                }),
                NativeValue::block(Ok(block_reply(&actor.task, Some(snapshot), true))),
            );
            assert_eq!(r.read_needed, pending_tail);
            assert_eq!(r.stopped, !pending_tail);
            assert!(!r.permit.load(Ordering::Acquire));
            assert_eq!(r.proof_deadline, deadline);
            assert_eq!(r.block.confirmed().1, text);
            assert!(s.native.is_none());
        }
    }
    #[tokio::test]
    async fn edit_then_immediate_finish_keeps_baseline_without_regeneration() {
        let (actor, mut s, native) = actor(None);
        s.draft = None;
        let mut r = State::new();
        baseline(&mut r, "用户删除后留下的内容");
        ending(&mut s, false);
        schedule(&actor, &mut s, &mut r).unwrap();
        assert!(s.unit.is_none());
        let Command::Block(_, _, request, text, _, reply) = command(&native).await else {
            panic!("read-only release")
        };
        assert_eq!(request.action, BlockAction::Release);
        assert!(text.is_empty());
        reply
            .send(Ok(block_reply(&actor.task, None, true)))
            .unwrap();
        let value = SessionDraft::native_value(&mut s.native).await.unwrap();
        let completed = s.native.take().unwrap();
        native_done(&actor, &mut s, &mut r, &completed.kind, value);
        schedule(&actor, &mut s, &mut r).unwrap();
        assert_eq!(s.report.text, "用户删除后留下的内容");
        assert!(s.unit.is_none());
    }
    #[tokio::test]
    async fn end_deadline_stops_successful_but_repeatedly_stale_reads() {
        let (actor, mut s, _) = actor(None);
        s.draft = None;
        let mut r = State::new();
        let current = baseline(&mut r, "最后确认正文");
        r.handoff();
        let stale_activity = input_state::activity().wrapping_sub(1);
        native_done(
            &actor,
            &mut s,
            &mut r,
            &NativeKind::Revisable(Effect::Read {
                activity: stale_activity,
            }),
            NativeValue::block(Ok(block_reply(&actor.task, Some(current), true))),
        );
        assert!(r.read_needed);
        ending(&mut s, true);
        r.expire();
        schedule(&actor, &mut s, &mut r).unwrap();
        assert!(matches!(
            s.native.as_ref().unwrap().kind,
            NativeKind::Revisable(Effect::Close)
        ));
        assert!(
            s.report.text.is_empty(),
            "cannot claim a stale read as current editor body"
        );
    }
}
