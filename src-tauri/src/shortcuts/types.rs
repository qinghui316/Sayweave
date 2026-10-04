use crate::history::types::Error;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub binding: String,
    pub mode: Mode,
    pub revision: i64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            binding: "AltRight".into(),
            mode: Mode::Toggle,
            revision: 1,
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    Toggle,
    Hold,
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub saved: Config,
    pub active: Option<Config>,
    pub capturing: bool,
    pub pending: bool,
    pub error: Option<String>,
    pub captured_binding: Option<String>,
}
#[derive(Clone)]
pub enum DbOperation {
    Load,
    Save(Config),
    Recover,
    Deliveries(String),
    Delivery {
        task: String,
        attempt: String,
        state: String,
        error: Option<String>,
    },
}
#[derive(Debug)]
pub enum DbOutput {
    Config(Config),
    Done,
    Deliveries(Vec<DeliveryReport>),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryReport {
    pub attempt_id: String,
    pub state: String,
    pub error: Option<String>,
    pub updated_at: i64,
}
// A physical primary key plus optional generic modifiers. Sides are meaningful for single modifiers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub key: String,
    pub modifiers: Vec<String>,
}
impl Binding {
    pub fn parse(value: &str) -> Result<Self, Error> {
        if value.len() > 128 {
            return Err(Error::new("invalidShortcut"));
        }
        let parts: Vec<_> = value.split('+').collect();
        if parts.is_empty() || parts.iter().any(|s| s.is_empty()) {
            return Err(Error::new("invalidShortcut"));
        }
        let raw = parts.last().unwrap();
        let key = raw
            .strip_prefix('F')
            .and_then(|v| v.parse::<u8>().ok())
            .map(|n| format!("F{n}"))
            .unwrap_or_else(|| raw.to_string());
        let side = matches!(
            key.as_str(),
            "AltRight" | "AltLeft" | "ControlRight" | "ControlLeft" | "ShiftRight" | "ShiftLeft"
        );
        let ordinary =
            key.starts_with("Key") && key.len() == 4 && key.as_bytes()[3].is_ascii_uppercase()
                || key.starts_with("Digit") && key.len() == 6 && key.as_bytes()[5].is_ascii_digit()
                || key
                    .strip_prefix('F')
                    .and_then(|v| v.parse::<u8>().ok())
                    .is_some_and(|n| (1..=24).contains(&n))
                || matches!(
                    key.as_str(),
                    "Space" | "CapsLock" | "Insert" | "Home" | "End" | "PageUp" | "PageDown"
                );
        if !side && !ordinary {
            return Err(Error::new("invalidShortcut"));
        }
        let mut modifiers = Vec::new();
        for candidate in &parts[..parts.len() - 1] {
            if !["Control", "Alt", "Shift", "Super"].contains(candidate)
                || modifiers.iter().any(|v| v == candidate)
            {
                return Err(Error::new("invalidShortcut"));
            }
            modifiers.push(candidate.to_string());
        }
        if side && !modifiers.is_empty() {
            return Err(Error::new("invalidShortcut"));
        }
        modifiers.sort_by_key(|s| {
            ["Control", "Alt", "Shift", "Super"]
                .iter()
                .position(|v| v == s)
                .unwrap()
        });
        if key == "KeyL" && modifiers.iter().any(|s| s == "Super") {
            return Err(Error::new("invalidShortcut"));
        }
        Ok(Self { key, modifiers })
    }
    pub fn canonical(&self) -> String {
        self.modifiers
            .iter()
            .chain(std::iter::once(&self.key))
            .cloned()
            .collect::<Vec<_>>()
            .join("+")
    }
    pub fn single_modifier(&self) -> bool {
        self.modifiers.is_empty()
            && matches!(
                self.key.as_str(),
                "AltRight"
                    | "AltLeft"
                    | "ControlRight"
                    | "ControlLeft"
                    | "ShiftRight"
                    | "ShiftLeft"
            )
    }
}
impl Config {
    pub fn validate(mut self) -> Result<Self, Error> {
        self.binding = Binding::parse(&self.binding)?.canonical();
        if self.revision < 1 {
            return Err(Error::new("invalidShortcut"));
        }
        Ok(self)
    }
}
