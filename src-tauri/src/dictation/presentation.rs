use crate::coordinator::Phase;
use crate::windows_input::{Delivery, Submission};
use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Presentation {
    #[default]
    Idle,
    Preparing,
    Recording,
    Editing,
    Resuming,
    Recognizing,
    Delivering,
    Completed,
    Result,
    Error,
}
impl Presentation {
    pub fn hide_after(self) -> Option<u64> {
        match self {
            Self::Completed => Some(600),
            Self::Idle => Some(0),
            _ => None,
        }
    }
}
pub fn speech_presentation(phase: Phase, stop: u8) -> Option<Presentation> {
    match phase {
        Phase::Recording if stop == 0 => Some(Presentation::Recording),
        Phase::Processing | Phase::Completed => Some(Presentation::Recognizing),
        _ => None,
    }
}

pub fn final_presentation(
    absent_target: bool,
    text: &str,
    delivery: Option<&Delivery>,
    error: Option<&str>,
    explicit_end: bool,
) -> Presentation {
    let interruption = |code: &str| {
        matches!(
            code,
            "emptyText"
                | "cancelled"
                | "staleSession"
                | "targetChanged"
                | "targetLost"
                | "targetClosed"
                | "editing"
                | "targetReadonlyContext"
                | "inputComposition"
        )
    };
    let safe_delivery = delivery.is_none_or(|d| {
        !d.restore_error
            && d.submission != Submission::Uncertain
            && d.error
                .as_deref()
                .is_none_or(|e| interruption(e) || e == "inputUnconfirmed")
    });
    if safe_delivery
        && error.is_none_or(interruption)
        && (text.trim().is_empty()
            || explicit_end
                && (error.is_some_and(interruption)
                    || delivery.is_some_and(|d| d.error.as_deref().is_some_and(interruption))))
    {
        // Projection alone becomes quiet. Delivery/history retain the original
        // interruption and submission facts, including a user-owned prefix.
        return Presentation::Idle;
    }
    let quiet_empty = text.trim().is_empty()
        && error.is_none_or(|e| matches!(e, "emptyText" | "cancelled"))
        && delivery.is_none_or(|d| {
            !d.restore_error
                && d.submission != Submission::Uncertain
                && d.error
                    .as_deref()
                    .is_none_or(|e| matches!(e, "emptyText" | "cancelled"))
        });
    if quiet_empty {
        return Presentation::Idle;
    }
    let cancelled = error == Some("cancelled")
        || delivery.is_some_and(|d| d.error.as_deref() == Some("cancelled"));
    if cancelled
        && error != Some("saveFailed")
        && delivery.is_none_or(|d| {
            !d.restore_error
                && d.submission != Submission::Uncertain
                && d.error.as_deref().is_none_or(|e| e == "cancelled")
        })
    {
        // A previously submitted prefix does not turn successful tail removal
        // into an input failure. Cancellation uses the existing close animation.
        return Presentation::Idle;
    }
    // A completed TSF commit ends the gesture even without an independent
    // renderer confirmation fence. Keep that distinction in Delivery/history;
    // do not turn every normal streaming task into a persistent warning.
    if error.is_none()
        && delivery.is_some_and(|d| {
            d.stage == "composition"
                && d.submission == Submission::Submitted
                && d.error.as_deref() == Some("inputUnconfirmed")
                && !d.restore_error
        })
    {
        return Presentation::Completed;
    }
    // Real failure uses the existing error surface even when a prefix was
    // submitted. It never offers another paste of already-visible input.
    let failed = |code: &str| {
        !matches!(
            code,
            "inputUnconfirmed"
                | "cancelled"
                | "staleSession"
                | "targetChanged"
                | "targetLost"
                | "targetClosed"
                | "editing"
        )
    };
    if error.is_some_and(failed)
        || delivery.is_some_and(|d| {
            d.restore_error
                || d.submission == Submission::Uncertain
                || d.error.as_deref().is_some_and(failed)
        })
    {
        return Presentation::Error;
    }
    // Normal target interruption closes without presenting a duplicate result.
    if delivery.is_some_and(|d| {
        d.submission != Submission::NotSubmitted
            && (d.state != "confirmed" || d.error.is_some() || d.restore_error)
    }) {
        return Presentation::Idle;
    }
    if error == Some("cancelled")
        || delivery.is_some_and(|d| d.error.as_deref() == Some("cancelled"))
    {
        return Presentation::Idle;
    }
    if let Some(delivery) = delivery {
        if delivery.state == "confirmed" && delivery.error.is_none() && !delivery.restore_error {
            return Presentation::Completed;
        }
        // Submitted input can already be visible in the target. Never offer a duplicate paste.
        if delivery.submission != Submission::NotSubmitted {
            return Presentation::Idle;
        }
    }
    if !text.trim().is_empty() {
        if absent_target {
            Presentation::Result
        } else {
            Presentation::Idle
        }
    } else if error.is_some_and(|e| e != "cancelled" && e != "staleSession") {
        Presentation::Error
    } else {
        Presentation::Idle
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_or_user_ended_interruption_is_quiet_but_real_effects_are_not() {
        for code in [
            "targetReadonlyContext",
            "targetChanged",
            "editing",
            "inputComposition",
        ] {
            let mut delivery = Delivery::not_submitted(code);
            assert_eq!(
                final_presentation(false, "", Some(&delivery), Some("emptyText"), false),
                Presentation::Idle
            );
            assert_eq!(
                final_presentation(false, "用户改好的正文", Some(&delivery), None, true),
                Presentation::Idle
            );
            delivery.restore_error = true;
            assert_eq!(
                final_presentation(false, "", Some(&delivery), Some("emptyText"), true),
                Presentation::Error
            );
            delivery.restore_error = false;
            delivery.submission = Submission::Uncertain;
            assert_eq!(
                final_presentation(false, "", Some(&delivery), Some("cancelled"), true),
                Presentation::Error
            );
        }
        for code in [
            "saveFailed",
            "compositionRollback",
            "compositionCleanup",
            "credentialMissing",
            "deviceAccess",
            "network",
        ] {
            assert_eq!(
                final_presentation(false, "", None, Some(code), true),
                Presentation::Error
            );
        }
    }
    #[test]
    fn completed_stream_has_short_feedback_without_claiming_readback() {
        let mut delivery = Delivery::not_submitted("inputUnconfirmed");
        delivery.stage = "composition".into();
        delivery.state = "unconfirmed".into();
        delivery.submission = Submission::Submitted;
        assert_eq!(
            final_presentation(false, "text", Some(&delivery), None, false),
            Presentation::Completed
        );
        assert_eq!(Presentation::Completed.hide_after(), Some(600));
        delivery.error = Some("compositionRejected".into());
        assert_eq!(
            final_presentation(false, "text", Some(&delivery), None, false),
            Presentation::Error
        );
    }
    #[test]
    fn late_recording_projection_cannot_reverse_finish_or_cancel() {
        assert_eq!(
            speech_presentation(Phase::Recording, 0),
            Some(Presentation::Recording)
        );
        assert_eq!(speech_presentation(Phase::Recording, 1), None);
        assert_eq!(speech_presentation(Phase::Recording, 2), None);
        assert_eq!(
            speech_presentation(Phase::Processing, 1),
            Some(Presentation::Recognizing)
        );
    }
    #[test]
    fn unsubmitted_text_is_recoverable_and_submitted_text_is_never_repeated() {
        let mut delivery = Delivery::not_submitted("targetChanged");
        assert_eq!(
            final_presentation(false, "文字", Some(&delivery), None, false),
            Presentation::Idle
        );
        assert_eq!(
            final_presentation(true, "文字", Some(&delivery), None, false),
            Presentation::Result
        );
        delivery.state = "unconfirmed".into();
        delivery.submission = Submission::Submitted;
        assert_eq!(
            final_presentation(true, "文字", Some(&delivery), None, false),
            Presentation::Idle
        );
        delivery.state = "confirmed".into();
        delivery.error = None;
        assert_eq!(
            final_presentation(false, "文字", Some(&delivery), None, false),
            Presentation::Completed
        );
        delivery.restore_error = true;
        assert_eq!(
            final_presentation(false, "文字", Some(&delivery), None, false),
            Presentation::Error
        );
    }
    #[test]
    fn terminal_failure_is_visible_and_successful_cancel_stays_quiet() {
        let mut delivery = Delivery::not_submitted("compositionRollback");
        delivery.state = "unconfirmed".into();
        delivery.submission = Submission::Submitted;
        assert_eq!(
            final_presentation(false, "部分文字", Some(&delivery), Some("cancelled"), false),
            Presentation::Error
        );
        delivery.submission = Submission::NotSubmitted;
        delivery.error = Some("cancelled".into());
        assert_eq!(
            final_presentation(false, "", Some(&delivery), Some("cancelled"), false),
            Presentation::Idle
        );
        delivery.submission = Submission::Submitted;
        assert_eq!(
            final_presentation(
                false,
                "已提交前段",
                Some(&delivery),
                Some("cancelled"),
                false
            ),
            Presentation::Idle
        );
        assert_eq!(
            final_presentation(
                false,
                "已提交前段",
                Some(&delivery),
                Some("saveFailed"),
                false
            ),
            Presentation::Error
        );
    }
    #[test]
    fn only_transient_feedback_has_an_automatic_deadline() {
        assert_eq!(Presentation::Completed.hide_after(), Some(600));
        assert_eq!(Presentation::Result.hide_after(), None);
        assert_eq!(Presentation::Error.hide_after(), None);
        assert_eq!(
            final_presentation(true, "", None, Some("cancelled"), false),
            Presentation::Idle
        );
    }
}
