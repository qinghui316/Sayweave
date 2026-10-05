use super::*;
use std::{
    collections::HashMap,
    mem::size_of,
    sync::{Mutex, atomic::AtomicU64},
    thread,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::*,
        Security::*,
        System::{
            Com::*, DataExchange::*, Memory::*, SystemInformation::GetTickCount, Threading::*,
        },
        UI::{
            Accessibility::*,
            Input::{Ime::*, KeyboardAndMouse::*},
            WindowsAndMessaging::*,
        },
    },
    core::{Interface, PCWSTR},
};

fn err(code: &str) -> Error {
    Error::new(code)
}
fn hwnd(value: usize) -> HWND {
    HWND(value as *mut _)
}
static CHANGES: AtomicU64 = AtomicU64::new(0);
static WATCH: Mutex<Option<(usize, usize, u32)>> = Mutex::new(None);
pub(super) fn start_anchor() -> Result<Anchor, Error> {
    let a = anchor()?;
    if let Ok(mut watch) = WATCH.lock() {
        *watch = Some((a.window, a.focus, a.captured_tick));
    }
    Ok(a)
}
fn event_invalidates(event: u32, window: usize, top: usize, focus: usize) -> bool {
    event == EVENT_OBJECT_DESTROY && (window == top || window == focus)
        || event == EVENT_SYSTEM_FOREGROUND && window != top
        || event == EVENT_OBJECT_FOCUS
            && window != focus
            && unsafe { GetAncestor(hwnd(window), GA_ROOT) == hwnd(top) }
        || [EVENT_OBJECT_VALUECHANGE, EVENT_OBJECT_TEXTSELECTIONCHANGED].contains(&event)
            && (window == focus || window == top)
}
pub(super) fn observe(event: u32, window: usize, _object: i32, event_tick: u32) {
    if let Ok(watch) = WATCH.lock()
        && let Some((top, focus, captured_tick)) = *watch
        && event_after_capture(event_tick, captured_tick)
    {
        // Caret geometry can change without an edit. Selection/range proof owns text position.
        if event_invalidates(event, window, top, focus) {
            CHANGES.fetch_add(1, Ordering::AcqRel);
            log::info!(target:"sayweave", "operation=input_target_event event={event}");
        }
    }
}
#[cfg(test)]
mod event_tests {
    use super::*;
    #[test]
    fn caret_geometry_is_advisory_but_text_and_selection_invalidate() {
        assert!(!event_invalidates(EVENT_OBJECT_LOCATIONCHANGE, 12, 10, 12));
        assert!(event_invalidates(
            EVENT_OBJECT_TEXTSELECTIONCHANGED,
            12,
            10,
            12
        ));
        assert!(event_invalidates(EVENT_OBJECT_VALUECHANGE, 12, 10, 12));
        assert!(event_invalidates(EVENT_OBJECT_DESTROY, 12, 10, 12));
        assert!(event_invalidates(EVENT_SYSTEM_FOREGROUND, 14, 10, 12));
    }
}
fn lifetime(pid: u32) -> Result<u64, Error> {
    unsafe {
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map_err(|_| err("targetPermission"))?;
        let (mut created, mut exit, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        let result = GetProcessTimes(p, &mut created, &mut exit, &mut kernel, &mut user);
        let _ = CloseHandle(p);
        result.map_err(|_| err("targetPermission"))?;
        Ok((created.dwHighDateTime as u64) << 32 | created.dwLowDateTime as u64)
    }
}
pub(super) fn anchor() -> Result<Anchor, Error> {
    unsafe {
        let captured_tick = GetTickCount();
        let window = GetForegroundWindow();
        if window.0.is_null() {
            return Err(err("targetUnavailable"));
        }
        let mut process = 0;
        let tid = GetWindowThreadProcessId(window, Some(&mut process));
        if process == std::process::id() {
            return Err(err("targetUnavailable"));
        }
        let mut gui = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(tid, &mut gui).map_err(|_| err("targetProbeFailed"))?;
        Ok(Anchor {
            window: window.0 as usize,
            focus: gui.hwndFocus.0 as usize,
            process,
            lifetime: lifetime(process)?,
            generation: CHANGES.load(Ordering::Acquire),
            captured_tick,
            activity: crate::input_state::activity(),
        })
    }
}
fn same(a: &Anchor) -> bool {
    anchor().is_ok_and(|now| {
        let unchanged = now.window == a.window
            && now.focus == a.focus
            && now.process == a.process
            && now.lifetime == a.lifetime
            && now.generation == a.generation;
        if !unchanged {
            log::info!(target:"sayweave", "operation=input_target_identity window_same={} focus_same={} process_same={} lifetime_same={} generation_same={}", now.window == a.window, now.focus == a.focus, now.process == a.process, now.lifetime == a.lifetime, now.generation == a.generation);
        }
        unchanged
    })
}
unsafe fn message(window: HWND, msg: u32, w: WPARAM, l: LPARAM) -> Result<usize, Error> {
    unsafe {
        budget()?;
        let timeout = DEADLINE.with(|d| {
            d.get()
                .map(|at| {
                    at.saturating_duration_since(Instant::now())
                        .as_millis()
                        .clamp(1, 100) as u32
                })
                .unwrap_or(100)
        });
        let mut result = 0;
        if SendMessageTimeoutW(
            window,
            msg,
            w,
            l,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            timeout,
            Some(&mut result),
        )
        .0 == 0
        {
            Err(err("targetTimeout"))
        } else {
            Ok(result)
        }
    }
}
fn native_text(window: HWND) -> Result<String, Error> {
    unsafe {
        let len = message(window, WM_GETTEXTLENGTH, WPARAM(0), LPARAM(0))?;
        if len > 32768 {
            return Err(err("targetReadUnavailable"));
        }
        let mut text = vec![0u16; len + 1];
        let count = message(
            window,
            WM_GETTEXT,
            WPARAM(text.len()),
            LPARAM(text.as_mut_ptr() as isize),
        )?;
        String::from_utf16(&text[..count.min(len)]).map_err(|_| err("targetReadUnavailable"))
    }
}
fn native_selection(window: HWND) -> Result<(usize, usize), Error> {
    unsafe {
        let (mut start, mut end) = (0u32, 0u32);
        message(
            window,
            0x00B0,
            WPARAM(&mut start as *mut u32 as usize),
            LPARAM(&mut end as *mut u32 as isize),
        )?;
        Ok((start as usize, end as usize))
    }
}

thread_local! { static DEADLINE:std::cell::Cell<Option<Instant>>=const{std::cell::Cell::new(None)}; }
fn deadline(ms: u64) {
    DEADLINE.with(|v| v.set(Some(Instant::now() + Duration::from_millis(ms))));
}
fn budget() -> Result<(), Error> {
    if DEADLINE.with(|v| v.get().is_some_and(|at| Instant::now() >= at)) {
        Err(err("targetTimeout"))
    } else {
        Ok(())
    }
}
macro_rules! probe {
    ($value:expr) => {{
        budget()?;
        $value
    }};
}
fn integrity(pid: u32) -> Result<u32, Error> {
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map_err(|_| err("targetPermission"))?;
        let mut token = HANDLE::default();
        let opened = OpenProcessToken(process, TOKEN_QUERY, &mut token);
        let _ = CloseHandle(process);
        opened.map_err(|_| err("targetPermission"))?;
        let mut bytes = vec![0u64; 64];
        let mut size = 0;
        let ok = GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(bytes.as_mut_ptr().cast()),
            (bytes.len() * 8) as u32,
            &mut size,
        );
        let _ = CloseHandle(token);
        ok.map_err(|_| err("targetPermission"))?;
        let label = &*(bytes.as_ptr().cast::<TOKEN_MANDATORY_LABEL>());
        let count = *GetSidSubAuthorityCount(label.Label.Sid);
        Ok(*GetSidSubAuthority(label.Label.Sid, (count - 1) as u32))
    }
}
fn permitted(a: &Anchor) -> bool {
    integrity(a.process)
        .and_then(|target| integrity(std::process::id()).map(|own| target <= own))
        .unwrap_or(false)
}
fn composing(a: &Anchor) -> bool {
    unsafe {
        let h = hwnd(a.focus);
        let context = ImmGetContext(h);
        if context.0.is_null() {
            return false;
        }
        let count = ImmGetCompositionStringW(context, GCS_COMPSTR, None, 0);
        let _ = ImmReleaseContext(h, context);
        count > 0
    }
}
#[windows::core::implement(IUIAutomationEventHandler)]
struct Changes {
    dirty: Arc<AtomicBool>,
}
impl IUIAutomationEventHandler_Impl for Changes_Impl {
    fn HandleAutomationEvent(
        &self,
        _: windows::core::Ref<IUIAutomationElement>,
        _: UIA_EVENT_ID,
    ) -> windows::core::Result<()> {
        self.dirty.store(true, Ordering::Release);
        Ok(())
    }
}
enum Proof {
    Edit {
        before: Option<String>,
        start: usize,
        end: usize,
    },
    Uia {
        element: IUIAutomationElement,
        pattern: Option<IUIAutomationTextPattern>,
        context: Option<(String, String, String)>,
        prefix_start: Option<IUIAutomationTextRange>,
        at_document_start: bool,
        initially_empty: bool,
        selection: Option<IUIAutomationTextRange>,
        position: Option<(i32, i32)>,
    },
}
struct Target {
    anchor: Anchor,
    proof: Proof,
    dirty: Arc<AtomicBool>,
    watching: bool,
    handler: Option<IUIAutomationEventHandler>,
    automation: Option<IUIAutomation>,
}
impl Drop for Target {
    fn drop(&mut self) {
        unwatch(self, &self.automation);
    }
}
fn uia_selection(pattern: &IUIAutomationTextPattern) -> Result<IUIAutomationTextRange, Error> {
    unsafe {
        let ranges =
            probe!(pattern.GetSelection()).map_err(|_| err("targetSelectionUnavailable"))?;
        if probe!(ranges.Length()).map_err(|_| err("targetSelectionUnavailable"))? != 1 {
            return Err(err("targetSelectionUnavailable"));
        }
        probe!(ranges.GetElement(0)).map_err(|_| err("targetSelectionUnavailable"))
    }
}
fn uia_position(pattern: &IUIAutomationTextPattern) -> Result<(i32, i32), Error> {
    unsafe {
        let selected = uia_selection(pattern)?;
        let offset = |endpoint| -> Result<i32, Error> {
            let range = probe!(selected.Clone()).map_err(|_| err("targetSelectionUnavailable"))?;
            probe!(range.MoveEndpointByRange(TextPatternRangeEndpoint_Start, &selected, endpoint))
                .map_err(|_| err("targetSelectionUnavailable"))?;
            probe!(range.MoveEndpointByRange(TextPatternRangeEndpoint_End, &selected, endpoint))
                .map_err(|_| err("targetSelectionUnavailable"))?;
            probe!(range.MoveEndpointByUnit(
                TextPatternRangeEndpoint_Start,
                TextUnit_Character,
                -i32::MAX
            ))
            .map_err(|_| err("targetSelectionUnavailable"))?
            .checked_neg()
            .ok_or_else(|| err("targetSelectionUnavailable"))
        };
        Ok((
            offset(TextPatternRangeEndpoint_Start)?,
            offset(TextPatternRangeEndpoint_End)?,
        ))
    }
}
// Read only the selection and bounded adjacent context, independently of editability.
type UiaContext = ((String, String, String), IUIAutomationTextRange);
fn empty_value_candidate(prefix: &str, selected: &str, suffix: &str) -> bool {
    prefix.is_empty()
        && selected.is_empty()
        && suffix
            .chars()
            .all(|c| matches!(c, '\u{fffc}' | '\r' | '\n'))
}
#[cfg(test)]
mod empty_value_tests {
    use super::empty_value_candidate;
    #[test]
    fn virtual_objects_are_candidates_but_real_text_is_not() {
        assert!(empty_value_candidate("", "", "\u{fffc}\r\n\u{fffc}"));
        assert!(!empty_value_candidate("", "", "\u{fffc}tail"));
        assert!(!empty_value_candidate("before", "", "\u{fffc}"));
        assert!(!empty_value_candidate("", "selected", "\u{fffc}"));
    }
}
fn uia_context(pattern: &IUIAutomationTextPattern) -> Result<UiaContext, Error> {
    unsafe {
        let doc = probe!(pattern.DocumentRange()).map_err(|_| err("targetReadUnavailable"))?;
        let selected = match uia_selection(pattern) {
            Ok(range) => range,
            Err(error) => {
                // Some providers expose no selected range until the empty editor contains text.
                if !probe!(doc.GetText(1))
                    .map_err(|_| err("targetReadUnavailable"))?
                    .is_empty()
                {
                    return Err(error);
                }
                return Ok(((String::new(), String::new(), String::new()), doc));
            }
        };
        let prefix = probe!(selected.Clone()).map_err(|_| err("targetReadUnavailable"))?;
        probe!(prefix.MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            &selected,
            TextPatternRangeEndpoint_Start
        ))
        .map_err(|_| err("targetReadUnavailable"))?;
        probe!(prefix.MoveEndpointByUnit(
            TextPatternRangeEndpoint_Start,
            TextUnit_Character,
            -2048
        ))
        .map_err(|_| err("targetReadUnavailable"))?;
        // Chromium ranges can move into sibling controls even though DocumentRange belongs
        // to this editor. Bound context to that document before reading any surrounding text.
        if probe!(prefix.CompareEndpoints(
            TextPatternRangeEndpoint_Start,
            &doc,
            TextPatternRangeEndpoint_Start
        ))
        .map_err(|_| err("targetReadUnavailable"))?
            < 0
        {
            probe!(prefix.MoveEndpointByRange(
                TextPatternRangeEndpoint_Start,
                &doc,
                TextPatternRangeEndpoint_Start
            ))
            .map_err(|_| err("targetReadUnavailable"))?;
        }
        let prefix_start = probe!(prefix.Clone()).map_err(|_| err("targetReadUnavailable"))?;
        probe!(prefix_start.MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            &prefix,
            TextPatternRangeEndpoint_Start
        ))
        .map_err(|_| err("targetReadUnavailable"))?;
        let suffix = probe!(selected.Clone()).map_err(|_| err("targetReadUnavailable"))?;
        probe!(suffix.MoveEndpointByRange(
            TextPatternRangeEndpoint_Start,
            &selected,
            TextPatternRangeEndpoint_End
        ))
        .map_err(|_| err("targetReadUnavailable"))?;
        probe!(suffix.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, 2048))
            .map_err(|_| err("targetReadUnavailable"))?;
        if probe!(suffix.CompareEndpoints(
            TextPatternRangeEndpoint_End,
            &doc,
            TextPatternRangeEndpoint_End
        ))
        .map_err(|_| err("targetReadUnavailable"))?
            > 0
        {
            probe!(suffix.MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                &doc,
                TextPatternRangeEndpoint_End
            ))
            .map_err(|_| err("targetReadUnavailable"))?;
        }
        let prefix = probe!(prefix.GetText(8193))
            .map_err(|_| err("targetReadUnavailable"))?
            .to_string();
        let value = probe!(selected.GetText(8193))
            .map_err(|_| err("targetReadUnavailable"))?
            .to_string();
        if value.encode_utf16().count() > 8192 {
            return Err(err("targetReadUnavailable"));
        }
        let suffix = probe!(suffix.GetText(8193))
            .map_err(|_| err("targetReadUnavailable"))?
            .to_string();
        if prefix.encode_utf16().count() > 8192 || suffix.encode_utf16().count() > 8192 {
            return Err(err("targetReadUnavailable"));
        }
        Ok((
            (normalize(&prefix), normalize(&value), normalize(&suffix)),
            prefix_start,
        ))
    }
}
fn uia_parts(pattern: &IUIAutomationTextPattern) -> Result<(String, String, String), Error> {
    uia_context(pattern).map(|(text, _)| text)
}
fn uia_composing(element: &IUIAutomationElement) -> Result<bool, Error> {
    unsafe {
        budget()?;
        let Ok(pattern) =
            element.GetCurrentPatternAs::<IUIAutomationTextEditPattern>(UIA_TextEditPatternId)
        else {
            return Ok(false);
        };
        budget()?;
        // This API legitimately returns a null range when no IME conversion is active.
        let mut range = std::ptr::null_mut();
        (Interface::vtable(&pattern).GetActiveComposition)(Interface::as_raw(&pattern), &mut range)
            .ok()
            .map_err(|_| err("targetProbeFailed"))?;
        let active = !range.is_null();
        if active {
            drop(IUIAutomationTextRange::from_raw(range));
        }
        budget()?;
        Ok(active)
    }
}
fn exact_uia_bool(value: &windows::Win32::System::Variant::VARIANT) -> Result<bool, Error> {
    if value.vt() != windows::Win32::System::Variant::VT_BOOL {
        return Err(err("targetEditabilityUnknown"));
    }
    bool::try_from(value).map_err(|_| err("targetEditabilityUnknown"))
}
// A writable ValuePattern or current editing range proves admission. Whole-document
// attributes can be mixed in rich editors and are not evidence about the current caret.
fn uia_readonly(
    element: &IUIAutomationElement,
    pattern: Option<&IUIAutomationTextPattern>,
) -> Result<bool, Error> {
    unsafe {
        if let Ok(value) =
            probe!(element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId))
        {
            return Ok(probe!(value.CurrentIsReadOnly())
                .map_err(|_| err("targetProbeFailed"))?
                .as_bool());
        }
        let pattern = pattern.ok_or_else(|| err("targetEditabilityUnknown"))?;
        let selected = uia_selection(pattern)?;
        let attribute = probe!(selected.GetAttributeValue(UIA_IsReadOnlyAttributeId))
            .map_err(|_| err("targetEditabilityUnknown"))?;
        exact_uia_bool(&attribute).map_err(|_| err("targetEditabilityUnknown"))
    }
}
fn capture(a: Anchor, automation: &Option<IUIAutomation>) -> Result<Target, Error> {
    unsafe {
        if !same(&a) {
            return Err(err("targetChanged"));
        }
        if !permitted(&a) {
            return Err(err("targetPermission"));
        }
        if composing(&a) {
            return Err(err("inputComposition"));
        }
        let control = hwnd(a.focus);
        let mut class = [0u16; 128];
        let count = GetClassNameW(control, &mut class) as usize;
        let proof = if String::from_utf16_lossy(&class[..count]) == "Edit" {
            let style = GetWindowLongPtrW(control, GWL_STYLE) as u32;
            if style & 0x20 != 0 {
                return Err(err("targetPassword"));
            }
            if style & 0x800 != 0 || !IsWindowEnabled(control).as_bool() {
                return Err(err("targetReadonly"));
            }
            let before = native_text(control).ok();
            let (start, end) = native_selection(control)?;
            if before
                .as_ref()
                .is_some_and(|before| expected_text(before, start, end, "").is_none())
            {
                return Err(err("targetSelectionUnavailable"));
            }
            if before.is_none() && !crate::input_state::tracking_ready() {
                return Err(err("targetTrackingUnavailable"));
            }
            Proof::Edit { before, start, end }
        } else {
            let auto = automation
                .as_ref()
                .ok_or_else(|| err("targetProbeFailed"))?;
            let element = probe!(auto.GetFocusedElement()).map_err(|_| err("targetProbeFailed"))?;
            if uia_composing(&element)? {
                return Err(err("inputComposition"));
            }
            if probe!(element.CurrentProcessId()).map_err(|_| err("targetProbeFailed"))?
                != a.process as i32
            {
                return Err(err("targetChanged"));
            }
            if probe!(element.CurrentIsPassword())
                .map_err(|_| err("targetProbeFailed"))?
                .as_bool()
            {
                return Err(err("targetPassword"));
            }
            if !probe!(element.CurrentIsEnabled())
                .map_err(|_| err("targetProbeFailed"))?
                .as_bool()
            {
                return Err(err("targetReadonly"));
            }
            let pattern =
                probe!(element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId))
                    .ok();
            if uia_readonly(&element, pattern.as_ref())? {
                return Err(err("targetReadonly"));
            }
            let readback = pattern.as_ref().and_then(|p| uia_context(p).ok());
            let context = readback.as_ref().map(|(text, _)| text.clone());
            let prefix_start = readback.map(|(_, range)| range);
            let at_document_start = match (&pattern, &prefix_start) {
                (Some(pattern), Some(start)) => pattern.DocumentRange().ok().is_some_and(|doc| {
                    start
                        .CompareEndpoints(
                            TextPatternRangeEndpoint_Start,
                            &doc,
                            TextPatternRangeEndpoint_Start,
                        )
                        .is_ok_and(|offset| offset == 0)
                }),
                _ => false,
            };
            let document_probe = pattern
                .as_ref()
                .and_then(|pattern| pattern.DocumentRange().and_then(|doc| doc.GetText(1)).ok());
            let initially_empty = document_probe.as_ref().is_some_and(|text| text.is_empty())
                || context.as_ref().is_some_and(|(prefix, selected, suffix)| {
                    // Empty text controls can expose a virtual object in their TextPattern.
                    // The bounded context must contain only virtual content, then the actual
                    // ValuePattern must independently be strictly empty.
                    empty_value_candidate(prefix, selected, suffix)
                        && element
                            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                            .and_then(|value| value.CurrentValue())
                            .is_ok_and(|value| value.is_empty())
                });
            let selection = pattern.as_ref().and_then(|p| uia_selection(p).ok());
            let position = pattern.as_ref().and_then(|p| uia_position(p).ok());
            if context.is_none() && selection.is_none() && !crate::input_state::tracking_ready() {
                return Err(err("targetTrackingUnavailable"));
            }
            Proof::Uia {
                element,
                pattern,
                context,
                prefix_start,
                at_document_start,
                initially_empty,
                selection,
                position,
            }
        };
        let mut target = Target {
            anchor: a,
            proof,
            dirty: Arc::new(AtomicBool::new(false)),
            watching: false,
            handler: None,
            automation: automation.clone(),
        };
        if let Proof::Uia { element, .. } = &target.proof {
            let handler: IUIAutomationEventHandler = Changes {
                dirty: target.dirty.clone(),
            }
            .into();
            target.handler = Some(handler.clone());
            let auto = automation.as_ref().unwrap();
            let text_watch = probe!(auto.AddAutomationEventHandler(
                UIA_Text_TextChangedEventId,
                element,
                TreeScope_Element,
                None,
                &handler
            ))
            .is_ok();
            let selection_watch = probe!(auto.AddAutomationEventHandler(
                UIA_Text_TextSelectionChangedEventId,
                element,
                TreeScope_Element,
                None,
                &handler
            ))
            .is_ok();
            target.watching = text_watch && selection_watch;
            if let Proof::Uia {
                selection,
                position,
                ..
            } = &target.proof
                && selection.is_none()
                && position.is_none()
                && !target.watching
            {
                return Err(err("targetTrackingUnavailable"));
            }
        }
        budget()?;
        if !same(&target.anchor) {
            return Err(err("targetChanged"));
        }
        Ok(target)
    }
}
fn unchanged(t: &Target, automation: &Option<IUIAutomation>) -> Result<(), Error> {
    if !same(&t.anchor) || crate::input_state::activity() != t.anchor.activity {
        return Err(err("targetChanged"));
    }
    if !permitted(&t.anchor) {
        return Err(err("targetPermission"));
    }
    if composing(&t.anchor) {
        return Err(err("inputComposition"));
    }
    budget()?;
    match &t.proof {
        Proof::Edit { before, start, end } => unsafe {
            let h = hwnd(t.anchor.focus);
            let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
            if style & 0x20 != 0 {
                return Err(err("targetPassword"));
            }
            if style & 0x800 != 0 || !IsWindowEnabled(h).as_bool() {
                return Err(err("targetReadonly"));
            }
            if native_selection(h)? != (*start, *end) {
                return Err(err("targetChanged"));
            }
            if let Some(before) = before {
                if native_text(h)? != *before {
                    return Err(err("targetChanged"));
                }
            } else if !crate::input_state::tracking_ready() {
                return Err(err("targetTrackingUnavailable"));
            }
        },
        Proof::Uia {
            element,
            pattern,
            context,
            selection,
            position,
            ..
        } => unsafe {
            if element
                .CurrentIsPassword()
                .map_err(|_| err("targetProbeFailed"))?
                .as_bool()
            {
                return Err(err("targetPassword"));
            }
            if !element
                .CurrentIsEnabled()
                .map_err(|_| err("targetProbeFailed"))?
                .as_bool()
                || uia_readonly(element, pattern.as_ref())?
            {
                return Err(err("targetReadonly"));
            }
            if uia_composing(element)? {
                return Err(err("inputComposition"));
            }
            let auto = automation
                .as_ref()
                .ok_or_else(|| err("targetProbeFailed"))?;
            let now = probe!(auto.GetFocusedElement()).map_err(|_| err("targetProbeFailed"))?;
            if !probe!(auto.CompareElements(element, &now))
                .map_err(|_| err("targetProbeFailed"))?
                .as_bool()
            {
                return Err(err("targetChanged"));
            }
            if t.dirty.load(Ordering::Acquire) {
                return Err(err("targetChanged"));
            }
            if let (Some(pattern), Some(position)) = (pattern, position)
                && uia_position(pattern)? != *position
            {
                return Err(err("targetChanged"));
            }
            if let (Some(pattern), Some(old)) = (pattern, selection) {
                let now = uia_selection(pattern)?;
                if now
                    .CompareEndpoints(
                        TextPatternRangeEndpoint_Start,
                        old,
                        TextPatternRangeEndpoint_Start,
                    )
                    .map_err(|_| err("targetSelectionUnavailable"))?
                    != 0
                    || now
                        .CompareEndpoints(
                            TextPatternRangeEndpoint_End,
                            old,
                            TextPatternRangeEndpoint_End,
                        )
                        .map_err(|_| err("targetSelectionUnavailable"))?
                        != 0
                {
                    return Err(err("targetChanged"));
                }
            }
            if let (Some(pattern), Some(old)) = (pattern, context)
                && uia_parts(pattern)? != *old
            {
                return Err(err("targetChanged"));
            }
            if selection.is_none()
                && position.is_none()
                && (!crate::input_state::tracking_ready() || !t.watching)
            {
                return Err(err("targetTrackingUnavailable"));
            }
        },
    }
    budget()?;
    if same(&t.anchor) {
        Ok(())
    } else {
        Err(err("targetChanged"))
    }
}
fn verified(t: &Target, text: &str) -> Result<bool, Error> {
    match &t.proof {
        Proof::Edit { before, start, end } => {
            let before = before
                .as_ref()
                .ok_or_else(|| err("targetReadUnavailable"))?;
            let expected = expected_text(before, *start, *end, text)
                .ok_or_else(|| err("targetReadUnavailable"))?;
            let (a, b) = native_selection(hwnd(t.anchor.focus))?;
            let actual = native_text(hwnd(t.anchor.focus))?;
            Ok(normalize(&actual) == expected
                && a == b
                && insertion_prefix(&actual, a, "") == insertion_prefix(before, *start, text))
        }
        Proof::Uia {
            element,
            pattern: Some(pattern),
            context: Some((prefix, _, suffix)),
            prefix_start: Some(prefix_start),
            at_document_start,
            initially_empty,
            ..
        } => unsafe {
            // For an initially empty field, its complete value is also exact insertion proof.
            // Chromium can replace the empty TextPattern provider on the first paste.
            if *initially_empty
                && text.chars().count() <= 1024
                && let Ok(value) = probe!(
                    element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                )
                && let Ok(actual) = probe!(value.CurrentValue())
                && normalize(&actual.to_string()) == normalize(text)
            {
                return Ok(true);
            }
            let fresh_pattern;
            let pattern = if *at_document_start {
                fresh_pattern = probe!(
                    element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                )
                .map_err(|_| err("targetReadUnavailable"))?;
                &fresh_pattern
            } else {
                pattern
            };
            let (_, selected, after) = uia_parts(pattern)?;
            let caret = uia_selection(pattern)?;
            if probe!(caret.CompareEndpoints(
                TextPatternRangeEndpoint_Start,
                &caret,
                TextPatternRangeEndpoint_End
            ))
            .map_err(|_| err("targetReadUnavailable"))?
                != 0
            {
                return Ok(false);
            }
            if text.chars().count() > 1024 {
                return Err(err("targetReadUnavailable"));
            }
            // Preserve the provider's range boundary; Character units are not UTF-16 offsets.
            let inserted = probe!(caret.Clone()).map_err(|_| err("targetReadUnavailable"))?;
            let boundary = if *at_document_start {
                // An empty provider may replace its old live range when the first text is inserted.
                probe!(pattern.DocumentRange()).map_err(|_| err("targetReadUnavailable"))?
            } else {
                prefix_start.clone()
            };
            probe!(inserted.MoveEndpointByRange(
                TextPatternRangeEndpoint_Start,
                &boundary,
                TextPatternRangeEndpoint_Start
            ))
            .map_err(|_| err("targetReadUnavailable"))?;
            let actual = probe!(inserted.GetText(16385))
                .map_err(|_| err("targetReadUnavailable"))?
                .to_string();
            if actual.encode_utf16().count() > 16384 {
                return Err(err("targetReadUnavailable"));
            }
            Ok(selected.is_empty()
                && normalize(&actual) == format!("{prefix}{}", normalize(text))
                && after == *suffix)
        },
        _ => Err(err("targetReadUnavailable")),
    }
}
fn unwatch(t: &Target, auto: &Option<IUIAutomation>) {
    if let (Some(handler), Proof::Uia { element, .. }, Some(a)) = (&t.handler, &t.proof, auto) {
        unsafe {
            let _ = a.RemoveAutomationEventHandler(UIA_Text_TextChangedEventId, element, handler);
            let _ = a.RemoveAutomationEventHandler(
                UIA_Text_TextSelectionChangedEventId,
                element,
                handler,
            );
        }
    }
}
pub(super) struct Clipboard;
impl Drop for Clipboard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}
pub(super) fn open(owner: usize) -> Result<Clipboard, Error> {
    let begin = Instant::now();
    loop {
        if unsafe { OpenClipboard(Some(hwnd(owner))) }.is_ok() {
            return Ok(Clipboard);
        }
        if begin.elapsed() >= Duration::from_millis(200) {
            return Err(err("clipboardBusy"));
        }
        thread::sleep(Duration::from_millis(10));
    }
}
pub(super) fn format(name: &str) -> u32 {
    let wide: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
    unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) }
}
pub(super) fn backup() -> Result<Vec<(u32, Vec<u8>)>, Error> {
    unsafe {
        let mut output = vec![];
        let mut last = 0;
        let mut total = 0;
        let common = [
            format("HTML Format"),
            format("Rich Text Format"),
            format("PNG"),
            format("CanIncludeInClipboardHistory"),
            format("CanUploadToCloudClipboard"),
            format("ExcludeClipboardContentFromMonitorProcessing"),
            // Chromium text/HTML copies carry opaque source metadata in global memory.
            // Preserve the complete payload without interpreting or reporting its content.
            format("Chromium internal source URL"),
            format("Chromium internal source RFH token"),
        ];
        loop {
            budget()?;
            SetLastError(WIN32_ERROR(0));
            let next = EnumClipboardFormats(last);
            if next == 0 {
                if GetLastError().0 != 0 {
                    return Err(err("clipboardUnsupported"));
                }
                break;
            }
            last = next;
            // Windows synthesizes bitmap handles from DIB; retain the independent pixel formats instead.
            if next == 2 && IsClipboardFormatAvailable(8).is_ok() {
                continue;
            }
            if ![1, 7, 8, 13, 15, 16, 17].contains(&next) && !common.contains(&next) {
                return Err(err("clipboardUnsupported"));
            }
            let h = GetClipboardData(next).map_err(|_| err("clipboardUnsupported"))?;
            let handle = HGLOBAL(h.0);
            let size = GlobalSize(handle);
            total += size;
            if size == 0 || total > 32 * 1024 * 1024 {
                return Err(err("clipboardUnsupported"));
            }
            let data = GlobalLock(handle);
            if data.is_null() {
                return Err(err("clipboardUnsupported"));
            }
            let bytes = std::slice::from_raw_parts(data.cast::<u8>(), size).to_vec();
            let _ = GlobalUnlock(handle);
            output.push((next, bytes));
        }
        Ok(output)
    }
}
static BACKUP_BUSY: AtomicBool = AtomicBool::new(false);
// Some owners render clipboard formats synchronously inside GetClipboardData (Windows may wait 30s).
// A single read-only helper bounds our wait. Its late result can never modify the clipboard or input.
fn protected_backup(owner: usize) -> Result<(Backup, u32), Error> {
    if BACKUP_BUSY.swap(true, Ordering::AcqRel) {
        return Err(err("clipboardBusy"));
    }
    let (tx, rx) = mpsc::sync_channel(1);
    if thread::Builder::new()
        .name("sayweave-clipboard-read".into())
        .spawn(move || {
            deadline(500);
            let result = (|| {
                let _lock = open(owner)?;
                let saved = backup()?;
                Ok((saved, unsafe { GetClipboardSequenceNumber() }))
            })();
            BACKUP_BUSY.store(false, Ordering::Release);
            let _ = tx.send(result);
        })
        .is_err()
    {
        BACKUP_BUSY.store(false, Ordering::Release);
        return Err(err("clipboardBusy"));
    }
    rx.recv_timeout(Duration::from_millis(500))
        .map_err(|_| err("clipboardBusy"))?
}
struct Allocation {
    format: u32,
    handle: Option<HGLOBAL>,
}
#[cfg(feature = "desktop-acceptance")]
pub(super) static FAIL_ALLOCATION: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "desktop-acceptance")]
pub(super) static FAIL_PUT_AT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "desktop-acceptance")]
pub(super) static FAIL_PUT_CHAIN: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(feature = "desktop-acceptance")]
pub(super) static FAIL_PUT_LEFT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
impl Allocation {
    fn new(format: u32, bytes: &[u8]) -> Result<Self, Error> {
        #[cfg(feature = "desktop-acceptance")]
        if FAIL_ALLOCATION.swap(false, Ordering::AcqRel) {
            return Err(err("copyFailed"));
        }
        unsafe {
            let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len().max(1))
                .map_err(|_| err("copyFailed"))?;
            let p = GlobalLock(h);
            if p.is_null() {
                let _ = GlobalFree(Some(h));
                return Err(err("copyFailed"));
            }
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), p.cast::<u8>(), bytes.len());
            let _ = GlobalUnlock(h);
            Ok(Self {
                format,
                handle: Some(h),
            })
        }
    }
    fn put(&mut self) -> Result<(), Error> {
        #[cfg(feature = "desktop-acceptance")]
        if FAIL_PUT_LEFT
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
            .is_ok()
        {
            return Err(err("copyFailed"));
        }
        #[cfg(feature = "desktop-acceptance")]
        if FAIL_PUT_AT.try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1))
            == Ok(1)
        {
            FAIL_PUT_LEFT.store(FAIL_PUT_CHAIN.swap(0, Ordering::AcqRel), Ordering::Release);
            return Err(err("copyFailed"));
        }
        unsafe {
            let h = self.handle.unwrap();
            SetClipboardData(self.format, Some(HANDLE(h.0))).map_err(|_| err("copyFailed"))?;
            self.handle = None;
            Ok(())
        }
    }
}
impl Drop for Allocation {
    fn drop(&mut self) {
        if let Some(h) = self.handle {
            unsafe {
                let _ = GlobalFree(Some(h));
            }
        }
    }
}
fn put(kind: u32, bytes: &[u8]) -> Result<(), Error> {
    Allocation::new(kind, bytes)?.put()
}
type Backup = Vec<(u32, Vec<u8>)>;
struct Recovery {
    sequence: u32,
    marker: Vec<u8>,
    saved: Backup,
    owner_evidence: bool,
}
thread_local! {static RECOVERY:std::cell::RefCell<Option<Recovery>>=const{std::cell::RefCell::new(None)};}
pub(super) fn install(saved: &[(u32, Vec<u8>)]) -> Result<(), Error> {
    let mut prepared = saved
        .iter()
        .map(|(kind, bytes)| Allocation::new(*kind, bytes))
        .collect::<Result<Vec<_>, _>>()?;
    unsafe {
        EmptyClipboard().map_err(|_| err("clipboardRestore"))?;
    }
    let mut failed = false;
    for item in &mut prepared {
        failed |= item.put().is_err();
    }
    if failed {
        Err(err("clipboardRestore"))
    } else {
        Ok(())
    }
}
pub(super) fn text_bytes(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(Some(0))
        .flat_map(u16::to_ne_bytes)
        .collect()
}
fn write_text(text: &str, marker: &[u8]) -> Result<u32, Error> {
    install(&[
        (13, text_bytes(text)),
        (format("Sayweave.Delivery"), marker.to_vec()),
        (
            format("CanIncludeInClipboardHistory"),
            0u32.to_ne_bytes().to_vec(),
        ),
        (
            format("CanUploadToCloudClipboard"),
            0u32.to_ne_bytes().to_vec(),
        ),
    ])?;
    Ok(unsafe { GetClipboardSequenceNumber() })
}
fn owned(sequence: u32, marker: &[u8]) -> bool {
    unsafe {
        if GetClipboardSequenceNumber() != sequence {
            return false;
        }
        let Ok(h) = GetClipboardData(format("Sayweave.Delivery")) else {
            return false;
        };
        let h = HGLOBAL(h.0);
        let size = GlobalSize(h);
        let p = GlobalLock(h);
        if p.is_null() {
            return false;
        }
        let owned =
            size == marker.len() && std::slice::from_raw_parts(p.cast::<u8>(), size) == marker;
        let _ = GlobalUnlock(h);
        owned
    }
}
fn restore(
    owner: usize,
    sequence: u32,
    marker: &[u8],
    saved: &[(u32, Vec<u8>)],
    owner_evidence: bool,
) -> Result<(), Error> {
    let mut changed = false;
    let result = (|| {
        let _lock = open(owner)?;
        if !owned(sequence, marker)
            && !(owner_evidence
                && unsafe {
                    GetClipboardSequenceNumber() == sequence
                        && GetClipboardOwner().is_ok_and(|w| w == hwnd(owner))
                })
        {
            return Ok(());
        }
        if let Err(error) = install(saved) {
            changed = true;
            let _ = put(format("Sayweave.Delivery"), marker);
            RECOVERY.with(|r| {
                *r.borrow_mut() = Some(Recovery {
                    sequence: unsafe { GetClipboardSequenceNumber() },
                    marker: marker.to_vec(),
                    saved: saved.to_vec(),
                    owner_evidence: unsafe { GetClipboardOwner().is_ok_and(|w| w == hwnd(owner)) },
                })
            });
            return Err(error);
        }
        Ok(())
    })();
    if result.is_err() {
        RECOVERY.with(|r| {
            let mut r = r.borrow_mut();
            if changed && let Some(r) = r.as_mut() {
                r.sequence = unsafe { GetClipboardSequenceNumber() };
            }
            if r.is_none() {
                *r = Some(Recovery {
                    sequence,
                    marker: marker.to_vec(),
                    saved: saved.to_vec(),
                    owner_evidence,
                });
            }
        });
    }
    result
}
fn retry_restore(owner: usize) -> Result<(), Error> {
    let recovery = RECOVERY.with(|r| r.borrow_mut().take());
    if let Some(r) = recovery {
        restore(owner, r.sequence, &r.marker, &r.saved, r.owner_evidence)
    } else {
        Ok(())
    }
}
fn paste(t: &Target) -> (Submission, u32, Option<Error>) {
    unsafe {
        match &t.proof {
            Proof::Edit { .. } => {
                match message(hwnd(t.anchor.focus), WM_PASTE, WPARAM(0), LPARAM(0)) {
                    Ok(_) => (Submission::Submitted, 1, None),
                    Err(e) => (Submission::Uncertain, 0, Some(e)),
                }
            }
            Proof::Uia { .. } => {
                let inputs = [
                    (VK_CONTROL, 0),
                    (VK_V, 0),
                    (VK_V, KEYEVENTF_KEYUP.0),
                    (VK_CONTROL, KEYEVENTF_KEYUP.0),
                ]
                .map(|(key, flags)| INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: KEYBDINPUT {
                            wVk: key,
                            dwFlags: KEYBD_EVENT_FLAGS(flags),
                            dwExtraInfo: 0x53595756,
                            ..Default::default()
                        },
                    },
                });
                let count = SendInput(&inputs, size_of::<INPUT>() as i32);
                if count == 4 {
                    (Submission::Submitted, count, None)
                } else {
                    if count > 0 {
                        let release = if count == 2 {
                            vec![inputs[2], inputs[3]]
                        } else {
                            vec![inputs[3]]
                        };
                        let _ = SendInput(&release, size_of::<INPUT>() as i32);
                    }
                    (
                        if count == 0 {
                            Submission::NotSubmitted
                        } else {
                            Submission::Uncertain
                        },
                        count,
                        Some(err(if count == 0 {
                            "inputSubmitFailed"
                        } else {
                            "inputUnconfirmed"
                        })),
                    )
                }
            }
        }
    }
}
fn logical_modifiers_down() -> bool {
    unsafe {
        [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
            .iter()
            .any(|k| GetAsyncKeyState(k.0 as i32) < 0)
    }
}
fn deliver(
    t: &Target,
    text: &str,
    cancel: &DeliveryGate,
    auto: &Option<IUIAutomation>,
    owner: usize,
) -> Delivery {
    let began = Instant::now();
    let mut result = Delivery::not_submitted("targetChanged");
    let work = (|| -> Result<(), Error> {
        result.stage = "keyboard".into();
        let wait = Instant::now();
        while let Some(reason) = crate::input_state::modifier_block(
            crate::input_state::modifier_held(),
            logical_modifiers_down(),
        ) {
            if cancel.cancelled() {
                return Err(err("cancelled"));
            }
            if wait.elapsed() >= Duration::from_millis(500) {
                return Err(err(reason));
            }
            thread::sleep(Duration::from_millis(10));
        }
        result.stage = "target".into();
        deadline(500);
        if cancel.cancelled() {
            return Err(err("cancelled"));
        }
        unchanged(t, auto)?;
        result.stage = "clipboard".into();
        let marker = uuid::Uuid::new_v4().as_bytes().to_vec();
        let (saved, before_sequence) = protected_backup(owner)?;
        deadline(500);
        {
            let lock = open(owner)?;
            if unsafe { GetClipboardSequenceNumber() } != before_sequence {
                return Err(err("clipboardChanged"));
            }
            if cancel.cancelled() {
                return Err(err("cancelled"));
            }
            unchanged(t, auto)?;
            if let Err(e) = write_text(text, &marker) {
                if install(&saved).is_err() {
                    let _ = put(format("Sayweave.Delivery"), &marker);
                    drop(lock);
                    RECOVERY.with(|r| {
                        *r.borrow_mut() = Some(Recovery {
                            sequence: unsafe { GetClipboardSequenceNumber() },
                            marker: marker.clone(),
                            saved: saved.clone(),
                            owner_evidence: unsafe {
                                GetClipboardOwner().is_ok_and(|w| w == hwnd(owner))
                            },
                        })
                    });
                    return Err(err("clipboardRestore"));
                }
                return Err(e);
            }
        }
        let sequence = unsafe { GetClipboardSequenceNumber() };
        let applied = (|| -> Result<(), Error> {
            result.stage = "target".into();
            if cancel.cancelled() {
                return Err(err("cancelled"));
            }
            unchanged(t, auto)?;
            {
                let _lock = open(owner)?;
                if !owned(sequence, &marker) {
                    return Err(err("clipboardChanged"));
                }
            }
            unchanged(t, auto)?;
            if unsafe { GetClipboardSequenceNumber() } != sequence {
                return Err(err("clipboardChanged"));
            }
            result.stage = "keyboard".into();
            if let Some(reason) = crate::input_state::modifier_block(
                crate::input_state::modifier_held(),
                logical_modifiers_down(),
            ) {
                return Err(err(reason));
            }
            if !cancel.commit() {
                return Err(err("cancelled"));
            }
            result.stage = "submit".into();
            let (submission, count, error) = paste(t);
            result.submission = submission;
            result.submitted_events = count;
            if let Some(e) = error {
                return Err(e);
            }
            result.stage = "verify".into();
            deadline(1000);
            let at = Instant::now();
            loop {
                match verified(t, text) {
                    Ok(true) => {
                        result.verification = Verification::Confirmed;
                        return Ok(());
                    }
                    Ok(false) => result.verification = Verification::Mismatch,
                    Err(_) => result.verification = Verification::Unavailable,
                }
                let limit = if result.verification == Verification::Unavailable {
                    500
                } else {
                    1000
                };
                if at.elapsed() >= Duration::from_millis(limit) {
                    if result.verification != Verification::Unavailable {
                        result.verification = Verification::Timeout;
                    }
                    return Err(err("inputUnconfirmed"));
                }
                thread::sleep(Duration::from_millis(20));
            }
        })();
        // Restoration is independent of readback: never turn an already submitted paste into a retry.
        result.restore_error = restore(owner, sequence, &marker, &saved, false).is_err();
        applied
    })();
    result.error = work.err().map(|e| e.code);
    result.restore_error |= result.error.as_deref() == Some("clipboardRestore");
    result.state = if result.verification == Verification::Confirmed {
        "confirmed"
    } else if result.submission != Submission::NotSubmitted {
        "unconfirmed"
    } else {
        "copy"
    }
    .into();
    result.elapsed_ms = began.elapsed().as_millis() as u64;
    result
}
pub(super) fn worker(
    receiver: mpsc::Receiver<Envelope>,
    owner: Arc<AtomicUsize>,
    stopping: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let automation: Option<IUIAutomation> =
        unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).ok() };
    if let Some(a) = &automation
        && let Ok(a2) = a.cast::<IUIAutomation2>()
    {
        unsafe {
            let _ = a2.SetConnectionTimeout(50);
            let _ = a2.SetTransactionTimeout(50);
        }
    }
    let mut targets: HashMap<String, Target> = HashMap::new();
    loop {
        let request = receiver.recv().ok();
        let Some(envelope) = request else {
            break;
        };
        let request = envelope.request;
        if matches!(request, Request::Stop) {
            break;
        }
        busy.store(true, Ordering::Release);
        if stopping.load(Ordering::Acquire) {
            match request {
                Request::Capture(_, _, r) | Request::Copy(_, _, r) | Request::Restore(r) => {
                    let _ = r.send(Err(err("stopping")));
                }
                Request::Deliver(_, _, _, r) => {
                    let _ = r.send(Delivery::not_submitted("stopping"));
                }
                _ => {}
            }
            busy.store(false, Ordering::Release);
            break;
        }
        let _ = retry_restore(owner.load(Ordering::Acquire));
        match request {
            Request::Capture(id, anchor, response) => {
                deadline(500);
                let at = Instant::now();
                let result = capture(anchor, &automation).and_then(|t| {
                    if at.elapsed() > Duration::from_millis(500) {
                        Err(err("targetTimeout"))
                    } else {
                        for old in targets.values() {
                            unwatch(old, &automation);
                        }
                        targets.clear();
                        let (readable, initially_empty) = match &t.proof { Proof::Edit { before, .. } => (before.is_some(), before.as_ref().is_some_and(String::is_empty)), Proof::Uia { context, initially_empty, .. } => (context.is_some(), *initially_empty) };
                        log::info!(target:"sayweave", "operation=input_capture task_id={} stage=capture editable=true readable={} initially_empty={} elapsed_ms={}", id, readable, initially_empty, at.elapsed().as_millis());
                        targets.insert(id.clone(), t);
                        Ok(())
                    }
                });
                if let Err(e) = &result {
                    log::info!(target:"sayweave", "operation=input_capture task_id={} stage=capture error={} elapsed_ms={}", id, e.code, at.elapsed().as_millis());
                }
                let _ = response.send(result);
            }
            Request::Deliver(id, text, cancel, response) => {
                deadline(500);
                let result = targets
                    .remove(&id)
                    .map(|t| {
                        let result = deliver(
                            &t,
                            &text,
                            &cancel,
                            &automation,
                            owner.load(Ordering::Acquire),
                        );
                        unwatch(&t, &automation);
                        result
                    })
                    .unwrap_or_else(|| Delivery::not_submitted("targetUnavailable"));
                log::info!(target:"sayweave", "operation=input_delivery task_id={} stage={} submission={:?} verification={:?} submitted_events={} restore_error={} error={} elapsed_ms={}", id, result.stage, result.submission, result.verification, result.submitted_events, result.restore_error, result.error.as_deref().unwrap_or("none"), result.elapsed_ms);
                let _ = response.send(result);
            }
            Request::Copy(text, gate, response) => {
                let result = open(owner.load(Ordering::Acquire)).and_then(|_lock| {
                    if gate.is_some_and(|gate| !gate.commit()) {
                        return Err(err("staleSession"));
                    }
                    install(&[(13, text_bytes(&text))])
                });
                let _ = response.send(result);
            }
            Request::Forget(id) => {
                if let Some(t) = targets.remove(&id) {
                    unwatch(&t, &automation);
                }
            }
            Request::Restore(response) => {
                let _ = response.send(retry_restore(owner.load(Ordering::Acquire)));
            }
            Request::Stop => break,
        }
        busy.store(false, Ordering::Release);
    }
    let _ = retry_restore(owner.load(Ordering::Acquire));
    while BACKUP_BUSY.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(10));
    }
    busy.store(false, Ordering::Release);
    for t in targets.values() {
        unwatch(t, &automation);
    }
    drop(targets);
    drop(automation);
    unsafe {
        CoUninitialize();
    }
}

#[cfg(test)]
mod attribute_tests {
    use super::*;
    use windows::Win32::System::Variant::VARIANT;
    #[test]
    fn only_exact_uia_booleans_prove_pattern_availability_or_editability() {
        assert!(!exact_uia_bool(&VARIANT::from(false)).unwrap());
        assert!(exact_uia_bool(&VARIANT::from(true)).unwrap());
        assert!(exact_uia_bool(&VARIANT::from(0i32)).is_err());
        assert!(exact_uia_bool(&VARIANT::default()).is_err());
    }
}
