use crate::models::types::ModelError;
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;
#[derive(Default)]
pub struct RequestCoordinator {
    requests: Mutex<HashMap<String, watch::Sender<bool>>>,
    cancelled: Mutex<HashMap<String, Instant>>,
    stopping: AtomicBool,
}
impl RequestCoordinator {
    pub fn cancel(&self, id: &str) -> Result<(), ModelError> {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(ModelError::field("requestId"));
        }
        if let Some(tx) = self
            .requests
            .lock()
            .map_err(|_| ModelError::new("unavailable"))?
            .get(id)
        {
            let _ = tx.send(true);
        } else {
            let mut cancelled = self
                .cancelled
                .lock()
                .map_err(|_| ModelError::new("unavailable"))?;
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
    pub fn register(self: &Arc<Self>, id: &str) -> Result<RequestGuard, ModelError> {
        if uuid::Uuid::parse_str(id).is_err() {
            return Err(ModelError::field("requestId"));
        }
        let mut requests = self
            .requests
            .lock()
            .map_err(|_| ModelError::new("unavailable"))?;
        let mut cancelled = self
            .cancelled
            .lock()
            .map_err(|_| ModelError::new("unavailable"))?;
        cancelled.retain(|_, at| at.elapsed() < Duration::from_secs(30));
        if cancelled.remove(id).is_some() {
            return Err(ModelError::new("cancelled"));
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err(ModelError::new("stopping"));
        }
        if requests.contains_key(id) {
            return Err(ModelError::new("conflict"));
        }
        if requests.len() >= 4 {
            return Err(ModelError::new("busy"));
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
    pub async fn run<T>(
        &mut self,
        seconds: u64,
        future: impl std::future::Future<Output = Result<T, ModelError>>,
    ) -> Result<T, ModelError> {
        let result = if *self.cancel.borrow() {
            Err(ModelError::new("cancelled"))
        } else {
            tokio::select! {_ = self.cancel.changed()=>Err(ModelError::new("cancelled")),
            value = tokio::time::timeout(Duration::from_secs(seconds),future)=>value.unwrap_or_else(|_|Err(ModelError::new("timeout")))}
        };
        result.map_err(|mut e| {
            e.request_id = Some(self.id.clone());
            e
        })
    }
}
impl Drop for RequestGuard {
    fn drop(&mut self) {
        if let Ok(mut requests) = self.coordinator.requests.lock() {
            requests.remove(&self.id);
        }
    }
}
