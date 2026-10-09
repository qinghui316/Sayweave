//! Explicit selected-default evaluation: synthetic data and isolated learning DB.
use super::{
    credentials::{Credentials, WindowsCredentials},
    provider::Provider,
    types::*,
};
use crate::contracts::language::{DbOperation, DbOutput};
use crate::{
    contracts::{
        dictionary::{Kind, Record, Source},
        language::*,
        processing::Message,
    },
    data::{DataWorker, Operation, Output},
    language_learning,
    processing::prompt::{self, DictionaryIndex},
};
use serde_json::json;
use std::{path::Path, sync::Arc, time::Duration};

#[tokio::test]
#[ignore = "Explicit authorized selected-default synthetic prompt and learning evaluation"]
async fn selected_language_evaluation() {
    assert_eq!(
        std::env::var("SAYWEAVE_LIVE_LANGUAGE_ACCEPTANCE").as_deref(),
        Ok("selected-default-synthetic")
    );
    assert!(std::env::var_os("SAYWEAVE_DATA_ACCEPTANCE_ID").is_none());
    let database = Path::new(&std::env::var("APPDATA").unwrap())
        .join("local.sayweave.skeleton/data/dictionary.sqlite");
    let connection = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let counts = || {
        ["speech_records", "model_configs", "model_defaults"].map(|table| {
            connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| {
                    r.get::<_, i64>(0)
                })
                .unwrap()
        })
    };
    let before_counts = counts();
    assert!(before_counts[0] > 0, "Refuse redirected empty profile");
    let (id,revision,input,reference) = connection.query_row("SELECT c.id,c.revision,c.name,c.protocol,c.base_url,c.model_id,c.credential_ref FROM model_defaults d JOIN model_configs c ON c.id=d.config_id WHERE d.usage='text' AND c.text=1",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,ConfigInput { name:r.get(2)?,protocol:r.get(3)?,base_url:r.get(4)?,model_id:r.get(5)?,usages:vec![Usage::Text] },r.get::<_,String>(6)?))).unwrap();
    let input = input.validate().unwrap();
    let key = WindowsCredentials::new("local.sayweave.skeleton.release".into())
        .get(&reference)
        .map_err(|e| e.code)
        .expect("Selected release credential unavailable");
    let provider = Provider::new(Arc::new(crate::network::RequestCoordinator::default())).unwrap();
    let refinement =
        std::env::var("SAYWEAVE_LIVE_LANGUAGE_CASES").as_deref() == Ok("prompt-refinement");
    let report_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(if refinement {
        "../.local/language-prompt-refinement.json"
    } else {
        "../.local/language-selected-model.json"
    });
    let mut report = json!({"syntheticOnly":true,"readOnlyDailyProfile":true,"configId":id,"configRevision":revision,"modelId":input.model_id,"promptVersion":prompt::VERSION,"cases":[],"countsBefore":before_counts});
    let app = ApplicationContext::from_identity(None, "Codex.exe");
    let terms = DictionaryIndex::new(vec![Record {
        id: "synthetic-term".into(),
        kind: Kind::Term,
        original: "Sayweave".into(),
        replacement: None,
        category: None,
        revision: 1,
        created_at: 0,
        updated_at: 0,
        source: Source::Manual,
    }]);
    let baseline = include_str!("../../../tests/fixtures/faithful-v1.txt");
    for (case, body) in [
        (
            "tone-order",
            "呃，你先给我看一下理由吧，然后再做测试，最后再发布，好吗？",
        ),
        (
            "numbers-correction",
            "端口八千零八，预算五千元，今天十月八号，哦，不对，今天十月七号，先别发公告。",
        ),
        (
            "term-path",
            "把赛维的日志存到 E:\\workspace\\Sayweave\\logs，先别删除。",
        ),
    ] {
        let new = prompt::build_with_context(body, "", &terms, Some(&app), &[]).unwrap();
        let old = vec![Message { role:"system",content:baseline.into() },Message { role:"user",content:json!({"input_text":body,"previous_context":"","terms":["Sayweave"],"corrections":[]}).to_string() }];
        for (version, messages) in [("faithful-v1", old), ("faithful-v2", new.messages)] {
            if refinement && version == "faithful-v1" {
                continue;
            }
            let result = provider
                .generate_unit(
                    &input,
                    &key,
                    &uuid::Uuid::new_v4().to_string(),
                    &messages,
                    tokio::time::Instant::now() + Duration::from_secs(15),
                )
                .await;
            let row = match result {
                Ok(r) => {
                    json!({"case":case,"version":version,"input":body,"output":r.text,"elapsedMs":r.elapsed_ms,"firstOutputMs":r.first_output_ms})
                }
                Err(e) => json!({"case":case,"version":version,"error":e.code}),
            };
            report["cases"].as_array_mut().unwrap().push(row);
            std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        }
    }
    if refinement {
        report["countsAfter"] = json!(counts());
        std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        assert_eq!(before_counts, counts());
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let data = DataWorker::start(root.path().to_path_buf()).unwrap();
    let mut edit_passed = 0;
    for (case, before, after, expected) in [
        (
            "lexical",
            "请更新赛维。",
            "请更新Sayweave。",
            EditClass::Lexical,
        ),
        (
            "content",
            "周五上线，预算5000元。",
            "周四上线，预算3000元。",
            EditClass::Content,
        ),
        (
            "style",
            "修改这里，发给我。",
            "麻烦修改一下这里，再发给我吧。",
            EditClass::Style,
        ),
    ] {
        let e = EditEvidence {
            id: uuid::Uuid::new_v4().to_string(),
            task: uuid::Uuid::new_v4().to_string(),
            turn: 1,
            operation: 2,
            sequence: 1,
            region: "tail".into(),
            expires_at: crate::data::repository::now() + 600000,
            application: app.clone(),
            settings_revision: 1,
            raw: before.into(),
            before: before.into(),
            after: after.into(),
        };
        let messages =
            language_learning::edit_messages(&e, &json!({"terms":[],"ambiguous_candidates":[]}))
                .unwrap();
        let result = provider
            .generate_unit(
                &input,
                &key,
                &uuid::Uuid::new_v4().to_string(),
                &messages,
                tokio::time::Instant::now() + Duration::from_secs(15),
            )
            .await;
        let mut row = json!({"case":case,"expected":expected});
        if let Ok(r) = result {
            row["output"] = json!(r.text);
            row["elapsedMs"] = json!(r.elapsed_ms);
            let body = r
                .text
                .trim()
                .strip_prefix("```json")
                .and_then(|b| b.strip_suffix("```"))
                .unwrap_or(r.text.trim())
                .trim();
            if let Ok(decision) = serde_json::from_str::<LexicalDecision>(body) {
                row["classMatches"] = json!(decision.class == expected);
                if decision.class == expected {
                    language_learning::operation(&data, DbOperation::SaveEdit(e.clone()))
                        .await
                        .unwrap();
                    let settled = language_learning::operation(
                        &data,
                        DbOperation::ClassifyEdit {
                            id: e.id,
                            settings_revision: 1,
                            decision,
                            analysis: stamp(&id, revision, &input, "lexical-v1"),
                        },
                    )
                    .await;
                    row["isolatedSettled"] = json!(settled.is_ok());
                    if settled.is_ok() {
                        edit_passed += 1;
                    }
                }
            }
        } else if let Err(e) = result {
            row["error"] = json!(e.code);
        }
        report["cases"].as_array_mut().unwrap().push(row);
        std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    let mut observations = Vec::new();
    for _ in 0..3 {
        let e = EditEvidence {
            id: uuid::Uuid::new_v4().to_string(),
            task: uuid::Uuid::new_v4().to_string(),
            turn: 1,
            operation: 2,
            sequence: 1,
            region: "tail".into(),
            expires_at: crate::data::repository::now() + 600000,
            application: app.clone(),
            settings_revision: 1,
            raw: "修改这里，发给我。".into(),
            before: "修改这里，发给我。".into(),
            after: "麻烦修改一下这里，再发给我吧。".into(),
        };
        language_learning::operation(&data, DbOperation::SaveEdit(e.clone()))
            .await
            .unwrap();
        language_learning::operation(
            &data,
            DbOperation::ClassifyEdit {
                id: e.id.clone(),
                settings_revision: 1,
                decision: LexicalDecision {
                    class: EditClass::Style,
                    original: None,
                    term: None,
                    summary: Some(
                        "交流技术修改请求时使用自然礼貌的口语语气，并保留完整句式".into(),
                    ),
                },
                analysis: stamp(&id, revision, &input, "synthetic-fixture"),
            },
        )
        .await
        .unwrap();
        observations.push(e.id);
    }
    let DbOutput::Batch(Some(batch)) =
        language_learning::operation(&data, DbOperation::Batch(app.key.clone()))
            .await
            .unwrap()
    else {
        panic!("independent style evidence required")
    };
    let (messages, allowed) = language_learning::profile_messages(&batch).unwrap();
    let result = provider
        .generate_unit(
            &input,
            &key,
            &uuid::Uuid::new_v4().to_string(),
            &messages,
            tokio::time::Instant::now() + Duration::from_secs(15),
        )
        .await;
    let mut profile_passed = false;
    let mut row =
        json!({"case":"profile-evolution","scope":app.key,"fixtureEvidence":observations});
    if let Ok(r) = result {
        row["output"] = json!(r.text);
        row["elapsedMs"] = json!(r.elapsed_ms);
        if let Ok(proposal) = language_learning::profile_proposal(
            batch,
            &allowed,
            &r.text,
            stamp(&id, revision, &input, "app-style-v1"),
        ) {
            row["parsed"] = json!(true);
            row["preferenceCount"] = json!(proposal.preferences.len());
            profile_passed = !proposal.preferences.is_empty()
                && language_learning::operation(&data, DbOperation::Publish(proposal))
                    .await
                    .is_ok();
        }
    } else if let Err(e) = result {
        row["error"] = json!(e.code);
    }
    row["published"] = json!(profile_passed);
    report["cases"].as_array_mut().unwrap().push(row);
    let rows = data
        .request(
            "main",
            Operation::List(crate::contracts::dictionary::Query {
                kind: Kind::Term,
                search: "".into(),
                page: 1,
                source: Some(Source::Automatic),
            }),
        )
        .await
        .unwrap();
    report["automaticTermCount"] = json!(match rows {
        Output::Page(p) => p.records.len(),
        _ => 0,
    });
    report["countsAfter"] = json!(counts());
    report["editCasesPassed"] = json!(edit_passed);
    report["profilePassed"] = json!(profile_passed);
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    data.shutdown().await;
    assert_eq!(before_counts, counts());
    assert_eq!(edit_passed, 3, "See synthetic evidence report");
    assert!(profile_passed, "See profile evidence report");
}
fn stamp(id: &str, revision: i64, input: &ConfigInput, template: &str) -> AnalysisStamp {
    AnalysisStamp {
        config_id: id.into(),
        config_revision: revision,
        model_id: input.model_id.clone(),
        service_host: reqwest::Url::parse(&input.base_url)
            .unwrap()
            .host_str()
            .unwrap()
            .into(),
        template_version: template.into(),
        strategy_version: "application-learning-v1".into(),
    }
}
