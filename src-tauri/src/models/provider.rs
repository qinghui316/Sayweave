use super::types::*;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::watch;
#[cfg(test)]
#[path = "provider_tests.rs"]
mod tests;
const LIMIT: usize = 1024 * 1024;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub tested_at: i64,
    pub elapsed_ms: u64,
}
pub struct Provider {
    client: reqwest::Client,
    requests: Mutex<HashMap<String, watch::Sender<bool>>>,
    cancelled: Mutex<HashMap<String, Instant>>,
    stopping: AtomicBool,
}
impl Provider {
    pub fn new() -> Result<Self, ModelError> {
        Self::with_client(
            Self::builder()
                .build()
                .map_err(|_| ModelError::new("unavailable"))?,
        )
    }
    pub fn builder() -> reqwest::ClientBuilder {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
    }
    pub fn with_client(client: reqwest::Client) -> Result<Self, ModelError> {
        Ok(Self {
            client,
            requests: Mutex::new(HashMap::new()),
            cancelled: Mutex::new(HashMap::new()),
            stopping: AtomicBool::new(false),
        })
    }
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
            provider: self.clone(),
            id: id.into(),
            cancel: rx,
        })
    }
    async fn response(&self, request: reqwest::RequestBuilder) -> Result<(u16, Value), ModelError> {
        let mut response = request.send().await.map_err(network_error)?;
        let status = response.status().as_u16();
        if response.content_length().is_some_and(|n| n > LIMIT as u64) {
            return Err(ModelError::new("responseTooLarge"));
        }
        let mut body = vec![];
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
            if body.len() + chunk.len() > LIMIT {
                return Err(ModelError::new("responseTooLarge"));
            }
            body.extend_from_slice(&chunk);
        }
        // HTTP classification does not depend on a provider returning valid JSON.
        let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
        Ok((status, value))
    }
    pub async fn models(&self, input: &ConfigInput, key: &str) -> Result<Vec<String>, ModelError> {
        let (status, body) = self
            .response(
                self.client
                    .get(format!("{}/models", input.base_url))
                    .bearer_auth(key),
            )
            .await?;
        http_status(status)?;
        let data = body
            .get("data")
            .and_then(Value::as_array)
            .ok_or(ModelError::new("protocol"))?;
        let mut ids = vec![];
        for item in data {
            let id = item
                .get("id")
                .and_then(Value::as_str)
                .ok_or(ModelError::new("protocol"))?;
            let id = text(id, 256, "modelId").map_err(|_| ModelError::new("protocol"))?;
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        Ok(ids)
    }
    pub async fn text(&self, input: &ConfigInput, key: &str) -> Result<TestResult, ModelError> {
        let started = Instant::now();
        for completion in [false, true] {
            let mut payload = json!({"model":input.model_id,"messages":[{"role":"user","content":"Reply with OK."}],"stream":false});
            payload[if completion {
                "max_completion_tokens"
            } else {
                "max_tokens"
            }] = json!(128);
            let (status, body) = self
                .response(
                    self.client
                        .post(format!("{}/chat/completions", input.base_url))
                        .bearer_auth(key)
                        .json(&payload),
                )
                .await?;
            if !completion && unsupported_max_tokens(status, &body) {
                continue;
            }
            http_status(status)?;
            let message = body
                .pointer("/choices/0/message")
                .and_then(Value::as_object)
                .ok_or(ModelError::new("protocol"))?;
            let content = match message.get("content") {
                None | Some(Value::Null) => return Err(ModelError::new("emptyText")),
                Some(Value::String(v)) => v.as_str(),
                _ => return Err(ModelError::new("protocol")),
            };
            if content.trim().is_empty() {
                return Err(ModelError::new("emptyText"));
            }
            return Ok(TestResult {
                tested_at: crate::data::repository::now(),
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        }
        Err(ModelError::new("protocol"))
    }
}
fn network_error(e: reqwest::Error) -> ModelError {
    ModelError::new(if e.is_timeout() { "timeout" } else { "network" })
}
fn http_status(status: u16) -> Result<(), ModelError> {
    if (200..300).contains(&status) {
        return Ok(());
    }
    Err(ModelError::new(match status {
        401 | 403 => "auth",
        429 => "rateLimit",
        500..=599 => "service",
        _ => "protocol",
    }))
}
fn unsupported_max_tokens(status: u16, body: &Value) -> bool {
    if !matches!(status, 400 | 422) {
        return false;
    }
    let Some(error) = body.get("error") else {
        return false;
    };
    let param = error.get("param").and_then(Value::as_str);
    let code = error.get("code").and_then(Value::as_str);
    // Do not guess from arbitrary raw response text; require parameter plus an explicit unsupported marker.
    param == Some("max_tokens")
        && (code == Some("unsupported_parameter") || code == Some("unsupported_value"))
}
pub struct RequestGuard {
    provider: Arc<Provider>,
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
        if let Ok(mut requests) = self.provider.requests.lock() {
            requests.remove(&self.id);
        }
    }
}
