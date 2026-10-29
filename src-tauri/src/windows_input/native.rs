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
pub(super) fn start_anchor() -> Result<Option<Anchor>, Error> {
    let Some(mut a) = read_anchor()? else {
        return Ok(None);
    };
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
    Ok(Some(a))
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
        || !editor_matches(anchor)?
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
    if current.editor != expected.editor {
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
        || next.editor != original.editor
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
/// Apartment-owned identity fence for a refused initial Begin. Ordinary input
/// does not depend on this optional capability. It never reads or writes text.
pub(super) struct PendingFocus {
    automation: IUIAutomation,
    element: IUIAutomationElement,
}
impl PendingFocus {
    pub(super) fn capture(anchor: &Anchor) -> Result<Self, Error> {
        unsafe {
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
                    .map_err(|_| err("targetProbeFailed"))?;
            if let Ok(options) = automation.cast::<IUIAutomation2>() {
                let _ = options.SetConnectionTimeout(50);
                let _ = options.SetTransactionTimeout(50);
            }
            let element = automation
                .GetFocusedElement()
                .map_err(|_| err("targetProbeFailed"))?;
            if element
                .CurrentProcessId()
                .map_err(|_| err("targetProbeFailed"))?
                != anchor.editor.process as i32
            {
                return Err(err("targetChanged"));
            }
            // Retain the original object before sending Begin. Safety and
            // current-caret editability are checked only if a retry is needed;
            // ordinary successful native admission does not depend on UIA.
            Ok(Self {
                automation,
                element,
            })
        }
    }
    pub(super) fn check(&self, anchor: &Anchor) -> Result<(), Error> {
        deadline(350);
        unsafe {
            let focused = self
                .automation
                .GetFocusedElement()
                .map_err(|_| err("targetChanged"))?;
            if !self
                .automation
                .CompareElements(&focused, &self.element)
                .map_err(|_| err("targetChanged"))?
                .as_bool()
                || self
                    .element
                    .CurrentProcessId()
                    .map_err(|_| err("targetChanged"))?
                    != anchor.editor.process as i32
            {
                return Err(err("targetChanged"));
            }
            if self
                .element
                .CurrentIsPassword()
                .map_err(|_| err("targetSafetyUnknown"))?
                .as_bool()
            {
                return Err(err("targetPassword"));
            }
            if !self
                .element
                .CurrentIsEnabled()
                .map_err(|_| err("targetSafetyUnknown"))?
                .as_bool()
            {
                return Err(err("targetReadonly"));
            }
            if !self
                .element
                .CurrentHasKeyboardFocus()
                .map_err(|_| err("targetEditabilityUnknown"))?
                .as_bool()
            {
                return Err(err("targetChanged"));
            }
            // Rich edit fields may have Document role. Require positive
            // writable-caret evidence instead of a particular UIA role.
            let pattern = self
                .element
                .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                .ok();
            if uia_readonly(&self.element, pattern.as_ref())? {
                return Err(err("targetReadonly"));
            }
            budget()
        }
    }
}
/// Physical pointer evidence shares the same task-window lease as WinEvent evidence.
/// WebView descendants can live in another process; process identity is not ownership.
pub(super) fn observe_pointer(window: usize, event_tick: u32) -> bool {
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
    own
}
#[derive(Debug, PartialEq, Eq)]
enum EventDecision {
    Unrelated,
    Original,
    OwnOverlay,
    NonActivatingNotice,
    Changed(&'static str),
}
struct EventEvidence {
    window: usize,
    object: i32,
    root: usize,
    foreground: usize,
    own_event: bool,
    own_foreground: bool,
    nonactivating_notice: bool,
}
fn event_decision(event: u32, top: usize, focus: usize, e: &EventEvidence) -> EventDecision {
    if event == EVENT_SYSTEM_FOREGROUND {
        if e.nonactivating_notice && e.foreground == top {
            return EventDecision::NonActivatingNotice;
        }
        if e.window != top && e.root != top && !e.own_event {
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
        let event_root = unsafe { GetAncestor(hwnd(window), GA_ROOT).0 as usize };
        let nonactivating_notice = event == EVENT_SYSTEM_FOREGROUND
            && foreground == watch.anchor.window
            && event_root != 0
            && event_root != watch.anchor.window
            && unsafe { IsWindow(Some(hwnd(event_root))) }.as_bool()
            && unsafe { GetWindowLongPtrW(hwnd(event_root), GWL_EXSTYLE) } as u32
                & (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0)
                == (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0);
        let evidence = EventEvidence {
            window,
            object,
            root: event_root,
            foreground,
            own_event: watch
                .overlay
                .as_ref()
                .is_some_and(|o| crate::voice_window::contains(o, window, event_tick)),
            own_foreground: watch
                .overlay
                .as_ref()
                .is_some_and(|o| crate::voice_window::contains(o, foreground, received_tick)),
            nonactivating_notice,
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
            let (mut event_process, mut root_process) = (0, 0);
            unsafe {
                GetWindowThreadProcessId(hwnd(window), Some(&mut event_process));
                GetWindowThreadProcessId(hwnd(evidence.root), Some(&mut root_process));
            }
            log::info!(target:"sayweave", "operation=input_target_event task_id={} event={event} object={object} event_tick={event_tick} received_tick={received_tick} event_window={window} event_root={} event_process={event_process} root_process={root_process} target_window={} target_process={} current_foreground={foreground} root_original={} own_overlay={} foreground_original={} foreground_overlay={} decision={decision:?} generation={}",watch.task.as_deref().unwrap_or("pending"),evidence.root,watch.anchor.window,watch.anchor.process,evidence.root==watch.anchor.window,evidence.own_event,foreground==watch.anchor.window,evidence.own_foreground,CHANGES.load(Ordering::Acquire));
        }
    }
}
#[cfg(test)]
mod event_tests {
    use super::*;
    #[test]
    fn nonactivating_notice_does_not_invalidate_original_foreground() {
        let mut evidence = EventEvidence {
            window: 15,
            object: OBJID_WINDOW.0,
            root: 15,
            foreground: 10,
            own_event: false,
            own_foreground: false,
            nonactivating_notice: true,
        };
        assert_eq!(
            event_decision(EVENT_SYSTEM_FOREGROUND, 10, 12, &evidence),
            EventDecision::NonActivatingNotice
        );
        // Style does not grant permission when the helper really owns foreground.
        evidence.foreground = 15;
        assert!(matches!(
            event_decision(EVENT_SYSTEM_FOREGROUND, 10, 12, &evidence),
            EventDecision::Changed(_)
        ));
        // An ordinary external window's delayed event remains invalidating.
        evidence.foreground = 10;
        evidence.nonactivating_notice = false;
        assert_eq!(
            event_decision(EVENT_SYSTEM_FOREGROUND, 10, 12, &evidence),
            EventDecision::Changed("externalForeground")
        );
    }
    #[test]
    fn foreground_notification_from_original_child_is_not_an_external_switch() {
        let mut evidence = EventEvidence {
            window: 12,
            object: OBJID_WINDOW.0,
            root: 10,
            foreground: 10,
            own_event: false,
            own_foreground: false,
            nonactivating_notice: false,
        };
        assert_eq!(
            event_decision(EVENT_SYSTEM_FOREGROUND, 10, 12, &evidence),
            EventDecision::Original
        );
        // Same current foreground cannot erase a genuine earlier other-root event.
        evidence.root = 15;
        assert_eq!(
            event_decision(EVENT_SYSTEM_FOREGROUND, 10, 12, &evidence),
            EventDecision::Changed("externalForeground")
        );
        evidence.root = 0;
        assert_eq!(
            event_decision(EVENT_SYSTEM_FOREGROUND, 10, 12, &evidence),
            EventDecision::Changed("externalForeground")
        );
    }
    #[test]
    fn caret_geometry_is_advisory_but_text_and_selection_invalidate() {
        let e = EventEvidence {
            window: 12,
            object: OBJID_CLIENT.0,
            root: 10,
            foreground: 10,
            own_event: false,
            own_foreground: false,
            nonactivating_notice: false,
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
            nonactivating_notice: false,
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
            nonactivating_notice: false,
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
    read_anchor()?.ok_or_else(|| err("targetUnavailable"))
}
fn read_anchor() -> Result<Option<Anchor>, Error> {
    unsafe {
        let captured_tick = GetTickCount();
        let window = GetForegroundWindow();
        if window.0.is_null() {
            return Ok(None);
        }
        let mut process = 0;
        let tid = GetWindowThreadProcessId(window, Some(&mut process));
        if process == std::process::id() {
            return Ok(None);
        }
        let mut gui = GUITHREADINFO {
            cbSize: size_of::<GUITHREADINFO>() as u32,
            ..Default::default()
        };
        GetGUIThreadInfo(tid, &mut gui).map_err(|_| err("targetProbeFailed"))?;
        if gui.hwndFocus.0.is_null() {
            return Ok(None);
        }
        let mut editor_process = 0;
        let editor_thread = GetWindowThreadProcessId(gui.hwndFocus, Some(&mut editor_process));
        if editor_thread == 0
            || editor_process == 0
            || GetAncestor(gui.hwndFocus, GA_ROOT) != window
        {
            return Err(err("targetChanged"));
        }
        Ok(Some(Anchor {
            capture_id: 0,
            window: window.0 as usize,
            focus: gui.hwndFocus.0 as usize,
            process,
            lifetime: lifetime(process)?,
            editor: EditorEndpoint {
                window: gui.hwndFocus.0 as usize,
                process: editor_process,
                thread: editor_thread,
                lifetime: lifetime(editor_process)?,
            },
            generation: CHANGES.load(Ordering::Acquire),
            captured_tick,
            activity: crate::input_state::activity(),
        }))
    }
}
fn same(a: &Anchor) -> bool {
    anchor().is_ok_and(|now| {
        let unchanged = now.window == a.window
            && now.focus == a.focus
            && now.process == a.process
            && now.lifetime == a.lifetime
            && now.editor == a.editor
            && now.generation == a.generation;
        if !unchanged {
            let task = WATCH.lock().ok().and_then(|w| w.as_ref().filter(|w| w.anchor.capture_id == a.capture_id).and_then(|w| w.task.clone())).unwrap_or_else(|| "pending".into());
            log::info!(target:"sayweave", "operation=input_target_identity task_id={task} window_same={} focus_same={} process_same={} lifetime_same={} generation_same={}", now.window == a.window, now.focus == a.focus, now.process == a.process, now.lifetime == a.lifetime, now.generation == a.generation);
        }
        unchanged
    })
}
fn editor_matches(anchor: &Anchor) -> Result<bool, Error> {
    unsafe {
        let editor = &anchor.editor;
        if !IsWindow(Some(hwnd(editor.window))).as_bool()
            || GetAncestor(hwnd(editor.window), GA_ROOT).0 as usize != anchor.window
            || lifetime(editor.process)? != editor.lifetime
        {
            return Ok(false);
        }
        let mut process = 0;
        let thread = GetWindowThreadProcessId(hwnd(editor.window), Some(&mut process));
        Ok(process == editor.process && thread == editor.thread)
    }
}
pub(super) fn editor_endpoint(anchor: &Anchor) -> Result<(u32, u32), Error> {
    if lifetime(anchor.process)? != anchor.lifetime || !editor_matches(anchor)? {
        return Err(err("targetChanged"));
    }
    let mut process = 0;
    unsafe {
        GetWindowThreadProcessId(hwnd(anchor.window), Some(&mut process));
    }
    if process != anchor.process {
        return Err(err("targetChanged"));
    }
    Ok((anchor.editor.process, anchor.editor.thread))
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
        .and_then(|root| {
            integrity(a.editor.process).and_then(|editor| {
                integrity(std::process::id()).map(|own| root <= own && editor <= own)
            })
        })
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
/// Read-only observation has no dependency on the task's writable WATCH.
pub(super) fn learning_target(a: &Anchor) -> Result<(), Error> {
    let current = read_anchor()?.ok_or_else(|| err("targetChanged"))?;
    if current.window != a.window
        || current.process != a.process
        || current.lifetime != a.lifetime
        || current.editor != a.editor
        || !permitted(a)
    {
        return Err(err("targetChanged"));
    }
    editor_endpoint(a).map(|_| ())
}
#[cfg(test)]
pub(super) fn learning_composition(a: &Anchor) -> Option<bool> {
    unsafe {
        let context = ImmGetContext(hwnd(a.focus));
        if context.0.is_null() {
            return None;
        }
        let count = ImmGetCompositionStringW(context, GCS_COMPSTR, None, 0);
        let _ = ImmReleaseContext(hwnd(a.focus), context);
        (count >= 0).then_some(count > 0)
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
fn non_editing_control(element: &IUIAutomationElement) -> Result<bool, Error> {
    unsafe {
        let kind = probe!(element.CurrentControlType()).map_err(|_| err("targetProbeFailed"))?;
        if ![
            UIA_ButtonControlTypeId,
            UIA_HyperlinkControlTypeId,
            UIA_ImageControlTypeId,
            UIA_ListItemControlTypeId,
            UIA_ListControlTypeId,
            UIA_MenuItemControlTypeId,
            UIA_TabItemControlTypeId,
            UIA_TextControlTypeId,
            UIA_TreeItemControlTypeId,
            UIA_ToolBarControlTypeId,
            UIA_StatusBarControlTypeId,
            UIA_TitleBarControlTypeId,
            UIA_SeparatorControlTypeId,
        ]
        .contains(&kind)
        {
            return Ok(false);
        }
        for property in [
            UIA_IsTextPatternAvailablePropertyId,
            UIA_IsValuePatternAvailablePropertyId,
        ] {
            let value = probe!(element.GetCurrentPropertyValue(property))
                .map_err(|_| err("targetProbeFailed"))?;
            if exact_uia_bool(&value)? {
                return Ok(false);
            }
        }
        Ok(true)
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
        let input_composing = composing(&a);
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
                != a.editor.process as i32
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
            if non_editing_control(&element)? {
                return Err(err("targetNotEditable"));
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
        if input_composing {
            log::info!(target:"sayweave", "operation=input_composition_guard capture_id={} source=imm", a.capture_id);
            return Err(err("inputComposition"));
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
struct FinalTarget {
    id: String,
    element: IUIAutomationElement,
    selection: IUIAutomationTextRange,
    text: String,
}
impl FinalTarget {
    fn capture(
        id: &str,
        anchor: &Anchor,
        automation: &Option<IUIAutomation>,
    ) -> Result<Self, Error> {
        capture(anchor.clone(), automation)?;
        let auto = automation
            .as_ref()
            .ok_or_else(|| err("targetProbeFailed"))?;
        unsafe {
            let element = probe!(auto.GetFocusedElement()).map_err(|_| err("targetProbeFailed"))?;
            let pattern =
                probe!(element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId))
                    .map_err(|_| err("targetProbeFailed"))?;
            let ranges = probe!(pattern.GetSelection()).map_err(|_| err("targetProbeFailed"))?;
            if probe!(ranges.Length()).map_err(|_| err("targetProbeFailed"))? != 1 {
                return Err(err("targetProbeFailed"));
            }
            let selection = probe!(ranges.GetElement(0)).map_err(|_| err("targetProbeFailed"))?;
            let document = probe!(pattern.DocumentRange()).map_err(|_| err("targetProbeFailed"))?;
            let text = probe!(document.GetText(262_145))
                .map_err(|_| err("targetProbeFailed"))?
                .to_string();
            if text.encode_utf16().count() >= 262_145 {
                return Err(err("targetProbeFailed"));
            }
            composition_begin_target(id, anchor)?;
            Ok(Self {
                id: id.into(),
                element,
                selection,
                text,
            })
        }
    }
    fn verify(
        self,
        id: &str,
        anchor: &Anchor,
        automation: &Option<IUIAutomation>,
    ) -> Result<(), Error> {
        if self.id != id {
            return Err(err("staleSession"));
        }
        composition_begin_target(id, anchor)?;
        let current = Self::capture(id, anchor, automation)?;
        let auto = automation
            .as_ref()
            .ok_or_else(|| err("targetProbeFailed"))?;
        unsafe {
            if !probe!(auto.CompareElements(&self.element, &current.element))
                .map_err(|_| err("targetChanged"))?
                .as_bool()
                || self.text != current.text
                || probe!(self.selection.CompareEndpoints(
                    TextPatternRangeEndpoint_Start,
                    &current.selection,
                    TextPatternRangeEndpoint_Start
                ))
                .map_err(|_| err("targetChanged"))?
                    != 0
                || probe!(self.selection.CompareEndpoints(
                    TextPatternRangeEndpoint_End,
                    &current.selection,
                    TextPatternRangeEndpoint_End
                ))
                .map_err(|_| err("targetChanged"))?
                    != 0
            {
                return Err(err("targetChanged"));
            }
        }
        // Last COM call can reenter the host. The original event/physical fence
        // must still hold; equal text alone never reacquires a changed target.
        composition_begin_target(id, anchor)
    }
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
    let mut final_target: Option<FinalTarget> = None;
    while let Ok(envelope) = receiver.recv() {
        let request = envelope.request;
        if matches!(request, Request::Stop) {
            break;
        }
        busy.store(true, Ordering::Release);
        if stopping.load(Ordering::Acquire) {
            match request {
                Request::Capture(_, _, r)
                | Request::Copy(_, _, r)
                | Request::FinalTarget(_, _, _, r) => {
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
            Request::FinalTarget(id, anchor, verify, response) => {
                deadline(500);
                let result = if verify {
                    final_target
                        .take()
                        .ok_or_else(|| err("targetChanged"))
                        .and_then(|target| target.verify(&id, &anchor, &automation))
                } else {
                    final_target = None;
                    FinalTarget::capture(&id, &anchor, &automation)
                        .map(|target| final_target = Some(target))
                };
                let _ = response.send(result);
            }
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
                if final_target.as_ref().is_some_and(|target| target.id == id) {
                    final_target = None;
                }
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
