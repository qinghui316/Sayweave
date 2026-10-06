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
}
impl CompositionText {
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
