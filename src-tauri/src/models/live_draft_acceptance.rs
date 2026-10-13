//! Opt-in paired provider comparison. Reads configuration only; no editor/data writes.
use super::{
    credentials::{Credentials, WindowsCredentials},
    types::*,
};
use crate::processing::prompt::{self, DictionaryIndex};
use serde_json::json;
use std::{path::Path, sync::Arc, time::Duration};

#[tokio::test]
#[ignore = "Explicit selected-default provider comparison; synthetic trajectories only"]
async fn live_revisable_draft_comparison() {
    assert_eq!(
        std::env::var("SAYWEAVE_LIVE_DRAFT_ACCEPTANCE").as_deref(),
        Ok("selected-default-synthetic")
    );
    assert!(std::env::var_os("SAYWEAVE_DATA_ACCEPTANCE_ID").is_none());
    let database = Path::new(&std::env::var("APPDATA").unwrap())
        .join("local.sayweave.skeleton/data/dictionary.sqlite");
    let db = rusqlite::Connection::open_with_flags(
        &database,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let selected = || {
        db.query_row("SELECT c.id,c.revision,c.name,c.protocol,c.base_url,c.model_id,c.credential_ref FROM model_defaults d JOIN model_configs c ON c.id=d.config_id WHERE d.usage='text' AND c.text=1", [], |r| Ok((r.get::<_,String>(0)?, r.get::<_,i64>(1)?, ConfigInput {name:r.get(2)?, protocol:r.get(3)?, base_url:r.get(4)?, model_id:r.get(5)?, usages:vec![Usage::Text]}, r.get::<_,String>(6)?))).unwrap()
    };
    let (id, revision, input, reference) = selected();
    let input = input.validate().unwrap();
    let key = WindowsCredentials::new("local.sayweave.skeleton.release".into())
        .get(&reference)
        .unwrap();
    let provider =
        super::provider::Provider::new(Arc::new(crate::network::RequestCoordinator::default()))
            .unwrap();
    let dictionary = DictionaryIndex::new(vec![]);
    let mut results = vec![];
    for (case, first, later) in [
        (
            "date",
            "周三下午两点开会。",
            "不对，是周四下午两点，地点不变。",
        ),
        (
            "condition",
            "把这个版本发给所有人。",
            "等等，只发给测试组，其他人先别发。",
        ),
        (
            "repeat",
            "请帮我整理昨天的会议记录。",
            "昨天那个会议记录帮我整理一下，然后发给小王。",
        ),
    ] {
        for revisable in [false, true] {
            let mut previous = String::new();
            let mut requests = vec![];
            for (index, part) in [first, later].into_iter().enumerate() {
                let text = if revisable && index == 1 {
                    format!("{first}{later}")
                } else {
                    part.into()
                };
                let built = if revisable {
                    prompt::build_with_context(&text, "", &dictionary, None, &[])
                } else {
                    prompt::build_legacy_with_context(&text, &previous, &dictionary, None, &[])
                }
                .unwrap();
                let request_id = uuid::Uuid::new_v4().to_string();
                let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
                let generated = if revisable {
                    provider
                        .generate_draft_unit(&input, &key, &request_id, &built.messages, deadline)
                        .await
                } else {
                    provider
                        .generate_unit(&input, &key, &request_id, &built.messages, deadline)
                        .await
                };
                match generated {
                    Ok(output) => {
                        if revisable {
                            previous = output.text.clone();
                        } else {
                            previous.push_str(&output.text);
                        }
                        requests.push(json!({"index":index,"input":text,"output":output.text,"messageBytes":built.serialized_bytes,"inputTokens":output.usage.input,"outputTokens":output.usage.output,"firstOutputMs":output.first_output_ms,"requestMs":output.elapsed_ms}));
                    }
                    Err(error) => {
                        requests.push(json!({"index":index,"error":error.code}));
                    }
                }
            }
            results.push(json!({"case":case,"strategy":if revisable {"revisable"} else {"segmented"},"requestCount":requests.len(),"finalBody":previous,"requests":requests}));
        }
    }
    let after = selected();
    assert_eq!(
        (&id, revision, &reference),
        (&after.0, after.1, &after.3),
        "Selected configuration changed during comparison"
    );
    let passed = results.iter().all(|v| {
        v["requests"]
            .as_array()
            .unwrap()
            .iter()
            .all(|r| r.get("error").is_none())
    });
    let report = json!({"passed":passed,"syntheticOnly":true,"readOnlyProfile":true,"configId":id,"configRevision":revision,"modelId":input.model_id,"trace":"two final ASR snapshots; first snapshot then later supplement; same number of requests","scope":"provider latency/output only; no native display or microphone timing","results":results});
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../.local/revisable-draft-live-provider.json");
    std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    assert!(
        passed,
        "Provider comparison includes errors; see stable-code report"
    );
}
