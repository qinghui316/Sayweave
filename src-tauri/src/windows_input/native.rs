use super::*;
use std::{
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
            Input::{Ime::*, KeyboardAndMouse::IsWindowEnabled},
            WindowsAndMessaging::*,
        },
    },
    core::Interface,
};

fn err(code: &str) -> Error {
    Error::new(code)
}
fn hwnd(value: usize) -> HWND {
    HWND(value as *mut _)
}
static CHANGES: AtomicU64 = AtomicU64::new(0);
static CAPTURE_ID: AtomicU64 = AtomicU64::new(0);
struct TargetWatch {
    anchor: Anchor,
    task: Option<String>,
    overlay: Option<crate::voice_window::TaskWindow>,
    identity_changed: bool,
}
static WATCH: Mutex<Option<TargetWatch>> = Mutex::new(None);
pub(super) fn start_anchor() -> Result<Anchor, Error> {
    let mut a = anchor()?;
    a.capture_id = CAPTURE_ID.fetch_add(1, Ordering::AcqRel) + 1;
    if let Ok(mut watch) = WATCH.lock() {
        // A second toggle/queued key must not replace the active task's event evidence.
        if !watch.as_ref().is_some_and(|w| w.task.is_some()) {
            *watch = Some(TargetWatch {
                anchor: a.clone(),
                task: None,
                overlay: None,
                identity_changed: false,
            });
        }
    }
    Ok(a)
}
pub(super) fn bind_watch(
    id: &str,
    anchor: &Anchor,
    overlay: Option<crate::voice_window::TaskWindow>,
) -> Result<(), Error> {
    let mut guard = WATCH.lock().map_err(|_| err("targetProbeFailed"))?;
    let watch = guard.as_mut().ok_or_else(|| err("targetChanged"))?;
    if watch.anchor.capture_id != anchor.capture_id
        || watch.task.as_deref().is_some_and(|t| t != id)
    {
        return Err(err("targetChanged"));
    }
    if watch.task.is_none() {
        watch.task = Some(id.into());
        watch.overlay = overlay;
    }
    Ok(())
}
pub(super) fn release_watch(id: &str) {
    if let Ok(mut watch) = WATCH.lock()
        && watch
            .as_ref()
            .is_some_and(|w| w.task.as_deref() == Some(id))
    {
        *watch = None;
    }
}
/// Composition owns its range in the target apartment. Its own text/selection notifications
/// must not invalidate that range; external foreground changes remain sticky for the task.
pub(crate) fn composition_target(id: &str, anchor: &Anchor) -> Result<bool, Error> {
    let guard = WATCH.lock().map_err(|_| err("targetProbeFailed"))?;
    let watch = guard
        .as_ref()
        .filter(|w| w.task.as_deref() == Some(id) && w.anchor.capture_id == anchor.capture_id)
        .ok_or_else(|| err("targetChanged"))?;
    if watch.identity_changed
        || lifetime(anchor.process)? != anchor.lifetime
        || !unsafe { IsWindow(Some(hwnd(anchor.window))) }.as_bool()
    {
        return Err(err("targetChanged"));
    }
    let foreground = unsafe { GetForegroundWindow().0 as usize };
    if foreground == anchor.window {
        return Ok(true);
    }
    if watch
        .overlay
        .as_ref()
        .is_some_and(|o| crate::voice_window::contains(o, foreground, unsafe { GetTickCount() }))
    {
        return Ok(false); // Own interaction is advisory, but never submit while it has focus.
    }
    Err(err("targetChanged"))
}
pub(super) fn composition_begin_target(id: &str, expected: &Anchor) -> Result<(), Error> {
    if !composition_target(id, expected)? {
        return Err(err("targetChanged"));
    }
    let current = anchor()?;
    if current.focus != expected.focus {
        log::info!(target:"sayweave", "operation=draft_begin_guard task_id={id} reason=targetChanged focus_same=false");
        return Err(err("targetChanged"));
    }
    if current.activity != expected.activity {
        log::info!(target:"sayweave", "operation=draft_begin_guard task_id={id} reason=editing focus_same={} generation_same={}", current.focus == expected.focus, current.generation == expected.generation);
        return Err(err("editing"));
    }
    if current.generation != expected.generation {
        log::info!(target:"sayweave", "operation=draft_begin_guard task_id={id} reason=targetChanged focus_same={} generation_same={}", current.focus == expected.focus, current.generation == expected.generation);
        return Err(err("targetChanged"));
    }
    Ok(())
}
/// Acquire a new edit position in the original session window after handing off the
/// previous range. Never clear sticky foreground/identity evidence or its capture time.
pub(crate) fn reanchor_turn(id: &str, original: &Anchor) -> Result<Anchor, Error> {
    if !composition_target(id, original)? {
        return Err(err("targetChanged"));
    }
    let mut next = anchor()?;
    if next.window != original.window
        || next.process != original.process
        || next.lifetime != original.lifetime
    {
        return Err(err("targetChanged"));
    }
    next.capture_id = original.capture_id;
    next.captured_tick = original.captured_tick;
    let mut state = WATCH.lock().map_err(|_| err("targetProbeFailed"))?;
    let watch = state
        .as_mut()
        .filter(|w| {
            w.task.as_deref() == Some(id)
                && w.anchor.capture_id == original.capture_id
                && !w.identity_changed
        })
        .ok_or_else(|| err("targetChanged"))?;
    watch.anchor = next.clone();
    Ok(next)
}
/// Physical pointer evidence shares the same task-window lease as WinEvent evidence.
/// WebView descendants can live in another process; process identity is not ownership.
pub(super) fn observe_pointer(window: usize, event_tick: u32) {
    let watch = WATCH.lock().ok();
    let active = watch.as_ref().and_then(|w| w.as_ref());
    let own = active
        .and_then(|w| w.overlay.as_ref())
        .is_some_and(|overlay| crate::voice_window::contains(overlay, window, event_tick));
    if let Some(w) = active {
        log::info!(target:"sayweave", "operation=input_pointer task_id={} task_overlay={own} activity_changed={}", w.task.as_deref().unwrap_or("pending"), !own);
    }
    if !own {
        crate::input_state::edited();
    }
}
#[derive(Debug, PartialEq, Eq)]
enum EventDecision {
    Unrelated,
    Original,
    OwnOverlay,
    Changed(&'static str),
}
struct EventEvidence {
    window: usize,
    object: i32,
    root: usize,
    foreground: usize,
    own_event: bool,
    own_foreground: bool,
}
fn event_decision(event: u32, top: usize, focus: usize, e: &EventEvidence) -> EventDecision {
    if event == EVENT_SYSTEM_FOREGROUND {
        if e.window != top && !e.own_event {
            return EventDecision::Changed("externalForeground");
        }
        if e.foreground != top && !e.own_foreground {
            return EventDecision::Changed("currentForeground");
        }
        return if e.own_event {
            EventDecision::OwnOverlay
        } else {
            EventDecision::Original
        };
    }
    if event == EVENT_OBJECT_DESTROY && (e.window == top || e.window == focus) {
        // WinEvent identifies an accessible object, not just its hosting HWND. RichEdit
        // recreates OBJID_CARET during normal TSF updates while the editor stays alive.
        return match e.object {
            value if value == OBJID_WINDOW.0 => EventDecision::Changed("targetDestroyed"),
            value if value == OBJID_CARET.0 => EventDecision::Unrelated,
            _ => EventDecision::Changed("targetObjectDestroyed"),
        };
    }
    if event == EVENT_OBJECT_FOCUS && e.window != focus && e.root == top {
        return EventDecision::Changed("controlFocus");
    }
    if [EVENT_OBJECT_VALUECHANGE, EVENT_OBJECT_TEXTSELECTIONCHANGED].contains(&event)
        && (e.window == focus || e.window == top)
    {
        return EventDecision::Changed("textOrSelection");
    }
    EventDecision::Unrelated
}
pub(super) fn observe(event: u32, window: usize, object: i32, event_tick: u32) {
    if let Ok(mut watch) = WATCH.lock()
        && let Some(watch) = watch.as_mut()
        && event_after_capture(event_tick, watch.anchor.captured_tick)
    {
        let received_tick = unsafe { GetTickCount() };
        let foreground = unsafe { GetForegroundWindow().0 as usize };
        let evidence = EventEvidence {
            window,
            object,
            root: unsafe { GetAncestor(hwnd(window), GA_ROOT).0 as usize },
            foreground,
            own_event: watch
                .overlay
                .as_ref()
                .is_some_and(|o| crate::voice_window::contains(o, window, event_tick)),
            own_foreground: watch
                .overlay
                .as_ref()
                .is_some_and(|o| crate::voice_window::contains(o, foreground, received_tick)),
        };
        let decision = event_decision(event, watch.anchor.window, watch.anchor.focus, &evidence);
        watch.identity_changed |= matches!(
            decision,
            EventDecision::Changed("externalForeground" | "currentForeground" | "targetDestroyed")
        );
        if matches!(decision, EventDecision::Changed(_)) {
            CHANGES.fetch_add(1, Ordering::AcqRel);
        }
        if decision != EventDecision::Unrelated {
            log::info!(target:"sayweave", "operation=input_target_event task_id={} event={event} object={object} event_tick={event_tick} received_tick={received_tick} own_overlay={} foreground_original={} foreground_overlay={} decision={decision:?} generation={}",watch.task.as_deref().unwrap_or("pending"),evidence.own_event,foreground==watch.anchor.window,evidence.own_foreground,CHANGES.load(Ordering::Acquire));
        }
    }
}
#[cfg(test)]
mod event_tests {
    use super::*;
    #[test]
    fn caret_geometry_is_advisory_but_text_and_selection_invalidate() {
        let e = EventEvidence {
            window: 12,
            object: OBJID_CLIENT.0,
            root: 10,
            foreground: 10,
            own_event: false,
            own_foreground: false,
        };
        assert_eq!(
            event_decision(EVENT_OBJECT_LOCATIONCHANGE, 10, 12, &e),
            EventDecision::Unrelated
        );
        for event in [
            EVENT_OBJECT_TEXTSELECTIONCHANGED,
            EVENT_OBJECT_VALUECHANGE,
            EVENT_OBJECT_DESTROY,
        ] {
            assert!(matches!(
                event_decision(event, 10, 12, &e),
                EventDecision::Changed(_)
            ));
        }
    }
    #[test]
    fn accessible_caret_destruction_is_not_editor_destruction() {
        let mut e = EventEvidence {
            window: 12,
            object: OBJID_CARET.0,
            root: 10,
            foreground: 10,
            own_event: false,
            own_foreground: false,
        };
        assert_eq!(
            event_decision(EVENT_OBJECT_DESTROY, 10, 12, &e),
            EventDecision::Unrelated
        );
        e.object = OBJID_CLIENT.0;
        assert_eq!(
            event_decision(EVENT_OBJECT_DESTROY, 10, 12, &e),
            EventDecision::Changed("targetObjectDestroyed")
        );
        e.object = OBJID_WINDOW.0;
        assert_eq!(
            event_decision(EVENT_OBJECT_DESTROY, 10, 12, &e),
            EventDecision::Changed("targetDestroyed")
        );
    }
    #[test]
    fn only_task_overlay_is_advisory_and_external_switches_remain_sticky() {
        let mut e = EventEvidence {
            window: 14,
            object: OBJID_WINDOW.0,
            root: 14,
            foreground: 10,
            own_event: true,
            own_foreground: false,
        };
        assert_eq!(
            event_decision(EVENT_SYSTEM_FOREGROUND, 10, 12, &e),
            EventDecision::OwnOverlay
        );
        e.foreground = 14;
        e.own_foreground = true;
        assert_eq!(
            event_decision(EVENT_SYSTEM_FOREGROUND, 10, 12, &e),
            EventDecision::OwnOverlay
        );
        e.foreground = 15;
        e.own_foreground = false;
        assert_eq!(
            event_decision(EVENT_SYSTEM_FOREGROUND, 10, 12, &e),
            EventDecision::Changed("currentForeground")
        );
        e.foreground = 10;
        e.own_event = false;
        // Another application, main window, old lease or unknown window remains a change even
        // when the asynchronously delivered event arrives after the original regained foreground.
        assert_eq!(
            event_decision(EVENT_SYSTEM_FOREGROUND, 10, 12, &e),
            EventDecision::Changed("externalForeground")
        );
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
            capture_id: 0,
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
            let task = WATCH.lock().ok().and_then(|w| w.as_ref().filter(|w| w.anchor.capture_id == a.capture_id).and_then(|w| w.task.clone())).unwrap_or_else(|| "pending".into());
            log::info!(target:"sayweave", "operation=input_target_identity task_id={task} window_same={} focus_same={} process_same={} lifetime_same={} generation_same={}", now.window == a.window, now.focus == a.focus, now.process == a.process, now.lifetime == a.lifetime, now.generation == a.generation);
        }
        unchanged
    })
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
// Probe only target nature. TSF owns the selection/range and performs write-time checks.
// A probe failure never releases the task's shared WATCH.
fn capture(a: Anchor, automation: &Option<IUIAutomation>) -> Result<(), Error> {
    unsafe {
        if !same(&a) {
            return Err(err("targetChanged"));
        }
        if !permitted(&a) {
            return Err(err("targetPermission"));
        }
        if composing(&a) {
            log::info!(target:"sayweave", "operation=input_composition_guard capture_id={} source=imm", a.capture_id);
            return Err(err("inputComposition"));
        }
        let control = hwnd(a.focus);
        let mut class = [0u16; 128];
        let count = GetClassNameW(control, &mut class) as usize;
        if String::from_utf16_lossy(&class[..count]) == "Edit" {
            let style = GetWindowLongPtrW(control, GWL_STYLE) as u32;
            if style & 0x20 != 0 {
                return Err(err("targetPassword"));
            }
            if style & 0x800 != 0 || !IsWindowEnabled(control).as_bool() {
                return Err(err("targetReadonly"));
            }
        } else {
            let auto = automation
                .as_ref()
                .ok_or_else(|| err("targetProbeFailed"))?;
            let element = probe!(auto.GetFocusedElement()).map_err(|_| err("targetProbeFailed"))?;
            if probe!(element.CurrentProcessId()).map_err(|_| err("targetProbeFailed"))?
                != a.process as i32
            {
                return Err(err("targetChanged"));
            }
            if probe!(element.CurrentIsPassword())
                .map_err(|_| err("targetSafetyUnknown"))?
                .as_bool()
            {
                return Err(err("targetPassword"));
            }
            if !probe!(element.CurrentIsEnabled())
                .map_err(|_| err("targetSafetyUnknown"))?
                .as_bool()
            {
                return Err(err("targetReadonly"));
            }
            // The TSF Begin edit session checks compositions in the actual input context.
            // UIA range availability is not a second composition admission policy.
            let pattern =
                probe!(element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId))
                    .ok();
            if uia_readonly(&element, pattern.as_ref())? {
                return Err(err("targetReadonly"));
            }
        }
        budget()?;
        if !same(&a) {
            return Err(err("targetChanged"));
        }
        Ok(())
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
struct Allocation {
    format: u32,
    handle: Option<HGLOBAL>,
}
impl Allocation {
    fn new(format: u32, bytes: &[u8]) -> Result<Self, Error> {
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
pub(super) fn install(saved: &[(u32, Vec<u8>)]) -> Result<(), Error> {
    let mut prepared = saved
        .iter()
        .map(|(kind, bytes)| Allocation::new(*kind, bytes))
        .collect::<Result<Vec<_>, _>>()?;
    unsafe {
        EmptyClipboard().map_err(|_| err("copyFailed"))?;
    }
    let mut failed = false;
    for item in &mut prepared {
        failed |= item.put().is_err();
    }
    if failed {
        Err(err("copyFailed"))
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
    while let Ok(envelope) = receiver.recv() {
        let request = envelope.request;
        if matches!(request, Request::Stop) {
            break;
        }
        busy.store(true, Ordering::Release);
        if stopping.load(Ordering::Acquire) {
            match request {
                Request::Capture(_, _, r) | Request::Copy(_, _, r) => {
                    let _ = r.send(Err(err("stopping")));
                }
                _ => {}
            }
            busy.store(false, Ordering::Release);
            // Stop may not fit in the bounded queue. Shutdown is authoritative;
            // dropping the receiver also releases all queued request guards.
            break;
        }
        match request {
            Request::Capture(id, anchor, response) => {
                deadline(500);
                let at = Instant::now();
                let result =
                    bind_watch(&id, &anchor, None).and_then(|()| capture(anchor, &automation));
                log::info!(target:"sayweave", "operation=input_capture task_id={} stage=capture nature_checked={} error={} elapsed_ms={}", id, result.is_ok(), result.as_ref().err().map(|e| e.code.as_str()).unwrap_or("none"), at.elapsed().as_millis());
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
                release_watch(&id);
            }
            Request::Stop => break,
        }
        busy.store(false, Ordering::Release);
    }
    busy.store(false, Ordering::Release);
    if let Ok(mut watch) = WATCH.lock() {
        *watch = None;
    }
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
    fn shutdown_exits_when_the_stop_message_does_not_fit() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let pending = Arc::new(AtomicUsize::new(1));
        sender
            .send(super::super::Envelope {
                request: Request::Forget("shutdown-test".into()),
                _pending: super::super::Pending(pending.clone()),
            })
            .unwrap();
        pending.fetch_add(1, Ordering::AcqRel);
        assert!(
            sender
                .try_send(super::super::Envelope {
                    request: Request::Stop,
                    _pending: super::super::Pending(pending.clone()),
                })
                .is_err()
        );
        let (done, finished) = mpsc::channel();
        let thread = std::thread::spawn(move || {
            worker(
                receiver,
                Arc::new(AtomicUsize::new(0)),
                Arc::new(AtomicBool::new(true)),
                Arc::new(AtomicBool::new(false)),
            );
            let _ = done.send(());
        });
        // Keep sender alive: completion must not depend on channel disconnection.
        assert!(finished.recv_timeout(Duration::from_secs(3)).is_ok());
        thread.join().unwrap();
        assert_eq!(pending.load(Ordering::Acquire), 0);
        drop(sender);
    }
    #[test]
    fn only_exact_uia_booleans_prove_pattern_availability_or_editability() {
        assert!(!exact_uia_bool(&VARIANT::from(false)).unwrap());
        assert!(exact_uia_bool(&VARIANT::from(true)).unwrap());
        assert!(exact_uia_bool(&VARIANT::from(0i32)).is_err());
        assert!(exact_uia_bool(&VARIANT::default()).is_err());
    }
}
