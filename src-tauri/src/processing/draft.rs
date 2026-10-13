//! Pure, revisable document state. Effects belong to the dictation actor.
//! Source versions, visible revisions and native leases are deliberately separate.
use super::{ProcessingError, quality};
pub use crate::contracts::draft::{BaselineKind, Snapshot};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

pub const BLOCK_LIMIT: usize = 2000;
pub use crate::contracts::processing::MESSAGE_LIMIT;

#[derive(Clone, Debug)]
struct Preview {
    source: Snapshot,
    text: String,
}

#[derive(Clone, Debug)]
pub struct Block {
    id: String,
    version: u64,
    source_epoch: u64,
    kind: BaselineKind,
    baseline: String,
    insertion: Range<usize>,
    speech: String,
    stable: usize,
    preview: Option<Preview>,
    attempted: Option<Snapshot>,
    confirmed_visible: String,
    confirmed_revision: u64,
    next_revision: u64,
    stopped: bool,
    suspended: bool,
}

fn slice(text: &str, range: Range<usize>) -> String {
    text.chars().skip(range.start).take(range.len()).collect()
}

fn boundary(text: &str, position: usize) -> bool {
    let mut scalars = 0;
    for part in text.graphemes(true) {
        if scalars == position {
            return true;
        }
        scalars += part.chars().count();
    }
    scalars == position
}

