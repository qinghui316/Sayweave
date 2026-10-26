//! Explicit test-only developer handoff. Never linked into a shipping application.
use super::credentials::{Credentials, WindowsCredentials};
use crate::data::{DataWorker, Operation};
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::{collections::BTreeMap, io::Write, path::Path, time::Duration};

const PRESERVED: &[&str] = &[
    "model_configs",
    "model_defaults",
    "dictionary_terms",
    "correction_rules",
    "language_term_sources",
    "language_suppression",
    "language_settings",
    "automatic_term_settings",
    "speech_preferences",
    "shortcut_settings",
    "processing_settings",
];
#[derive(Debug, Serialize)]
struct Receipt {
    source_schema: i64,
    target_schema: i64,
    source_history: i64,
    target_history: i64,
    preserved: BTreeMap<String, i64>,
    credentials_copied: usize,
    source_read_only: bool,
}
fn count(db: &Connection, table: &str) -> Result<i64, &'static str> {
    db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
        .map_err(|_| "profile-count")
}
async fn seed(
    source_file: &Path,
    destination: &Path,
    source_vault: &mut dyn Credentials,
    target_vault: &mut dyn Credentials,
) -> Result<Receipt, &'static str> {
    if !source_file.is_absolute() || !destination.is_absolute() || destination.exists() {
        return Err("profile-target-exists");
    }
    let source = Connection::open_with_flags(source_file, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| "profile-source")?;
    source
        .execute_batch("BEGIN")
        .map_err(|_| "profile-source")?;
    if count(&source, "credential_operations")? != 0 {
        return Err("profile-pending-credentials");
    }
    let source_schema = source
        .query_row("SELECT max(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })
        .map_err(|_| "profile-schema")?;
    if source_schema != 10 && source_schema != 11 {
        return Err("profile-schema");
    }
    let source_history = count(&source, "speech_records")?;
    let preserved = PRESERVED
        .iter()
        .map(|t| Ok((t.to_string(), count(&source, t)?)))
        .collect::<Result<BTreeMap<_, _>, &'static str>>()?;
    let parent = destination.parent().ok_or("profile-target")?;
    let staging = tempfile::Builder::new()
        .prefix(".local-asr-seed-")
        .tempdir_in(parent)
        .map_err(|_| "profile-staging")?;
    let db_file = staging.path().join("dictionary.sqlite");
    {
        let mut target = Connection::open(&db_file).map_err(|_| "profile-staging")?;
        rusqlite::backup::Backup::new(&source, &mut target)
            .map_err(|_| "profile-backup")?
            .run_to_completion(128, Duration::from_millis(1), None)
            .map_err(|_| "profile-backup")?;
        target.execute_batch("PRAGMA foreign_keys=ON; BEGIN;
            DELETE FROM speech_records;
            DELETE FROM language_edits WHERE id NOT IN (SELECT evidence_id FROM language_term_sources WHERE evidence_id IS NOT NULL);
            DELETE FROM language_lexical_candidates;
            UPDATE language_edits SET
                state=CASE WHEN state='pending' THEN 'expired' ELSE state END,
                automatic_terms_state=CASE WHEN automatic_terms_state='pending' THEN 'cancelled' ELSE automatic_terms_state END,
                personalization_state=CASE WHEN personalization_state='pending' THEN 'cancelled' ELSE personalization_state END;
            DELETE FROM language_observations;
            DELETE FROM language_profile_revisions;
            DELETE FROM language_profiles;
            COMMIT; VACUUM;")
            .map_err(|_| "profile-prune")?;
        for table in [
            "speech_records",
            "speech_attempts",
            "audio_assets",
            "language_observations",
        ] {
            if count(&target, table)? != 0 {
                return Err("profile-prune");
            }
        }
    }
    // Only the independent staging database is migrated. The source connection
    // remains read-only throughout, including all credential-copy failures.
    let data = DataWorker::start(staging.path().to_path_buf()).map_err(|_| "profile-migration")?;
    let ready = data
        .request(
            "main",
            Operation::Models(super::types::DbOperation::Overview),
        )
        .await;
    let status = data.status();
    let drained = data.seal_and_drain().await;
    drop(data);
    if ready.is_err()
        || drained.is_err()
        || !status.is_ok_and(|s| s.state == "ready" && s.schema_version == 11)
    {
        return Err("profile-migration");
    }
    let target = Connection::open_with_flags(&db_file, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|_| "profile-verification")?;
    for (table, before) in &preserved {
        if count(&target, table)? != *before {
            return Err("profile-preservation");
        }
    }
    let violations = target
        .prepare("PRAGMA foreign_key_check")
        .map_err(|_| "profile-verification")?
        .exists([])
        .map_err(|_| "profile-verification")?;
    if violations {
        return Err("profile-foreign-key");
    }
    let target_schema = target
        .query_row("SELECT max(version) FROM schema_migrations", [], |r| {
            r.get(0)
        })
        .map_err(|_| "profile-verification")?;
    if target_schema != 11 {
        return Err("profile-migration");
    }
    let refs = target
        .prepare(
            "SELECT DISTINCT credential_ref FROM model_configs WHERE credential_ref IS NOT NULL",
        )
        .map_err(|_| "profile-verification")?
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(|_| "profile-verification")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "profile-verification")?;
    drop(target);
    let mut pending = Vec::new();
    for reference in refs {
        let key = source_vault
            .get(&reference)
            .map_err(|_| "profile-source-credential")?;
        match target_vault.get(&reference) {
            Ok(existing) if existing == key => {}
            Ok(_) => return Err("profile-existing-credential"),
            Err(error) if error.code == "credentialMissing" => pending.push((reference, key)),
            Err(_) => return Err("profile-target-credential"),
        }
    }
    let intent = serde_json::json!({"namespace":"local.sayweave.skeleton.dev","target":destination,"references":pending.iter().map(|(r,_)|r).collect::<Vec<_>>()});
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(staging.path().join("credential-copy-intent.json"))
        .map_err(|_| "profile-credential-intent")?;
    file.write_all(&serde_json::to_vec(&intent).map_err(|_| "profile-credential-intent")?)
        .map_err(|_| "profile-credential-intent")?;
    file.sync_all().map_err(|_| "profile-credential-intent")?;
    drop(file);
    let mut created = Vec::new();
    let result = (|| {
        for (reference, key) in &pending {
            created.push(reference.clone());
            target_vault
                .put(reference, key)
                .map_err(|_| "profile-credential-copy")?;
        }
        std::fs::rename(staging.path(), destination).map_err(|_| "profile-publish")
    })();
    if let Err(code) = result {
        let unresolved = created
            .into_iter()
            .filter(|r| target_vault.delete(r).is_err())
            .collect::<Vec<_>>();
        if !unresolved.is_empty() {
            // Keep the pre-synced intent even if the secondary failure receipt
            // cannot be written (e.g. a full disk). It lists only newly owned
            // dev references, never key values or pre-existing credentials.
            let kept = staging.keep();
            let failure = serde_json::json!({"cause":code,"unresolved":unresolved});
            // Emit the cause before any fallible recovery-file write, so a
            // full disk cannot replace the original error with only a path.
            eprintln!("developer_profile_cleanup_failure={failure}");
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(kept.join("credential-copy-failure.json"))
                && let Ok(bytes) = serde_json::to_vec(&failure)
            {
                let _ = file.write_all(&bytes);
                let _ = file.sync_all();
            }
            eprintln!("developer_profile_recovery={}", kept.display());
            return Err("profile-credential-cleanup");
        }
        return Err(code);
    }
    let _ = std::fs::remove_file(destination.join("credential-copy-intent.json"));
    Ok(Receipt {
        source_schema,
        target_schema,
        source_history,
        target_history: 0,
        preserved,
        credentials_copied: pending.len(),
        source_read_only: true,
    })
}

