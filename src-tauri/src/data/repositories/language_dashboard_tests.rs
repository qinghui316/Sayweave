use super::*;
fn observation(
    repo: &mut Repository,
    app: &ApplicationContext,
    id: &str,
    kind: &str,
    source: Option<&str>,
) {
    let value = serde_json::json!({"application":app,"sourceRef":source,"summary":"PRIVATE BODY NOT FOR DASHBOARD"});
    repo.connection.execute("INSERT OR IGNORE INTO language_observations(id,task_id,scope,settings_revision,kind,observation,created_at) VALUES(?1,?1,?2,1,?3,?4,1000)",params![id,app.key,kind,value.to_string()]).unwrap();
}
fn published(
    repo: &mut Repository,
    requested: &str,
    version: i64,
    rules: usize,
    candidates: usize,
) {
    let values = |count: usize| {
        (0..count)
            .map(|index| Preference {
                scope: requested.into(),
                rule: format!("保留理由{index}"),
                language: WritingLanguage::Any,
                context: None,
                evidence: vec!["private-source".into()],
            })
            .collect::<Vec<_>>()
    };
    repo.connection.execute("INSERT INTO language_profile_revisions(scope,version,predecessor,watermark,settings_revision,preferences,candidates,evidence,reason,analysis,created_at) VALUES(?1,?2,0,0,1,?3,?4,'[]','PRIVATE ANALYSIS','{}',1000)",params![requested,version,json(&values(rules)).unwrap(),json(&values(candidates)).unwrap()]).unwrap();
    repo.connection.execute("INSERT INTO language_profiles(scope,version,published_at) VALUES(?1,?2,1000) ON CONFLICT(scope) DO UPDATE SET version=excluded.version",params![requested,version]).unwrap();
}
#[test]
fn dashboard_empty_and_published_only_counts_remain_readonly() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let empty = overview(&mut repo.connection, None).unwrap();
    assert_eq!(
        (
            empty.preference_count,
            empty.application_count,
            empty.observation_count
        ),
        (0, 0, 0)
    );
    let app = ApplicationContext::from_identity(None, "Codex.exe");
    let web = ApplicationContext::from_identity(None, "chrome.exe");
    observation(&mut repo, &app, "edit-one", "edit", None);
    observation(&mut repo, &app, "edit-one", "edit", None);
    observation(&mut repo, &web, "web-one", "edit", None);
    observation(
        &mut repo,
        &app,
        "missing-audio-source",
        "speech",
        Some("deleted"),
    );
    published(&mut repo, &app.key, 1, 6, 0);
    published(&mut repo, &app.key, 2, 2, 4);
    published(&mut repo, "shared", 1, 1, 0);
    let before = repo.connection.total_changes();
    let value = overview(&mut repo.connection, None).unwrap();
    assert_eq!(
        (
            value.preference_count,
            value.application_count,
            value.observation_count
        ),
        (3, 2, 2)
    );
    assert_eq!(value.shared.preference_count, 1);
    assert_eq!(
        value
            .applications
            .iter()
            .find(|v| v.scope == "web")
            .unwrap()
            .name,
        "网页"
    );
    let profile = profile(&repo.connection, app.key.clone()).unwrap();
    assert_eq!(profile.version, 2);
    assert_eq!(profile.preferences.len(), 2);
    let encoded = serde_json::to_string(&profile).unwrap();
    assert!(!encoded.contains("PRIVATE"));
    assert!(!encoded.contains("private-source"));
    assert_eq!(repo.connection.total_changes(), before);
    repo.language_operation(DbOperation::SaveSettings(LanguageSettingsPatch {
        personalization: Some(Settings {
            enabled: false,
            revision: 1,
        }),
        ..Default::default()
    }))
    .unwrap();
    assert_eq!(
        overview(&mut repo.connection, None)
            .unwrap()
            .preference_count,
        3
    );
}
#[test]
fn dashboard_pages_are_bounded_and_no_scope_repeats() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    for index in 0..25 {
        let app = ApplicationContext::from_identity(None, &format!("Editor{index}.exe"));
        observation(&mut repo, &app, &format!("e{index}"), "edit", None);
    }
    let first = overview(&mut repo.connection, None).unwrap();
    assert_eq!(first.applications.len(), 20);
    assert_eq!(first.application_count, 25);
    let second = overview(&mut repo.connection, first.next).unwrap();
    assert_eq!(second.applications.len(), 5);
    assert!(second.next.is_none());
    assert!(
        second
            .applications
            .iter()
            .all(|v| !first.applications.iter().any(|old| old.scope == v.scope))
    );
    assert!(overview(&mut repo.connection, Some("bad".into())).is_err());
    assert!(profile(&repo.connection, "bad".into()).is_err());
}
#[test]
fn dashboard_counts_successful_speech_sources_only_while_they_remain_available() {
    let root = tempfile::tempdir().unwrap();
    let mut repo = Repository::open(root.path()).unwrap();
    let app = ApplicationContext::from_identity(None, "Codex.exe");
    repo.connection
        .execute(
            "INSERT INTO speech_records(id,created_at,status) VALUES('r',1000,'success')",
            [],
        )
        .unwrap();
    repo.connection.execute("INSERT INTO speech_attempts(id,record_id,kind,config_id,config_name,model_id,service_host,config_revision,created_at,status,text) VALUES('a','r','live','test','test','test','test',1,1000,'success','PRIVATE TRANSCRIPT')",[]).unwrap();
    observation(&mut repo, &app, "speech-one", "speech", Some("a"));
    let ready = overview(&mut repo.connection, None).unwrap();
    assert_eq!(ready.observation_count, 1);
    assert_eq!(ready.application_count, 1);
    assert!(!serde_json::to_string(&ready).unwrap().contains("PRIVATE"));
    repo.connection
        .execute(
            "UPDATE speech_attempts SET status='failed' WHERE id='a'",
            [],
        )
        .unwrap();
    assert_eq!(
        overview(&mut repo.connection, None)
            .unwrap()
            .observation_count,
        0
    );
}
