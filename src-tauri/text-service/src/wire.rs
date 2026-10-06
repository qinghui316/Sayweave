use serde::{Deserialize, Serialize};

pub const MAX_FRAME: usize = 128 * 1024;
pub const MAX_TEXT_UTF16: usize = 16 * 1024;
// v5 settles a processed prefix without changing source offsets or ending the
// logical draft. The remaining tail can be an owned empty insertion lease.
// A registered upgrade is insufficient: the target must be running this version.
pub const PROTOCOL: u32 = 5;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Runtime {
    pub protocol: u32,
    pub process: u32,
    pub thread: u32,
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<Prefix>,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Prefix {
    pub text: String,
    pub expected_revision: u64,
    pub range_epoch: u64,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    Inspect,
    Begin,
    Update,
    Settle,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<Runtime>,
    #[serde(default)]
    pub range_epoch: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settlement: Option<Settlement>,
}
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub struct Settlement {
    pub prefix: Mutation,
    /// The tail's logical write lease remains valid, including an empty tail.
    /// This is not independent confirmation of target rendering.
    pub tail_owned: bool,
}
impl Reply {
    pub fn compatible_runtime(&self, process: u32, thread: u32) -> bool {
        self.applied
            && self.mutation == Mutation::NotAttempted
            && self.runtime.as_ref().is_some_and(|r| {
                r.protocol == PROTOCOL && r.process == process && r.thread == thread
            })
    }
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
        self.protocol == PROTOCOL
            && uuid::Uuid::parse_str(&self.task).is_ok()
            && uuid::Uuid::parse_str(&self.capability).is_ok()
            && self.turn > 0
            && self.operation > 0
            && self.text.encode_utf16().count()
                + self
                    .prefix
                    .as_ref()
                    .map_or(0, |p| p.text.encode_utf16().count())
                <= MAX_TEXT_UTF16
            && match (self.action, &self.prefix) {
                (Action::Settle, Some(prefix)) => {
                    !prefix.text.is_empty()
                        && prefix.range_epoch > 0
                        && prefix.expected_revision < self.revision
                }
                (Action::Settle, None) => false,
                (_, prefix) => prefix.is_none(),
            }
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
    /// A cleanup acknowledgement for the same relinquished range, never a new write.
    pub fn acknowledges_handoff(&self, request: &Request) -> bool {
        request.valid()
            && request.action == Action::Handoff
            && !self.owned
            && self.task == request.task
            && self.turn == request.turn
            && request.operation > self.operation
            && request.revision >= self.revision
    }
    pub fn accepts(&self, request: &Request) -> bool {
        request.valid()
            && request.action != Action::Inspect
            && if request.action == Action::Begin {
                // Operation counters belong to a turn, not to the lifetime of the
                // loaded text service. A new host/turn starts its own counter.
                !self.owned && (self.task != request.task || request.turn > self.turn)
            } else {
                self.owned
                    && self.task == request.task
                    && self.turn == request.turn
                    && request.operation > self.operation
                    && request.revision >= self.revision
                    && (request.action != Action::Settle
                        || request
                            .prefix
                            .as_ref()
                            .is_some_and(|p| p.expected_revision == self.revision))
                    && (!matches!(request.action, Action::Update | Action::Settle)
                        || request.revision > self.revision)
            }
    }
    pub fn applied(&mut self, request: &Request) {
        self.task.clone_from(&request.task);
        self.turn = request.turn;
        self.operation = request.operation;
        self.revision = request.revision;
        self.owned = matches!(
            request.action,
            Action::Begin | Action::Update | Action::Settle
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request(action: Action, operation: u64, revision: u64) -> Request {
        Request {
            protocol: crate::wire::PROTOCOL,
            capability: uuid::Uuid::new_v4().to_string(),
            task: "3923cd50-c588-49b5-9895-ce2e23e21b5b".into(),
            turn: 1,
            operation,
            revision,
            deadline_tick: 1000,
            action,
            text: "中文🙂".into(),
            prefix: None,
        }
    }
    #[test]
    fn runtime_admission_rejects_old_or_different_targets_without_edits() {
        let mut r = request(Action::Inspect, 1, 0);
        assert!(r.valid());
        assert!(!DraftVersion::default().accepts(&r));
        r.protocol = 1;
        assert!(!r.valid());
        let mut reply = Reply {
            range_epoch: 0,
            settlement: None,
            operation: 1,
            revision: 0,
            applied: true,
            code: "runtime".into(),
            mutation: Mutation::NotAttempted,
            runtime: None,
        };
        assert!(!reply.compatible_runtime(10, 20));
        reply.runtime = Some(Runtime {
            protocol: PROTOCOL,
            process: 10,
            thread: 20,
        });
        assert!(reply.compatible_runtime(10, 20));
        assert!(!reply.compatible_runtime(11, 20));
        assert!(!reply.compatible_runtime(10, 21));
        reply.mutation = Mutation::Changed;
        assert!(!reply.compatible_runtime(10, 20));
    }
    #[test]
    fn prefix_settlement_requires_the_live_write_revision_and_retains_tail_ownership() {
        let mut state = DraftVersion::default();
        state.applied(&request(Action::Begin, 1, 0));
        state.applied(&request(Action::Update, 2, 1));
        let mut settle = request(Action::Settle, 3, 2);
        settle.prefix = Some(Prefix {
            text: "已整理".into(),
            expected_revision: 1,
            range_epoch: 4,
        });
        assert!(state.accepts(&settle));
        state.applied(&settle);
        assert!(state.owned);
        assert!(!state.accepts(&settle));
        let update = request(Action::Update, 4, 3);
        assert!(state.accepts(&update));
        state.applied(&update);
        settle.operation = 5;
        settle.revision = 4;
        assert!(!state.accepts(&settle)); // Old generation result needs a fresh write transaction.
        settle.prefix.as_mut().unwrap().expected_revision = 3;
        assert!(state.accepts(&settle));
    }
    #[test]
    fn released_ranges_acknowledge_only_their_own_handoff_and_never_mutate_again() {
        let mut draft = DraftVersion::default();
        draft.applied(&request(Action::Begin, 1, 0));
        draft.applied(&request(Action::Update, 2, 1));
        draft.owned = false; // target has already handed it to a foreign editor
        assert!(draft.acknowledges_handoff(&request(Action::Handoff, 3, 1)));
        assert!(!draft.accepts(&request(Action::Update, 3, 2)));
        assert!(!draft.acknowledges_handoff(&request(Action::Abort, 3, 1)));
        let mut other = request(Action::Handoff, 3, 1);
        other.turn = 2;
        assert!(!draft.acknowledges_handoff(&other));
        other.turn = 1;
        other.task = uuid::Uuid::new_v4().to_string();
        assert!(!draft.acknowledges_handoff(&other));
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
    fn a_reused_service_accepts_new_tasks_and_turns_with_fresh_counters() {
        let mut draft = DraftVersion::default();
        draft.applied(&request(Action::Begin, 1, 0));
        draft.applied(&request(Action::Update, 2, 1));
        let mut next = request(Action::Begin, 1, 0);
        next.turn = 2;
        assert!(!draft.accepts(&next)); // an owned range must first be released
        draft.applied(&request(Action::Handoff, 3, 1));
        assert!(draft.accepts(&next));
        draft.applied(&next);
        assert!(!draft.accepts(&request(Action::Update, 99, 99)));
        assert!(!draft.accepts(&next));
        let mut commit = next.clone();
        commit.action = Action::Commit;
        commit.operation = 2;
        assert!(draft.accepts(&commit));
        draft.applied(&commit);
        assert!(!draft.accepts(&request(Action::Begin, 100, 0)));
        next.task = uuid::Uuid::new_v4().to_string();
        next.turn = 1;
        assert!(draft.accepts(&next));
        draft.applied(&next);
        assert!(!draft.accepts(&commit));
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
