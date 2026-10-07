//! Task-owned, bounded persistence. This queue never owns input or recognition state.
use crate::data::{DataWorker, Operation};
use std::{
    collections::VecDeque,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::sync::{Notify, watch};

const CAPACITY: usize = 64;
const BYTE_CAPACITY: usize = 2 * 1024 * 1024;
static QUEUES: LazyLock<Arc<tokio::sync::Semaphore>> =
    LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(16)));
static WORKERS: AtomicUsize = AtomicUsize::new(0);
struct Pending {
    writes: VecDeque<(String, Operation, usize)>,
    bytes: usize,
    sealed: bool,
}
struct Shared {
    pending: Mutex<Pending>,
    wake: Notify,
    failed: AtomicBool,
    _permit: Option<tokio::sync::OwnedSemaphorePermit>,
}
fn bytes(operation: &Operation) -> usize {
    use crate::contracts::{history::DbOperation as H, processing::DbOperation as P};
    match operation {
        Operation::History(H::Checkpoint { text, .. } | H::Finish { text, .. }) => {
            text.len() + 1024
        }
        Operation::Processing(P::SaveUnit(value)) => {
            value.source_text.len()
                + value.source_refs.len()
                + value.candidate_text.as_ref().map_or(0, String::len)
                + 2048
        }
        _ => 1024,
    }
}
struct Producer(Arc<Shared>);
impl Drop for Producer {
    fn drop(&mut self) {
        if let Ok(mut pending) = self.0.pending.lock() {
            pending.sealed = true;
        }
        self.0.wake.notify_one();
    }
}
#[derive(Clone)]
pub(crate) struct Archive {
    producer: Arc<Producer>,
    done: watch::Receiver<bool>,
}
impl Archive {
    pub fn new(
        data: DataWorker,
        task: String,
    ) -> Result<Self, crate::contracts::dictionary::DataError> {
        let permit = QUEUES
            .clone()
            .try_acquire_owned()
            .map_err(|_| crate::contracts::dictionary::DataError::new("busy"))?;
        Ok(Self::with_permit(task, Some(permit), move |operation| {
            let data = data.clone();
            async move { data.request_archive(operation).await.is_ok() }
        }))
    }
    #[cfg(test)]
    fn with_writer<F, Fut>(task: String, write: F) -> Self
    where
        F: Fn(Operation) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = bool> + Send,
    {
        Self::with_permit(task, None, write)
    }
    fn with_permit<F, Fut>(
        task: String,
        permit: Option<tokio::sync::OwnedSemaphorePermit>,
        write: F,
    ) -> Self
    where
        F: Fn(Operation) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = bool> + Send,
    {
        let shared = Arc::new(Shared {
            pending: Mutex::new(Pending {
                writes: VecDeque::new(),
                bytes: 0,
                sealed: false,
            }),
            wake: Notify::new(),
            failed: AtomicBool::new(false),
            _permit: permit,
        });
        let (done, receiver) = watch::channel(false);
        let state = shared.clone();
        WORKERS.fetch_add(1, Ordering::AcqRel);
        tokio::spawn(async move {
            struct Worker;
            impl Drop for Worker {
                fn drop(&mut self) {
                    WORKERS.fetch_sub(1, Ordering::AcqRel);
                }
            }
            let _worker = Worker;
            loop {
                let wake = state.wake.notified();
                let item = {
                    let Ok(mut pending) = state.pending.lock() else {
                        state.failed.store(true, Ordering::Release);
                        break;
                    };
                    match pending.writes.pop_front() {
                        Some((_, operation, size)) => {
                            pending.bytes -= size;
                            Some(operation)
                        }
                        None if pending.sealed => break,
                        None => None,
                    }
                };
                if let Some(operation) = item {
                    if !write(operation).await {
                        state.failed.store(true, Ordering::Release);
                    }
                } else {
                    wake.await;
                }
            }
            log::info!(target:"sayweave", "operation=archive_settled task_id={} failed={}", task, state.failed.load(Ordering::Acquire));
            done.send_replace(true);
        });
        Self {
            producer: Arc::new(Producer(shared)),
            done: receiver,
        }
    }
    /// Replace a pending checkpoint/unit only, never an already executing write.
    /// Terminal facts use seal_with, which has one reserved slot beyond capacity.
    pub fn enqueue(&self, key: String, operation: Operation) -> bool {
        let state = &self.producer.0;
        let Ok(mut pending) = state.pending.lock() else {
            return false;
        };
        if pending.sealed {
            return false;
        }
        let previous = pending.writes.iter().position(|(old, _, _)| old == &key);
        let size = bytes(&operation);
        let previous_size = previous.map_or(0, |index| pending.writes[index].2);
        if (previous.is_none() && pending.writes.len() >= CAPACITY)
            || pending.bytes - previous_size + size > BYTE_CAPACITY
        {
            state.failed.store(true, Ordering::Release);
            return false;
        }
        if let Some(index) = previous {
            pending.writes.remove(index);
        }
        pending.bytes = pending.bytes - previous_size + size;
        pending.writes.push_back((key, operation, size));
        state.wake.notify_one();
        true
    }
    pub fn seal_with(&self, terminal: Option<Operation>) {
        let state = &self.producer.0;
        if let Ok(mut pending) = state.pending.lock() {
            if pending.sealed {
                return;
            }
            if let Some(operation) = terminal {
                let size = bytes(&operation);
                pending.bytes += size;
                pending.writes.push_back((String::new(), operation, size));
            }
            pending.sealed = true;
        }
        state.wake.notify_one();
    }
    pub async fn finish(&self) -> bool {
        self.seal_with(None);
        let mut done = self.done.clone();
        while !*done.borrow_and_update() {
            if done.changed().await.is_err() {
                return false;
            }
        }
        !self.producer.0.failed.load(Ordering::Acquire)
    }
}
/// Tracks application archive finalizers through the existing bounded shutdown.
pub(crate) fn spawn(work: impl std::future::Future<Output = ()> + Send + 'static) {
    WORKERS.fetch_add(1, Ordering::AcqRel);
    tokio::spawn(async move {
        struct Finalizer;
        impl Drop for Finalizer {
            fn drop(&mut self) {
                WORKERS.fetch_sub(1, Ordering::AcqRel);
            }
        }
        let _finalizer = Finalizer;
        work.await;
    });
}
pub(crate) async fn drain() {
    while WORKERS.load(Ordering::Acquire) != 0 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn slow_storage_coalesces_and_seals_without_blocking_producers() {
        let gate = Arc::new(Notify::new());
        let writes = Arc::new(AtomicUsize::new(0));
        let (g, w) = (gate.clone(), writes.clone());
        let archive = Archive::with_writer("test".into(), move |_| {
            let (g, w) = (g.clone(), w.clone());
            async move {
                g.notified().await;
                w.fetch_add(1, Ordering::AcqRel);
                true
            }
        });
        for _ in 0..1000 {
            assert!(archive.enqueue("checkpoint".into(), Operation::Drain));
        }
        archive.seal_with(Some(Operation::Drain));
        assert!(!archive.enqueue("late".into(), Operation::Drain));
        assert_eq!(writes.load(Ordering::Acquire), 0);
        gate.notify_one();
        tokio::task::yield_now().await;
        gate.notify_one();
        assert!(archive.finish().await);
        assert_eq!(writes.load(Ordering::Acquire), 2);
    }
    #[tokio::test]
    async fn saturation_keeps_terminal_capacity_and_reports_failure() {
        let archive = Archive::with_writer("capacity".into(), |_| async { true });
        for n in 0..CAPACITY {
            assert!(archive.enqueue(n.to_string(), Operation::Drain));
        }
        assert!(!archive.enqueue("overflow".into(), Operation::Drain));
        archive.seal_with(Some(Operation::Drain));
        assert!(!archive.finish().await);
    }
    #[tokio::test(start_paused = true)]
    async fn five_second_storage_delay_does_not_occupy_producer_or_terminal_submission() {
        let completed = Arc::new(AtomicUsize::new(0));
        let seen = completed.clone();
        let archive = Archive::with_writer("delayed".into(), move |_| {
            let seen = seen.clone();
            async move {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                seen.fetch_add(1, Ordering::AcqRel);
                true
            }
        });
        assert!(archive.enqueue("checkpoint".into(), Operation::Drain));
        tokio::task::yield_now().await;
        let began = tokio::time::Instant::now();
        for _ in 0..1000 {
            assert!(archive.enqueue("checkpoint".into(), Operation::Drain));
        }
        archive.seal_with(Some(Operation::Drain));
        assert_eq!(tokio::time::Instant::now(), began);
        assert_eq!(completed.load(Ordering::Acquire), 0);
        tokio::time::advance(std::time::Duration::from_secs(5)).await;
        assert!(archive.finish().await);
        assert_eq!(completed.load(Ordering::Acquire), 3);
    }
}
