//! Pure input boundary policy: a trailing paragraph separator stays in the
//! owned draft until there is following committed text. No target-specific offsets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettlementBoundary {
    pub committed: String,
    pub retained_separator: String,
}
impl SettlementBoundary {
    /// Previous separators belong to the current owned range, never to the
    /// already-submitted prefix. The caller advances its watermark only after
    /// native acknowledgement and retains this separator across tail updates.
    pub fn prepare(previous_separator: &str, processed: &str) -> Option<Self> {
        if !previous_separator.chars().all(|c| matches!(c, '\r' | '\n')) {
            return None;
        }
        let body = processed.trim_end_matches(['\r', '\n']);
        if body.is_empty() {
            return None; // No irreversible submission of separator-only units.
        }
        Some(Self {
            committed: format!("{previous_separator}{body}"),
            retained_separator: processed[body.len()..].into(),
        })
    }
    pub fn tail(&self, latest_tail: &str) -> String {
        format!("{}{latest_tail}", self.retained_separator)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn two_units_keep_every_separator_exactly_once() {
        let first = SettlementBoundary::prepare("", "第一段🙂\r\n\n").unwrap();
        assert_eq!(first.committed, "第一段🙂");
        assert_eq!(first.tail("临时"), "\r\n\n临时");
        assert_eq!(first.tail("最新尾部"), "\r\n\n最新尾部");
        let next = SettlementBoundary::prepare(&first.retained_separator, "第二段\n").unwrap();
        assert_eq!(
            format!("{}{}{}", first.committed, next.committed, next.tail("结束")),
            "第一段🙂\r\n\n第二段\n结束"
        );
        // Cancellation removes the owned tail, including its still-unsubmitted
        // separator, while the exact submitted text remains independently known.
        assert_eq!(
            format!("{}{}", first.committed, next.committed),
            "第一段🙂\r\n\n第二段"
        );
    }
    #[test]
    fn empty_tail_and_non_generated_unicode_are_preserved() {
        let unit = SettlementBoundary::prepare("", "原有\u{2028}行\n").unwrap();
        assert_eq!(unit.committed, "原有\u{2028}行");
        assert_eq!(unit.tail(""), "\n");
        assert!(SettlementBoundary::prepare("", "\r\n").is_none());
        assert!(SettlementBoundary::prepare("旧正文", "新正文").is_none());
        assert!(SettlementBoundary::prepare("", "").is_none());
    }
}
