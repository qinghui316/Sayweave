use super::types::{Binding, Config};
use crate::{
    history::types::Error,
    windows_input::{Anchor, WindowsInputService},
};
use std::{
    collections::HashSet,
    sync::{Mutex, OnceLock},
};
use tokio::sync::mpsc::Sender;
use windows::{
    Win32::{
        Foundation::*,
        System::{RemoteDesktop::*, Threading::*},
        UI::{Accessibility::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
    core::w,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskStamp {
    pub id: Option<String>,
    pub sequence: u64,
}
fn stamp(b: &Bridge) -> TaskStamp {
    TaskStamp {
        id: b.task_active.clone(),
        sequence: b.task_sequence,
    }
}
fn set_task(b: &mut Bridge, task: Option<String>) {
    if b.task_active != task {
        b.task_sequence += 1;
        b.task_active = task;
    }
}
#[derive(Clone, Debug)]
pub enum Event {
    Key {
        epoch: u64,
        generation: u64,
        revision: i64,
        pressed: bool,
        anchor: Option<Anchor>,
        task: TaskStamp,
    },
    Captured(String, u64),
    Cancel(Option<String>, u64),
    Interrupt(Option<String>),
}
struct Bridge {
    sender: Sender<Event>,
    config: Config,
    binding: Binding,
    pressed: HashSet<u16>,
    held: Option<u64>,
    epoch: u64,
    capturing: bool,
    overflow: std::sync::Arc<std::sync::atomic::AtomicBool>,
    capture_key: Option<String>,
    armed: bool,
    task_active: Option<String>,
    task_sequence: u64,
    generation: u64,
    altgr: bool,
    consumed: HashSet<u16>,
}
static BRIDGE: OnceLock<Mutex<Option<Bridge>>> = OnceLock::new();
fn state() -> &'static Mutex<Option<Bridge>> {
    BRIDGE.get_or_init(|| Mutex::new(None))
}
fn vk(key: &str) -> u16 {
    match key {
        "AltRight" => VK_RMENU.0,
        "AltLeft" => VK_LMENU.0,
        "ControlRight" => VK_RCONTROL.0,
        "ControlLeft" => VK_LCONTROL.0,
        "ShiftRight" => VK_RSHIFT.0,
        "ShiftLeft" => VK_LSHIFT.0,
        "Space" => VK_SPACE.0,
        "CapsLock" => VK_CAPITAL.0,
        "Insert" => VK_INSERT.0,
        "Home" => VK_HOME.0,
        "End" => VK_END.0,
        "PageUp" => VK_PRIOR.0,
        "PageDown" => VK_NEXT.0,
        value if value.starts_with("Key") => {
            value.as_bytes().get(3).copied().unwrap_or_default() as u16
        }
        value if value.starts_with("Digit") => {
            value.as_bytes().get(5).copied().unwrap_or_default() as u16
        }
        value => value
            .strip_prefix('F')
            .and_then(|v| v.parse::<u16>().ok())
            .map(|v| VK_F1.0 + v - 1)
            .unwrap_or(0),
    }
}
fn code(value: u16) -> Option<String> {
    for name in [
        "AltRight",
        "AltLeft",
        "ControlRight",
        "ControlLeft",
        "ShiftRight",
        "ShiftLeft",
        "Space",
        "CapsLock",
        "Insert",
        "Home",
        "End",
        "PageUp",
        "PageDown",
    ] {
        if vk(name) == value {
            return Some(name.into());
        }
    }
    if (0x41..=0x5a).contains(&value) {
        Some(format!("Key{}", char::from_u32(value as u32)?))
    } else if (0x30..=0x39).contains(&value) {
        Some(format!("Digit{}", char::from_u32(value as u32)?))
    } else if (VK_F1.0..=VK_F24.0).contains(&value) {
        Some(format!("F{}", value - VK_F1.0 + 1))
    } else {
        None
    }
}
fn down(keys: &HashSet<u16>, modifier: &str) -> bool {
    match modifier {
        "Control" => keys.contains(&VK_LCONTROL.0) || keys.contains(&VK_RCONTROL.0),
        "Alt" => keys.contains(&VK_LMENU.0) || keys.contains(&VK_RMENU.0),
        "Shift" => keys.contains(&VK_LSHIFT.0) || keys.contains(&VK_RSHIFT.0),
        "Super" => keys.contains(&VK_LWIN.0) || keys.contains(&VK_RWIN.0),
        _ => false,
    }
}
fn matches(b: &Bridge) -> bool {
    if b.binding.key == "AltRight" && b.altgr {
        return false;
    }
    b.pressed.contains(&vk(&b.binding.key))
        && b.binding.modifiers.iter().all(|m| down(&b.pressed, m))
        && ["Control", "Alt", "Shift", "Super"].iter().all(|m| {
            b.binding.modifiers.iter().any(|v| v == m)
                || !down(&b.pressed, m)
                || b.binding.single_modifier()
                    && vk(&b.binding.key)
                        == match *m {
                            "Control" => {
                                if b.binding.key.ends_with("Right") {
                                    VK_RCONTROL.0
                                } else {
                                    VK_LCONTROL.0
                                }
                            }
                            "Alt" => {
                                if b.binding.key.ends_with("Right") {
                                    VK_RMENU.0
                                } else {
                                    VK_LMENU.0
                                }
                            }
                            "Shift" => {
                                if b.binding.key.ends_with("Right") {
                                    VK_RSHIFT.0
                                } else {
                                    VK_LSHIFT.0
                                }
                            }
                            _ => 0,
                        }
        })
}
fn send(b: &Bridge, event: Event) {
    if b.sender.try_send(event).is_err()
        && !b.overflow.swap(true, std::sync::atomic::Ordering::AcqRel)
    {
        let sender = b.sender.clone();
        let overflow = b.overflow.clone();
        let task = b.task_active.clone();
        tauri::async_runtime::spawn(async move {
            let _ = sender.send(Event::Interrupt(task)).await;
            overflow.store(false, std::sync::atomic::Ordering::Release);
        });
    }
}
fn release(b: &mut Bridge) {
    if let Some(epoch) = b.held.take() {
        send(
            b,
            Event::Key {
                epoch,
                generation: b.generation,
                revision: b.config.revision,
                pressed: false,
                anchor: None,
                task: stamp(b),
            },
        );
    }
    unsafe {
        let _ = KillTimer(None, 1);
    }
}
unsafe extern "system" fn hook(n: i32, w: WPARAM, l: LPARAM) -> LRESULT {
    if n >= 0 {
        let key = unsafe { &*(l.0 as *const KBDLLHOOKSTRUCT) };
        // Track the synthetic Ctrl used by AltGr without turning injected keys into triggers.
        if key.flags.0 & LLKHF_INJECTED.0 != 0
            && key.vkCode == VK_LCONTROL.0 as u32
            && key.dwExtraInfo != 0x53595756
            && let Ok(mut state) = state().lock()
            && let Some(b) = state.as_mut()
        {
            b.altgr = [WM_KEYDOWN, WM_SYSKEYDOWN].contains(&(w.0 as u32));
        }
        // Ignore synthetic injection, including our paste. Hooks never release user-held keys.
        if key.flags.0 & LLKHF_INJECTED.0 == 0
            && let Ok(mut state) = state().lock()
            && let Some(b) = state.as_mut()
        {
            let pressed = [WM_KEYDOWN, WM_SYSKEYDOWN].contains(&(w.0 as u32));
            let released = [WM_KEYUP, WM_SYSKEYUP].contains(&(w.0 as u32));
            let value = key.vkCode as u16;
            if pressed && !b.pressed.insert(value) {
                return if b.consumed.contains(&value) {
                    LRESULT(1)
                } else {
                    unsafe { CallNextHookEx(None, n, w, l) }
                };
            }
            if released {
                b.pressed.remove(&value);
            }
            if b.capturing {
                let mut pid = 0;
                unsafe {
                    GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
                }
                if pid != std::process::id() {
                    b.capturing = false;
                    b.capture_key = None;
                    send(b, Event::Cancel(b.task_active.clone(), b.generation));
                    return unsafe { CallNextHookEx(None, n, w, l) };
                }

                if value == VK_ESCAPE.0 && pressed {
                    send(b, Event::Cancel(b.task_active.clone(), b.generation));
                } else if pressed && let Some(name) = code(value) {
                    let mut mods = vec![];
                    if ![
                        "AltRight",
                        "AltLeft",
                        "ControlRight",
                        "ControlLeft",
                        "ShiftRight",
                        "ShiftLeft",
                    ]
                    .contains(&name.as_str())
                    {
                        for m in ["Control", "Alt", "Shift", "Super"] {
                            if down(&b.pressed, m) {
                                mods.push(m);
                            }
                        }
                    }
                    let value = mods
                        .into_iter()
                        .chain(std::iter::once(name.as_str()))
                        .collect::<Vec<_>>()
                        .join("+");
                    b.capture_key = Some(value);
                } else if released
                    && let Some(value) = b.capture_key.take()
                    && Binding::parse(&value).is_ok()
                {
                    send(b, Event::Captured(value, b.generation));
                }
                let mut pid = 0;
                unsafe {
                    GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
                }
                if pid == std::process::id() && pressed {
                    b.consumed.insert(value);
                    return LRESULT(1);
                }
            } else {
                if b.task_active.is_some() && value == VK_ESCAPE.0 && pressed {
                    send(b, Event::Cancel(b.task_active.clone(), b.generation));
                    b.consumed.insert(value);
                    return LRESULT(1);
                }
                if b.held.is_some() && !matches(b) {
                    release(b);
                    b.armed = false;
                }
                if b.pressed.is_empty() {
                    b.armed = true;
                }
                if pressed
                    && b.armed
                    && b.binding.single_modifier()
                    && value == vk(&b.binding.key)
                    && matches(b)
                {
                    b.epoch += 1;
                    b.held = Some(b.epoch);
                    b.consumed.insert(value);
                    send(
                        b,
                        Event::Key {
                            epoch: b.epoch,
                            generation: b.generation,
                            revision: b.config.revision,
                            pressed: true,
                            anchor: WindowsInputService::anchor(),
                            task: stamp(b),
                        },
                    );
                    unsafe {
                        SetTimer(None, 1, 50, None);
                    }
                }
                if pressed && b.consumed.contains(&value) {
                    return LRESULT(1);
                }
            }
            if released && b.consumed.remove(&value) {
                return LRESULT(1);
            }
        }
    }
    unsafe { CallNextHookEx(None, n, w, l) }
}
unsafe extern "system" fn input_event(
    _: HWINEVENTHOOK,
    event: u32,
    window: HWND,
    object: i32,
    _: i32,
    _: u32,
    event_tick: u32,
) {
    crate::windows_input::observe(event, window.0 as usize, object, event_tick);
}
#[derive(Clone)]
pub struct Backend {
    thread: u32,
}
fn interrupt(b: &mut Bridge) {
    send(b, Event::Interrupt(b.task_active.clone()));
    b.generation += 1;
    b.pressed.clear();
    b.held = None;
    b.armed = false;
    b.altgr = false;
    unsafe {
        let _ = KillTimer(None, 1);
    }
}
fn rearm(b: &mut Bridge, physical: HashSet<u16>) {
    b.pressed = physical;
    b.consumed.retain(|key| b.pressed.contains(key));
    b.armed = b.pressed.is_empty();
}
unsafe extern "system" fn listener_proc(
    window: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
) -> LRESULT {
    // Power broadcasts are sent to the window procedure, rather than retrieved by GetMessage.
    if (message == WM_WTSSESSION_CHANGE && w.0 == WTS_SESSION_LOCK as usize
        || message == WM_POWERBROADCAST && w.0 == PBT_APMSUSPEND as usize)
        && let Ok(mut state) = state().lock()
        && let Some(b) = state.as_mut()
    {
        interrupt(b);
    }
    if (message == WM_WTSSESSION_CHANGE && w.0 == WTS_SESSION_UNLOCK as usize
        || message == WM_POWERBROADCAST
            && [
                PBT_APMRESUMEAUTOMATIC as usize,
                PBT_APMRESUMESUSPEND as usize,
            ]
            .contains(&w.0))
        && let Ok(mut state) = state().lock()
        && let Some(b) = state.as_mut()
    {
        let physical = (1u16..=254)
            .filter(|key| unsafe { GetAsyncKeyState(*key as i32) < 0 })
            .collect();
        rearm(b, physical);
    }
    unsafe { DefWindowProcW(window, message, w, l) }
}
impl Backend {
    pub fn start(sender: Sender<Event>) -> Result<Self, Error> {
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("sayweave-keyboard".into())
            .spawn(move || unsafe {
                let config = Config::default();
                *state().lock().unwrap() = Some(Bridge {
                    binding: Binding::parse(&config.binding).unwrap(),
                    config,
                    sender,
                    pressed: HashSet::new(),
                    held: None,
                    epoch: 0,
                    capturing: false,
                    overflow: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                    capture_key: None,
                    armed: true,
                    task_active: None,
                    task_sequence: 0,
                    generation: 0,
                    altgr: false,
                    consumed: HashSet::new(),
                });
                let mut message = MSG::default();
                let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
                let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook), None, 0);
                RegisterClassW(&WNDCLASSW {
                    lpfnWndProc: Some(listener_proc),
                    lpszClassName: w!("SayweaveShortcutEvents"),
                    ..Default::default()
                });
                let window = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("SayweaveShortcutEvents"),
                    w!("Sayweave input events"),
                    WINDOW_STYLE(0),
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    None,
                    None,
                );
                let foreground = SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    None,
                    Some(input_event),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );
                let destroyed = SetWinEventHook(
                    EVENT_OBJECT_DESTROY,
                    EVENT_OBJECT_DESTROY,
                    None,
                    Some(input_event),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );
                let objects = SetWinEventHook(
                    EVENT_OBJECT_FOCUS,
                    EVENT_OBJECT_TEXTSELECTIONCHANGED,
                    None,
                    Some(input_event),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                );
                let notification = window.as_ref().ok().is_some_and(|w| {
                    WTSRegisterSessionNotification(*w, NOTIFY_FOR_THIS_SESSION).is_ok()
                });
                if keyboard.is_err()
                    || window.is_err()
                    || foreground.0.is_null()
                    || destroyed.0.is_null()
                    || objects.0.is_null()
                    || !notification
                {
                    let _ = tx.send(Err(Error::new("shortcutUnavailable")));
                    if let Ok(k) = keyboard {
                        let _ = UnhookWindowsHookEx(k);
                    }
                    if let Ok(w) = window {
                        if notification {
                            let _ = WTSUnRegisterSessionNotification(w);
                        }
                        let _ = DestroyWindow(w);
                    }
                    let _ = UnhookWinEvent(foreground);
                    let _ = UnhookWinEvent(objects);
                    let _ = UnhookWinEvent(destroyed);
                    *state().lock().unwrap() = None;
                    return;
                }
                let keyboard = keyboard.unwrap();
                let window = window.unwrap();
                let _ = tx.send(Ok(GetCurrentThreadId()));
                while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                    if message.message == WM_APP + 1 {
                        SetTimer(None, 1, 50, None);
                    }
                    if message.message == WM_TIMER
                        && let Ok(mut state) = state().lock()
                        && let Some(b) = state.as_mut()
                    {
                        b.pressed.retain(|key| GetAsyncKeyState(*key as i32) < 0);
                        if b.held.is_some() && !matches(b) {
                            release(b);
                            b.armed = b.pressed.is_empty();
                        }
                    }
                    let _ = TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
                let _ = WTSUnRegisterSessionNotification(window);
                let _ = UnhookWindowsHookEx(keyboard);
                let _ = UnhookWinEvent(foreground);
                let _ = UnhookWinEvent(objects);
                let _ = UnhookWinEvent(destroyed);
                let _ = DestroyWindow(window);
                *state().lock().unwrap() = None;
            })
            .map_err(|_| Error::new("shortcutUnavailable"))?;
        Ok(Self {
            thread: rx.recv().map_err(|_| Error::new("shortcutUnavailable"))??,
        })
    }
    pub fn configure(&self, config: Config, capturing: bool) {
        if let Ok(mut state) = state().lock()
            && let Some(b) = state.as_mut()
        {
            b.generation += 1;
            b.binding = Binding::parse(&config.binding).unwrap();
            b.config = config;
            b.capturing = capturing;
            b.capture_key = None;
            b.held = None;
            b.armed = b.pressed.is_empty();
        }
    }
    pub fn generation(&self) -> Option<u64> {
        state()
            .lock()
            .ok()
            .and_then(|s| s.as_ref().map(|b| b.generation))
    }
    pub fn task_active(&self, active: Option<String>) {
        if let Ok(mut state) = state().lock()
            && let Some(b) = state.as_mut()
        {
            set_task(b, active);
        }
    }
    pub fn task_is_current(&self, task: &TaskStamp) -> bool {
        state()
            .lock()
            .ok()
            .and_then(|s| {
                s.as_ref()
                    .map(|b| b.task_sequence == task.sequence && b.task_active == task.id)
            })
            .unwrap_or(false)
    }
    pub fn combo_pressed(&self, binding: &str) {
        if let Ok(mut state) = state().lock()
            && let Some(b) = state.as_mut()
            && !b.capturing
            && b.config.binding == binding
            && b.held.is_none()
            && b.armed
            && matches(b)
        {
            b.epoch += 1;
            b.held = Some(b.epoch);
            send(
                b,
                Event::Key {
                    epoch: b.epoch,
                    generation: b.generation,
                    revision: b.config.revision,
                    pressed: true,
                    anchor: WindowsInputService::anchor(),
                    task: stamp(b),
                },
            );
            unsafe {
                let _ = PostThreadMessageW(self.thread, WM_APP + 1, WPARAM(0), LPARAM(0));
            }
        }
    }
    pub fn stop(&self) {
        unsafe {
            let _ = PostThreadMessageW(self.thread, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn bridge() -> (Bridge, tokio::sync::mpsc::Receiver<Event>) {
        let (sender, receiver) = tokio::sync::mpsc::channel(8);
        (
            Bridge {
                sender,
                config: Config::default(),
                binding: Binding::parse("AltRight").unwrap(),
                pressed: HashSet::new(),
                held: None,
                epoch: 0,
                capturing: false,
                capture_key: None,
                armed: true,
                task_active: None,
                task_sequence: 0,
                generation: 0,
                altgr: false,
                consumed: HashSet::new(),
                overflow: Default::default(),
            },
            receiver,
        )
    }
    #[test]
    fn altgr_and_unrequired_modifiers_cannot_trigger() {
        let (mut b, _) = bridge();
        b.pressed.insert(VK_RMENU.0);
        assert!(matches(&b));
        b.altgr = true;
        assert!(!matches(&b));
        b.altgr = false;
        b.pressed.insert(VK_LCONTROL.0);
        assert!(!matches(&b));
        b.pressed.clear();
        b.binding = Binding::parse("Control+Shift+KeyD").unwrap();
        b.pressed.extend([VK_LCONTROL.0, VK_RSHIFT.0, VK_D.0]);
        assert!(matches(&b));
        b.pressed.remove(&VK_LCONTROL.0);
        assert!(!matches(&b));
    }
    #[test]
    fn a_release_carries_its_original_epoch_and_emits_once() {
        let (mut b, mut receiver) = bridge();
        b.held = Some(7);
        b.config.revision = 4;
        release(&mut b);
        release(&mut b);
        let Event::Key {
            epoch,
            revision,
            pressed,
            ..
        } = receiver.try_recv().unwrap()
        else {
            panic!()
        };
        assert_eq!((epoch, revision, pressed), (7, 4, false));
        assert!(receiver.try_recv().is_err());
    }
    #[test]
    fn task_stamps_distinguish_idle_periods_and_queued_task_events() {
        let (mut b, _) = bridge();
        let before = stamp(&b);
        set_task(&mut b, Some("A".into()));
        let during = stamp(&b);
        set_task(&mut b, Some("A".into()));
        assert_eq!(stamp(&b), during);
        set_task(&mut b, None);
        assert_ne!(stamp(&b), before);
        set_task(&mut b, Some("B".into()));
        assert_ne!(stamp(&b), during);
    }
    #[test]
    fn suspend_binds_the_old_task_and_requires_a_new_press_after_resume() {
        let (mut b, mut receiver) = bridge();
        b.task_active = Some("old-task".into());
        b.held = Some(7);
        b.pressed.insert(VK_RMENU.0);
        b.altgr = true;
        let queued_generation = b.generation;
        interrupt(&mut b);
        b.task_active = Some("new-task".into());
        let Event::Interrupt(task) = receiver.try_recv().unwrap() else {
            panic!()
        };
        assert_eq!(task.as_deref(), Some("old-task"));
        assert!(b.held.is_none() && b.pressed.is_empty() && !b.armed);
        assert!(!b.altgr);
        assert_ne!(b.generation, queued_generation);
        release(&mut b);
        assert!(receiver.try_recv().is_err());
        rearm(&mut b, HashSet::from([VK_RMENU.0]));
        assert!(!b.armed);
        rearm(&mut b, HashSet::new());
        assert!(b.armed);
        b.pressed.insert(VK_RMENU.0);
        assert!(matches(&b));
    }
}
