//! Read-only paired snapshot. UIA explains representation, never grants a range.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct VerifiedBlockContent {
    pub native_text: String,
    pub semantic_text: String,
    pub native_end: i32,
    pub selection_start: u32,
    pub selection_end: u32,
    pub content_version: u64,
}

pub(super) unsafe fn capture(
    context: &ITfContext,
    range: &ITfRange,
    field: &initial_selection::FocusIdentity,
    original: Option<&initial_selection::SemanticBoundary>,
    ec: u32,
    version: u64,
) -> windows::core::Result<(VerifiedBlockContent, initial_selection::SemanticBoundary)> {
    unsafe {
        if !field.matches()? {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let document = context.GetStart(ec)?;
        document.ShiftEndToRange(ec, &context.GetEnd(ec)?, TF_ANCHOR_END)?;
        let native_document = range_text(&document, ec)?;
        let position = range_position(context, range, ec)?;
        let native_text = range_text(range, ec)?;
        let selected = range_position(context, &selection(context, ec)?, ec)?;
        let (_, proof) = initial_selection::inspect(&native_document);
        let proof = proof.ok_or_else(|| failure(TF_E_LOCKED))?;
        if proof.start != selected.start || proof.end != selected.end {
            return Err(failure(TF_E_LOCKED));
        }
        let mapped = proof.semantic_block(position.start, position.end, original);
        proof.revalidate(&range_text(&document, ec)?)?;
        let after = range_position(context, range, ec)?;
        let selection_after = range_position(context, &selection(context, ec)?, ec)?;
        if after.start != position.start
            || after.end != position.end
            || selection_after.start != selected.start
            || selection_after.end != selected.end
            || range_text(range, ec)? != native_text
            || !field.matches()?
        {
            return Err(failure(TF_E_LOCKED));
        }
        proof.revalidate(&range_text(&document, ec)?)?;
        // A semantic boundary disagreement is permanent only after both
        // representations, selection and provider proved stable together.
        let mapped = mapped.map_err(|reason| {
            failure(match reason {
                initial_selection::BlockReadError::Unproven => TF_E_LOCKED,
                initial_selection::BlockReadError::OutsideScope => TF_E_DISCONNECTED,
            })
        })?;
        Ok((
            VerifiedBlockContent {
                native_text,
                semantic_text: mapped.text,
                native_end: mapped.native_end,
                selection_start: mapped.selection_start,
                selection_end: mapped.selection_end,
                content_version: version,
            },
            mapped.boundary,
        ))
    }
}
