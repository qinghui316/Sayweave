use super::types::*;
use crate::data::repository::Repository;
#[test]
fn defaults_share_a_tap_prefix_but_hold_and_reserved_keys_are_rejected() {
    let base = Config::default();
    assert_eq!(
        base.clone().validate().unwrap().translation_binding,
        "AltRight+ShiftRight"
    );
    assert_eq!(
        Binding::parse("ShiftRight+AltRight").unwrap().canonical(),
        "AltRight+ShiftRight"
    );
    assert_eq!(
        Config {
            mode: Mode::Hold,
            ..base.clone()
        }
        .validate()
        .unwrap_err()
        .code,
        "shortcutHoldConflict"
    );
    for key in [
        "AltLeft",
        "Alt+KeyA",
        "Alt+Tab",
        "Alt+F4",
        "Alt+Space",
        "Alt+Shift+Tab",
        "Super+KeyL",
        "F12",
        "Control+Escape",
        "Control+Shift+Escape",
        "Control+Alt+Delete",
    ] {
        for ordinary in [true, false] {
            let mut c = base.clone();
            if ordinary {
                c.binding = key.into();
            } else {
                c.translation_binding = key.into();
            }
            assert_eq!(
                c.validate().unwrap_err().code,
                "shortcutSystemReserved",
                "{key}"
            );
        }
    }
    assert_eq!(
        Config {
            translation_binding: "AltRight".into(),
            ..base.clone()
        }
        .validate()
        .unwrap_err()
        .code,
        "shortcutConflict"
    );
    assert!(
        Config {
            translation_binding: "F9".into(),
            mode: Mode::Hold,
            ..base
        }
        .validate()
        .is_ok()
    );
}
#[test]
fn modifier_pair_rejects_another_generic_alt_chord_in_either_role() {
    for key in ["Alt+F8", "Control+Alt+F8"] {
        for reverse in [false, true] {
            let (binding, translation_binding) = if reverse {
                ("AltRight+ShiftRight", key)
            } else {
                (key, "AltRight+ShiftRight")
            };
            assert_eq!(
                Config {
                    binding: binding.into(),
                    translation_binding: translation_binding.into(),
                    ..Config::default()
                }
                .validate()
                .unwrap_err()
                .code,
                "shortcutConflict"
            );
        }
    }
}
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
        "F25",
        "Control+",
    ] {
        assert!(Binding::parse(invalid).is_err(), "{invalid}");
    }
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let DbOutput::Config(initial) = repo.shortcut_operation(DbOperation::Load).unwrap();
    assert_eq!(
        initial,
        Config {
            revision: 2,
            ..Config::default()
        }
    );
    let DbOutput::Config(saved) = repo
        .shortcut_operation(DbOperation::Save(Config {
            binding: "Control+Space".into(),
            translation_binding: "F9".into(),
            mode: Mode::Hold,
            ..initial.clone()
        }))
        .unwrap();
    assert_eq!(saved.revision, initial.revision + 1);
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

#[test]
fn initial_translation_conflict_preserves_original_registration_but_explicit_changes_roll_back() {
    use super::register_additions;
    use tauri_plugin_global_shortcut::Shortcut;
    let ordinary: Shortcut = "Control+Space".parse().unwrap();
    let translation: Shortcut = "F8".parse().unwrap();
    let mut removed = Vec::new();
    let (added, conflict) = register_additions(
        &[ordinary, translation],
        &[],
        Some(translation),
        |key| if key == translation { Err(()) } else { Ok(()) },
        |key| removed.push(key),
    )
    .unwrap();
    assert!(conflict);
    assert_eq!(added, vec![ordinary]);
    assert!(removed.is_empty());
    let result = register_additions(
        &[ordinary, translation],
        &[],
        None,
        |key| if key == translation { Err(()) } else { Ok(()) },
        |key| removed.push(key),
    );
    assert_eq!(result.err().unwrap().code, "shortcutConflict");
    assert_eq!(removed, vec![ordinary]);
}
