//! Explicit test-only developer handoff. Never linked into a shipping application.
use super::credentials::{Credentials, WindowsCredentials};
use crate::data::DataWorker;
use rusqlite::{Connection, OpenFlags};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path, time::Duration};

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
        target.execute_batch("PRAGMA foreign_keys=ON; BEGIN; DELETE FROM speech_records; DELETE FROM language_edits WHERE id NOT IN (SELECT evidence_id FROM language_term_sources WHERE evidence_id IS NOT NULL); DELETE FROM language_lexical_candidates; UPDATE language_edits SET state=CASE WHEN state='pending' THEN 'expired' ELSE state END,automatic_terms_state=CASE WHEN automatic_terms_state='pending' THEN 'expired' ELSE automatic_terms_state END,personalization_state=CASE WHEN personalization_state='pending' THEN 'expired' ELSE personalization_state END; DELETE FROM language_observations; DELETE FROM language_profile_revisions; DELETE FROM language_profiles; COMMIT; VACUUM;")
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
    data.shutdown().await;
    drop(data);
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
        for reference in created {
            target_vault
                .delete(&reference)
                .map_err(|_| "profile-credential-cleanup")?;
        }
        return Err(code);
    }
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
        },
    )
}
#[tokio::test]
async fn developer_seed_preserves_settings_and_words_without_history_or_shared_credentials() {
    let (folder, source, mut original) = fixture().await;
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
