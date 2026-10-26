//! Confirmation policy for engine-native append-only tokens, never display stability alone.
#[derive(Default)]
pub struct NativeAppendProof {
    tokens: Vec<i32>,
    confirmed: String,
    retired: bool,
}
impl NativeAppendProof {
    pub fn observe(
        &mut self,
        tokens: &[i32],
        raw: &str,
        full: &str,
        display_committed: &str,
    ) -> Result<String, &'static str> {
        let valid = !self.retired
            && raw == full
            && tokens.starts_with(&self.tokens)
            && full.starts_with(&self.confirmed)
            && full.starts_with(display_committed)
            && (full.is_empty() || !tokens.is_empty());
        if !valid {
            self.retired = true;
            return Err("localSourceDiverged");
        }
        self.tokens = tokens.to_vec();
        self.confirmed = full.into();
        Ok(self.confirmed.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_growth_is_confirmed_from_native_identity() {
        let mut proof = NativeAppendProof::default();
        assert_eq!(proof.observe(&[1], "周", "周", ""), Ok("周".into()));
        assert_eq!(
            proof.observe(&[1, 2], "周三🙂", "周三🙂", "周三"),
            Ok("周三🙂".into())
        );
    }
    #[test]
    fn same_display_text_cannot_restore_changed_native_identity() {
        let mut proof = NativeAppendProof::default();
        proof.observe(&[1], "周", "周", "周").unwrap();
        assert!(proof.observe(&[9], "周", "周", "周").is_err());
        assert!(proof.observe(&[1, 2], "周三", "周三", "周三").is_err());
    }
    #[test]
    fn raw_revisions_and_display_divergence_cannot_manufacture_confirmation() {
        let mut proof = NativeAppendProof::default();
        proof.observe(&[1], "周", "周", "周").unwrap();
        assert!(proof.observe(&[1, 2], "舟三", "舟三", "周").is_err());
        assert!(
            NativeAppendProof::default()
                .observe(&[], "非空", "非空", "")
                .is_err()
        );
        assert!(
            NativeAppendProof::default()
                .observe(&[1], "原始", "别的", "")
                .is_err()
        );
    }
}
