//! Bounded client-coordinate evidence for receipt-backed range handoff.
//! These observations are not operation acknowledgements or write permission.
use super::*;
use windows::Win32::{
    System::Threading::{GetCurrentProcessId, GetCurrentThreadId},
    UI::Input::KeyboardAndMouse::GetFocus,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub text: String,
    pub composition: std::ops::Range<usize>,
    pub target: std::ops::Range<usize>,
}

// Facts from the caller's existing read only. They never authorize a range and
// contain no external text. Keep the exact first predicate that refused pairing.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub(super) struct PairFacts {
    pub reason: &'static str,
    expected: usize,
    client: usize,
    composition: [usize; 2],
    target: [usize; 2],
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FirstContent {
    Ready,
    Unpublished,
    Rejected,
}

impl Snapshot {
    fn parse(buffer: &[u32]) -> Option<Self> {
        let header = buffer.get(..8)?;
        let size = header[0] as usize;
        let offset = header[3] as usize;
        let length = header[2] as usize;
        if size < 32
            || size > buffer.len().checked_mul(4)?
            || header[1] != 0
            || offset < 32
            || !offset.is_multiple_of(2)
            || length > 16384
            || offset.checked_add(length.checked_mul(2)?)? > size
        {
            return None;
        }
        let span = |index: usize| {
            let byte_start = header[index + 1] as usize;
            let count = header[index] as usize;
            let start = byte_start / 2;
            let end = start.checked_add(count)?;
            (byte_start.is_multiple_of(2) && end <= length).then_some(start..end)
        };
        // Caller-owned aligned storage, checked entirely before dereferencing.
        let units = unsafe {
            std::slice::from_raw_parts(
                buffer.as_ptr().cast::<u8>().add(offset).cast::<u16>(),
                length,
            )
        };
        Some(Self {
            text: String::from_utf16(units).ok()?,
            composition: span(4)?,
            target: span(6)?,
        })
    }

    fn covers_own_field(&self, text: &str) -> bool {
        let units = text.encode_utf16().count();
        self.composition == (0..units)
            && self.target == self.composition
            && self.has_own_field_body(text)
    }
    fn has_own_field_body(&self, text: &str) -> bool {
        self.text
            .strip_prefix(text)
            .is_some_and(|tail| tail.chars().all(|c| matches!(c, '\r' | '\n')))
    }
}

pub(super) struct Reader {
    identity: initial_selection::FocusIdentity,
    focus: HWND,
    window: HWND,
}

impl Reader {
    pub(super) unsafe fn capture(context: &ITfContext) -> windows::core::Result<(Self, Snapshot)> {
        unsafe {
            let identity = initial_selection::FocusIdentity::capture()?;
            let focus = GetFocus();
            let mut windows = vec![focus, GetAncestor(focus, GA_ROOT)];
            if let Ok(window) = context.GetActiveView().and_then(|view| view.GetWnd()) {
                windows.push(window);
            }
            for window in windows {
                let reader = Self {
                    identity: identity.clone(),
                    focus,
                    window,
                };
                if let Ok(snapshot) = reader.read() {
                    return Ok((reader, snapshot));
                }
            }
            Err(failure(E_NOINTERFACE))
        }
    }

    fn current(&self) -> bool {
        unsafe {
            let mut process = 0;
            self.identity.matches().unwrap_or(false)
                && GetFocus() == self.focus
                && !self.window.0.is_null()
                && (self.window == self.focus || IsChild(self.window, self.focus).as_bool())
                && GetWindowThreadProcessId(self.window, Some(&mut process)) == GetCurrentThreadId()
                && process == GetCurrentProcessId()
        }
    }

