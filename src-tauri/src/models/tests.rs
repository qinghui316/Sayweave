use super::*;
use credentials::Credentials;

#[cfg(target_os = "windows")]
#[tokio::test]
#[ignore = "cleanup only explicit credentials from a UUID-scoped native acceptance profile"]
async fn model_desktop_cleanup() {
    let id = std::env::var("SAYWEAVE_DATA_ACCEPTANCE_ID").unwrap();
    let id = uuid::Uuid::parse_str(&id).unwrap().to_string();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.local/data-acceptance")
        .join(&id);
    let mut refs = std::collections::BTreeSet::new();
    // Retire remnants from the earlier internal-channel fixture layout as well.
    for profile in ["data", "data-dev"] {
        let file = root.join(profile).join("dictionary.sqlite");
        if !file.exists() {
            continue;
        }
        let c =
            rusqlite::Connection::open_with_flags(file, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
                .unwrap();
        let mut q = c.prepare("SELECT credential_ref FROM model_configs WHERE credential_ref IS NOT NULL UNION SELECT credential_ref FROM credential_operations").unwrap();
        for reference in q.query_map([], |r| r.get::<_, String>(0)).unwrap() {
            let reference = reference.unwrap();
            uuid::Uuid::parse_str(&reference).unwrap();
            refs.insert(reference);
        }
    }
    let vault = Vault::new(WindowsCredentials::new(format!(
        "local.sayweave.skeleton.acceptance.{id}"
    )));
    let count = refs.len();
    let mut failures = Vec::new();
    for reference in refs {
        if let Err(error) = vault.delete(reference.clone()).await {
            failures.push(error.code);
            continue;
        }
        match vault.get(reference).await {
            Err(error) if error.code == "credentialMissing" => {}
            _ => failures.push("credentialCleanupUnconfirmed".into()),
        }
    }
    assert!(
        failures.is_empty(),
        "fixture credential cleanup failed: {failures:?}"
    );
    eprintln!("SAYWEAVE_FIXTURE_CREDENTIAL_CLEANUP checked={count} missing_verified={count}");
}

#[test]
#[ignore = "explicit isolated native HTTPS fixture generation"]
fn model_desktop_fixture() {
    let id = std::env::var("SAYWEAVE_DATA_ACCEPTANCE_ID").unwrap();
    let id = uuid::Uuid::parse_str(&id).unwrap().to_string();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.local/data-acceptance")
        .join(id)
        .join("data");
    std::fs::create_dir_all(&root).unwrap();
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    std::fs::write(root.join("fixture-ca.der"), cert.cert.der()).unwrap();
    std::fs::write(root.join("fixture-ca.pem"), cert.cert.pem()).unwrap();
    std::fs::write(
        root.join("fixture-key.pem"),
        cert.signing_key.serialize_pem(),
    )
    .unwrap();
}
use std::{collections::HashMap, sync::Mutex as SyncMutex};
type AfterPut = Arc<SyncMutex<Option<Box<dyn FnOnce() + Send>>>>;
#[derive(Clone, Default)]
struct FakeVault {
    values: Arc<SyncMutex<HashMap<String, String>>>,
    fail_put: Arc<AtomicBool>,
    fail_delete: Arc<AtomicBool>,
    after_put: AfterPut,
}
impl Credentials for FakeVault {
    fn get(&mut self, r: &str) -> Result<String, ModelError> {
        self.values
            .lock()
            .unwrap()
            .get(r)
            .cloned()
            .ok_or(ModelError::new("credentialMissing"))
    }
    fn put(&mut self, r: &str, v: &str) -> Result<(), ModelError> {
        if self.fail_put.load(Ordering::Acquire) {
            return Err(ModelError::new("credentialUnavailable"));
        }
        self.values.lock().unwrap().insert(r.into(), v.into());
        if let Some(action) = self.after_put.lock().unwrap().take() {
            action();
        }
        Ok(())
    }
    fn delete(&mut self, r: &str) -> Result<(), ModelError> {
        if self.fail_delete.load(Ordering::Acquire) {
            return Err(ModelError::new("credentialUnavailable"));
        }
        self.values.lock().unwrap().remove(r);
        Ok(())
    }
}
fn input(name: &str, usages: Vec<Usage>) -> ConfigInput {
    ConfigInput {
        name: name.into(),
        protocol: "openai-chat".into(),
        base_url: "https://example.invalid/v1".into(),
        model_id: "example-model".into(),
        usages,
    }
}
#[tokio::test]
async fn second_registered_asset_uses_the_same_model_selection_and_frozen_asr_contract() {
    use sha2::{Digest, Sha256};
    let (root, service, vault) = service().await;
    let definitions = Box::leak(
        vec![local::AsrDefinition {
            asset_id: "fixture-second-asr",
            engine: "transcribe-cpp-0.1.3",
            right_context: 5,
        }]
        .into_boxed_slice(),
    );
    let descriptors = Box::leak(
        vec![crate::model_assets::catalog::Descriptor {
            id: "fixture-second-asr",
            file: "second.gguf",
            bytes: 7,
            sha256: Box::leak(format!("{:x}", Sha256::digest(b"fixture")).into_boxed_str()),
            sources: &[],
        }]
        .into_boxed_slice(),
    );
    std::fs::write(root.path().join("second.gguf"), b"fixture").unwrap();
    let assets =
        crate::model_assets::Assets::with_catalog(root.path().to_owned(), descriptors).unwrap();
    assert!(
        service
            .local
            .set(local::LocalServices {
                runtime: crate::local_asr::Runtime::new(
                    root.path().join("unstarted-worker.exe"),
                    vec![]
                ),
                assets,
                definitions,
            })
            .is_ok()
    );
    let view = service
        .save_local(
            "main",
            LocalSaveRequest {
                input: LocalConfigInput {
                    name: "Second adapter fixture".into(),
                    asset_id: "fixture-second-asr".into(),
                    engine: "transcribe-cpp-0.1.3".into(),
                    device: sayweave_local_asr_protocol::DevicePolicy::Cpu,
                },
                id: None,
                revision: None,
            },
        )
        .await
        .unwrap();
    let view = service
        .set_default(
            "main",
            DefaultRequest {
                usage: Usage::Speech,
                config_id: view.local_configs[0].id.clone(),
                revision: 1,
            },
        )
        .await
        .unwrap();
    let frozen = service
        .speech_choice()
        .await
        .unwrap()
        .configuration
        .unwrap();
    assert_eq!(frozen.id(), view.local_configs[0].id);
    let local::SpeechConfiguration::Local(config) = frozen else {
        panic!("expected local ASR");
    };
    assert_eq!(config.record.input.asset_id, "fixture-second-asr");
    assert_eq!(config.settings.right_context, 5);
    assert!(vault.values.lock().unwrap().is_empty());
    drop(config);
    service.finish_shutdown().await.unwrap();
}
#[tokio::test]
async fn local_configuration_has_no_vault_or_transport_and_shares_guarded_defaults() {
    let (root, service, vault) = service().await;
    let _ = service
        .local
        .set(local::LocalServices::new(root.path()).unwrap());
    let local = LocalConfigInput {
        name: "本地识别".into(),
        asset_id: crate::model_assets::MODEL_ID.into(),
        engine: "transcribe-cpp-0.1.3".into(),
        device: sayweave_local_asr_protocol::DevicePolicy::Cpu,
    };
    assert!(serde_json::from_value::<LocalConfigInput>(serde_json::json!({"name":"local","assetId":local.asset_id,"engine":local.engine,"device":"cpu","baseUrl":"https://example.com"})).is_err());
    let view = service
        .save_local(
            "main",
            LocalSaveRequest {
                input: local.clone(),
                id: None,
                revision: None,
            },
        )
        .await
        .unwrap();
    assert!(view.configs.is_empty());
    assert!(view.defaults.iter().all(|d| d.config_id.is_none()));
    assert!(vault.values.lock().unwrap().is_empty());
    let record = &view.local_configs[0];
    let json = serde_json::to_value(record).unwrap();
    assert!(json.get("baseUrl").is_none());
    assert!(json.get("credentialStatus").is_none());
    assert!(
        service
            .save_local(
                "voice-overlay",
                LocalSaveRequest {
                    input: local.clone(),
                    id: Some(record.id.clone()),
                    revision: Some(record.revision)
                }
            )
            .await
            .is_err()
    );
    service
        .db(DbOperation::SetDefault(DefaultRequest {
            usage: Usage::Speech,
            config_id: record.id.clone(),
            revision: 1,
        }))
        .await
        .unwrap();
    assert!(
        service
            .db(DbOperation::SetDefault(DefaultRequest {
                usage: Usage::Text,
                config_id: record.id.clone(),
                revision: 1
            }))
            .await
            .is_err()
    );
    assert_eq!(
        service
            .save_local(
                "main",
                LocalSaveRequest {
                    input: local,
                    id: Some(record.id.clone()),
                    revision: Some(99)
                }
            )
            .await
            .err()
            .unwrap()
            .code,
        "conflict"
    );
    let before = service.stored().await.unwrap();
    service
        .remove(
            "main",
            RemoveRequest {
                id: record.id.clone(),
                revision: record.revision,
                expectation: Expectation {
                    defaults: before.defaults.clone(),
                    replacements: replacements(&before, &record.id, &[]),
                },
            },
        )
        .await
        .unwrap();
    assert!(
        service
            .stored()
            .await
            .unwrap()
            .defaults
            .iter()
            .all(|d| d.config_id.is_none())
    );
    assert!(vault.values.lock().unwrap().is_empty());
}

#[test]
fn schema10_upgrade_preserves_cloud_identity_and_rejects_mixed_local_secrets() {
    use crate::data::repository::{MIGRATIONS, configure, migrate};
    let root = tempfile::tempdir().unwrap();
    let mut connection = rusqlite::Connection::open(root.path().join("dictionary.sqlite")).unwrap();
    configure(&connection).unwrap();
    migrate(&mut connection, root.path(), &MIGRATIONS[..10]).unwrap();
    connection.execute("INSERT INTO model_configs VALUES('existing','cloud','qwen-audio-streaming','https://example.com','model',1,0,'credential-ref',7,100,200)",[]).unwrap();
    connection
        .execute(
            "UPDATE model_defaults SET config_id='existing',revision=9 WHERE usage='speech'",
            [],
        )
        .unwrap();
    migrate(&mut connection, root.path(), MIGRATIONS).unwrap();
    let state:(String,String,i64,i64)=connection.query_row("SELECT c.id,c.credential_ref,c.revision,d.revision FROM model_configs c JOIN model_defaults d ON d.config_id=c.id WHERE d.usage='speech'",[],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap();
    assert_eq!(state, ("existing".into(), "credential-ref".into(), 7, 9));
    assert!(connection.execute("INSERT INTO model_configs(id,name,speech,text,revision,created_at,updated_at,source_kind,asset_id,engine,device,credential_ref) VALUES('bad','bad',1,0,1,1,1,'local','model','engine','cpu','secret')",[]).is_err());
    assert_eq!(
        connection
            .query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        0
    );
}
fn request(name: &str, usages: Vec<Usage>) -> SaveRequest {
    SaveRequest {
        input: input(name, usages),
        key: KeyChange::Replace {
            value: "test-secret-not-a-provider-key".into(),
        },
        id: None,
        revision: None,
        expectation: None,
    }
}
async fn service() -> (tempfile::TempDir, ModelService, FakeVault) {
    let root = tempfile::tempdir().unwrap();
    let data = DataWorker::start(root.path().to_path_buf()).unwrap();
    let fake = FakeVault::default();
    let service = ModelService::with_vault(
        data,
        Vault::new(fake.clone()),
        Arc::new(Provider::new(Arc::new(crate::network::RequestCoordinator::default())).unwrap()),
    )
    .unwrap();
    assert_eq!(service.status("main", true).await.unwrap().state, "ready");
    (root, service, fake)
}
async fn expect(service: &ModelService, id: &str, usages: &[Usage]) -> Expectation {
    let view = service.stored().await.unwrap();
    Expectation {
        defaults: view.defaults.clone(),
        replacements: replacements(&view, id, usages),
    }
}
#[test]
fn fields_url_origin_and_secret_limits() {
    assert_eq!(
        input("  名称  ", vec![Usage::Text])
            .validate()
            .unwrap()
            .name,
        "名称"
    );
    for url in [
        "http://localhost/v1",
        "https://u:p@example.com",
        "https://example.com?q=x",
        "https://example.com/#x",
        "/relative",
    ] {
        assert!(base_url(url).is_err());
    }
    assert_eq!(
        base_url(" https://example.com/api/v2/// ").unwrap(),
        "https://example.com/api/v2"
    );
    assert!(same_origin(
        "https://EXAMPLE.com:443/v1",
        "https://example.com/v2"
    ));
    assert!(!same_origin(
        "https://example.com/v1",
        "https://other.example/v1"
    ));
    assert!(!same_origin(
        "https://example.com:444",
        "https://example.com"
    ));
    assert!(secret("key\r\nInjected: yes").is_err());
    assert!(secret(&"a".repeat(1025)).is_err());
    for value in ["a\n", "a\0b", "a\u{2028}b", ""] {
        assert!(input(value, vec![Usage::Text]).validate().is_err());
    }
    assert!(
        input(&"字".repeat(64), vec![Usage::Text])
            .validate()
            .is_ok()
    );
    assert!(
        input(&"字".repeat(65), vec![Usage::Text])
            .validate()
            .is_err()
    );
}
#[tokio::test]
async fn defaults_independent_stable_replacement_and_revision_guards() {
    let (_root, s, _) = service().await;
    let one = s
        .save("main", request("same", vec![Usage::Speech, Usage::Text]))
        .await
        .unwrap();
    let id = one.configs[0].id.clone();
    assert!(one.defaults.iter().all(|d| d.config_id.as_deref()
        == if d.usage == Usage::Text {
            Some(&id)
        } else {
            None
        }));
    let two = s
        .save("main", request("same", vec![Usage::Text]))
        .await
        .unwrap();
    let next = two.configs.iter().find(|c| c.id != id).unwrap().id.clone();
    assert!(two.defaults.iter().all(|d| d.config_id.as_deref()
        == if d.usage == Usage::Text {
            Some(&id)
        } else {
            None
        }));
    let expected = expect(&s, &id, &[]).await;
    assert_eq!(expected.replacements.len(), 1);
    assert_eq!(expected.replacements[0].usage, Usage::Text);
    assert_eq!(expected.replacements[0].config_id, Some(next.clone()));
    let d = two
        .defaults
        .iter()
        .find(|d| d.usage == Usage::Text)
        .unwrap();
    s.set_default(
        "main",
        DefaultRequest {
            usage: Usage::Text,
            config_id: next.clone(),
            revision: d.revision,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        s.remove(
            "main",
            RemoveRequest {
                id: id.clone(),
                revision: 1,
                expectation: expected
            }
        )
        .await
        .err()
        .unwrap()
        .code,
        "conflict"
    );
    let result = s
        .remove(
            "main",
            RemoveRequest {
                id: id.clone(),
                revision: 1,
                expectation: expect(&s, &id, &[]).await,
            },
        )
        .await
        .unwrap();
    assert_eq!(result.defaults[0].config_id, None);
    assert_eq!(result.defaults[1].config_id, Some(next.clone()));
    assert_eq!(
        s.remove(
            "main",
            RemoveRequest {
                id,
                revision: 1,
                expectation: expect(&s, &next, &[]).await
            }
        )
        .await
        .err()
        .unwrap()
        .code,
        "notFound"
    );
}
#[tokio::test]
async fn origin_binding_update_and_secret_redaction() {
    let (root, s, fake) = service().await;
    let view = s
        .save("main", request("one", vec![Usage::Text]))
        .await
        .unwrap();
    let id = view.configs[0].id.clone();
    let mut req = SaveRequest {
        input: input("two", vec![Usage::Text]),
        key: KeyChange::Keep,
        id: Some(id.clone()),
        revision: Some(1),
        expectation: Some(expect(&s, &id, &[Usage::Text]).await),
    };
    req.input.base_url = "https://other.invalid/v1".into();
    assert_eq!(
        s.save("main", req.clone()).await.err().unwrap().code,
        "originChanged"
    );
    assert_eq!(
        s.draft(
            "main",
            NetworkDraft {
                input: req.input.clone(),
                key: KeyChange::Keep,
                id: Some(id.clone()),
                revision: Some(1)
            }
        )
        .await
        .err()
        .unwrap()
        .code,
        "originChanged"
    );
    req.input.base_url = "https://example.invalid/v2".into();
    let result = s.save("main", req.clone()).await.unwrap();
    assert_eq!(result.configs[0].revision, 2);
    assert_eq!(s.save("main", req).await.err().unwrap().code, "conflict");
    let json = serde_json::to_string(&result).unwrap();
    assert!(!json.contains("test-secret"));
    assert!(!json.contains("credentialRef"));
    assert!(json.contains("present"));
    let db = std::fs::read(root.path().join("dictionary.sqlite")).unwrap();
    assert!(
        !db.windows(b"test-secret".len())
            .any(|s| s == b"test-secret")
    );
    assert_eq!(fake.values.lock().unwrap().len(), 1);
}
#[tokio::test]
async fn save_failures_cleanup_and_recovery_are_durable() {
    let (_root, s, fake) = service().await;
    fake.fail_put.store(true, Ordering::Release);
    assert_eq!(
        s.save("main", request("fail", vec![Usage::Text]))
            .await
            .err()
            .unwrap()
            .code,
        "credentialUnavailable"
    );
    assert!(s.overview("main").await.unwrap().configs.is_empty());
    fake.fail_put.store(false, Ordering::Release);
    let view = s
        .save("main", request("one", vec![Usage::Text]))
        .await
        .unwrap();
    let id = view.configs[0].id.clone();
    fake.fail_delete.store(true, Ordering::Release);
    let result = s
        .save(
            "main",
            SaveRequest {
                input: input("updated", vec![Usage::Text]),
                key: KeyChange::Replace {
                    value: "second-secret".into(),
                },
                id: Some(id.clone()),
                revision: Some(1),
                expectation: Some(expect(&s, &id, &[Usage::Text]).await),
            },
        )
        .await
        .unwrap();
    assert!(result.cleanup_pending);
    assert_eq!(fake.values.lock().unwrap().len(), 2);
    // Simulate a process stopping after vault write, before commit.
    let orphan = uuid::Uuid::new_v4().to_string();
    s.db(DbOperation::Prepare(orphan.clone())).await.unwrap();
    s.vault
        .put(orphan.clone(), "interrupted-secret".into())
        .await
        .unwrap();
    fake.fail_delete.store(false, Ordering::Release);
    s.status("main", true).await.unwrap();
    assert!(!fake.values.lock().unwrap().contains_key(&orphan));
    assert_eq!(fake.values.lock().unwrap().len(), 1);
    // A pre-existing stale revision is rejected before a new credential is written.
    let stale = s
        .save(
            "main",
            SaveRequest {
                input: input("stale", vec![Usage::Text]),
                key: KeyChange::Replace {
                    value: "third-secret".into(),
                },
                id: Some(id.clone()),
                revision: Some(1),
                expectation: Some(expect(&s, &id, &[Usage::Text]).await),
            },
        )
        .await;
    assert_eq!(stale.err().unwrap().code, "conflict");
    assert_eq!(
        s.overview("main").await.unwrap().configs[0].input.name,
        "updated"
    );
}

#[tokio::test]
async fn conflict_after_vault_write_preserves_old_record_and_cleans_new_secret() {
    let (root, s, fake) = service().await;
    let view = s
        .save("main", request("original", vec![Usage::Text]))
        .await
        .unwrap();
    let id = view.configs[0].id.clone();
    let expected = expect(&s, &id, &[Usage::Text]).await;
    let file = root.path().join("dictionary.sqlite");
    *fake.after_put.lock().unwrap() = Some(Box::new(move || {
        let c = rusqlite::Connection::open(file).unwrap();
        c.execute("UPDATE model_configs SET revision=revision+1", [])
            .unwrap();
    }));
    let result = s
        .save(
            "main",
            SaveRequest {
                input: input("uncommitted", vec![Usage::Text]),
                key: KeyChange::Replace {
                    value: "replacement-fixture".into(),
                },
                id: Some(id),
                revision: Some(1),
                expectation: Some(expected),
            },
        )
        .await;
    assert_eq!(result.err().unwrap().code, "conflict");
    assert_eq!(fake.values.lock().unwrap().len(), 1);
    assert_eq!(
        s.overview("main").await.unwrap().configs[0].input.name,
        "original"
    );
    assert!(
        matches!(s.db(DbOperation::Pending).await.unwrap(), DbOutput::Pending(v) if v.is_empty())
    );
}

#[cfg(target_os = "windows")]
#[tokio::test]
#[ignore = "child process interrupted at explicit durable journal stages"]
async fn model_crash_fixture() {
    let id = std::env::var("SAYWEAVE_CRASH_TEST_ID").unwrap();
    uuid::Uuid::parse_str(&id).unwrap();
    let root = std::path::PathBuf::from(std::env::var("SAYWEAVE_CRASH_TEST_ROOT").unwrap());
    let phase = std::env::var("SAYWEAVE_CRASH_TEST_PHASE").unwrap();
    assert!(["prepared", "written", "committed"].contains(&phase.as_str()));
    let vault = Vault::new(WindowsCredentials::new(format!(
        "local.sayweave.skeleton.test.{id}"
    )));
    let s = ModelService::with_vault(
        DataWorker::start(root.clone()).unwrap(),
        vault,
        Arc::new(Provider::new(Arc::new(crate::network::RequestCoordinator::default())).unwrap()),
    )
    .unwrap();
    s.status("main", true).await.unwrap();
    let view = s
        .save("main", request("old", vec![Usage::Text]))
        .await
        .unwrap();
    let old = s.stored().await.unwrap().configs[0].credential_ref.clone();
    let fresh = uuid::Uuid::new_v4().to_string();
    std::fs::write(
        root.join("references.json"),
        serde_json::to_vec(&[old, fresh.clone()]).unwrap(),
    )
    .unwrap();
    s.db(DbOperation::Prepare(fresh.clone())).await.unwrap();
    if phase == "prepared" {
        std::process::exit(0);
    }
    s.vault
        .put(fresh.clone(), "interrupted-fixture-new-key".into())
        .await
        .unwrap();
    if phase == "written" {
        std::process::exit(0);
    }
    let id = view.configs[0].id.clone();
    s.db(DbOperation::Commit {
        input: input("new", vec![Usage::Text]),
        id: Some(id.clone()),
        revision: Some(1),
        expectation: Some(expect(&s, &id, &[Usage::Text]).await),
        credential_ref: fresh,
    })
    .await
    .unwrap();
    std::process::exit(0);
}

#[cfg(target_os = "windows")]
#[tokio::test]
async fn actual_process_interruption_recovers_each_journal_stage() {
    use std::os::windows::process::CommandExt;
    for phase in ["prepared", "written", "committed"] {
        let root = tempfile::tempdir().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let exit = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["models::tests::model_crash_fixture", "--exact", "--ignored"])
            .env("SAYWEAVE_CRASH_TEST_ID", &id)
            .env("SAYWEAVE_CRASH_TEST_ROOT", root.path())
            .env("SAYWEAVE_CRASH_TEST_PHASE", phase)
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(exit.status.success());
        let refs: Vec<String> =
            serde_json::from_slice(&std::fs::read(root.path().join("references.json")).unwrap())
                .unwrap();
        let vault = Vault::new(WindowsCredentials::new(format!(
            "local.sayweave.skeleton.test.{id}"
        )));
        let s = ModelService::with_vault(
            DataWorker::start(root.path().to_path_buf()).unwrap(),
            vault,
            Arc::new(
                Provider::new(Arc::new(crate::network::RequestCoordinator::default())).unwrap(),
            ),
        )
        .unwrap();
        let recovered = s.status("main", true).await;
        let overview = s.overview("main").await;
        let old_exists = s.vault.get(refs[0].clone()).await.is_ok();
        let new_exists = s.vault.get(refs[1].clone()).await.is_ok();
        for reference in refs {
            s.vault.delete(reference).await.unwrap();
        }
        recovered.unwrap();
        let view = overview.unwrap();
        assert_eq!(
            view.configs[0].input.name,
            if phase == "committed" { "new" } else { "old" }
        );
        assert_eq!(
            (old_exists, new_exists),
            if phase == "committed" {
                (false, true)
            } else {
                (true, false)
            }
        );
        assert!(!view.cleanup_pending);
    }
}
#[tokio::test]
async fn main_only_queue_shutdown_and_missing_credential() {
    let (_root, s, fake) = service().await;
    assert_eq!(
        s.status("voice-overlay", false).await.err().unwrap().code,
        "forbidden"
    );
    assert_eq!(
        s.save("voice-overlay", request("no", vec![Usage::Text]))
            .await
            .err()
            .unwrap()
            .code,
        "forbidden"
    );
    let view = s
        .save("main", request("one", vec![Usage::Text]))
        .await
        .unwrap();
    fake.values.lock().unwrap().clear();
    assert_eq!(
        s.overview("main").await.unwrap().configs[0].credential_status,
        "missing"
    );
    assert_eq!(
        s.draft(
            "main",
            NetworkDraft {
                input: input("one", vec![Usage::Text]),
                key: KeyChange::Keep,
                id: Some(view.configs[0].id.clone()),
                revision: Some(1)
            }
        )
        .await
        .err()
        .unwrap()
        .code,
        "credentialMissing"
    );
    s.begin_shutdown();
    s.finish_shutdown().await.unwrap();
    assert_eq!(
        s.save("main", request("no", vec![Usage::Text]))
            .await
            .err()
            .unwrap()
            .code,
        "stopping"
    );
}
#[cfg(target_os = "windows")]
#[tokio::test]
async fn real_windows_vault_isolated_roundtrip() {
    let namespace = format!("local.sayweave.skeleton.test.{}", uuid::Uuid::new_v4());
    let vault = Vault::new(WindowsCredentials::new(namespace));
    let reference = uuid::Uuid::new_v4().to_string();
    let result = async {
        vault
            .put(
                reference.clone(),
                "isolated-fixture-not-provider-key".into(),
            )
            .await?;
        assert_eq!(
            vault.get(reference.clone()).await?,
            "isolated-fixture-not-provider-key"
        );
        Ok::<_, ModelError>(())
    }
    .await;
    let cleanup = vault.delete(reference.clone()).await;
    result.unwrap();
    cleanup.unwrap();
    assert_eq!(
        vault.get(reference).await.err().unwrap().code,
        "credentialMissing"
    );
}
#[test]
fn actual_001_upgrade_backup_contains_no_new_tables() {
    use crate::data::repository::{MIGRATIONS, Repository, configure, migrate};
    let root = tempfile::tempdir().unwrap();
    let mut c = rusqlite::Connection::open(root.path().join("dictionary.sqlite")).unwrap();
    migrate(&mut c, root.path(), &MIGRATIONS[..1]).unwrap();
    configure(&c).unwrap();
    drop(c);
    let r = Repository::open(root.path()).unwrap();
    assert_eq!(
        r.connection
            .query_row("SELECT max(version) FROM schema_migrations", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        MIGRATIONS.len() as i64
    );
    let backup = std::fs::read_dir(root.path().join("backups"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let c = rusqlite::Connection::open(backup).unwrap();
    assert_eq!(
        c.query_row("SELECT max(version) FROM schema_migrations", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn protocol_capabilities_match_frontend_contract_samples() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../tests/contracts/model-capabilities.json"
    ))
    .unwrap();
    for (protocol, expected) in fixture.as_object().unwrap() {
        let capability = types::capabilities(protocol).unwrap();
        assert_eq!(
            serde_json::json!({"usages":capability.usages,"liveText":capability.live_text,"textTest":capability.text_test,"originOnly":capability.origin_only}),
            *expected
        );
        for usage in [Usage::Speech, Usage::Text] {
            let mut input = input("fixture", vec![usage.clone()]);
            input.protocol = protocol.clone();
            input.usages = vec![usage.clone()];
            assert_eq!(
                types::executable(&input, &usage),
                capability.usages.contains(&usage)
            );
        }
    }
    assert!(types::capabilities("unknown").is_none());
}
