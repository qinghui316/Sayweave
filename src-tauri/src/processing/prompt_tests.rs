use super::prompt::*;
use crate::contracts::dictionary::Kind;
#[test]
fn persisted_legacy_mapping_survives_term_only_processing_and_reopen() {
    use crate::{contracts::dictionary::Input, data::repository::Repository};
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let mapping = repo
        .save(
            Input {
                kind: Kind::Correction,
                original: "听写旧词".into(),
                replacement: Some("旧映射目标".into()),
                category: None,
            },
            None,
        )
        .unwrap();
    let term = repo
        .save(
            Input {
                kind: Kind::Term,
                original: "Sayweave".into(),
                replacement: None,
                category: None,
            },
            None,
        )
        .unwrap();
    let index = DictionaryIndex::new(vec![mapping.clone(), term]);
    let prompt = build("听写旧词", "", &index).unwrap();
    assert!(!prompt.messages[1].content.contains("旧映射目标"));
    drop(repo);
    let repo = Repository::open(root.path()).unwrap();
    assert_eq!(
        serde_json::to_value(repo.get(Kind::Correction, &mapping.id).unwrap()).unwrap(),
        serde_json::to_value(mapping).unwrap()
    );
}
