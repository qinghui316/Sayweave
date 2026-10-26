use crate::contracts::dictionary::DataError;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ModelError {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
}
impl ModelError {
    pub fn new(code: &str) -> Self {
        Self {
            code: code.into(),
            field: None,
            request_id: None,
        }
    }
    pub fn field(field: &str) -> Self {
        Self {
            code: "invalid".into(),
            field: Some(field.into()),
            request_id: None,
        }
    }
}
impl From<DataError> for ModelError {
    fn from(e: DataError) -> Self {
        Self::new(&e.code)
    }
}
pub struct ProtocolCapabilities {
    pub usages: &'static [Usage],
    pub live_text: bool,
    pub text_test: bool,
    pub origin_only: bool,
}
pub fn capabilities(protocol: &str) -> Option<ProtocolCapabilities> {
    match protocol {
        "openai-chat" => Some(ProtocolCapabilities {
            usages: &[Usage::Text],
            live_text: false,
            text_test: true,
            origin_only: false,
        }),
        "qwen-audio-streaming" | "doubao-asr-streaming" => Some(ProtocolCapabilities {
            usages: &[Usage::Speech],
            live_text: true,
            text_test: false,
            origin_only: true,
        }),
        _ => None,
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Usage {
    Speech,
    Text,
}
impl Usage {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Speech => "speech",
            Self::Text => "text",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfigInput {
    pub name: String,
    pub protocol: String,
    pub base_url: String,
    pub model_id: String,
    pub usages: Vec<Usage>,
}
impl ConfigInput {
    pub fn validate(mut self) -> Result<Self, ModelError> {
        self.name = text(&self.name, 64, "name")?;
        self.model_id = text(&self.model_id, 256, "modelId")?;
        let capability = capabilities(&self.protocol).ok_or(ModelError::field("protocol"))?;
        if self.protocol == "doubao-asr-streaming" && self.model_id != "Doubao-Seed-ASR-2.0" {
            return Err(ModelError::field("modelId"));
        }
        self.base_url = base_url(&self.base_url)?;
        if self.usages.is_empty()
            || self.usages.len() > 2
            || (self.usages.len() == 2 && self.usages[0] == self.usages[1])
        {
            return Err(ModelError::field("usages"));
        }
        self.usages.sort_by_key(Usage::key);
        if capability.origin_only {
            if self.usages != vec![Usage::Speech] {
                return Err(ModelError::field("usages"));
            }
            let url =
                reqwest::Url::parse(&self.base_url).map_err(|_| ModelError::field("baseUrl"))?;
            if url.path() != "/" {
                return Err(ModelError::field("baseUrl"));
            }
        }
        Ok(self)
    }
}
pub fn executable(input: &ConfigInput, usage: &Usage) -> bool {
    input.usages.contains(usage)
        && capabilities(&input.protocol).is_some_and(|c| c.usages.contains(usage))
}
pub fn text(raw: &str, max: usize, field: &str) -> Result<String, ModelError> {
    if raw
        .chars()
        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
    {
        return Err(ModelError::field(field));
    }
    let value = raw.trim();
    if value.is_empty() || value.chars().count() > max {
        return Err(ModelError::field(field));
    }
    Ok(value.into())
}
pub fn base_url(raw: &str) -> Result<String, ModelError> {
    let value = raw.trim().trim_end_matches('/');
    if value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(ModelError::field("baseUrl"));
    }
    let url = reqwest::Url::parse(value).map_err(|_| ModelError::field("baseUrl"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(ModelError::field("baseUrl"));
    }
    Ok(url.as_str().trim_end_matches('/').into())
}
pub fn same_origin(a: &str, b: &str) -> bool {
    match (reqwest::Url::parse(a), reqwest::Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin(),
        _ => false,
    }
}
pub fn secret(raw: &str) -> Result<String, ModelError> {
    let value = raw.trim();
    if value.is_empty() || value.len() > 1024 || !value.bytes().all(|b| (0x21..=0x7e).contains(&b))
    {
        return Err(ModelError::field("key"));
    }
    Ok(value.into())
}
// Deliberately neither Debug nor Serialize: drafts/keys must not enter diagnostics or responses.
#[derive(Clone, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum KeyChange {
    Keep,
    Replace { value: String },
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SaveRequest {
    pub input: ConfigInput,
    pub key: KeyChange,
    pub id: Option<String>,
    pub revision: Option<i64>,
    pub expectation: Option<Expectation>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NetworkDraft {
    pub input: ConfigInput,
    pub key: KeyChange,
    pub id: Option<String>,
    pub revision: Option<i64>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DefaultRecord {
    pub usage: Usage,
    pub config_id: Option<String>,
    pub revision: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Replacement {
    pub usage: Usage,
    pub config_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Expectation {
    pub defaults: Vec<DefaultRecord>,
    pub replacements: Vec<Replacement>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoveRequest {
    pub id: String,
    pub revision: i64,
    pub expectation: Expectation,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DefaultRequest {
    pub usage: Usage,
    pub config_id: String,
    pub revision: i64,
}
#[derive(Clone, Debug)]
pub struct StoredConfig {
    pub id: String,
    pub input: ConfigInput,
    pub credential_ref: String,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Clone, Debug)]
pub struct StoredOverview {
    pub configs: Vec<StoredConfig>,
    pub local_configs: Vec<LocalConfigRecord>,
    pub defaults: Vec<DefaultRecord>,
}
/// Local configurations carry neither a transport endpoint nor a credential reference.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalConfigInput {
    pub name: String,
    pub asset_id: String,
    pub engine: String,
    pub device: sayweave_local_asr_protocol::DevicePolicy,
}
impl LocalConfigInput {
    pub fn validate(mut self) -> Result<Self, ModelError> {
        self.name = text(&self.name, 64, "name")?;
        self.asset_id = text(&self.asset_id, 128, "assetId")?;
        self.engine = text(&self.engine, 64, "engine")?;
        Ok(self)
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalConfigRecord {
    pub id: String,
    #[serde(flatten)]
    pub input: LocalConfigInput,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalSaveRequest {
    pub input: LocalConfigInput,
    pub id: Option<String>,
    pub revision: Option<i64>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigRecord {
    pub id: String,
    #[serde(flatten)]
    pub input: ConfigInput,
    pub credential_status: String,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub configs: Vec<ConfigRecord>,
    pub local_configs: Vec<LocalConfigRecord>,
    pub defaults: Vec<DefaultRecord>,
    pub cleanup_pending: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatus {
    pub state: String,
    pub schema_version: u32,
    pub cleanup_pending: bool,
}
#[derive(Clone)]
pub enum DbOperation {
    Overview,
    CommitLocal(LocalSaveRequest),
    Prepare(String),
    Commit {
        input: ConfigInput,
        id: Option<String>,
        revision: Option<i64>,
        expectation: Option<Expectation>,
        credential_ref: String,
    },
    Remove(RemoveRequest),
    SetDefault(DefaultRequest),
    Pending,
    Referenced(String),
    Clear(String),
}
#[derive(Debug)]
pub enum DbOutput {
    Overview(StoredOverview),
    Pending(Vec<String>),
    Referenced(bool),
    Done,
}
pub fn authorize(caller: &str) -> Result<(), ModelError> {
    if caller == "main" {
        Ok(())
    } else {
        Err(ModelError::new("forbidden"))
    }
}
pub fn replacements(view: &StoredOverview, id: &str, usages: &[Usage]) -> Vec<Replacement> {
    view.defaults
        .iter()
        .filter(|d| d.config_id.as_deref() == Some(id) && !usages.contains(&d.usage))
        .map(|d| Replacement {
            usage: d.usage.clone(),
            config_id: {
                let mut candidates = view
                    .configs
                    .iter()
                    .filter(|c| c.id != id && executable(&c.input, &d.usage))
                    .map(|c| (c.created_at, c.id.clone()))
                    .collect::<Vec<_>>();
                if d.usage == Usage::Speech {
                    candidates.extend(
                        view.local_configs
                            .iter()
                            .filter(|c| c.id != id)
                            .map(|c| (c.created_at, c.id.clone())),
                    );
                }
                candidates.sort();
                candidates.into_iter().next().map(|(_, id)| id)
            },
        })
        .collect()
}