    pub(super) unsafe fn read(&self) -> windows::core::Result<Snapshot> {
        unsafe {
            if !self.current() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let size = SendMessageW(
                self.window,
                WM_IME_REQUEST,
                Some(WPARAM(7)),
                Some(LPARAM(0)),
            )
            .0;
            if !(32..=65536).contains(&size) || size % 2 != 0 {
                return Err(failure(E_NOINTERFACE));
            }
            let mut buffer = vec![0u32; (size as usize).div_ceil(4)];
            buffer[0] = size as u32;
            if !self.current() {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let reply = SendMessageW(
                self.window,
                WM_IME_REQUEST,
                Some(WPARAM(7)),
                Some(LPARAM(buffer.as_mut_ptr() as isize)),
            );
            if !self.current() || reply.0 == 0 {
                return Err(failure(TF_E_DISCONNECTED));
            }
            Snapshot::parse(&buffer).ok_or_else(|| failure(E_INVALIDARG))
        }
    }
}

pub(super) struct Trail {
    baseline: Snapshot,
    client_baseline: bool,
    pending: Option<(u64, String)>,
    observed: Option<(u64, String)>,
    distinct: usize,
    #[cfg(feature = "acceptance")]
    last: Option<serde_json::Value>,
}

impl Trail {
    pub(super) fn pending(&self) -> Option<(u64, String)> {
        self.pending.clone()
    }
    pub(super) fn new(snapshot: Snapshot, native: &str, selection: RangePosition) -> Option<Self> {
        (snapshot.text == native
            && snapshot.composition.is_empty()
            && snapshot.target.is_empty()
            && selection.start == selection.end
            && snapshot.target.start == usize::try_from(selection.start).ok()?)
        .then_some(Self {
            baseline: snapshot,
            client_baseline: false,
            pending: None,
            observed: None,
            distinct: 0,
            #[cfg(feature = "acceptance")]
            last: None,
        })
    }

    // The caller must also validate the independently retained
    // retained original paragraph/point and native snapshot before publication.
    // Client offsets are never interpreted as native endpoints.
    pub(super) fn client_baseline(
        snapshot: Snapshot,
        native: &str,
        selection: RangePosition,
        raw: &str,
    ) -> Option<Self> {
        (matches!(raw, "" | "\n")
            && snapshot.text != native
            && snapshot.composition.is_empty()
            && snapshot.target == snapshot.composition
            && snapshot.target.end <= snapshot.text.encode_utf16().count()
            && selection.start >= 0
            && selection.start == selection.end)
            .then_some(Self {
                baseline: snapshot,
                client_baseline: true,
                pending: None,
                observed: None,
                distinct: 0,
                #[cfg(feature = "acceptance")]
                last: None,
            })
    }

    pub(super) fn baseline_kind(&self) -> &'static str {
        if self.client_baseline {
            "clientCollapsed"
        } else {
            "nativeMatched"
        }
    }

    pub(super) fn written(&mut self, operation: u64, text: &str) {
        self.pending = Some((operation, text.to_owned()));
    }

    pub(super) fn observe(&mut self, snapshot: &Snapshot) -> bool {
        let Some((operation, text)) = self.pending.as_ref() else {
            return false;
        };
        #[cfg(feature = "acceptance")]
        {
            self.last = Some(
                serde_json::json!({"operation": operation, "expectedUnits": text.encode_utf16().count(), "clientUnits": snapshot.text.encode_utf16().count(), "composition": [snapshot.composition.start,snapshot.composition.end], "target": [snapshot.target.start,snapshot.target.end], "coversOwnField":snapshot.covers_own_field(text)}),
            );
        }
        if text.is_empty() || !snapshot.covers_own_field(text) {
            return false;
        }
        if self
            .observed
            .as_ref()
            .is_none_or(|(previous, _)| previous != operation)
        {
            if self
                .observed
                .as_ref()
                .is_none_or(|(_, previous)| previous != text)
            {
                self.distinct += 1;
            }
            self.observed = Some((*operation, text.clone()));
        }
        true
    }

    pub(super) fn observe_pending(&mut self, pending: &(u64, String), snapshot: &Snapshot) -> bool {
        self.pending.as_ref() == Some(pending) && self.observe(snapshot)
    }