#[tokio::test]
#[ignore = "explicit user-approved independent developer profile; native context only"]
async fn seed_independent_developer_profile() {
    assert_eq!(
        std::env::var("SAYWEAVE_LOCAL_DEVELOPER_SEED").as_deref(),
        Ok("copy-settings-and-dictionary")
    );
    assert!(std::env::var_os("SAYWEAVE_DATA_ACCEPTANCE_ID").is_none());
    let base =
        std::path::PathBuf::from(std::env::var("APPDATA").unwrap()).join("local.sayweave.skeleton");
    let source = base.join("data/dictionary.sqlite");
    let db = Connection::open_with_flags(&source, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert!(
        count(&db, "speech_records").unwrap() > 0,
        "Refuse a redirected empty profile"
    );
    drop(db);
    let receipt = seed(
        &source,
        &base.join("data-dev"),
        &mut WindowsCredentials::new("local.sayweave.skeleton.release".into()),
        &mut WindowsCredentials::new("local.sayweave.skeleton.dev".into()),
    )
    .await
    .unwrap();
    let report = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.local/local-asr/reports/developer-profile-seed.json");
    std::fs::write(report, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
}

#[derive(Default)]
struct MemoryVault {
    values: BTreeMap<String, String>,
    fail: Option<String>,
    delete_fail: Option<String>,
    deletions: Vec<String>,
}
impl Credentials for MemoryVault {
    fn get(&mut self, r: &str) -> Result<String, super::types::ModelError> {
        self.values
            .get(r)
            .cloned()
            .ok_or(super::types::ModelError::new("credentialMissing"))
    }
    fn put(&mut self, r: &str, k: &str) -> Result<(), super::types::ModelError> {
        if self.fail.as_deref() == Some(r) {
            return Err(super::types::ModelError::new("credentialUnavailable"));
        }
        self.values.insert(r.into(), k.into());
        Ok(())
    }
    fn delete(&mut self, r: &str) -> Result<(), super::types::ModelError> {
        self.deletions.push(r.into());
        if self.delete_fail.as_deref() == Some(r) {
            return Err(super::types::ModelError::new("credentialUnavailable"));
        }
        self.values.remove(r);
        Ok(())
    }
}
async fn fixture() -> (tempfile::TempDir, std::path::PathBuf, MemoryVault) {
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("source");
    let data = DataWorker::start(path.clone()).unwrap();
    data.shutdown().await;
    drop(data);
    let db = Connection::open(path.join("dictionary.sqlite")).unwrap();
    for id in ["one", "two"] {
        db.execute("INSERT INTO model_configs(id,name,protocol,base_url,model_id,credential_ref,speech,text,revision,created_at,updated_at) VALUES(?1,?1,'qwen-audio-streaming','https://example.com','fixture',?1,1,0,1,1,1)",[id]).unwrap();
    }
    db.execute_batch("INSERT INTO dictionary_terms(id,term,normalized_term,revision,created_at,updated_at) VALUES('word','SQLite','sqlite',1,1,1); INSERT INTO speech_records(id,created_at,status) VALUES('voice',1,'success'); UPDATE model_defaults SET config_id='one' WHERE usage='speech';").unwrap();
    db.execute_batch("INSERT INTO language_edits(id,task_id,scope,settings_revision,evidence,state,created_at,expires_at) VALUES('proof','past','global',1,'{}','settled',1,2),('pending','past','global',1,'{}','pending',1,2); INSERT INTO language_term_sources(term_id,source,evidence_id,settings_revision,created_at) VALUES('word','automatic','proof',1,1);").unwrap();
    drop(db);
    (
        folder,
        path.join("dictionary.sqlite"),
        MemoryVault {
            values: BTreeMap::from([
                ("one".into(), "first".into()),
                ("two".into(), "second".into()),
            ]),
            fail: None,
            ..Default::default()
        },
    )
}
#[tokio::test]
async fn developer_seed_preserves_settings_and_words_without_history_or_shared_credentials() {
    let (folder, source, mut original) = fixture().await;
    let original_db = Connection::open(&source).unwrap();
    original_db.execute_batch("UPDATE language_edits SET automatic_terms_state='settled',personalization_state='pending' WHERE id='proof';").unwrap();
    drop(original_db);
    let dest = folder.path().join("dev");
    let mut independent = MemoryVault::default();
    let receipt = seed(&source, &dest, &mut original, &mut independent)
        .await
        .unwrap();
    assert_eq!(receipt.target_history, 0);
    assert_eq!(receipt.source_history, 1);
    assert_eq!(receipt.preserved["dictionary_terms"], 1);
    assert_eq!(receipt.preserved["model_configs"], 2);
    assert_eq!(receipt.target_schema, 11);
    let copied = Connection::open_with_flags(
        dest.join("dictionary.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    assert_eq!(count(&copied, "language_edits").unwrap(), 1);
    assert_eq!(
        copied
            .query_row(
                "SELECT personalization_state FROM language_edits WHERE id='proof'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "cancelled"
    );
    assert_eq!(
        copied
            .query_row(
                "SELECT evidence_id FROM language_term_sources WHERE term_id='word'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
        "proof"
    );
    assert_eq!(
        copied
            .query_row(
                "SELECT count(*) FROM language_edits WHERE state='pending'",
                [],
                |r| r.get::<_, i64>(0)
            )
            .unwrap(),
        0
    );
    drop(copied);
    independent.delete("one").unwrap();
    assert_eq!(original.get("one").unwrap(), "first");
    let db = Connection::open_with_flags(source, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    assert_eq!(count(&db, "speech_records").unwrap(), 1);
    assert_eq!(count(&db, "language_edits").unwrap(), 2);
    let protected = dest.join("keep.txt");
    std::fs::write(&protected, b"keep").unwrap();
    assert_eq!(
        seed(
            &folder.path().join("missing"),
            &dest,
            &mut original,
            &mut independent
        )
        .await
        .unwrap_err(),
        "profile-target-exists"
    );
    assert_eq!(std::fs::read(protected).unwrap(), b"keep");
}
#[tokio::test]
async fn developer_seed_does_not_publish_or_leave_copied_keys_when_vault_write_fails() {
    let (folder, source, mut original) = fixture().await;
    let dest = folder.path().join("dev");
    let mut target = MemoryVault {
        fail: Some("two".into()),
        ..Default::default()
    };
    assert_eq!(
        seed(&source, &dest, &mut original, &mut target)
            .await
            .unwrap_err(),
        "profile-credential-copy"
    );
    assert!(!dest.exists());
    assert!(target.values.is_empty());
    assert_eq!(original.values.len(), 2);
}
#[tokio::test]
async fn developer_seed_rejects_unready_migration_before_any_key_copy() {
    let (folder, source, mut original) = fixture().await;
    let db = Connection::open(&source).unwrap();
    db.execute(
        "UPDATE schema_migrations SET checksum='broken' WHERE version=1",
        [],
    )
    .unwrap();
    drop(db);
    let dest = folder.path().join("dev");
    let mut target = MemoryVault::default();
    assert_eq!(
        seed(&source, &dest, &mut original, &mut target)
            .await
            .unwrap_err(),
        "profile-migration"
    );
    assert!(!dest.exists());
    assert!(target.values.is_empty());
}
#[tokio::test]
async fn developer_seed_attempts_all_rollback_and_retains_nonsecret_recovery() {
    let (folder, source, mut original) = fixture().await;
    let dest = folder.path().join("dev");
    let mut target = MemoryVault {
        fail: Some("two".into()),
        delete_fail: Some("one".into()),
        ..Default::default()
    };
    assert_eq!(
        seed(&source, &dest, &mut original, &mut target)
            .await
            .unwrap_err(),
        "profile-credential-cleanup"
    );
    assert_eq!(target.deletions, vec!["one", "two"]);
    assert!(!dest.exists());
    let recovery = std::fs::read_dir(folder.path())
        .unwrap()
        .map(|p| p.unwrap().path())
        .find(|p| p.join("credential-copy-intent.json").is_file())
        .unwrap();
    let intent = std::fs::read_to_string(recovery.join("credential-copy-intent.json")).unwrap();
    assert!(intent.contains("one") && intent.contains("two"));
    assert!(!intent.contains("first") && !intent.contains("second"));
    let failure: serde_json::Value = serde_json::from_slice(
        &std::fs::read(recovery.join("credential-copy-failure.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(failure["cause"], "profile-credential-copy");
    assert_eq!(failure["unresolved"], serde_json::json!(["one"]));
    assert_eq!(original.values.len(), 2);
}
