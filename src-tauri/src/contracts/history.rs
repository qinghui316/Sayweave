use serde::{Deserialize, Serialize};
pub type Error = crate::contracts::dictionary::DataError;
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    pub device_id: Option<String>,
    pub history_enabled: bool,
    pub audio_enabled: bool,
    pub retention_days: Option<u32>,
    pub revision: i64,
}
impl Preferences {
    pub fn validate(&self) -> Result<(), Error> {
        if self.revision < 1
            || self
                .retention_days
                .is_some_and(|d| ![7, 30, 90].contains(&d))
            || self
                .device_id
                .as_ref()
                .is_some_and(|s| s.len() > 1024 || s.chars().any(char::is_control))
        {
            return Err(Error::new("invalid"));
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Query {
    pub search: String,
    pub page: u32,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Cursor {
    pub created_at: i64,
    pub id: String,
}
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowseQuery {
    pub search: String,
    pub cursor: Option<Cursor>,
    pub page_size: Option<u32>,
    pub ids: Option<Vec<String>>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    pub created_at: i64,
    pub duration_ms: u64,
    pub status: String,
    pub revision: i64,
    pub display_attempt: Option<String>,
    pub display_reference: Option<TextReference>,
    pub preview: String,
    pub truncated: bool,
    pub matched_old_version: bool,
    pub latest_attempt: Option<String>,
    pub latest_status: Option<String>,
    pub latest_error: Option<String>,
    pub audio: Option<Audio>,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Feed {
    pub records: Vec<Summary>,
    pub total: u32,
    pub next_cursor: Option<Cursor>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attempt {
    #[serde(default)]
    pub purpose: crate::contracts::translation::Purpose,
    #[serde(default)]
    pub target_language: Option<String>,
    pub id: String,
    pub record_id: String,
    pub kind: String,
    pub config_id: String,
    pub config_name: String,
    pub model_id: String,
    pub service_host: String,
    #[serde(default)]
    pub source: RecognitionSource,
    pub config_revision: i64,
    pub created_at: i64,
    pub ended_at: Option<i64>,
    pub status: String,
    pub text: String,
    pub error_code: Option<String>,
    pub elapsed_ms: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum RecognitionSource {
    #[default]
    Cloud,
    Local {
        asset_id: String,
        engine: String,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Audio {
    pub id: String,
    pub record_id: String,
    #[serde(skip)]
    pub file_key: String,
    pub status: String,
    pub bytes: u64,
    pub expires_at: Option<i64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub created_at: i64,
    pub ended_at: Option<i64>,
    pub duration_ms: u64,
    pub status: String,
    pub current_attempt: Option<String>,
    pub revision: i64,
    pub attempts: Vec<Attempt>,
    pub audio: Option<Audio>,
    #[serde(default)]
    pub processing: Vec<crate::contracts::processing::RunRecord>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TextReference {
    pub kind: String,
    pub id: String,
    pub attempt_id: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub records: Vec<Record>,
    pub total: u32,
    pub page: u32,
    pub page_size: u32,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub characters: u64,
    pub duration_ms: u64,
    pub count: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Prepare {
    pub record_id: String,
    pub attempt: Attempt,
    pub audio: Option<Audio>,
}
#[derive(Clone, Debug)]
pub enum DbOperation {
    Translation {
        attempt_id: String,
        target: String,
    },
    PruneCancelled {
        attempt_id: String,
        text: String,
    },
    DiscardAudio(String),
    DiscardedAudio,
    DeleteDiscardedAudio(String),
    RecoverDelivery,
    Deliveries(String),
    Delivery {
        task: String,
        attempt: String,
        state: String,
        error: Option<String>,
    },
    Stage {
        attempt_id: String,
        status: String,
    },
    Preferences,
    SavePreferences(Preferences),
    List(Query),
    Browse(BrowseQuery),
    Detail(String),
    Usage,
    Prepare(Box<Prepare>),
    Retry(Box<Attempt>),
    Checkpoint {
        attempt_id: String,
        text: String,
    },
    Finish {
        attempt_id: String,
        status: String,
        text: String,
        error: Option<String>,
        duration_ms: u64,
        elapsed_ms: u64,
    },
    AudioState {
        id: String,
        status: String,
        bytes: u64,
    },
    Pending,
    MarkDelete {
        id: String,
        revision: i64,
    },
    DeleteRecord(String),
    Expired(i64),
    Interrupted,
    Audio(String),
}
#[derive(Debug)]
pub enum DbOutput {
    Deliveries(Vec<DeliveryReport>),
    Preferences(Preferences),
    Page(Page),
    Feed(Feed),
    Record(Record),
    Usage(Usage),
    Audio(Audio),
    Assets(Vec<Audio>),
    Done,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryReport {
    pub attempt_id: String,
    pub state: String,
    pub error: Option<String>,
    pub updated_at: i64,
}
