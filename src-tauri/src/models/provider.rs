use super::types::*;
use crate::contracts::processing::{GeneratedText, Message};
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
    /// Generates one immutable unit. The caller owns cancellation and the total
    /// queue/credential/network deadline; no transport retries occur here.
    pub async fn generate(
        &self,
        input: &ConfigInput,
        key: &str,
        messages: &[Message],
    ) -> Result<GeneratedText, ModelError> {
        if !executable(input, &Usage::Text) || messages.is_empty() {
            return Err(ModelError::new("unsupported"));
        }
        if serde_json::to_vec(messages)
            .map_err(|_| ModelError::new("invalid"))?
            .len()
            > 12 * 1024
        {
            return Err(ModelError::new("promptTooLarge"));
        }
        let began = Instant::now();
        for completion in [false, true] {
            let mut payload = json!({"model":input.model_id,"messages":messages,"stream":true});
            payload[if completion {
                "max_completion_tokens"
            } else {
                "max_tokens"
            }] = json!(2048);
            let mut response = self
                .client
                .post(format!("{}/chat/completions", input.base_url))
                .bearer_auth(key)
                .json(&payload)
                .send()
                .await
                .map_err(network_error)?;
            let status = response.status().as_u16();
            if response.content_length().is_some_and(|n| n > LIMIT as u64) {
                return Err(ModelError::new("responseTooLarge"));
            }
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|s| s.to_str().ok())
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            if !(200..300).contains(&status) {
                let mut body = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(network_error)? {
                    if body.len() + chunk.len() > LIMIT {
                        return Err(ModelError::new("responseTooLarge"));
                    }
                    body.extend_from_slice(&chunk);
                }
                let value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                if !completion && unsupported_max_tokens(status, &value) {
                    continue;
                }
                http_status(status)?;
            }
            let mut first_output_ms = None;
            let (text, usage) = if content_type == "text/event-stream" {
                let mut decoder = super::streaming::Decoder::default();
                while let Some(chunk) = response.chunk().await.map_err(network_error)? {
                    if decoder.push(&chunk)? > 0 && first_output_ms.is_none() {
                        first_output_ms = Some(began.elapsed().as_millis() as u64);
                    }
                    if decoder.is_done() {
                        break;
                    }
                }
                decoder.finish()?
            } else if content_type == "application/json" || content_type.ends_with("+json") {
                let mut body = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(network_error)? {
                    if body.len() + chunk.len() > LIMIT {
                        return Err(ModelError::new("responseTooLarge"));
                    }
                    body.extend_from_slice(&chunk);
                }
                super::streaming::complete_json(&body)?
            } else {
                return Err(ModelError::new("protocol"));
            };
            return Ok(GeneratedText {
                text,
                usage,
                first_output_ms,
                elapsed_ms: began.elapsed().as_millis() as u64,
            });
        }
        Err(ModelError::new("protocol"))
    }
}
impl Provider {
    pub async fn generate_unit(
        &self,
        input: &ConfigInput,
        key: &str,
        request_id: &str,
        messages: &[Message],
        deadline: tokio::time::Instant,
    ) -> Result<GeneratedText, ModelError> {
        let mut request = self.requests.register_until(request_id, deadline).await?;
        request
            .run_until(deadline, self.generate(input, key, messages))
            .await
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
