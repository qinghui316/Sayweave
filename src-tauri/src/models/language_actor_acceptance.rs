//! Opt-in isolated actor acceptance. No fixture profiles or direct Batch/Publish.
use super::{
    ModelService,
    credentials::Credentials,
    credentials::WindowsCredentials,
    types::{
        ConfigInput, DefaultRequest, Expectation, KeyChange, RemoveRequest, Replacement,
        SaveRequest, Usage,
    },
};
use crate::{
    contracts::{dictionary as dictionary_contract, language::*},
    data::{DataWorker, Operation, Output},
    language_learning::{self, LanguageLearning},
    processing::prompt::DictionaryIndex,
};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, time::Duration};
use tauri::Manager;

fn require(condition: bool, reason: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(reason.into())
    }
}
fn uid() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn source_db(root: &Path) -> Result<rusqlite::Connection, String> {
    rusqlite::Connection::open_with_flags(
        root.join("dictionary.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .map_err(|_| "source database unavailable".into())
}
fn source_counts(root: &Path) -> Result<Value, String> {
    let db = source_db(root)?;
    let mut counts = json!({});
    for table in [
        "speech_records",
        "model_configs",
        "dictionary_terms",
        "correction_rules",
        "language_profiles",
    ] {
        let count: i64 = db
            .query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
            .map_err(|_| "source count unavailable")?;
        counts[table] = json!(count);
    }
    Ok(counts)
}
async fn import_model(app: &tauri::AppHandle, source: &Path) -> Result<String, String> {
    let (input, reference) = source_db(source)?.query_row(
        "SELECT c.name,c.protocol,c.base_url,c.model_id,c.credential_ref FROM model_configs c JOIN model_defaults d ON d.config_id=c.id WHERE d.usage='text'",
        [], |r| Ok((ConfigInput {name:r.get(0)?,protocol:r.get(1)?,base_url:r.get(2)?,model_id:r.get(3)?,usages:vec![Usage::Text]},r.get::<_,String>(4)?)),
    ).map_err(|_| "selected default text model missing")?;
    // Secret stays in native memory and the isolated Windows credential namespace.
    let key = WindowsCredentials::new("local.sayweave.skeleton.release".into())
        .get(&reference)
        .map_err(|e| e.code)?;
    let service = app.state::<ModelService>();
    let view = service
        .save(
            "main",
            SaveRequest {
                input,
                key: KeyChange::Replace { value: key },
                id: None,
                revision: None,
                expectation: None,
            },
        )
        .await
        .map_err(|e| e.code)?;
    let saved = view.configs.first().ok_or("isolated model save missing")?;
    let id = saved.id.clone();
    Ok(id)
}
async fn cleanup_model(app: &tauri::AppHandle, id: &str) -> Result<(), String> {
    let service = app.state::<ModelService>();
    let view = service.overview("main").await.map_err(|e| e.code)?;
    let saved = view
        .configs
        .iter()
        .find(|c| c.id == id)
        .ok_or("temporary model missing")?;
    let removed = service
        .remove(
            "main",
            RemoveRequest {
                id: id.into(),
                revision: saved.revision,
                expectation: Expectation {
                    defaults: view.defaults,
                    replacements: vec![Replacement {
                        usage: Usage::Text,
                        config_id: None,
                    }],
                },
            },
        )
        .await
        .map_err(|e| e.code)?;
    require(
        removed.configs.is_empty() && !removed.cleanup_pending,
        "temporary credential cleanup incomplete",
    )
}
async fn controls(
    app: &tauri::AppHandle,
    terms: bool,
    style: bool,
) -> Result<LanguageSettings, String> {
    let data = app.state::<DataWorker>();
    let DbOutput::Settings(old) = language_learning::operation(&data, DbOperation::Settings)
        .await
        .map_err(|e| e.code)?
    else {
        return Err("settings missing".into());
    };
    language_learning::language_save(
        app.clone(),
        app.get_webview_window("main").ok_or("main missing")?,
        data,
        LanguageSettingsPatch {
            automatic_terms: Some(Settings {
                enabled: terms,
                revision: old.automatic_terms.revision,
            }),
            personalization: Some(Settings {
                enabled: style,
                revision: old.personalization.revision,
            }),
        },
    )
    .await
    .map_err(|e| e.code)
}
fn evidence(
    app: &ApplicationContext,
    config: &LanguageSettings,
    before: &str,
    after: &str,
) -> EditEvidence {
    EditEvidence {
        id: uid(),
        task: uid(),
        turn: 1,
        operation: 1,
        sequence: 1,
        region: "controlled-receipt".into(),
        expires_at: crate::data::repository::now() + 600000,
        application: app.clone(),
        eligibility: LearningEligibility::capture(config),
        raw: before.into(),
        before: before.into(),
        after: after.into(),
    }
}
async fn overview(app: &tauri::AppHandle) -> Result<ProfileOverview, String> {
    let DbOutput::Overview(value) =
        language_learning::operation(&app.state::<DataWorker>(), DbOperation::Overview(None))
            .await
            .map_err(|e| e.code)?
    else {
        return Err("overview missing".into());
    };
    Ok(value)
}
async fn terms(app: &tauri::AppHandle) -> Result<Vec<dictionary_contract::Record>, String> {
    let Output::Page(value) = app
        .state::<DataWorker>()
        .request(
            "main",
            Operation::List(dictionary_contract::Query {
                kind: dictionary_contract::Kind::Term,
                search: String::new(),
                page: 1,
                source: Some(dictionary_contract::Source::Automatic),
            }),
        )
        .await
        .map_err(|e| e.code)?
    else {
        return Err("terms missing".into());
    };
    Ok(value.records)
}
async fn exercise(app: &tauri::AppHandle, report: &mut Value) -> Result<(), String> {
    let application = ApplicationContext::from_identity(None, "Codex.exe");
    let config = controls(app, true, false).await?;
    app.state::<LanguageLearning>().edit(evidence(
        &application,
        &config,
        "项目名是起蓝。",
        "项目名是绮蓝。",
    ));
    let end = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let words = terms(app).await?;
        if words.iter().any(|word| word.original == "绮蓝") {
            break;
        }
        require(
            tokio::time::Instant::now() < end,
            "independent automatic term actor timed out",
        )?;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    require(
        overview(app).await?.observation_count == 0,
        "personalization-off created style observations",
    )?;
    report["automaticTermsWithPersonalizationOff"] = json!(true);
    let config = controls(app, false, true).await?;
    let frozen = language_learning::snapshot(&app.state::<DataWorker>(), application.clone())
        .await
        .map_err(|e| e.code)?;
    require(
        frozen.profile_version == 0 && frozen.preferences.is_empty(),
        "profile fixture was not empty",
    )?;
    let prior = frozen.clone();
    let dictionary = Arc::new(DictionaryIndex::new(terms(app).await?));
    for (before, after) in [
        ("修改这里，发给我。", "麻烦修改一下这里，再发给我吧。"),
        (
            "检查这个配置，告诉我原因。",
            "麻烦检查一下这个配置，再告诉我原因吧。",
        ),
        (
            "保留这些步骤，解释一下。",
            "麻烦保留这些步骤，再解释一下吧。",
        ),
    ] {
        crate::input_state::edited();
        let item = evidence(&application, &config, before, after);
        language_learning::operation(
            &app.state::<DataWorker>(),
            DbOperation::SaveEdit(item.clone()),
        )
        .await
        .map_err(|e| e.code)?;
        language_learning::operation(
            &app.state::<DataWorker>(),
            DbOperation::ClassifyEdit {
                id: item.id,
                eligibility: item.eligibility,
                decision: LexicalDecision {
                    class: EditClass::Style,
                    original: None,
                    term: None,
                    summary: Some(
                        "交流技术修改请求时使用自然礼貌的口语语气，并保留完整句式".into(),
                    ),
                },
                analysis: AnalysisStamp {
                    config_id: "acceptance".into(),
                    config_revision: 1,
                    model_id: "acceptance".into(),
                    service_host: "acceptance".into(),
                    template_version: "lexical-v1".into(),
                    strategy_version: "acceptance".into(),
                },
            },
        )
        .await
        .map_err(|e| e.code)?;
    }
    let submitted = tokio::time::Instant::now();
    let end = submitted + Duration::from_secs(150);
    let next = loop {
        let value = language_learning::snapshot(&app.state::<DataWorker>(), application.clone())
            .await
            .map_err(|e| e.code)?;
        if value.profile_version > 0 {
            break value;
        }
        require(
            tokio::time::Instant::now() < end,
            "automatic idle/threshold profile publication timed out",
        )?;
        tokio::time::sleep(Duration::from_millis(250)).await;
    };
    require(
        !next.preferences.is_empty(),
        "automatic publication had no learned preferences",
    )?;
    require(
        submitted.elapsed() >= Duration::from_secs(30),
        "profile published before idle policy",
    )?;
    require(
        prior.profile_version == 0 && prior.preferences.is_empty(),
        "current Session changed profile",
    )?;
    require(
        terms(app).await?.len() == 1,
        "terms-off style work created a term",
    )?;
    let other = language_learning::snapshot(
        &app.state::<DataWorker>(),
        ApplicationContext::from_identity(None, "Weixin.exe"),
    )
    .await
    .map_err(|e| e.code)?;
    require(
        other.preferences.is_empty(),
        "one-app profile leaked into another app",
    )?;
    report["automaticProfile"] = json!({"scope":application.key,"version":next.profile_version,"rules":next.preferences.len(),"elapsedMs":submitted.elapsed().as_millis(),"frozenCurrentSession":true,"otherAppUnchanged":true});
    let input = "修改这里，发给我。";
    let prompt = crate::processing::prompt::build_with_context(
        input,
        "",
        &dictionary,
        Some(&next.application),
        &next.preferences,
    )
    .map_err(|e| e.code)?;
    let prompt_text =
        serde_json::to_string(&prompt.messages).map_err(|_| "prompt serialization")?;
    require(
        next.preferences
            .iter()
            .all(|preference| prompt_text.contains(&preference.rule)),
        "next-session prompt omitted learned application profile",
    )?;
    report["nextSessionPromptApplied"] = json!(true);
    let view = overview(app).await?;
    require(
        view.preference_count > 0 && view.application_count == 1 && view.observation_count == 3,
        "dashboard totals differ from automatic facts",
    )?;
    report["overview"] = serde_json::to_value(view).map_err(|_| "overview serialization")?;
    Ok(())
}
pub(crate) fn start(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut report = json!({"passed":false,"syntheticEditEvidence":true,"productionActor":true,"directBatchPublish":false});
        let source = app
            .path()
            .app_data_dir()
            .map(|p| crate::data::profile_root(&p, false));
        let isolated = std::env::var("SAYWEAVE_DATA_ACCEPTANCE_ID")
            .ok()
            .filter(|id| uuid::Uuid::parse_str(id).is_ok());
        report["isolatedProfileId"] = json!(isolated);
        let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.local");
        let mut model = None;
        let result = async {
            require(isolated.is_some(), "isolated acceptance id required")?;
            let source = source.map_err(|_| "source profile path unavailable")?;
            let before = source_counts(&source)?;
            let ready = tokio::time::Instant::now() + Duration::from_secs(5);
            while app.state::<ModelService>().overview("main").await.is_err() {
                require(
                    tokio::time::Instant::now() < ready,
                    "isolated models not ready",
                )?;
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let id = import_model(&app, &source).await?;
            model = Some(id.clone());
            let view = app
                .state::<ModelService>()
                .overview("main")
                .await
                .map_err(|e| e.code)?;
            let default = view
                .defaults
                .iter()
                .find(|d| d.usage == Usage::Text)
                .ok_or("text default missing")?;
            app.state::<ModelService>()
                .set_default(
                    "main",
                    DefaultRequest {
                        usage: Usage::Text,
                        config_id: id,
                        revision: default.revision,
                    },
                )
                .await
                .map_err(|e| e.code)?;
            exercise(&app, &mut report).await?;
            require(
                source_counts(&source)? == before,
                "daily profile counts changed",
            )?;
            report["dailyCounts"] = before;
            Ok::<(), String>(())
        }
        .await;
        let cleanup = match model {
            Some(id) => cleanup_model(&app, &id).await,
            None => Ok(()),
        };
        report["temporaryCredentialCleaned"] = json!(cleanup.is_ok());
        report["passed"] = json!(result.is_ok() && cleanup.is_ok());
        report["error"] = json!(result.err().or_else(|| cleanup.err()));
        let _ = std::fs::write(
            folder.join("language-actor-acceptance.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        );
        println!(
            "SAYWEAVE_LANGUAGE_ACTOR_SETTLED passed={}",
            report["passed"]
        );
        // The owned WebView runner reads native dashboard values and exits via
        // the existing acceptance_exit command. It never opens the daily app.
    });
}
