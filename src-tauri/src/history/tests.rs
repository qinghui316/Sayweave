use super::*;
use crate::data::repository::Repository;
use rusqlite::{Connection, params};

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}

#[test]
fn migration_v4_to_v5_preserves_configs_defaults_history_and_constraints() {
    use crate::data::repository::{MIGRATIONS, configure, migrate};
    let root = tempfile::tempdir().unwrap();
    let mut c = Connection::open(root.path().join("dictionary.sqlite")).unwrap();
    configure(&c).unwrap();
    migrate(&mut c, root.path(), &MIGRATIONS[..4]).unwrap();
    c.execute("INSERT INTO model_configs VALUES('q','千问','qwen-audio-streaming','https://example.invalid','qwen-model',1,0,'vault-ref',3,10,20)",[]).unwrap();
    c.execute(
        "UPDATE model_defaults SET config_id='q',revision=7 WHERE usage='speech'",
        [],
    )
    .unwrap();
    c.execute(
        "INSERT INTO speech_records(id,created_at,status) VALUES('history',10,'success')",
        [],
    )
    .unwrap();
    migrate(&mut c, root.path(), MIGRATIONS).unwrap();
    let saved: (String, String, i64) = c
        .query_row(
            "SELECT credential_ref,model_id,revision FROM model_configs WHERE id='q'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(saved, ("vault-ref".into(), "qwen-model".into(), 3));
    assert_eq!(
        c.query_row(
            "SELECT revision FROM model_defaults WHERE usage='speech'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        7
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM speech_records", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    c.execute("INSERT INTO model_configs VALUES('d','豆包','doubao-asr-streaming','https://openspeech.bytedance.com','Doubao-Seed-ASR-2.0',1,0,'doubao-ref',1,30,30)",[]).unwrap();
    assert!(
        c.execute("UPDATE model_configs SET text=1 WHERE id='d'", [])
            .is_err()
    );
    assert!(
        c.execute(
            "UPDATE model_defaults SET config_id='d' WHERE usage='text'",
            []
        )
        .is_err()
    );
    assert!(
        c.execute(
            "UPDATE model_defaults SET config_id='missing' WHERE usage='speech'",
            []
        )
        .is_err()
    );
    let backup = std::fs::read_dir(root.path().join("backups"))
        .unwrap()
        .map(|f| f.unwrap().path())
        .find(|p| {
            Connection::open(p)
                .unwrap()
                .query_row("SELECT max(version) FROM schema_migrations", [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap()
                == 4
        })
        .unwrap();
    assert_eq!(
        Connection::open(backup)
            .unwrap()
            .query_row("SELECT count(*) FROM model_configs", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
fn attempt(record: &str, kind: &str) -> Attempt {
    Attempt {
        id: id(),
        record_id: record.into(),
        kind: kind.into(),
        config_id: id(),
        config_name: "测试配置".into(),
        config_revision: 1,
        model_id: "fixture".into(),
        service_host: "example.invalid".into(),
        created_at: crate::data::repository::now(),
        ended_at: None,
        status: "preparing".into(),
        text: String::new(),
        error_code: None,
        elapsed_ms: 0,
    }
}
fn prepare(repo: &mut Repository, audio: bool) -> (String, Attempt, Option<Audio>) {
    let record = id();
    let a = attempt(&record, "live");
    let resource = audio.then(|| Audio {
        id: id(),
        record_id: record.clone(),
        file_key: id(),
        status: "prepared".into(),
        bytes: 0,
        expires_at: Some(1),
    });
    repo.history_operation(DbOperation::Prepare(Prepare {
        record_id: record.clone(),
        attempt: a.clone(),
        audio: resource.clone(),
    }))
    .unwrap();
    (record, a, resource)
}
fn finish(repo: &mut Repository, a: &Attempt, status: &str, text: &str) {
    repo.history_operation(DbOperation::Finish {
        attempt_id: a.id.clone(),
        status: status.into(),
        text: text.into(),
        error: None,
        duration_ms: 1000,
        elapsed_ms: 20,
    })
    .unwrap();
}
fn record(repo: &mut Repository, id: &str) -> Record {
    match repo
        .history_operation(DbOperation::Detail(id.into()))
        .unwrap()
    {
        DbOutput::Record(r) => r,
        _ => panic!(),
    }
}
fn current(r: &Record) -> &str {
    &r.attempts
        .iter()
        .find(|a| Some(&a.id) == r.current_attempt.as_ref())
        .unwrap()
        .text
}

#[test]
fn retry_versions_preserve_success_and_do_not_double_count() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let (r, a, _) = prepare(&mut repo, false);
    finish(&mut repo, &a, "success", "原结果");
    for state in ["failed", "cancelled"] {
        let b = attempt(&r, "retry");
        repo.history_operation(DbOperation::Retry(b.clone()))
            .unwrap();
        finish(&mut repo, &b, state, "暂定文字");
        assert_eq!(current(&record(&mut repo, &r)), "原结果");
    }
    let b = attempt(&r, "retry");
    repo.history_operation(DbOperation::Retry(b.clone()))
        .unwrap();
    finish(&mut repo, &b, "success", "新结果");
    assert_eq!(current(&record(&mut repo, &r)), "新结果");
    assert_eq!(record(&mut repo, &r).attempts.len(), 4);
    assert_eq!(
        repo.history_operation(DbOperation::Finish {
            attempt_id: b.id,
            status: "success".into(),
            text: "迟到".into(),
            error: None,
            duration_ms: 1,
            elapsed_ms: 1
        })
        .unwrap_err()
        .code,
        "conflict"
    );
    match repo.history_operation(DbOperation::Usage).unwrap() {
        DbOutput::Usage(u) => {
            assert_eq!(u.count, 1);
            assert_eq!(u.duration_ms, 1000);
            assert_eq!(u.characters, 3);
        }
        _ => panic!(),
    }
    drop(repo);
    assert_eq!(
        current(&record(&mut Repository::open(root.path()).unwrap(), &r)),
        "新结果"
    );
}
#[test]
fn history_literal_search_pagination_and_pending_delete_are_recoverable() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    for n in 0..55 {
        let (_, a, _) = prepare(&mut repo, false);
        finish(&mut repo, &a, "success", &format!("文字{n}"));
    }
    let (r, a, _) = prepare(&mut repo, false);
    finish(&mut repo, &a, "success", "100%_正确");
    match repo
        .history_operation(DbOperation::List(Query {
            search: "%_".into(),
            page: 99,
        }))
        .unwrap()
    {
        DbOutput::Page(p) => {
            assert_eq!(p.total, 1);
            assert_eq!(p.page, 1);
        }
        _ => panic!(),
    }
    match repo
        .history_operation(DbOperation::List(Query {
            search: "".into(),
            page: 99,
        }))
        .unwrap()
    {
        DbOutput::Page(p) => {
            assert_eq!(p.total, 56);
            assert_eq!(p.page, 2);
            assert_eq!(p.records.len(), 6);
        }
        _ => panic!(),
    }
    let revision = record(&mut repo, &r).revision;
    assert_eq!(
        repo.history_operation(DbOperation::MarkDelete {
            id: r.clone(),
            revision: revision - 1
        })
        .unwrap_err()
        .code,
        "conflict"
    );
    repo.history_operation(DbOperation::MarkDelete {
        id: r.clone(),
        revision,
    })
    .unwrap();
    assert_eq!(record(&mut repo, &r).status, "delete_pending");
    repo.history_operation(DbOperation::Interrupted).unwrap();
    assert_eq!(
        repo.history_operation(DbOperation::Detail(r))
            .unwrap_err()
            .code,
        "notFound"
    );
}
#[test]
fn migration_v2_preserves_ids_defaults_keys_and_dictionary() {
    use crate::data::repository::{MIGRATIONS, configure, migrate};
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("dictionary.sqlite");
    let mut c = Connection::open(&file).unwrap();
    configure(&c).unwrap();
    migrate(&mut c, root.path(), &MIGRATIONS[..2]).unwrap();
    c.execute(
        "INSERT INTO dictionary_terms VALUES('term','Figma','figma',NULL,NULL,1,1,1)",
        [],
    )
    .unwrap();
    c.execute("INSERT INTO model_configs VALUES('old','原配置','openai-chat','https://example.invalid/v1','m',1,1,'ref',3,1,1)",[]).unwrap();
    c.execute(
        "UPDATE model_defaults SET config_id='old',revision=7 WHERE usage='speech'",
        [],
    )
    .unwrap();
    migrate(&mut c, root.path(), MIGRATIONS).unwrap();
    assert_eq!(
        c.query_row(
            "SELECT credential_ref FROM model_configs WHERE id='old'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "ref"
    );
    assert_eq!(
        c.query_row(
            "SELECT revision FROM model_defaults WHERE usage='speech'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        7
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| r
            .get::<_, i64>(
            0
        ))
        .unwrap(),
        0
    );
    assert!(c.execute("INSERT INTO model_configs VALUES('invalid','不合法','qwen-audio-streaming','https://example.invalid','m',1,1,'r',1,1,1)",[]).is_err());
    assert!(
        c.execute(
            "UPDATE model_configs SET speech_enabled=0 WHERE id='old'",
            []
        )
        .is_err()
    );
    assert_eq!(
        c.query_row("SELECT term FROM dictionary_terms", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "Figma"
    );
    assert!(root.path().join("backups").exists());
}
#[test]
fn wav_finalize_and_crash_recovery_keep_samples_and_reject_paths() {
    let root = tempfile::tempdir().unwrap();
    let files = files::AudioFiles::new(root.path()).unwrap();
    assert_eq!(
        files.path("../escape", false).unwrap_err().code,
        "forbidden"
    );
    let key = id();
    let mut writer = files.writer(&key).unwrap();
    for n in 0..1600 {
        writer.write_sample((n % 120) as i16).unwrap();
    }
    writer.finalize().unwrap();
    // Simulate a crash with audio present but stale RIFF/data lengths.
    let part = files.path(&key, true).unwrap();
    let mut bytes = std::fs::read(&part).unwrap();
    bytes[4..8].fill(0);
    bytes[40..44].fill(0);
    bytes.push(0);
    std::fs::write(&part, bytes).unwrap();
    assert_eq!(files.recover(&key).unwrap(), 3200);
    assert_eq!(files.read(&key).unwrap().len(), 1600);
    assert!(!part.exists());
    assert_eq!(files.recover(&key).unwrap(), 3200);
    files.remove(&key).unwrap();
    files.remove(&key).unwrap();
    let invalid = id();
    std::fs::write(files.path(&invalid, true).unwrap(), b"bad").unwrap();
    assert!(files.recover(&invalid).is_err());
}
#[tokio::test]
async fn retention_and_restart_recovery_keep_text_and_skip_active_audio() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let (r, a, resource) = prepare(&mut repo, true);
    finish(&mut repo, &a, "success", "永久文字");
    let resource = resource.unwrap();
    drop(repo);
    let worker = DataWorker::start(root.path().to_path_buf()).unwrap();
    let service = HistoryService::new(worker, root.path());
    let files = service.files.clone().unwrap();
    let mut w = files.writer(&resource.file_key).unwrap();
    for _ in 0..1600 {
        w.write_sample(10i16).unwrap();
    }
    w.finalize().unwrap();
    files.complete(&resource.file_key).unwrap();
    service
        .db(DbOperation::AudioState {
            id: resource.id.clone(),
            status: "ready".into(),
            bytes: 3200,
        })
        .await
        .unwrap();
    let lease = service.lease(&resource.id).await.unwrap();
    service.cleanup().await.unwrap();
    assert_eq!(
        service.detail(&r).await.unwrap().audio.unwrap().status,
        "ready"
    );
    assert_eq!(service.remove(&r, 2).await.unwrap_err().code, "busy");
    drop(lease);
    // Persist deletion intent before simulating a process restart.
    service
        .db(DbOperation::AudioState {
            id: resource.id,
            status: "expire_pending".into(),
            bytes: 3200,
        })
        .await
        .unwrap();
    service.recover().await.unwrap();
    let record = service.detail(&r).await.unwrap();
    assert_eq!(record.audio.unwrap().status, "expired");
    assert_eq!(current(&service.detail(&r).await.unwrap()), "永久文字");
    assert!(files.read(&resource.file_key).is_err());
}
#[test]
fn interrupted_attempt_and_query_indexes() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let (r, a, _) = prepare(&mut repo, false);
    repo.history_operation(DbOperation::Checkpoint {
        attempt_id: a.id,
        text: "中断前文字".into(),
    })
    .unwrap();
    repo.history_operation(DbOperation::Interrupted).unwrap();
    let r = record(&mut repo, &r);
    assert_eq!(r.status, "interrupted");
    assert_eq!(current(&r), "中断前文字");
    for sql in [
        "EXPLAIN QUERY PLAN SELECT id FROM speech_records ORDER BY created_at DESC,id ASC LIMIT 50",
        "EXPLAIN QUERY PLAN SELECT id FROM audio_assets WHERE status='ready' AND expires_at<=1",
    ] {
        let details = repo
            .connection
            .prepare(sql)
            .unwrap()
            .query_map([], |r| r.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join(" ");
        assert!(details.contains("INDEX"), "{details}");
        assert!(!details.contains("TEMP B-TREE"));
    }
    let mut p = match repo.history_operation(DbOperation::Preferences).unwrap() {
        DbOutput::Preferences(p) => p,
        _ => panic!(),
    };
    assert_eq!(p.retention_days, Some(30));
    p.retention_days = Some(7);
    repo.history_operation(DbOperation::SavePreferences(p.clone()))
        .unwrap();
    assert_eq!(
        repo.history_operation(DbOperation::SavePreferences(p))
            .unwrap_err()
            .code,
        "conflict"
    );
    assert!(
        repo.connection
            .execute("UPDATE speech_preferences SET retention_days=8", params![])
            .is_err()
    );
}

#[tokio::test]
async fn missing_corrupt_and_failed_deletion_are_visible_and_recoverable() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let (record_id, a, audio) = prepare(&mut repo, true);
    finish(&mut repo, &a, "success", "保留文字");
    let audio = audio.unwrap();
    repo.history_operation(DbOperation::AudioState {
        id: audio.id.clone(),
        status: "ready".into(),
        bytes: 3200,
    })
    .unwrap();
    drop(repo);
    let service = HistoryService::new(
        DataWorker::start(root.path().to_path_buf()).unwrap(),
        root.path(),
    );
    let files = service.files.clone().unwrap();
    assert!(service.validate_asset(&audio).await.is_err());
    assert_eq!(service.asset(&audio.id).await.unwrap().status, "missing");
    let path = files.path(&audio.file_key, false).unwrap();
    std::fs::write(&path, b"damaged").unwrap();
    assert!(service.validate_asset(&audio).await.is_err());
    assert_eq!(service.asset(&audio.id).await.unwrap().status, "corrupt");
    // A directory cannot be deleted as an owned recording. Keep the tombstone until repaired.
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(service.remove(&record_id, 2).await.is_err());
    assert_eq!(
        service.detail(&record_id).await.unwrap().status,
        "delete_pending"
    );
    service.recover().await.unwrap();
    assert_eq!(
        service.detail(&record_id).await.unwrap().status,
        "delete_pending"
    );
    std::fs::remove_dir(&path).unwrap();
    service.recover().await.unwrap();
    assert_eq!(
        service.detail(&record_id).await.unwrap_err().code,
        "notFound"
    );
    let held = service.reserve(&audio.id).unwrap();
    assert_eq!(service.reserve(&audio.id).err().unwrap().code, "busy");
    drop(held);
    drop(service.reserve(&audio.id).unwrap());
}

#[tokio::test]
async fn recovery_finalizes_only_registered_parts_and_preserves_checkpoint() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let (r, a, resource) = prepare(&mut repo, true);
    repo.history_operation(DbOperation::Checkpoint {
        attempt_id: a.id,
        text: "已确认句段".into(),
    })
    .unwrap();
    drop(repo);
    let service = HistoryService::new(
        DataWorker::start(root.path().to_path_buf()).unwrap(),
        root.path(),
    );
    let files = service.files.clone().unwrap();
    let resource = resource.unwrap();
    // The active capture flushes but has not renamed this registered file.
    let mut w = files.writer(&resource.file_key).unwrap();
    for _ in 0..1600 {
        w.write_sample(100i16).unwrap();
    }
    w.finalize().unwrap();
    let unknown = id();
    std::fs::write(files.path(&unknown, true).unwrap(), b"unregistered").unwrap();
    service.recover().await.unwrap();
    let recovered = service.detail(&r).await.unwrap();
    assert_eq!(recovered.status, "interrupted");
    assert_eq!(current(&recovered), "已确认句段");
    // Prepared expiry is 1ms in this fixture, so recovery also performs retention cleanup.
    assert_eq!(recovered.audio.unwrap().status, "expired");
    assert!(files.path(&unknown, true).unwrap().exists());
}

#[test]
#[ignore = "Explicit isolated history query benchmark; reports machine-dependent p95"]
fn history_query_benchmark() {
    use std::time::Instant;
    for size in [1000usize, 10000, 100000] {
        let root = tempfile::tempdir().unwrap();
        let mut repo = Repository::open(root.path()).unwrap();
        let tx = repo.connection.transaction().unwrap();
        {
            let mut records = tx.prepare("INSERT INTO speech_records(id,created_at,status,current_attempt,ended_at,duration_ms) VALUES(?1,?2,'success',NULL,?2,1000)").unwrap();
            let mut attempts = tx.prepare("INSERT INTO speech_attempts(id,record_id,kind,config_id,config_name,model_id,service_host,config_revision,created_at,status,text,search_key) VALUES(?1,?2,'live','fixture','fixture','fixture','example.invalid',1,?3,'success',?4,?4)").unwrap();
            let mut current = tx
                .prepare("UPDATE speech_records SET current_attempt=?2 WHERE id=?1")
                .unwrap();
            for n in 0..size {
                let r = format!("record-{n:08}");
                let a = format!("attempt-{n:08}");
                records.execute(params![r, n as i64]).unwrap();
                attempts
                    .execute(params![a, r, n as i64, format!("中文测试 english {n}")])
                    .unwrap();
                current.execute(params![r, a]).unwrap();
            }
        }
        tx.commit().unwrap();
        for (label, search, page) in [
            ("recent", "", 1),
            ("last", "", u32::MAX),
            ("chinese", "中文", 1),
            ("english", "english", 1),
            ("empty", "missing", 1),
        ] {
            let mut samples = vec![];
            for _ in 0..30 {
                let started = Instant::now();
                repo.history_operation(DbOperation::List(Query {
                    search: search.into(),
                    page,
                }))
                .unwrap();
                samples.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            samples.sort_by(f64::total_cmp);
            let p95 = samples[28];
            println!("history size={size} query={label} p95_ms={p95:.3}");
            if size == 10000 {
                assert!(p95 < if search.is_empty() { 50.0 } else { 100.0 });
            }
        }
    }
}
