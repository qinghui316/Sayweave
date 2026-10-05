use serde::{Deserialize, Serialize};

pub const MAX_FRAME: usize = 128 * 1024;
pub const MAX_TEXT_UTF16: usize = 16 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub protocol: u32,
    pub capability: String,
    pub task: String,
    pub turn: u64,
    pub operation: u64,
    pub revision: u64,
    pub deadline_tick: u64,
    pub action: Action,
    pub text: String,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Begin,
    Update,
    Commit,
    Handoff,
    Abort,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Reply {
    pub operation: u64,
    pub revision: u64,
    pub applied: bool,
    pub code: String,
    pub mutation: Mutation,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Mutation {
    #[default]
    NotAttempted,
    Changed,
    Uncertain,
}
impl Request {
    pub fn valid(&self) -> bool {
        self.protocol == 1
            && uuid::Uuid::parse_str(&self.task).is_ok()
            && uuid::Uuid::parse_str(&self.capability).is_ok()
            && self.turn > 0
            && self.operation > 0
            && self.text.encode_utf16().count() <= MAX_TEXT_UTF16
    }
    pub fn live(&self, now: u64) -> bool {
        self.deadline_tick >= now && self.deadline_tick - now <= 2000
    }
}
/// Metadata only. The range and its text live in the target apartment.
#[derive(Default, Debug)]
pub struct DraftVersion {
    pub task: String,
    pub turn: u64,
    pub revision: u64,
    pub operation: u64,
    pub owned: bool,
}
impl DraftVersion {
    pub fn accepts(&self, request: &Request) -> bool {
        request.valid()
            && request.operation > self.operation
            && if request.action == Action::Begin {
                !self.owned && (self.task != request.task || request.turn > self.turn)
            } else {
                self.owned
                    && self.task == request.task
                    && self.turn == request.turn
                    && request.revision >= self.revision
                    && (request.action != Action::Update || request.revision > self.revision)
            }
    }
    pub fn applied(&mut self, request: &Request) {
        self.task.clone_from(&request.task);
        self.turn = request.turn;
        self.operation = request.operation;
        self.revision = request.revision;
        self.owned = matches!(request.action, Action::Begin | Action::Update);
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request(action: Action, operation: u64, revision: u64) -> Request {
        Request {
            protocol: 1,
            capability: uuid::Uuid::new_v4().to_string(),
            task: "3923cd50-c588-49b5-9895-ce2e23e21b5b".into(),
            turn: 1,
            operation,
            revision,
            deadline_tick: 1000,
            action,
            text: "中文🙂".into(),
        }
    }
    #[test]
    fn old_updates_and_handed_off_turns_never_write_again() {
        let mut draft = DraftVersion::default();
        let begin = request(Action::Begin, 1, 0);
        assert!(draft.accepts(&begin));
        draft.applied(&begin);
        let update = request(Action::Update, 2, 1);
        assert!(draft.accepts(&update));
        draft.applied(&update);
        assert!(!draft.accepts(&update));
        assert!(!draft.accepts(&request(Action::Update, 3, 0)));
        let handoff = request(Action::Handoff, 4, 1);
        assert!(draft.accepts(&handoff));
        draft.applied(&handoff);
        assert!(!draft.accepts(&request(Action::Update, 5, 2)));
        assert!(!draft.accepts(&request(Action::Begin, 6, 0)));
        let mut next = request(Action::Begin, 7, 0);
        next.turn = 2;
        assert!(draft.accepts(&next));
    }
    #[test]
    fn bounds_count_utf16_and_reject_unknown_fields() {
        let mut r = request(Action::Begin, 1, 0);
        r.text = "🙂".repeat(MAX_TEXT_UTF16 / 2);
        assert!(r.valid());
        r.text.push('a');
        assert!(!r.valid());
        assert!(serde_json::from_str::<Request>("{\"window\":42}").is_err());
    }
    #[test]
    fn requests_have_a_bounded_execution_deadline() {
        let mut r = request(Action::Begin, 1, 0);
        r.deadline_tick = 3000;
        assert!(r.live(1000));
        assert!(r.live(3000));
        assert!(!r.live(3001));
        assert!(!r.live(999));
    }
}
