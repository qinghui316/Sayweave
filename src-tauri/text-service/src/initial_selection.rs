//! Same-thread selection evidence, kept with its provider until native revalidation.
use windows::{
    Win32::{
        Foundation::*,
        System::{Com::*, Ole::*, Threading::*, Variant::*},
        UI::{Accessibility::*, Input::KeyboardAndMouse::GetFocus, WindowsAndMessaging::*},
    },
    core::{IUnknown, Interface, Result},
};

struct Array(*mut SAFEARRAY);
/// Identity-only focus evidence. No name, value, text, URL or title is read.
#[derive(Clone)]
pub(super) struct FocusIdentity {
    window: HWND,
    class: String,
    object: Option<FieldIdentity>,
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
                if root == window || GetWindowThreadProcessId(root, None) != GetCurrentThreadId() {
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
        if unsafe { GetFocus() } != self.window {
            return Ok(false);
        }
        let current = Self::capture()?;
        Ok(current.window == self.window
            && current.class == self.class
            && match (&self.object, &current.object) {
                (None, None) => true,
                (Some(before), Some(after)) => {
                    before.fragment==after.fragment
                    && before.element==after.element
                    // Keep and revalidate the original leaf, so reuse of a
                    // runtime ID after a field dies cannot revive this scope.
                    && unsafe { identity_bool(&before.provider.GetPropertyValue(UIA_HasKeyboardFocusPropertyId)?)? }
                }
                _ => false,
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
            // fragment to its proven same-thread native root, never to text.
            let raw: IRawElementProviderSimple = root.cast()?;
            let handle = i32::try_from(&raw.GetPropertyValue(UIA_NativeWindowHandlePropertyId)?)?
                as u32 as usize;
            let native = HWND(handle as *mut _);
            if handle == 0
                || native != GetAncestor(window, GA_ROOT)
                || GetWindowThreadProcessId(native, None) != GetCurrentThreadId()
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
impl Evidence {
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
