//! One serial owner. Network/native effects complete back into this loop.
#[path = "revisable.rs"]
mod revisable;
#[cfg(test)]
#[path = "drafts_tests.rs"]
mod tests;
use super::{Presentation, processing_context::Context};
use crate::{
    history::types::Error,
    input_state,
    processing::{
        quality,
        units::{self, Unit},
    },
    speech::{NativeTranscript, continuous::Control},
    windows_input::{
        self, Anchor, Delivery, Submission,
        composition::{Connection, Draft, Finish, FinishOutcome, PrefixOutcome},
    },
};
use sayweave_text_service::{settlement_boundary::SettlementBoundary, wire::Mutation};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::Instant;
type Generation =
    Result<crate::contracts::processing::GeneratedText, crate::models::types::ModelError>;
fn normal_preparation_refusal(code: &str) -> bool {
    matches!(
        code,
        "targetReadonlyContext"
            | "targetChanged"
            | "targetLost"
            | "targetClosed"
            | "editing"
            | "inputComposition"
            | "staleSession"
            | "staleSnapshot"
    )
}
fn finish_for_signal(mode: Finish, signal: u8) -> Finish {
    if mode == Finish::Abort {
        return mode;
    }
    match signal {
        2 => Finish::Abort,
        3 => Finish::Handoff("interrupted".into()),
        _ => mode,
    }
}

