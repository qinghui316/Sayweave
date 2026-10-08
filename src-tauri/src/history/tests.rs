use super::*;
use crate::data::repository::Repository;
use rusqlite::{Connection, params};

fn id() -> String {
    uuid::Uuid::new_v4().to_string()
}
#[test]
fn cancelled_partial_attempt_retains_text_without_poisoning_record_state() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let (record_id, attempt, _) = prepare(&mut repo, false);
    repo.history_operation(DbOperation::Finish {
        attempt_id: attempt.id.clone(),
        status: "cancelled".into(),
        text: "已提交前段".into(),
        error: Some("cancelled".into()),
        duration_ms: 0,
        elapsed_ms: 20,
    })
    .unwrap();
    let current = record(&mut repo, &record_id);
    assert_eq!(current.current_attempt, Some(attempt.id));
    assert_eq!(current.status, "interrupted");
    assert_eq!(current.attempts[0].status, "cancelled");
    assert_eq!(current.attempts[0].text, "已提交前段");
}
#[test]
fn processed_history_reference_search_and_source_versions_remain_independent() {
    use crate::contracts::processing::{DbOperation as P, RunMetadata, UnitRecord};
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let (rid, attempt, _) = prepare(&mut repo, false);
    finish(&mut repo, &attempt, "success", "呃，赛维原文。");
    let run = RunMetadata {
        id: id(),
        attempt_id: attempt.id.clone(),
        task_id: id(),
        config_id: id(),
        config_name: "文字服务".into(),
        model_id: "fixture".into(),
        config_revision: 1,
        prompt_version: "faithful-v1".into(),
        settings_revision: 1,
        dictionary_version: "snapshot".into(),
    };
    repo.processing_operation(P::Prepare(run.clone())).unwrap();
    let unit = UnitRecord {
        id: id(),
        run_id: run.id.clone(),
        turn: 1,
        source_start: 0,
        source_end: 8,
        source_refs: "[]".into(),
        source_text: "呃，赛维原文。".into(),
        candidate_text: Some("Sayweave 整理结果。".into()),
        generation_state: "success".into(),
        application_state: "applied".into(),
        source_committed_end: Some(8),
        error_code: None,
        prompt_tokens: Some(10),
        completion_tokens: Some(5),
        elapsed_ms: 20,
    };
    repo.processing_operation(P::SaveUnit(unit.clone()))
        .unwrap();
    // A new content revision at the same source start has a new request identity;
    // provenance is retained instead of an unconditional source-start uniqueness.
    let mut revised = unit.clone();
    revised.id = id();
    revised.source_text = "修订来源".into();
    revised.source_refs = "[{\"revision\":2}]".into();
    revised.application_state = "notApplied".into();
    repo.processing_operation(P::SaveUnit(revised)).unwrap();
    repo.processing_operation(P::Finish {
        id: run.id.clone(),
        text: "Sayweave 整理结果。".into(),
        state: "success".into(),
        error: None,
    })
    .unwrap();
    let summary = browse(&mut repo, "", None).records.remove(0);
    assert_eq!(summary.preview, "Sayweave 整理结果。");
    assert_eq!(
        summary.display_reference.as_ref().unwrap().kind,
        "processed"
    );
    assert_eq!(summary.display_reference.as_ref().unwrap().id, run.id);
    let raw = browse(&mut repo, "赛维", None).records.remove(0);
    assert_eq!(raw.display_reference.as_ref().unwrap().kind, "raw");
    assert_eq!(raw.preview, "呃，赛维原文。");
    assert_eq!(browse(&mut repo, "整理结果", None).total, 1);
    let complete = record(&mut repo, &rid);
    assert_eq!(complete.attempts[0].text, "呃，赛维原文。");
    assert_eq!(complete.processing[0].text, "Sayweave 整理结果。");
    assert_eq!(complete.processing[0].units.len(), 2);
    let runs = record(&mut repo, &rid).processing;
    assert_eq!(runs[0].units.len(), 2);
    assert!(
        repo.processing_operation(P::SaveUnit(unit)).is_err(),
        "late write must not mutate settled run"
    );
}

