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
    let DbOutput::Config(initial) = repo.shortcut_operation(DbOperation::Load).unwrap() else {
        panic!()
    };
    assert_eq!(initial, Config::default());
    let DbOutput::Config(saved) = repo
        .shortcut_operation(DbOperation::Save(Config {
            binding: "Control+Space".into(),
            mode: Mode::Hold,
            ..initial.clone()
        }))
        .unwrap()
    else {
        panic!()
    };
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
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(reopened, saved);
}
#[test]
fn migration_journal_refuses_late_writes_and_never_replays() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    repo.connection
        .execute(
            "INSERT INTO speech_records(id,created_at,status) VALUES('r',1,'success')",
            [],
        )
        .unwrap();
    repo.connection.execute("INSERT INTO speech_attempts(id,record_id,kind,config_id,config_name,model_id,service_host,config_revision,created_at,status) VALUES('a','r','live','c','test','model','host',1,1,'success')",[]).unwrap();
    let write = |task: &str, attempt: &str, state: &str| DbOperation::Delivery {
        task: task.into(),
        attempt: attempt.into(),
        state: state.into(),
        error: None,
    };
    repo.shortcut_operation(write("t", "a", "pending")).unwrap();
    repo.shortcut_operation(write("t", "a", "delivering"))
        .unwrap();
    repo.shortcut_operation(write("t", "a", "confirmed"))
        .unwrap();
    assert_eq!(
        repo.shortcut_operation(write("t", "a", "pending"))
            .unwrap_err()
            .code,
        "conflict"
    );
    assert!(
        repo.shortcut_operation(write("t", "other", "confirmed"))
            .is_err()
    );
    repo.shortcut_operation(write("incomplete", "a", "submitted"))
        .unwrap();
    repo.shortcut_operation(DbOperation::Recover).unwrap();
    let state: String = repo
        .connection
        .query_row(
            "SELECT state FROM delivery_attempts WHERE task_id='incomplete'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "unconfirmed");
    let state: String = repo
        .connection
        .query_row(
            "SELECT state FROM delivery_attempts WHERE task_id='t'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(state, "confirmed");
    let DbOutput::Deliveries(rows) = repo
        .shortcut_operation(DbOperation::Deliveries("r".into()))
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .any(|r| r.state == "unconfirmed" && r.error.as_deref() == Some("interrupted"))
    );
    repo.connection
        .execute("DELETE FROM speech_records WHERE id='r'", [])
        .unwrap();
    let count: i64 = repo
        .connection
        .query_row("SELECT COUNT(*) FROM delivery_attempts", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 0);
}
