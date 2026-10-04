use super::types::*;
use crate::network::RequestCoordinator;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
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
    pub requests: Arc<RequestCoordinator>,
}
impl Provider {
    pub fn new(requests: Arc<RequestCoordinator>) -> Result<Self, ModelError> {
        Self::with_requests(
            Self::builder()
                .build()
                .map_err(|_| ModelError::new("unavailable"))?,
            requests,
        )
    }
    pub fn builder() -> reqwest::ClientBuilder {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
    }
    #[cfg(test)]
    pub fn with_client(client: reqwest::Client) -> Result<Self, ModelError> {
        Self::with_requests(client, Arc::new(RequestCoordinator::default()))
    }
    pub fn with_requests(
        client: reqwest::Client,
        requests: Arc<RequestCoordinator>,
    ) -> Result<Self, ModelError> {
        Ok(Self { client, requests })
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
