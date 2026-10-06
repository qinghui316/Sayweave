use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;

#[derive(Debug)]
pub struct RequestError {
    pub code: String,
    pub field: Option<String>,
}
impl RequestError {
    fn new(code: &str) -> Self {
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
    requests: Mutex<HashMap<String, watch::Sender<bool>>>,
    cancelled: Mutex<HashMap<String, Instant>>,
    stopping: AtomicBool,
}
impl RequestCoordinator {
    pub fn cancel(&self, id: &str) -> Result<(), RequestError> {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(RequestError::field("requestId"));
        }
        if let Some(tx) = self
            .requests
            .lock()
            .map_err(|_| RequestError::new("unavailable"))?
            .get(id)
        {
            let _ = tx.send(true);
        } else {
            let mut cancelled = self
                .cancelled
                .lock()
                .map_err(|_| RequestError::new("unavailable"))?;
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
        Ok(())
    }
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        if let Ok(requests) = self.requests.lock() {
            for tx in requests.values() {
                let _ = tx.send(true);
            }
        }
    }
    // Registration precedes draft credential resolution so cancellation also invalidates a waiting vault read.
    pub fn register(self: &Arc<Self>, id: &str) -> Result<RequestGuard, RequestError> {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(RequestError::field("requestId"));
        }
        let mut requests = self
            .requests
            .lock()
            .map_err(|_| RequestError::new("unavailable"))?;
        let mut cancelled = self
            .cancelled
            .lock()
            .map_err(|_| RequestError::new("unavailable"))?;
        cancelled.retain(|_, at| at.elapsed() < Duration::from_secs(30));
        if cancelled.remove(id).is_some() {
            return Err(RequestError::new("cancelled"));
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err(RequestError::new("stopping"));
        }
        if requests.contains_key(id) {
            return Err(RequestError::new("conflict"));
        }
        if requests.len() >= 4 {
            return Err(RequestError::new("busy"));
        }
        let (tx, rx) = watch::channel(false);
        requests.insert(id.into(), tx);
        Ok(RequestGuard {
            coordinator: self.clone(),
            id: id.into(),
            cancel: rx,
        })
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
        if *self.cancel.borrow() {
            Err(RequestError::new("cancelled").into())
        } else {
            tokio::select! {_ = self.cancel.changed()=>Err(RequestError::new("cancelled").into()),
            value = tokio::time::timeout(Duration::from_secs(seconds),future)=>value.unwrap_or_else(|_|Err(RequestError::new("timeout").into()))}
        }
    }
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if let Ok(mut requests) = self.coordinator.requests.lock() {
            requests.remove(&self.id);
        }
    }
}
