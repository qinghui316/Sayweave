//! Internal model wire values. Neither requests nor results are diagnostic DTOs.
use serde::{Deserialize, Serialize};
pub const MESSAGE_LIMIT: usize = 32 * 1024;
pub const OUTPUT_TOKENS: u32 = 4096;
#[derive(Clone, PartialEq, Eq)]
pub struct SourceSegment {
    pub id: u64,
    pub revision: u64,
    pub text: String,
    pub confirmed: bool,
}
#[derive(Clone)]
pub struct SourceTurn {
    pub turn: u64,
    pub text: String,
    pub segments: Vec<SourceSegment>,
}

#[derive(Clone, Serialize)]
pub struct Message {
    pub role: &'static str,
    pub content: String,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub total: Option<u64>,
}

pub struct GeneratedText {
    pub text: String,
    pub usage: TokenUsage,
    pub first_output_ms: Option<u64>,
    pub elapsed_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub enabled: bool,
    pub revision: i64,
}
#[derive(Clone, Debug)]
pub enum DbOperation {
    Settings,
    SaveSettings(Settings),
    DictionarySnapshot,
    Prepare(RunMetadata),
    SaveUnit(UnitRecord),
    SaveDraft(super::draft::Checkpoint),
    Finish {
        id: String,
        text: String,
        state: String,
        error: Option<String>,
    },
    Cancel {
        id: String,
        text: String,
        raw: String,
        rollback_failed: bool,
        units: Vec<UnitRecord>,
    },
    Recover,
}
#[derive(Debug)]
pub enum DbOutput {
    Settings(Settings),
    Dictionary(Vec<crate::contracts::dictionary::Record>),
    Done,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunMetadata {
    #[serde(default = "model_mode", skip_serializing_if = "is_model_mode")]
    pub mode: String,
    pub id: String,
    pub attempt_id: String,
    pub task_id: String,
    pub config_id: String,
    pub config_name: String,
    pub model_id: String,
    pub config_revision: i64,
    pub prompt_version: String,
    pub settings_revision: i64,
    pub dictionary_version: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UnitRecord {
    pub id: String,
    pub run_id: String,
    pub turn: u64,
    pub source_start: usize,
    pub source_end: usize,
    pub source_refs: String,
    pub source_text: String,
    pub candidate_text: Option<String>,
    pub generation_state: String,
    pub application_state: String,
    pub source_committed_end: Option<usize>,
    pub error_code: Option<String>,
    pub prompt_tokens: Option<u64>,
    pub completion_tokens: Option<u64>,
    pub elapsed_ms: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunRecord {
    pub mode: String,
    pub id: String,
    pub attempt_id: String,
    pub config_name: String,
    pub model_id: String,
    pub state: String,
    pub text: String,
    pub revision: i64,
    pub error_code: Option<String>,
    pub units: Vec<UnitRecord>,
}
fn model_mode() -> String {
    "model".into()
}
fn is_model_mode(value: &str) -> bool {
    value == "model"
}
