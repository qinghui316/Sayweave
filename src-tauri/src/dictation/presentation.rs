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
    Recognizing,
    Delivering,
    Completed,
    Notice,
    Result,
    Error,
}
impl Presentation {
    pub fn hide_after(self) -> Option<u64> {
        match self {
            Self::Completed => Some(600),
            Self::Notice => Some(3500),
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
    _absent_target: bool,
    text: &str,
    delivery: Option<&Delivery>,
    error: Option<&str>,
) -> Presentation {
    if error == Some("cancelled")
        || delivery.is_some_and(|d| d.error.as_deref() == Some("cancelled"))
    {
        return Presentation::Idle;
    }
    if let Some(delivery) = delivery {
        if delivery.state == "confirmed" && !delivery.restore_error {
            return Presentation::Completed;
        }
        // Submitted input can already be visible in the target. Never offer a duplicate paste.
        if delivery.submission != Submission::NotSubmitted {
            return Presentation::Notice;
        }
    }
    if !text.trim().is_empty() {
        Presentation::Result
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
            final_presentation(false, "文字", Some(&delivery), None),
            Presentation::Result
        );
        assert_eq!(
            final_presentation(true, "文字", Some(&delivery), None),
            Presentation::Result
        );
        delivery.state = "unconfirmed".into();
        delivery.submission = Submission::Submitted;
        assert_eq!(
            final_presentation(true, "文字", Some(&delivery), None),
            Presentation::Notice
        );
        delivery.state = "confirmed".into();
        assert_eq!(
            final_presentation(false, "文字", Some(&delivery), None),
            Presentation::Completed
        );
        delivery.restore_error = true;
        assert_eq!(
            final_presentation(false, "文字", Some(&delivery), None),
            Presentation::Notice
        );
    }
    #[test]
    fn only_transient_feedback_has_an_automatic_deadline() {
        assert_eq!(Presentation::Completed.hide_after(), Some(600));
        assert_eq!(Presentation::Result.hide_after(), None);
        assert_eq!(Presentation::Error.hide_after(), None);
        assert_eq!(
            final_presentation(true, "", None, Some("cancelled")),
            Presentation::Idle
        );
    }
}
