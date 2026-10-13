//! Evidence for coarse host notifications. These ranges never grant write scope.
use super::*;
use sha2::{Digest, Sha256};

const MAX_UNITS: usize = 64 * 1024;
#[derive(PartialEq, Eq)]
struct Fingerprint {
    units: usize,
    hash: [u8; 32],
    normalized_units: usize,
    normalized_hash: [u8; 32],
    paragraphs_only: bool,
}
struct FingerprintBuilder {
    units: usize,
    hash: Sha256,
    normalized_units: usize,
    normalized_hash: Sha256,
    previous_cr: bool,
    paragraphs_only: bool,
}
impl FingerprintBuilder {
    fn new() -> Self {
        Self {
            units: 0,
            hash: Sha256::new(),
            normalized_units: 0,
            normalized_hash: Sha256::new(),
            previous_cr: false,
            paragraphs_only: true,
        }
    }
    fn append(&mut self, values: &[u16]) {
        for &value in values {
            self.hash.update(value.to_le_bytes());
            self.units += 1;
            self.paragraphs_only &= matches!(value, 10 | 13);
            if !(self.previous_cr && value == 10) {
                let normalized = if value == 13 { 10u16 } else { value };
                self.normalized_hash.update(normalized.to_le_bytes());
                self.normalized_units += 1;
            }
            self.previous_cr = value == 13;
        }
    }
    fn finish(self) -> Fingerprint {
        Fingerprint {
            units: self.units,
            hash: self.hash.finalize().into(),
            normalized_units: self.normalized_units,
            normalized_hash: self.normalized_hash.finalize().into(),
            paragraphs_only: self.paragraphs_only,
        }
    }
}
impl Fingerprint {
    // Representation diagnostics never change the exact raw-byte comparison.
    fn mismatch(&self, current: &Self, side: &str) -> String {
        let equivalent = self.normalized_units == current.normalized_units
            && self.normalized_hash == current.normalized_hash;
        format!(
            "block{side}Text:b{}a{}p{}{}n{}",
            self.units,
            current.units,
            u8::from(self.paragraphs_only),
            u8::from(current.paragraphs_only),
            u8::from(equivalent),
        )
    }
}
pub(super) struct BoundaryProof {
    prefix: ITfRange,
    suffix: ITfRange,
    before: Fingerprint,
    after: Fingerprint,
}
#[derive(PartialEq, Eq, Debug)]
pub(super) enum BoundaryState {
    Matched,
    Pending,
    Outside,
}
fn suffix_state(original: &Fingerprint, current: &Fingerprint) -> BoundaryState {
    if original == current {
        BoundaryState::Matched
    } else if original.paragraphs_only && current.paragraphs_only {
        BoundaryState::Pending
    } else {
        BoundaryState::Outside
    }
}
unsafe fn fingerprint(
    context: &ITfContext,
    range: &ITfRange,
    ec: u32,
    limit: usize,
    phase: &mut String,
) -> windows::core::Result<Fingerprint> {
    unsafe {
        if password(context, range, ec)? {
            *phase = "blockGuardProtected".into();
            return Err(failure(E_ACCESSDENIED));
        }
        let cursor = range.Clone()?;
        let mut fingerprint = FingerprintBuilder::new();
        let mut buffer = [0u16; 512];
        for _ in 0..=MAX_UNITS / buffer.len() {
            if cursor.IsEmpty(ec)?.as_bool() {
                return Ok(fingerprint.finish());
            }
            let room = buffer.len().min(limit.saturating_sub(fingerprint.units));
            if room == 0 {
                *phase = "blockGuardBudget".into();
                return Err(failure(E_FAIL));
            }
            let mut copied = 0;
            cursor.GetText(ec, TF_TF_MOVESTART, &mut buffer[..room], &mut copied)?;
            let copied = copied as usize;
            if copied == 0 || copied > room {
                *phase = "blockGuardReadProgress".into();
                return Err(failure(E_FAIL));
            }
            fingerprint.append(&buffer[..copied]);
        }
        Err(failure(E_FAIL))
    }
}
impl BoundaryProof {
    pub(super) unsafe fn follow_owned_write(
        &self,
        context: &ITfContext,
        block: &ITfRange,
        ec: u32,
    ) -> windows::core::Result<()> {
        unsafe {
            let mut phase = "blockGuardCapture".into();
            let current = Self::capture(context, block, ec, &mut phase)?;
            if current.before != self.before || current.after != self.after {
                return Err(failure(TF_E_DISCONNECTED));
            }
            self.prefix
                .ShiftStartToRange(ec, &current.prefix, TF_ANCHOR_START)?;
            self.prefix
                .ShiftEndToRange(ec, &current.prefix, TF_ANCHOR_END)?;
            self.suffix
                .ShiftStartToRange(ec, &current.suffix, TF_ANCHOR_START)?;
            self.suffix
                .ShiftEndToRange(ec, &current.suffix, TF_ANCHOR_END)?;
            Ok(())
        }
    }
    pub(super) unsafe fn capture(
        context: &ITfContext,
        block: &ITfRange,
        ec: u32,
        phase: &mut String,
    ) -> windows::core::Result<Self> {
        unsafe {
            *phase = "blockGuardPrefixRoot".into();
            let prefix = context.GetStart(ec)?;
            *phase = "blockGuardPrefixAnchor".into();
            prefix.ShiftEndToRange(ec, block, TF_ANCHOR_START)?;
            *phase = "blockGuardPrefixGravity".into();
            prefix.SetGravity(ec, TF_GRAVITY_BACKWARD, TF_GRAVITY_BACKWARD)?;
            *phase = "blockGuardSuffixRoot".into();
            let suffix = context.GetEnd(ec)?;
            *phase = "blockGuardSuffixAnchor".into();
            suffix.ShiftStartToRange(ec, block, TF_ANCHOR_END)?;
            *phase = "blockGuardSuffixGravity".into();
            suffix.SetGravity(ec, TF_GRAVITY_FORWARD, TF_GRAVITY_FORWARD)?;
            *phase = "blockGuardPrefixRead".into();
            let before = fingerprint(context, &prefix, ec, MAX_UNITS, phase)?;
            *phase = "blockGuardSuffixRead".into();
            let after = fingerprint(context, &suffix, ec, MAX_UNITS - before.units, phase)?;
            Ok(Self {
                prefix,
                suffix,
                before,
                after,
            })
        }
    }
    pub(super) unsafe fn matches(
        &self,
        context: &ITfContext,
        block: &ITfRange,
        ec: u32,
        phase: &mut String,
    ) -> windows::core::Result<BoundaryState> {
        unsafe {
            // Check original anchors, not offsets or a text search. Collapsed,
            // rebased or non-contiguous guards cannot revive the block.
            *phase = "blockPrefixDocument".into();
            if self
                .prefix
                .CompareStart(ec, &context.GetStart(ec)?, TF_ANCHOR_START)?
                != 0
            {
                return Ok(BoundaryState::Outside);
            }
            *phase = "blockPrefixAnchor".into();
            if self.prefix.CompareEnd(ec, block, TF_ANCHOR_START)? != 0 {
                return Ok(BoundaryState::Outside);
            }
            *phase = "blockSuffixAnchor".into();
            if self.suffix.CompareStart(ec, block, TF_ANCHOR_END)? != 0 {
                return Ok(BoundaryState::Outside);
            }
            *phase = "blockSuffixDocument".into();
            if self
                .suffix
                .CompareEnd(ec, &context.GetEnd(ec)?, TF_ANCHOR_END)?
                != 0
            {
                return Ok(BoundaryState::Outside);
            }
            *phase = "blockPrefixText".into();
            let before = fingerprint(context, &self.prefix, ec, MAX_UNITS, phase)?;
            if before != self.before {
                *phase = self.before.mismatch(&before, "Prefix");
                return Ok(BoundaryState::Outside);
            }
            *phase = "blockSuffixText".into();
            let after = fingerprint(
                context,
                &self.suffix,
                ec,
                MAX_UNITS - self.before.units,
                phase,
            )?;
            if after != self.after {
                *phase = self.after.mismatch(&after, "Suffix");
                let state = suffix_state(&self.after, &after);
                if state == BoundaryState::Pending {
                    // Unconfirmed representation is readonly. Only exact
                    // original raw evidence can ever return Matched later.
                    *phase = format!(
                        "blockRepresentationPending:{}",
                        self.after.mismatch(&after, "Suffix")
                    );
                }
                return Ok(state);
            }
            *phase = "block".into();
            Ok(BoundaryState::Matched)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn digest(chunks: &[&str]) -> Fingerprint {
        let mut builder = FingerprintBuilder::new();
        for chunk in chunks {
            builder.append(&chunk.encode_utf16().collect::<Vec<_>>());
        }
        builder.finish()
    }
    #[test]
    fn representation_diagnostic_preserves_raw_refusal_and_chunked_crlf() {
        let original = digest(&["右\r", "\n末"]);
        let normalized = digest(&["右\n末"]);
        assert!(original != normalized);
        assert_eq!(
            original.mismatch(&normalized, "Suffix"),
            "blockSuffixText:b4a3p00n1"
        );
        assert_eq!(original.hash, digest(&["右\r\n末"]).hash);
        assert_eq!(original.normalized_hash, normalized.normalized_hash);
        assert_eq!(digest(&["\r", "\n"]).normalized_units, 1);
        assert_eq!(digest(&["\r", "\r\n"]).normalized_units, 2);
    }
    #[test]
    fn diagnostics_distinguish_added_paragraphs_and_actual_other_content() {
        let empty = digest(&[""]);
        assert_eq!(
            empty.mismatch(&digest(&["\n"]), "Suffix"),
            "blockSuffixText:b0a1p11n0"
        );
        assert_eq!(
            empty.mismatch(&digest(&["字"]), "Suffix"),
            "blockSuffixText:b0a1p10n0"
        );
        for text in [" ", "\u{2028}", "\u{2029}", "\u{000b}"] {
            assert!(!digest(&[text]).paragraphs_only);
        }
        assert!(digest(&["\r\n"]) != digest(&["\n"]));
        assert_ne!(
            digest(&["\n"]).normalized_hash,
            digest(&["\n\n"]).normalized_hash
        );
    }
    #[test]
    fn terminal_mismatch_never_grants_permission_even_if_other_provider_is_stale() {
        let original = digest(&["\n"]);
        assert_eq!(
            suffix_state(&original, &digest(&["\n\n"])),
            BoundaryState::Pending
        );
        assert_eq!(
            suffix_state(&original, &digest(&["\n"])),
            BoundaryState::Matched
        );
        assert_eq!(
            suffix_state(&original, &digest(&["外\n"])),
            BoundaryState::Outside
        );
        assert_eq!(
            suffix_state(&digest(&["右\n"]), &digest(&["右\n\n"])),
            BoundaryState::Outside
        );
    }
}
