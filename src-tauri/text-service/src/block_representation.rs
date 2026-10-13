//! Complete-document representation proof. Never searches for block text.
//! Coordinates are UTF-16 in TSF and byte boundaries in the semantic document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Mapping {
    pub start: usize,
    pub end: usize,
    pub native_end: i32,
}

pub(super) fn map(
    semantic: &str,
    native: &str,
    start: i32,
    end: i32,
    terminal_scope: bool,
) -> Option<Mapping> {
    if start < 0 || end < start || end > native.encode_utf16().count() as i32 {
        return None;
    }
    let mut result = None;
    for doubled in [false, true] {
        let represented = if doubled {
            semantic.replace('\n', "\n\n")
        } else {
            semantic.into()
        };
        if !native
            .strip_prefix(&represented)
            .is_some_and(|tail| tail.chars().all(|c| matches!(c, '\r' | '\n')))
        {
            continue;
        }
        let length = represented.encode_utf16().count() as i32;
        // Only an already-proven terminal block can shed host padding. The
        // candidate endpoint can shrink inside its native range, never expand.
        let precise_end = if terminal_scope && end > length {
            length
        } else {
            end
        };
        let mut units = 0;
        let mut a = (start == 0).then_some(0);
        let mut b = (precise_end == 0).then_some(0);
        for (byte, c) in semantic.char_indices() {
            units += if doubled && c == '\n' {
                2
            } else {
                c.len_utf16() as i32
            };
            if units == start {
                a = Some(byte + c.len_utf8());
            }
            if units == precise_end {
                b = Some(byte + c.len_utf8());
            }
        }
        // A representation explaining the document but not both boundaries
        // is ambiguous, not a candidate that may simply be ignored.
        let mapping = Mapping {
            start: a?,
            end: b?,
            native_end: precise_end,
        };
        if mapping.start > mapping.end || result.as_ref().is_some_and(|old| old != &mapping) {
            return None;
        }
        result = Some(mapping);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_padding_requires_original_terminal_scope() {
        assert!(map("周五开会", "周五开会\n\n", 0, 5, false).is_none());
        assert_eq!(
            map("周五开会", "周五开会\n\n", 0, 5, true),
            Some(Mapping {
                start: 0,
                end: 12,
                native_end: 4
            })
        );
        assert!(map("周五开会", "周五开会\n\n", 5, 6, true).is_none());
    }
    #[test]
    fn user_newlines_are_not_trimmed_and_ambiguity_refuses() {
        let proof = map("甲\n乙🙂", "甲\n\n乙🙂\n", 0, 6, true).unwrap();
        assert_eq!(&"甲\n乙🙂"[proof.start..proof.end], "甲\n乙🙂");
        assert!(map("甲\n", "甲\n\n", 0, 3, true).is_none());
        let proof = map("甲\n", "甲\n", 0, 2, true).unwrap();
        assert_eq!(proof.end, "甲\n".len());
    }
    #[test]
    fn outside_text_and_split_codepoints_do_not_map() {
        assert!(map("左🙂右", "左🙂右别的正文", 1, 3, false).is_none());
        assert!(map("左🙂右", "左🙂右\n", 1, 2, false).is_none());
        assert_eq!(map("左🙂右", "左🙂右\n", 1, 3, false).unwrap().end, 7);
    }
}
