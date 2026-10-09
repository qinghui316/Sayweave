use super::{ProcessingError, quality::protected_ranges};
pub use crate::contracts::processing::SourceSegment as Segment;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, ops::Range};
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
        self.matches_in(turn, &SourceIndex::new(segments))
    }
    pub fn matches_in(&self, turn: u64, source: &SourceIndex<'_>) -> bool {
        if self.turn != turn || !source.valid || self.chars.start > self.chars.end {
            return false;
        }
        let mut position = self.chars.start;
        let mut expected = self.text.chars();
        for span in &self.source {
            let Some(segment) = source.segments.get(&span.segment) else {
                return false;
            };
            if span.chars.start >= span.chars.end
                || span.chars.end >= segment.bytes.len()
                || !(segment.value.confirmed || self.final_source)
                || segment.start.checked_add(span.chars.start) != Some(position)
            {
                return false;
            }
            let Some(end) = position.checked_add(span.chars.end - span.chars.start) else {
                return false;
            };
            if end > self.chars.end {
                return false;
            }
            let text =
                &segment.value.text[segment.bytes[span.chars.start]..segment.bytes[span.chars.end]];
            if !expected
                .by_ref()
                .take(span.chars.end - span.chars.start)
                .eq(text.chars())
            {
                return false;
            }
            position = end;
        }
        position == self.chars.end && expected.next().is_none()
    }
}
struct IndexedSegment<'a> {
    value: &'a Segment,
    start: usize,
    bytes: Vec<usize>,
}
/// Immutable per-ASR-snapshot positions. Reused by all frozen units in the
/// actor/history pass; validation never rescans or concatenates the whole
/// recording per unit. Identity, exact local text and global position remain
/// independent checks. Appending outside a frozen span does not retire it.
pub struct SourceIndex<'a> {
    segments: HashMap<u64, IndexedSegment<'a>>,
    valid: bool,
}
impl<'a> SourceIndex<'a> {
    pub fn new(segments: &'a [Segment]) -> Self {
        let mut indexed = HashMap::with_capacity(segments.len());
        let mut start = 0;
        let mut valid = true;
        for segment in segments {
            let mut bytes = segment
                .text
                .char_indices()
                .map(|(byte, _)| byte)
                .collect::<Vec<_>>();
            bytes.push(segment.text.len());
            let length = bytes.len() - 1;
            if indexed
                .insert(
                    segment.id,
                    IndexedSegment {
                        value: segment,
                        start,
                        bytes,
                    },
                )
                .is_some()
            {
                valid = false;
            }
            start += length;
        }
        Self {
            segments: indexed,
            valid,
        }
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
    #[ignore = "Explicit isolated source-validation benchmark; no device or provider"]
    fn source_validation_benchmark() {
        use std::{hint::black_box, time::Instant};
        for size in [10_000usize, 50_000, 100_000] {
            let mut segments = Vec::new();
            let mut units = Vec::new();
            let mut start = 0;
            while start < size {
                let length = 128.min(size - start);
                let id = segments.len() as u64 + 1;
                let text = "字".repeat(length);
                segments.push(seg(id, &text, true));
                units.push(Unit {
                    id: id.to_string(),
                    turn: 1,
                    chars: start..start + length,
                    source: vec![SourceSpan {
                        segment: id,
                        revision: 1,
                        chars: 0..length,
                    }],
                    text,
                    preserve_raw: false,
                    final_source: false,
                });
                start += length;
            }
            segments.push(seg(999_999, "未定尾部", false));
            let mut elapsed = Vec::new();
            for pass in 0..3 {
                segments.last_mut().unwrap().text.push_str("增长");
                let begin = Instant::now();
                let source = SourceIndex::new(black_box(&segments));
                assert!(
                    units
                        .iter()
                        .all(|unit| black_box(unit).matches_in(1, black_box(&source)))
                );
                elapsed.push(begin.elapsed().as_micros());
                println!(
                    "SOURCE_VALIDATION_SAMPLE source_chars={size} units={} pass={pass} elapsed_us={}",
                    units.len(),
                    elapsed.last().unwrap()
                );
            }
            elapsed.sort_unstable();
            println!(
                "SOURCE_VALIDATION_RESULT source_chars={size} units={} max_us={} median_us={}",
                units.len(),
                elapsed[2],
                elapsed[1]
            );
        }
    }
    #[test]
    fn indexed_spans_preserve_unicode_positions_confirmation_and_identity() {
        let mut segments = vec![
            seg(10, "甲🙂", true),
            seg(20, "e\u{301}乙", true),
            seg(30, "尾", false),
        ];
        let unit = next(7, &segments, 1, false).unwrap().unwrap();
        assert_eq!(unit.text, "🙂e\u{301}乙");
        assert!(unit.matches_in(7, &SourceIndex::new(&segments)));
        segments[1].revision += 1;
        segments[1].text.push_str("后续");
        assert!(unit.matches_in(7, &SourceIndex::new(&segments)));
        segments[1].confirmed = false;
        assert!(!unit.matches(7, &segments));
        segments[1].confirmed = true;
        segments[0].text.insert(0, '前');
        assert!(!unit.matches(7, &segments));
        segments[0].text.remove(0);
        segments[1].id = 21;
        assert!(!unit.matches(7, &segments));
        segments[1].id = 20;
        segments.push(seg(20, "重复身份", true));
        assert!(!unit.matches(7, &segments));
    }
    #[test]
    fn invalid_source_ranges_are_rejected_without_panicking_or_aliasing() {
        let segments = vec![seg(1, "中文🙂", true)];
        let original = next(1, &segments, 0, false).unwrap().unwrap();
        for range in [std::ops::Range { start: 2, end: 1 }, 0..usize::MAX, 0..2] {
            let mut unit = original.clone();
            unit.source[0].chars = range;
            assert!(!unit.matches(1, &segments));
        }
        let mut unit = original;
        unit.source.clear();
        assert!(!unit.matches(1, &segments));
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
