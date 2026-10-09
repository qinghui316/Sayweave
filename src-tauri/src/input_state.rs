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
    fn undo_redo_markers_do_not_classify_altgr_or_ordinary_typing() {
        assert!(super::reversal_key(0x5a, true, false, false));
        assert!(super::reversal_key(0x59, true, false, false));
        assert!(super::reversal_key(0x08, false, true, false));
        assert!(!super::reversal_key(0x5a, true, true, true));
        assert!(!super::reversal_key(0x5a, false, false, false));
        assert!(!super::reversal_key(0x41, true, false, false));
    }
}
