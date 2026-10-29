//! Translation facts are independent of ASR provider and native write permission.
use crate::contracts::dictionary::DataError as Error;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Purpose {
    #[default]
    Dictation,
    Translation,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Language {
    pub code: &'static str,
    pub label: &'static str,
    pub instruction: &'static str,
}

pub const LANGUAGES: &[Language] = &[
    Language {
        code: "en-US",
        label: "英语（美国）",
        instruction: "American English",
    },
    Language {
        code: "en-GB",
        label: "英语（英国）",
        instruction: "British English",
    },
    Language {
        code: "zh-Hans",
        label: "中文（简体）",
        instruction: "Simplified Chinese",
    },
    Language {
        code: "zh-Hant",
        label: "中文（繁体）",
        instruction: "Traditional Chinese",
    },
    Language {
        code: "ja",
        label: "日语",
        instruction: "Japanese",
    },
    Language {
        code: "ko",
        label: "韩语",
        instruction: "Korean",
    },
    Language {
        code: "fr",
        label: "法语",
        instruction: "French",
    },
    Language {
        code: "de",
        label: "德语",
        instruction: "German",
    },
    Language {
        code: "es",
        label: "西班牙语",
        instruction: "Spanish",
    },
    Language {
        code: "pt",
        label: "葡萄牙语",
        instruction: "Portuguese",
    },
    Language {
        code: "ru",
        label: "俄语",
        instruction: "Russian",
    },
    Language {
        code: "ar",
        label: "阿拉伯语",
        instruction: "Arabic",
    },
    Language {
        code: "hi",
        label: "印地语",
        instruction: "Hindi",
    },
    Language {
        code: "th",
        label: "泰语",
        instruction: "Thai",
    },
    Language {
        code: "vi",
        label: "越南语",
        instruction: "Vietnamese",
    },
    Language {
        code: "it",
        label: "意大利语",
        instruction: "Italian",
    },
    Language {
        code: "nl",
        label: "荷兰语",
        instruction: "Dutch",
    },
    Language {
        code: "tr",
        label: "土耳其语",
        instruction: "Turkish",
    },
    Language {
        code: "pl",
        label: "波兰语",
        instruction: "Polish",
    },
    Language {
        code: "uk",
        label: "乌克兰语",
        instruction: "Ukrainian",
    },
    Language {
        code: "id",
        label: "印度尼西亚语",
        instruction: "Indonesian",
    },
    Language {
        code: "ms",
        label: "马来语",
        instruction: "Malay",
    },
];
pub fn language(code: &str) -> Option<&'static Language> {
    LANGUAGES.iter().find(|language| language.code == code)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub targets: Vec<String>,
    pub active_target: String,
    pub revision: i64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            targets: ["en-US", "ja", "ko", "fr", "zh-Hans"]
                .into_iter()
                .map(String::from)
                .collect(),
            active_target: "en-US".into(),
            revision: 1,
        }
    }
}
impl Settings {
    pub fn validate(&self) -> Result<(), Error> {
        let unique: std::collections::HashSet<_> = self.targets.iter().collect();
        if self.revision < 1
            || self.targets.is_empty()
            || unique.len() != self.targets.len()
            || self.targets.iter().any(|code| language(code).is_none())
            || !self.targets.contains(&self.active_target)
        {
            return Err(Error::new("invalidTranslationSettings"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub enum DbOperation {
    Load,
    Save(Settings),
}
#[derive(Debug)]
pub enum DbOutput {
    Settings(Settings),
}

/// A task-owned selection, separate from persistent defaults. Only the recording
/// owner may change it; ending the recording freezes the last accepted choice.
#[derive(Clone, Debug)]
pub struct Target {
    code: String,
    frozen: bool,
}
impl Target {
    pub fn new(code: String) -> Result<Self, Error> {
        language(&code).ok_or_else(|| Error::new("invalidTranslationLanguage"))?;
        Ok(Self {
            code,
            frozen: false,
        })
    }
    pub fn frozen(&self) -> bool {
        self.frozen
    }
    pub fn select(&mut self, code: String, allowed: &[String]) -> Result<(), Error> {
        if self.frozen {
            return Err(Error::new("staleSession"));
        }
        if language(&code).is_none() || !allowed.contains(&code) {
            return Err(Error::new("invalidTranslationLanguage"));
        }
        self.code = code;
        Ok(())
    }
    pub fn freeze(&mut self) -> &str {
        self.frozen = true;
        &self.code
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_reject_empty_duplicate_unknown_and_missing_selection() {
        assert!(Settings::default().validate().is_ok());
        for targets in [vec![], vec!["en-US", "en-US"], vec!["unknown"], vec!["ja"]] {
            assert!(
                Settings {
                    targets: targets.into_iter().map(String::from).collect(),
                    ..Default::default()
                }
                .validate()
                .is_err()
            );
        }
    }
    #[test]
    fn all_catalog_languages_are_valid_without_a_favorites_ceiling() {
        let settings = Settings {
            targets: LANGUAGES
                .iter()
                .map(|language| language.code.into())
                .collect(),
            ..Default::default()
        };
        assert!(settings.targets.len() > 5);
        settings.validate().unwrap();
    }
    #[test]
    fn accepted_recording_choice_is_frozen_once_at_finish() {
        let mut target = Target::new("en-US".into()).unwrap();
        let allowed = vec!["en-US".into(), "ja".into()];
        target.select("ja".into(), &allowed).unwrap();
        assert_eq!(target.freeze(), "ja");
        assert_eq!(
            target.select("en-US".into(), &allowed).unwrap_err().code,
            "staleSession"
        );
        assert_eq!(target.freeze(), "ja");
    }
}
