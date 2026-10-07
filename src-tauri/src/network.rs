use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Notify, watch};

#[derive(Debug)]
pub struct RequestError {
    pub code: String,
    pub field: Option<String>,
}
impl RequestError {
    pub(crate) fn new(code: &str) -> Self {
        Self {
            code: code.into(),
            field: None,
        }
    }
    fn field(field: &str) -> Self {
        Self {
            code: "invalid".into(),
            field: Some(field.into()),
        }
    }
}
#[derive(Default)]
pub struct RequestCoordinator {
    state: Mutex<RequestState>,
    available: Notify,
    stopping: AtomicBool,
}
#[derive(Default)]
struct RequestState {
    requests: HashMap<String, watch::Sender<bool>>,
    waiting: HashMap<String, watch::Sender<bool>>,
    cancelled: HashMap<String, Instant>,
}
impl RequestCoordinator {
    pub fn cancel(&self, id: &str) -> Result<(), RequestError> {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(RequestError::field("requestId"));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| RequestError::new("unavailable"))?;
        if let Some(tx) = state.requests.get(id).or_else(|| state.waiting.get(id)) {
            tx.send_replace(true);
        } else {
            let cancelled = &mut state.cancelled;
            cancelled.retain(|_, at| at.elapsed() < Duration::from_secs(30));
            if cancelled.len() >= 64
                && let Some(old) = cancelled
                    .iter()
                    .min_by_key(|(_, at)| **at)
                    .map(|(id, _)| id.clone())
            {
                cancelled.remove(&old);
            }
            cancelled.insert(id.into(), Instant::now());
        }
        self.available.notify_waiters();
        Ok(())
    }
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        if let Ok(state) = self.state.lock() {
            for tx in state.requests.values().chain(state.waiting.values()) {
                tx.send_replace(true);
            }
        }
        self.available.notify_waiters();
    }
    // Registration precedes draft credential resolution so cancellation also invalidates a waiting vault read.
    pub fn register(self: &Arc<Self>, id: &str) -> Result<RequestGuard, RequestError> {
        self.enroll(id, false)
    }
    fn enroll(self: &Arc<Self>, id: &str, waiting: bool) -> Result<RequestGuard, RequestError> {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(RequestError::field("requestId"));
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| RequestError::new("unavailable"))?;
        let cancelled = &mut state.cancelled;
        cancelled.retain(|_, at| at.elapsed() < Duration::from_secs(30));
        if cancelled.remove(id).is_some() {
            return Err(RequestError::new("cancelled"));
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err(RequestError::new("stopping"));
        }
        if state.requests.contains_key(id) || state.waiting.contains_key(id) {
            return Err(RequestError::new("conflict"));
        }
        if (!waiting && state.requests.len() >= 4) || (waiting && state.waiting.len() >= 64) {
            return Err(RequestError::new("busy"));
        }
        let (tx, rx) = watch::channel(false);
        if waiting {
            state.waiting.insert(id.into(), tx);
        } else {
            state.requests.insert(id.into(), tx);
        }
        Ok(RequestGuard {
            coordinator: self.clone(),
            id: id.into(),
            cancel: rx,
        })
    }
    /// One total deadline covers queueing, credentials and transport. Existing
    /// interactive callers retain register's immediate busy result.
    pub async fn register_until(
        self: &Arc<Self>,
        id: &str,
        deadline: tokio::time::Instant,
    ) -> Result<RequestGuard, RequestError> {
        let mut guard = self.enroll(id, true)?;
        loop {
            let notified = self.available.notified();
            tokio::pin!(notified);
            // Enroll before inspecting the slot to avoid a lost release wakeup.
            notified.as_mut().enable();
            if *guard.cancel.borrow() {
                return Err(RequestError::new("cancelled"));
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(RequestError::new("timeout"));
            }
            {
                let mut state = self
                    .state
                    .lock()
                    .map_err(|_| RequestError::new("unavailable"))?;
                if self.stopping.load(Ordering::Acquire) {
                    return Err(RequestError::new("stopping"));
                }
                if state.requests.len() < 4 {
                    let tx = state
                        .waiting
                        .remove(id)
                        .ok_or(RequestError::new("unavailable"))?;
                    if *tx.borrow() {
                        return Err(RequestError::new("cancelled"));
                    }
                    state.requests.insert(id.into(), tx);
                    return Ok(guard);
                }
            }
            tokio::select! {
                biased;
                _ = guard.cancel.changed() => return Err(RequestError::new("cancelled")),
                _ = tokio::time::sleep_until(deadline) => return Err(RequestError::new("timeout")),
                _ = &mut notified => {},
            }
        }
    }
}
pub struct RequestGuard {
    coordinator: Arc<RequestCoordinator>,
    pub id: String,
    cancel: watch::Receiver<bool>,
}
impl RequestGuard {
    pub async fn run<T, E: From<RequestError>>(
        &mut self,
        seconds: u64,
        future: impl std::future::Future<Output = Result<T, E>>,
    ) -> Result<T, E> {
        self.run_until(
            tokio::time::Instant::now() + Duration::from_secs(seconds),
            future,
        )
        .await
    }
    pub async fn run_until<T, E: From<RequestError>>(
        &mut self,
        deadline: tokio::time::Instant,
        future: impl std::future::Future<Output = Result<T, E>>,
    ) -> Result<T, E> {
        if *self.cancel.borrow() {
            Err(RequestError::new("cancelled").into())
        } else if tokio::time::Instant::now() >= deadline {
            Err(RequestError::new("timeout").into())
        } else {
            tokio::select! {biased; _ = self.cancel.changed()=>Err(RequestError::new("cancelled").into()),
            value = tokio::time::timeout_at(deadline,future)=>value.unwrap_or_else(|_|Err(RequestError::new("timeout").into()))}
        }
    }
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if let Ok(mut state) = self.coordinator.state.lock() {
            state.requests.remove(&self.id);
            state.waiting.remove(&self.id);
        }
        self.coordinator.available.notify_waiters();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn id() -> String {
        uuid::Uuid::new_v4().to_string()
    }
    #[tokio::test(start_paused = true)]
    async fn queued_request_shares_deadline_and_does_not_exceed_four_slots() {
        let coordinator = Arc::new(RequestCoordinator::default());
        let mut occupied = (0..4)
            .map(|_| coordinator.register(&id()).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(coordinator.register(&id()).err().unwrap().code, "busy");
        let pending_id = id();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        let waiting = {
            let coordinator = coordinator.clone();
            tokio::spawn(async move { coordinator.register_until(&pending_id, deadline).await })
        };
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(10)).await;
        assert!(!waiting.is_finished());
        occupied.pop();
        let mut admitted = waiting.await.unwrap().unwrap();
        assert_eq!(coordinator.state.lock().unwrap().requests.len(), 4);
        let result = admitted
            .run_until(deadline, async {
                tokio::time::sleep(Duration::from_secs(6)).await;
                Ok::<_, RequestError>(())
            })
            .await;
        assert_eq!(result.unwrap_err().code, "timeout");
        drop(admitted);
        assert_eq!(coordinator.state.lock().unwrap().requests.len(), 3);
    }
    #[tokio::test(start_paused = true)]
    async fn queued_cancel_shutdown_and_timeout_release_registration() {
        for reason in ["cancelled", "stopping", "timeout"] {
            let coordinator = Arc::new(RequestCoordinator::default());
            let _occupied = (0..4)
                .map(|_| coordinator.register(&id()).unwrap())
                .collect::<Vec<_>>();
            let pending_id = id();
            let waiting = {
                let coordinator = coordinator.clone();
                let pending_id = pending_id.clone();
                tokio::spawn(async move {
                    coordinator
                        .register_until(
                            &pending_id,
                            tokio::time::Instant::now() + Duration::from_secs(15),
                        )
                        .await
                })
            };
            tokio::task::yield_now().await;
            match reason {
                "cancelled" => coordinator.cancel(&pending_id).unwrap(),
                "stopping" => coordinator.shutdown(),
                _ => tokio::time::advance(Duration::from_secs(15)).await,
            }
            let error = waiting.await.unwrap().err().unwrap().code;
            // Shutdown sends cancellation to admitted/waiting tasks, while new
            // admission returns stopping. Both stop transport before execution.
            assert_eq!(
                error,
                if reason == "stopping" {
                    "cancelled"
                } else {
                    reason
                }
            );
            assert!(coordinator.state.lock().unwrap().waiting.is_empty());
        }
    }
    #[tokio::test]
    async fn cancelled_before_admission_and_duplicate_waiter_do_not_run() {
        let coordinator = Arc::new(RequestCoordinator::default());
        let request = id();
        coordinator.cancel(&request).unwrap();
        assert_eq!(
            coordinator
                .register_until(
                    &request,
                    tokio::time::Instant::now() + Duration::from_secs(1)
                )
                .await
                .err()
                .unwrap()
                .code,
            "cancelled"
        );
        let _guard = coordinator.register(&request).unwrap();
        assert_eq!(
            coordinator
                .register_until(
                    &request,
                    tokio::time::Instant::now() + Duration::from_secs(1)
                )
                .await
                .err()
                .unwrap()
                .code,
            "conflict"
        );
        coordinator.cancel(&request).unwrap();
        assert!(
            coordinator
                .state
                .lock()
                .unwrap()
                .requests
                .get(&request)
                .unwrap()
                .borrow()
                .to_owned()
        );
    }
}