#[cfg(test)]
#[test]
fn interruption_and_cancel_have_order_independent_finish_semantics() {
    assert_eq!(
        finish_for_signal(Finish::Commit, 3),
        Finish::Handoff("interrupted".into())
    );
    assert_eq!(
        finish_for_signal(Finish::Handoff("interrupted".into()), 3),
        Finish::Handoff("interrupted".into())
    );
    assert_eq!(finish_for_signal(Finish::Commit, 2), Finish::Abort);
    assert_eq!(finish_for_signal(Finish::Abort, 2), Finish::Abort);
    assert_eq!(finish_for_signal(Finish::Abort, 3), Finish::Abort);
    assert_eq!(finish_for_signal(Finish::Commit, 1), Finish::Commit);
}
fn closed_delivery(
    handoff: Option<FinishOutcome>,
    cancelled: bool,
    failed: bool,
    intact: bool,
) -> Delivery {
    let (mut delivery, handed_off) = handoff
        .map(|o| (o.delivery, o.handed_off))
        .unwrap_or_else(|| (Delivery::not_submitted("editing"), false));
    if failed {
        delivery.error = Some("compositionCleanup".into());
    } else if handed_off && intact {
        if cancelled {
            delivery.error = Some("cancelled".into());
        } else if delivery.submission == crate::windows_input::Submission::Submitted {
            delivery.error = Some("inputUnconfirmed".into());
        }
    }
    delivery
}
#[cfg(test)]
#[test]
fn ending_after_user_handoff_never_promotes_an_unknown_input_or_adds_normal_notice() {
    let receipt = |handed_off| {
        let mut delivery = Delivery::not_submitted("editing");
        delivery.stage = "composition".into();
        delivery.state = "unconfirmed".into();
        delivery.submission = crate::windows_input::Submission::Submitted;
        FinishOutcome {
            delivery,
            committed: false,
            handed_off,
            cancelled: false,
            rollback_failed: false,
        }
    };
    let done = closed_delivery(Some(receipt(true)), false, false, true);
    assert_eq!(
        super::presentation::final_presentation(false, "text", Some(&done), None, false),
        Presentation::Completed
    );
    let cancelled = closed_delivery(Some(receipt(true)), true, false, true);
    assert_eq!(
        super::presentation::final_presentation(
            false,
            "text",
            Some(&cancelled),
            Some("cancelled"),
            false
        ),
        Presentation::Idle
    );
    let uncertain = closed_delivery(Some(receipt(false)), false, false, true);
    assert_eq!(uncertain.error.as_deref(), Some("editing"));
    let failed = closed_delivery(Some(receipt(true)), true, true, true);
    assert_eq!(failed.error.as_deref(), Some("compositionCleanup"));
    let empty = closed_delivery(None, false, false, true);
    assert_eq!(
        empty.submission,
        crate::windows_input::Submission::NotSubmitted
    );
}
#[derive(Clone)]
pub(crate) struct UnitResult {
    pub unit: Unit,
    pub candidate: Option<String>,
    pub generation: String,
    pub application: String,
    pub error: Option<String>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub elapsed_ms: u64,
    pub source_committed_end: Option<usize>,
    pub applied_text: Option<String>,
    pub retained_separator: String,
}
fn settle_separators(units: &mut [UnitResult], turn: u64) {
    for unit in units
        .iter_mut()
        .filter(|u| u.unit.turn == turn && !u.retained_separator.is_empty())
    {
        unit.applied_text
            .get_or_insert_with(String::new)
            .push_str(&unit.retained_separator);
        unit.retained_separator.clear();
        unit.application = "applied".into();
        unit.source_committed_end = Some(unit.unit.chars.end);
    }
}
#[cfg(test)]
#[test]
fn separator_only_unit_keeps_its_source_when_a_later_prefix_is_submitted() {
    let record =
        |id: u64, start: usize, text: &str, application: &str, applied: Option<&str>| UnitResult {
            unit: Unit {
                draft: None,
                id: id.to_string(),
                turn: 1,
                chars: start..start + text.chars().count(),
                source: vec![crate::processing::units::SourceSpan {
                    segment: id,
                    revision: 1,
                    chars: 0..text.chars().count(),
                }],
                text: text.into(),
                preserve_raw: false,
                final_source: false,
            },
            candidate: None,
            generation: "skipped".into(),
            application: application.into(),
            error: None,
            input_tokens: None,
            output_tokens: None,
            elapsed_ms: 0,
            source_committed_end: None,
            applied_text: applied.map(str::to_owned),
            retained_separator: "\n".into(),
        };
    let mut records = vec![
        record(1u64, 0, "正文\n", "applied", Some("正文")),
        record(2, 3, "\n", "pending", None),
    ];
    settle_separators(&mut records, 1);
    assert_eq!(records[0].applied_text.as_deref(), Some("正文\n"));
    assert_eq!(records[0].source_committed_end, Some(3));
    assert_eq!(records[1].applied_text.as_deref(), Some("\n"));
    assert_eq!(records[1].source_committed_end, Some(4));
    assert_eq!(records[1].application, "applied");
    settle_separators(&mut records, 1);
    assert_eq!(records[0].applied_text.as_deref(), Some("正文\n"));
}
#[derive(Clone, Default)]
pub(crate) struct Report {
    pub revisable: bool,
    pub text: String,
    pub raw: String,
    pub recovery_raw: String,
    pub units: Vec<UnitResult>,
    pub error: Option<String>,
    pub cancelled: bool,
    pub rollback_failed: bool,
}
impl Report {
    pub(crate) fn cancellation_raw(&self) -> &str {
        if self.rollback_failed || self.revisable && !self.recovery_raw.is_empty() {
            &self.recovery_raw
        } else {
            &self.raw
        }
    }
}
struct Generating {
    unit: Unit,
    job: tokio::task::JoinHandle<Generation>,
}
impl Drop for Generating {
    fn drop(&mut self) {
        self.job.abort();
    }
}
enum UnitSlot {
    Generating(Generating),
    Ready(Unit, Generation),
}
impl UnitSlot {
    fn source(&self) -> &Unit {
        match self {
            Self::Generating(g) => &g.unit,
            Self::Ready(u, _) => u,
        }
    }
}
enum NativeKind {
    Revisable(revisable::Effect),
    Replace {
        sequence: u64,
        raw: String,
    },
    Settle {
        record: Box<UnitResult>,
        boundary: SettlementBoundary,
        sequence: u64,
        raw: String,
        source_end: usize,
        permit: Arc<AtomicBool>,
    },
    Begin {
        turn: u64,
        activity: u64,
    },
    Handoff,
    Final {
        mode: Finish,
        raw: String,
        text: String,
    },
    Close,
}
enum NativeValue {
    Reacquired(Result<windows_input::composition::ReacquireOutcome, Error>),
    Block(Result<Box<sayweave_text_service::wire::Reply>, Error>),
    Replaced(Result<(), Error>),
    Settled(PrefixOutcome),
    Begun(Result<Arc<Draft>, Error>),
    HandedOff(FinishOutcome),
    Finalized(FinishOutcome),
    Closed(Result<(), Error>),
}
impl NativeValue {
    fn block(result: Result<sayweave_text_service::wire::Reply, Error>) -> Self {
        Self::Block(result.map(Box::new))
    }
}
struct NativeJob {
    kind: NativeKind,
    job: tokio::task::JoinHandle<NativeValue>,
}
impl Drop for NativeJob {
    fn drop(&mut self) {
        self.job.abort();
    }
}
struct End {
    latest: NativeTranscript,
    mode: Finish,
    reply: oneshot::Sender<Delivery>,
}
struct Ending {
    mode: Finish,
    reply: oneshot::Sender<Delivery>,
    deadline: tokio::time::Instant,
    expired: bool,
}
struct State {
    revisable: Option<revisable::State>,
    draft: Option<Arc<Draft>>,
    turn: u64,
    activity: u64,
    latest: NativeTranscript,
    written_sequence: u64,
    written_raw: String,
    consumed: usize,
    committed_source: usize,
    submitted: String,
    separator: String,
    paused_at: Option<Instant>,
    merge_at: Option<Instant>,
    dispatch_diagnostic: Option<(u64, &'static str)>,
    last_write: Option<Instant>,
    stopping: bool,
    handoff: bool,
    last_handoff: Option<FinishOutcome>,
    stop_error: Option<String>,
    secondary_errors: Vec<String>,
    model_disabled: bool,
    unit: Option<UnitSlot>,
    native: Option<NativeJob>,
    ending: Option<Ending>,
    report: Report,
}
impl State {
    fn record_failure(&mut self, task: &str, error: String) {
        if let Some(first) = &self.stop_error {
            if first != &error
                && self.secondary_errors.len() < 8
                && !self.secondary_errors.contains(&error)
            {
                log::info!(target:"sayweave", "operation=dictation_secondary_error task_id={task} turn={} first={first} secondary={error}", self.turn);
                self.secondary_errors.push(error);
            }
        } else {
            log::info!(target:"sayweave", "operation=dictation_first_error task_id={task} turn={} error={error}", self.turn);
            self.stop_error = Some(error);
        }
    }
    fn terminal_failure(&mut self, task: &str, delivery: &mut Delivery) {
        if let Some(first) = self.stop_error.clone() {
            if let Some(secondary) = delivery.error.clone()
                && !matches!(secondary.as_str(), "inputUnconfirmed" | "cancelled")
            {
                self.record_failure(task, secondary);
            }
            self.report.error = Some(first.clone());
            // Keep actual rollback/uncertain effects and a winning cancellation.
            // Ordinary teardown refusal is secondary to the failed live update.
            if !self.report.cancelled
                && !delivery.restore_error
                && delivery.submission != Submission::Uncertain
            {
                delivery.error = Some(first);
            }
        }
    }
}
#[derive(Default)]
struct View {
    turn: u64,
    paused: bool,
    report: Option<Report>,
}
pub(crate) struct SessionDraft {
    connection: Arc<Connection>,
    task: String,
    anchor: Anchor,
    control: Control,
    cancel: watch::Sender<u8>,
    context: Option<Context>,
    latest: watch::Sender<NativeTranscript>,
    ends: mpsc::Sender<End>,
    initial: Mutex<Option<(State, mpsc::Receiver<End>)>>,
    view: Mutex<View>,
    record: Mutex<Option<Arc<crate::history::RecordLease>>>,
}
impl SessionDraft {
    pub fn new(
        task: String,
        anchor: Anchor,
        draft: Arc<Draft>,
        control: Control,
        cancel: watch::Sender<u8>,
        context: Option<Context>,
        record: Option<Arc<crate::history::RecordLease>>,
    ) -> Arc<Self> {
        let (latest, _) = watch::channel(NativeTranscript::default());
        let (ends, receiver) = mpsc::channel(1);
        Arc::new(Self {
            connection: draft.connection(),
            task,
            anchor: anchor.clone(),
            control,
            cancel,
            context,
            latest,
            ends,
            initial: Mutex::new(Some((
                State {
                    revisable: revisable::State::configured(),
                    draft: Some(draft),
                    turn: 1,
                    activity: anchor.activity,
                    latest: NativeTranscript::default(),
                    written_sequence: 0,
                    written_raw: String::new(),
                    consumed: 0,
                    committed_source: 0,
                    submitted: String::new(),
                    separator: String::new(),
                    paused_at: None,
                    merge_at: None,
                    dispatch_diagnostic: None,
                    last_write: None,
                    stopping: false,
                    handoff: false,
                    last_handoff: None,
                    stop_error: None,
                    secondary_errors: Vec::new(),
                    model_disabled: false,
                    unit: None,
                    native: None,
                    ending: None,
                    report: Report::default(),
                },
                receiver,
            ))),
            view: Mutex::new(View {
                turn: 1,
                ..View::default()
            }),
            record: Mutex::new(record),
        })
    }
    pub fn presentation(&self) -> Presentation {
        self.view
            .lock()
            .map(|v| {
                if v.paused {
                    Presentation::Editing
                } else if *self.control.readiness().borrow() != v.turn {
                    Presentation::Resuming
                } else {
                    Presentation::Recording
                }
            })
            .unwrap_or(Presentation::Editing)
    }
    pub fn activate(&self) {
        self.control.draft_started(self.anchor.activity);
    }
    pub fn update(&self, value: &NativeTranscript) {
        if value.task_id == self.task {
            self.latest.send_replace(value.clone());
        }
    }
    pub fn report(&self) -> Option<Report> {
        self.view.lock().ok().and_then(|v| v.report.clone())
    }
    pub async fn archive(&self) -> Result<Option<(Report, bool)>, Error> {
        let report = self.report();
        let mut persisted = false;
        if let (Some(context), Some(report)) = (&self.context, &report) {
            persisted = context.finish(report, &self.control.source_turns()).await?;
        }
        Ok(report.map(|report| (report, persisted)))
    }
    pub async fn stage_cancel(&self) -> Result<(), Error> {
        if let (Some(context), Some(report)) = (&self.context, self.report()) {
            context.stage_cancel(&report).await?;
        }
        Ok(())
    }
    pub fn release_record(&self) {
        if let Ok(mut record) = self.record.lock() {
            record.take();
        }
    }
    fn project(&self, s: &State) {
        if let Ok(mut v) = self.view.lock() {
            v.turn = s.turn;
            v.paused = s.paused_at.is_some() || s.handoff;
        }
    }
    fn observe(&self, s: &mut State, value: NativeTranscript) {
        if let Some(mut revised) = s.revisable.take() {
            revisable::observe(self, s, &mut revised, value);
            s.revisable = Some(revised);
            return;
        }
        if value.task_id != self.task
            || value.turn != s.turn
            || value.sequence <= s.latest.sequence
            || s.activity != input_state::activity()
            || s.handoff
            || s.draft.is_none()
        {
            return;
        }
        if value.text.chars().count() < s.consumed {
            s.record_failure(&self.task, "sourceChanged".into());
            return;
        }
        // Keep the frozen source identity of every irreversible prefix, not
        // merely its length. Never reinterpret an old commit as revised ASR.
        let source = units::SourceIndex::new(&value.segments);
        if s.report
            .units
            .iter()
            .filter(|u| u.application == "applied" && u.unit.turn == s.turn)
            .any(|u| !u.unit.matches_in(s.turn, &source))
        {
            s.record_failure(&self.task, "sourceChanged".into());
            return;
        }
        if s.unit
            .as_ref()
            .is_some_and(|u| !u.source().matches_in(s.turn, &source))
        {
            self.cancel_generation(s);
        }
        if let Some(NativeJob {
            kind: NativeKind::Settle { record, permit, .. },
            ..
        }) = &s.native
            && !record.unit.matches_in(s.turn, &source)
        {
            permit.store(false, Ordering::Release);
            self.cancel_generation(s); // Lookahead depended on this prefix.
        }
        if value
            .segments
            .iter()
            .take_while(|seg| seg.confirmed)
            .map(|seg| seg.text.chars().count())
            .sum::<usize>()
            > s.consumed
            && s.merge_at.is_none()
        {
            s.merge_at = Some(Instant::now());
        }
        s.latest = value;
    }
    fn tail(s: &State) -> String {
        format!(
            "{}{}",
            s.separator,
            units::slice(&s.latest.text, s.consumed..s.latest.text.chars().count())
        )
    }
    fn cancel_generation(&self, s: &mut State) {
        if let Some(mut revised) = s.revisable.take() {
            revisable::cancel(self, s, &mut revised);
            s.revisable = Some(revised);
            return;
        }
        if let Some(UnitSlot::Generating(g)) = s.unit.take()
            && let Some(c) = &self.context
        {
            c.cancel(&g.unit);
        }
    }
    fn request_handoff(&self, s: &mut State) {
        if s.activity == input_state::activity() {
            return;
        }
        self.control.handoff_started();
        if s.revisable.is_some() {
            self.control.hold_input();
        } else {
            self.control.pause();
        }
        self.cancel_generation(s);
        if let Some(revised) = &mut s.revisable {
            revised.handoff();
        }
        s.handoff = true;
        s.paused_at = Some(Instant::now());
        if let Some(NativeJob {
            kind: NativeKind::Settle { permit, .. },
            ..
        }) = &s.native
        {
            permit.store(false, Ordering::Release);
        }
    }
    fn archive_turn(&self, s: &mut State, raw: &str, text: &str) {
        if raw.is_empty() && text.is_empty() {
            return;
        }
        if !s.report.raw.is_empty() {
            s.report.raw.push('\n');
        }
        if !s.report.text.is_empty() {
            s.report.text.push('\n');
        }
        s.report.raw.push_str(raw);
        s.report.text.push_str(text);
    }
    fn reset_turn(&self, s: &mut State) {
        s.draft = None;
        s.turn += 1;
        s.activity = input_state::activity();
        s.latest = NativeTranscript::default();
        s.written_sequence = 0;
        s.written_raw.clear();
        s.consumed = 0;
        s.committed_source = 0;
        s.submitted.clear();
        s.separator.clear();
        s.merge_at = None;
        s.last_write = None;
        s.handoff = false;
    }
    fn start_generation(&self, s: &mut State) -> Result<(), Error> {
        if let Some(mut revised) = s.revisable.take() {
            let result = revisable::start_generation(self, s, &mut revised);
            s.revisable = Some(revised);
            return result;
        }
        if s.stop_error.is_some()
            || s.ending
                .as_ref()
                .is_some_and(|end| end.mode != Finish::Commit)
        {
            return Ok(());
        }
        let pressure = Self::tail(s).encode_utf16().count() >= 14 * 1024;
        if pressure
            && matches!(s.unit, Some(UnitSlot::Generating(_)))
            && let Some(UnitSlot::Generating(g)) = s.unit.take()
        {
            if let Some(c) = &self.context {
                c.cancel(&g.unit);
            }
            s.unit = Some(UnitSlot::Ready(
                g.unit.clone(),
                Err(crate::models::types::ModelError::new("capacity")),
            ));
        }
        if (self.context.is_none() && !pressure)
            || s.unit.is_some()
            || s.draft.is_none()
            || s.handoff
        {
            return Ok(());
        }
        let final_result = s.ending.is_some();
        if !final_result
            && !pressure
            && s.merge_at
                .is_none_or(|at| at.elapsed() < Duration::from_millis(150))
        {
            return Ok(());
        }
        let (source_start, previous) = match &s.native {
            Some(NativeJob {
                kind:
                    NativeKind::Settle {
                        record,
                        boundary,
                        permit,
                        ..
                    },
                ..
            }) => {
                if !permit.load(Ordering::Acquire)
                    || !record.unit.matches(s.turn, &s.latest.segments)
                {
                    return Ok(());
                }
                (
                    record.unit.chars.end,
                    format!(
                        "{}{}{}",
                        s.submitted, boundary.committed, boundary.retained_separator
                    ),
                )
            }
            Some(NativeJob {
                kind: NativeKind::Replace { .. },
                ..
            })
            | None => (s.consumed, s.submitted.clone()),
            _ => return Ok(()),
        };
        let Some(unit) = units::next(s.turn, &s.latest.segments, source_start, final_result)
            .map_err(|e| Error::new(e.code))?
        else {
            return Ok(());
        };
        let expired = s.ending.as_ref().is_some_and(|e| e.expired);
        if pressure || expired || s.model_disabled || unit.preserve_raw {
            s.unit = Some(UnitSlot::Ready(
                unit,
                Err(crate::models::types::ModelError::new(if pressure {
                    "capacity"
                } else if expired {
                    "timeout"
                } else if s.model_disabled {
                    "auth"
                } else {
                    "preserveRaw"
                })),
            ));
            return Ok(());
        }
        let context = self.context.as_ref().unwrap().clone();
        let request = unit.clone();
        let deadline = s
            .ending
            .as_ref()
            .map(|e| e.deadline)
            .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(15));
        let job =
            tokio::spawn(async move { context.generate(&request, &previous, deadline).await });
        // Already confirmed following text retains its original merge window;
        // it must not wait for another ASR event or the preceding native ACK.
        let following_ready = s
            .latest
            .segments
            .iter()
            .take_while(|segment| segment.confirmed || final_result)
            .map(|segment| segment.text.chars().count())
            .sum::<usize>()
            > unit.chars.end;
        if !following_ready {
            s.merge_at = None;
        }
        s.unit = Some(UnitSlot::Generating(Generating { unit, job }));
        Ok(())
    }
    fn trace_dispatch(&self, s: &mut State) {
        let reason = if s.stop_error.is_some() {
            "inputStopped"
        } else if s.handoff {
            "editingHandoff"
        } else if self.context.is_none() {
            "disabled"
        } else if s.draft.is_none() {
            "draftNotReady"
        } else if matches!(s.unit, Some(UnitSlot::Generating(_))) {
            "generating"
        } else if matches!(s.unit, Some(UnitSlot::Ready(..))) {
            "candidateWaiting"
        } else if s
            .latest
            .segments
            .iter()
            .skip_while(|v| v.confirmed)
            .any(|v| v.confirmed)
        {
            "confirmationGap"
        } else if s
            .merge_at
            .is_some_and(|at| at.elapsed() < Duration::from_millis(150))
        {
            "merging"
        } else if s.native.is_some() {
            "nativePending"
        } else {
            "awaitingConfirmedSource"
        };
        if s.dispatch_diagnostic != Some((s.turn, reason)) {
            log::info!(target:"sayweave", "operation=processing_dispatch task_id={} turn={} state={} source_sequence={} consumed={} finishing={}",self.task,s.turn,reason,s.latest.sequence,s.consumed,s.ending.is_some());
            s.dispatch_diagnostic = Some((s.turn, reason));
        }
    }
    fn prepare_settle(&self, s: &mut State, unit: Unit, result: Generation) -> Result<(), Error> {
        if s.activity != input_state::activity()
            || s.handoff
            || s.draft.is_none()
            || unit.chars.start != s.consumed
            || !unit.matches(s.turn, &s.latest.segments)
        {
            return Ok(());
        }
        let mut record = UnitResult {
            unit: unit.clone(),
            candidate: None,
            generation: "failed".into(),
            application: "pending".into(),
            error: None,
            input_tokens: None,
            output_tokens: None,
            elapsed_ms: 0,
            source_committed_end: None,
            applied_text: None,
            retained_separator: String::new(),
        };
        let output = match result {
            Ok(value) => {
                record.candidate = Some(value.text.clone());
                record.input_tokens = value.usage.input;
                record.output_tokens = value.usage.output;
                record.elapsed_ms = value.elapsed_ms;
                log::info!(target:"sayweave","operation=processing_result task_id={} unit_id={} first_output_ms={} elapsed_ms={} result=complete",self.task,unit.id,value.first_output_ms.map(|n|n.to_string()).unwrap_or_else(||"unavailable".into()),value.elapsed_ms);
                match quality::validate_output(&value.text) {
                    Ok(()) => {
                        record.generation = "success".into();
                        record.candidate = Some(value.text.clone());
                        value.text
                    }
                    Err(error) => {
                        record.error = Some(error.code.into());
                        unit.text.clone()
                    }
                }
            }
            Err(error) => {
                if error.code == "auth" {
                    s.model_disabled = true;
                }
                record.generation = if ["preserveRaw", "capacity"].contains(&error.code.as_str()) {
                    "skipped"
                } else {
                    "failed"
                }
                .into();
                record.error = Some(error.code);
                unit.text.clone()
            }
        };
        log::info!(target:"sayweave","operation=processing_acceptance task_id={} unit_id={} generation={} error={}",self.task,unit.id,record.generation,record.error.as_deref().unwrap_or("none"));
        let source_body = unit.text.trim_end_matches(['\r', '\n']);
        let suffix = &unit.text[source_body.len()..];
        let output = quality::retain_boundaries(&unit.text, &output);
        let Some(boundary) = SettlementBoundary::prepare(&s.separator, &output) else {
            record.retained_separator = output.clone();
            s.separator.push_str(&output);
            s.consumed = unit.chars.end;
            s.report.units.push(record);
            return Ok(());
        };
        let tail = boundary.tail(&units::slice(
            &s.latest.text,
            unit.chars.end..s.latest.text.chars().count(),
        ));
        let permit = Arc::new(AtomicBool::new(true));
        let permission = permit.clone();
        let draft = s.draft.as_ref().unwrap().clone();
        let sequence = s.latest.sequence;
        let prefix = boundary.committed.clone();
        let source_end = unit.chars.end - suffix.chars().count();
        let prefix_source = units::slice(&s.latest.text, s.committed_source..source_end);
        let tail_source = units::slice(&s.latest.text, source_end..s.latest.text.chars().count());
        let job = tokio::spawn(async move {
            NativeValue::Settled(
                draft
                    .settle_with_source(
                        sequence,
                        prefix,
                        tail,
                        Some((prefix_source, tail_source)),
                        permission,
                    )
                    .await,
            )
        });
        s.native = Some(NativeJob {
            kind: NativeKind::Settle {
                record: Box::new(record),
                boundary,
                sequence,
                raw: s.latest.text.clone(),
                source_end: unit.chars.end - suffix.chars().count(),
                permit,
            },
            job,
        });
        Ok(())
    }
    fn schedule(&self, s: &mut State) -> Result<(), Error> {
        if let Some(mut revised) = s.revisable.take() {
            let result = revisable::schedule(self, s, &mut revised);
            s.revisable = Some(revised);
            return result;
        }
        if s.native.is_some() {
            return Ok(());
        }
        if s.handoff {
            if let Some(draft) = s.draft.take() {
                let job =
                    tokio::spawn(
                        async move { NativeValue::HandedOff(draft.handoff_receipt().await) },
                    );
                s.native = Some(NativeJob {
                    kind: NativeKind::Handoff,
                    job,
                });
            } else {
                s.activity = input_state::activity();
                s.handoff = false;
            }
            return Ok(());
        }
        if s.ending.as_ref().is_some_and(|e| e.mode != Finish::Commit)
            || s.stop_error.is_some() && s.ending.is_some()
        {
            return self.schedule_final(s);
        }
        if matches!(s.unit, Some(UnitSlot::Ready(..)))
            && let Some(UnitSlot::Ready(unit, result)) = s.unit.take()
        {
            return self.prepare_settle(s, unit, result);
        }
        if s.ending.is_some() {
            self.start_generation(s)?;
            if matches!(s.unit, Some(UnitSlot::Ready(..)))
                && let Some(UnitSlot::Ready(unit, result)) = s.unit.take()
            {
                return self.prepare_settle(s, unit, result);
            }
            if s.unit.is_none() {
                return self.schedule_final(s);
            }
            return Ok(());
        }
        if s.stopping || self.control.stopping() {
            return Ok(());
        }
        if s.draft.is_none() {
            if s.paused_at
                .is_some_and(|at| at.elapsed() >= Duration::from_millis(300))
                && !input_state::editing_key_held()
            {
                let anchor = windows_input::reanchor_turn(&self.task, &self.anchor)?;
                let connection = self.connection.clone();
                let turn = s.turn;
                let activity = s.activity;
                let job = tokio::spawn(async move {
                    NativeValue::Begun(connection.begin_turn(turn, anchor).await)
                });
                s.native = Some(NativeJob {
                    kind: NativeKind::Begin { turn, activity },
                    job,
                });
            }
            return Ok(());
        }
        if s.latest.text == s.written_raw {
            // Confirmation/source changes were already consumed by observe and
            // dispatch; only omit an identical body replacement.
            s.written_sequence = s.latest.sequence;
        }
        if s.latest.sequence != s.written_sequence
            && s.last_write
                .is_none_or(|at| at.elapsed() >= Duration::from_millis(50))
        {
            let tail = Self::tail(s);
            let draft = s.draft.as_ref().unwrap().clone();
            let sequence = s.latest.sequence;
            let source = units::slice(
                &s.latest.text,
                s.committed_source..s.latest.text.chars().count(),
            );
            let job = tokio::spawn(async move {
                NativeValue::Replaced(
                    draft
                        .replace_with_source(sequence, tail, Some(source))
                        .await,
                )
            });
            s.native = Some(NativeJob {
                kind: NativeKind::Replace {
                    sequence,
                    raw: s.latest.text.clone(),
                },
                job,
            });
        }
        Ok(())
    }
    fn schedule_final(&self, s: &mut State) -> Result<(), Error> {
        self.cancel_generation(s);
        let Some(ending) = &s.ending else {
            return Ok(());
        };
        let mut mode = ending.mode.clone();
        if mode == Finish::Abort {
            mode = Finish::Handoff("cancelled".into());
        }
        if let Some(error) = &s.stop_error
            && mode == Finish::Commit
        {
            mode = Finish::Handoff(error.clone());
        }
        if mode == Finish::Commit && s.written_raw.is_empty() && s.latest.text.is_empty() {
            // A resumed insertion lease with no accepted text has nothing to
            // commit. Handoff releases it without claiming a text submission
            // or an uncertain effect, and preserves the user's edited prefix.
            mode = Finish::Handoff("editing".into());
        }
        if let Some(draft) = s.draft.take() {
            let tail = if matches!(mode, Finish::Handoff(_)) {
                format!(
                    "{}{}",
                    s.separator,
                    units::slice(
                        &s.written_raw,
                        s.consumed..s.written_raw.chars().count().max(s.consumed)
                    )
                )
            } else {
                Self::tail(s)
            };
            let raw = if mode == Finish::Abort {
                units::slice(&s.written_raw, 0..s.committed_source)
            } else if matches!(mode, Finish::Handoff(_)) {
                s.written_raw.clone()
            } else {
                s.latest.text.clone()
            };
            let text = if mode == Finish::Abort {
                s.submitted.clone()
            } else {
                format!("{}{tail}", s.submitted)
            };
            let action = mode.clone();
            let tail_source = units::slice(
                &raw,
                s.committed_source..raw.chars().count().max(s.committed_source),
            );
            let job = tokio::spawn(async move {
                NativeValue::Finalized(
                    draft
                        .finish_receipt_with_source(tail, action, tail_source)
                        .await,
                )
            });
            s.native = Some(NativeJob {
                kind: NativeKind::Final { mode, raw, text },
                job,
            });
        } else {
            let connection = self.connection.clone();
            let job = tokio::spawn(async move { NativeValue::Closed(connection.close().await) });
            s.native = Some(NativeJob {
                kind: NativeKind::Close,
                job,
            });
        }
        Ok(())
    }
    fn native_done(
        &self,
        s: &mut State,
        kind: &NativeKind,
        value: NativeValue,
    ) -> Option<Delivery> {
        if let Some(mut revised) = s.revisable.take() {
            let result = revisable::native_done(self, s, &mut revised, kind, value);
            s.revisable = Some(revised);
            return result;
        }
        match (kind, value) {
            (NativeKind::Replace { sequence, raw }, NativeValue::Replaced(result)) => {
                match result {
                    Ok(()) => {
                        s.written_sequence = *sequence;
                        s.written_raw = raw.clone();
                        s.last_write = Some(Instant::now());
                    }
                    Err(e) => s.record_failure(&self.task, e.code),
                }
            }
            (
                NativeKind::Settle {
                    record,
                    boundary,
                    sequence,
                    raw,
                    source_end,
                    ..
                },
                NativeValue::Settled(outcome),
            ) => {
                if record.unit.chars.start != s.consumed || record.unit.turn != s.turn {
                    s.record_failure(&self.task, "compositionProtocol".into());
                }
                if outcome.prefix != Mutation::Changed || !outcome.tail_owned {
                    self.cancel_generation(s);
                }
                let mut record = record.as_ref().clone();
                record.error = outcome.error.clone().or(record.error);
                if outcome.prefix == Mutation::Changed {
                    record.application = "applied".into();
                    record.applied_text = Some(boundary.committed[s.separator.len()..].into());
                    settle_separators(&mut s.report.units, s.turn);
                    record.retained_separator = boundary.retained_separator.clone();
                    s.submitted.push_str(&boundary.committed);
                    s.consumed = record.unit.chars.end;
                    s.committed_source = *source_end;
                    record.source_committed_end = Some(*source_end);
                    for previous in &mut s.report.units {
                        if previous.unit.turn == s.turn && previous.application == "applied" {
                            previous.source_committed_end = Some(previous.unit.chars.end);
                        }
                    }
                    s.separator = boundary.retained_separator.clone();
                    s.written_raw = raw.clone();
                    if !record.unit.matches(s.turn, &s.latest.segments) {
                        s.record_failure(&self.task, "sourceChanged".into());
                    }
                    self.control.handoff_started();
                } else {
                    record.application = if outcome.prefix == Mutation::Uncertain {
                        s.report.recovery_raw = raw.clone();
                        "uncertain"
                    } else {
                        "notApplied"
                    }
                    .into();
                }
                log::info!(target:"sayweave","operation=processing_apply task_id={} unit_id={} turn={} prefix={:?} tail_owned={} error={}",self.task,record.unit.id,s.turn,outcome.prefix,outcome.tail_owned,outcome.error.as_deref().unwrap_or("none"));
                s.report.units.push(record);
                if outcome.tail_owned && outcome.prefix == Mutation::Changed {
                    s.separator = boundary.retained_separator.clone();
                    s.written_sequence = *sequence;
                    s.written_raw = raw.clone();
                    s.last_write = Some(Instant::now());
                    s.merge_at.get_or_insert_with(Instant::now);
                } else if outcome.prefix == Mutation::NotAttempted && s.handoff { /* Real editing owns the next transition. */
                } else if outcome.prefix == Mutation::NotAttempted
                    && outcome.error.as_deref() == Some("staleSnapshot")
                {
                    // This source operation was revoked before IPC; refreeze it.
                    s.merge_at = Some(Instant::now());
                } else {
                    s.record_failure(
                        &self.task,
                        outcome.error.unwrap_or_else(|| "compositionPartial".into()),
                    );
                }
            }
            (NativeKind::Begin { turn, activity }, NativeValue::Begun(result)) => match result {
                Ok(draft) => {
                    s.draft = Some(draft);
                    if *activity != input_state::activity() || s.stopping || self.control.stopping()
                    {
                        s.handoff = true;
                    } else {
                        s.paused_at = None;
                        self.control.resume(*turn, *activity);
                    }
                }
                Err(e)
                    if (s.stopping || self.control.stopping())
                        && normal_preparation_refusal(&e.code) =>
                {
                    // Begin has not written text. A retired preparation may
                    // reject its lease after Finish; it cannot restart or fail
                    // the already-ending gesture. Mutation/cleanup errors do
                    // not enter this branch.
                    log::info!(target:"sayweave", "operation=composition_prepare_retired task_id={} turn={} error={}", self.task, turn, e.code);
                }
                Err(e) if e.code == "inputComposition" => s.paused_at = Some(Instant::now()),
                Err(e) if e.code == "editing" => {
                    s.turn += 1;
                    s.activity = input_state::activity();
                    s.paused_at = Some(Instant::now());
                }
                Err(e) => s.record_failure(&self.task, e.code),
            },
            (NativeKind::Handoff, NativeValue::HandedOff(outcome)) => {
                if outcome.handed_off {
                    settle_separators(&mut s.report.units, s.turn);
                }
                for unit in &mut s.report.units {
                    if unit.unit.turn == s.turn && unit.application == "pending" {
                        unit.application = if outcome.handed_off {
                            "handedOff"
                        } else {
                            "uncertain"
                        }
                        .into();
                        if outcome.handed_off {
                            unit.source_committed_end = Some(unit.unit.chars.end);
                        }
                    }
                }
                if !outcome.handed_off {
                    s.record_failure(&self.task, "compositionHandoffFailed".into());
                }
                let raw = s.written_raw.clone();
                let text = format!(
                    "{}{}{}",
                    s.submitted,
                    s.separator,
                    units::slice(&raw, s.consumed..raw.chars().count().max(s.consumed))
                );
                self.archive_turn(s, &raw, &text);
                s.last_handoff = Some(outcome);
                self.reset_turn(s);
            }
            (NativeKind::Final { mode, raw, text }, NativeValue::Finalized(outcome)) => {
                let delivery = outcome.delivery;
                let retained_stop = matches!(mode,Finish::Handoff(reason) if reason=="cancelled")
                    && s.ending.as_ref().is_some_and(|e| e.mode == Finish::Abort);
                let cancelled = *mode == Finish::Abort || outcome.cancelled || retained_stop;
                if retained_stop {
                    self.archive_turn(s, raw, text);
                    s.report.cancelled = true;
                } else if cancelled {
                    if outcome.rollback_failed {
                        s.report.rollback_failed = true;
                        s.report.recovery_raw = if s.report.raw.is_empty() {
                            s.written_raw.clone()
                        } else {
                            format!("{}\n{}", s.report.raw, s.written_raw)
                        };
                    }
                    self.archive_turn(
                        s,
                        &units::slice(&s.written_raw, 0..s.committed_source),
                        &s.submitted.clone(),
                    );
                    s.report.cancelled = true;
                } else {
                    self.archive_turn(s, raw, text);
                }
                if outcome.committed {
                    settle_separators(&mut s.report.units, s.turn);
                    for unit in s.report.units.iter_mut().filter(|u| u.unit.turn == s.turn) {
                        if unit.application == "pending" {
                            unit.application = "applied".into();
                        }
                        if unit.unit.turn == s.turn && unit.application == "applied" {
                            unit.source_committed_end = Some(unit.unit.chars.end);
                        }
                    }
                }
                return Some(delivery);
            }
            (NativeKind::Close, NativeValue::Closed(result)) => {
                s.report.cancelled = s.ending.as_ref().is_some_and(|e| e.mode == Finish::Abort);
                let delivery = closed_delivery(
                    s.last_handoff.take(),
                    s.report.cancelled,
                    result.is_err(),
                    s.stop_error.is_none(),
                );
                return Some(delivery);
            }
            _ => s.record_failure(&self.task, "compositionProtocol".into()),
        }
        None
    }
    async fn native_value(native: &mut Option<NativeJob>) -> Option<NativeValue> {
        match native {
            Some(value) => (&mut value.job).await.ok(),
            None => std::future::pending().await,
        }
    }
    async fn generation_value(unit: &mut Option<UnitSlot>) -> Generation {
        match unit {
            Some(UnitSlot::Generating(value)) => (&mut value.job)
                .await
                .unwrap_or_else(|_| Err(crate::models::types::ModelError::new("interrupted"))),
            _ => std::future::pending().await,
        }
    }
    pub async fn maintain(self: Arc<Self>, publish: impl Fn(Presentation) + Send + Sync + 'static) {
        let Some((mut s, mut ends)) = self.initial.lock().ok().and_then(|mut v| v.take()) else {
            return;
        };
        let mut latest = self.latest.subscribe();
        let mut edits = input_state::subscribe_edits();
        let mut cancel = self.cancel.subscribe();
        let mut readiness = self.control.readiness();
        self.observe(&mut s, latest.borrow().clone());
        s.stopping = *cancel.borrow() > 0;
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                Some(end)=ends.recv()=>{
                    // Finish can win the select before the edit notification.
                    // Retire the user-owned range before considering final ASR.
                    self.request_handoff(&mut s);
                    self.observe(&mut s,end.latest);s.stopping=true;
                    if s.ending.is_some(){let _=end.reply.send(Delivery::not_submitted("staleSession"));}
                    else{s.ending=Some(Ending {mode:finish_for_signal(end.mode,*cancel.borrow()),reply:end.reply,deadline:tokio::time::Instant::now()+Duration::from_secs(15),expired:false});}
                },
                _=edits.changed()=>{self.request_handoff(&mut s);if !s.stopping{publish(Presentation::Editing);}},
                _=cancel.changed()=>{
                    s.stopping=*cancel.borrow()>0;
                    if *cancel.borrow()>=2 && let Some(revised)=&mut s.revisable { revised.stop(); }
                    if *cancel.borrow()>=2{self.cancel_generation(&mut s);if let Some(NativeJob {kind:NativeKind::Settle{permit,..},..})=&s.native {permit.store(false,Ordering::Release);}
                    if let Some(e)=&mut s.ending{e.mode=finish_for_signal(e.mode.clone(),*cancel.borrow());}}
                },
                _=latest.changed()=>{self.observe(&mut s,latest.borrow_and_update().clone());},
                value=Self::native_value(&mut s.native),if s.native.is_some()=>{
                    let native=s.native.take().unwrap();
                    if let Some(value)=value{
                        if let Some(mut delivery)=self.native_done(&mut s,&native.kind,value){
                            s.terminal_failure(&self.task, &mut delivery);
                            if let Ok(mut view)=self.view.lock(){view.report=Some(s.report.clone());}
                            if let Some(ending)=s.ending.take(){let _=ending.reply.send(delivery);}return;
                        }
                    }else{s.record_failure(&self.task, "compositionWorker".into());}
                },
                result=Self::generation_value(&mut s.unit),if matches!(s.unit,Some(UnitSlot::Generating(_)))=>{
                    if let Some(UnitSlot::Generating(generating))=s.unit.take(){s.unit=Some(UnitSlot::Ready(generating.unit.clone(),result));}
                },
                _=readiness.changed()=>{let _=readiness.borrow_and_update();if !s.stopping{publish(self.presentation());}},
                _=tick.tick()=>{
                    self.request_handoff(&mut s);
                    if !s.stopping&&let Err(error)=windows_input::composition_target(&self.task,&self.anchor){s.record_failure(&self.task, error.code);}
                },
            }
            if s.ending
                .as_ref()
                .is_some_and(|e| !e.expired && tokio::time::Instant::now() >= e.deadline)
            {
                if matches!(s.unit, Some(UnitSlot::Generating(_)))
                    && let Some(UnitSlot::Generating(g)) = s.unit.take()
                {
                    if let Some(c) = &self.context {
                        c.cancel(&g.unit);
                    }
                    s.unit = Some(UnitSlot::Ready(
                        g.unit.clone(),
                        Err(crate::models::types::ModelError::new("timeout")),
                    ));
                }
                if let Some(e) = &mut s.ending {
                    e.expired = true;
                }
                if let Some(revised) = &mut s.revisable {
                    revised.expire();
                }
            }
            if !s.stopping || s.ending.is_some() {
                if let Err(error) = self.start_generation(&mut s) {
                    s.record_failure(&self.task, error.code);
                }
                if let Err(error) = self.schedule(&mut s) {
                    s.record_failure(&self.task, error.code);
                }
            }
            if s.stop_error.is_some() && !s.stopping {
                s.stopping = true;
                self.control.request_finish();
                self.control.pause();
                let _ = crate::task_stop::request(
                    &self.cancel,
                    crate::contracts::speech::StopReason::Finish,
                );
            }
            self.trace_dispatch(&mut s);
            self.project(&s);
        }
    }
    pub async fn finish(&self, latest: &NativeTranscript, mode: Finish) -> Delivery {
        let (reply, result) = oneshot::channel();
        if self
            .ends
            .send(End {
                latest: latest.clone(),
                mode,
                reply,
            })
            .await
            .is_err()
        {
            return Delivery::not_submitted("compositionWorker");
        }
        result
            .await
            .unwrap_or_else(|_| Delivery::not_submitted("compositionWorker"))
    }
}
