use super::{ProcessingError, quality::protected_ranges};
pub use crate::contracts::processing::SourceSegment as Segment;
use serde::{Deserialize, Serialize};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub segment: u64,
    pub revision: u64,
    pub chars: Range<usize>,
}
#[derive(Clone)]
pub struct Unit {
    pub id: String,
    pub turn: u64,
    pub chars: Range<usize>,
    pub source: Vec<SourceSpan>,
    pub text: String,
    pub preserve_raw: bool,
    pub final_source: bool,
}
pub fn slice(text: &str, chars: Range<usize>) -> String {
    text.chars()
        .skip(chars.start)
        .take(chars.end - chars.start)
        .collect()
}
impl Unit {
    pub fn matches(&self, turn: u64, segments: &[Segment]) -> bool {
        let mut offset = 0;
        self.turn == turn
            && self.source.iter().all(|span| {
                let length = span.chars.end - span.chars.start;
                let expected = slice(&self.text, offset..offset + length);
                offset += length;
                let mut start = 0;
                segments
                    .iter()
                    .find_map(|s| {
                        let global = start;
                        start += s.text.chars().count();
                        (s.id == span.segment).then_some((s, global))
                    })
                    .is_some_and(|(s, global)| {
                        global + span.chars.start == self.chars.start + offset - length
                            && global + span.chars.end == self.chars.start + offset
                            && (s.confirmed || self.final_source)
                            && s.text.chars().count() >= span.chars.end
                            && slice(&s.text, span.chars.clone()) == expected
                    })
            })
            && slice(
                &segments.iter().map(|s| s.text.as_str()).collect::<String>(),
                self.chars.clone(),
            ) == self.text
    }
}
/// Returns only a contiguous confirmed source prefix. Model output length never
/// determines this watermark. The caller retains at most one frozen unit.
pub fn next(
    turn: u64,
    segments: &[Segment],
    consumed: usize,
    final_result: bool,
) -> Result<Option<Unit>, ProcessingError> {
    let full = segments.iter().map(|s| s.text.as_str()).collect::<String>();
    let stable = segments
        .iter()
        .take_while(|s| s.confirmed || final_result)
        .map(|s| s.text.chars().count())
        .sum::<usize>();
    if consumed > stable {
        return Err(ProcessingError::new("sourceChanged"));
    }
    if consumed == stable {
        return Ok(None);
    }
    let rest = slice(&full, consumed..stable);
    let protected = protected_ranges(&rest);
    let mut end = 0usize;
    let mut chars = 0usize;
    let mut last_break = None;
    for (byte, grapheme) in rest.grapheme_indices(true) {
        let n = grapheme.chars().count();
        if chars + n > 512 {
            break;
        }
        end = byte + grapheme.len();
        chars += n;
        if grapheme
            .chars()
            .any(|c| c.is_whitespace() || "，。！？；,.!?;".contains(c))
            && !protected.iter().any(|r| r.start < end && end < r.end)
        {
            last_break = Some(end);
        }
    }
    if end == 0 {
        // An indivisible grapheme can itself exceed the model's character
        // budget. Preserve it as raw rather than cut or lose its source.
        end = rest.graphemes(true).next().unwrap().len();
    }
    let too_long = end < rest.len();
    if too_long {
        if let Some(range) = protected.iter().find(|r| r.start < end && end < r.end) {
            end = range.start;
            if end == 0 {
                end = range.end;
            }
        } else if let Some(boundary) = last_break {
            end = boundary;
        }
    }
    let text = rest[..end].to_string();
    let length = text.chars().count();
    let preserve_raw = length > 512 || text.trim().is_empty();
    if length == 0 {
        return Err(ProcessingError::new("sourceBoundary"));
    }
    let range = consumed..consumed + length;
    let mut position = 0;
    let mut source = Vec::new();
    for segment in segments {
        let len = segment.text.chars().count();
        let start = range.start.max(position);
        let end = range.end.min(position + len);
        if start < end {
            source.push(SourceSpan {
                segment: segment.id,
                revision: segment.revision,
                chars: start - position..end - position,
            });
        }
        position += len;
    }
    Ok(Some(Unit {
        id: uuid::Uuid::new_v4().to_string(),
        turn,
        chars: range,
        source,
        text,
        preserve_raw,
        final_source: final_result,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    fn seg(id: u64, text: &str, confirmed: bool) -> Segment {
        Segment {
            id,
            revision: 1,
            text: text.into(),
            confirmed,
        }
    }
    #[test]
    fn tail_growth_does_not_invalidate_prefix_but_source_revision_does() {
        let mut segments = vec![seg(1, "第一段。", true), seg(2, "临时", false)];
        let unit = next(1, &segments, 0, false).unwrap().unwrap();
        assert_eq!(unit.text, "第一段。");
        segments[1].text = "继续增长的尾部🙂".into();
        segments[1].revision += 1;
        assert!(unit.matches(1, &segments));
        assert!(next(1, &segments, unit.chars.end, false).unwrap().is_none());
        segments[0].revision += 1;
        segments[0].text.push_str("追加尾部");
        assert!(unit.matches(1, &segments));
        segments[0].text = "改写段。追加尾部".into();
        assert!(!unit.matches(1, &segments));
        assert!(!unit.matches(2, &segments));
    }
    #[test]
    fn splitting_never_cuts_grapheme_url_or_uses_processed_length() {
        let source = format!(
            "{}🙂👩‍💻https://a.test/{}。",
            "字".repeat(505),
            "x".repeat(650)
        );
        let segments = vec![seg(1, &source, true)];
        let a = next(1, &segments, 0, false).unwrap().unwrap();
        assert!(!a.text.ends_with('\u{200d}'));
        assert!(!a.text.contains("https:"));
        let b = next(1, &segments, a.chars.end, false).unwrap().unwrap();
        assert!(b.preserve_raw);
        assert!(b.text.starts_with("https://"));
        let c = next(1, &segments, b.chars.end, false).unwrap();
        let joined = format!(
            "{}{}{}",
            a.text,
            b.text,
            c.map(|u| u.text).unwrap_or_default()
        );
        assert_eq!(joined, source);
    }
    #[test]
    fn repeated_content_cannot_alias_a_moved_source_span() {
        let mut segments = vec![seg(1, "a", true), seg(2, "bb", true)];
        let unit = next(1, &segments, 1, false).unwrap().unwrap();
        assert!(unit.matches(1, &segments));
        segments[0].text = "abb".into();
        assert!(!unit.matches(1, &segments));
    }
    #[test]
    fn unconfirmed_gap_blocks_later_confirmed_and_final_flushes() {
        let segments = vec![seg(1, "未定", false), seg(2, "确定", true)];
        assert!(next(1, &segments, 0, false).unwrap().is_none());
        assert_eq!(
            next(1, &segments, 0, true).unwrap().unwrap().text,
            "未定确定"
        );
    }
}
