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
    object: Option<IUnknown>,
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
                    VT_I4 => {
                        let child = i32::try_from(&focus)?;
                        if child != 0 {
                            accessible = accessible.get_accChild(&focus)?.cast()?;
                            continue;
                        }
                        let child = VARIANT::from(0i32);
                        let role = i32::try_from(&accessible.get_accRole(&child)?)?;
                        let state = i32::try_from(&accessible.get_accState(&child)?)? as u32;
                        // A generic window/document focus is not an editable field identity.
                        if [
                            ROLE_SYSTEM_WINDOW as i32,
                            ROLE_SYSTEM_CLIENT as i32,
                            ROLE_SYSTEM_DOCUMENT as i32,
                        ]
                        .contains(&role)
                            || state & STATE_SYSTEM_FOCUSED == 0
                            || state & (STATE_SYSTEM_PROTECTED | STATE_SYSTEM_READONLY) != 0
                        {
                            return Err(E_ACCESSDENIED.into());
                        }
                        return Ok(Self {
                            window,
                            class,
                            object: Some(accessible.cast()?),
                        });
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
            && current.object == self.object)
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
