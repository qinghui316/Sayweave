//! Native UTF-16 geometry is separate from ASR source-unit consumption.
pub(crate) fn boundary(actual: &str, semantic: &str) -> Option<(usize, i32)> {
    let semantic = semantic.replace("\r\n", "\n").replace('\r', "\n");
    let mut actual_chars = actual.chars().peekable();
    let mut bytes = 0;
    let mut utf16 = 0;
    for expected in semantic.chars() {
        let value = actual_chars.next()?;
        bytes += value.len_utf8();
        utf16 += value.len_utf16() as i32;
        if expected == '\n' && value == '\r' {
            if actual_chars.peek() == Some(&'\n') {
                actual_chars.next();
                bytes += 1;
                utf16 += 1;
            }
        } else if value != expected {
            return None;
        }
    }
    Some((bytes, utf16))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn provider_line_endings_change_geometry_without_changing_source_identity() {
        assert_eq!(boundary("前🙂\r\n尾", "前🙂\n"), Some((9, 5)));
        assert_eq!(boundary("前🙂\n尾", "前🙂\n"), Some((8, 4)));
        assert_eq!(boundary("前\u{2028}尾", "前\n"), None);
        assert_eq!(boundary("前\u{2028}尾", "前\u{2028}"), Some((6, 2)));
        assert_eq!(boundary("同前尾", "不同"), None);
    }
}
