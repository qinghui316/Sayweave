//! Physical input evidence shared by shortcut admission and text delivery.
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicU64, Ordering},
};

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
