//! Test-build-only synthetic data fixtures for real NSIS replacement/upgrade checks.
use crate::{
    contracts::{
        dictionary::{Input, Kind, Query},
        history::{Attempt, DbOperation as H, Prepare},
        language::{
            AnalysisStamp, ApplicationContext, DbOperation as L, DbOutput as LO, Preference,
            ProfileProposal, StyleObservation, WritingLanguage,
        },
    },
    data::{DataWorker, Operation, Output},
    models::{
        ModelService,
        types::{ConfigInput, KeyChange, SaveRequest, Usage},
    },
};
use tauri::{Manager, WebviewWindow};
fn authorize(window: &WebviewWindow) -> Result<(), String> {
    if window.label() != "main"
        || crate::build_info::current().channel != "test"
        || !env!("SAYWEAVE_BUILD_IDENTIFIER").starts_with("local.sayweave.acceptance.")
    {
        return Err("forbidden".into());
    }
    Ok(())
}
async fn db(app: &tauri::AppHandle, op: Operation) -> Result<Output, String> {
    app.state::<DataWorker>()
        .request_admitted(op)
        .await
        .map_err(|e| e.code)
}
#[tauri::command]
pub async fn acceptance_update_seed(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> Result<serde_json::Value, String> {
    authorize(&window)?;
    let model = app.state::<ModelService>();
    model.status("main", false).await.map_err(|e| e.code)?;
    let view = model
        .save(
            "main",
            SaveRequest {
                input: ConfigInput {
                    name: "Update synthetic model".into(),
                    protocol: "openai-chat".into(),
                    base_url: "https://example.invalid/v1".into(),
                    model_id: "update-fixture".into(),
                    usages: vec![Usage::Text],
                },
                key: KeyChange::Replace {
                    value: "isolated-update-test-key".into(),
                },
                id: None,
                revision: None,
                expectation: None,
            },
        )
        .await
        .map_err(|e| e.code)?;
    let config_id = view.configs.first().ok_or("model missing")?.id.clone();
    db(
        &app,
        Operation::Create(Input {
            kind: Kind::Term,
            original: "更新验收术语".into(),
            category: None,
            replacement: None,
        }),
    )
    .await?;
    let application = ApplicationContext::from_identity(None, "C:\\Fixture\\codex.exe");
    let mut evidence = vec![];
    for _ in 0..10 {
        let id = uuid::Uuid::new_v4().to_string();
        let record = uuid::Uuid::new_v4().to_string();
        let task = uuid::Uuid::new_v4().to_string();
        let attempt = Attempt {
            id: id.clone(),
            record_id: record.clone(),
            kind: "capture".into(),
            config_id: config_id.clone(),
            config_name: "Update synthetic model".into(),
            model_id: "update-fixture".into(),
            service_host: "example.invalid".into(),
            config_revision: 1,
            created_at: 1,
            ended_at: None,
            status: "preparing".into(),
            text: String::new(),
            error_code: None,
            elapsed_ms: 0,
        };
        db(
            &app,
            Operation::History(H::Prepare(Prepare {
                record_id: record,
                attempt,
                audio: None,
            })),
        )
        .await?;
        db(
            &app,
            Operation::History(H::Finish {
                attempt_id: id.clone(),
                status: "success".into(),
                text: "这是合成更新验收数据，保留完整理由和步骤。".into(),
                error: None,
                duration_ms: 1000,
                elapsed_ms: 1000,
            }),
        )
        .await?;
        db(
            &app,
            Operation::History(H::Delivery {
                task: task.clone(),
                attempt: id.clone(),
                state: "submitted".into(),
                error: None,
            }),
        )
        .await?;
        let observation = uuid::Uuid::new_v4().to_string();
        evidence.push(observation.clone());
        db(
            &app,
            Operation::Language(L::Observe(StyleObservation {
                id: observation,
                task,
                application: application.clone(),
                settings_revision: 1,
                language: WritingLanguage::Han,
                kind: "speech".into(),
                summary: "保留完整理由和步骤".into(),
                summary_complete: true,
                source_ref: Some(id),
            })),
        )
        .await?;
    }
    let Output::Language(LO::Batch(Some(batch))) =
        db(&app, Operation::Language(L::Batch(application.key.clone()))).await?
    else {
        return Err("profile batch missing".into());
    };
    db(
        &app,
        Operation::Language(L::Publish(ProfileProposal {
            scope: application.key.clone(),
            settings_revision: 1,
            previous_version: 0,
            watermark: batch.watermark,
            evidence: evidence.clone(),
            preferences: vec![Preference {
                scope: application.key,
                rule: "保留完整理由和步骤".into(),
                language: WritingLanguage::Han,
                context: None,
                evidence,
            }],
            candidates: vec![],
            reason: "Synthetic installation acceptance fixture".into(),
            analysis: AnalysisStamp {
                config_id,
                config_revision: 1,
                model_id: "update-fixture".into(),
                service_host: "example.invalid".into(),
                template_version: "fixture-v1".into(),
                strategy_version: "application-learning-v1".into(),
            },
        })),
    )
    .await?;
    acceptance_update_data(app, window).await
}
#[tauri::command]
pub async fn acceptance_update_data(
    app: tauri::AppHandle,
    window: WebviewWindow,
) -> Result<serde_json::Value, String> {
    authorize(&window)?;
    let model = app.state::<ModelService>();
    model.status("main", false).await.map_err(|e| e.code)?;
    let models = model.overview("main").await.map_err(|e| e.code)?;
    let key_correct =
        model.text_config().await.map_err(|e| e.code)?.key == "isolated-update-test-key";
    let Output::Page(dictionary) = db(
        &app,
        Operation::List(Query {
            kind: Kind::Term,
            search: String::new(),
            page: 1,
            source: None,
        }),
    )
    .await?
    else {
        return Err("dictionary missing".into());
    };
    let Output::History(crate::contracts::history::DbOutput::Usage(history)) =
        db(&app, Operation::History(H::Usage)).await?
    else {
        return Err("history missing".into());
    };
    let Output::Language(LO::Overview(profile)) =
        db(&app, Operation::Language(L::Overview(None))).await?
    else {
        return Err("profile missing".into());
    };
    let status = app.state::<DataWorker>().status().map_err(|e| e.code)?;
    Ok(
        serde_json::json!({"models":models,"credentialMatches":key_correct,"dictionary":dictionary,"history":history,"profile":profile,"schema":status.schema_version,"component":crate::text_service::text_service_status(window).await.map_err(|e|e.code)?}),
    )
}
#[tauri::command]
pub fn acceptance_update_exit(app: tauri::AppHandle, window: WebviewWindow) -> Result<(), String> {
    authorize(&window)?;
    crate::tray_action(&app, "exit").map_err(|e| e.code.to_owned())
}
#[tauri::command]
pub async fn acceptance_update_hold(
    app: tauri::AppHandle,
    window: WebviewWindow,
    kind: String,
) -> Result<(), String> {
    authorize(&window)?;
    match kind.as_str() {
        "mutation" => {
            let permit = app
                .state::<crate::lifecycle::Lifecycle>()
                .enter()
                .map_err(str::to_owned)?;
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(12)).await;
                drop(permit);
            });
        }
        "stop" => {
            let (send, receive) = tokio::sync::oneshot::channel();
            tauri::async_runtime::spawn(async move {
                app.state::<crate::shortcuts::ShortcutService>()
                    .acceptance_hold_drain(send)
                    .await;
            });
            receive.await.map_err(|_| "hold failed")?;
        }
        _ => return Err("invalid hold".into()),
    }
    Ok(())
}
