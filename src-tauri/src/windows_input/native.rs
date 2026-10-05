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
pub(super) fn observe(event: u32, window: usize, object: i32, event_tick: u32) {
    if let Ok(watch) = WATCH.lock()
        && let Some((top, focus, captured_tick)) = *watch
        && event_after_capture(event_tick, captured_tick)
    {
        let changed = event == EVENT_OBJECT_DESTROY && (window == top || window == focus)
            || event == EVENT_SYSTEM_FOREGROUND && window != top
            || event == EVENT_OBJECT_FOCUS
                && window != focus
                && unsafe { GetAncestor(hwnd(window), GA_ROOT) == hwnd(top) }
            || [EVENT_OBJECT_VALUECHANGE, EVENT_OBJECT_TEXTSELECTIONCHANGED].contains(&event)
                && (window == focus || window == top)
            || event == EVENT_OBJECT_LOCATIONCHANGE
                && object == OBJID_CARET.0
                && (window == focus || window == top);
        if changed {
            CHANGES.fetch_add(1, Ordering::AcqRel);
        }
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
        GetGUIThreadInfo(tid, &mut gui).map_err(|_| err("targetUnsupported"))?;
        Ok(Anchor {
            window: window.0 as usize,
            focus: gui.hwndFocus.0 as usize,
            process,
            lifetime: lifetime(process)?,
            generation: CHANGES.load(Ordering::Acquire),
            captured_tick,
        })
    }
}
fn same(a: &Anchor) -> bool {
    anchor().is_ok_and(|now| {
        now.window == a.window
            && now.focus == a.focus
            && now.process == a.process
            && now.lifetime == a.lifetime
            && now.generation == a.generation
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
            return Err(err("targetUnsupported"));
        }
        let mut text = vec![0u16; len + 1];
        let count = message(
            window,
            WM_GETTEXT,
            WPARAM(text.len()),
            LPARAM(text.as_mut_ptr() as isize),
        )?;
        String::from_utf16(&text[..count.min(len)]).map_err(|_| err("targetUnsupported"))
    }
}
fn native_selection(window: HWND) -> Result<(usize, usize), Error> {
    unsafe {
        let value = message(window, 0x00B0, WPARAM(0), LPARAM(0))?;
        Ok((value & 65535, (value >> 16) & 65535))
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
        count != 0
    }
}
#[windows::core::implement(IUIAutomationEventHandler)]
struct Changes {
    generation: u64,
}
impl IUIAutomationEventHandler_Impl for Changes_Impl {
    fn HandleAutomationEvent(
        &self,
        _: windows::core::Ref<IUIAutomationElement>,
        _: UIA_EVENT_ID,
    ) -> windows::core::Result<()> {
        let _ = CHANGES.compare_exchange(
            self.generation,
            self.generation + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        Ok(())
    }
}
enum Proof {
    Edit {
        before: String,
        start: usize,
        end: usize,
    },
    Uia {
        element: IUIAutomationElement,
        pattern: IUIAutomationTextPattern,
        before: String,
        prefix: String,
        selected: String,
    },
}
struct Target {
    anchor: Anchor,
    proof: Proof,
    invalid: bool,
    handler: Option<IUIAutomationEventHandler>,
    automation: Option<IUIAutomation>,
}
impl Drop for Target {
    fn drop(&mut self) {
        unwatch(self, &self.automation);
    }
}
fn uia_parts(pattern: &IUIAutomationTextPattern) -> Result<(String, String, String), Error> {
    unsafe {
        let doc = probe!(pattern.DocumentRange()).map_err(|_| err("targetUnsupported"))?;
        let ranges = probe!(pattern.GetSelection()).map_err(|_| err("targetUnsupported"))?;
        if probe!(ranges.Length()).map_err(|_| err("targetUnsupported"))? != 1 {
            return Err(err("targetUnsupported"));
        }
        let selected = probe!(ranges.GetElement(0)).map_err(|_| err("targetUnsupported"))?;
        let prefix = probe!(doc.Clone()).map_err(|_| err("targetUnsupported"))?;
        probe!(prefix.MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            &selected,
            TextPatternRangeEndpoint_Start
        ))
        .map_err(|_| err("targetUnsupported"))?;
        let before = probe!(doc.GetText(32769))
            .map_err(|_| err("targetUnsupported"))?
            .to_string();
        let prefix = probe!(prefix.GetText(32769))
            .map_err(|_| err("targetUnsupported"))?
            .to_string();
        let selected = probe!(selected.GetText(32769))
            .map_err(|_| err("targetUnsupported"))?
            .to_string();
        if before.encode_utf16().count() > 32768
            || !before.starts_with(&prefix)
            || !before[prefix.len()..].starts_with(&selected)
        {
            return Err(err("targetUnsupported"));
        }
        Ok((before, prefix, selected))
    }
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
            .map_err(|_| err("targetUnsupported"))?;
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
        return Err(err("targetUnsupported"));
    }
    bool::try_from(value).map_err(|_| err("targetUnsupported"))
}
// ValuePattern is optional for contenteditable. Admit a TextPattern provider only when its
// exact boolean read-only attribute positively proves editability; mixed/unsupported is rejected.
fn uia_readonly(
    element: &IUIAutomationElement,
    pattern: &IUIAutomationTextPattern,
) -> Result<bool, Error> {
    unsafe {
        let available =
            probe!(element.GetCurrentPropertyValueEx(UIA_IsValuePatternAvailablePropertyId, true))
                .map_err(|_| err("targetUnsupported"))?;
        if exact_uia_bool(&available)? {
            let value = probe!(
                element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            )
            .map_err(|_| err("targetUnsupported"))?;
            return Ok(probe!(value.CurrentIsReadOnly())
                .map_err(|_| err("targetUnsupported"))?
                .as_bool());
        }
        let document = probe!(pattern.DocumentRange()).map_err(|_| err("targetUnsupported"))?;
        let attribute = probe!(document.GetAttributeValue(UIA_IsReadOnlyAttributeId))
            .map_err(|_| err("targetUnsupported"))?;
        exact_uia_bool(&attribute)
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
            let before = native_text(control)?;
            let (start, end) = native_selection(control)?;
            if expected_text(&before, start, end, "").is_none() {
                return Err(err("targetUnsupported"));
            }
            Proof::Edit { before, start, end }
        } else {
            let auto = automation
                .as_ref()
                .ok_or_else(|| err("targetUnsupported"))?;
            let element = probe!(auto.GetFocusedElement()).map_err(|_| err("targetUnsupported"))?;
            if uia_composing(&element)? {
                return Err(err("inputComposition"));
            }
            if probe!(element.CurrentProcessId()).map_err(|_| err("targetUnsupported"))?
                != a.process as i32
            {
                return Err(err("targetChanged"));
            }
            if probe!(element.CurrentIsPassword())
                .map_err(|_| err("targetUnsupported"))?
                .as_bool()
            {
                return Err(err("targetPassword"));
            }
            if !probe!(element.CurrentIsEnabled())
                .map_err(|_| err("targetUnsupported"))?
                .as_bool()
            {
                return Err(err("targetReadonly"));
            }
            let pattern =
                probe!(element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId))
                    .map_err(|_| err("targetUnsupported"))?;
            if uia_readonly(&element, &pattern)? {
                return Err(err("targetReadonly"));
            }
            let (before, prefix, selected) = uia_parts(&pattern)?;
            Proof::Uia {
                element,
                pattern,
                before,
                prefix,
                selected,
            }
        };
        let mut target = Target {
            anchor: a,
            proof,
            invalid: false,
            handler: None,
            automation: automation.clone(),
        };
        if let Proof::Uia { element, .. } = &target.proof {
            let handler: IUIAutomationEventHandler = Changes {
                generation: target.anchor.generation,
            }
            .into();
            target.handler = Some(handler.clone());
            let auto = automation.as_ref().unwrap();
            probe!(auto.AddAutomationEventHandler(
                UIA_Text_TextChangedEventId,
                element,
                TreeScope_Element,
                None,
                &handler
            ))
            .map_err(|_| err("targetUnsupported"))?;
            probe!(auto.AddAutomationEventHandler(
                UIA_Text_TextSelectionChangedEventId,
                element,
                TreeScope_Element,
                None,
                &handler
            ))
            .map_err(|_| err("targetUnsupported"))?;
        }
        budget()?;
        if !same(&target.anchor) {
            return Err(err("targetChanged"));
        }
        Ok(target)
    }
}
fn unchanged(t: &Target, automation: &Option<IUIAutomation>) -> bool {
    if t.invalid
        || !same(&t.anchor)
        || !permitted(&t.anchor)
        || composing(&t.anchor)
        || budget().is_err()
    {
        return false;
    }
    let valid = match &t.proof {
        Proof::Edit { before, start, end } => unsafe {
            let h = hwnd(t.anchor.focus);
            let style = GetWindowLongPtrW(h, GWL_STYLE) as u32;
            style & (0x20 | 0x800) == 0
                && IsWindowEnabled(h).as_bool()
                && native_text(h).is_ok_and(|v| v == *before)
                && native_selection(h).is_ok_and(|v| v == (*start, *end))
        },
        Proof::Uia {
            element,
            pattern,
            before,
            prefix,
            selected,
        } => (|| -> Result<bool, Error> {
            unsafe {
                if probe!(element.CurrentIsPassword())
                    .map_err(|_| err("targetUnsupported"))?
                    .as_bool()
                    || !probe!(element.CurrentIsEnabled())
                        .map_err(|_| err("targetUnsupported"))?
                        .as_bool()
                    || uia_composing(element)?
                {
                    return Ok(false);
                }
                if uia_readonly(element, pattern)? {
                    return Ok(false);
                }
                let auto = automation
                    .as_ref()
                    .ok_or_else(|| err("targetUnsupported"))?;
                let now = probe!(auto.GetFocusedElement()).map_err(|_| err("targetUnsupported"))?;
                if !probe!(auto.CompareElements(element, &now))
                    .map_err(|_| err("targetUnsupported"))?
                    .as_bool()
                {
                    return Ok(false);
                }
                Ok(uia_parts(pattern)? == (before.clone(), prefix.clone(), selected.clone()))
            }
        })()
        .unwrap_or(false),
    };
    valid && budget().is_ok() && same(&t.anchor)
}
fn expected(t: &Target, text: &str) -> Option<String> {
    match &t.proof {
        Proof::Edit { before, start, end } => expected_text(before, *start, *end, text),
        Proof::Uia {
            before,
            prefix,
            selected,
            ..
        } => Some(normalize(&format!(
            "{prefix}{text}{}",
            &before[prefix.len() + selected.len()..]
        ))),
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
fn selection_confirmed(t: &Target, text: &str) -> bool {
    match &t.proof {
        Proof::Edit { before, start, .. } => {
            native_selection(hwnd(t.anchor.focus)).is_ok_and(|(a, b)| {
                a == b
                    && native_text(hwnd(t.anchor.focus)).is_ok_and(|actual| {
                        insertion_prefix(&actual, a, "").is_some_and(|prefix| {
                            Some(prefix) == insertion_prefix(before, *start, text)
                        })
                    })
            })
        }
        Proof::Uia {
            pattern, prefix, ..
        } => uia_parts(pattern).is_ok_and(|(_, now, selected)| {
            selected.is_empty() && normalize(&now) == normalize(&format!("{prefix}{text}"))
        }),
    }
}
fn read_text(t: &Target) -> Result<String, Error> {
    match &t.proof {
        Proof::Edit { .. } => native_text(hwnd(t.anchor.focus)).map(|s| normalize(&s)),
        Proof::Uia { pattern, .. } => unsafe {
            let doc = probe!(pattern.DocumentRange()).map_err(|_| err("targetUnsupported"))?;
            probe!(doc.GetText(32769))
                .map(|s| normalize(&s.to_string()))
                .map_err(|_| err("targetUnsupported"))
        },
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
fn paste(t: &Target) -> Result<(), Error> {
    unsafe {
        match &t.proof {
            Proof::Edit { .. } => {
                message(hwnd(t.anchor.focus), WM_PASTE, WPARAM(0), LPARAM(0))?;
                Ok(())
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
                if count != inputs.len() as u32 {
                    if count > 0 {
                        let release = if count == 2 {
                            vec![inputs[2], inputs[3]]
                        } else {
                            vec![inputs[3]]
                        };
                        let _ = SendInput(&release, size_of::<INPUT>() as i32);
                    }
                    Err(err("inputUnconfirmed"))
                } else {
                    Ok(())
                }
            }
        }
    }
}
fn modifiers_down() -> bool {
    unsafe {
        [VK_SHIFT, VK_CONTROL, VK_MENU, VK_LWIN, VK_RWIN]
            .iter()
            .any(|k| GetAsyncKeyState(k.0 as i32) < 0)
    }
}
type PasteOutcome = Result<(Result<bool, Error>, Result<(), Error>, bool), Error>;
fn deliver(
    t: &Target,
    text: &str,
    cancel: &DeliveryGate,
    auto: &Option<IUIAutomation>,
    owner: usize,
) -> Delivery {
    let began = Instant::now();
    let mut result = Delivery::copy("targetChanged");
    let work = || -> PasteOutcome {
        let wait = Instant::now();
        while modifiers_down() {
            if cancel.cancelled() {
                return Err(err("cancelled"));
            }
            if wait.elapsed() > Duration::from_millis(500) {
                return Err(err("keysHeld"));
            }
            thread::sleep(Duration::from_millis(10));
        }
        deadline(500);
        if cancel.cancelled() || !unchanged(t, auto) {
            return Err(err("targetChanged"));
        }
        let expected = expected(t, text).ok_or_else(|| err("targetUnsupported"))?;
        let marker = uuid::Uuid::new_v4().as_bytes().to_vec();
        let (saved, before_sequence) = protected_backup(owner)?;
        deadline(500);
        let saved = {
            let _lock = open(owner)?;
            if unsafe { GetClipboardSequenceNumber() } != before_sequence {
                return Err(err("clipboardChanged"));
            }
            if cancel.cancelled() || !unchanged(t, auto) {
                return Err(err("targetChanged"));
            }
            match write_text(text, &marker) {
                Ok(_) => {}
                Err(e) => {
                    if install(&saved).is_err() {
                        let _ = put(format("Sayweave.Delivery"), &marker);
                        RECOVERY.with(|r| {
                            drop(_lock);
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
            saved
        };
        // Closing the clipboard publishes the write and may advance its sequence.
        let sequence = unsafe { GetClipboardSequenceNumber() };
        // From here any attempted input is uncertain until readback; never retry a second strategy.
        let mut submitted = false;
        let applied = (|| {
            if cancel.cancelled() || !unchanged(t, auto) {
                return Err(err("targetChanged"));
            }
            if !{
                let _lock = open(owner)?;
                owned(sequence, &marker)
            } {
                return Err(err("clipboardChanged"));
            }
            // Clipboard access may have waited for another application; recheck the bound target now.
            if !unchanged(t, auto) {
                return Err(err("targetChanged"));
            }
            if unsafe { GetClipboardSequenceNumber() } != sequence {
                return Err(err("clipboardChanged"));
            }
            if !cancel.commit() {
                return Err(err("cancelled"));
            }
            submitted = true;
            paste(t)?;
            deadline(1000);
            let at = Instant::now();
            loop {
                if read_text(t).is_ok_and(|s| s == expected) && selection_confirmed(t, text) {
                    return Ok(true);
                }
                if at.elapsed() >= Duration::from_secs(1) {
                    return Ok(false);
                }
                thread::sleep(Duration::from_millis(20));
            }
        })();
        let restored = restore(owner, sequence, &marker, &saved, false);
        Ok((applied, restored, submitted))
    };
    // Keep state separate so clipboard-restoration failure cannot turn a confirmed input into a retry.
    match work() {
        Ok((Ok(true), restored, _)) => {
            result.state = "confirmed".into();
            result.error = None;
            result.restore_error = restored.is_err();
        }
        Ok((applied, restored, submitted)) => {
            result.state = if submitted { "unconfirmed" } else { "copy" }.into();
            result.error = Some(
                applied
                    .err()
                    .map(|e| e.code)
                    .unwrap_or_else(|| "inputUnconfirmed".into()),
            );
            result.restore_error = restored.is_err();
        }
        Err(e) => {
            result.restore_error = e.code == "clipboardRestore";
            result.error = Some(e.code);
        }
    }
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
                    let _ = r.send(Delivery::copy("stopping"));
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
                        targets.insert(id, t);
                        Ok(())
                    }
                });
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
                    .unwrap_or_else(|| Delivery::copy("targetUnavailable"));
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
