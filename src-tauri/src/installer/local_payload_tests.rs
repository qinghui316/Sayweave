use super::record::Target;
use crate::build_info::local_runtime::RuntimeFile;

fn legacy() -> Target {
    serde_json::from_value(serde_json::json!({"version":"0.1.18","commit":"a".repeat(40),"product":"Sayweave","payloadHash":"b".repeat(64),"mainHash":"c".repeat(64),"helperHash":"d".repeat(64),"uninstallHash":"e".repeat(64)})).unwrap()
}
#[test]
fn old_receipts_keep_their_exact_optional_field_representation() {
    let target = legacy();
    target.validate().unwrap();
    assert!(target.local_files.is_empty());
    assert!(
        serde_json::to_value(target)
            .unwrap()
            .get("localFiles")
            .is_none()
    );
}
#[test]
fn new_receipts_require_complete_unique_bounded_runtime_identity() {
    let mut target = legacy();
    target.local_files = sayweave_local_asr_protocol::runtime_file_names()
        .iter()
        .map(|name| RuntimeFile {
            name: name.clone(),
            bytes: 512,
            sha256: "f".repeat(64),
        })
        .collect();
    target.validate().unwrap();
    let full = target.clone();
    target.local_files.pop();
    assert!(target.validate().is_err());
    target = full.clone();
    target.local_files[1] = target.local_files[0].clone();
    assert!(target.validate().is_err());
    target = full.clone();
    target.local_files[0].name = "../outside.exe".into();
    assert!(target.validate().is_err());
    target = full;
    target.local_files[0].bytes = 128 * 1024 * 1024 + 1;
    assert!(target.validate().is_err());
}
