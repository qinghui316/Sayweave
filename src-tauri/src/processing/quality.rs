//! Conservative structural checks. These do not prove complete semantic fidelity.
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
struct Fact {
    range: Range<usize>,
    text: String,
}
fn facts(input: &str) -> Vec<Fact> {
    let mut result = Vec::new();
    let mut cursor = 0;
    while cursor < input.len() {
        let rest = &input[cursor..];
        let first = rest.chars().next().unwrap();
        let end = if first.is_ascii_digit() {
            cursor
                + rest
                    .char_indices()
                    .find_map(|(i, c)| (!c.is_ascii_digit() && !".,:/%+-".contains(c)).then_some(i))
                    .unwrap_or(rest.len())
        } else if rest.starts_with('周')
            && rest
                .chars()
                .nth(1)
                .is_some_and(|c| "一二三四五六日天".contains(c))
        {
            cursor + '周'.len_utf8() + rest.chars().nth(1).unwrap().len_utf8()
        } else {
            cursor += first.len_utf8();
            continue;
        };
        let mut end = end;
        while end > cursor && ".,:/+-".contains(input[..end].chars().next_back().unwrap()) {
            end -= 1;
        }
        if end == cursor {
            cursor += first.len_utf8();
            continue;
        }
        for unit in [
            "万元", "美元", "公里", "毫秒", "GB", "MB", "kg", "ms", "元", "天", "年", "月", "日",
            "个", "米", "秒",
        ] {
            if input[end..].starts_with(unit) {
                end += unit.len();
                break;
            }
        }
        result.push(Fact {
            range: cursor..end,
            text: input[cursor..end].into(),
        });
        cursor = end;
    }
    result
}
fn correction_gap(text: &str) -> bool {
    let stripped = text.trim_matches(|c: char| c.is_whitespace() || "，,：:".contains(c));
    matches!(stripped, "" | "是" | "应该是" | "改为" | "说的是")
}
fn expected_facts(input: &str) -> Vec<String> {
    let facts = facts(input);
    let mut corrected = Vec::new();
    for marker in ["不对", "说错了", "更正为"] {
        for (at, _) in input.match_indices(marker) {
            let before = facts
                .iter()
                .enumerate()
                .rev()
                .find(|(_, f)| f.range.end <= at);
            let after = facts.iter().find(|f| f.range.start >= at + marker.len());
            if let (Some((index, old)), Some(new)) = (before, after)
                && correction_gap(&input[old.range.end..at])
                && correction_gap(&input[at + marker.len()..new.range.start])
            {
                corrected.push(index);
            }
        }
    }
    facts
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !corrected.contains(i))
        .map(|(_, f)| f.text)
        .collect()
}
fn negations(input: &str) -> Vec<&'static str> {
    let mut rest = input;
    let mut result = Vec::new();
    while !rest.is_empty() {
        if rest.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) {
            let end = rest
                .char_indices()
                .find_map(|(i, c)| (!c.is_ascii_alphabetic() && c != '\'').then_some(i))
                .unwrap_or(rest.len());
            let word = rest[..end].to_ascii_lowercase();
            match word.as_str() {
                "not" | "don't" | "doesn't" | "isn't" | "aren't" | "wasn't" | "weren't" => {
                    result.push("not")
                }
                "cannot" | "can't" => result.push("unable"),
                "no" => result.push("none"),
                "never" => result.push("never"),
                "won't" => result.push("willNot"),
                _ => {}
            }
            rest = &rest[end..];
            continue;
        }
        let found = [
            ("不对", ""),
            ("说错了", ""),
            ("不要", "prohibit"),
            ("不能", "unable"),
            ("没有", "absent"),
            ("不是", "not"),
            ("不", "not"),
            ("别", "prohibit"),
            ("未", "uncompleted"),
        ]
        .into_iter()
        .find(|(word, _)| rest.starts_with(word));
        if let Some((word, meaning)) = found {
            if !meaning.is_empty() {
                result.push(meaning);
            }
            rest = &rest[word.len()..];
        } else {
            rest = &rest[rest.chars().next().unwrap().len_utf8()..];
        }
    }
    result
}
pub fn validate(input: &str, output: &str) -> Result<(), ProcessingError> {
    let output = output.trim();
    let input_chars = input.chars().count();
    let output_chars = output.chars().count();
    if output.is_empty() {
        return Err(ProcessingError::new("emptyText"));
    }
    if output_chars > input_chars.saturating_mul(2).saturating_add(32)
        || (input_chars > 40 && output_chars * 4 < input_chars)
    {
        return Err(ProcessingError::new("qualityLength"));
    }
    if output
        .chars()
        .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(ProcessingError::new("qualityControl"));
    }
    if output.starts_with("```") && !input.starts_with("```") {
        return Err(ProcessingError::new("qualityWrapper"));
    }
    for range in protected_ranges(input) {
        let token = &input[range];
        if input.matches(token).count() != output.matches(token).count() {
            return Err(ProcessingError::new("qualityProtected"));
        }
    }
    if expected_facts(input)
        != facts(output)
            .into_iter()
            .map(|f| f.text)
            .collect::<Vec<_>>()
    {
        return Err(ProcessingError::new("qualityFacts"));
    }
    if negations(input) != negations(output) {
        return Err(ProcessingError::new("qualityNegation"));
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn correction_relaxes_only_nearest_fact_and_never_url_or_other_numbers() {
        let raw = "呃，预算5000元，周五，不对，周四再上线，先别发公告。";
        assert!(validate(raw, "预算5000元，周四再上线，先别发公告。").is_ok());
        assert!(validate(raw, "预算6000元，周四再上线，先别发公告。").is_err());
        assert!(validate(raw, "预算5000元，周四再上线，发公告。").is_err());
        let raw = "保留 https://a.test/path 和预算5000元，日期5，不对，6。";
        assert!(validate(raw, "保留 https://a.test/path 和预算5000元，日期6。").is_ok());
        assert!(validate(raw, "保留 https://b.test/path 和预算5000元，日期6。").is_err());
    }
    #[test]
    fn code_email_paths_and_newline_preserved() {
        let raw = "呃，用 `x=5`，保存 C:\\work\\a.txt，发到 a@test.com。";
        assert!(validate(raw, "用 `x=5`，保存 C:\\work\\a.txt，发到 a@test.com。").is_ok());
        assert!(validate(raw, "用 `x=6`，保存 C:\\work\\a.txt，发到 a@test.com。").is_err());
        assert!(validate("第一段。第二段。", "第一段。\n第二段。").is_ok());
        assert!(
            validate(
                "Uh, we should not send it today.",
                "We should not send it today."
            )
            .is_ok()
        );
        assert!(
            validate(
                "Uh, we should not send it today.",
                "We should send it today."
            )
            .is_err()
        );
        assert!(validate("保留`abc=xyz`。", "保留`abc=def`。").is_err());
    }
}
