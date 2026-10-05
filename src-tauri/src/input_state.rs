//! Physical input evidence shared by shortcut admission and text delivery.
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

static READY: AtomicBool = AtomicBool::new(false);
static ACTIVITY: AtomicU64 = AtomicU64::new(0);
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
    ACTIVITY.load(Ordering::Acquire)
}
pub fn edited() {
    ACTIVITY.fetch_add(1, Ordering::AcqRel);
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
