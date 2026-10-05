//! Physical input evidence shared by shortcut admission and text delivery.
use std::sync::{
    Arc, Mutex, OnceLock,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

static READY: AtomicBool = AtomicBool::new(false);
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
static KEYS: Mutex<Vec<u16>> = Mutex::new(Vec::new());
pub fn ready(value: bool) {
    READY.store(value, Ordering::Release);
    if let Ok(mut keys) = KEYS.lock() {
        keys.clear();
    }
}
pub fn tracking_ready() -> bool {
    READY.load(Ordering::Acquire)
}
pub fn activity() -> u64 {
    evidence().load(Ordering::Acquire)
}
pub fn edited() {
    let version = evidence().fetch_add(1, Ordering::AcqRel) + 1;
    edits().send_replace(version);
}
pub fn key(value: u16, down: bool) {
    if let Ok(mut keys) = KEYS.lock() {
        keys.retain(|k| *k != value);
        if down {
            keys.push(value);
        }
    }
}
pub fn modifier_held() -> bool {
    KEYS.lock()
        .is_ok_and(|keys| keys.iter().any(|k| matches!(k, 0xa0..=0xa5 | 0x5b..=0x5c)))
}
pub fn modifier_block(physical: bool, logical: bool) -> Option<&'static str> {
    if physical {
        Some("keysHeld")
    } else if logical {
        Some("keyboardStateConflict")
    } else {
        None
    }
}
#[cfg(test)]
mod tests {
    #[test]
    fn logical_state_alone_never_claims_a_physical_key_is_held() {
        assert_eq!(
            super::modifier_block(false, true),
            Some("keyboardStateConflict")
        );
        assert_eq!(super::modifier_block(true, false), Some("keysHeld"));
        assert_eq!(super::modifier_block(false, false), None);
    }
}
