//! Authenticated control1. Expected hashes are identities, never caller-selected paths.
use crate::{
    engine_abi::CONTROL_PROTOCOL,
    wire::{EngineBinding, Runtime},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ControlAction {
    PrepareEngine,
    QueryEngine,
    DrainRetired,
    ReadRetired,
    AcknowledgeRetired,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlRequest {
    pub control_protocol: u32,
    pub capability: String,
    pub task: String,
    pub turn: u64,
    pub operation: u64,
    pub deadline_tick: u64,
    pub action: ControlAction,
    pub expected_core: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub receipt: Option<ReceiptKey>,
}
impl ControlRequest {
    pub fn valid(&self, now: u64) -> bool {
        self.control_protocol == CONTROL_PROTOCOL
            && uuid::Uuid::parse_str(&self.capability).is_ok()
            && uuid::Uuid::parse_str(&self.task).is_ok()
            && self.turn > 0
            && self.operation > 0
            && self.deadline_tick >= now
            && self.deadline_tick - now <= 15_000
            && self.expected_core.len() == 64
            && self.expected_core.bytes().all(|b| b.is_ascii_hexdigit())
            && match self.action {
                ControlAction::ReadRetired | ControlAction::AcknowledgeRetired => {
                    self.receipt.as_ref().is_some_and(|key| {
                        key.valid()
                            && key.task == self.task
                            && key.turn == self.turn
                            && key.operation == self.operation
                            && (!matches!(self.action, ControlAction::AcknowledgeRetired)
                                || key.sha256.is_some())
                    })
                }
                _ => self.receipt.is_none(),
            }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ControlReply {
    pub control_protocol: u32,
    pub task: String,
    pub turn: u64,
    pub operation: u64,
    pub ready: bool,
    pub code: String,
    pub engine: Option<EngineBinding>,
    pub runtime: Option<Runtime>,
    pub retired: u32,
}
/// Exact text-effect identity. Reading never acknowledges or replays an edit.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ReceiptKey {
    pub engine: EngineBinding,
    pub task: String,
    pub turn: u64,
    pub operation: u64,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}
impl ReceiptKey {
    pub fn valid(&self) -> bool {
        self.engine.valid()
            && uuid::Uuid::parse_str(&self.task).is_ok()
            && self.turn > 0
            && self.operation > 0
            && self
                .sha256
                .as_ref()
                .is_none_or(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
    }
    pub fn matches(&self, other: &Self) -> bool {
        self.engine == other.engine
            && self.task == other.task
            && self.turn == other.turn
            && self.operation == other.operation
            && self.revision == other.revision
            && self
                .sha256
                .as_ref()
                .is_none_or(|h| other.sha256.as_ref() == Some(h))
    }
    pub fn from_frame(frame: &[u8]) -> Option<Self> {
        use sha2::{Digest, Sha256};
        let value: serde_json::Value = serde_json::from_slice(frame).ok()?;
        let key = Self {
            engine: serde_json::from_value(value.get("engine")?.clone()).ok()?,
            task: value.get("task")?.as_str()?.into(),
            turn: value.get("turn")?.as_u64()?,
            operation: value.get("operation")?.as_u64()?,
            revision: value.get("revision")?.as_u64()?,
            sha256: Some(format!("{:x}", Sha256::digest(frame))),
        };
        key.valid().then_some(key)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CurrentEngine {
    pub schema_version: u32,
    pub revision: String,
    pub proxy_sha256: String,
    pub core_sha256: String,
    pub abi: u32,
    pub input_protocol: u32,
    pub platform: String,
}
impl CurrentEngine {
    pub fn valid(&self) -> bool {
        self.schema_version == 1
            && self.abi == 1
            && self.input_protocol > 0
            && self.platform == "x86_64-pc-windows-msvc"
            && uuid::Uuid::parse_str(&self.revision).is_ok()
            && [&self.proxy_sha256, &self.core_sha256]
                .into_iter()
                .all(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
    }
}
// The proxy forwards business payloads unchanged. Only stable routing/auth fields
// belong to control1; a compatible core may evolve its own input protocol.
#[derive(Clone, Debug, Deserialize)]
pub struct InputHeader {
    pub protocol: u32,
    pub engine: EngineBinding,
    pub capability: String,
    pub task: String,
    pub turn: u64,
    pub operation: u64,
    pub revision: u64,
    pub deadline_tick: u64,
}
impl InputHeader {
    pub fn valid(&self, now: u64) -> bool {
        self.protocol > 0
            && self.engine.valid()
            && uuid::Uuid::parse_str(&self.capability).is_ok()
            && uuid::Uuid::parse_str(&self.task).is_ok()
            && self.turn > 0
            && self.operation > 0
            && self.deadline_tick >= now
            && self.deadline_tick - now <= 2000
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receipt_ack_is_exact_and_cannot_cross_task_or_generation() {
        let frame = serde_json::json!({"engine":{"proxy_instance":uuid::Uuid::new_v4().to_string(),"engine_generation":3},
            "task":uuid::Uuid::new_v4().to_string(),"turn":2,"operation":7,"revision":4,"mutation":"changed"});
        let bytes = serde_json::to_vec(&frame).unwrap();
        let key = ReceiptKey::from_frame(&bytes).unwrap();
        assert!(key.valid());
        let mut selector = key.clone();
        selector.sha256 = None;
        assert!(selector.matches(&key));
        selector.engine.engine_generation += 1;
        assert!(!selector.matches(&key));
        selector = key.clone();
        selector.task = uuid::Uuid::new_v4().to_string();
        assert!(!selector.matches(&key));
        selector = key.clone();
        selector.sha256 = Some("0".repeat(64));
        assert!(!selector.matches(&key));
    }
    #[test]
    fn lifecycle_is_bounded_and_never_accepts_path_as_identity() {
        let mut request = ControlRequest {
            control_protocol: 1,
            capability: uuid::Uuid::new_v4().to_string(),
            task: uuid::Uuid::new_v4().to_string(),
            turn: 1,
            operation: 1,
            deadline_tick: 15100,
            action: ControlAction::PrepareEngine,
            expected_core: "a".repeat(64),
            receipt: None,
        };
        assert!(request.valid(100));
        request.deadline_tick += 1;
        assert!(!request.valid(100));
        request.deadline_tick = 101;
        request.expected_core = "C:\\arbitrary.dll".into();
        assert!(!request.valid(100));
    }
}
