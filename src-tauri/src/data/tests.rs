use super::{
    repository::{MIGRATIONS, configure, migrate},
    *,
};
use rusqlite::{Connection, params};
use std::time::{Duration, Instant};
fn term(value: &str) -> Input {
    Input {
        kind: Kind::Term,
        original: value.into(),
        category: None,
        replacement: None,
    }
}
fn correction(value: &str, target: &str) -> Input {
    Input {
        kind: Kind::Correction,
        original: value.into(),
        category: None,
        replacement: Some(target.into()),
    }
}
fn query(kind: Kind, search: &str, page: u32) -> Query {
    Query {
        kind,
        search: search.into(),
        page,
        source: None,
    }
}
#[test]
fn normalization_validation_and_global_conflicts() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let first = repo.save(term(" Figma "), None).unwrap();
    assert_eq!(first.original, "Figma");
    assert_eq!(
        repo.save(term("figma"), None).unwrap_err().existing_id,
        Some(first.id)
    );
    assert_eq!(key("e\u{301}"), key("é"));
    assert_ne!(key("e"), key("é"));
    assert_ne!(key("Ａ"), key("A"));
    for invalid in ["", " ", "a\n", "a\0b", "a\u{2028}b", "a\u{2029}b"] {
        assert_eq!(repo.save(term(invalid), None).unwrap_err().code, "invalid");
    }
    assert!(repo.save(term(&"字".repeat(128)), None).is_ok());
    assert!(repo.save(term(&"字".repeat(129)), None).is_err());
    let mut value = term("类别");
    value.category = Some(" ".into());
    assert!(repo.save(value, None).unwrap().category.is_none());
    let mut control_category = term("控制字符分类");
    control_category.category = Some("\n".into());
    assert_eq!(
        repo.save(control_category, None).unwrap_err().code,
        "invalid"
    );
    let original = repo.save(correction("飞格玛", "Figma"), None).unwrap();
    assert_eq!(
        repo.save(correction("飞格玛", "Other"), None)
            .unwrap_err()
            .existing_id,
        Some(original.id)
    );
    assert!(repo.save(term("飞格玛"), None).is_ok());
    assert!(repo.save(correction("figma", "Figma"), None).is_ok());
}
#[test]
fn optimistic_crud_and_actual_reopen() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let first = repo.save(term("旧名称"), None).unwrap();
    let edit = repo
        .save(term("新名称"), Some((first.id.clone(), first.revision)))
        .unwrap();
    assert_eq!(edit.id, first.id);
    assert_eq!(edit.created_at, first.created_at);
    assert_eq!(edit.revision, 2);
    assert_eq!(
        repo.save(term("过期"), Some((first.id.clone(), 1)))
            .unwrap_err()
            .code,
        "conflict"
    );
    assert_eq!(
        repo.delete(Kind::Term, &first.id, 1).unwrap_err().code,
        "conflict"
    );
    drop(repo);
    let mut repo = Repository::open(root.path()).unwrap();
    assert_eq!(repo.get(Kind::Term, &first.id).unwrap().original, "新名称");
    repo.delete(Kind::Term, &first.id, 2).unwrap();
    drop(repo);
    assert_eq!(
        Repository::open(root.path())
            .unwrap()
            .get(Kind::Term, &first.id)
            .unwrap_err()
            .code,
        "notFound"
    );
}
#[test]
fn literal_contains_pagination_and_strict_constraints() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    for i in 0..55 {
        repo.save(term(&format!("词条{i}")), None).unwrap();
    }
    repo.save(term("100%_正确"), None).unwrap();
    assert_eq!(repo.list(query(Kind::Term, "%_", 1)).unwrap().total, 1);
    let first = repo.list(query(Kind::Term, "", 1)).unwrap();
    let last = repo.list(query(Kind::Term, "", 100)).unwrap();
    assert_eq!(first.records.len(), 50);
    assert_eq!(last.records.len(), 6);
    assert_eq!(last.page, 2);
    assert!(
        !last
            .records
            .iter()
            .any(|r| first.records.iter().any(|f| f.id == r.id))
    );
    assert!(
        repo.connection
            .execute("UPDATE dictionary_terms SET revision='invalid'", [])
            .is_err()
    );
    assert!(
        repo.connection
            .execute(
                "UPDATE dictionary_terms SET normalized_term='duplicate'",
                []
            )
            .is_err()
    );
}
#[test]
fn legacy_migration_newlines_preserve_ledger_and_data() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let saved = repo.save(term("换行兼容词条"), None).unwrap();
    let windows: Vec<_> = MIGRATIONS
        .iter()
        .map(|(name, sql)| (*name, sql.replace("\r\n", "\n").replace('\n', "\r\n")))
        .collect();
    for (i, (_, sql)) in windows.iter().enumerate().skip(3) {
        repo.connection
            .execute(
                "UPDATE schema_migrations SET checksum=?1 WHERE version=?2",
                rusqlite::params![
                    format!("{:x}", Sha256::digest(sql.as_bytes())),
                    i as i64 + 1
                ],
            )
            .unwrap();
    }
    let ledger = |connection: &Connection| {
        connection
            .prepare("SELECT checksum FROM schema_migrations ORDER BY version")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let before = ledger(&repo.connection);
    migrate(&mut repo.connection, root.path(), MIGRATIONS).unwrap();
    let windows: Vec<_> = windows
        .iter()
        .map(|(name, sql)| (*name, sql.as_str()))
        .collect();
    migrate(&mut repo.connection, root.path(), &windows).unwrap();
    assert_eq!(ledger(&repo.connection), before);
    assert_eq!(
        repo.get(Kind::Term, &saved.id).unwrap().original,
        "换行兼容词条"
    );
    assert!(!root.path().join("backups").exists());
    let mut changed = MIGRATIONS.to_vec();
    changed[4].1 = "SELECT 1;";
    assert!(migrate(&mut repo.connection, root.path(), &changed).is_err());
    assert_eq!(ledger(&repo.connection), before);
}
#[test]
fn new_migration_checksums_are_stable_across_checkout_newlines() {
    use sha2::{Digest, Sha256};
    let root = tempfile::tempdir().unwrap();
    let mut connection = Connection::open(root.path().join("dictionary.sqlite")).unwrap();
    let windows: Vec<_> = MIGRATIONS
        .iter()
        .map(|(name, sql)| (*name, sql.replace("\r\n", "\n").replace('\n', "\r\n")))
        .collect();
    let borrowed: Vec<_> = windows
        .iter()
        .map(|(name, sql)| (*name, sql.as_str()))
        .collect();
    migrate(&mut connection, root.path(), &borrowed).unwrap();
    for (i, (_, sql)) in MIGRATIONS.iter().enumerate() {
        let checksum: String = connection
            .query_row(
                "SELECT checksum FROM schema_migrations WHERE version=?1",
                [i as i64 + 1],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            checksum,
            format!("{:x}", Sha256::digest(sql.replace("\r\n", "\n").as_bytes()))
        );
    }
    migrate(&mut connection, root.path(), MIGRATIONS).unwrap();
    assert!(!root.path().join("backups").exists());
}
#[test]
fn migrations_backup_rollback_checksum_future_and_retention() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let saved = repo.save(term("升级前词条"), None).unwrap();
    drop(repo);
    let mut connection = Connection::open(root.path().join("dictionary.sqlite")).unwrap();
    let mut migrations = MIGRATIONS.to_vec();
    migrations.push(("second", "CREATE TABLE second(value INTEGER) STRICT;"));
    migrate(&mut connection, root.path(), &migrations).unwrap();
    let backup = std::fs::read_dir(root.path().join("backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let snapshot = Connection::open(&backup).unwrap();
    assert_eq!(
        snapshot
            .query_row("SELECT count(*) FROM schema_migrations", [], |r| r
                .get::<_, i32>(0))
            .unwrap(),
        MIGRATIONS.len() as i32
    );
    drop(snapshot);
    // Restore only into an independent copy: the live upgraded database is untouched.
    let restored = tempfile::tempdir().unwrap();
    fs::copy(&backup, restored.path().join("dictionary.sqlite")).unwrap();
    let restored_repo = Repository::open(restored.path()).unwrap();
    assert_eq!(
        restored_repo.get(Kind::Term, &saved.id).unwrap().original,
        "升级前词条"
    );
    let mut invalid = migrations.clone();
    invalid.push((
        "bad",
        "CREATE TABLE bad(value INTEGER); THIS IS INVALID SQL;",
    ));
    assert!(migrate(&mut connection, root.path(), &invalid).is_err());
    assert_eq!(
        connection
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE name='bad'",
                [],
                |r| r.get::<_, i32>(0)
            )
            .unwrap(),
        0
    );
    assert!(
        std::fs::read_dir(root.path().join("backups"))
            .unwrap()
            .any(|e| e.unwrap().path().to_string_lossy().contains("protected"))
    );
    assert!(migrate(&mut connection, root.path(), MIGRATIONS).is_err());
    let mut changed = migrations.clone();
    changed[0].1 = "SELECT 1;";
    assert!(migrate(&mut connection, root.path(), &changed).is_err());
    for (name, sql) in [
        ("third", "CREATE TABLE third(value INTEGER) STRICT;"),
        ("fourth", "CREATE TABLE fourth(value INTEGER) STRICT;"),
        ("fifth", "CREATE TABLE fifth(value INTEGER) STRICT;"),
    ] {
        migrations.push((name, sql));
        migrate(&mut connection, root.path(), &migrations).unwrap();
    }
    let paths: Vec<_> = std::fs::read_dir(root.path().join("backups"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(
        paths
            .iter()
            .filter(|p| p.to_string_lossy().contains("successful"))
            .count(),
        3
    );
    assert_eq!(
        paths
            .iter()
            .filter(|p| p.to_string_lossy().contains("protected"))
            .count(),
        1
    );
}
#[test]
fn corrupt_unknown_and_backup_failure_preserve_source() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("dictionary.sqlite");
    fs::write(&path, b"not sqlite").unwrap();
    assert!(Repository::open(root.path()).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"not sqlite");
    let unknown = tempfile::tempdir().unwrap();
    let c = Connection::open(unknown.path().join("dictionary.sqlite")).unwrap();
    c.execute("CREATE TABLE unknown(value INTEGER)", [])
        .unwrap();
    drop(c);
    assert!(Repository::open(unknown.path()).is_err());
    let root = tempfile::tempdir().unwrap();
    let repo = Repository::open(root.path()).unwrap();
    drop(repo);
    fs::write(root.path().join("backups"), b"blocked").unwrap();
    let mut c = Connection::open(root.path().join("dictionary.sqlite")).unwrap();
    let mut migrations = MIGRATIONS.to_vec();
    migrations.push(("second", "CREATE TABLE second(value INTEGER);"));
    assert!(migrate(&mut c, root.path(), &migrations).is_err());
    assert_eq!(
        c.query_row("SELECT count(*) FROM schema_migrations", [], |r| r
            .get::<_, u32>(0))
            .unwrap(),
        MIGRATIONS.len() as u32
    );
}
#[test]
fn busy_database_and_directory_lease() {
    let root = tempfile::tempdir().unwrap();
    let first = acquire(root.path()).unwrap();
    assert_eq!(acquire(root.path()).unwrap_err().code, "alreadyRunning");
    drop(first);
    assert!(acquire(root.path()).is_ok());
    let mut repo = Repository::open(root.path()).unwrap();
    let other = Connection::open(root.path().join("dictionary.sqlite")).unwrap();
    other.execute_batch("BEGIN IMMEDIATE").unwrap();
    repo.connection
        .busy_timeout(Duration::from_millis(30))
        .unwrap();
    assert_eq!(repo.save(term("busy"), None).unwrap_err().code, "busy");
    other.execute_batch("ROLLBACK").unwrap();
    assert!(repo.save(term("busy"), None).is_ok());
    assert_ne!(
        profile_root(root.path(), true),
        profile_root(root.path(), false)
    );
}
#[test]
fn worker_authorization_recovery_and_bounded_queue() {
    tauri::async_runtime::block_on(async {
        let root = tempfile::tempdir().unwrap();
        let worker = DataWorker::start(root.path().to_owned()).unwrap();
        assert_eq!(
            worker
                .request("voice-overlay", Operation::List(query(Kind::Term, "", 1)))
                .await
                .unwrap_err()
                .code,
            "forbidden"
        );
        assert!(
            matches!(worker.request("main",Operation::Retry).await.unwrap(),Output::Status(Status{state,..}) if state=="ready")
        );
        assert!(matches!(
            worker
                .request("main", Operation::Create(term("持久化")))
                .await
                .unwrap(),
            Output::Record(_)
        ));
        // Fill a standalone queue deterministically without racing a consumer.
        let (sender, _receiver) = mpsc::channel(64);
        let fake = DataWorker {
            sender,
            status: worker.status.clone(),
            stopping: Arc::new(AtomicBool::new(false)),
            dispatch: Arc::new(Mutex::new(())),
            root: worker.root.clone(),
            journal_io: worker.journal_io.clone(),
            deferred_cancellations: worker.deferred_cancellations.clone(),
            lease_active: worker.lease_active.clone(),
        };
        for _ in 0..64 {
            let (response, _) = oneshot::channel();
            fake.sender
                .try_send(Request {
                    operation: Operation::Retry,
                    response,
                    queued: Instant::now(),
                })
                .unwrap();
        }
        assert_eq!(
            fake.request("main", Operation::Retry)
                .await
                .unwrap_err()
                .code,
            "busy"
        );
        let blocked = tempfile::tempdir().unwrap();
        let path = blocked.path().join("directory");
        fs::write(&path, b"blocked").unwrap();
        let recovering = DataWorker::start(path.clone()).unwrap();
        assert!(
            matches!(recovering.request("main",Operation::Retry).await.unwrap(),Output::Status(Status{state,..}) if state=="unavailable")
        );
        fs::remove_file(&path).unwrap();
        assert!(
            matches!(recovering.request("main",Operation::Retry).await.unwrap(),Output::Status(Status{state,..}) if state=="ready")
        );
    });
}
#[test]
fn cross_language_dictionary_fixture() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("../../../tests/contracts/dictionary.json")).unwrap();
    let record: Record = serde_json::from_value(fixture["record"].clone()).unwrap();
    assert_eq!(serde_json::to_value(record).unwrap(), fixture["record"]);
    for sample in fixture["keys"].as_array().unwrap() {
        assert_eq!(
            key(sample["input"].as_str().unwrap()),
            sample["key"].as_str().unwrap()
        );
    }
    let status: Status = serde_json::from_value(fixture["status"].clone()).unwrap();
    assert_eq!(serde_json::to_value(status).unwrap(), fixture["status"]);
}
#[test]
#[ignore = "explicit disk benchmark; writes an evidence report under .local"]
fn dictionary_query_benchmark() {
    let mut reports = Vec::new();
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.local");
    fs::create_dir_all(&directory).unwrap();
    for count in [1_000, 10_000, 100_000] {
        let root = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(root.path()).unwrap();
        configure(&repo.connection).unwrap();
        {
            let tx = repo.connection.transaction().unwrap();
            {
                let mut insert = tx
                    .prepare("INSERT INTO dictionary_terms VALUES(?1,?2,?2,NULL,NULL,1,?3,?3)")
                    .unwrap();
                for i in 0..count {
                    let value = if i % 10 == 0 {
                        format!("figma{i:08}")
                    } else {
                        format!("常用词条{i:08}")
                    };
                    insert
                        .execute(params![format!("id-{i:08}"), value, i])
                        .unwrap();
                }
            }
            tx.commit().unwrap();
        }
        let plan = |sql: &str| {
            let mut stmt = repo.connection.prepare(sql).unwrap();
            stmt.query_map([], |r| r.get::<_, String>(3))
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap()
                .join("; ")
        };
        let exact = plan(
            "EXPLAIN QUERY PLAN SELECT id FROM dictionary_terms WHERE normalized_term='常用词条00000001'",
        );
        let order = plan(
            "EXPLAIN QUERY PLAN SELECT * FROM dictionary_terms WHERE (''='' OR instr(normalized_term,'')>0) ORDER BY updated_at DESC,id ASC LIMIT 50 OFFSET 50",
        );
        assert!(exact.contains("SEARCH"));
        assert!(order.contains("terms_updated"));
        assert!(!order.contains("TEMP B-TREE"));
        let mut timings = Vec::new();
        for (name, search, page) in [
            ("list", "", 1),
            ("last-page", "", u32::MAX),
            ("chinese-short", "词", 1),
            ("english", "figma", 1),
            ("no-result", "不存在", 1),
        ] {
            let mut samples = Vec::new();
            let cold = Instant::now();
            repo.list(query(Kind::Term, search, page)).unwrap();
            let cold_ms = cold.elapsed().as_secs_f64() * 1000.0;
            for _ in 0..50 {
                let at = Instant::now();
                repo.list(query(Kind::Term, search, page)).unwrap();
                samples.push(at.elapsed().as_secs_f64() * 1000.0);
            }
            samples.sort_by(f64::total_cmp);
            let p95 = samples[47];
            if count == 10_000 {
                assert!(
                    p95 <= if search.is_empty() { 50.0 } else { 100.0 },
                    "{name} exceeded: {p95}ms"
                );
            }
            timings.push(serde_json::json!({"operation":name,"firstMs":cold_ms,"p95Ms":p95}));
        }
        let mut writes = Vec::new();
        for i in 0..50 {
            let at = Instant::now();
            let r = repo.save(term(&format!("写入{i}")), None).unwrap();
            writes.push(at.elapsed().as_secs_f64() * 1000.0);
            repo.delete(Kind::Term, &r.id, r.revision).unwrap();
        }
        writes.sort_by(f64::total_cmp);
        if count == 10_000 {
            assert!(writes[47] <= 50.0);
        }
        reports.push(serde_json::json!({"rows":count,"exactPlan":exact,"orderPlan":order,"queries":timings,"writeP95Ms":writes[47]}));
        if count == 10_000 {
            let destination = directory.join("dictionary-benchmark-10000.sqlite");
            if destination.exists() {
                fs::remove_file(&destination).unwrap();
            }
            let mut snapshot = Connection::open(destination).unwrap();
            let backup = rusqlite::backup::Backup::new(&repo.connection, &mut snapshot).unwrap();
            backup
                .run_to_completion(100, Duration::from_millis(1), None)
                .unwrap();
        }
    }
    fs::write(
        directory.join("dictionary-benchmark.json"),
        serde_json::to_vec_pretty(&reports).unwrap(),
    )
    .unwrap();
}
