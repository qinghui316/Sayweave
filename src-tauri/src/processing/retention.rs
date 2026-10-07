//! Cancellation retains receipt-backed text, never pending generation payloads.
use super::{
    ProcessingError,
    units::{SourceSpan, slice},
};
use crate::contracts::processing::UnitRecord;

pub fn cancelled(
    mut unit: UnitRecord,
    applied: Option<&str>,
) -> Result<Option<UnitRecord>, ProcessingError> {
    if unit.application_state == "uncertain" {
        unit.error_code = Some("compositionUncertain".into());
        return Ok(Some(unit));
    }
    if !["applied", "handedOff"].contains(&unit.application_state.as_str()) {
        return Ok(None);
    }
    let Some(end) = unit.source_committed_end else {
        return Ok(None);
    };
    if end <= unit.source_start {
        return Ok(None);
    }
    if end > unit.source_end || end - unit.source_start > unit.source_text.chars().count() {
        return Err(ProcessingError::new("sourceChanged"));
    }
    let count = end - unit.source_start;
    unit.source_text = slice(&unit.source_text, 0..count);
    unit.source_end = end;
    let spans: Vec<SourceSpan> =
        serde_json::from_str(&unit.source_refs).map_err(|_| ProcessingError::new("invalid"))?;
    let mut remaining = count;
    let mut kept = Vec::new();
    for mut span in spans {
        if remaining == 0 {
            break;
        }
        let size = span
            .chars
            .end
            .checked_sub(span.chars.start)
            .ok_or(ProcessingError::new("invalid"))?;
        let size = size.min(remaining);
        span.chars.end = span.chars.start + size;
        if size > 0 {
            kept.push(span);
            remaining -= size;
        }
    }
    if remaining != 0 {
        return Err(ProcessingError::new("sourceChanged"));
    }
    unit.source_refs = serde_json::to_string(&kept).map_err(|_| ProcessingError::new("invalid"))?;
    // Native receipts carry the actual submitted body; it cannot be derived
    // from the original or generated string length after faithful cleanup.
    unit.candidate_text = if unit.generation_state == "success" {
        applied.map(str::to_owned)
    } else {
        None
    };
    Ok(Some(unit))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn unit() -> UnitRecord {
        UnitRecord {
            id: uuid::Uuid::new_v4().to_string(),
            run_id: uuid::Uuid::new_v4().to_string(),
            turn: 1,
            source_start: 4,
            source_end: 8,
            source_refs: serde_json::to_string(&vec![SourceSpan {
                segment: 7,
                revision: 1,
                chars: 2..6,
            }])
            .unwrap(),
            source_text: "呃🙂。\n".into(),
            candidate_text: Some("🙂。\n".into()),
            generation_state: "success".into(),
            application_state: "applied".into(),
            source_committed_end: Some(7),
            error_code: None,
            prompt_tokens: None,
            completion_tokens: None,
            elapsed_ms: 5,
        }
    }
    #[test]
    fn cancelled_prefix_trims_unicode_source_refs_and_unsubmitted_separator() {
        let kept = cancelled(unit(), Some("🙂。")).unwrap().unwrap();
        assert_eq!(kept.source_text, "呃🙂。");
        assert_eq!(kept.source_end, 7);
        assert_eq!(kept.candidate_text.as_deref(), Some("🙂。"));
        let refs: Vec<SourceSpan> = serde_json::from_str(&kept.source_refs).unwrap();
        assert_eq!(refs[0].chars, 2..5);
    }
    #[test]
    fn pending_tail_is_dropped_and_unknown_submission_is_explicit() {
        let mut pending = unit();
        pending.application_state = "pending".into();
        pending.source_committed_end = None;
        assert!(cancelled(pending.clone(), None).unwrap().is_none());
        pending.application_state = "uncertain".into();
        let recovery = cancelled(pending, None).unwrap().unwrap();
        assert_eq!(recovery.error_code.as_deref(), Some("compositionUncertain"));
    }
}
