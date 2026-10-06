use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    Term,
    Correction,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Input {
    pub kind: Kind,
    pub original: String,
    pub replacement: Option<String>,
    pub category: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    pub id: String,
    pub kind: Kind,
    pub original: String,
    pub category: Option<String>,
    pub replacement: Option<String>,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Query {
    pub kind: Kind,
    pub search: String,
    pub page: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Page {
    pub records: Vec<Record>,
    pub total: u32,
    pub page: u32,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DataError {
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub existing_id: Option<String>,
}
impl DataError {
    pub fn new(code: &str) -> Self {
        Self {
            code: code.into(),
            existing_id: None,
        }
    }
    pub fn duplicate(id: String) -> Self {
        Self {
            code: "duplicate".into(),
            existing_id: Some(id),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: String,
    pub error: Option<DataError>,
    pub schema_version: u32,
    pub normalization_version: u32,
}
pub fn key(text: &str) -> String {
    text.trim()
        .nfc()
        .collect::<String>()
        .to_lowercase()
        .nfc()
        .collect()
}
fn forbidden_character(value: char) -> bool {
    value.is_control() || matches!(value, '\u{2028}' | '\u{2029}')
}
fn text(raw: &str, max: usize) -> Result<String, DataError> {
    if raw.chars().any(forbidden_character) {
        return Err(DataError::new("invalid"));
    }
    let value = raw.trim();
    if value.is_empty() || value.chars().count() > max {
        return Err(DataError::new("invalid"));
    }
    Ok(value.into())
}
pub fn validate(input: Input) -> Result<Input, DataError> {
    let original = text(&input.original, 128)?;
    match input.kind {
        Kind::Term => {
            if input.replacement.is_some() {
                return Err(DataError::new("invalid"));
            }
            if input
                .category
                .as_deref()
                .is_some_and(|s| s.chars().any(forbidden_character))
            {
                return Err(DataError::new("invalid"));
            }
            let category = input
                .category
                .filter(|s| !s.trim().is_empty())
                .map(|s| text(&s, 64))
                .transpose()?;
            Ok(Input {
                original,
                category,
                ..input
            })
        }
        Kind::Correction => {
            if input.category.is_some() {
                return Err(DataError::new("invalid"));
            }
            let replacement = text(
                input
                    .replacement
                    .as_deref()
                    .ok_or(DataError::new("invalid"))?,
                128,
            )?;
            Ok(Input {
                original,
                replacement: Some(replacement),
                ..input
            })
        }
    }
}
pub fn authorize(label: &str) -> Result<(), DataError> {
    if label == "main" {
        Ok(())
    } else {
        Err(DataError::new("forbidden"))
    }
}
