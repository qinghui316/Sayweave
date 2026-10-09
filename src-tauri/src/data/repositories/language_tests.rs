use super::*;
use crate::contracts::dictionary::{Input, Kind, Query, Source};
fn analysis() -> AnalysisStamp {
    AnalysisStamp {
        config_id: "test".into(),
        config_revision: 1,
        model_id: "fixture".into(),
        service_host: "localhost".into(),
        template_version: "fixture-v1".into(),
        strategy_version: "application-learning-v1".into(),
    }
}
fn evidence(app: &str, before: &str, after: &str) -> EditEvidence {
    EditEvidence {
        id: uuid::Uuid::new_v4().to_string(),
        task: uuid::Uuid::new_v4().to_string(),
        turn: 1,
        operation: 2,
        sequence: 1,
        region: "tail".into(),
        expires_at: now() + 600000,
        application: ApplicationContext::from_identity(None, app),
        settings_revision: 1,
        raw: before.into(),
        before: before.into(),
        after: after.into(),
    }
}
fn settle(repo: &mut Repository, e: &EditEvidence, class: EditClass, term: Option<&str>) {
    repo.language_operation(DbOperation::SaveEdit(e.clone()))
        .unwrap();
    repo.language_operation(DbOperation::ClassifyEdit {
        analysis: analysis(),
        id: e.id.clone(),
        settings_revision: e.settings_revision,
        decision: LexicalDecision {
            class,
            original: term.map(|_| e.before.clone()),
            term: term.map(str::to_owned),
            summary: Some("保留完整理由和步骤".into()),
        },
    })
    .unwrap();
}
fn terms(repo: &mut Repository) -> Vec<crate::contracts::dictionary::Record> {
    repo.list(Query {
        kind: Kind::Term,
        search: "".into(),
        page: 1,
        source: None,
    })
    .unwrap()
    .records
}
#[test]
fn clear_correction_learns_once_ambiguity_does_not_and_deleted_words_stay_deleted() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let e = evidence("Codex.exe", "赛维", "Sayweave");
    settle(&mut repo, &e, EditClass::Lexical, Some("Sayweave"));
    let entries = terms(&mut repo);
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].source, Source::Automatic);
    assert!(
        repo.language_operation(DbOperation::ClassifyEdit {
            analysis: analysis(),
            id: e.id.clone(),
            settings_revision: 1,
            decision: LexicalDecision {
                class: EditClass::Lexical,
                original: Some("赛维".into()),
                term: Some("Sayweave".into()),
                summary: None
            }
        })
        .is_err()
    );
    repo.delete(Kind::Term, &entries[0].id, entries[0].revision)
        .unwrap();
    let late = evidence("Codex.exe", "赛维", "Sayweave");
    settle(&mut repo, &late, EditClass::Lexical, Some("Sayweave"));
    assert!(terms(&mut repo).is_empty());
    let ambiguous = evidence("Codex.exe", "奇览", "绮蓝引擎");
    settle(
        &mut repo,
        &ambiguous,
        EditClass::Ambiguous,
        Some("绮蓝引擎"),
    );
    assert!(terms(&mut repo).is_empty());
    repo.save(
        Input {
            kind: Kind::Term,
            original: "Sayweave".into(),
            replacement: None,
            category: None,
        },
        None,
    )
    .unwrap();
    assert_eq!(terms(&mut repo)[0].source, Source::Manual);
}
#[test]
fn scope_evidence_and_immutable_versions_prevent_cross_application_and_stale_publication() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let mut ids = vec![];
    let mut app = None;
    for _ in 0..3 {
        let e = evidence("Codex.exe", "请修改", "请修改，并保留理由");
        settle(&mut repo, &e, EditClass::Style, None);
        ids.push(e.id);
        app = Some(e.application);
    }
    let app = app.unwrap();
    let DbOutput::Batch(Some(batch)) = repo
        .language_operation(DbOperation::Batch(app.key.clone()))
        .unwrap()
    else {
        panic!("three independent edits should form a batch");
    };
    let p = ProfileProposal {
        scope: app.key.clone(),
        settings_revision: 1,
        previous_version: 0,
        watermark: batch.watermark,
        evidence: ids.clone(),
        preferences: vec![Preference {
            scope: app.key.clone(),
            rule: "保留完整理由和步骤".into(),
            language: WritingLanguage::Han,
            context: Some("解释技术修改时".into()),
            evidence: ids,
        }],
        candidates: vec![],
        reason: "用户在三个独立口述中主动补充理由".into(),
        analysis: analysis(),
    };
    repo.language_operation(DbOperation::Publish(p.clone()))
        .unwrap();
    assert!(repo.language_operation(DbOperation::Publish(p)).is_err());
    let DbOutput::Snapshot(first) = repo.language_operation(DbOperation::Snapshot(app)).unwrap()
    else {
        panic!()
    };
    assert_eq!(first.profile_version, 1);
    assert_eq!(first.preferences.len(), 1);
    let DbOutput::Snapshot(chat) = repo
        .language_operation(DbOperation::Snapshot(ApplicationContext::from_identity(
            None,
            "Weixin.exe",
        )))
        .unwrap()
    else {
        panic!()
    };
    assert!(chat.preferences.is_empty());
    repo.language_operation(DbOperation::SaveSettings(Settings {
        enabled: false,
        revision: 1,
    }))
    .unwrap();
    let DbOutput::Snapshot(off) = repo
        .language_operation(DbOperation::Snapshot(first.application))
        .unwrap()
    else {
        panic!()
    };
    assert!(!off.settings.enabled);
    assert!(off.preferences.is_empty());
    assert_eq!(first.preferences.len(), 1);
}
#[test]
fn disabling_learning_invalidates_pending_edits_without_affecting_manual_dictionary() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let e = evidence("Codex.exe", "赛维", "Sayweave");
    repo.language_operation(DbOperation::SaveEdit(e.clone()))
        .unwrap();
    repo.language_operation(DbOperation::SaveSettings(Settings {
        enabled: false,
        revision: 1,
    }))
    .unwrap();
    assert!(
        repo.language_operation(DbOperation::ClassifyEdit {
            analysis: analysis(),
            id: e.id,
            settings_revision: 1,
            decision: LexicalDecision {
                class: EditClass::Lexical,
                original: Some("赛维".into()),
                term: Some("Sayweave".into()),
                summary: None
            }
        })
        .is_err()
    );
    repo.save(
        Input {
            kind: Kind::Term,
            original: "SQLite".into(),
            replacement: None,
            category: None,
        },
        None,
    )
    .unwrap();
    assert_eq!(terms(&mut repo).len(), 1);
    let retained: i64 = repo
        .connection
        .query_row(
            "SELECT count(*) FROM language_edits WHERE evidence<>'{}'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(retained, 0);
}
#[test]
fn unchanged_context_is_not_a_lexical_edit_and_capitalization_remains_supported() {
    assert!(changed_terms(
        "请用 SQLit",
        "请用 SQLite",
        "SQLit",
        "SQLite"
    ));
    assert!(changed_terms(
        "用 PostgresSQL 备份",
        "用 PostgreSQL 备份",
        "PostgresSQL",
        "PostgreSQL"
    ));
    assert!(!changed_terms(
        "今天 Sqlite 和 SQLite 两种写法都保留，预算5000元",
        "明天 Sqlite 和 SQLite 两种写法都保留，预算3000元",
        "Sqlite",
        "SQLite"
    ));
    assert!(!changed_terms(
        "SQLite 比 Sqlite 快。今天上线",
        "SQLite 比 Sqlite 快。明天上线",
        "Sqlite",
        "SQLite"
    ));
    assert!(changed_terms(
        "用 Sqlite 备份",
        "用 SQLite 备份",
        "Sqlite",
        "SQLite"
    ));
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let e = evidence(
        "Codex.exe",
        "SQLite 比 Sqlite 快。今天上线",
        "SQLite 比 Sqlite 快。明天上线",
    );
    repo.language_operation(DbOperation::SaveEdit(e.clone()))
        .unwrap();
    assert!(
        repo.language_operation(DbOperation::ClassifyEdit {
            analysis: analysis(),
            id: e.id,
            settings_revision: 1,
            decision: LexicalDecision {
                class: EditClass::Lexical,
                original: Some("Sqlite".into()),
                term: Some("SQLite".into()),
                summary: None
            }
        })
        .is_err()
    );
    assert!(terms(&mut repo).is_empty());
}
#[test]
fn repeated_sessions_do_not_hide_later_evidence_or_a_second_application() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let task = uuid::Uuid::new_v4().to_string();
    let mut app = None;
    for _ in 0..16 {
        let mut e = evidence("Codex.exe", "请修改", "请修改，并保留理由");
        e.task = task.clone();
        settle(&mut repo, &e, EditClass::Style, None);
        app = Some(e.application);
    }
    for _ in 0..3 {
        let e = evidence("Codex.exe", "请修改", "请修改，并保留理由");
        settle(&mut repo, &e, EditClass::Style, None);
    }
    let app = app.unwrap();
    assert!(matches!(
        repo.language_operation(DbOperation::Batch(app.key))
            .unwrap(),
        DbOutput::Batch(Some(_))
    ));
    let other = evidence("Weixin.exe", "请修改", "请修改，并保留理由");
    settle(&mut repo, &other, EditClass::Style, None);
    let DbOutput::Batch(Some(shared)) = repo
        .language_operation(DbOperation::Batch("shared".into()))
        .unwrap()
    else {
        panic!("later app must participate");
    };
    assert!(
        shared
            .observations
            .iter()
            .any(|o| o.application.key == other.application.key)
    );
    let DbOutput::PendingScopes(scopes) =
        repo.language_operation(DbOperation::PendingScopes).unwrap()
    else {
        panic!()
    };
    assert!(scopes.contains(&other.application.key));
}
#[test]
fn ten_full_speech_observations_are_counted_before_prompt_sampling() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let app = ApplicationContext::from_identity(None, "Codex.exe");
    for _ in 0..10 {
        let record = uuid::Uuid::new_v4().to_string();
        let attempt = uuid::Uuid::new_v4().to_string();
        let task = uuid::Uuid::new_v4().to_string();
        repo.connection
            .execute(
                "INSERT INTO speech_records(id,created_at,status) VALUES(?1,?2,'success')",
                params![record, now()],
            )
            .unwrap();
        repo.connection.execute("INSERT INTO speech_attempts(id,record_id,kind,config_id,config_name,model_id,service_host,config_revision,created_at,status,text) VALUES(?1,?2,'live','test','test','test','test',1,?3,'success',?4)",params![attempt,record,now(),"文".repeat(160)]).unwrap();
        repo.connection.execute("INSERT INTO delivery_attempts(task_id,attempt_id,state,created_at,updated_at) VALUES(?1,?2,'submitted',?3,?3)",params![task,attempt,now()]).unwrap();
        repo.language_operation(DbOperation::Observe(StyleObservation {
            id: format!("speech:{task}"),
            task,
            application: app.clone(),
            settings_revision: 1,
            language: WritingLanguage::Han,
            kind: "speech".into(),
            summary: String::new(),
            source_ref: Some(attempt),
        }))
        .unwrap();
    }
    let DbOutput::Batch(Some(batch)) = repo
        .language_operation(DbOperation::Batch(app.key))
        .unwrap()
    else {
        panic!("ten speech records must dispatch");
    };
    assert_eq!(batch.observations.len(), 10);
}
#[test]
fn each_shared_rule_needs_its_own_cross_app_evidence_and_cooldown_survives_reopen() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let mut first = Vec::new();
    let mut second = Vec::new();
    for (app, ids) in [("Codex.exe", &mut first), ("Weixin.exe", &mut second)] {
        for _ in 0..3 {
            let e = evidence(app, "请修改", "请修改，并保留理由");
            settle(&mut repo, &e, EditClass::Style, None);
            ids.push(e.id);
        }
    }
    let DbOutput::Batch(Some(batch)) = repo
        .language_operation(DbOperation::Batch("shared".into()))
        .unwrap()
    else {
        panic!()
    };
    let mut all = first.clone();
    all.extend(second.clone());
    let mut proposal = ProfileProposal {
        scope: "shared".into(),
        settings_revision: 1,
        previous_version: 0,
        watermark: batch.watermark,
        evidence: all,
        preferences: vec![Preference {
            scope: "shared".into(),
            rule: "解释时保留理由".into(),
            language: WritingLanguage::Han,
            context: Some("说明修改时".into()),
            evidence: first,
        }],
        candidates: vec![],
        reason: "跨应用编辑证据".into(),
        analysis: analysis(),
    };
    assert!(
        repo.language_operation(DbOperation::Publish(proposal.clone()))
            .is_err(),
        "unrelated app evidence must not promote one-app rule"
    );
    proposal.preferences[0].evidence.push(second[0].clone());
    repo.language_operation(DbOperation::Publish(proposal))
        .unwrap();
    let new = evidence("Codex.exe", "请修改", "请修改，并保留理由");
    settle(&mut repo, &new, EditClass::Style, None);
    drop(repo);
    let mut repo = Repository::open(root.path()).unwrap();
    assert!(matches!(
        repo.language_operation(DbOperation::Batch("shared".into()))
            .unwrap(),
        DbOutput::Batch(None)
    ));
    let DbOutput::PendingScopes(scopes) =
        repo.language_operation(DbOperation::PendingScopes).unwrap()
    else {
        panic!()
    };
    assert!(
        scopes.contains(&"shared".into()),
        "cooldown does not erase pending evidence on restart"
    );
}
#[test]
fn expired_edit_body_is_purged_without_waiting_for_model_classification() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let e = evidence("Codex.exe", "赛维", "Sayweave");
    repo.language_operation(DbOperation::SaveEdit(e.clone()))
        .unwrap();
    repo.connection
        .execute(
            "UPDATE language_edits SET expires_at=?1 WHERE id=?2",
            params![now() - 1, e.id],
        )
        .unwrap();
    repo.language_operation(DbOperation::Expire).unwrap();
    let (state, body): (String, String) = repo
        .connection
        .query_row(
            "SELECT state,evidence FROM language_edits WHERE id=?1",
            [&e.id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(state, "expired");
    assert_eq!(body, "{}");
}
#[test]
fn failed_classification_is_not_replayed_and_empty_profile_results_advance_watermark() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let failed = evidence("Codex.exe", "赛维", "Sayweave");
    repo.language_operation(DbOperation::SaveEdit(failed.clone()))
        .unwrap();
    repo.language_operation(DbOperation::RetireEdit {
        id: failed.id.clone(),
        reason: "protocol".into(),
    })
    .unwrap();
    assert!(matches!(
        repo.language_operation(DbOperation::SaveEdit(failed))
            .unwrap(),
        DbOutput::EditReady(None)
    ));
    let mut app = None;
    for _ in 0..3 {
        let e = evidence("Codex.exe", "修改这里", "请修改这里");
        settle(&mut repo, &e, EditClass::Style, None);
        app = Some(e.application);
    }
    let app = app.unwrap();
    let DbOutput::Batch(Some(batch)) = repo
        .language_operation(DbOperation::Batch(app.key.clone()))
        .unwrap()
    else {
        panic!()
    };
    repo.language_operation(DbOperation::Publish(ProfileProposal {
        scope: app.key.clone(),
        settings_revision: 1,
        previous_version: 0,
        watermark: batch.watermark,
        evidence: vec![],
        preferences: vec![],
        candidates: vec![],
        reason: "没有形成稳定偏好".into(),
        analysis: analysis(),
    }))
    .unwrap();
    assert!(matches!(
        repo.language_operation(DbOperation::Batch(app.key))
            .unwrap(),
        DbOutput::Batch(None)
    ));
}
#[test]
fn profile_failure_waits_for_new_evidence_and_foreground_preemption_can_resume() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let mut app = None;
    for _ in 0..3 {
        let e = evidence("Codex.exe", "修改这里", "请修改这里");
        settle(&mut repo, &e, EditClass::Style, None);
        app = Some(e.application);
    }
    let scope = app.unwrap().key;
    let DbOutput::Batch(Some(batch)) = repo
        .language_operation(DbOperation::Batch(scope.clone()))
        .unwrap()
    else {
        panic!()
    };
    repo.language_operation(DbOperation::ProfileAttempt {
        scope: scope.clone(),
        settings_revision: 1,
        watermark: batch.watermark,
        released: false,
    })
    .unwrap();
    repo.connection
        .execute(
            "UPDATE language_profiles SET attempted_at=?1 WHERE scope=?2",
            params![now() - 600001, scope],
        )
        .unwrap();
    assert!(
        matches!(
            repo.language_operation(DbOperation::Batch(scope.clone()))
                .unwrap(),
            DbOutput::Batch(None)
        ),
        "no new evidence must not repeat the model request"
    );
    repo.language_operation(DbOperation::ProfileAttempt {
        scope: scope.clone(),
        settings_revision: 1,
        watermark: batch.watermark,
        released: true,
    })
    .unwrap();
    assert!(matches!(
        repo.language_operation(DbOperation::Batch(scope)).unwrap(),
        DbOutput::Batch(Some(_))
    ));
}
#[test]
fn newer_capitalization_updates_automatic_terms_but_preserves_manual_maintenance() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    settle(
        &mut repo,
        &evidence("Codex.exe", "SQLit", "sqlite"),
        EditClass::Lexical,
        Some("sqlite"),
    );
    let old = terms(&mut repo).remove(0);
    settle(
        &mut repo,
        &evidence("Codex.exe", "sqlite", "SQLite"),
        EditClass::Lexical,
        Some("SQLite"),
    );
    let current = terms(&mut repo).remove(0);
    assert_eq!(current.id, old.id);
    assert_eq!(current.original, "SQLite");
    assert!(current.revision > old.revision);
    repo.save(
        Input {
            kind: Kind::Term,
            original: "SQLite".into(),
            replacement: None,
            category: None,
        },
        Some((current.id, current.revision)),
    )
    .unwrap();
    settle(
        &mut repo,
        &evidence("Codex.exe", "SQLite", "SQLITE"),
        EditClass::Lexical,
        Some("SQLITE"),
    );
    let maintained = terms(&mut repo).remove(0);
    assert_eq!(maintained.original, "SQLite");
    assert_eq!(maintained.source, Source::Manual);
}
