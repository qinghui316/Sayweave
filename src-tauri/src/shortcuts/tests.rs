use super::types::*;
use crate::data::repository::Repository;
#[test]
fn keys_and_revision_are_explicit() {
    assert_eq!(Binding::parse("F01").unwrap().canonical(), "F1");
    assert_eq!(
        Binding::parse("Alt+Control+KeyD").unwrap().canonical(),
        "Control+Alt+KeyD"
    );
    assert!(Binding::parse("AltRight").unwrap().single_modifier());
    for invalid in [
        "",
        "Ctrl+A",
        "Control+Control+KeyD",
        "Control+AltRight",
        "Super+KeyL",
        "F25",
        "Control+",
    ] {
        assert!(Binding::parse(invalid).is_err(), "{invalid}");
    }
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let DbOutput::Config(initial) = repo.shortcut_operation(DbOperation::Load).unwrap();
    assert_eq!(initial, Config::default());
    let DbOutput::Config(saved) = repo
        .shortcut_operation(DbOperation::Save(Config {
            binding: "Control+Space".into(),
            mode: Mode::Hold,
            ..initial.clone()
        }))
        .unwrap();
    assert_eq!(saved.revision, 2);
    assert_eq!(
        repo.shortcut_operation(DbOperation::Save(initial))
            .unwrap_err()
            .code,
        "conflict"
    );
    drop(repo);
    let DbOutput::Config(reopened) = Repository::open(root.path())
        .unwrap()
        .shortcut_operation(DbOperation::Load)
        .unwrap();
    assert_eq!(reopened, saved);
}
