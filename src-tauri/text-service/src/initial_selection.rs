//! Same-thread selection evidence, kept with its provider until native revalidation.
use std::rc::Rc;
use windows::{
    Win32::{
        Foundation::*,
        System::{Com::*, Ole::*, Threading::*, Variant::*},
        UI::{Accessibility::*, Input::KeyboardAndMouse::GetFocus, WindowsAndMessaging::*},
    },
    core::{HRESULT, IUnknown, Interface, Result},
};
#[path = "block_representation.rs"]
mod block_representation;

/// Immutable outside semantic evidence, subordinate to native anchor continuity.
#[derive(Clone)]
pub(super) struct SemanticBoundary {
    identity: IUnknown,
    before: SemanticFingerprint,
    after: SemanticFingerprint,
}
#[derive(Clone, PartialEq, Eq)]
struct SemanticFingerprint {
    bytes: usize,
    hash: [u8; 32],
}
impl SemanticFingerprint {
    fn of(text: &str) -> Self {
        use sha2::{Digest, Sha256};
        Self {
            bytes: text.len(),
            hash: Sha256::digest(text.as_bytes()).into(),
        }
    }
}
pub(super) struct SemanticBlock {
    pub text: String,
    pub native_end: i32,
    pub selection_start: u32,
    pub selection_end: u32,
    pub boundary: SemanticBoundary,
}
pub(super) enum BlockReadError {
    Unproven,
    OutsideScope,
}

