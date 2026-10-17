//! Input retirement is serialized by SessionDraft; recognition retains its turn.
use super::*;

pub(super) enum InputDelivery {
    Live,
    Retiring,
    Result(Delivery),
}
impl InputDelivery {
    pub(super) fn live(&self) -> bool {
        matches!(self, Self::Live)
    }
}

impl SessionDraft {
    pub(super) fn detach_failed_input(&self, s: &mut State) {
        if !s.input_delivery.live()
            || s.stop_error.is_none()
            || s.stopping && s.ending.as_ref().is_none_or(|e| e.mode != Finish::Commit)
        {
            return;
        }
        s.input_delivery = InputDelivery::Retiring;
        self.cancel_generation(s);
        if let Some(revised) = &mut s.revisable {
            revised.stop();
        }
        if let Some(NativeJob {
            kind: NativeKind::Settle { permit, .. },
            ..
        }) = &s.native
        {
            permit.store(false, Ordering::Release);
        }
        s.handoff = false;
        s.paused_at = None;
        s.report.result_only = true;
        self.control.detach_input();
        log::info!(target:"sayweave", "operation=draft_delivery_detached task_id={} turn={} reason={} audio_route={}", self.task,s.turn,s.stop_error.as_deref().unwrap_or("none"),self.control.audio.current());
    }

    pub(super) fn schedule_delivery_retirement(&self, s: &mut State) {
        if s.native.is_some() || !matches!(s.input_delivery, InputDelivery::Retiring) {
            return;
        }
        if let Some(draft) = s.draft.take() {
            s.native = Some(NativeJob {
                kind: NativeKind::RetireHandoff,
                job: tokio::spawn(
                    async move { NativeValue::HandedOff(draft.handoff_receipt().await) },
                ),
            });
        } else {
            let connection = self.connection.clone();
            let budget = s.ending.as_ref().map_or(Duration::from_secs(15), |e| {
                e.deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(15))
            });
            s.native = Some(NativeJob {
                kind: NativeKind::RetireClose,
                job: tokio::spawn(async move {
                    NativeValue::Closed(
                        tokio::time::timeout(budget, connection.close())
                            .await
                            .unwrap_or_else(|_| Err(Error::new("compositionTimeout"))),
                    )
                }),
            });
        }
    }

    pub(super) fn delivery_retired(&self, s: &mut State, kind: &NativeKind, value: NativeValue) {
        match (kind, value) {
            (NativeKind::RetireHandoff, NativeValue::HandedOff(outcome)) => {
                if !outcome.handed_off {
                    s.record_failure(&self.task, "compositionCleanup".into());
                }
                s.last_handoff = Some(outcome);
            }
            (NativeKind::RetireClose, NativeValue::Closed(result)) => {
                let failed = result.is_err();
                if let Err(error) = result {
                    windows_input::composition::retain_failed(&self.connection);
                    s.record_failure(&self.task, error.code);
                }
                if let Some(mut revised) = s.revisable.take() {
                    revisable::archive_detached(self, s, &mut revised);
                    s.revisable = Some(revised);
                } else {
                    let raw = s.written_raw.clone();
                    let text = format!(
                        "{}{}{}",
                        s.submitted,
                        s.separator,
                        units::slice(&raw, s.consumed..raw.chars().count().max(s.consumed))
                    );
                    self.archive_turn(s, &raw, &text);
                }
                let mut delivery = closed_delivery(s.last_handoff.take(), false, failed, false);
                if let Some(revised) = &s.revisable {
                    revised.preserve_submission(&mut delivery);
                }
                if s.revisable
                    .as_ref()
                    .is_some_and(revisable::State::uncertain)
                {
                    delivery.submission = Submission::Uncertain;
                    delivery.state = "unconfirmed".into();
                }
                s.input_delivery = InputDelivery::Result(delivery);
            }
            _ => s.record_failure(&self.task, "compositionProtocol".into()),
        }
    }

    pub(super) fn result_delivery(&self, s: &mut State) -> Option<Delivery> {
        let InputDelivery::Result(delivery) = &s.input_delivery else {
            return None;
        };
        let ending = s.ending.as_ref()?;
        s.report.cancelled = ending.mode == Finish::Abort || *self.cancel.borrow() >= 2;
        if !s.report.cancelled {
            s.report.result_raw = self
                .control
                .latest_raw()
                .map(|raw| raw.text)
                .unwrap_or_else(|| s.latest.text.clone());
        }
        let mut delivery = delivery.clone();
        if s.report.cancelled {
            delivery.error = Some("cancelled".into());
        }
        Some(delivery)
    }
}
