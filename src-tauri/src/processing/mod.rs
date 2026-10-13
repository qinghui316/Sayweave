//! Pure source units and prompt/quality policies. No windows, capture or database.
pub mod draft;
pub mod prompt;
pub mod quality;
pub mod render;
pub mod retention;
pub mod units;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessingError {
    pub code: &'static str,
}
impl ProcessingError {
    pub fn new(code: &'static str) -> Self {
        Self { code }
    }
}

#[cfg(test)]
mod prompt_tests;
