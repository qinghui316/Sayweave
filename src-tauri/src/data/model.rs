pub use crate::contracts::dictionary::*;

impl Kind {
    pub fn table(&self) -> &'static str {
        match self {
            Self::Term => "dictionary_terms",
            Self::Correction => "correction_rules",
        }
    }
    pub fn columns(&self) -> &'static str {
        match self {
            Self::Term => "id,term,category,NULL,revision,created_at,updated_at",
            Self::Correction => "id,heard,NULL,corrected,revision,created_at,updated_at",
        }
    }
    pub fn key_column(&self) -> &'static str {
        match self {
            Self::Term => "normalized_term",
            Self::Correction => "normalized_heard",
        }
    }
}