    pub(super) fn ready(&self, expected: &str) -> bool {
        self.distinct >= 2
            && self.observed == self.pending
            && self
                .observed
                .as_ref()
                .is_some_and(|(_, text)| text == expected)
            && self.baseline.text != expected
    }
    // Separate from Trail accumulation: an immutable first-write receipt is
    // paired once after its edit lock has returned. A prior identical client
    // value (including only its protocol tail) cannot prove a new transition.
    #[cfg(test)]
    pub(super) fn first_receipt_pair(&self, pending: &(u64, String), snapshot: &Snapshot) -> bool {
        self.first_receipt_facts(pending, snapshot).reason == "paired"
    }
    pub(super) fn first_receipt_facts(
        &self,
        pending: &(u64, String),
        snapshot: &Snapshot,
    ) -> PairFacts {
        let expected = pending.1.encode_utf16().count();
        let reason = if let Some(reason) = self.first_baseline_error(pending) {
            reason
        } else if snapshot.composition != (0..expected) {
            "composition"
        } else if snapshot.target != snapshot.composition {
            "target"
        } else if !snapshot.has_own_field_body(&pending.1) {
            "body"
        } else {
            "paired"
        };
        PairFacts {
            reason,
            expected,
            client: snapshot.text.encode_utf16().count(),
            composition: [snapshot.composition.start, snapshot.composition.end],
            target: [snapshot.target.start, snapshot.target.end],
        }
    }
    fn first_baseline_error(&self, pending: &(u64, String)) -> Option<&'static str> {
        if self.pending.as_ref() != Some(pending) {
            Some("operation")
        } else if pending.1.is_empty() {
            Some("emptyOutput")
        } else if !self.baseline.composition.is_empty() {
            Some("baselineComposition")
        } else if !self.baseline.target.is_empty() {
            Some("baselineTarget")
        } else if self
            .baseline
            .text
            .strip_prefix(&pending.1)
            .is_some_and(|tail| tail.chars().all(|c| matches!(c, '\r' | '\n')))
        {
            Some("baselineEqual")
        } else {
            None
        }
    }
    // Only the native receipt path may replace active clause coverage. All
    // original baseline and complete-field content checks remain mandatory.
    pub(super) fn first_receipt_content(
        &self,
        pending: &(u64, String),
        snapshot: &Snapshot,
    ) -> FirstContent {
        if self.first_baseline_error(pending).is_some() {
            FirstContent::Rejected
        } else if snapshot.has_own_field_body(&pending.1) {
            FirstContent::Ready
        } else if snapshot == &self.baseline {
            FirstContent::Unpublished
        } else {
            FirstContent::Rejected
        }
    }
    #[cfg(feature = "acceptance")]
    pub(super) fn diagnostic(&self) -> serde_json::Value {
        serde_json::json!({"baseline":self.baseline_kind(), "distinct":self.distinct, "pending":self.pending.as_ref().map(|(op,text)|(*op,text.encode_utf16().count())), "observed":self.observed.as_ref().map(|(op,text)|(*op,text.encode_utf16().count())), "last":self.last})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(text: &str, range: std::ops::Range<usize>) -> Snapshot {
        Snapshot {
            text: text.into(),
            composition: range.clone(),
            target: range,
        }
    }
    #[test]
    fn selection_and_prior_composition_are_not_an_empty_baseline() {
        let position = RangePosition { start: 0, end: 0 };
        assert!(Trail::new(snapshot("T", 0..1), "T", position).is_none());
        let mut selected = snapshot("T", 0..0);
        selected.target = 0..1;
        assert!(Trail::new(selected, "T", position).is_none());
    }
    #[test]
    fn same_text_prefix_or_suffix_cannot_become_the_owned_field() {
        assert!(!snapshot("TT\n\n", 1..2).covers_own_field("T"));
        assert!(!snapshot("TT\n\n", 0..1).covers_own_field("T"));
        assert!(snapshot("T\n\n", 0..1).covers_own_field("T"));
    }
    #[test]
    fn repeated_snapshot_does_not_supply_a_distinct_observation() {
        let mut trail = Trail::new(
            snapshot("\n\n", 0..0),
            "\n\n",
            RangePosition { start: 0, end: 0 },
        )
        .unwrap();
        trail.written(1, "周");
        assert!(trail.observe(&snapshot("周\n\n", 0..1)));
        assert!(trail.observe(&snapshot("周\n\n", 0..1)));
        assert!(!trail.ready("周"));
        trail.written(2, "周三");
        assert!(!trail.observe(&snapshot("周\n\n", 0..1)));
        assert!(trail.observe(&snapshot("周三\n\n", 0..2)));
        assert!(trail.ready("周三"));
        trail.written(3, "周三开会");
        assert!(!trail.ready("周三开会"));
    }
    #[test]
    fn first_receipt_pair_never_advances_the_later_observation_trail() {
        let mut trail = Trail::new(
            snapshot("\n\n", 0..0),
            "\n\n",
            RangePosition { start: 0, end: 0 },
        )
        .unwrap();
        trail.written(1, "周三开会");
        let pending = trail.pending().unwrap();
        assert!(trail.first_receipt_pair(&pending, &snapshot("周三开会\n\n", 0..4)));
        assert_eq!(trail.distinct, 0);
        assert!(trail.observed.is_none());
        assert!(!trail.ready("周三开会"));
        assert!(!trail.first_receipt_pair(&pending, &snapshot("周三开会\n\n", 4..4)));
        trail.written(2, "周三开会地点");
        assert!(!trail.first_receipt_pair(&pending, &snapshot("周三开会\n\n", 0..4)));
    }
    #[test]
    fn first_receipt_facts_preserve_each_original_refusal() {
        let mut trail = Trail::new(
            snapshot("\n\n", 0..0),
            "\n\n",
            RangePosition { start: 0, end: 0 },
        )
        .unwrap();
        trail.written(1, "周三开会");
        let pending = trail.pending().unwrap();
        for (value, reason) in [
            (snapshot("周三开会\n\n", 0..4), "paired"),
            (snapshot("周三开会\n\n", 4..4), "composition"),
            (
                Snapshot {
                    text: "周三开会\n\n".into(),
                    composition: 0..4,
                    target: 1..4,
                },
                "target",
            ),
            (snapshot("外来周三开会\n\n", 0..4), "body"),
        ] {
            let facts = trail.first_receipt_facts(&pending, &value);
            assert_eq!(facts.reason, reason);
            assert_eq!(
                trail.first_receipt_pair(&pending, &value),
                reason == "paired"
            );
            assert_eq!(facts.expected, 4);
        }
        trail.written(2, "周三开会");
        assert_eq!(
            trail
                .first_receipt_facts(&pending, &snapshot("周三开会", 0..4))
                .reason,
            "operation"
        );
        assert_eq!(trail.distinct, 0);
        assert!(trail.observed.is_none());
    }
    #[test]
    fn receipt_content_keeps_baseline_and_full_field_refusals_without_a_live_clause() {
        let mut trail = Trail::new(
            snapshot("\n\n", 0..0),
            "\n\n",
            RangePosition { start: 0, end: 0 },
        )
        .unwrap();
        trail.written(1, "T");
        let pending = trail.pending().unwrap();
        assert_eq!(
            trail.first_receipt_content(&pending, &snapshot("T\n\n", 1..1)),
            FirstContent::Ready
        );
        assert_eq!(
            trail.first_receipt_content(&pending, &snapshot("\n\n", 0..0)),
            FirstContent::Unpublished
        );
        for body in ["TT", "XT", "TX", "other", ""] {
            assert_eq!(
                trail.first_receipt_content(&pending, &snapshot(body, 0..0)),
                FirstContent::Rejected
            );
        }
        trail.written(2, "T");
        assert_eq!(
            trail.first_receipt_content(&pending, &snapshot("T", 1..1)),
            FirstContent::Rejected
        );
        assert_eq!(trail.distinct, 0);
        assert!(trail.observed.is_none());

        // External T already present in the client's initial representation
        // cannot become this write's source, including after its own tail is gone.
        for body in ["T", "T\r", "T\n\n", "T\r\n"] {
            let mut prior = Trail::new(
                snapshot(body, 0..0),
                body,
                RangePosition { start: 0, end: 0 },
            )
            .unwrap();
            prior.written(1, "T");
            for current in ["T", "TT", "T\n\n"] {
                assert_eq!(
                    prior
                        .first_receipt_content(&prior.pending().unwrap(), &snapshot(current, 0..0)),
                    FirstContent::Rejected
                );
            }
        }
    }
    #[test]
    fn first_receipt_rejects_a_prior_equal_body_even_with_protocol_tail() {
        for baseline in ["T", "T\r", "T\n\n", "T\r\n"] {
            let mut trail = Trail::new(
                snapshot(baseline, 0..0),
                baseline,
                RangePosition { start: 0, end: 0 },
            )
            .unwrap();
            trail.written(1, "T");
            assert!(!trail.first_receipt_pair(&trail.pending().unwrap(), &snapshot("T\n\n", 0..1)));
        }
    }
    #[test]
    fn first_receipt_cannot_adopt_external_prefix_suffix_or_foreign_target() {
        let mut trail = Trail::new(
            snapshot("\n\n", 0..0),
            "\n\n",
            RangePosition { start: 0, end: 0 },
        )
        .unwrap();
        trail.written(1, "T");
        let pending = trail.pending().unwrap();
        for value in [
            snapshot("TT\n", 0..1),
            snapshot("TT\n", 1..2),
            Snapshot {
                text: "T\n\n".into(),
                composition: 0..1,
                target: 1..1,
            },
        ] {
            assert!(!trail.first_receipt_pair(&pending, &value));
        }
    }
    #[test]
    fn changed_pending_operation_cannot_publish_a_paired_read() {
        let mut trail = Trail::new(
            snapshot("\n\n", 0..0),
            "\n\n",
            RangePosition { start: 0, end: 0 },
        )
        .unwrap();
        trail.written(1, "周");
        let pending = trail.pending().unwrap();
        trail.written(2, "周");
        assert!(!trail.observe_pending(&pending, &snapshot("周\n\n", 0..1)));
        assert_eq!(trail.distinct, 0);
        assert!(trail.observe_pending(&trail.pending().unwrap(), &snapshot("周\n\n", 0..1)));
        trail.written(3, "周三");
        assert!(!trail.ready("周三"));
    }
    #[test]
    fn malformed_document_feed_never_provides_a_snapshot() {
        assert!(Snapshot::parse(&[]).is_none());
        assert!(Snapshot::parse(&[32, 0, 0, 31, 0, 0, 0, 0]).is_none());
        assert!(Snapshot::parse(&[32, 0, 0, 32, 1, 0, 0, 0]).is_none());
        assert!(Snapshot::parse(&[32, 1, 0, 32, 0, 0, 0, 0]).is_none());
        assert!(Snapshot::parse(&[32, 0, 0, 32, 0, 0, 0, 0]).is_some());
    }

    #[test]
    fn different_cached_baselines_still_require_two_distinct_owned_observations() {
        let position = RangePosition { start: 33, end: 33 };
        let mut trail = Trail::client_baseline(
            snapshot("client hint\n\n", 11..11),
            "different native hint\n\n",
            position,
            "\n",
        )
        .unwrap();
        assert_eq!(trail.baseline_kind(), "clientCollapsed");
        assert!(!trail.ready("周"));
        trail.written(1, "周");
        assert!(trail.observe(&snapshot("周\n\n", 0..1)));
        assert!(!trail.ready("周"));
        trail.written(2, "周三");
        assert!(trail.observe(&snapshot("周三\n\n", 0..2)));
        assert!(trail.ready("周三"));
    }

    #[test]
    fn client_baseline_refuses_existing_text_selection_and_prior_composition() {
        let position = RangePosition { start: 33, end: 33 };
        let candidate = snapshot("hint\n\n", 4..4);
        for raw in ["T", "\n\n", " \n"] {
            assert!(Trail::client_baseline(candidate.clone(), "native", position, raw).is_none());
        }
        assert!(
            Trail::client_baseline(candidate.clone(), &candidate.text, position, "\n").is_none()
        );
        assert!(
            Trail::client_baseline(
                candidate.clone(),
                "native",
                RangePosition { start: 1, end: 2 },
                "\n"
            )
            .is_none()
        );
        assert!(
            Trail::client_baseline(snapshot("hint\n\n", 0..1), "native", position, "\n").is_none()
        );
        let mut selected = candidate;
        selected.target = 3..3;
        assert!(Trail::client_baseline(selected, "native", position, "\n").is_none());
        assert!(Trail::client_baseline(snapshot("hint", 5..5), "native", position, "\n").is_none());
    }
}
