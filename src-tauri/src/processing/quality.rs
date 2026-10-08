//! Output usability and source chunk boundaries. Content fidelity belongs to the
//! prompt and evaluation, not lexical vetoes on completed model output.
use super::ProcessingError;
use std::ops::Range;
pub fn retain_boundaries(input: &str, output: &str) -> String {
    let lead = &input[..input.len() - input.trim_start_matches(char::is_whitespace).len()];
    let body = input.trim_end_matches(char::is_whitespace);
    let tail = &input[body.len()..];
    let mut result = output.to_string();
    if !lead.is_empty() && !result.starts_with(char::is_whitespace) {
        result.insert_str(0, lead);
    }
    if !tail.is_empty() && !result.ends_with(char::is_whitespace) {
        result.push_str(tail);
    }
    result
}

fn boundary(c: char) -> bool {
    c.is_whitespace() || "，。；！？、（）【】\"<>`".contains(c)
}
pub fn protected_ranges(text: &str) -> Vec<Range<usize>> {
    let mut result = Vec::new();
    let mut cursor = 0;
    while cursor < text.len() {
        let rest = &text[cursor..];
        if rest.starts_with('`') {
            let marker = if rest.starts_with("```") { "```" } else { "`" };
            let end = rest[marker.len()..]
                .find(marker)
                .map(|n| cursor + marker.len() + n + marker.len())
                .unwrap_or(text.len());
            result.push(cursor..end);
            cursor = end;
            continue;
        }
        let end = rest
            .char_indices()
            .find_map(|(i, c)| boundary(c).then_some(cursor + i))
            .unwrap_or(text.len());
        if end == cursor {
            cursor += rest.chars().next().unwrap().len_utf8();
            continue;
        }
        let token = &text[cursor..end];
        let token = token.trim_end_matches([',', ';', '!', '?', ')', ']']);
        let url = ["https://", "http://", "www."]
            .iter()
            .filter_map(|prefix| token.find(prefix))
            .min();
        let path = token.find(['/', '\\']);
        let address = token.find('@').filter(|_| token.contains('.'));
        if let Some(start) = url.or_else(|| {
            path.or(address).map(|at| {
                token[..at]
                    .char_indices()
                    .rev()
                    .find_map(|(i, c)| {
                        (!c.is_ascii() || !(c.is_ascii_alphanumeric() || "._%+-:".contains(c)))
                            .then_some(i + c.len_utf8())
                    })
                    .unwrap_or(0)
            })
        }) {
            result.push(cursor + start..cursor + token.len());
        }
        cursor = end;
    }
    result
}
pub fn validate_output(output: &str) -> Result<(), ProcessingError> {
    if output.trim().is_empty() {
        return Err(ProcessingError::new("emptyText"));
    }
    if output
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(ProcessingError::new("qualityControl"));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn completed_rewrites_are_not_vetoed_by_lexical_heuristics() {
        // The actual model reply to a spoken date correction was rejected by
        // the old fact-list comparison. Completed rewrites need not preserve
        // old numbers, literal negations, punctuation or input/output ratios.
        for output in [
            "今天10月7号。",
            "预算5000 元，等待200 ms。",
            "明天再发。",
            "第一段。\n第二段🙂。",
            "```rust\nlet count = 5;\n```",
            "简短结论。",
        ] {
            assert!(validate_output(output).is_ok(), "{output}");
        }
        assert!(validate_output(&"完整正文。".repeat(100)).is_ok());
    }
    #[test]
    fn unusable_output_still_falls_back() {
        for output in [
            "",
            " \n\t",
            "正文\0后文",
            "正文\u{001b}",
            "\u{000b}正文",
            "正文\u{000c}",
        ] {
            assert!(validate_output(output).is_err());
        }
    }
    #[test]
    fn chunk_boundaries_keep_protected_source_spans_whole() {
        let raw = "用 `x=5`，保存 C:\\work\\a.txt，发到 a@test.com，访问 https://a.test/path。";
        let spans = protected_ranges(raw)
            .into_iter()
            .map(|r| &raw[r])
            .collect::<Vec<_>>();
        assert_eq!(
            spans,
            vec![
                "`x=5`",
                "C:\\work\\a.txt",
                "a@test.com",
                "https://a.test/path"
            ]
        );
        assert_eq!(retain_boundaries("\n原文\n", "整理正文"), "\n整理正文\n");
    }
}