#[test]
fn cancelled_processing_atomically_drops_pending_tail_and_queues_audio_cleanup() {
    use crate::contracts::processing::{DbOperation as P, RunMetadata, UnitRecord};
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let (rid, attempt, resource) = prepare(&mut repo, true);
    finish(&mut repo, &attempt, "success", "前段。未提交尾部。");
    let run = RunMetadata {
        id: id(),
        attempt_id: attempt.id.clone(),
        task_id: id(),
        config_id: id(),
        config_name: "fixture".into(),
        model_id: "fixture".into(),
        config_revision: 1,
        prompt_version: "faithful-v1".into(),
        settings_revision: 1,
        dictionary_version: "snapshot".into(),
    };
    repo.processing_operation(P::Prepare(run.clone())).unwrap();
    let kept = UnitRecord {
        id: id(),
        run_id: run.id.clone(),
        turn: 1,
        source_start: 0,
        source_end: 3,
        source_refs: "[]".into(),
        source_text: "前段。".into(),
        candidate_text: Some("前段。".into()),
        generation_state: "success".into(),
        application_state: "applied".into(),
        source_committed_end: Some(3),
        error_code: None,
        prompt_tokens: Some(12),
        completion_tokens: Some(3),
        elapsed_ms: 5,
    };
    repo.processing_operation(P::SaveUnit(kept.clone()))
        .unwrap();
    let mut pending = kept.clone();
    pending.id = id();
    pending.source_start = 3;
    pending.source_end = 9;
    pending.source_text = "未提交尾部。".into();
    pending.candidate_text = Some("未提交候选。".into());
    pending.application_state = "pending".into();
    pending.source_committed_end = None;
    repo.processing_operation(P::SaveUnit(pending.clone()))
        .unwrap();
    repo.processing_operation(P::Cancel {
        id: run.id.clone(),
        text: "前段。".into(),
        raw: "前段。".into(),
        rollback_failed: false,
        units: vec![kept],
    })
    .unwrap();
    let complete = record(&mut repo, &rid);
    assert_eq!(complete.attempts[0].text, "前段。");
    assert_eq!(complete.processing[0].state, "cancelled");
    assert_eq!(complete.processing[0].units.len(), 1);
    assert_eq!(complete.processing[0].units[0].prompt_tokens, Some(12));
    assert_eq!(browse(&mut repo, "未提交", None).total, 0);
    assert!(repo.processing_operation(P::SaveUnit(pending)).is_err());
    let runs = record(&mut repo, &rid).processing;
    assert_eq!(runs[0].units.len(), 1);
    let queued: i64 = repo
        .connection
        .query_row(
            "SELECT count(*) FROM discarded_audio WHERE id=?1",
            [resource.unwrap().id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(queued, 1);
}

fn browse(repo: &mut Repository, search: &str, cursor: Option<Cursor>) -> Feed {
    match repo
        .history_operation(DbOperation::Browse(BrowseQuery {
            search: search.into(),
            cursor,
            ..Default::default()
        }))
        .unwrap()
    {
        DbOutput::Feed(page) => page,
        _ => panic!("wrong output"),
    }
}

#[test]
fn summaries_bind_full_versions_and_keep_success_after_failed_retry() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let (rid, first, resource) = prepare(&mut repo, true);
    let text = "旧版本😀".repeat(100);
    finish(&mut repo, &first, "success", &text);
    let mut second = attempt(&rid, "retry");
    second.created_at = first.created_at + 1;
    repo.history_operation(DbOperation::Retry(second.clone()))
        .unwrap();
    finish(&mut repo, &second, "success", "新的完整文字");
    let revision = record(&mut repo, &rid).revision;
    let mut failed = attempt(&rid, "retry");
    failed.created_at = second.created_at + 1;
    repo.history_operation(DbOperation::Retry(failed.clone()))
        .unwrap();
    repo.history_operation(DbOperation::Finish {
        attempt_id: failed.id.clone(),
        status: "failed".into(),
        text: "".into(),
        error: Some("timeout".into()),
        duration_ms: 0,
        elapsed_ms: 30,
    })
    .unwrap();
    let current = browse(&mut repo, "", None).records.remove(0);
    assert_eq!(current.revision, revision);
    assert_eq!(current.display_attempt, Some(second.id));
    assert_eq!(current.preview, "新的完整文字");
    assert_eq!(current.latest_attempt, Some(failed.id));
    assert_eq!(current.latest_error.as_deref(), Some("timeout"));
    let matched = browse(&mut repo, "旧版本", None).records.remove(0);
    assert!(matched.matched_old_version);
    assert!(matched.truncated);
    assert_eq!(matched.preview.chars().count(), 240);
    assert_eq!(matched.display_attempt, Some(first.id.clone()));
    assert_eq!(
        record(&mut repo, &rid)
            .attempts
            .iter()
            .find(|a| a.id == first.id)
            .unwrap()
            .text,
        text
    );
    let a = resource.unwrap();
    repo.history_operation(DbOperation::AudioState {
        id: a.id,
        status: "expired".into(),
        bytes: 0,
    })
    .unwrap();
    assert_eq!(
        browse(&mut repo, "", None).records[0]
            .audio
            .as_ref()
            .unwrap()
            .status,
        "expired"
    );
    assert_eq!(browse(&mut repo, "%_", None).total, 0);
    assert!(
        repo.history_operation(DbOperation::Browse(BrowseQuery {
            search: "\u{0}".into(),
            cursor: None,
            ..Default::default()
        }))
        .is_err()
    );
}

#[test]
fn cursor_survives_deleted_anchor_without_duplicate_or_missing_rows() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    for i in 0..115 {
        let rid = id();
        let mut a = attempt(&rid, "live");
        a.created_at = 1000 + i / 3;
        repo.history_operation(DbOperation::Prepare(Prepare {
            record_id: rid,
            attempt: a.clone(),
            audio: None,
        }))
        .unwrap();
        finish(&mut repo, &a, "success", "你好");
    }
    let page = browse(&mut repo, "你好", None);
    assert_eq!(page.total, 115);
    assert_eq!(page.records.len(), 50);
    let cursor = page.next_cursor.clone().unwrap();
    let deleted = record(&mut repo, &cursor.id);
    repo.history_operation(DbOperation::MarkDelete {
        id: cursor.id.clone(),
        revision: deleted.revision,
    })
    .unwrap();
    repo.history_operation(DbOperation::DeleteRecord(cursor.id.clone()))
        .unwrap();
    let expected = match repo
        .history_operation(DbOperation::List(Query {
            search: "你好".into(),
            page: 2,
        }))
        .unwrap()
    {
        DbOutput::Page(p) => p.total,
        _ => 0,
    };
    assert_eq!(expected, 114);
    let mut ids: Vec<_> = page.records.into_iter().map(|r| r.id).collect();
    let next = browse(&mut repo, "你好", Some(cursor));
    assert_eq!(next.records.len(), 50);
    ids.extend(next.records.into_iter().map(|r| r.id));
    let last = browse(&mut repo, "你好", next.next_cursor);
    assert!(last.next_cursor.is_none());
    ids.extend(last.records.into_iter().map(|r| r.id));
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), 115);
    assert_eq!(ids.len(), 115);
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
                let r = uuid::Uuid::from_u128(n as u128 + 1).to_string();
                let a = uuid::Uuid::from_u128(n as u128 + size as u128 + 1).to_string();
                records.execute(params![r, n as i64]).unwrap();
                attempts
                    .execute(params![a, r, n as i64, format!("中文测试 english {n}")])
                    .unwrap();
                current.execute(params![r, a]).unwrap();
            }
        }
        tx.commit().unwrap();
        let plans=repo.connection.prepare("EXPLAIN QUERY PLAN SELECT id FROM speech_records ORDER BY created_at DESC,id ASC LIMIT 50").unwrap().query_map([],|row|row.get::<_,String>(3)).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
        assert!(plans.iter().any(|p| p.contains("speech_records_recent")));
        println!("history size={size} order_plan={plans:?}");
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
            let mut summaries = vec![];
            for _ in 0..30 {
                let started = Instant::now();
                let cursor = if page == u32::MAX {
                    Some(Cursor {
                        created_at: 50,
                        id: uuid::Uuid::from_u128(51).to_string(),
                    })
                } else {
                    None
                };
                let _ = browse(&mut repo, search, cursor);
                summaries.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            summaries.sort_by(f64::total_cmp);
            let p95 = summaries[28];
            println!("history_browse size={size} query={label} p95_ms={p95:.3}");
            if size == 10000 {
                assert!(p95 < if search.is_empty() { 50.0 } else { 100.0 });
            }
        }
    }
}

