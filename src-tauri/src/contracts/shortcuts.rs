use crate::contracts::dictionary::DataError as Error;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub binding: String,
    pub translation_binding: String,
    pub mode: Mode,
    pub revision: i64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            binding: "AltRight".into(),
            translation_binding: "AltRight+ShiftRight".into(),
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
    pub sequence: u64,
    pub capture_token: Option<String>,
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
}
#[derive(Debug)]
pub enum DbOutput {
    Config(Config),
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
        // Keep the physical sides of the modifier-only gesture. No timer decides
        // whether this is an ordinary tap or the translation chord.
        if parts.len() == 2 && parts.contains(&"AltRight") && parts.contains(&"ShiftRight") {
            return Ok(Self {
                key: "ShiftRight".into(),
                modifiers: vec!["AltRight".into()],
            });
        }
        if parts.len() == 2 && parts.contains(&"AltLeft") && parts.contains(&"ShiftLeft") {
            return Err(Error::new("shortcutSystemReserved"));
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
                    "Space"
                        | "CapsLock"
                        | "Insert"
                        | "Home"
                        | "End"
                        | "PageUp"
                        | "PageDown"
                        | "Tab"
                        | "Escape"
                        | "Delete"
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
    pub fn modifier_pair(&self) -> bool {
        self.key == "ShiftRight" && self.modifiers == ["AltRight"]
    }
    pub fn system_reserved(&self) -> bool {
        self.modifiers.iter().any(|m| m == "Super")
            || self.key == "F12"
            || self.modifiers.is_empty()
                && matches!(
                    self.key.as_str(),
                    "AltLeft" | "ControlLeft" | "ControlRight" | "ShiftLeft" | "ShiftRight"
                )
            || self.modifiers == ["Alt"]
                && (matches!(self.key.as_str(), "Tab" | "Escape" | "F4" | "Space")
                    || self.key.starts_with("Key")
                    || self.key.starts_with("Digit"))
            || self.modifiers == ["Alt", "Shift"]
                && matches!(self.key.as_str(), "Tab" | "Escape" | "F4")
            || matches!(self.key.as_str(), "Escape")
                && (self.modifiers == ["Control"] || self.modifiers == ["Control", "Shift"])
            || self.key == "Delete" && self.modifiers == ["Control", "Alt"]
    }
    pub fn is_pair_prefix(&self, pair: &Self) -> bool {
        pair.modifier_pair()
            && self.single_modifier()
            && matches!(self.key.as_str(), "AltRight" | "ShiftRight")
    }
}
impl Config {
    pub fn validate(mut self) -> Result<Self, Error> {
        self.binding = Binding::parse(&self.binding)?.canonical();
        let ordinary = Binding::parse(&self.binding)?;
        let translation = Binding::parse(&self.translation_binding)?;
        if ordinary.system_reserved() || translation.system_reserved() {
            return Err(Error::new("shortcutSystemReserved"));
        }
        let pair_prefix =
            ordinary.is_pair_prefix(&translation) || translation.is_pair_prefix(&ordinary);
        if self.mode == Mode::Hold && (ordinary.modifier_pair() || translation.modifier_pair()) {
            // This modifier-only gesture is resolved on release. Hold remains
            // available with ordinary non-overlapping key bindings.
            return Err(Error::new("shortcutHoldConflict"));
        }
        let side_group = |key: &str| {
            ["Control", "Alt", "Shift"]
                .into_iter()
                .find(|group| key.starts_with(group))
        };
        if ordinary == translation
            || ordinary.modifier_pair() && translation.modifiers.iter().any(|m| m == "Alt")
            || translation.modifier_pair() && ordinary.modifiers.iter().any(|m| m == "Alt")
            || !pair_prefix
                && (ordinary.key == translation.key
                    || side_group(&translation.key)
                        .is_some_and(|group| ordinary.modifiers.iter().any(|m| m == group))
                    || side_group(&ordinary.key)
                        .is_some_and(|group| translation.modifiers.iter().any(|m| m == group)))
        {
            return Err(Error::new("shortcutConflict"));
        }
        self.translation_binding = translation.canonical();
        if self.revision < 1 {
            return Err(Error::new("invalidShortcut"));
        }
        Ok(self)
    }
}
