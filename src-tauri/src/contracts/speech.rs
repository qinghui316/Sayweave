use serde::{Deserialize, Serialize};
#[derive(Clone, Copy)]
pub enum StopReason {
    Finish,
    Cancel,
    Interrupted,
}
impl StopReason {
    pub fn merge(self, current: u8) -> u8 {
        match (current, self) {
            (2, _) | (_, Self::Cancel) => 2,
            (3, _) | (_, Self::Interrupted) => 3,
            _ => 1,
        }
    }
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Phase {
    Idle,
    Connecting,
    Recording,
    Processing,
    Completed,
    Cancelled,
    Failed,
}
