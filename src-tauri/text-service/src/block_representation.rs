//! Identity-only cross-provider check. Stability does not prove shared revision.
//! Coordinates are UTF-16 in TSF and byte boundaries in the semantic document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Mapping {
    pub start: usize,
    pub end: usize,
    pub native_end: i32,
}

pub(super) fn map(semantic: &str, native: &str, start: i32, end: i32) -> Option<Mapping> {
    if semantic != native || start < 0 || end < start || end > native.encode_utf16().count() as i32
    {
        return None;
    }
    // A cached UIA value can differ from TSF by a genuine user newline.
    // Without write provenance or a shared revision, do not infer padding,
    // collapse doubled breaks, or move either native endpoint.
    let mut units = 0;
    let mut a = (start == 0).then_some(0);
    let mut b = (end == 0).then_some(0);
    for (byte, c) in semantic.char_indices() {
        units += c.len_utf16() as i32;
        if units == start {
            a = Some(byte + c.len_utf8());
        }
        if units == end {
            b = Some(byte + c.len_utf8());
        }
    }
    Some(Mapping {
        start: a?,
        end: b?,
        native_end: end,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_semantic_provider_cannot_reclassify_user_tail_as_padding() {
        for end in [4, 5, 6] {
            assert!(map("周五开会", "周五开会\n\n", 0, end).is_none());
        }
    }
    #[test]
    fn user_newlines_are_not_trimmed_and_ambiguity_refuses() {
        assert!(map("甲\n乙🙂", "甲\n\n乙🙂", 0, 6).is_none());
        for text in ["甲\n", "甲\n\n", "甲\r\n", "甲\n乙🙂"] {
            let end = text.encode_utf16().count() as i32;
            let proof = map(text, text, 0, end).unwrap();
            assert_eq!(&text[proof.start..proof.end], text);
            assert_eq!(proof.native_end, end);
        }
    }
    #[test]
    fn outside_text_and_split_codepoints_do_not_map() {
        assert!(map("左🙂右", "左🙂右别的正文", 1, 3).is_none());
        assert!(map("左🙂右", "左🙂右", 1, 2).is_none());
        assert_eq!(map("左🙂右", "左🙂右", 1, 3).unwrap().end, 7);
    }
}
