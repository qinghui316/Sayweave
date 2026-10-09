//! Correct-word hints only. Provider adapters own the wire representation.
pub(super) fn select(words: &[String], limit: usize) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut bytes: usize = 0;
    words
        .iter()
        .map(|word| word.trim())
        .filter(|word| {
            !word.is_empty() && word.chars().count() <= 128 && !word.chars().any(char::is_control)
        })
        .filter(|word| seen.insert(crate::contracts::dictionary::key(word)))
        .filter(|word| {
            // Reserve per-item structure and double JSON escaping used by
            // Doubao's context string. Never split a term to fit the request.
            let encoded = serde_json::to_string(word).unwrap_or_default();
            let cost =
                serde_json::to_vec(&encoded).map_or(usize::MAX, |s| s.len().saturating_add(32));
            if bytes.saturating_add(cost) > 8 * 1024 {
                return false;
            }
            bytes += cost;
            true
        })
        .take(limit)
        .map(str::to_owned)
        .collect()
}
#[cfg(test)]
mod tests {
    #[test]
    fn large_dictionary_hints_stay_bounded_without_truncating_words() {
        let words = (0..3000)
            .map(|n| format!("{n}{}", "词".repeat(120)))
            .collect::<Vec<_>>();
        let selected = super::select(&words, 100);
        assert!(!selected.is_empty() && selected.len() < 100);
        let context=serde_json::json!({"hotwords":selected.iter().map(|s|serde_json::json!({"word":s})).collect::<Vec<_>>()}).to_string();
        assert!(serde_json::to_vec(&context).unwrap().len() < 8 * 1024);
        assert!(selected.iter().all(|word| words.contains(word)));
    }
    #[test]
    fn correct_spelling_preserved_and_normalized_duplicates_bounded() {
        assert_eq!(
            super::select(
                &[
                    " Sayweave ".into(),
                    "sayweave".into(),
                    "绮蓝引擎".into(),
                    "\n".into()
                ],
                100
            ),
            vec!["Sayweave", "绮蓝引擎"]
        );
        assert_eq!(super::select(&["one".into(), "two".into()], 1), vec!["one"]);
    }
}
