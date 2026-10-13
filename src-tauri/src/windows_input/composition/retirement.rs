//! A connection's completion outlives both its command queue and its STA worker.
use super::{Error, RETAINED, clear_retained};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::watch;

pub(super) struct Retirement {
    pub task: String,
    pub turn: u64,
    requested: AtomicBool,
    result: watch::Sender<Option<Result<(), Error>>>,
}
impl Retirement {
    pub fn new(task: String, turn: u64) -> Arc<Self> {
        Arc::new(Self {
            task,
            turn,
            requested: AtomicBool::new(false),
            result: watch::channel(None).0,
        })
    }
    pub async fn close(&self) -> Result<(), Error> {
        // Out of band: a full/disappearing business queue cannot lose shutdown.
        self.requested.store(true, Ordering::Release);
        let mut result = self.result.subscribe();
        loop {
            if let Some(value) = result.borrow_and_update().clone() {
                return value;
            }
            result
                .changed()
                .await
                .map_err(|_| Error::new("compositionWorker"))?;
        }
    }
}
pub(super) struct WorkerCompletion {
    retirement: Arc<Retirement>,
    finished: bool,
}
impl WorkerCompletion {
    pub fn new(retirement: Arc<Retirement>) -> Self {
        Self {
            retirement,
            finished: false,
        }
    }
    pub fn requested(&self) -> bool {
        self.retirement.requested.load(Ordering::Acquire)
    }
    /// Call only after Host and its activation/COM/pipe resources have dropped.
    pub fn finish(&mut self, result: Result<(), Error>, reason: &'static str) {
        if self.finished {
            return;
        }
        self.finished = true;
        log::info!(target:"sayweave","operation=composition_retired task_id={} turn={} reason={} phase=resourcesReleased result={} error={}",self.retirement.task,self.retirement.turn,reason,if result.is_ok(){"complete"}else{"failed"},result.as_ref().err().map_or("none",|e|e.code.as_str()));
        let success = result.is_ok();
        self.retirement.result.send_replace(Some(result));
        if success {
            clear_retained(&RETAINED, &self.retirement);
        }
    }
}
impl Drop for WorkerCompletion {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(Err(Error::new("compositionWorker")), "workerUnconfirmed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{Command, Connection};
    use super::*;
    use std::sync::{Mutex, mpsc};

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
    }
    fn connection() -> (Arc<Connection>, WorkerCompletion, mpsc::Receiver<Command>) {
        let (sender, receiver) = mpsc::sync_channel(1);
        let retirement = Retirement::new("retirement-test".into(), 1);
        (
            Arc::new(Connection {
                sender,
                retirement: retirement.clone(),
            }),
            WorkerCompletion::new(retirement),
            receiver,
        )
    }
    #[test]
    fn observer_automatic_exit_must_complete_a_concurrent_close() {
        let (connection, mut worker, receiver) = connection();
        runtime().block_on(async {
            let close = connection.close();
            tokio::pin!(close);
            // Deterministically request Close after the worker has selected
            // automatic expiry, before it releases resources. No queue reply.
            tokio::select! { biased;
                _ = &mut close => panic!("resources have not been released"),
                _ = std::future::ready(()) => {}
            }
            assert!(worker.requested());
            drop(receiver);
            worker.finish(Ok(()), "observationEnded");
            assert!(close.await.is_ok());
            assert!(connection.close().await.is_ok());
        });
    }
    #[test]
    fn multiple_waiters_observe_the_same_cleanup_failure() {
        let (connection, mut worker, _receiver) = connection();
        runtime().block_on(async {
            let (first, second, ()) = tokio::join!(connection.close(), connection.close(), async {
                tokio::task::yield_now().await;
                assert!(worker.requested());
                worker.finish(Err(Error::new("compositionCleanup")), "closeRequested");
            });
            assert_eq!(first.unwrap_err().code, "compositionCleanup");
            assert_eq!(second.unwrap_err().code, "compositionCleanup");
        });
    }
    #[test]
    fn dropped_waiter_does_not_cancel_retirement_or_lose_its_result() {
        let (connection, mut worker, _receiver) = connection();
        runtime().block_on(async {
            {
                let close = connection.close();
                tokio::pin!(close);
                tokio::select! { biased;
                    _ = &mut close => panic!("not retired"),
                    _ = std::future::ready(()) => {}
                }
            }
            assert!(worker.requested());
            worker.finish(Ok(()), "observationExpired");
            assert!(connection.close().await.is_ok());
        });
    }
    #[test]
    fn disconnected_queue_does_not_claim_retirement_and_unwind_is_failure() {
        let (connection, worker, receiver) = connection();
        drop(receiver);
        runtime().block_on(async {
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(1), connection.close())
                    .await
                    .is_err()
            );
            drop(worker);
            assert_eq!(
                connection.close().await.unwrap_err().code,
                "compositionWorker"
            );
        });
    }
    #[test]
    fn old_completion_cannot_clear_a_new_retained_connection() {
        let (old, _worker, _receiver) = connection();
        let (new, _new_worker, _new_receiver) = connection();
        let slot = Mutex::new(Some(new.clone()));
        clear_retained(&slot, &old.retirement);
        assert!(Arc::ptr_eq(slot.lock().unwrap().as_ref().unwrap(), &new));
        clear_retained(&slot, &new.retirement);
        assert!(slot.lock().unwrap().is_none());
    }
    #[test]
    fn preparation_cancel_and_deadline_leave_cleanup_owned_by_worker() {
        let (connection, mut worker, _receiver) = connection();
        let gate = super::super::DeliveryGate::default();
        gate.set_preparation_budget(std::time::Duration::from_secs(1));
        runtime().block_on(async {
            let (result, ()) = tokio::join!(
                super::super::wait_retirement(&connection, Some(&gate)),
                async {
                    tokio::task::yield_now().await;
                    assert!(worker.requested());
                    gate.cancel();
                }
            );
            assert_eq!(result.unwrap_err().code, "cancelled");
            let deadline = super::super::DeliveryGate::default();
            deadline.set_preparation_budget(std::time::Duration::ZERO);
            assert_eq!(
                super::super::wait_retirement(&connection, Some(&deadline))
                    .await
                    .unwrap_err()
                    .code,
                "compositionTimeout"
            );
            worker.finish(Ok(()), "closeRequested");
            assert!(connection.close().await.is_ok());
        });
    }
    #[test]
    fn unwinding_releases_resources_before_publishing_failure() {
        struct Resource(Arc<AtomicBool>);
        impl Drop for Resource {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }
        let (connection, worker, receiver) = connection();
        let released = Arc::new(AtomicBool::new(false));
        let proof = released.clone();
        let thread = std::thread::spawn(move || {
            let _completion = worker;
            let _resource = Resource(proof);
            let _receiver = receiver;
            panic!("injected worker unwind");
        });
        runtime().block_on(async {
            assert_eq!(
                connection.close().await.unwrap_err().code,
                "compositionWorker"
            );
            assert!(released.load(Ordering::Acquire));
        });
        assert!(thread.join().is_err());
    }
}
