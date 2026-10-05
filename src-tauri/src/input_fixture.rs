//! An owned blank Win32 editor for opt-in input acceptance. No personal application is reused.
use std::{
    io::{self, BufRead, Write},
    sync::mpsc,
    thread,
};
use windows::{
    Win32::{
        Foundation::*,
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
    core::{BOOL, w},
};
unsafe extern "system" fn fixture_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if m == WM_RENDERFORMAT {
        std::thread::sleep(std::time::Duration::from_millis(1200));
        unsafe {
            use windows::Win32::System::{DataExchange::*, Memory::*};
            let text = b"{\\rtf1 delayed original}\0";
            if let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, text.len()) {
                let data = GlobalLock(memory);
                if !data.is_null() {
                    std::ptr::copy_nonoverlapping(text.as_ptr(), data.cast(), text.len());
                    let _ = GlobalUnlock(memory);
                    let _ = SetClipboardData(w.0 as u32, Some(HANDLE(memory.0)));
                }
            }
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(h, m, w, l) }
}
fn message(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { SendMessageW(h, m, Some(w), Some(l)) }
}
unsafe fn focus_window(window: HWND) {
    unsafe {
        use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
        let current = GetCurrentThreadId();
        let foreground = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let attached =
            current != foreground && AttachThreadInput(current, foreground, true).as_bool();
        let target_thread = GetWindowThreadProcessId(window, None);
        let target_attached = target_thread != current
            && target_thread != foreground
            && AttachThreadInput(current, target_thread, true).as_bool();
        let _ = ShowWindow(window, SW_RESTORE);
        let _ = BringWindowToTop(window);
        let _ = SetForegroundWindow(window);
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow(window);
        if target_attached {
            let _ = AttachThreadInput(current, target_thread, false);
        }
        if attached {
            let _ = AttachThreadInput(current, foreground, false);
        }
    }
}
struct ProcessWindow {
    process: u32,
    window: HWND,
    overlay: bool,
}
unsafe extern "system" fn process_window(window: HWND, parameter: LPARAM) -> BOOL {
    unsafe {
        let target = &mut *(parameter.0 as *mut ProcessWindow);
        let mut process = 0;
        GetWindowThreadProcessId(window, Some(&mut process));
        if process == target.process
            && IsWindowVisible(window).as_bool()
            && GetWindow(window, GW_OWNER).unwrap_or_default().0.is_null()
            && (!target.overlay
                || GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & WS_EX_NOACTIVATE.0 != 0)
        {
            target.window = window;
            return BOOL(0);
        }
        BOOL(1)
    }
}
fn main() {
    if std::env::var("SAYWEAVE_INPUT_ACCEPTANCE").as_deref() != Ok("1") {
        return;
    }
    unsafe {
        let composition = std::env::args().nth(1).as_deref() == Some("composition");
        let mut tsf_manager = None;
        let mut rich_library = None;
        if composition {
            use windows::Win32::System::{Com::*, LibraryLoader::*};
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            rich_library = Some(
                LoadLibraryExW(w!("Msftedit.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).unwrap(),
            );
        }
        use windows::Win32::UI::HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
        };
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let class = WNDCLASSW {
            lpfnWndProc: Some(fixture_proc),
            lpszClassName: w!("SayweaveInputFixture"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let top = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("SayweaveInputFixture"),
            w!("Sayweave isolated input"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            30,
            30,
            480,
            260,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let editor = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            if composition {
                w!("RICHEDIT50W")
            } else {
                w!("EDIT")
            },
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_MULTILINE as u32),
            10,
            10,
            440,
            180,
            Some(top),
            None,
            None,
            None,
        )
        .unwrap();
        if composition {
            use windows::Win32::{System::Com::*, UI::TextServices::*};
            let manager: ITfThreadMgr =
                CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER).unwrap();
            manager.Activate().unwrap();
            message(editor, WM_USER + 204, WPARAM(0x10000), LPARAM(0x10000));
            tsf_manager = Some(manager);
        }
        let (t, r) = mpsc::channel::<serde_json::Value>();
        let (ok, ready) = mpsc::channel::<()>();
        let h = top.0 as usize;
        thread::spawn(move || {
            for line in io::stdin().lock().lines() {
                let Ok(line) = line else {
                    break;
                };
                let Ok(value) = serde_json::from_str(&line) else {
                    break;
                };
                if t.send(value).is_err() {
                    break;
                }
                let _ = PostMessageW(Some(HWND(h as *mut _)), WM_APP + 7, WPARAM(0), LPARAM(0));
                if ready.recv().is_err() {
                    break;
                }
            }
        });
        println!(
            "{{\"window\":{},\"focus\":{}}}",
            top.0 as usize, editor.0 as usize
        );
        io::stdout().flush().unwrap();
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_APP + 7 {
                let value = r.recv().unwrap();
                if value["action"] == "exit" {
                    let _ = DestroyWindow(top);
                    PostQuitMessage(0);
                    let _ = ok.send(());
                    break;
                }
                if value["action"] != "type"
                    && let Some(text) = value["text"].as_str()
                {
                    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
                    let _ = message(
                        editor,
                        WM_SETTEXT,
                        WPARAM(0),
                        LPARAM(wide.as_ptr() as isize),
                    );
                    let _ = message(
                        editor,
                        0x00B1,
                        WPARAM(value["start"].as_u64().unwrap_or(0) as usize),
                        LPARAM(value["end"].as_i64().unwrap_or(0) as isize),
                    );
                }
                if value["action"] == "readonly" {
                    let _ = message(editor, 0x00CF, WPARAM(1), LPARAM(0));
                }
                if value["action"] == "selection" {
                    let _ = message(
                        editor,
                        0x00B1,
                        WPARAM(value["start"].as_u64().unwrap_or(0) as usize),
                        LPARAM(value["end"].as_i64().unwrap_or(0) as isize),
                    );
                }
                if value["action"] == "type"
                    && let Some(text) = value["text"].as_str()
                {
                    // Test-only foreign edit, delivered to this owned target's message loop.
                    for unit in text.encode_utf16() {
                        message(editor, WM_CHAR, WPARAM(unit as usize), LPARAM(0));
                    }
                }
                if value["action"] == "delayClipboard" {
                    use windows::Win32::System::DataExchange::*;
                    if OpenClipboard(Some(top)).is_ok() {
                        let _ = EmptyClipboard();
                        let name: Vec<u16> =
                            "Rich Text Format".encode_utf16().chain(Some(0)).collect();
                        let kind = RegisterClipboardFormatW(windows::core::PCWSTR(name.as_ptr()));
                        let _ = SetClipboardData(kind, None);
                        let _ = CloseClipboard();
                    }
                }
                if value["action"] == "focus" {
                    // Only this owned test fixture may establish focus; product delivery never does so.
                    focus_window(top);
                    let _ = SetFocus(Some(editor));
                }
                // The test runner passes the PID of its own launched browser/editor. This command
                // is absent from production and never receives a personal application's identity.
                if value["action"] == "focus-process"
                    && let Some(process) = value["process"]
                        .as_u64()
                        .and_then(|p| u32::try_from(p).ok())
                {
                    let mut target = ProcessWindow {
                        process,
                        window: HWND::default(),
                        overlay: false,
                    };
                    let _ =
                        EnumWindows(Some(process_window), LPARAM(&mut target as *mut _ as isize));
                    if !target.window.0.is_null() {
                        focus_window(target.window);
                    }
                }
                let mut clicked = None;
                let mut click_check = None;
                let mut overlay_window = None;
                if value["action"] == "inspect-overlay" {
                    let mut target = ProcessWindow {
                        process: value["process"].as_u64().unwrap_or(0) as u32,
                        window: HWND::default(),
                        overlay: true,
                    };
                    let _ =
                        EnumWindows(Some(process_window), LPARAM(&mut target as *mut _ as isize));
                    if !target.window.0.is_null() {
                        overlay_window = Some(target.window.0 as usize);
                    }
                }
                if value["action"] == "click-window" {
                    let window = HWND(value["window"].as_u64().unwrap_or(0) as usize as *mut _);
                    let mut process = 0;
                    GetWindowThreadProcessId(window, Some(&mut process));
                    if process == value["process"].as_u64().unwrap_or(0) as u32
                        && IsWindow(Some(window)).as_bool()
                    {
                        let mut point = POINT {
                            x: value["x"].as_i64().unwrap_or(0) as i32,
                            y: value["y"].as_i64().unwrap_or(0) as i32,
                        };
                        use windows::Win32::Graphics::Gdi::ClientToScreen;
                        let converted = ClientToScreen(window, &mut point).as_bool();
                        let mut client = RECT::default();
                        let _ = GetClientRect(window, &mut client);
                        click_check = Some(
                            serde_json::json!({"converted":converted,"width":client.right,"height":client.bottom,"pointRootMatches":GetAncestor(WindowFromPoint(point), GA_ROOT) == window,"x":point.x,"y":point.y}),
                        );
                        if converted && GetAncestor(WindowFromPoint(point), GA_ROOT) == window {
                            let mut old = POINT::default();
                            let _ = GetCursorPos(&mut old);
                            let _ = SetCursorPos(point.x, point.y);
                            let events =
                                [MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP].map(|flags| INPUT {
                                    r#type: INPUT_MOUSE,
                                    Anonymous: INPUT_0 {
                                        mi: MOUSEINPUT {
                                            dwFlags: flags,
                                            ..Default::default()
                                        },
                                    },
                                });
                            clicked = Some(SendInput(&events, std::mem::size_of::<INPUT>() as i32));
                            let _ = SetCursorPos(old.x, old.y);
                        }
                    }
                }
                let mut foreground_process = 0;
                GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut foreground_process));
                let mut buf = [0u16; 4096];
                let len = GetWindowTextW(editor, &mut buf) as usize;
                let selection = message(editor, 0x00B0, WPARAM(0), LPARAM(0)).0 as usize;
                println!(
                    "{}",
                    serde_json::json!({"foreground":GetForegroundWindow()==top,"foregroundProcess":foreground_process,"text":String::from_utf16_lossy(&buf[..len]),"start":selection&65535,"end":(selection>>16)&65535,"clicked":clicked,"clickCheck":click_check,"overlayWindow":overlay_window})
                );
                io::stdout().flush().unwrap();
                let _ = ok.send(());
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        if let Some(manager) = tsf_manager {
            let _ = manager.Deactivate();
            drop(manager);
            windows::Win32::System::Com::CoUninitialize();
        }
        if let Some(library) = rich_library {
            let _ = FreeLibrary(library);
        }
    }
}
