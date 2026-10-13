//! Semantic text remains immutable; marked line separators keep one editable range.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TextPhase {
    Marked,
    Materializing,
    Semantic,
}
#[derive(Clone, Debug)]
pub(crate) struct CompositionText {
    pub semantic: String,
    pub marked: String,
    pub phase: TextPhase,
    verified: Option<(String, Vec<(u32, u32)>)>,
}
impl CompositionText {
    /// Called only after the paired document proof and range check succeed.
    /// Store exact endpoints; this does not normalize later editor snapshots.
    pub fn from_verified(semantic: &str, native: &str) -> Option<Self> {
        let doubled = if semantic == native {
            false
        } else if semantic.replace('\n', "\n\n") == native {
            true
        } else {
            return None;
        };
        let mut semantic_at = 0;
        let mut native_at = 0;
        let mut endpoints = vec![(0, 0)];
        for c in semantic.chars() {
            semantic_at += c.len_utf16() as u32;
            native_at += if doubled && c == '\n' {
                2
            } else {
                c.len_utf16() as u32
            };
            endpoints.push((semantic_at, native_at));
        }
        let mut text = Self::new(semantic);
        text.phase = TextPhase::Semantic;
        text.verified = Some((native.into(), endpoints));
        Some(text)
    }
    /// Endpoint conversion uses this exact write's semantic/marked provenance.
    /// It cannot normalize an unrelated editor snapshot.
    pub fn position(&self, actual: &str, at: u32, to_native: bool) -> Option<u32> {
        if let Some((native, endpoints)) = &self.verified {
            if actual != native {
                return None;
            }
            return endpoints.iter().find_map(|&(semantic, native)| {
                if to_native {
                    (semantic == at).then_some(native)
                } else {
                    (native == at).then_some(semantic)
                }
            });
        }
        if self.phase == TextPhase::Marked {
            if !self.matches_marked(actual) {
                return None;
            }
        } else if !equivalent_native_text(actual, &self.semantic) {
            return None;
        }
        let tokens = |text: &str| {
            let mut iter = text.chars().peekable();
            let mut units = 0u32;
            let mut positions = vec![0];
            while let Some(c) = iter.next() {
                units += c.len_utf16() as u32;
                if c == '\r' && iter.peek() == Some(&'\n') {
                    iter.next();
                    units += 1;
                }
                positions.push(units);
            }
            positions
        };
        let semantic = tokens(&self.semantic);
        let native = tokens(actual);
        if semantic.len() != native.len() {
            return None;
        }
        let (from, to) = if to_native {
            (&semantic, &native)
        } else {
            (&native, &semantic)
        };
        from.iter().position(|p| *p == at).map(|i| to[i])
    }
    pub fn new(text: &str) -> Self {
        let mut marked = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\r' => {
                    if chars.peek() == Some(&'\n') {
                        chars.next();
                    }
                    marked.push('\u{2028}');
                }
                '\n' => marked.push('\u{2028}'),
                _ => marked.push(c),
            }
        }
        Self {
            semantic: text.into(),
            marked,
            phase: TextPhase::Marked,
            verified: None,
        }
    }
    pub fn needs_materialization(&self) -> bool {
        self.phase == TextPhase::Marked && self.semantic != self.marked
    }
    pub fn commit_text(&self) -> String {
        self.semantic.replace("\r\n", "\n").replace('\r', "\n")
    }
    pub fn matches_marked(&self, actual: &str) -> bool {
        // Some native stores expose their soft break as CR. Only generated breaks
        // may use that representation; an original U+2028 must never be decoded.
        let actual = actual.replace("\r\n", "\n").replace('\r', "\n");
        let mut actual = actual.chars();
        for c in self.commit_text().chars() {
            let value = actual.next();
            if c == '\n' {
                if !matches!(value, Some('\n' | '\u{2028}' | '\u{000b}')) {
                    return false;
                }
            } else if value != Some(c) {
                return false;
            }
        }
        actual.next().is_none()
    }
}
pub(crate) fn equivalent_native_text(actual: &str, expected: &str) -> bool {
    actual.replace("\r\n", "\n").replace('\r', "\n")
        == expected.replace("\r\n", "\n").replace('\r', "\n")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verified_host_mapping_preserves_user_break_and_utf16_endpoints() {
        let text = CompositionText::from_verified("甲\n🙂", "甲\n\n🙂").unwrap();
        assert_eq!(text.position("甲\n\n🙂", 3, false), Some(2));
        assert_eq!(text.position("甲\n\n🙂", 4, true), Some(5));
        assert_eq!(text.position("甲\n\n🙂", 2, false), None);
        assert_eq!(text.position("甲\n\n🙂", 4, false), None);
        assert_eq!(text.position("甲\n\n🙂\n", 5, false), None);
        assert_eq!(text.semantic, "甲\n🙂");
        assert!(CompositionText::from_verified("甲", "甲\n").is_none());
    }
    #[test]
    fn marked_breaks_preserve_original_text_and_real_line_separators() {
        let raw = "前🙂\r\n中\n\n后\r末\u{2028}e\u{301}";
        let text = CompositionText::new(raw);
        assert_eq!(text.semantic, raw);
        assert_eq!(
            text.marked,
            "前🙂\u{2028}中\u{2028}\u{2028}后\u{2028}末\u{2028}e\u{301}"
        );
        assert!(text.needs_materialization());
        assert!(!CompositionText::new("原本的\u{2028}分隔").needs_materialization());
        assert!(!CompositionText::new("普通口述🙂").needs_materialization());
    }
    #[test]
    fn native_equivalence_never_decodes_generated_or_original_separators() {
        assert!(equivalent_native_text("前\r\n后", "前\n后"));
        assert!(!equivalent_native_text("前\u{2028}后", "前\n后"));
        assert!(!equivalent_native_text("前\n\n后", "前\n后"));
        let marked = CompositionText::new("原\u{2028}有\n新");
        assert!(marked.matches_marked("原\u{2028}有\r新"));
        assert!(marked.matches_marked("原\u{2028}有\u{000b}新"));
        assert!(!marked.matches_marked("原\r有\r新"));
        assert!(!marked.matches_marked("原\u{2028}有\n\n新"));
    }
}
