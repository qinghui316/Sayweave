//! Shared draft provenance. Storage depends on this contract, not the actor.
use super::dictionary::DataError;
use serde::{Deserialize, Serialize};
use std::ops::Range;
pub fn equivalent_text(a: &str, b: &str) -> bool {
    a.replace("\r\n", "\n").replace('\r', "\n") == b.replace("\r\n", "\n").replace('\r', "\n")
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BaselineKind {
    Asr,
    EditorSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Snapshot {
    pub block: String,
    pub version: u64,
    pub source_epoch: u64,
    pub baseline_kind: BaselineKind,
    pub text: String,
    pub speech_end: usize,
    pub insertion_end: usize,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    pub segment: u64,
    pub revision: u64,
    pub chars: Range<usize>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Checkpoint {
    pub run_id: String,
    pub order: u64,
    pub source: Snapshot,
    pub visible_revision: u64,
    pub visible_text: String,
    pub state: String,
}
impl Checkpoint {
    pub fn valid(&self) -> bool {
        uuid::Uuid::parse_str(&self.run_id).is_ok()
            && uuid::Uuid::parse_str(&self.source.block).is_ok()
            && self.order > 0
            && self.source.version > 0
            && self.source.source_epoch > 0
            && self.source.text.len() <= 1024 * 1024
            && self.visible_text.len() <= 1024 * 1024
            && ["preview", "delivered", "stopped", "uncertain"].contains(&self.state.as_str())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Association {
    pub format: String,
    pub order: u64,
    pub source: Snapshot,
    pub asr_turn: u64,
    pub asr_sources: Vec<SourceSpan>,
    pub delivery: bool,
    pub visible_revision: Option<u64>,
}
impl Association {
    pub fn attempt(order: u64, source: Snapshot) -> Self {
        Self {
            format: "revisable-v1".into(),
            order,
            source,
            asr_turn: 1,
            asr_sources: vec![],
            delivery: false,
            visible_revision: None,
        }
    }
    pub fn valid(&self) -> bool {
        self.format == "revisable-v1"
            && uuid::Uuid::parse_str(&self.source.block).is_ok()
            && self.order > 0
            && self.asr_turn > 0
            && self.source.version > 0
            && self.source.source_epoch > 0
            && self.source.text.len() <= 1024 * 1024
            && self.source.insertion_end <= self.source.text.chars().count()
            && self
                .asr_sources
                .iter()
                .all(|s| s.revision > 0 && s.chars.start < s.chars.end)
            && (!self.delivery || self.visible_revision.is_some())
    }
}
pub fn read(value: &str) -> Result<Option<Association>, DataError> {
    if value.trim_start().starts_with('[') {
        return Ok(None);
    }
    let association: Association =
        serde_json::from_str(value).map_err(|_| DataError::new("invalid"))?;
    if !association.valid() {
        return Err(DataError::new("invalid"));
    }
    Ok(Some(association))
}
