//! Session-owned draft turns. Speech still owns the microphone and supplier connections.
use super::Presentation;
use crate::{
    history::types::Error,
    input_state,
    speech::{NativeTranscript, continuous::Control},
    windows_input::{
        self, Anchor, Delivery, DeliveryGate,
        composition::{Draft, Finish},
    },
};
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::sync::{Mutex as AsyncMutex, watch};

struct State {
    draft: Option<Arc<Draft>>,
    turn: u64,
    activity: u64,
    sequence: u64,
    text: String,
    closing: bool,
    paused_at: Option<Instant>,
    last_delivery: Option<Delivery>,
}
pub(crate) struct SessionDraft {
    input: windows_input::WindowsInputService,
    task: String,
    anchor: Anchor,
    gate: Arc<DeliveryGate>,
    control: Control,
    cancel: watch::Sender<u8>,
    state: Mutex<State>,
    operation: AsyncMutex<()>,
}
impl SessionDraft {
    pub fn presentation(&self) -> Presentation {
        self.state
            .lock()
            .map(|state| {
                if state.paused_at.is_some() {
                    Presentation::Editing
                } else if *self.control.readiness().borrow() != state.turn {
                    Presentation::Resuming
                } else {
                    Presentation::Recording
                }
            })
            .unwrap_or(Presentation::Editing)
    }
    pub fn new(
        task: String,
        anchor: Anchor,
        draft: Arc<Draft>,
        gate: Arc<DeliveryGate>,
        control: Control,
        cancel: watch::Sender<u8>,
    ) -> Arc<Self> {
        Arc::new(Self {
            input: draft.input(),
            task,
            anchor: anchor.clone(),
            gate,
            control,
            cancel,
            state: Mutex::new(State {
                draft: Some(draft),
                turn: 1,
                activity: anchor.activity,
                sequence: 0,
                text: String::new(),
                closing: false,
                paused_at: None,
                last_delivery: None,
            }),
            operation: AsyncMutex::new(()),
        })
    }
    pub fn activate(&self) {
        self.control.draft_started(self.anchor.activity);
    }
    pub fn update(&self, value: &NativeTranscript) {
        if let Ok(mut state) = self.state.lock()
            && !state.closing
            && state.turn == value.turn
            && state.activity == input_state::activity()
            && value.task_id == self.task
            && value.sequence > state.sequence
        {
            state.sequence = value.sequence;
            state.text.clone_from(&value.text);
            if let Some(draft) = &state.draft {
                draft.update(value.turn, value.sequence, &value.text);
            }
        }
    }
    fn feedback(&self, publish: &impl Fn(Presentation), presentation: Presentation) {
        if *self.cancel.borrow() == 0 {
            publish(presentation);
        }
    }
    async fn handoff(&self, activity: u64) {
        self.control.handoff_started();
        self.control.pause();
        let _operation = self.operation.lock().await;
        let draft = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state.closing {
                return;
            }
            let draft = state.draft.take();
            if let Some(draft) = &draft {
                draft.freeze();
            }
            state.activity = activity;
            state.sequence = 0;
            state.text.clear();
            state.paused_at = Some(Instant::now());
            if draft.is_some() {
                state.turn += 1;
            }
            draft
        };
        if let Some(draft) = draft {
            let delivery = draft
                .finish(String::new(), Finish::Handoff("editing".into()))
                .await;
            if let Ok(mut state) = self.state.lock() {
                state.last_delivery = Some(delivery);
            }
        }
    }
    async fn resume(&self, publish: &impl Fn(Presentation)) -> Result<(), Error> {
        let _operation = self.operation.lock().await;
        if *self.cancel.borrow() > 0 || self.control.stopping() {
            return Ok(());
        }
        let (turn, activity) = {
            let state = self.state.lock().map_err(|_| Error::new("unavailable"))?;
            if state.closing
                || state.draft.is_some()
                || state
                    .paused_at
                    .is_none_or(|at| at.elapsed() < Duration::from_millis(300))
            {
                return Ok(());
            }
            (state.turn, state.activity)
        };
        if activity != input_state::activity() {
            return Ok(());
        }
        let anchor = windows_input::reanchor_turn(&self.task, &self.anchor)?;
        self.feedback(publish, Presentation::Resuming);
        let draft = match Draft::begin(
            self.task.clone(),
            turn,
            anchor.clone(),
            self.gate.clone(),
            self.input.clone(),
        )
        .await
        {
            Ok(draft) => draft,
            // Begin does not write text. Wait for the foreign IME composition to actually
            // end; the300ms debounce alone is not permission to acquire its range.
            Err(error) if error.code == "inputComposition" => {
                if let Ok(mut state) = self.state.lock() {
                    state.paused_at = Some(Instant::now());
                }
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        if *self.cancel.borrow() > 0
            || self.control.stopping()
            || input_state::activity() != activity
        {
            let _ = draft
                .finish(String::new(), Finish::Handoff("editing".into()))
                .await;
            return Ok(());
        }
        let accepted = if let Ok(mut state) = self.state.lock() {
            if !state.closing && state.turn == turn && state.activity == activity {
                state.draft = Some(draft.clone());
                state.paused_at = None;
                true
            } else {
                false
            }
        } else {
            false
        };
        if accepted {
            self.control.resume(turn, activity);
            log::info!(target:"sayweave", "operation=dictation_turn task_id={} turn={turn} state=resuming", self.task);
        } else {
            let _ = draft
                .finish(String::new(), Finish::Handoff("editing".into()))
                .await;
        }
        Ok(())
    }
    pub async fn maintain(self: Arc<Self>, publish: impl Fn(Presentation) + Send + Sync + 'static) {
        let mut edits = input_state::subscribe_edits();
        let mut cancel = self.cancel.subscribe();
        let mut readiness = self.control.readiness();
        let mut tick = tokio::time::interval(Duration::from_millis(100));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if *cancel.borrow() > 0 || self.state.lock().is_ok_and(|s| s.closing) {
                break;
            }
            tokio::select! {
                biased;
                _=cancel.changed()=>{},
                _=edits.changed()=>{
                    let activity = input_state::activity();
                    self.feedback(&publish, Presentation::Editing);
                    self.handoff(activity).await;
                },
                _=readiness.changed()=>{
                    let turn = *readiness.borrow_and_update();
                    if turn > 0 && self.state.lock().is_ok_and(|s|s.turn==turn && s.draft.is_some()) {
                        self.feedback(&publish, Presentation::Recording);
                    }
                },
                _=tick.tick()=>{
                    if windows_input::composition_target(&self.task,&self.anchor).is_err() {
                        self.control.request_finish(); self.control.pause(); self.cancel.send_replace(1); break;
                    }
                    if self.state.lock().is_ok_and(|s|s.activity!=input_state::activity()) {
                        self.feedback(&publish, Presentation::Editing);
                        self.handoff(input_state::activity()).await;
                    }
                    let failure = self.state.lock().ok().and_then(|s|s.draft.as_ref().and_then(|d|d.failure()));
                    if let Some(error) = failure {
                        log::info!(target:"sayweave", "operation=dictation_turn task_id={} state=stopped error={error}", self.task);
                        self.control.request_finish(); self.control.pause(); self.cancel.send_replace(1); break;
                    }
                    if let Err(error) = self.resume(&publish).await {
                        log::info!(target:"sayweave", "operation=dictation_turn task_id={} state=stopped error={}", self.task,error.code);
                        self.control.request_finish(); self.control.pause(); self.cancel.send_replace(1); break;
                    }
                }
            }
        }
    }
    /// Only the current turn's cumulative text can finalize its range. Aggregate raw
    /// history text contains earlier turns and must never be pasted into this draft.
    pub async fn finish(&self, latest: &NativeTranscript, mode: Finish) -> Delivery {
        let _operation = self.operation.lock().await;
        let (draft, text, old, edited) = {
            let Ok(mut state) = self.state.lock() else {
                return Delivery::not_submitted("unavailable");
            };
            state.closing = true;
            let text = if latest.task_id == self.task
                && latest.turn == state.turn
                && state.activity == input_state::activity()
            {
                latest.text.clone()
            } else {
                state.text.clone()
            };
            (
                state.draft.take(),
                text,
                state.last_delivery.take(),
                state.activity != input_state::activity(),
            )
        };
        if let Some(draft) = draft {
            let mode = if mode == Finish::Abort && edited {
                self.control.handoff_started();
                Finish::Handoff("editing".into())
            } else {
                mode
            };
            draft.finish(text, mode).await
        } else {
            old.unwrap_or_else(|| Delivery::not_submitted("editing"))
        }
    }
}
