//! Source identities absorbed by an editor baseline. Never deduplicates text.
use crate::contracts::processing::SourceSegment;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Default, Clone)]
pub struct Watermark(Vec<Consumed>);
#[derive(Clone)]
struct Consumed {
    id: u64,
    whole_confirmed: bool,
    prefix: String,
    observed: String,
}
impl Consumed {
    fn offset(&self, segment: &SourceSegment) -> Option<usize> {
        if segment.id != self.id {
            None
        } else if self.whole_confirmed {
            Some(segment.text.chars().count())
        } else if segment.text.starts_with(&self.prefix) {
            Some(self.prefix.chars().count())
        } else {
            revised_boundary(&self.observed, &self.prefix, &segment.text)
        }
    }
}
impl Watermark {
    pub fn capture(segments: &[SourceSegment], mut count: usize) -> Option<Self> {
        let mut result = Vec::new();
        for segment in segments {
            if count == 0 {
                break;
            }
            let size = segment.text.chars().count();
            let used = size.min(count);
            result.push(Consumed {
                id: segment.id,
                whole_confirmed: segment.confirmed && used == size,
                prefix: segment.text.chars().take(used).collect(),
                observed: segment.text.clone(),
            });
            count -= used;
        }
        (count == 0).then_some(Self(result))
    }
    pub fn offset(&self, segments: &[SourceSegment]) -> Option<usize> {
        let mut count = 0;
        for (index, consumed) in self.0.iter().enumerate() {
            count += consumed.offset(segments.get(index)?)?;
        }
        Some(count)
    }
    pub fn refreshed(&self, segments: &[SourceSegment]) -> Option<(usize, Self)> {
        let mut count = 0;
        let mut refreshed = Vec::with_capacity(self.0.len());
        for (index, consumed) in self.0.iter().enumerate() {
            let segment = segments.get(index)?;
            let used = consumed.offset(segment)?;
            count += used;
            // An absorbed confirmed identity remains absorbed through every
            // later revision, including a temporarily empty/partial result.
            let mut next = consumed.clone();
            if !consumed.whole_confirmed {
                next.prefix = segment.text.chars().take(used).collect();
                next.observed.clone_from(&segment.text);
                next.whole_confirmed = segment.confirmed && used == segment.text.chars().count();
            }
            refreshed.push(next);
        }
        Some((count, Self(refreshed)))
    }
}

// A source revision can change the already delivered spelling without moving
// its boundary. Keep it only when every minimum edit path crosses the cut at
// the same grapheme, immediately followed by unchanged unconsumed source.
// No editor text is read or searched here; an unavailable split stays refused.
fn revised_boundary(old: &str, prefix: &str, new: &str) -> Option<usize> {
    let old_parts: Vec<_> = old.graphemes(true).take(257).collect();
    let new_parts: Vec<_> = new.graphemes(true).take(257).collect();
    if old_parts.len() > 256 || new_parts.len() > 256 {
        return None;
    }
    let cut = old
        .grapheme_indices(true)
        .position(|(at, _)| at == prefix.len())?;
    if cut == 0 || !old.starts_with(prefix) {
        return None;
    }
    let forward = edit_row(&old_parts[..cut], &new_parts);
    let old_right: Vec<_> = old_parts[cut..].iter().copied().rev().collect();
    let new_reverse: Vec<_> = new_parts.iter().copied().rev().collect();
    let backward = edit_row(&old_right, &new_reverse);
    let distance = edit_row(&old_parts, &new_parts)[new_parts.len()];
    let mut candidates = (0..=new_parts.len())
        .filter(|&at| forward[at] + backward[new_parts.len() - at] == distance);
    let at = candidates.next()?;
    if candidates.next().is_some() || new_parts.get(at) != old_parts.get(cut) {
        return None;
    }
    Some(
        new_parts[..at]
            .iter()
            .map(|value| value.chars().count())
            .sum(),
    )
}

