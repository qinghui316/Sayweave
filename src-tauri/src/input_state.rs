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
