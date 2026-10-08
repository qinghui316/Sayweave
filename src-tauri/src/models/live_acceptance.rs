//! Explicit selected-provider acceptance. Reads the daily configuration/vault;
//! sends only the synthetic string below and never changes user configuration.
use super::{
    credentials::{Credentials, WindowsCredentials},
    provider::Provider,
    types::*,
};
use crate::processing::{
    prompt::{self, DictionaryIndex},
    quality,
};
use serde_json::json;
use std::{path::Path, sync::Arc, time::Duration};

#[tokio::test]
#[ignore = "Explicit authorized selected-default provider request; synthetic text only"]
async fn live_processing_acceptance() {
    assert_eq!(
        std::env::var("SAYWEAVE_LIVE_PROCESSING_ACCEPTANCE").as_deref(),
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
    let count = |table: &str| {
        connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
    };
    let before = [
        count("speech_records"),
        count("model_configs"),
        count("model_defaults"),
    ];
    assert!(before[0] > 0, "Refuse redirected empty test profile");
    let (id,revision,input,reference) = connection.query_row(
        "SELECT c.id,c.revision,c.name,c.protocol,c.base_url,c.model_id,c.credential_ref FROM model_defaults d JOIN model_configs c ON c.id=d.config_id WHERE d.usage='text' AND c.text=1", [], |row|{
            Ok((row.get::<_,String>(0)?,row.get::<_,i64>(1)?,ConfigInput{name:row.get(2)?,protocol:row.get(3)?,base_url:row.get(4)?,model_id:row.get(5)?,usages:vec![Usage::Text]},row.get::<_,String>(6)?))
        }).unwrap();
    let input = input.validate().unwrap();
    // Match the daily release namespace assembled in lib.rs, independently
    // of whether this explicit test binary was built in debug or release.
    let key = match WindowsCredentials::new("local.sayweave.skeleton.release".into())
        .get(&reference)
    {
        Ok(key) => key,
        Err(error) => {
            let report = json!({"passed":false,"stage":"credential","error":error.code,"providerRequest":false,"readOnlyProfile":true,"countsBefore":before});
            std::fs::write(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../.local/polish-live-provider.json"),
                serde_json::to_vec_pretty(&report).unwrap(),
            )
            .unwrap();
            panic!("Daily release credential unavailable; no provider request sent");
        }
    };
    let synthetic = "呃，先给 Sayweave 补一个 SQLite 备份，预算5000元，先别发公告。";
    let prompt = prompt::build(synthetic, "", &DictionaryIndex::new(vec![])).unwrap();
    let provider = Provider::new(Arc::new(crate::network::RequestCoordinator::default())).unwrap();
    let began = std::time::Instant::now();
    let result = provider
        .generate_unit(
            &input,
            &key,
            &uuid::Uuid::new_v4().to_string(),
            &prompt.messages,
            tokio::time::Instant::now() + Duration::from_secs(15),
        )
        .await;
    // Provider errors are stable codes, never raw response bodies or key values.
    let mut report = json!({"passed":false,"providerRequest":true,"syntheticOnly":true,"readOnlyProfile":true,"profile":database,"pid":std::process::id(),"exe":std::env::current_exe().unwrap(),"countsBefore":before,"configId":id,"configRevision":revision,"modelId":input.model_id,"promptVersion":prompt::VERSION,"promptBytes":prompt.serialized_bytes,"totalMs":began.elapsed().as_millis()});
    if let Ok(generated) = &result {
        report["output"] = json!(generated.text);
        report["firstOutputMs"] = json!(generated.first_output_ms);
        report["generationMs"] = json!(generated.elapsed_ms);
        report["inputTokens"] = json!(generated.usage.input);
        report["outputTokens"] = json!(generated.usage.output);
        report["qualityError"] = json!(
            quality::validate_output(&generated.text)
                .err()
                .map(|e| e.code)
        );
        report["passed"] = json!(quality::validate_output(&generated.text).is_ok());
    } else if let Err(error) = &result {
        report["error"] = json!(error.code);
    }
    let after = [
        count("speech_records"),
        count("model_configs"),
        count("model_defaults"),
    ];
    report["countsAfter"] = json!(after);
    let output = Path::new(env!("CARGO_MANIFEST_DIR")).join("../.local/polish-live-provider.json");
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert_eq!(
        before, after,
        "Read-only acceptance must preserve daily records/configuration"
    );
    assert_eq!(
        report["passed"], true,
        "Selected-provider acceptance failed; see stable-code report"
    );
}