#[test]
#[ignore = "Populate only a UUID-scoped desktop acceptance directory while its app is stopped"]
fn history_desktop_fixture() {
    let token = std::env::var("SAYWEAVE_DATA_ACCEPTANCE_ID").unwrap();
    uuid::Uuid::parse_str(&token).unwrap();
    let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap();
    let root = project
        .join(".local/data-acceptance")
        .join(token)
        .join("data");
    let mut repo = Repository::open(&root).unwrap();
    assert_eq!(browse(&mut repo, "", None).total, 0);
    let at = crate::data::repository::now();
    let tx = repo.connection.transaction().unwrap();
    {
        let mut records=tx.prepare("INSERT INTO speech_records(id,created_at,status,current_attempt,duration_ms,ended_at) VALUES(?1,?2,'success',NULL,1000,?2)").unwrap();
        let mut attempts=tx.prepare("INSERT INTO speech_attempts(id,record_id,kind,config_id,config_name,model_id,service_host,config_revision,created_at,status,text,search_key) VALUES(?1,?2,'live','fixture','模型','fixture','example.invalid',1,?3,'success',?4,?4)").unwrap();
        let mut current = tx
            .prepare("UPDATE speech_records SET current_attempt=?2 WHERE id=?1")
            .unwrap();
        for n in 0..10000 {
            let r = id();
            let a = id();
            let created = at - n * 60000;
            records.execute(params![r, created]).unwrap();
            let text = if n == 0 {
                "长中文识别文字，支持完整复制与展开。".repeat(50)
            } else {
                format!("历史验收 {n}：中文 english 文字。")
            };
            attempts.execute(params![a, r, created, text]).unwrap();
            current.execute(params![r, a]).unwrap();
        }
    }
    tx.commit().unwrap();
    assert_eq!(browse(&mut repo, "", None).total, 10000);
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
    repo.history_operation(write("t", "a", "pending")).unwrap();
    repo.history_operation(write("t", "a", "delivering"))
        .unwrap();
    repo.history_operation(write("t", "a", "confirmed"))
        .unwrap();
    assert_eq!(
        repo.history_operation(write("t", "a", "pending"))
            .unwrap_err()
            .code,
        "conflict"
    );
    assert!(
        repo.history_operation(write("t", "other", "confirmed"))
            .is_err()
    );
    repo.history_operation(write("incomplete", "a", "submitted"))
        .unwrap();
    repo.history_operation(DbOperation::RecoverDelivery)
        .unwrap();
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
        .history_operation(DbOperation::Deliveries("r".into()))
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

#[test]
fn bounded_reading_pages_reread_only_requested_summaries_and_share_search_rules() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    for _ in 0..21 {
        let (_, a, _) = prepare(&mut repo, false);
        finish(&mut repo, &a, "success", "目标文字");
    }
    let query =
        |repo: &mut Repository, q| match repo.history_operation(DbOperation::Browse(q)).unwrap() {
            DbOutput::Feed(page) => page,
            _ => panic!("feed"),
        };
    let page = query(
        &mut repo,
        BrowseQuery {
            page_size: Some(20),
            ..Default::default()
        },
    );
    assert_eq!(page.records.len(), 20);
    assert_eq!(page.total, 21);
    assert!(page.next_cursor.is_some());
    let ids: Vec<_> = page.records.iter().map(|r| r.id.clone()).collect();
    let original = record(&mut repo, &ids[0]);
    repo.history_operation(DbOperation::MarkDelete {
        id: ids[0].clone(),
        revision: original.revision,
    })
    .unwrap();
    repo.history_operation(DbOperation::DeleteRecord(ids[0].clone()))
        .unwrap();
    let reread = query(
        &mut repo,
        BrowseQuery {
            ids: Some(ids.clone()),
            search: "目标".into(),
            ..Default::default()
        },
    );
    assert_eq!(reread.records.len(), 19);
    assert_eq!(reread.total, 20);
    assert!(reread.next_cursor.is_none());
    assert!(
        reread
            .records
            .iter()
            .all(|r| ids.contains(&r.id) && r.preview == "目标文字")
    );
    assert_eq!(
        query(
            &mut repo,
            BrowseQuery {
                ids: Some(ids.clone()),
                search: "不匹配".into(),
                ..Default::default()
            }
        )
        .records
        .len(),
        0
    );
    for q in [
        BrowseQuery {
            page_size: Some(0),
            ..Default::default()
        },
        BrowseQuery {
            page_size: Some(51),
            ..Default::default()
        },
        BrowseQuery {
            ids: Some(vec![id(); 21]),
            ..Default::default()
        },
        BrowseQuery {
            ids: Some(vec![ids[1].clone(); 2]),
            ..Default::default()
        },
    ] {
        assert!(repo.history_operation(DbOperation::Browse(q)).is_err());
    }
}