fn edit_row(left: &[&str], right: &[&str]) -> Vec<usize> {
    let mut previous: Vec<_> = (0..=right.len()).collect();
    for (i, a) in left.iter().enumerate() {
        let mut row = vec![i + 1; right.len() + 1];
        for (j, b) in right.iter().enumerate() {
            row[j + 1] = (previous[j] + usize::from(a != b))
                .min(previous[j + 1] + 1)
                .min(row[j] + 1);
        }
        previous = row;
    }
    previous
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(id: u64, text: &str, confirmed: bool) -> SourceSegment {
        SourceSegment {
            id,
            revision: 1,
            text: text.into(),
            confirmed,
        }
    }
    #[test]
    fn confirmed_revision_is_absorbed_by_identity_not_old_length() {
        let old = [source(1, "周三开会", true)];
        let mark = Watermark::capture(&old, 4).unwrap();
        assert_eq!(
            mark.offset(&[source(1, "周三下午开会", true), source(2, "改成周一", true)]),
            Some(6)
        );
        assert_eq!(mark.offset(&[source(2, "周三开会", true)]), None);
    }
    #[test]
    fn partial_update_keeps_new_tail_once_and_refuses_changed_boundary() {
        let mark = Watermark::capture(&[source(7, "开会", false)], 2).unwrap();
        assert_eq!(mark.offset(&[source(7, "开会在周一", true)]), Some(2));
        assert_eq!(mark.offset(&[source(7, "会议在周一", true)]), None);
    }
    #[test]
    fn revised_absorbed_prefix_uses_the_retained_unconsumed_source_anchor() {
        let mark = Watermark::capture(&[source(7, "周三", false)], 1).unwrap();
        assert_eq!(mark.offset(&[source(7, "舟三开会", false)]), Some(1));
        // The unchanged right-hand anchor is part of the same ASR source,
        // never a search in the external editor or a count-based split.
        assert_eq!(mark.offset(&[source(8, "舟三开会", false)]), None);
        assert_eq!(mark.offset(&[source(7, "舟三三开会", false)]), None);
        assert_eq!(mark.offset(&[source(7, "舟四开会", false)]), None);
        let old = Watermark::capture(&[source(7, "周", false)], 1).unwrap();
        assert_eq!(old.offset(&[source(7, "舟三开会", false)]), None);
        let (offset, advanced) = old.refreshed(&[source(7, "周三", false)]).unwrap();
        assert_eq!(offset, 1);
        assert_eq!(advanced.offset(&[source(7, "舟三开会", false)]), Some(1));
    }
    #[test]
    fn revised_source_cut_preserves_graphemes_and_requires_bounded_context() {
        let mark = Watermark::capture(&[source(1, "e\u{301}三", false)], 2).unwrap();
        assert_eq!(mark.offset(&[source(1, "é三开会", false)]), Some(1));
        let split = Watermark::capture(&[source(1, "e\u{301}三", false)], 1).unwrap();
        assert_eq!(split.offset(&[source(1, "é三开会", false)]), None);
        let text = format!("周{}", "三".repeat(256));
        let mark = Watermark::capture(&[source(1, &text, false)], 1).unwrap();
        assert_eq!(
            mark.offset(&[source(1, &text.replacen('周', "舟", 1), false)]),
            None
        );
    }
    #[test]
    fn refreshing_a_confirmed_absorption_cannot_restore_old_source() {
        let mark = Watermark::capture(&[source(1, "周三", true)], 2).unwrap();
        let (_, mark) = mark.refreshed(&[source(1, "周三下午", false)]).unwrap();
        assert_eq!(mark.offset(&[source(1, "周三下午开会", false)]), Some(6));
        let (_, mark) = mark.refreshed(&[source(1, "", false)]).unwrap();
        assert_eq!(mark.offset(&[source(1, "旧结果", false)]), Some(3));
        assert_eq!(mark.offset(&[source(2, "旧结果", false)]), None);
    }
}
