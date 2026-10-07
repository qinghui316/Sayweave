use super::types::*;
use crate::contracts::processing::{GeneratedText, Message};
use crate::network::RequestCoordinator;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, hash_map::RandomState},
    hash::{BuildHasher, Hash, Hasher},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
#[cfg(test)]
#[path = "provider_tests.rs"]
pub(crate) mod tests;
const LIMIT: usize = 1024 * 1024;
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TestResult {
    pub tested_at: i64,
    pub elapsed_ms: u64,
}
pub struct Provider {
    client: reqwest::Client,
    parameters: Mutex<HashMap<(String, String, u64, u64), super::latency::Parameters>>,
    cache_epoch: AtomicU64,
    cache_hash: RandomState,
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
        Ok(Self {
            client,
            requests,
            parameters: Mutex::new(HashMap::new()),
            cache_epoch: AtomicU64::new(1),
            cache_hash: RandomState::new(),
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
        {
            let response = self.chat(input,key,json!({"model":input.model_id,"messages":[{"role":"user","content":"Reply with OK."}],"stream":false}),128).await?;
            let body = read_json(response).await?;
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
            Ok(TestResult {
                tested_at: crate::data::repository::now(),
                elapsed_ms: started.elapsed().as_millis() as u64,
            })
        }
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
        {
            let mut response = self
                .chat(
                    input,
                    key,
                    json!({"model":input.model_id,"messages":messages,"stream":true}),
                    2048,
                )
                .await?;
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
            Ok(GeneratedText {
                text,
                usage,
                first_output_ms,
                elapsed_ms: began.elapsed().as_millis() as u64,
            })
        }
    }
}
impl Provider {
    pub fn invalidate_parameters(&self) {
        self.cache_epoch.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut cache) = self.parameters.lock() {
            cache.clear();
        }
    }
    async fn chat(
        &self,
        input: &ConfigInput,
        key: &str,
        base: Value,
        tokens: u32,
    ) -> Result<reqwest::Response, ModelError> {
        let mut hash = self.cache_hash.build_hasher();
        key.hash(&mut hash);
        input.name.hash(&mut hash);
        let endpoint = reqwest::Url::parse(&input.base_url)
            .map_err(|_| ModelError::new("invalid"))?
            .to_string();
        // Random per-process credential fingerprint is never logged or persisted.
        let identity = (
            endpoint,
            input.model_id.clone(),
            self.cache_epoch.load(Ordering::Acquire),
            hash.finish(),
        );
        let mut parameters = self
            .parameters
            .lock()
            .ok()
            .and_then(|c| c.get(&identity).cloned())
            .unwrap_or_else(|| {
                super::latency::Parameters::initial(&input.base_url, &input.model_id)
            });
        let mut thinking_adjusted = false;
        for _ in 0..3 {
            let mut body = base.clone();
            parameters.apply(&mut body, tokens);
            let response = self
                .client
                .post(format!("{}/chat/completions", input.base_url))
                .bearer_auth(key)
                .json(&body)
                .send()
                .await
                .map_err(network_error)?;
            let status = response.status().as_u16();
            if response.content_length().is_some_and(|n| n > LIMIT as u64) {
                return Err(ModelError::new("responseTooLarge"));
            }
            if (200..300).contains(&status) {
                if let Ok(mut cache) = self.parameters.lock() {
                    if cache.len() >= 256 {
                        cache.clear();
                    }
                    cache.insert(identity, parameters);
                }
                return Ok(response);
            }
            let error = read_json(response).await?;
            if parameters.adjust(status, &error, &mut thinking_adjusted) {
                continue;
            }
            http_status(status)?;
        }
        Err(ModelError::new("protocol"))
    }
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
async fn read_json(mut response: reqwest::Response) -> Result<Value, ModelError> {
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if bytes.len() + chunk.len() > LIMIT {
            return Err(ModelError::new("responseTooLarge"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&bytes).unwrap_or(Value::Null))
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
pub(super) fn unsupported_max_tokens(status: u16, body: &Value) -> bool {
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