/// Maps only positions outside the changed interval. An ambiguous interior
/// caret must never be guessed from a repeated substring or a model response.
pub fn map_position(before: &str, after: &str, position: usize) -> Option<usize> {
    if !boundary(before, position) {
        return None;
    }
    if before == after {
        return Some(position);
    }
    let old: Vec<_> = before.graphemes(true).collect();
    let new: Vec<_> = after.graphemes(true).collect();
    let head = old.iter().zip(&new).take_while(|(a, b)| a == b).count();
    let suffix = old[head..]
        .iter()
        .rev()
        .zip(new[head..].iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let prefix_chars: usize = old[..head].iter().map(|v| v.chars().count()).sum();
    let old_end: usize = old[..old.len() - suffix]
        .iter()
        .map(|v| v.chars().count())
        .sum();
    let new_end: usize = new[..new.len() - suffix]
        .iter()
        .map(|v| v.chars().count())
        .sum();
    if position >= old_end {
        Some(new_end + position - old_end)
    } else if position <= prefix_chars {
        Some(position)
    } else {
        None
    }
}

impl Block {
    pub fn new(id: String) -> Self {
        Self {
            id,
            version: 1,
            source_epoch: 1,
            kind: BaselineKind::Asr,
            baseline: String::new(),
            insertion: 0..0,
            speech: String::new(),
            stable: 0,
            preview: None,
            attempted: None,
            confirmed_visible: String::new(),
            confirmed_revision: 0,
            next_revision: 0,
            stopped: false,
            suspended: false,
        }
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn confirmed(&self) -> (u64, &str) {
        (self.confirmed_revision, &self.confirmed_visible)
    }
    pub fn stable_covers_all(&self) -> bool {
        self.stable == self.speech.chars().count()
    }
    pub fn speech_capacity(&self) -> usize {
        BLOCK_LIMIT.saturating_sub(
            self.baseline
                .chars()
                .count()
                .saturating_sub(self.insertion.len()),
        )
    }
    pub fn discard_preview(&mut self) {
        self.preview = None;
    }
    pub fn preview_source(&self) -> Option<Snapshot> {
        self.preview.as_ref().map(|p| p.source.clone())
    }
    pub fn state_snapshot(&self) -> Snapshot {
        Snapshot {
            block: self.id.clone(),
            version: self.version,
            source_epoch: self.source_epoch,
            baseline_kind: self.kind.clone(),
            text: self.effective(self.speech.chars().count()),
            speech_end: self.speech.chars().count(),
            insertion_end: self.insertion.start + self.speech.chars().count(),
        }
    }

    /// The caller supplies the latest full ASR text in this source epoch.
    pub fn observe(&mut self, text: String, stable: usize) -> Result<(), ProcessingError> {
        if self.stopped || self.suspended {
            return Err(ProcessingError::new("staleDraft"));
        }
        if stable > text.chars().count() {
            return Err(ProcessingError::new("sourceBoundary"));
        }
        let mut safe = 0;
        let mut at = 0;
        for grapheme in text.graphemes(true) {
            at += grapheme.chars().count();
            if at > stable {
                break;
            }
            safe = at;
        }
        let stable = safe;
        if self.speech != text || self.stable != stable {
            self.version += 1;
            self.speech = text;
            self.stable = stable;
            if self
                .preview
                .as_ref()
                .is_some_and(|p| !self.matches(&p.source))
            {
                self.preview = None;
            }
        }
        Ok(())
    }

    fn effective(&self, end: usize) -> String {
        if end == 0 {
            return self.baseline.clone();
        }
        format!(
            "{}{}{}",
            slice(&self.baseline, 0..self.insertion.start),
            slice(&self.speech, 0..end),
            slice(
                &self.baseline,
                self.insertion.end..self.baseline.chars().count()
            )
        )
    }

    pub fn snapshot(&self, final_source: bool) -> Option<Snapshot> {
        if self.stopped || self.suspended {
            return None;
        }
        let end = if final_source {
            self.speech.chars().count()
        } else {
            self.stable
        };
        // An editor baseline alone is not a request to reformat user edits.
        if end == 0 {
            return None;
        }
        Some(Snapshot {
            block: self.id.clone(),
            version: self.version,
            source_epoch: self.source_epoch,
            baseline_kind: self.kind.clone(),
            text: self.effective(end),
            speech_end: end,
            insertion_end: self.insertion.start + end,
        })
    }

    pub fn matches(&self, snapshot: &Snapshot) -> bool {
        !self.stopped
            && !self.suspended
            && snapshot.block == self.id
            && snapshot.source_epoch == self.source_epoch
            && snapshot.speech_end <= self.speech.chars().count()
            && boundary(&self.speech, snapshot.speech_end)
            && snapshot.text == self.effective(snapshot.speech_end)
    }

    pub fn needs_generation(&self, snapshot: &Snapshot) -> bool {
        self.matches(snapshot)
            && self.attempted.as_ref().is_none_or(|old| {
                old.source_epoch != snapshot.source_epoch || old.text != snapshot.text
            })
    }

    pub fn attempted(&mut self, snapshot: Snapshot) {
        if self.matches(&snapshot) {
            self.attempted = Some(snapshot);
        }
    }

    pub fn accept(&mut self, snapshot: Snapshot, text: String) -> Result<(), ProcessingError> {
        if !self.matches(&snapshot) {
            return Err(ProcessingError::new("staleDraft"));
        }
        quality::validate_output(&text)?;
        if text.len() > 16 * 1024 {
            return Err(ProcessingError::new("outputTooLarge"));
        }
        self.preview = Some(Preview {
            text: quality::retain_boundaries(&snapshot.text, &text),
            source: snapshot,
        });
        Ok(())
    }

    pub fn visible(&self) -> String {
        if self.stopped || self.suspended {
            return self.confirmed_visible.clone();
        }
        if let Some(preview) = &self.preview {
            let tail = slice(
                &self.speech,
                preview.source.speech_end..self.speech.chars().count(),
            );
            if tail.is_empty() {
                return preview.text.clone();
            }
            if let Some(at) = map_position(
                &preview.source.text,
                &preview.text,
                preview.source.insertion_end,
            ) {
                return format!(
                    "{}{}{}",
                    slice(&preview.text, 0..at),
                    tail,
                    slice(&preview.text, at..preview.text.chars().count())
                );
            }
        }
        self.effective(self.speech.chars().count())
    }

    pub fn write(&mut self) -> Option<(u64, String)> {
        if self.stopped || self.suspended {
            return None;
        }
        let text = self.visible();
        if crate::contracts::draft::equivalent_text(&text, &self.confirmed_visible) {
            return None;
        }
        self.next_revision += 1;
        Some((self.next_revision, text))
    }

    pub fn acknowledge(&mut self, revision: u64, text: String) -> Result<(), ProcessingError> {
        if revision <= self.confirmed_revision || revision > self.next_revision {
            return Err(ProcessingError::new("staleDraft"));
        }
        self.confirmed_revision = revision;
        self.confirmed_visible = text;
        Ok(())
    }

    pub fn handoff(&mut self) {
        self.suspended = true;
        self.source_epoch += 1;
        self.preview = None;
        self.attempted = None;
    }

    /// Only a native, version-checked read may supply this editor baseline.
    pub fn rebase(&mut self, text: String, selection: Range<usize>) -> Result<(), ProcessingError> {
        if self.stopped || !self.suspended {
            return Err(ProcessingError::new("staleDraft"));
        }
        if selection.start > selection.end
            || !boundary(&text, selection.start)
            || !boundary(&text, selection.end)
        {
            return Err(ProcessingError::new("sourceBoundary"));
        }
        self.baseline = text.clone();
        self.confirmed_visible = text;
        self.next_revision += 1;
        self.confirmed_revision = self.next_revision;
        self.insertion = selection;
        self.kind = BaselineKind::EditorSnapshot;
        self.speech.clear();
        self.stable = 0;
        self.version += 1;
        self.suspended = false;
        Ok(())
    }
    pub fn rebase_retained(
        &mut self,
        text: String,
        selection: Range<usize>,
    ) -> Result<(), ProcessingError> {
        let stopped = self.stopped;
        self.stopped = false;
        let result = self.rebase(text, selection);
        self.stopped = stopped;
        result
    }

    pub fn stop(&mut self) -> &str {
        self.stopped = true;
        self.preview = None;
        &self.confirmed_visible
    }
}

/// Capacity is a delivery boundary, not a pause/final-ASR boundary.
pub fn capacity_boundary(text: &str) -> Option<usize> {
    capacity_boundary_at(text, BLOCK_LIMIT)
}
/// Literal transport chunks may cross a protected token only at native
/// capacity. Oversized chunks never enter a model request or change spelling.
pub fn native_capacity_boundary(text: &str, limit: usize) -> Option<usize> {
    let mut units = 0;
    let mut scalars = 0;
    for part in text.graphemes(true) {
        let next = part.encode_utf16().count();
        if units + next > limit {
            return (scalars > 0).then_some(scalars);
        }
        units += next;
        scalars += part.chars().count();
    }
    None
}
pub fn capacity_boundary_at(text: &str, limit: usize) -> Option<usize> {
    if text.chars().count() <= limit {
        return None;
    }
    let protected = quality::protected_ranges(text);
    let mut bytes = 0;
    let mut count = 0;
    let mut sentence = None;
    for (offset, grapheme) in text.grapheme_indices(true) {
        if count + grapheme.chars().count() > limit {
            break;
        }
        count += grapheme.chars().count();
        bytes = offset + grapheme.len();
        if grapheme.chars().any(|c| "。！？；.!?;\n".contains(c))
            && !protected.iter().any(|r| r.start < bytes && bytes < r.end)
        {
            sentence = Some(bytes);
        }
    }
    let end = if let Some(range) = protected.iter().find(|r| r.start < bytes && bytes < r.end) {
        if range.start == 0 {
            range.end
        } else {
            sentence.unwrap_or(range.start)
        }
    } else {
        sentence.unwrap_or(bytes)
    };
    let end = if end == 0 {
        text.graphemes(true).next()?.len()
    } else {
        end
    };
    Some(text[..end].chars().count())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn later_speech_can_revise_an_already_displayed_sentence() {
        let mut b = Block::new("block".into());
        b.observe("周三开会".into(), 4).unwrap();
        let snapshot = b.snapshot(false).unwrap();
        b.attempted(snapshot.clone());
        b.accept(snapshot, "周三开会。".into()).unwrap();
        let (revision, visible) = b.write().unwrap();
        b.acknowledge(revision, visible).unwrap();
        assert!(!b.needs_generation(&b.snapshot(false).unwrap()));
        b.observe("周三开会不对周四".into(), 8).unwrap();
        let snapshot = b.snapshot(false).unwrap();
        assert_eq!(snapshot.text, "周三开会不对周四");
        b.accept(snapshot, "周四开会。".into()).unwrap();
        assert_eq!(b.visible(), "周四开会。");
    }

    #[test]
    fn late_preview_retains_new_speech_and_asr_revision_invalidates_it() {
        let mut b = Block::new("b".into());
        b.observe("hello".into(), 5).unwrap();
        let first = b.snapshot(false).unwrap();
        b.observe("hello world".into(), 5).unwrap();
        b.accept(first, "Hello".into()).unwrap();
        assert_eq!(b.visible(), "Hello world");
        b.observe("yellow world".into(), 6).unwrap();
        assert_eq!(b.visible(), "yellow world");
    }

    #[test]
    fn edited_baseline_is_not_restored_from_old_asr_or_reformatted_on_finish() {
        let mut b = Block::new("b".into());
        b.observe("周三开会".into(), 4).unwrap();
        let old = b.snapshot(false).unwrap();
        b.handoff();
        b.rebase("周四开会".into(), 4..4).unwrap();
        assert!(b.snapshot(true).is_none());
        assert!(b.accept(old, "周三开会。".into()).is_err());
        b.observe("还是周五".into(), 4).unwrap();
        let input = b.snapshot(true).unwrap();
        assert_eq!(input.text, "周四开会还是周五");
        assert_eq!(input.baseline_kind, BaselineKind::EditorSnapshot);
        b.accept(input, "周五开会。".into()).unwrap();
        assert_eq!(b.visible(), "周五开会。");
    }

    #[test]
    fn stop_keeps_only_acknowledged_text_and_rejects_late_result() {
        let mut b = Block::new("b".into());
        b.observe("first".into(), 5).unwrap();
        let (revision, text) = b.write().unwrap();
        b.acknowledge(revision, text).unwrap();
        b.observe("first second".into(), 12).unwrap();
        let late = b.snapshot(true).unwrap();
        assert_eq!(b.stop(), "first");
        assert!(b.accept(late, "First, second.".into()).is_err());
        assert!(b.write().is_none());
    }

    #[test]
    fn selected_text_is_replaced_at_the_real_insertion_position() {
        let mut b = Block::new("b".into());
        b.handoff();
        b.rebase("会议周四下午".into(), 2..4).unwrap();
        b.observe("周五".into(), 2).unwrap();
        assert_eq!(b.visible(), "会议周五下午");
        assert_eq!(map_position("abcDEFghi", "abcXYghi", 4), None);
        assert_eq!(map_position("abcDEFghi", "abcXYghi", 6), Some(5));
    }

    #[test]
    fn capacity_never_splits_graphemes_or_protected_tokens() {
        assert_eq!(capacity_boundary(&"字".repeat(2000)), None);
        let text = format!("{}。{}", "字".repeat(1900), "后".repeat(200));
        assert_eq!(capacity_boundary(&text), Some(1901));
        let url = format!("https://example.com/{}", "x".repeat(2200));
        assert_eq!(capacity_boundary(&url), Some(url.chars().count()));
        let text = format!("{}👩‍💻后", "字".repeat(1999));
        assert_eq!(capacity_boundary(&text), Some(1999));
    }
    #[test]
    fn extending_a_grapheme_invalidates_the_old_covered_prefix() {
        let mut b = Block::new("b".into());
        b.observe("e".into(), 1).unwrap();
        let old = b.snapshot(false).unwrap();
        b.observe("e\u{301} next".into(), 1).unwrap();
        assert!(!b.matches(&old));
        assert!(b.snapshot(false).is_none());
        assert!(b.accept(old, "E".into()).is_err());
        b.observe("e\u{301} next".into(), 7).unwrap();
        assert_eq!(b.snapshot(false).unwrap().text, "e\u{301} next");
    }
    #[test]
    fn stopped_editor_read_keeps_identity_and_versions_without_model_permission() {
        let mut b = Block::new("b".into());
        b.observe("周三".into(), 2).unwrap();
        let (rev, text) = b.write().unwrap();
        b.acknowledge(rev, text).unwrap();
        b.handoff();
        b.stop();
        b.rebase_retained("周四".into(), 2..2).unwrap();
        assert_eq!(b.id(), "b");
        assert!(b.confirmed().0 > rev);
        assert_eq!(b.confirmed().1, "周四");
        assert!(b.write().is_none());
        assert!(b.snapshot(true).is_none());
    }
}
