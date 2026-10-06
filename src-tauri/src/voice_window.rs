//! Native voice-window identity and nonactivating interaction. Target proof lives in windows_input.
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{LogicalPosition, LogicalSize, WebviewWindow};
static VISIBILITY_OPERATION: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TaskWindow {
    pub task: String,
    pub root: usize,
    pub instance: u64,
    pub bound_tick: u32,
}
#[cfg(windows)]
mod native {
    use super::TaskWindow;
    use std::sync::Mutex;
    use windows::Win32::{
        Foundation::*,
        System::SystemInformation::GetTickCount,
        UI::{Shell::*, WindowsAndMessaging::*},
    };
    struct Registered {
        root: usize,
        instance: u64,
        task: Option<TaskWindow>,
    }
    impl Registered {
        fn owns(&self, task: &TaskWindow, event_tick: u32) -> bool {
            self.root != 0
                && self.root == task.root
                && self.instance == task.instance
                && self.task.as_ref() == Some(task)
                && event_tick.wrapping_sub(task.bound_tick) <= i32::MAX as u32
        }
        fn release(&mut self, task: &str) {
            if self.task.as_ref().is_some_and(|t| t.task == task) {
                self.task = None;
            }
        }
    }
    static WINDOW: Mutex<Registered> = Mutex::new(Registered {
        root: 0,
        instance: 0,
        task: None,
    });
    const SUBCLASS: usize = 0x535756;
    fn context() -> (usize, u64, String) {
        WINDOW
            .lock()
            .map(|s| {
                (
                    s.root,
                    s.instance,
                    s.task.as_ref().map(|t| t.task.clone()).unwrap_or_default(),
                )
            })
            .unwrap_or_default()
    }
    pub(super) fn bind(task: &str) -> Option<TaskWindow> {
        let mut s = WINDOW.lock().ok()?;
        let root = HWND(s.root as *mut _);
        let mut process = 0;
        unsafe {
            GetWindowThreadProcessId(root, Some(&mut process));
        }
        if s.root == 0
            || process != std::process::id()
            || !unsafe { IsWindow(Some(root)) }.as_bool()
        {
            return None;
        }
        let bound = TaskWindow {
            task: task.into(),
            root: s.root,
            instance: s.instance,
            bound_tick: unsafe { GetTickCount() },
        };
        s.task = Some(bound.clone());
        Some(bound)
    }
    pub(super) fn release(task: &str) {
        if let Ok(mut s) = WINDOW.lock() {
            s.release(task);
        }
    }
    pub(super) fn contains(task: &TaskWindow, window: usize, event_tick: u32) -> bool {
        let valid = WINDOW.lock().is_ok_and(|s| s.owns(task, event_tick));
        if !valid || window == 0 {
            return false;
        }
        unsafe {
            let root = HWND(task.root as *mut _);
            let mut process = 0;
            GetWindowThreadProcessId(root, Some(&mut process));
            process == std::process::id()
                && IsWindow(Some(root)).as_bool()
                && (window == task.root || IsChild(root, HWND(window as *mut _)).as_bool())
        }
    }
    pub(super) fn trace(action: &str, task: Option<&str>) {
        let (root, instance, bound_task) = context();
        let task = task.unwrap_or(&bound_task);
        log::info!(target:"sayweave","operation=voice_window task_id={task} instance={instance} action={action} tick={} foreground_overlay={}",unsafe{GetTickCount()},unsafe{GetForegroundWindow().0 as usize==root});
    }
    pub(super) fn instance(root: usize) -> Option<u64> {
        WINDOW
            .lock()
            .ok()
            .filter(|s| s.root == root && root != 0)
            .map(|s| s.instance)
    }
    pub(super) fn registered() -> Option<(usize, u64)> {
        WINDOW
            .lock()
            .ok()
            .filter(|s| s.root != 0)
            .map(|s| (s.root, s.instance))
    }
    unsafe extern "system" fn procedure(
        window: HWND,
        message: u32,
        w: WPARAM,
        l: LPARAM,
        _: usize,
        instance: usize,
    ) -> LRESULT {
        let registered = WINDOW
            .lock()
            .is_ok_and(|s| s.root == window.0 as usize && s.instance == instance as u64);
        if registered {
            // Wry forwards root WM_SETFOCUS to controller.MoveFocus; this window must never
            // become the keyboard target. Mouse messages still reach the WebView.
            if message == WM_SETFOCUS {
                trace("root_focus_suppressed", None);
                return LRESULT(0);
            }
            if message == WM_ACTIVATE {
                trace(
                    if w.0 & 0xffff == WA_INACTIVE as usize {
                        "inactive"
                    } else {
                        "activation_notification"
                    },
                    None,
                );
            }
        }
        let result = unsafe { DefSubclassProc(window, message, w, l) };
        if message == WM_NCDESTROY {
            unsafe {
                let _ = RemoveWindowSubclass(window, Some(procedure), SUBCLASS);
            }
            if let Ok(mut s) = WINDOW.lock()
                && s.root == window.0 as usize
                && s.instance == instance as u64
            {
                s.root = 0;
                s.task = None;
            }
        }
        result
    }
    pub(super) fn install(root: usize) -> Result<(), String> {
        let instance = {
            let mut s = WINDOW
                .lock()
                .map_err(|_| "voice registration unavailable")?;
            if s.root != 0 {
                return Err("voice window already registered".into());
            }
            s.root = root;
            s.instance += 1;
            s.task = None;
            s.instance
        };
        unsafe {
            let window = HWND(root as *mut _);
            if GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & WS_EX_NOACTIVATE.0 == 0
                || !SetWindowSubclass(window, Some(procedure), SUBCLASS, instance as usize)
                    .as_bool()
                || SetWindowPos(
                    window,
                    None,
                    0,
                    0,
                    0,
                    0,
                    SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
                .is_err()
            {
                let _ = RemoveWindowSubclass(window, Some(procedure), SUBCLASS);
                if let Ok(mut s) = WINDOW.lock() {
                    s.root = 0;
                }
                return Err("nonactivating voice window unavailable".into());
            }
        }
        // Creation can leave cached non-client insets on an undecorated window.
        // Recalculate once without activation: setting the same outer size later
        // does not refresh them when normal phases share a fixed canvas.
        log::info!(target:"sayweave","operation=voice_window_setup instance={instance} noactivate=true");
        Ok(())
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn old_task_cleanup_and_queued_events_cannot_inherit_new_window_lease() {
            let first = TaskWindow {
                task: "first".into(),
                root: 12,
                instance: 1,
                bound_tick: 100,
            };
            let mut state = Registered {
                root: 12,
                instance: 1,
                task: Some(first.clone()),
            };
            assert!(!state.owns(&first, 99));
            assert!(state.owns(&first, 100));
            let next = TaskWindow {
                task: "next".into(),
                bound_tick: 200,
                ..first.clone()
            };
            state.task = Some(next.clone());
            state.release("first");
            assert!(!state.owns(&first, 201));
            assert!(state.owns(&next, 201));
            state.instance = 2; // Same numerical HWND can later belong to a new instance.
            assert!(!state.owns(&next, 201));
            state.instance = 1;
            state.release("next");
            assert!(!state.owns(&next, 201));
        }
    }
}
pub(crate) fn bind(task: &str) -> Option<TaskWindow> {
    #[cfg(windows)]
    {
        native::bind(task)
    }
    #[cfg(not(windows))]
    {
        let _ = task;
        None
    }
}
pub(crate) fn release(task: &str) {
    #[cfg(windows)]
    native::release(task);
    #[cfg(not(windows))]
    let _ = task;
}
pub(crate) fn contains(task: &TaskWindow, window: usize, event_tick: u32) -> bool {
    #[cfg(windows)]
    {
        native::contains(task, window, event_tick)
    }
    #[cfg(not(windows))]
    {
        let _ = (task, window, event_tick);
        false
    }
}
fn trace(action: &str, task: Option<&str>) {
    #[cfg(windows)]
    native::trace(action, task);
    #[cfg(not(windows))]
    let _ = (action, task);
}
pub(crate) fn instance(window: &WebviewWindow) -> Option<u64> {
    #[cfg(windows)]
    {
        // The owning thread registers this identity at creation and clears it at
        // WM_NCDESTROY. A dispatcher HWND getter here would synchronously wait for
        // the UI thread while a caller can still own a task/state lock.
        (window.label() == "voice-overlay")
            .then(native::registered)
            .flatten()
            .map(|(_, instance)| instance)
    }
    #[cfg(not(windows))]
    {
        let _ = window;
        Some(0)
    }
}
pub(crate) fn install(root: usize) -> Result<(), String> {
    #[cfg(windows)]
    {
        native::install(root)
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        Ok(())
    }
}
pub(crate) fn visibility(
    window: &WebviewWindow,
    visible: bool,
    task: Option<String>,
) -> tauri::Result<()> {
    #[cfg(windows)]
    {
        let registered = (window.label() == "voice-overlay")
            .then(native::registered)
            .flatten();
        let Some((handle, instance)) = registered else {
            return Ok(());
        };
        let operation = VISIBILITY_OPERATION.fetch_add(1, Ordering::AcqRel) + 1;
        window.run_on_main_thread(move || unsafe {
            use windows::Win32::{Foundation::HWND, UI::WindowsAndMessaging::*};
            if native::instance(handle) != Some(instance)
                || VISIBILITY_OPERATION.load(Ordering::Acquire) != operation
            {
                return;
            }
            trace(
                if visible {
                    "show_execute"
                } else {
                    "hide_execute"
                },
                task.as_deref(),
            );
            let _ = ShowWindow(
                HWND(handle as *mut _),
                if visible { SW_SHOWNOACTIVATE } else { SW_HIDE },
            );
        })
    }
    #[cfg(not(windows))]
    let _ = task;
    #[cfg(not(windows))]
    if visible {
        window.show()
    } else {
        window.hide()
    }
}
/// Called on the window thread, after Runtime has validated its layout generation.
pub(crate) fn layout(window: &WebviewWindow, width: f64, height: f64, task: Option<&str>) {
    if let (Ok(size), Ok(scale)) = (window.inner_size(), window.scale_factor()) {
        let size = size.to_logical::<f64>(scale);
        if (size.width - width).abs() > 0.01 || (size.height - height).abs() > 0.01 {
            trace("size_execute", task);
            let _ = window.set_size(LogicalSize::new(width, height));
        }
    }
    if let (Ok(Some(monitor)), Ok(position), Ok(size), Ok(scale)) = (
        window.current_monitor(),
        window.outer_position(),
        window.outer_size(),
        window.scale_factor(),
    ) {
        let origin = monitor.work_area().position.to_logical::<f64>(scale);
        let area = monitor.work_area().size.to_logical::<f64>(scale);
        let p = position.to_logical::<f64>(scale);
        let size = size.to_logical::<f64>(scale);
        let x =
            p.x.clamp(origin.x, (origin.x + area.width - size.width).max(origin.x));
        let y = p.y.clamp(
            origin.y,
            (origin.y + area.height - size.height).max(origin.y),
        );
        if x != p.x || y != p.y {
            trace("position_execute", task);
            let _ = window.set_position(LogicalPosition::new(x, y));
        }
    }
}