struct Array(*mut SAFEARRAY);
/// Identity-only focus evidence. No name, value, text, URL or title is read.
#[derive(Clone)]
pub(super) struct FocusIdentity {
    window: HWND,
    class: String,
    object: Option<FieldIdentity>,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum FocusMatch {
    Current,
    DifferentIdentity,
    Unfocused,
}
#[derive(Clone)]
struct FieldIdentity {
    fragment: FragmentIdentity,
    element: Vec<i32>,
    provider: IRawElementProviderSimple,
}
#[derive(Clone, PartialEq, Eq)]
enum FragmentIdentity {
    Runtime(Vec<i32>),
    Window(usize),
}
impl FocusIdentity {
    pub(super) fn native_identity_text(&self) -> bool {
        // Only the explicitly identified native controls use TSF text itself
        // as semantic text. A failed rich-text provider read is not evidence.
        self.object.is_none()
    }
    pub(super) fn capture() -> Result<Self> {
        unsafe {
            let window = GetFocus();
            let mut process = 0;
            if window.0.is_null()
                || GetWindowThreadProcessId(window, Some(&mut process)) != GetCurrentThreadId()
                || process != GetCurrentProcessId()
            {
                return Err(E_ACCESSDENIED.into());
            }
            let mut class = [0u16; 256];
            let count = GetClassNameW(window, &mut class);
            if count <= 0 {
                return Err(E_FAIL.into());
            }
            let class = String::from_utf16_lossy(&class[..count as usize]);
            if class == "Edit" || class.to_ascii_lowercase().starts_with("richedit") {
                return Ok(Self {
                    window,
                    class,
                    object: None,
                });
            }
            let object = Self::focused_object(window).or_else(|error| {
                // Chromium-style hosts expose their focused editable leaf on
                // the root provider, while the native focus HWND is a child.
                // Use the same-thread root route already used by selection
                // evidence; a generic document/window is still rejected.
                // A known protected/read-only field is an explicit rejection,
                // not a missing provider for which another route may be tried.
                if error.code() == E_ACCESSDENIED {
                    return Err(error);
                }
                let root = GetAncestor(window, GA_ROOT);
                if root == window {
                    // A generic CLIENT without an editable provider is still
                    // unavailable evidence, not a protected-field verdict.
                    return Err(error);
                }
                if !Self::focused_root(root) {
                    return Err(E_ACCESSDENIED.into());
                }
                Self::focused_object(root)
            })?;
            Ok(Self {
                window,
                class,
                object: Some(object),
            })
        }
    }
    unsafe fn focused_root(root: HWND) -> bool {
        unsafe {
            let focus = GetFocus();
            let mut process = 0;
            !focus.0.is_null()
                && GetWindowThreadProcessId(focus, Some(&mut process)) == GetCurrentThreadId()
                && process == GetCurrentProcessId()
                && GetAncestor(focus, GA_ROOT) == root
                && GetForegroundWindow() == root
        }
    }
    unsafe fn focused_object(window: HWND) -> Result<FieldIdentity> {
        unsafe {
            let mut accessible: Option<IAccessible> = None;
            AccessibleObjectFromWindow(
                window,
                OBJID_CLIENT.0 as u32,
                &IAccessible::IID,
                (&mut accessible as *mut Option<IAccessible>).cast(),
            )?;
            let mut accessible =
                accessible.ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?;
            for _ in 0..16 {
                let focus = accessible.accFocus()?;
                match focus.vt() {
                    VT_DISPATCH => {
                        accessible = IDispatch::try_from(&focus)?.cast()?;
                    }
                    VT_I4 | VT_EMPTY => {
                        // A focused MSAA leaf can return EMPTY because it has
                        // no focused child. Verify its role/state, never accept
                        // EMPTY by itself as proof of an editable field.
                        let child = if focus.vt() == VT_I4 {
                            i32::try_from(&focus)?
                        } else {
                            0
                        };
                        if child != 0 {
                            accessible = accessible.get_accChild(&focus)?.cast()?;
                            continue;
                        }
                        let child = VARIANT::from(0i32);
                        let role = i32::try_from(&accessible.get_accRole(&child)?)?;
                        let state = i32::try_from(&accessible.get_accState(&child)?)? as u32;
                        // EMPTY means no focused child, not that a pane/group
                        // is an editable field. A rich group also requires
                        // explicit writable TextPattern evidence below.
                        let rich_edit = editable_focus(role, state)?;
                        return field_identity(&accessible, window, rich_edit);
                    }
                    _ => return Err(E_NOINTERFACE.into()),
                }
            }
            Err(E_FAIL.into())
        }
    }
    pub(super) fn matches(&self) -> Result<bool> {
        self.match_state().map(|state| state == FocusMatch::Current)
    }
    pub(super) fn match_state(&self) -> Result<FocusMatch> {
        if unsafe { GetFocus() } != self.window {
            return Ok(FocusMatch::Unfocused);
        }
        let current = Self::capture()?;
        if current.window != self.window || current.class != self.class {
            return Ok(FocusMatch::Unfocused);
        }
        // Keep and revalidate the original leaf. A same-identity focus flag
        // can lag; it still refuses admission, but does not prove a new field.
        let focused = match (&self.object, &current.object) {
            (None, None) => true,
            (Some(before), Some(after)) => {
                if before.fragment != after.fragment || before.element != after.element {
                    return Ok(FocusMatch::DifferentIdentity);
                }
                unsafe {
                    identity_bool(
                        &before
                            .provider
                            .GetPropertyValue(UIA_HasKeyboardFocusPropertyId)?,
                    )?
                }
            }
            _ => return Ok(FocusMatch::Unfocused),
        };
        Ok(if focused {
            FocusMatch::Current
        } else {
            FocusMatch::Unfocused
        })
    }
}
// IUnknown equality is COM object identity, not UI element identity. Providers
// may return a new wrapper for the same leaf. Keep the provider's runtime ID
// together with its fragment identity and the enclosing native focus HWND.
unsafe fn runtime_id(provider: &IRawElementProviderFragment) -> Result<Option<Vec<i32>>> {
    unsafe {
        let array = Array(provider.GetRuntimeId()?);
        if array.0.is_null() {
            return Ok(None);
        }
        if SafeArrayGetDim(array.0) != 1 || SafeArrayGetVartype(array.0)? != VT_I4 {
            return Err(E_NOINTERFACE.into());
        }
        let low = SafeArrayGetLBound(array.0, 1)?;
        let high = SafeArrayGetUBound(array.0, 1)?;
        let count = identity_array_len(low, high)
            .ok_or_else(|| windows::core::Error::from_hresult(E_INVALIDARG))?;
        let mut values = Vec::with_capacity(count);
        for index in low..=high {
            let mut value = 0i32;
            SafeArrayGetElement(array.0, &index, (&mut value as *mut i32).cast())?;
            values.push(value);
        }
        Ok(Some(values))
    }
}
fn identity_array_len(low: i32, high: i32) -> Option<usize> {
    let count = i64::from(high) - i64::from(low) + 1;
    (1..=64).contains(&count).then_some(count as usize)
}
unsafe fn field_identity(
    accessible: &IAccessible,
    window: HWND,
    rich_edit: bool,
) -> Result<FieldIdentity> {
    unsafe {
        let raw = raw_provider(accessible)?;
        if !identity_bool(&raw.GetPropertyValue(UIA_HasKeyboardFocusPropertyId)?)? {
            return Err(E_ACCESSDENIED.into());
        }
        if rich_edit {
            // contenteditable/ProseMirror expose GROUPING, not MSAA TEXT.
            // A group alone grants nothing: require a focused text provider
            // whose field range explicitly reports writable. No text is read.
            let text: ITextProvider = raw.GetPatternProvider(UIA_TextPatternId)?.cast()?;
            if identity_bool(
                &text
                    .DocumentRange()?
                    .GetAttributeValue(UIA_IsReadOnlyAttributeId)?,
            )? {
                return Err(E_ACCESSDENIED.into());
            }
        }
        let provider: IRawElementProviderFragment = raw.cast()?;
        let element = runtime_id(&provider)?
            .ok_or_else(|| windows::core::Error::from_hresult(E_NOINTERFACE))?;
        let root = provider.FragmentRoot()?;
        let fragment = if let Some(id) = runtime_id(&root.cast()?)? {
            FragmentIdentity::Runtime(id)
        } else {
            // A window-hosted top-level provider has no runtime ID. Bind the
            // fragment to the proven root of this thread's actual focus, never to text.
            let raw: IRawElementProviderSimple = root.cast()?;
            let handle = i32::try_from(&raw.GetPropertyValue(UIA_NativeWindowHandlePropertyId)?)?
                as u32 as usize;
            let native = HWND(handle as *mut _);
            if handle == 0
                || native != GetAncestor(window, GA_ROOT)
                || !FocusIdentity::focused_root(native)
            {
                return Err(E_ACCESSDENIED.into());
            }
            FragmentIdentity::Window(handle)
        };
        Ok(FieldIdentity {
            fragment,
            element,
            provider: raw,
        })
    }
}
fn identity_bool(value: &VARIANT) -> Result<bool> {
    if value.vt() != VT_BOOL {
        return Err(E_NOINTERFACE.into());
    }
    bool::try_from(value)
}
fn editable_focus(role: i32, state: u32) -> Result<bool> {
    if state & (STATE_SYSTEM_PROTECTED | STATE_SYSTEM_READONLY) != 0 {
        return Err(E_ACCESSDENIED.into());
    }
    if state & STATE_SYSTEM_FOCUSED == 0 {
        return Err(E_NOINTERFACE.into());
    }
    if role == ROLE_SYSTEM_TEXT as i32 {
        Ok(false)
    } else if role == ROLE_SYSTEM_GROUPING as i32 {
        Ok(true)
    } else {
        Err(E_NOINTERFACE.into())
    }
}
impl Drop for Array {
    fn drop(&mut self) {
        unsafe {
            let _ = SafeArrayDestroy(self.0);
        }
    }
}
unsafe fn selected(provider: &ITextProvider) -> Result<ITextRangeProvider> {
    unsafe {
        let array = Array(provider.GetSelection()?);
        if array.0.is_null()
            || SafeArrayGetDim(array.0) != 1
            || SafeArrayGetVartype(array.0)? != VT_UNKNOWN
        {
            return Err(E_INVALIDARG.into());
        }
        let index = SafeArrayGetLBound(array.0, 1)?;
        if SafeArrayGetUBound(array.0, 1)? != index {
            return Err(E_INVALIDARG.into());
        }
        let mut result: Option<IUnknown> = None;
        SafeArrayGetElement(
            array.0,
            &index,
            (&mut result as *mut Option<IUnknown>).cast(),
        )?;
        result
            .ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?
            .cast()
    }
}
unsafe fn focused_accessible(window: HWND) -> Result<IAccessible> {
    unsafe {
        let mut accessible: Option<IAccessible> = None;
        AccessibleObjectFromWindow(
            window,
            OBJID_CLIENT.0 as u32,
            &IAccessible::IID,
            (&mut accessible as *mut Option<IAccessible>).cast(),
        )?;
        let mut accessible =
            accessible.ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?;
        for _ in 0..16 {
            let value = accessible.accFocus()?;
            if value.Anonymous.Anonymous.vt == VT_I4 && i32::try_from(&value)? == 0 {
                return Ok(accessible);
            }
            if value.Anonymous.Anonymous.vt != VT_DISPATCH {
                return Err(E_NOINTERFACE.into());
            }
            accessible = IDispatch::try_from(&value)?.cast()?;
        }
        Err(E_FAIL.into())
    }
}
unsafe fn raw_provider(accessible: &IAccessible) -> Result<IRawElementProviderSimple> {
    unsafe {
        accessible.cast().or_else(|_| {
            accessible
                .cast::<IServiceProvider>()?
                .QueryService(&IAccessible::IID)
        })
    }
}
// Exact coordinates are retained with their provider and immutable text, never
// handed to the caller as free-standing offsets.
pub struct Evidence {
    window: HWND,
    accessible_window: HWND,
    identity: IUnknown,
    raw: IRawElementProviderSimple,
    pattern: ITextProvider,
    all: String,
    before: String,
    picked: String,
    tsf: String,
    pub start: i32,
    pub end: i32,
}

// Receipt-backed original paragraph evidence. It never obtains write
// permission and no text search supplies endpoints.
pub(super) struct ParagraphProbe {
    focus: FocusIdentity,
    raw: IRawElementProviderSimple,
    pattern: ITextProvider,
    paragraph: IRawElementProviderSimple,
    identity: Vec<i32>,
    range: ITextRangeProvider,
    sealed_output: Option<String>,
    source_version: Option<u64>,
    source_committed: bool,
    #[cfg(feature = "acceptance")]
    comparison_sampled: std::cell::Cell<u8>,
    #[cfg(feature = "acceptance")]
    comparison_samples: std::cell::RefCell<Vec<serde_json::Value>>,
}
#[derive(serde::Serialize)]
pub(super) struct ParagraphProjection {
    pub text: String,
    pub selected: [i32; 2],
    pub native: String,
    #[serde(skip)]
    pub range: ITextRangeProvider,
}
impl PartialEq for ParagraphProjection {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.selected == other.selected && self.native == other.native
    }
}
impl Eq for ParagraphProjection {}
// Call-scoped metadata only. No provider reads or authorization decisions.
// A missing bit means that check did not run in THIS call. Repeated checks can
// have both success and failure bits; firstFailure is never overwritten.
const SOURCE_CHECKS: &[&str] = &[
    "entry",
    "handoff.inactive",
    "handoff.disconnected",
    "handoff.keyFailure",
    "handoff.cleanup",
    "handoff.ownership",
    "handoff.context",
    "handoff.origin",
    "handoff.pending",
    "handoff.sourceOperation",
    "handoff.trackContext",
    "handoff.trackTask",
    "handoff.expectedText",
    "handoff.blockInvalid",
    "handoff.blockWriting",
    "handoff.blockOrigin",
    "handoff.blockText",
    "handoff.previousSource",
    "native.selection.capture",
    "native.current",
    "native.range",
    "native.enumerate",
    "native.compositionSet",
    "native.field",
    "native.ownedText",
    "native.trackedText",
    "native.rangeStart",
    "native.rangeEnd",
    "native.selection",
    "native.selection.changed",
    "handoff.authority.initial",
    "firstReceipt.nativeReceipt",
    "handoff.authority.awaitFollow",
    "follow.awaitText",
    "handoff.authority.follow",
    "handoff.current.follow",
    "handoff.publishFollow",
    "handoff.current.afterFollow",
    "handoff.authority.observe",
    "handoff.current.observe",
    "handoff.seal",
    "handoff.authority.awaitSeal",
    "firstReceipt.awaitClientBody",
    "handoff.authority.seal",
    "handoff.current.seal",
    "handoff.authority.release",
    "handoff.current.release",
    "handoff.sealed",
    "follow.compare",
    "follow.candidateText",
    "follow.paragraphText",
    "follow.originalAnchor",
    "originalPoint",
    "clientTrail",
    "firstReceipt.spanBefore",
    "firstReceipt.feedRead",
    "firstReceipt.spanAfter",
    "firstReceipt.spanSame",
    "firstReceipt.clientBody",
    "following.source",
    "ownedSpan",
    "proofRead",
    "originalPointRecheck",
    "sealed",
    "poll.authority",
    "poll.final",
    "probe.entry",
    "probe.composition",
    "probe.receipt",
    "follow.current",
    "follow.anchor",
    "follow.clone",
    "follow.moveEnd",
    "follow.final",
    "firstReceipt.final",
    "probe.final",
    "seal.originalPoint",
];
#[derive(Default)]
struct SourceDiagnostics {
    serial: std::cell::Cell<u64>,
    active: std::cell::RefCell<Vec<Rc<std::cell::RefCell<SourceCallData>>>>,
    calls: std::cell::RefCell<Vec<Rc<std::cell::RefCell<SourceCallData>>>>,
}
#[derive(serde::Serialize)]
struct SourceCallData {
    call: u64,
    parent: Option<u64>,
    task: String,
    operation: u64,
    revision: u64,
    entry: &'static str,
    stage: &'static str,
    #[serde(serialize_with = "source_mask")]
    entered: u128,
    #[serde(serialize_with = "source_mask")]
    passed: u128,
    #[serde(serialize_with = "source_mask")]
    failed: u128,
    #[serde(serialize_with = "source_mask")]
    waiting: u128,
    first_failure: Option<&'static str>,
    hr: Option<u32>,
    outcome: &'static str,
    settlement: &'static str,
    text: Option<serde_json::Value>,
    #[serde(skip)]
    pending: Option<usize>,
}
fn source_mask<S: serde::Serializer>(
    value: &u128,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(&format!("{value:x}"))
}
pub(super) struct SourceCall {
    owner: Rc<SourceDiagnostics>,
    data: Rc<std::cell::RefCell<SourceCallData>>,
}
impl SourceDiagnostics {
    fn begin(
        self: &Rc<Self>,
        task: &str,
        operation: u64,
        revision: u64,
        entry: &'static str,
    ) -> SourceCall {
        let call = self.serial.get().saturating_add(1);
        self.serial.set(call);
        let parent = self.active.borrow().last().map(|data| data.borrow().call);
        let data = Rc::new(std::cell::RefCell::new(SourceCallData {
            call,
            parent,
            task: task.into(),
            operation,
            revision,
            entry,
            stage: "entry",
            entered: 1,
            passed: 0,
            failed: 0,
            waiting: 0,
            first_failure: None,
            hr: None,
            outcome: "notRun",
            settlement: "notSubmitted",
            text: None,
            pending: Some(0),
        }));
        let mut calls = self.calls.borrow_mut();
        if calls.len() == 3 {
            calls.remove(0);
        }
        calls.push(data.clone());
        self.active.borrow_mut().push(data.clone());
        SourceCall {
            owner: self.clone(),
            data,
        }
    }
    fn stage(&self, stage: &'static str) {
        if let Some(data) = self.active.borrow().last() {
            let mut data = data.borrow_mut();
            data.stage = stage;
            data.pending = SOURCE_CHECKS.iter().position(|value| *value == stage);
            if let Some(index) = data.pending {
                data.entered |= 1 << index;
            }
        }
    }
    fn passed(&self) {
        if let Some(data) = self.active.borrow().last() {
            let mut data = data.borrow_mut();
            if let Some(index) = data.pending.take() {
                data.passed |= 1 << index;
            }
        }
    }
    fn waiting(&self) {
        if let Some(data) = self.active.borrow().last() {
            let mut data = data.borrow_mut();
            if let Some(index) = data.pending.take() {
                data.waiting |= 1 << index;
            }
        }
    }
    fn rejected(&self) {
        if let Some(data) = self.active.borrow().last() {
            let mut data = data.borrow_mut();
            if let Some(index) = data.pending.take() {
                data.failed |= 1 << index;
            }
            if data.first_failure.is_none() {
                data.first_failure = Some(data.stage);
            }
        }
    }
    fn snapshot(&self) -> serde_json::Value {
        serde_json::json!(
            self.calls
                .borrow()
                .iter()
                .map(|data| serde_json::to_value(&*data.borrow()).unwrap_or_default())
                .collect::<Vec<_>>()
        )
    }
    fn text(&self, actual: &str, expected: &str, baseline: &str) {
        if let Some(data) = self.active.borrow().last() {
            data.borrow_mut().text = Some(serde_json::json!({
                "actualUnits":actual.encode_utf16().count(),
                "expectedUnits":expected.encode_utf16().count(),
                "baselineUnits":baseline.encode_utf16().count(),
                "equalsBaseline":actual==baseline,
                "equalsExpected":actual==expected,
                "expectedThenBaseline":actual==format!("{expected}{baseline}"),
                "baselineThenExpected":actual==format!("{baseline}{expected}"),
            }));
        }
    }
}
impl SourceCall {
    pub(super) fn stage(&self, stage: &'static str) {
        self.owner.stage(stage);
    }
    pub(super) fn finish(&self, outcome: &'static str, hr: Option<HRESULT>) {
        let mut data = self.data.borrow_mut();
        let pending = data.pending.take();
        if let Some(index) = pending
            && hr.is_some()
        {
            data.failed |= 1 << index;
        }
        if hr.is_some() && data.first_failure.is_none() {
            data.first_failure = Some(if pending.is_some() {
                data.stage
            } else {
                "unscopedAfterCheck"
            });
        }
        data.hr = hr.map(|hr| hr.0 as u32);
        data.outcome = outcome;
    }
    pub(super) fn settlement(&self, result: &'static str) {
        self.data.borrow_mut().settlement = result;
    }
    pub(super) fn stage_name(&self) -> &'static str {
        self.data.borrow().stage
    }
    pub(super) fn finish_if_unrun(&self, hr: Option<HRESULT>) {
        if self.data.borrow().outcome == "notRun" {
            self.finish("earlyReturn", hr);
        }
    }
}
impl Drop for SourceCall {
    fn drop(&mut self) {
        self.owner
            .active
            .borrow_mut()
            .retain(|data| !Rc::ptr_eq(data, &self.data));
    }
}
pub(super) struct ParagraphOrigin {
    scope: ParagraphProbe,
    point: ITextRangeProvider,
    start: ITextRangeProvider,
    baseline: String,
    _native_baseline: String,
    live: std::cell::Cell<bool>,
    feed: super::composition_feed::Reader,
    trail: std::cell::RefCell<super::composition_feed::Trail>,
    last_observation: std::cell::Cell<&'static str>,
    seal_note: std::cell::Cell<&'static str>,
    retired_error: std::cell::Cell<u32>,
    first_point_failure: std::cell::Cell<Option<&'static str>>,
    followed_range: std::cell::RefCell<Option<Rc<ITextRangeProvider>>>,
    followed_operation: std::cell::Cell<u64>,
    first_pair: std::cell::Cell<Option<super::composition_feed::PairFacts>>,
    last_pair: std::cell::Cell<Option<super::composition_feed::PairFacts>>,
    first_pair_call: std::cell::Cell<Option<u64>>,
    last_pair_call: std::cell::Cell<Option<u64>>,
    source_diagnostics: Rc<SourceDiagnostics>,
    #[cfg(feature = "acceptance")]
    first_candidate_mismatch: std::cell::RefCell<Option<serde_json::Value>>,
}
pub(super) enum FollowPreparation {
    AwaitText,
    Unavailable,
    Candidate(ITextRangeProvider, u64),
}
impl ParagraphOrigin {
    pub(super) unsafe fn capture(
        context: &windows::Win32::UI::TextServices::ITfContext,
        ec: u32,
        phase: &mut &'static str,
        detail: &mut String,
    ) -> Result<Self> {
        unsafe {
            let scope = ParagraphProbe::capture_scope(phase)?;
            *phase = "baseline";
            let baseline = scope.range.GetText(16385)?.to_string();
            // This is the original observed text, not an empty-field verdict.
            if !matches!(baseline.as_str(), "" | "\n") {
                return Err(E_NOINTERFACE.into());
            }
            *phase = "point";
            let point = selected(&scope.pattern)?.Clone()?;
            let start = scope.range.Clone()?;
            start.MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                &scope.range,
                TextPatternRangeEndpoint_Start,
            )?;
            *phase = "native";
            let document = context.GetStart(ec)?;
            document.ShiftEndToRange(
                ec,
                &context.GetEnd(ec)?,
                windows::Win32::UI::TextServices::TF_ANCHOR_END,
            )?;
            let native = super::range_text(&document, ec)?;
            let selection = super::range_position(context, &super::selection(context, ec)?, ec)?;
            *phase = "feed";
            let (feed, snapshot) = super::composition_feed::Reader::capture(context)?;
            *phase = "trail";
            *detail = format!(
                "nativeUnits={} clientUnits={} rawUnits={} nativeClientEq={} rawClientEq={} composition={:?} target={:?} nativeSelection={}:{}",
                native.encode_utf16().count(),
                snapshot.text.encode_utf16().count(),
                baseline.encode_utf16().count(),
                snapshot.text == native,
                snapshot.text == baseline,
                snapshot.composition,
                snapshot.target,
                selection.start,
                selection.end,
            );
            let raw_baseline_only = snapshot.text != native
                && snapshot.text == baseline
                && snapshot.composition == (0..0)
                && snapshot.target == (0..0)
                && selection.start == selection.end;
            let trail = super::composition_feed::Trail::new(snapshot.clone(), &native, selection)
                .or_else(|| {
                    super::composition_feed::Trail::client_baseline(
                        snapshot, &native, selection, &baseline,
                    )
                })
                .ok_or_else(|| {
                    if raw_baseline_only {
                        *phase = "trail.rawBaselineOnly";
                    }
                    windows::core::Error::from_hresult(E_ACCESSDENIED)
                })?;
            *phase = "nativeRecheck";
            if super::range_text(&document, ec)? != native
                || super::range_position(context, &super::selection(context, ec)?, ec)? != selection
            {
                return Err(E_ACCESSDENIED.into());
            }
            let origin = Self {
                scope,
                point,
                start,
                baseline,
                _native_baseline: native,
                live: std::cell::Cell::new(true),
                feed,
                trail: std::cell::RefCell::new(trail),
                last_observation: std::cell::Cell::new("initial"),
                seal_note: std::cell::Cell::new("notCalled"),
                retired_error: std::cell::Cell::new(0),
                first_point_failure: std::cell::Cell::new(None),
                followed_range: std::cell::RefCell::new(None),
                followed_operation: std::cell::Cell::new(0),
                first_pair: std::cell::Cell::new(None),
                last_pair: std::cell::Cell::new(None),
                first_pair_call: std::cell::Cell::new(None),
                last_pair_call: std::cell::Cell::new(None),
                source_diagnostics: Rc::default(),
                #[cfg(feature = "acceptance")]
                first_candidate_mismatch: std::cell::RefCell::new(None),
            };
            *phase = "initial";
            origin.initial()?;
            Ok(origin)
        }
    }
    pub(super) fn baseline_kind(&self) -> &'static str {
        self.trail.borrow().baseline_kind()
    }
    pub(super) fn source_stage(&self) -> &'static str {
        self.seal_note.get()
    }
    pub(super) fn pair_diagnostic(&self) -> serde_json::Value {
        serde_json::json!({"first":self.first_pair.get(),"firstCall":self.first_pair_call.get(),"last":self.last_pair.get(),"lastCall":self.last_pair_call.get(),"calls":self.source_diagnostics.snapshot()})
    }
    pub(super) fn last_pair_reason(&self) -> Option<&'static str> {
        self.last_pair.get().map(|facts| facts.reason)
    }
    fn record_first_pair(
        &self,
        pending: &(u64, String),
        snapshot: &super::composition_feed::Snapshot,
    ) -> bool {
        let facts = self.trail.borrow().first_receipt_facts(pending, snapshot);
        if self.first_pair.get().is_none() {
            self.first_pair.set(Some(facts));
            self.first_pair_call.set(self.source_call_id());
        }
        self.last_pair.set(Some(facts));
        self.last_pair_call.set(self.source_call_id());
        facts.reason == "paired"
    }
    #[cfg(feature = "acceptance")]
    pub(super) fn diagnostic(&self) -> serde_json::Value {
        serde_json::json!({"live":self.live.get(), "observation":self.last_observation.get(), "seal":self.seal_note.get(), "retiredError":self.retired_error.get(), "firstPointFailure":self.first_point_failure.get(), "candidateMismatch":self.first_candidate_mismatch.borrow().clone(), "trail":self.trail.borrow().diagnostic()})
    }
    pub(super) fn handoff_stage(&self, phase: &'static str) {
        self.seal_note.set(phase);
        self.source_diagnostics.stage(phase);
    }
    pub(super) fn source_call(
        &self,
        task: &str,
        operation: u64,
        revision: u64,
        entry: &'static str,
    ) -> SourceCall {
        self.source_diagnostics
            .begin(task, operation, revision, entry)
    }
    pub(super) fn source_call_id(&self) -> Option<u64> {
        self.source_diagnostics
            .active
            .borrow()
            .last()
            .map(|data| data.borrow().call)
    }
    pub(super) fn source_check_passed(&self) {
        self.source_diagnostics.passed();
    }
    pub(super) fn source_check_failed(&self) {
        self.source_diagnostics.rejected();
    }
    pub(super) fn pending_write(&self) -> Option<(u64, String)> {
        self.trail.borrow().pending()
    }
    pub(super) fn awaiting_write(&self, operation: u64, text: &str) -> bool {
        self.live.get()
            && self
                .pending_write()
                .is_some_and(|pending| pending.0 == operation && pending.1 == text)
    }
    fn current_range(&self) -> Rc<ITextRangeProvider> {
        let followed = self.followed_range.borrow().clone();
        followed.unwrap_or_else(|| Rc::new(self.scope.range.clone()))
    }
    pub(super) fn followed(&self, operation: u64) -> bool {
        self.live.get() && self.followed_operation.get() == operation
    }
    pub(super) fn retire(&self) {
        self.live.set(false);
    }
    unsafe fn original_anchor(&self) -> Result<ITextRangeProvider> {
        let mut phase = "live";
        let result = (|| unsafe {
            let scope = &self.scope;
            if !self.live.get() {
                return Err(E_ACCESSDENIED.into());
            }
            phase = "focus";
            if !scope.focus.matches()? {
                return Err(E_ACCESSDENIED.into());
            }
            phase = "paragraphIdentity";
            if runtime_id(&scope.paragraph.cast::<IRawElementProviderFragment>()?)?.as_ref()
                != Some(&scope.identity)
            {
                return Err(E_ACCESSDENIED.into());
            }
            phase = "password";
            if bool::try_from(&scope.raw.GetPropertyValue(UIA_IsPasswordPropertyId)?)? {
                return Err(E_ACCESSDENIED.into());
            }
            phase = "pointText";
            if !self.point.GetText(16385)?.is_empty() {
                return Err(E_ACCESSDENIED.into());
            }
            phase = "startText";
            if !self.start.GetText(16385)?.is_empty() {
                return Err(E_ACCESSDENIED.into());
            }
            phase = "pointEqualsStart";
            if !self.point.Compare(&self.start)?.as_bool() {
                return Err(E_ACCESSDENIED.into());
            }
            phase = "paragraphRange";
            let paragraph = scope.pattern.RangeFromChild(&scope.paragraph)?;
            phase = "pointStartIsParagraphStart";
            if self.point.CompareEndpoints(
                TextPatternRangeEndpoint_Start,
                &paragraph,
                TextPatternRangeEndpoint_Start,
            )? != 0
            {
                return Err(E_ACCESSDENIED.into());
            }
            phase = "pointEndIsParagraphStart";
            if self.point.CompareEndpoints(
                TextPatternRangeEndpoint_End,
                &paragraph,
                TextPatternRangeEndpoint_Start,
            )? != 0
            {
                return Err(E_ACCESSDENIED.into());
            }
            phase = "scopeEqualsDocument";
            if !paragraph
                .Compare(&scope.pattern.DocumentRange()?)?
                .as_bool()
            {
                return Err(E_ACCESSDENIED.into());
            }
            phase = "readonly";
            if bool::try_from(&paragraph.GetAttributeValue(UIA_IsReadOnlyAttributeId)?)? {
                return Err(E_ACCESSDENIED.into());
            }
            Ok(paragraph)
        })();
        if result.is_err() && self.first_point_failure.get().is_none() {
            self.first_point_failure.set(Some(phase));
        }
        result
    }
    unsafe fn original_point(&self) -> Result<()> {
        unsafe {
            let paragraph = self.original_anchor()?;
            if !self.current_range().Compare(&paragraph)?.as_bool() {
                if self.first_point_failure.get().is_none() {
                    self.first_point_failure.set(Some("scopeEqualsParagraph"));
                }
                return Err(E_ACCESSDENIED.into());
            }
            Ok(())
        }
    }
    // A candidate follows one verified own write. The original range is never
    // changed; only the native caller's final fence may publish this candidate.
    pub(super) unsafe fn prepare_follow(
        &self,
        pending: &(u64, String),
    ) -> Result<FollowPreparation> {
        unsafe {
            self.handoff_stage("follow.current");
            if !self.live.get() || self.pending_write().as_ref() != Some(pending) {
                return Err(E_ACCESSDENIED.into());
            }
            let previous = self.followed_operation.get();
            self.source_check_passed();
            self.handoff_stage("follow.anchor");
            let paragraph = self.original_anchor()?;
            // The native caller verifies the owned composition's end selection.
            // UIA's caret can expose a later structural position after the first
            // write. It must not become the original block's source boundary.
            self.source_check_passed();
            self.handoff_stage("follow.clone");
            let candidate = self.point.Clone()?;
            self.source_check_passed();
            self.handoff_stage("follow.moveEnd");
            candidate.MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                &paragraph,
                TextPatternRangeEndpoint_End,
            )?;
            self.source_check_passed();
            self.handoff_stage("follow.compare");
            if !candidate.Compare(&paragraph)?.as_bool() {
                self.source_diagnostics.rejected();
                return Ok(FollowPreparation::Unavailable);
            }
            self.source_check_passed();
            self.handoff_stage("follow.candidateText");
            let candidate_text = candidate.GetText(16385)?;
            self.source_diagnostics
                .text(&candidate_text.to_string(), &pending.1, &self.baseline);
            if candidate_text != pending.1 {
                self.source_diagnostics.waiting();
                #[cfg(feature = "acceptance")]
                if self.first_candidate_mismatch.borrow().is_none() {
                    *self.first_candidate_mismatch.borrow_mut() = Some(serde_json::json!({
                        "tick":windows::Win32::System::SystemInformation::GetTickCount64(),"operation":pending.0,
                        "candidateUnits":candidate_text.to_string().encode_utf16().take(64).collect::<Vec<_>>(),
                        "expectedUnits":pending.1.encode_utf16().count(),
                    }));
                }
                return Ok(FollowPreparation::AwaitText);
            }
            self.source_check_passed();
            self.handoff_stage("follow.paragraphText");
            if paragraph.GetText(16385)? != pending.1 {
                self.source_diagnostics.rejected();
                return Ok(FollowPreparation::Unavailable);
            }
            self.source_check_passed();
            self.handoff_stage("follow.originalAnchor");
            if !paragraph.Compare(&self.original_anchor()?)?.as_bool() {
                self.source_diagnostics.rejected();
                return Ok(FollowPreparation::Unavailable);
            }
            self.source_check_passed();
            self.handoff_stage("follow.final");
            if self.pending_write().as_ref() != Some(pending)
                || self.followed_operation.get() != previous
                || !self.live.get()
            {
                return Err(E_ACCESSDENIED.into());
            }
            self.source_check_passed();
            Ok(FollowPreparation::Candidate(candidate, previous))
        }
    }
    pub(super) fn publish_follow(
        &self,
        pending: &(u64, String),
        range: ITextRangeProvider,
        previous: u64,
    ) -> bool {
        if !self.live.get()
            || self.pending_write().as_ref() != Some(pending)
            || self.followed_operation.get() != previous
        {
            return false;
        }
        let old = self.followed_range.borrow_mut().replace(Rc::new(range));
        self.followed_operation.set(pending.0);
        // Provider release occurs after the interior borrow is gone.
        drop(old);
        true
    }
    unsafe fn initial(&self) -> Result<()> {
        unsafe {
            self.original_point()?;
            if self.scope.range.GetText(16385)? != self.baseline
                || !self
                    .point
                    .Compare(&selected(&self.scope.pattern)?)?
                    .as_bool()
            {
                return Err(E_ACCESSDENIED.into());
            }
            Ok(())
        }
    }
    pub(super) unsafe fn before_write(&self) {
        if let Err(error) = unsafe { self.initial() } {
            self.last_observation.set("beforeWrite.initial");
            self.retired_error.set(error.code().0 as u32);
            self.retire();
        }
    }
    pub(super) unsafe fn observe_client(&self) -> bool {
        // There is no output to observe before the first write. In particular,
        // do not enter an auxiliary provider/window query inside first-entry
        // keyboard notification before its actual SetText.
        if !self.live.get() {
            return false;
        }
        let pending = self.trail.borrow().pending();
        let Some(pending) = pending else {
            self.last_observation.set("noPending");
            return false;
        };
        let paired = (|| unsafe {
            let Some(range) = self.owned_span(&pending.1)? else {
                return Ok(None);
            };
            self.last_observation.set("feed.read");
            let snapshot = self.feed.read()?;
            let Some(after_range) = self.owned_span(&pending.1)? else {
                return Ok(None);
            };
            if !range.Compare(&after_range)?.as_bool() {
                self.last_observation.set("pairedSpan.changed");
                return Ok(None);
            }
            Ok::<_, windows::core::Error>(Some(snapshot))
        })();
        match paired {
            Ok(Some(snapshot)) if self.live.get() => {
                let accepted = self.trail.borrow_mut().observe_pending(&pending, &snapshot);
                self.last_observation.set(if accepted {
                    "trail.accepted"
                } else {
                    "trail.rejected"
                });
                accepted
            }
            Ok(_) => false,
            Err(error) => {
                self.retired_error.set(error.code().0 as u32);
                self.retire();
                false
            }
        }
    }
    pub(super) fn remember_write(&self, operation: u64, text: &str) {
        if self.live.get() {
            self.trail.borrow_mut().written(operation, text);
        }
    }
    // Pair the client observation with the original paragraph span, never a
    // current caret or a text search. Native ownership is fenced by the caller.
    unsafe fn owned_span(&self, expected: &str) -> Result<Option<ITextRangeProvider>> {
        unsafe {
            self.last_observation.set("ownedSpan.originalPoint");
            self.original_point()?;
            let current = self.current_range();
            let range = self.point.Clone()?;
            self.last_observation.set("ownedSpan.expand");
            range.MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                &*current,
                TextPatternRangeEndpoint_End,
            )?;
            self.last_observation.set("ownedSpan.rangeCompare");
            if !range.Compare(&*current)?.as_bool() {
                return Ok(None);
            }
            self.last_observation.set("ownedSpan.rangeText");
            if range.GetText(16385)? != expected {
                return Ok(None);
            }
            self.last_observation.set("ownedSpan.scopeText");
            if current.GetText(16385)? != expected {
                return Ok(None);
            }
            self.last_observation.set("ownedSpan.originalPointRecheck");
            self.original_point()?;
            Ok(Some(range))
        }
    }
    pub(super) unsafe fn seal(&self, expected: &str, version: u64) -> Result<ParagraphProbe> {
        unsafe {
            self.handoff_stage("originalPoint");
            self.original_point()?;
            self.handoff_stage("clientTrail");
            if !self.trail.borrow().ready(expected) {
                return Err(E_ACCESSDENIED.into());
            }
            self.seal_range(expected, version)
        }
    }
    pub(super) fn source_ready(&self, expected: &str) -> bool {
        self.live.get() && self.trail.borrow().ready(expected)
    }
    // The caller must hold the immutable first receipt and its current native
    // owner fence on both sides. This does not advance Trail or grant writing.
    pub(super) unsafe fn seal_first_receipt(
        &self,
        pending: &(u64, String),
        version: u64,
        receipt: &super::block_tracking::FirstSourceReceipt,
    ) -> Result<Option<ParagraphProbe>> {
        unsafe {
            self.handoff_stage("firstReceipt.nativeReceipt");
            if !self.awaiting_write(pending.0, &pending.1)
                || !receipt.proves_origin(self, pending, version)
            {
                return Err(E_ACCESSDENIED.into());
            }
            self.source_check_passed();
            self.handoff_stage("firstReceipt.spanBefore");
            let Some(before) = self.owned_span(&pending.1)? else {
                return Err(E_ACCESSDENIED.into());
            };
            self.source_check_passed();
            self.handoff_stage("firstReceipt.feedRead");
            let snapshot = self.feed.read()?;
            // Retain the original client predicate as diagnostic evidence. Its
            // composing/target clause is presentation state, not this receipt's
            // ownership. No client observation is cached across edit sessions.
            self.record_first_pair(pending, &snapshot);
            self.source_check_passed();
            self.handoff_stage("firstReceipt.spanAfter");
            let Some(after) = self.owned_span(&pending.1)? else {
                return Err(E_ACCESSDENIED.into());
            };
            self.source_check_passed();
            self.handoff_stage("firstReceipt.spanSame");
            if !before.Compare(&after)?.as_bool() {
                return Err(E_ACCESSDENIED.into());
            }
            self.source_check_passed();
            self.handoff_stage("firstReceipt.clientBody");
            match self
                .trail
                .borrow()
                .first_receipt_content(pending, &snapshot)
            {
                super::composition_feed::FirstContent::Ready => (),
                super::composition_feed::FirstContent::Unpublished => {
                    self.source_diagnostics.waiting();
                    return Ok(None);
                }
                super::composition_feed::FirstContent::Rejected => {
                    return Err(E_ACCESSDENIED.into());
                }
            }
            self.source_check_passed();
            let proof = self.seal_range(&pending.1, version)?;
            self.handoff_stage("firstReceipt.final");
            if !proof.range.Compare(&after)?.as_bool()
                || self.pending_write().as_ref() != Some(pending)
            {
                return Err(E_ACCESSDENIED.into());
            }
            self.source_check_passed();
            Ok(Some(proof))
        }
    }
    // An established source may follow only an intervening verified own write.
    // The Track supplies the exact predecessor/version; user edits cannot
    // create this transition or replace the original point/paragraph.
    pub(super) unsafe fn seal_following(
        &self,
        previous: &ParagraphProbe,
        predecessor: u64,
        expected: &str,
        version: u64,
    ) -> Result<ParagraphProbe> {
        unsafe {
            self.handoff_stage("following.source");
            if !previous.committed_at(predecessor)
                || previous.identity != self.scope.identity
                || previous.raw.cast::<IUnknown>()? != self.scope.raw.cast::<IUnknown>()?
                || previous.paragraph.cast::<IUnknown>()?
                    != self.scope.paragraph.cast::<IUnknown>()?
                || previous.range.CompareEndpoints(
                    TextPatternRangeEndpoint_Start,
                    &self.point,
                    TextPatternRangeEndpoint_Start,
                )? != 0
            {
                return Err(E_ACCESSDENIED.into());
            }
            self.seal_range(expected, version)
        }
    }
    unsafe fn seal_range(&self, expected: &str, version: u64) -> Result<ParagraphProbe> {
        unsafe {
            self.handoff_stage("seal.originalPoint");
            self.original_point()?;
            self.source_check_passed();
            self.handoff_stage("ownedSpan");
            let range = self
                .owned_span(expected)?
                .ok_or_else(|| windows::core::Error::from_hresult(E_ACCESSDENIED))?;
            let proof = ParagraphProbe {
                focus: self.scope.focus.clone(),
                raw: self.scope.raw.clone(),
                pattern: self.scope.pattern.clone(),
                paragraph: self.scope.paragraph.clone(),
                identity: self.scope.identity.clone(),
                range,
                sealed_output: Some(expected.into()),
                source_version: Some(version),
                source_committed: false,
                #[cfg(feature = "acceptance")]
                comparison_sampled: std::cell::Cell::new(0),
                #[cfg(feature = "acceptance")]
                comparison_samples: std::cell::RefCell::new(Vec::new()),
            };
            self.handoff_stage("proofRead");
            if proof.read()? != expected {
                return Err(E_ACCESSDENIED.into());
            }
            self.handoff_stage("originalPointRecheck");
            self.original_point()?;
            self.handoff_stage("sealed");
            Ok(proof)
        }
    }
}
impl ParagraphProbe {
    // Only used after the committed source's original field, protection and
    // paragraph identity checks. A disconnected saved range cannot be reused;
    // reporting its loss never authorizes a replacement range or a write.
    fn retained_comparison(
        result: Result<windows::core::BOOL>,
        phase: &mut String,
    ) -> Result<windows::core::BOOL> {
        result.map_err(|error| {
            let code = error.code();
            if code == windows::Win32::UI::TextServices::TF_E_DISCONNECTED
                || code.0 as u32 == UIA_E_ELEMENTNOTAVAILABLE
            {
                *phase = format!("blockProviderRangeRetired:{phase}:{:08x}", code.0 as u32);
                windows::core::Error::from_hresult(
                    windows::Win32::UI::TextServices::TF_E_DISCONNECTED,
                )
            } else {
                *phase = format!("{phase}:hr={:08x}", code.0 as u32);
                error
            }
        })
    }
    pub(super) fn committed_at(&self, version: u64) -> bool {
        self.source_committed && self.source_version == Some(version)
    }
    // Only the Track owner calls this after its final native/state fence.
    // Publishing this retained provider range establishes read-only provenance;
    // it neither installs a native range nor grants a writer lease.
    pub(super) fn commit_source(mut self, expected: &str, version: u64) -> Result<Self> {
        if self.source_committed
            || self.source_version != Some(version)
            || self.sealed_output.as_deref() != Some(expected)
        {
            return Err(E_ACCESSDENIED.into());
        }
        self.source_committed = true;
        Ok(self)
    }
    #[cfg(feature = "acceptance")]
    pub(super) fn record_handoff(&self, stage: &str, facts: serde_json::Value) {
        let mut samples = self.comparison_samples.borrow_mut();
        if samples.len() < 16 && !samples.iter().any(|v| v["handoffStage"] == stage) {
            samples.push(serde_json::json!({"handoffStage":stage,"facts":facts}));
        }
    }
    unsafe fn capture_scope(phase: &mut &'static str) -> Result<Self> {
        unsafe {
            *phase = "scope.focus";
            let focus = FocusIdentity::capture()?;
            *phase = "scope.raw";
            let raw = focus
                .object
                .as_ref()
                .ok_or_else(|| windows::core::Error::from_hresult(E_NOINTERFACE))?
                .provider
                .clone();
            if !provider_capable(&raw)? {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "scope.pattern";
            let pattern: ITextProvider = raw.GetPatternProvider(UIA_TextPatternId)?.cast()?;
            if pattern.cast::<IClientSecurity>().is_ok() {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "scope.document";
            let range = pattern.DocumentRange()?.Clone()?;
            if bool::try_from(&range.GetAttributeValue(UIA_IsReadOnlyAttributeId)?)? {
                return Err(E_ACCESSDENIED.into());
            }
            let first = range.Clone()?;
            first.MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                &range,
                TextPatternRangeEndpoint_Start,
            )?;
            first.ExpandToEnclosingUnit(TextUnit_Character)?;
            *phase = "scope.parent";
            let paragraph: IRawElementProviderSimple = first
                .GetEnclosingElement()?
                .cast::<IRawElementProviderFragment>()?
                .Navigate(NavigateDirection_Parent)?
                .cast()?;
            *phase = "scope.parentIsField";
            if paragraph.cast::<IUnknown>()? == raw.cast::<IUnknown>()? {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "scope.childRange";
            let child = pattern.RangeFromChild(&paragraph)?;
            if !child.Compare(&range)?.as_bool() {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "scope.runtimeId";
            let identity = runtime_id(&paragraph.cast::<IRawElementProviderFragment>()?)?
                .ok_or_else(|| windows::core::Error::from_hresult(E_NOINTERFACE))?;
            let probe = Self {
                focus,
                raw,
                pattern,
                paragraph,
                identity,
                range: child.Clone()?,
                sealed_output: None,
                source_version: None,
                source_committed: false,
                #[cfg(feature = "acceptance")]
                comparison_sampled: std::cell::Cell::new(0),
                #[cfg(feature = "acceptance")]
                comparison_samples: std::cell::RefCell::new(Vec::new()),
            };
            Ok(probe)
        }
    }
    unsafe fn read(&self) -> Result<String> {
        unsafe { self.read_stage(&mut String::new()) }
    }
    #[cfg(feature = "acceptance")]
    pub(super) fn comparison_diagnostic(&self) -> serde_json::Value {
        serde_json::json!(self.comparison_samples.borrow().clone())
    }
    #[cfg(feature = "acceptance")]
    pub(super) unsafe fn sample_handoff(&self) -> Result<()> {
        let sampled = self.comparison_sampled.get();
        if self.sealed_output.is_none() || sampled & 4 != 0 {
            return Ok(());
        }
        self.comparison_sampled.set(sampled | 4);
        let result = unsafe {
            (|| -> Result<()> {
                let child = self.pattern.RangeFromChild(&self.paragraph)?;
                let document = self.pattern.DocumentRange()?;
                self.record_comparison("afterEndComposition", &child, &document);
                Ok(())
            })()
        };
        if let Err(error) = &result {
            self.comparison_samples
                .borrow_mut()
                .push(serde_json::json!({
                    "phase":"afterEndComposition","hr":error.code().0 as u32,
                }));
        }
        result
    }
    // A refused comparison remains refused. These acceptance-only samples
    // describe the existing two ranges and cannot publish a replacement range.
    #[cfg(feature = "acceptance")]
    unsafe fn sample_comparison(
        &self,
        phase: &str,
        bit: u8,
        child: &ITextRangeProvider,
        document: &ITextRangeProvider,
    ) {
        let sampled = self.comparison_sampled.get();
        if self.sealed_output.is_none() || sampled & bit != 0 {
            return;
        }
        self.comparison_sampled.set(sampled | bit);
        unsafe { self.record_comparison(phase, child, document) };
    }
    #[cfg(feature = "acceptance")]
    unsafe fn record_comparison(
        &self,
        phase: &str,
        child: &ITextRangeProvider,
        document: &ITextRangeProvider,
    ) {
        fn sample<T: serde::Serialize>(value: Result<T>) -> serde_json::Value {
            match value {
                Ok(value) => serde_json::json!({"hr":0,"value":value}),
                Err(error) => serde_json::json!({"hr":error.code().0 as u32}),
            }
        }
        unsafe {
            let starts = sample(self.range.CompareEndpoints(
                TextPatternRangeEndpoint_Start,
                child,
                TextPatternRangeEndpoint_Start,
            ));
            let ends = sample(self.range.CompareEndpoints(
                TextPatternRangeEndpoint_End,
                child,
                TextPatternRangeEndpoint_End,
            ));
            let saved_text = self.range.GetText(16385).map(|v| v.to_string());
            let child_text = child.GetText(16385).map(|v| v.to_string());
            let document_text = document.GetText(16385).map(|v| v.to_string());
            let text = |value: &Result<String>| match value {
                Ok(value) => serde_json::json!({
                    "hr":0,"units":value.encode_utf16().count(),
                    "equalsSealedOutput":self.sealed_output.as_ref() == Some(value),
                }),
                Err(error) => serde_json::json!({"hr":error.code().0 as u32}),
            };
            let field_matches = sample(self.focus.matches());
            let paragraph_matches = sample(
                self.paragraph
                    .cast::<IRawElementProviderFragment>()
                    .and_then(|p| runtime_id(&p).map(|id| id.as_ref() == Some(&self.identity))),
            );
            let final_starts = sample(self.range.CompareEndpoints(
                TextPatternRangeEndpoint_Start,
                child,
                TextPatternRangeEndpoint_Start,
            ));
            let final_ends = sample(self.range.CompareEndpoints(
                TextPatternRangeEndpoint_End,
                child,
                TextPatternRangeEndpoint_End,
            ));
            let final_compare = sample(self.range.Compare(child).map(|value| value.as_bool()));
            let diagnostic = serde_json::json!({
                "phase":phase,
                "starts":starts,"ends":ends,
                "saved":text(&saved_text),"child":text(&child_text),
                "document":text(&document_text),
                "sameTexts":saved_text.as_ref().ok().is_some_and(|v| child_text.as_ref().ok()==Some(v) && document_text.as_ref().ok()==Some(v)),
                "fieldMatches":field_matches,"paragraphMatches":paragraph_matches,
                "finalStarts":final_starts,"finalEnds":final_ends,
                "finalCompare":final_compare,
            });
            eprintln!("paragraph_saved_comparison {diagnostic}");
            self.comparison_samples.borrow_mut().push(diagnostic);
        }
    }
    unsafe fn read_stage(&self, phase: &mut String) -> Result<String> {
        unsafe { self.read_range_stage(&self.range, phase) }
    }
    unsafe fn read_range_stage(
        &self,
        range: &ITextRangeProvider,
        phase: &mut String,
    ) -> Result<String> {
        unsafe {
            *phase = "fieldIdentity".into();
            if !self.focus.matches()? {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "password".into();
            if bool::try_from(&self.raw.GetPropertyValue(UIA_IsPasswordPropertyId)?)? {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "paragraphIdentity".into();
            if runtime_id(&self.paragraph.cast::<IRawElementProviderFragment>()?)?.as_ref()
                != Some(&self.identity)
            {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "childRange".into();
            let child = self.pattern.RangeFromChild(&self.paragraph)?;
            *phase = "documentRange".into();
            let document = self.pattern.DocumentRange()?;
            *phase = "savedEqualsChild".into();
            if !range.Compare(&child)?.as_bool() {
                #[cfg(feature = "acceptance")]
                if range.as_raw() == self.range.as_raw() {
                    self.sample_comparison("refused", 2, &child, &document);
                }
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "savedEqualsDocument".into();
            if !range.Compare(&document)?.as_bool() {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "readonly".into();
            if bool::try_from(&range.GetAttributeValue(UIA_IsReadOnlyAttributeId)?)? {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "text".into();
            let text = range.GetText(16385)?.to_string();
            *phase = "completeText".into();
            if text.is_empty()
                || text.encode_utf16().count() > 16384
                || text.chars().any(char::is_control)
            {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "childText".into();
            if child.GetText(16385)? != text {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "documentText".into();
            if document.GetText(16385)? != text {
                return Err(E_ACCESSDENIED.into());
            }
            #[cfg(feature = "acceptance")]
            if range.as_raw() == self.range.as_raw() {
                self.sample_comparison("sealed", 1, &child, &document);
            }
            Ok(text)
        }
    }
    unsafe fn endpoint_within_field(&self, range: &ITextRangeProvider) -> Result<bool> {
        unsafe {
            let end = range.Clone()?;
            end.MoveEndpointByRange(
                TextPatternRangeEndpoint_Start,
                range,
                TextPatternRangeEndpoint_End,
            )?;
            let mut element = end
                .GetEnclosingElement()?
                .cast::<IRawElementProviderFragment>()?;
            let field = self.raw.cast::<IUnknown>()?;
            for _ in 0..16 {
                if element.cast::<IUnknown>()? == field {
                    return Ok(true);
                }
                element = element.Navigate(NavigateDirection_Parent)?;
            }
            Ok(false)
        }
    }
    // A cache projection follows only the already committed provider range.
    // Keyboard edits may change its body, never its source scope. One inward
    // empty-structure repair is allowed; no child/document text fills a gap.
    unsafe fn projection_range(
        &self,
        version: u64,
        phase: &mut String,
    ) -> Result<(ITextRangeProvider, bool)> {
        #[cfg(feature = "acceptance")]
        let sampled = self.comparison_sampled.get();
        #[cfg(feature = "acceptance")]
        let report = sampled & 8 == 0;
        #[cfg(feature = "acceptance")]
        if report {
            self.comparison_sampled.set(sampled | 8);
        }
        #[cfg(feature = "acceptance")]
        let mut detail = serde_json::json!({"phase":"cacheRange","version":version,"sourceVersion":self.source_version});
        let result: Result<(ITextRangeProvider, bool)> = (|| unsafe {
            *phase = "paragraph.cacheRange.sourceCommit".into();
            if !self.source_committed
                || self.source_version != Some(version)
                || self.sealed_output.is_none()
            {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "paragraph.cacheRange.field".into();
            if !self.focus.matches()?
                || bool::try_from(&self.raw.GetPropertyValue(UIA_IsPasswordPropertyId)?)?
            {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "paragraph.cacheRange.paragraph".into();
            if runtime_id(&self.paragraph.cast::<IRawElementProviderFragment>()?)?.as_ref()
                != Some(&self.identity)
            {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "paragraph.cacheRange.child".into();
            let child = self.pattern.RangeFromChild(&self.paragraph)?;
            *phase = "paragraph.cacheRange.document".into();
            let document = self.pattern.DocumentRange()?;
            *phase = "paragraph.cacheRange.childCompare".into();
            let same_child =
                Self::retained_comparison(self.range.Compare(&child), phase)?.as_bool();
            let same_document = if same_child {
                *phase = "paragraph.cacheRange.documentCompare".into();
                self.range.Compare(&document)?.as_bool()
            } else {
                false
            };
            if same_child && same_document {
                #[cfg(feature = "acceptance")]
                {
                    detail["unchanged"] = true.into();
                }
                *phase = "paragraph.cacheRange.clone".into();
                return Ok((self.range.Clone()?, false));
            }
            *phase = "paragraph.cacheRange.source".into();
            if bool::try_from(&self.range.GetAttributeValue(UIA_IsReadOnlyAttributeId)?)? {
                return Err(E_ACCESSDENIED.into());
            }
            // Source was sealed while the owned output still equalled the
            // receipt. Continue that SAME saved range through user edits.
            let expected = self.range.GetText(16385)?.to_string();
            if expected.is_empty()
                || expected.encode_utf16().count() > 16384
                || expected.chars().any(char::is_control)
                || child.GetText(16385)? != expected
                || document.GetText(16385)? != expected
            {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "paragraph.cacheRange.endpoints".into();
            let starts = self.range.CompareEndpoints(
                TextPatternRangeEndpoint_Start,
                &child,
                TextPatternRangeEndpoint_Start,
            )?;
            let ends = self.range.CompareEndpoints(
                TextPatternRangeEndpoint_End,
                &child,
                TextPatternRangeEndpoint_End,
            )?;
            #[cfg(feature = "acceptance")]
            {
                detail["starts"] = starts.into();
            }
            #[cfg(feature = "acceptance")]
            {
                detail["ends"] = ends.into();
            }
            if starts != 0
                || ends <= 0
                || !self.endpoint_within_field(&self.range)?
                || !self.endpoint_within_field(&child)?
            {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "paragraph.cacheRange.gap".into();
            let gap = self.range.Clone()?;
            gap.MoveEndpointByRange(
                TextPatternRangeEndpoint_Start,
                &child,
                TextPatternRangeEndpoint_End,
            )?;
            let empty_gap = gap.GetText(16385)?.is_empty();
            #[cfg(feature = "acceptance")]
            {
                detail["emptyGap"] = empty_gap.into();
            }
            if !empty_gap {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "paragraph.cacheRange.move".into();
            let candidate = self.range.Clone()?;
            candidate.MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                &child,
                TextPatternRangeEndpoint_End,
            )?;
            #[cfg(feature = "acceptance")]
            {
                detail["copiedOriginalEnd"] = true.into();
            }
            *phase = "paragraph.cacheRange.verify".into();
            if candidate.CompareEndpoints(
                TextPatternRangeEndpoint_Start,
                &self.range,
                TextPatternRangeEndpoint_Start,
            )? != 0
                || !candidate.Compare(&child)?.as_bool()
                || !candidate.Compare(&document)?.as_bool()
                || candidate.GetText(16385)? != expected
            {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "paragraph.cacheRange.inward".into();
            if candidate.CompareEndpoints(
                TextPatternRangeEndpoint_End,
                &self.range,
                TextPatternRangeEndpoint_End,
            )? > 0
            {
                return Err(E_ACCESSDENIED.into());
            }
            let excluded = self.range.Clone()?;
            excluded.MoveEndpointByRange(
                TextPatternRangeEndpoint_Start,
                &candidate,
                TextPatternRangeEndpoint_End,
            )?;
            if !excluded.GetText(16385)?.is_empty()
                || candidate.CompareEndpoints(
                    TextPatternRangeEndpoint_Start,
                    &self.range,
                    TextPatternRangeEndpoint_Start,
                )? != 0
                || candidate.CompareEndpoints(
                    TextPatternRangeEndpoint_End,
                    &self.range,
                    TextPatternRangeEndpoint_End,
                )? > 0
            {
                return Err(E_ACCESSDENIED.into());
            }
            #[cfg(feature = "acceptance")]
            {
                detail["inwardRechecked"] = true.into();
            }
            #[cfg(feature = "acceptance")]
            {
                detail["strictCandidate"] = true.into();
            }
            Ok((candidate, true))
        })();
        #[cfg(feature = "acceptance")]
        if report {
            #[cfg(feature = "acceptance")]
            {
                detail["stage"] = phase.clone().into();
            }
            #[cfg(feature = "acceptance")]
            {
                detail["error"] = result.as_ref().err().map(|e| e.code().0 as u32).into();
            }
            self.comparison_samples.borrow_mut().push(detail);
        }
        result
    }
    pub(super) unsafe fn projection(
        &self,
        native: &str,
        version: u64,
        failure_phase: &mut String,
    ) -> Result<Option<ParagraphProjection>> {
        let mut phase = String::new();
        let result = unsafe { self.projection_stage(native, version, &mut phase) };
        if result.is_err() {
            *failure_phase = phase;
        }
        result
    }
    // Only a committed source at the exact native version may explain the
    // interval in which the client's selection has refreshed before its text
    // cache. This is readonly evidence, not a replacement native interval.
    pub(super) unsafe fn pending_coordinates(
        &self,
        expected: &str,
        version: u64,
        phase: &mut String,
    ) -> Result<()> {
        unsafe {
            if !self.committed_at(version) || self.sealed_output.as_deref() != Some(expected) {
                return Err(E_ACCESSDENIED.into());
            }
            let (range, _) = self.projection_range(version, phase)?;
            let observed = self.read_projection(&range, expected, phase)?;
            if !observed.is_some_and(|value| value.text == expected) {
                return Err(E_ACCESSDENIED.into());
            }
            Ok(())
        }
    }
    unsafe fn projection_stage(
        &self,
        native: &str,
        version: u64,
        phase: &mut String,
    ) -> Result<Option<ParagraphProjection>> {
        unsafe {
            let (range, _) = self.projection_range(version, phase)?;
            if self.source_version != Some(version) {
                *phase = "paragraph.sourceVersion".into();
                return Err(E_ACCESSDENIED.into());
            }
            self.read_projection(&range, native, phase)
        }
    }
    // Only the owner of a committed continuous native range may call this.
    // It checks current content/selection against the retained provider range;
    // it cannot establish provenance or move either endpoint.
    pub(super) unsafe fn synchronize(
        &self,
        native: &str,
        phase: &mut String,
    ) -> Result<Option<ParagraphProjection>> {
        unsafe {
            // Provenance has already been committed to a continuous native
            // range. A provider selection is a current view of the SAME
            // retained paragraph, not a second source-range admission.
            // read_projection still checks original field/paragraph identity,
            // complete document, protection, stable partition and selection.
            *phase = "paragraph.currentSelectionRange".into();
            let current = self.pattern.RangeFromChild(&self.paragraph)?;
            self.read_projection(&current, native, phase)
        }
    }
    unsafe fn read_projection(
        &self,
        range: &ITextRangeProvider,
        native: &str,
        phase: &mut String,
    ) -> Result<Option<ParagraphProjection>> {
        unsafe {
            let mut read_phase = String::new();
            let text = self
                .read_range_stage(range, &mut read_phase)
                .inspect_err(|_| *phase = format!("paragraph.firstRead.{read_phase}"))?;
            // The remainder is retained and guarded as native text. It is not
            // interpreted as user text or removed from the document.
            if !native
                .strip_prefix(&text)
                .is_some_and(|tail| tail.chars().all(|c| matches!(c, '\r' | '\n')))
            {
                #[cfg(feature = "acceptance")]
                eprintln!(
                    "paragraph_snapshot_wait rawUnits={:?} nativeUnits={:?}",
                    text.encode_utf16().collect::<Vec<_>>(),
                    native.encode_utf16().collect::<Vec<_>>()
                );
                return Ok(None);
            }
            *phase = "paragraph.selection".into();
            let selection = selected(&self.pattern)?;
            *phase = "paragraph.selectionBefore".into();
            let before = range.Clone()?;
            before.MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                &selection,
                TextPatternRangeEndpoint_Start,
            )?;
            *phase = "paragraph.selectionAfter".into();
            let after = range.Clone()?;
            after.MoveEndpointByRange(
                TextPatternRangeEndpoint_Start,
                &selection,
                TextPatternRangeEndpoint_End,
            )?;
            *phase = "paragraph.selectionText".into();
            let before = before.GetText(16385)?.to_string();
            let picked = selection.GetText(16385)?.to_string();
            let after = after.GetText(16385)?.to_string();
            *phase = "paragraph.partition".into();
            if text != format!("{before}{picked}{after}") {
                return Err(E_ACCESSDENIED.into());
            }
            *phase = "paragraph.selectionRecheck".into();
            if !selection.Compare(&selected(&self.pattern)?)?.as_bool() {
                return Err(E_ACCESSDENIED.into());
            }
            let final_text = self
                .read_range_stage(range, &mut read_phase)
                .inspect_err(|_| *phase = format!("paragraph.finalRead.{read_phase}"))?;
            if final_text != text {
                *phase = "paragraph.finalRead.changedText".into();
                return Err(E_ACCESSDENIED.into());
            }
            Ok(Some(ParagraphProjection {
                text,
                selected: [
                    before.encode_utf16().count() as i32,
                    (before.encode_utf16().count() + picked.encode_utf16().count()) as i32,
                ],
                native: native.into(),
                range: range.clone(),
            }))
        }
    }
}
impl Evidence {
    pub(super) fn semantic_block(
        &self,
        start: i32,
        end: i32,
        original: Option<&SemanticBoundary>,
    ) -> std::result::Result<SemanticBlock, BlockReadError> {
        let mapping = block_representation::map(&self.all, &self.tsf, start, end)
            .ok_or(BlockReadError::Unproven)?;
        let boundary = SemanticBoundary {
            identity: self.identity.clone(),
            before: SemanticFingerprint::of(&self.all[..mapping.start]),
            after: SemanticFingerprint::of(&self.all[mapping.end..]),
        };
        if original.is_some_and(|old| {
            old.identity != boundary.identity
                || old.before != boundary.before
                || old.after != boundary.after
        }) {
            return Err(BlockReadError::OutsideScope);
        }
        let selected_start = self.before.len();
        let selected_end = selected_start + self.picked.len();
        if selected_start < mapping.start || selected_end > mapping.end {
            return Err(BlockReadError::OutsideScope);
        }
        Ok(SemanticBlock {
            text: self.all[mapping.start..mapping.end].into(),
            native_end: mapping.native_end,
            selection_start: self.all[mapping.start..selected_start]
                .encode_utf16()
                .count() as u32,
            selection_end: self.all[mapping.start..selected_end].encode_utf16().count() as u32,
            boundary,
        })
    }
    // Diagnostic only. Coordinates never create or move a native write range.
    pub(super) fn block_projection(&self, start: i32, end: i32) -> BlockProjection {
        use sha2::{Digest, Sha256};
        let mapped = diagnostic_block_mapping(&self.all, &self.tsf, start, end);
        let (before, after) = mapped.map_or((None, None), |(a, b)| {
            (Some(&self.all[..a]), Some(&self.all[b..]))
        });
        BlockProjection {
            units: self.all.encode_utf16().count(),
            document_terminal_units: self
                .all
                .chars()
                .rev()
                .take_while(|c| matches!(c, '\r' | '\n'))
                .count(),
            mapped: mapped.is_some(),
            before_units: before.map(|v| v.encode_utf16().count()),
            after_units: after.map(|v| v.encode_utf16().count()),
            provider_equal: None,
            before_equal: None,
            after_equal: None,
            identity: self.identity.clone(),
            before_hash: before.map(|v| Sha256::digest(v.as_bytes()).into()),
            after_hash: after.map(|v| Sha256::digest(v.as_bytes()).into()),
        }
    }
    pub fn selected_text(&self) -> &str {
        &self.picked
    }
    pub fn revalidate(&self, tsf_text: &str) -> Result<()> {
        unsafe {
            if GetFocus() != self.window || tsf_text != self.tsf {
                return Err(E_ACCESSDENIED.into());
            }
            let current = raw_provider(&focused_accessible(self.accessible_window)?)?;
            if current.cast::<IUnknown>()? != self.identity
                || !provider_capable(&current)?
                || !bool::try_from(&self.raw.GetPropertyValue(UIA_HasKeyboardFocusPropertyId)?)?
                || bool::try_from(&self.raw.GetPropertyValue(UIA_IsPasswordPropertyId)?)?
            {
                return Err(E_ACCESSDENIED.into());
            }
            let range = selected(&self.pattern)?;
            if bool::try_from(&range.GetAttributeValue(UIA_IsReadOnlyAttributeId)?)? {
                return Err(E_ACCESSDENIED.into());
            }
            let document = self.pattern.DocumentRange()?;
            let prefix = document.Clone()?;
            prefix.MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                &range,
                TextPatternRangeEndpoint_Start,
            )?;
            if document.GetText(16385)? != self.all
                || prefix.GetText(16385)? != self.before
                || range.GetText(16385)? != self.picked
                || !range.Compare(&selected(&self.pattern)?)?.as_bool()
                || raw_provider(&focused_accessible(self.accessible_window)?)?.cast::<IUnknown>()?
                    != self.identity
                || GetFocus() != self.window
            {
                return Err(E_ACCESSDENIED.into());
            }
            Ok(())
        }
    }
}
#[derive(serde::Serialize)]
pub(super) struct BlockProjection {
    units: usize,
    document_terminal_units: usize,
    mapped: bool,
    before_units: Option<usize>,
    after_units: Option<usize>,
    provider_equal: Option<bool>,
    before_equal: Option<bool>,
    after_equal: Option<bool>,
    #[serde(skip)]
    identity: IUnknown,
    #[serde(skip)]
    before_hash: Option<[u8; 32]>,
    #[serde(skip)]
    after_hash: Option<[u8; 32]>,
}
impl BlockProjection {
    pub(super) fn mapped(&self) -> bool {
        self.mapped
    }
    pub(super) fn compare(&mut self, original: &Self) {
        self.provider_equal = Some(self.identity == original.identity);
        self.before_equal = self
            .before_hash
            .zip(original.before_hash)
            .map(|(a, b)| a == b);
        self.after_equal = self
            .after_hash
            .zip(original.after_hash)
            .map(|(a, b)| a == b);
    }
}
// Explain the complete document using existing representations, then report
// boundaries only when every explaining representation agrees. Ambiguity and
// splitting a doubled newline are deliberately unavailable, even for telemetry.
fn diagnostic_block_mapping(raw: &str, tsf: &str, start: i32, end: i32) -> Option<(usize, usize)> {
    if start < 0 || end < start {
        return None;
    }
    let mut result = None;
    for doubled in [false, true] {
        let represented = if doubled {
            raw.replace('\n', "\n\n")
        } else {
            raw.into()
        };
        if !tsf
            .strip_prefix(&represented)
            .is_some_and(|v| v.chars().all(|c| matches!(c, '\r' | '\n')))
        {
            continue;
        }
        let mut units = 0i32;
        let mut a = (start == 0).then_some(0);
        let mut b = (end == 0).then_some(0);
        for (at, c) in raw.char_indices() {
            units += if doubled && c == '\n' {
                2
            } else {
                c.len_utf16() as i32
            };
            let byte = at + c.len_utf8();
            if units == start {
                a = Some(byte);
            }
            if units == end {
                b = Some(byte);
            }
        }
        let pair = (a?, b?);
        if result.is_some_and(|previous| previous != pair) {
            return None;
        }
        result = Some(pair);
    }
    result
}
fn mapped_document(raw: &str, before: &str, picked: &str, tsf: &str) -> Option<(i32, i32)> {
    let mut mapped = None;
    // A representation is accepted only when it explains the complete bounded
    // document. Never locate a selection by substring or normalize arbitrary
    // whitespace. Multiple representations must agree on both endpoints.
    for doubled in [false, true] {
        let represent = |value: &str| {
            if doubled {
                value.replace('\n', "\n\n")
            } else {
                value.into()
            }
        };
        let document = represent(raw);
        if !tsf
            .strip_prefix(&document)
            .is_some_and(|suffix| suffix.chars().all(|c| matches!(c, '\r' | '\n')))
        {
            continue;
        }
        let start = represent(before).encode_utf16().count() as i32;
        let end = start + represent(picked).encode_utf16().count() as i32;
        if mapped.is_some_and(|previous| previous != (start, end)) {
            return None;
        }
        mapped = Some((start, end));
    }
    mapped
}
unsafe fn provider_capable(raw: &IRawElementProviderSimple) -> Result<bool> {
    unsafe {
        let options = raw.ProviderOptions()?;
        let required = ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading;
        Ok((options & required) == required
            && (options & ProviderOptions_ClientSideProvider).0 == 0)
    }
}
pub fn inspect(tsf_text: &str) -> (String, Option<Evidence>) {
    let mut stage = "window";
    let started = std::time::Instant::now();
    let result = (|| unsafe {
        let window = GetFocus();
        let mut process = 0;
        let thread = GetWindowThreadProcessId(window, Some(&mut process));
        if window.0.is_null() || process != GetCurrentProcessId() || thread != GetCurrentThreadId()
        {
            return Err(E_ACCESSDENIED.into());
        }
        stage = "accessible";
        let mut accessible_window = window;
        let mut accessible = focused_accessible(accessible_window)?;
        stage = "raw";
        let raw: IRawElementProviderSimple = match raw_provider(&accessible) {
            Ok(raw) => raw,
            Err(_) => {
                accessible_window = GetAncestor(window, GA_ROOT);
                if accessible_window == window
                    || GetWindowThreadProcessId(accessible_window, None) != thread
                {
                    return Err(E_NOINTERFACE.into());
                }
                stage = "rootFocus";
                accessible = focused_accessible(accessible_window)?;
                stage = "rootRaw";
                raw_provider(&accessible)?
            }
        };
        stage = "properties";
        if !provider_capable(&raw)?
            || !bool::try_from(&raw.GetPropertyValue(UIA_HasKeyboardFocusPropertyId)?)?
            || bool::try_from(&raw.GetPropertyValue(UIA_IsPasswordPropertyId)?)?
        {
            return Err(E_ACCESSDENIED.into());
        }
        stage = "textPattern";
        let pattern: ITextProvider = raw.GetPatternProvider(UIA_TextPatternId)?.cast()?;
        stage = "selection";
        let range = selected(&pattern)?;
        if bool::try_from(&range.GetAttributeValue(UIA_IsReadOnlyAttributeId)?)? {
            return Err(E_ACCESSDENIED.into());
        }
        let document = pattern.DocumentRange()?;
        let prefix = document.Clone()?;
        prefix.MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            &range,
            TextPatternRangeEndpoint_Start,
        )?;
        let suffix = document.Clone()?;
        suffix.MoveEndpointByRange(
            TextPatternRangeEndpoint_Start,
            &range,
            TextPatternRangeEndpoint_End,
        )?;
        let all = document.GetText(16385)?.to_string();
        let before = prefix.GetText(16385)?.to_string();
        let picked = range.GetText(16385)?.to_string();
        let after = suffix.GetText(16385)?.to_string();
        let count = |text: &str| text.encode_utf16().count();
        if count(&all) > 16384 || all != format!("{before}{picked}{after}") {
            return Err(E_INVALIDARG.into());
        }
        let stable = GetFocus() == window
            && range.Compare(&selected(&pattern)?)?.as_bool()
            && raw.cast::<IUnknown>()?
                == raw_provider(&focused_accessible(accessible_window)?)?.cast::<IUnknown>()?;
        let proxy_interface = pattern.cast::<IClientSecurity>().is_ok();
        #[cfg(feature = "acceptance")]
        let module_name = provider_module(&pattern);
        #[cfg(not(feature = "acceptance"))]
        let module_name = "unqueried";
        let mapped = mapped_document(&all, &before, &picked, tsf_text);
        let eligible = stable && !proxy_interface && mapped.is_some();
        let (start, end) = mapped.unwrap_or_default();
        Ok::<_, windows::core::Error>((
            format!(
                "raw_start={} raw_end={} raw_len={} tsf_len={} raw_tsf_equal={} raw_tsf_mapped={} raw_stable={} raw_proxy_interface={} raw_module={}",
                count(&before),
                count(&before) + count(&picked),
                count(&all),
                count(tsf_text),
                all == tsf_text,
                mapped.is_some(),
                stable,
                proxy_interface,
                module_name
            ),
            eligible.then_some(Evidence {
                window,
                accessible_window,
                identity: raw.cast::<IUnknown>()?,
                raw,
                pattern,
                all,
                before,
                picked,
                tsf: tsf_text.into(),
                start,
                end,
            }),
        ))
    })();
    match result {
        Ok((value, position)) => (
            format!("{value} raw_us={}", started.elapsed().as_micros()),
            position,
        ),
        Err(error) => (
            format!(
                "raw_stage={stage} raw_error={:08x} raw_us={}",
                error.code().0 as u32,
                started.elapsed().as_micros()
            ),
            None,
        ),
    }
}

#[cfg(feature = "acceptance")]
unsafe fn provider_module(pattern: &ITextProvider) -> String {
    use windows::Win32::System::LibraryLoader::*;
    unsafe {
        let mut module = HMODULE::default();
        if GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            windows::core::PCWSTR(
                Interface::vtable(pattern).GetSelection as *const () as *const u16,
            ),
            &mut module,
        )
        .is_err()
        {
            return "unknown".into();
        }
        let mut module_path = [0u16; 1024];
        let length = GetModuleFileNameW(Some(module), &mut module_path);
        String::from_utf16_lossy(&module_path[..length as usize])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("unknown")
            .into()
    }
}

#[cfg(test)]
mod tests {
    use super::mapped_document;
    #[test]
    fn source_diagnostic_early_handoff_cannot_inherit_poll_success() {
        use super::{E_ACCESSDENIED, Rc, SOURCE_CHECKS, SourceDiagnostics};
        let diagnostics = Rc::new(SourceDiagnostics::default());
        {
            let poll = diagnostics.begin("same-task", 7, 2, "poll");
            poll.stage("native.selection");
            diagnostics.passed();
            poll.finish("awaitPublication", None);
            poll.settlement("awaitHandoff");
        }
        {
            let handoff = diagnostics.begin("same-task", 7, 2, "keyboard");
            // The current handoff returns before any native selection query.
            handoff.stage("handoff.cleanup");
            handoff.finish("error", Some(E_ACCESSDENIED));
            handoff.settlement("retired");
        }
        let calls = diagnostics.calls.borrow();
        let poll = calls[0].borrow();
        let handoff = calls[1].borrow();
        let selection = 1
            << SOURCE_CHECKS
                .iter()
                .position(|v| *v == "native.selection")
                .unwrap();
        assert_ne!(poll.passed & selection, 0);
        assert_eq!(handoff.entered & selection, 0);
        assert_eq!(handoff.passed & selection, 0);
        assert_eq!(handoff.failed & selection, 0);
        assert_eq!(handoff.first_failure, Some("handoff.cleanup"));
        assert_eq!(handoff.hr, Some(E_ACCESSDENIED.0 as u32));
        assert_eq!(handoff.task, poll.task);
        assert_eq!(handoff.operation, 7);
        assert_eq!(handoff.revision, 2);
        assert_eq!(handoff.call, poll.call + 1);
        assert_eq!(handoff.entry, "keyboard");
        assert_eq!(handoff.settlement, "retired");
    }
    #[test]
    fn source_diagnostic_wait_candidate_and_publication_are_separate() {
        use super::{E_FAIL, Rc, SOURCE_CHECKS, SourceDiagnostics};
        let diagnostics = Rc::new(SourceDiagnostics::default());
        let outer = diagnostics.begin("task", 4, 1, "poll");
        outer.stage("follow.candidateText");
        diagnostics.waiting();
        {
            let nested = diagnostics.begin("other-task", 5, 1, "host");
            nested.stage("handoff.context");
            nested.finish("error", Some(E_FAIL));
        }
        outer.stage("native.selection");
        outer.finish("awaitPublication", None);
        outer.settlement("awaitHandoff");
        let candidate = diagnostics.begin("task", 4, 1, "keyboard");
        candidate.stage("firstReceipt.final");
        candidate.finish("candidate", None);
        assert_eq!(candidate.data.borrow().settlement, "notSubmitted");
        candidate.settlement("retired");
        let data = outer.data.borrow();
        let waiting = 1
            << SOURCE_CHECKS
                .iter()
                .position(|v| *v == "follow.candidateText")
                .unwrap();
        assert_ne!(data.waiting & waiting, 0);
        assert_eq!(data.failed, 0);
        assert_eq!(data.outcome, "awaitPublication");
        assert_eq!(data.settlement, "awaitHandoff");
        assert_eq!(candidate.data.borrow().outcome, "candidate");
        assert_eq!(candidate.data.borrow().settlement, "retired");
        assert!(diagnostics.snapshot().to_string().len() < 1800);
    }
    #[test]
    fn source_diagnostic_stage_changes_never_imply_check_success() {
        use super::{E_FAIL, Rc, SOURCE_CHECKS, SourceDiagnostics};
        let diagnostics = Rc::new(SourceDiagnostics::default());
        let call = diagnostics.begin("task", 4, 1, "keyboard");
        call.stage("native.selection");
        diagnostics.rejected();
        call.stage("native.selection.changed");
        call.finish("error", Some(E_FAIL));
        let data = call.data.borrow();
        let selection = 1
            << SOURCE_CHECKS
                .iter()
                .position(|v| *v == "native.selection")
                .unwrap();
        assert_eq!(data.passed & selection, 0);
        assert_ne!(data.failed & selection, 0);
        assert_eq!(data.first_failure, Some("native.selection"));
        drop(data);
        let authority = diagnostics.begin("task", 4, 1, "host");
        authority.stage("follow.candidateText");
        authority.stage("handoff.authority.initial");
        authority.finish("error", Some(E_FAIL));
        assert_eq!(authority.data.borrow().passed, 0);
        assert_eq!(
            authority.data.borrow().first_failure,
            Some("handoff.authority.initial")
        );
        assert_eq!(
            authority.data.borrow().parent,
            Some(call.data.borrow().call)
        );
        let after_success = diagnostics.begin("task", 4, 1, "host");
        after_success.stage("firstReceipt.clientBody");
        diagnostics.passed();
        after_success.finish("error", Some(E_FAIL));
        assert_eq!(
            after_success.data.borrow().first_failure,
            Some("unscopedAfterCheck")
        );
    }
    #[test]
    fn source_diagnostic_distinguishes_unpublished_body_from_retained_tail() {
        use super::{Rc, SourceDiagnostics};
        let diagnostics = Rc::new(SourceDiagnostics::default());
        let call = diagnostics.begin("task", 4, 1, "keyboard");
        diagnostics.text("\n", "T", "\n");
        assert_eq!(
            call.data.borrow().text.as_ref().unwrap()["equalsBaseline"],
            true
        );
        assert_eq!(
            call.data.borrow().text.as_ref().unwrap()["expectedThenBaseline"],
            false
        );
        diagnostics.text("T\n", "T", "\n");
        assert_eq!(
            call.data.borrow().text.as_ref().unwrap()["equalsBaseline"],
            false
        );
        assert_eq!(
            call.data.borrow().text.as_ref().unwrap()["expectedThenBaseline"],
            true
        );
        assert_eq!(call.data.borrow().text.as_ref().unwrap()["actualUnits"], 2);
        assert!(!diagnostics.snapshot().to_string().contains("T\\n"));
    }
    #[test]
    fn retained_compare_only_retires_explicit_provider_range_loss() {
        use super::{
            E_ACCESSDENIED, E_FAIL, E_INVALIDARG, E_NOINTERFACE, ParagraphProbe,
            UIA_E_ELEMENTNOTAVAILABLE,
        };
        use windows::Win32::UI::TextServices::TF_E_DISCONNECTED;
        for code in [
            TF_E_DISCONNECTED,
            windows::core::HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32),
        ] {
            let mut phase = "paragraph.cacheRange.childCompare".into();
            let error = ParagraphProbe::retained_comparison(
                Err(windows::core::Error::from_hresult(code)),
                &mut phase,
            )
            .unwrap_err();
            assert_eq!(error.code(), TF_E_DISCONNECTED);
            assert_eq!(
                phase,
                format!(
                    "blockProviderRangeRetired:paragraph.cacheRange.childCompare:{:08x}",
                    code.0 as u32
                )
            );
        }
        for code in [E_ACCESSDENIED, E_NOINTERFACE, E_INVALIDARG, E_FAIL] {
            let mut phase = "paragraph.cacheRange.childCompare".into();
            let error = ParagraphProbe::retained_comparison(
                Err(windows::core::Error::from_hresult(code)),
                &mut phase,
            )
            .unwrap_err();
            assert_eq!(error.code(), code);
            assert!(!phase.starts_with("blockProviderRangeRetired:"));
        }
        let mut phase = "paragraph.cacheRange.childCompare".into();
        assert!(
            !ParagraphProbe::retained_comparison(Ok(false.into()), &mut phase)
                .unwrap()
                .as_bool()
        );
        assert_eq!(phase, "paragraph.cacheRange.childCompare");
    }
    #[test]
    fn diagnostic_projection_distinguishes_tail_representation_from_real_outside_text() {
        use super::diagnostic_block_mapping as map;
        assert_eq!(map("周五\n", "周五\n\n", 0, 2), Some((0, 6)));
        assert_eq!(map("周五\n\n", "周五\n\n", 0, 2), Some((0, 6)));
        // Same native anchors, different actual suffixes; diagnostic comparison
        // must compare the original raw provider suffix, not erase newlines.
        let (a, b) = map("周五\n", "周五\n\n", 0, 2).unwrap();
        assert_eq!(&"周五\n"[a..b], "周五");
        assert_ne!(&"周五\n"[b..], &"周五\n\n"[b..]);
        assert_eq!(map("左🙂右\n", "左🙂右\n\n", 1, 3), Some((3, 7)));
        assert_eq!(map("周五", "周四\n", 0, 2), None);
    }
    #[test]
    fn diagnostic_projection_refuses_ambiguous_and_split_native_anchors() {
        use super::diagnostic_block_mapping as map;
        assert_eq!(map("a\n", "a\n\n", 0, 2), None);
        assert_eq!(map("🙂", "🙂", 0, 1), None);
        assert_eq!(map("a", "a", -1, 1), None);
        assert_eq!(map("a", "a", 1, 0), None);
        assert_eq!(map("a", "a", 0, 3), None);
    }
    #[test]
    fn identity_array_bounds_are_bounded_before_allocation() {
        assert_eq!(super::identity_array_len(8, 8), Some(1));
        assert_eq!(super::identity_array_len(-4, 59), Some(64));
        assert_eq!(super::identity_array_len(-4, 60), None);
        assert_eq!(super::identity_array_len(8, 7), None);
        assert_eq!(super::identity_array_len(i32::MIN, i32::MAX), None);
        assert_eq!(super::identity_array_len(i32::MAX, i32::MIN), None);
        assert_eq!(super::identity_array_len(i32::MAX, i32::MAX), Some(1));
    }
    #[test]
    fn focused_leaf_rejects_root_containers_and_security_states() {
        use super::*;
        assert!(!editable_focus(ROLE_SYSTEM_TEXT as i32, STATE_SYSTEM_FOCUSED).unwrap());
        // This provisional classification requires the writable text attribute;
        // GROUPING alone is never the final field identity.
        assert!(editable_focus(ROLE_SYSTEM_GROUPING as i32, STATE_SYSTEM_FOCUSED).unwrap());
        for role in [ROLE_SYSTEM_PANE, ROLE_SYSTEM_DOCUMENT, ROLE_SYSTEM_CLIENT] {
            assert_eq!(
                editable_focus(role as i32, STATE_SYSTEM_FOCUSED)
                    .unwrap_err()
                    .code(),
                E_NOINTERFACE
            );
        }
        for state in [STATE_SYSTEM_PROTECTED, STATE_SYSTEM_READONLY] {
            // This code is also the no-fallback marker, even for a container.
            assert_eq!(
                editable_focus(ROLE_SYSTEM_TEXT as i32, STATE_SYSTEM_FOCUSED | state)
                    .unwrap_err()
                    .code(),
                E_ACCESSDENIED
            );
            assert_eq!(
                editable_focus(ROLE_SYSTEM_PANE as i32, STATE_SYSTEM_FOCUSED | state)
                    .unwrap_err()
                    .code(),
                E_ACCESSDENIED
            );
        }
        assert_eq!(
            editable_focus(ROLE_SYSTEM_TEXT as i32, 0)
                .unwrap_err()
                .code(),
            E_NOINTERFACE
        );
        assert!(!identity_bool(&VARIANT::from(false)).unwrap());
        assert!(identity_bool(&VARIANT::from(true)).unwrap());
        assert!(identity_bool(&VARIANT::from(0i32)).is_err());
        assert!(identity_bool(&VARIANT::default()).is_err());
    }
    #[test]
    fn full_document_representations_require_unique_selection_coordinates() {
        assert_eq!(mapped_document("甲\n", "甲", "\n", "甲\n\n"), None);
        assert_eq!(
            mapped_document("前🙂被选右", "前", "🙂被选", "前🙂被选右\n"),
            Some((1, 5))
        );
        assert_eq!(
            mapped_document("甲\n乙", "甲\n", "乙", "甲\n乙\r\n"),
            Some((2, 3))
        );
        assert_eq!(
            mapped_document("甲\n乙", "甲\n", "乙", "甲\n\n乙\n\n"),
            Some((3, 4))
        );
        assert_eq!(
            mapped_document("甲\n乙", "甲", "\n乙", "甲\n\n乙\n\n"),
            Some((1, 4))
        );
        assert_eq!(mapped_document("甲\n乙", "甲\n", "乙", "甲\r\n乙"), None);
        assert_eq!(
            mapped_document("甲\n乙\n丙", "甲\n", "乙", "甲\n\n乙\n丙"),
            None
        );
        assert_eq!(
            mapped_document("前🙂被选右", "前", "🙂被选", "前🙂被选右别的正文"),
            None
        );
        assert_eq!(mapped_document("前被选右", "前", "被选", "前被选"), None);
    }
}
