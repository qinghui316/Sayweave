//! Physical input evidence shared by shortcut admission and text delivery.
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};

fn held_edits() -> &'static Mutex<HashSet<u16>> {
    static KEYS: OnceLock<Mutex<HashSet<u16>>> = OnceLock::new();
    KEYS.get_or_init(|| Mutex::new(HashSet::new()))
}
pub fn edit_key_pressed(key: u16) {
    if let Ok(mut keys) = held_edits().lock() {
        keys.insert(key);
    }
    edited();
    observation_input(matches!(key, 0x0d | 0x09));
}

// Readonly observation metadata. Key-up and accessibility notifications may
// advance activity, but cannot disguise unread typing as an explicit boundary.
static OBSERVATION_INPUT: AtomicU64 = AtomicU64::new(0);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObservationInput(u64);
pub fn observation_input_stamp() -> ObservationInput {
    ObservationInput(OBSERVATION_INPUT.load(Ordering::Acquire))
}
fn observation_input(boundary: bool) {
    let _ = OBSERVATION_INPUT.try_update(Ordering::AcqRel, Ordering::Acquire, |old| {
        old.checked_add(2)
            .map(|next| (next & !1) | u64::from(boundary))
    });
}
pub fn observation_pointer() {
    observation_input(true);
}
impl ObservationInput {
    pub fn boundary_after(self, captured: Self) -> bool {
        self.0 & 1 != 0
            && (self.0 >> 1 == captured.0 >> 1
                || (captured.0 >> 1).checked_add(1) == Some(self.0 >> 1))
    }
}
pub fn key_released(key: u16) {
    let changed = held_edits().lock().is_ok_and(|mut keys| keys.remove(&key));
    if changed {
        edited();
    }
}
pub fn editing_key_held() -> bool {
    held_edits().lock().map_or(true, |keys| !keys.is_empty())
}
pub fn reset_edit_keys() {
    if let Ok(mut keys) = held_edits().lock() {
        keys.clear();
    }
}

fn evidence() -> &'static Arc<AtomicU64> {
    static ACTIVITY: OnceLock<Arc<AtomicU64>> = OnceLock::new();
    ACTIVITY.get_or_init(|| Arc::new(AtomicU64::new(0)))
}
fn edits() -> &'static tokio::sync::watch::Sender<u64> {
    static EDITS: OnceLock<tokio::sync::watch::Sender<u64>> = OnceLock::new();
    EDITS.get_or_init(|| tokio::sync::watch::channel(activity()).0)
}
pub fn activity_evidence() -> Arc<AtomicU64> {
    evidence().clone()
}
pub fn subscribe_edits() -> tokio::sync::watch::Receiver<u64> {
    edits().subscribe()
}
pub fn activity() -> u64 {
    evidence().load(Ordering::Acquire)
}
pub fn edited() {
    let version = evidence().fetch_add(1, Ordering::AcqRel) + 1;
    edits().send_replace(version);
}
// Undo/redo can restore a pre-dictation selection, which is not a spelling
// correction. This identity-only marker never intercepts a user's key.
static REVERSALS: AtomicU64 = AtomicU64::new(0);
pub fn reversal_epoch() -> u64 {
    REVERSALS.load(Ordering::Acquire)
}
pub fn reversed() {
    REVERSALS.fetch_add(1, Ordering::AcqRel);
}
pub fn reversal_key(key: u16, control: bool, alt: bool, altgr: bool) -> bool {
    !altgr && (control && !alt && matches!(key, 0x59 | 0x5a) || alt && !control && key == 0x08)
}
#[cfg(test)]
mod reversal_tests {
    #[test]
    fn observation_boundary_does_not_cover_intervening_unread_edits() {
        use super::ObservationInput as Input;
        assert!(Input(21).boundary_after(Input(18)));
        assert!(Input(21).boundary_after(Input(21)));
        assert!(!Input(21).boundary_after(Input(16)));
        assert!(!Input(20).boundary_after(Input(18)));
        assert!(!Input(21).boundary_after(Input(23)));
    }
    #[test]
    fn undo_redo_markers_do_not_classify_altgr_or_ordinary_typing() {
        assert!(super::reversal_key(0x5a, true, false, false));
        assert!(super::reversal_key(0x59, true, false, false));
        assert!(super::reversal_key(0x08, false, true, false));
        assert!(!super::reversal_key(0x5a, true, true, true));
        assert!(!super::reversal_key(0x5a, false, false, false));
        assert!(!super::reversal_key(0x41, true, false, false));
    }
}
