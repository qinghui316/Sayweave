//! Evidence for coarse host notifications. These ranges never grant write scope.
use super::*;
use sha2::{Digest, Sha256};

const MAX_UNITS: usize = 64 * 1024;
#[derive(PartialEq, Eq)]
struct Fingerprint {
    units: usize,
    hash: [u8; 32],
}
pub(super) struct BoundaryProof {
    prefix: ITfRange,
    suffix: ITfRange,
    before: Fingerprint,
    after: Fingerprint,
}
unsafe fn fingerprint(
    context: &ITfContext,
    range: &ITfRange,
    ec: u32,
    limit: usize,
    phase: &mut &'static str,
) -> windows::core::Result<Fingerprint> {
    unsafe {
        if password(context, range, ec)? {
            *phase = "blockGuardProtected";
            return Err(failure(E_ACCESSDENIED));
        }
        let cursor = range.Clone()?;
        let mut hash = Sha256::new();
        let mut units = 0;
        let mut buffer = [0u16; 512];
        for _ in 0..=MAX_UNITS / buffer.len() {
            if cursor.IsEmpty(ec)?.as_bool() {
                return Ok(Fingerprint {
                    units,
                    hash: hash.finalize().into(),
                });
            }
            let room = buffer.len().min(limit.saturating_sub(units));
            if room == 0 {
                *phase = "blockGuardBudget";
                return Err(failure(E_FAIL));
            }
            let mut copied = 0;
            cursor.GetText(ec, TF_TF_MOVESTART, &mut buffer[..room], &mut copied)?;
            let copied = copied as usize;
            if copied == 0 || copied > room {
                *phase = "blockGuardReadProgress";
                return Err(failure(E_FAIL));
            }
            for value in &buffer[..copied] {
                hash.update(value.to_le_bytes());
            }
            units += copied;
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
            let mut phase = "blockGuardCapture";
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
        phase: &mut &'static str,
    ) -> windows::core::Result<Self> {
        unsafe {
            *phase = "blockGuardPrefixRoot";
            let prefix = context.GetStart(ec)?;
            *phase = "blockGuardPrefixAnchor";
            prefix.ShiftEndToRange(ec, block, TF_ANCHOR_START)?;
            *phase = "blockGuardPrefixGravity";
            prefix.SetGravity(ec, TF_GRAVITY_BACKWARD, TF_GRAVITY_BACKWARD)?;
            *phase = "blockGuardSuffixRoot";
            let suffix = context.GetEnd(ec)?;
            *phase = "blockGuardSuffixAnchor";
            suffix.ShiftStartToRange(ec, block, TF_ANCHOR_END)?;
            *phase = "blockGuardSuffixGravity";
            suffix.SetGravity(ec, TF_GRAVITY_FORWARD, TF_GRAVITY_FORWARD)?;
            *phase = "blockGuardPrefixRead";
            let before = fingerprint(context, &prefix, ec, MAX_UNITS, phase)?;
            *phase = "blockGuardSuffixRead";
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
        phase: &mut &'static str,
    ) -> windows::core::Result<bool> {
        unsafe {
            // Check original anchors, not offsets or a text search. Collapsed,
            // rebased or non-contiguous guards cannot revive the block.
            *phase = "blockPrefixDocument";
            if self
                .prefix
                .CompareStart(ec, &context.GetStart(ec)?, TF_ANCHOR_START)?
                != 0
            {
                return Ok(false);
            }
            *phase = "blockPrefixAnchor";
            if self.prefix.CompareEnd(ec, block, TF_ANCHOR_START)? != 0 {
                return Ok(false);
            }
            *phase = "blockSuffixAnchor";
            if self.suffix.CompareStart(ec, block, TF_ANCHOR_END)? != 0 {
                return Ok(false);
            }
            *phase = "blockSuffixDocument";
            if self
                .suffix
                .CompareEnd(ec, &context.GetEnd(ec)?, TF_ANCHOR_END)?
                != 0
            {
                return Ok(false);
            }
            *phase = "blockPrefixText";
            if fingerprint(context, &self.prefix, ec, MAX_UNITS, phase)? != self.before {
                return Ok(false);
            }
            *phase = "blockSuffixText";
            if fingerprint(
                context,
                &self.suffix,
                ec,
                MAX_UNITS - self.before.units,
                phase,
            )? != self.after
            {
                return Ok(false);
            }
            *phase = "block";
            Ok(true)
        }
    }
}
