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
fn context_status() -> serde_json::Value {
    use windows::Win32::{System::Com::*, UI::TextServices::*};
    let result = (|| unsafe {
        let manager: ITfThreadMgr =
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
        let context = manager.GetFocus()?.GetTop()?;
        let status = context.GetStatus()?;
        Ok::<_, windows::core::Error>(serde_json::json!({
            "staticFlags": status.dwStaticFlags,
            "dynamicFlags": status.dwDynamicFlags,
            "transitory": status.dwStaticFlags & TF_SS_TRANSITORY != 0,
        }))
    })();
    result.unwrap_or_else(|e| serde_json::json!({"error":format!("{:08x}",e.code().0 as u32)}))
}
// Metadata-only diagnostic for an explicitly owned foreground fixture. Never
// reads a field name/value/body and never supplies production range authority.
fn focus_metadata(process: u32) -> serde_json::Value {
    std::thread::spawn(move || unsafe {
        use windows::{core::Interface, Win32::{System::{Com::*, Ole::*, Variant::*}, UI::Accessibility::*}};
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = (|| {
            let window=GetForegroundWindow();let mut actual=0;
            let thread=GetWindowThreadProcessId(window,Some(&mut actual));
            if actual!=process { return Err(windows::core::Error::from_hresult(E_ACCESSDENIED)); }
            let mut info=GUITHREADINFO { cbSize:std::mem::size_of::<GUITHREADINFO>() as u32,..Default::default() };
            GetGUIThreadInfo(thread,&mut info)?;
            let mut routes=vec![];let mut identities:Vec<windows::core::IUnknown>=vec![];
            let mut stable=vec![];let mut runtime_ids=vec![];
            for target in [info.hwndFocus,window] {
                let mut accessible: Option<IAccessible>=None;
                let value=AccessibleObjectFromWindow(target,OBJID_CLIENT.0 as u32,&IAccessible::IID,(&mut accessible as *mut Option<IAccessible>).cast());
                let mut route=serde_json::json!({"window":target.0 as usize,"error":value.as_ref().err().map(|e|format!("{:08x}",e.code().0 as u32)),"chain":[]});
                if value.is_ok() && let Some(mut current)=accessible {
                    for _ in 0..16 {
                        let child=VARIANT::from(0i32);
                        let focus=current.accFocus();
                        route["chain"].as_array_mut().unwrap().push(serde_json::json!({"focusVt":focus.as_ref().ok().map(|f|f.vt().0),"focusChild":focus.as_ref().ok().and_then(|f|i32::try_from(f).ok()),"role":current.get_accRole(&child).ok().and_then(|r|i32::try_from(&r).ok()),"state":current.get_accState(&child).ok().and_then(|r|i32::try_from(&r).ok()),"focusError":focus.as_ref().err().map(|e|format!("{:08x}",e.code().0 as u32))}));
                        let Ok(focus)=focus else { break; };
                        let next=if focus.vt()==VT_DISPATCH { IDispatch::try_from(&focus).and_then(|v|v.cast()) } else if focus.vt()==VT_I4 && i32::try_from(&focus).unwrap_or(0)!=0 { current.get_accChild(&focus).and_then(|v|v.cast()) } else { break; };
                        let Ok(next)=next else { break; };current=next;
                    }
                    let mut opaque=std::ptr::null_mut();let mut length=0;
                    let bytes=if let Ok(identity)=current.cast::<IAccIdentity>() {
                        let result=identity.GetIdentityString(0,&mut opaque,&mut length);
                        let bytes=if result.is_ok()&&!opaque.is_null()&&length>0&&length<=1024 {Some(std::slice::from_raw_parts(opaque,length as usize).to_vec())}else{None};
                        if !opaque.is_null(){CoTaskMemFree(Some(opaque.cast()));}bytes
                    }else{None};
                    stable.push(bytes);
                    let runtime_id=(|| {
                        let raw=current.cast::<IRawElementProviderSimple>().or_else(|_|current.cast::<IServiceProvider>()?.QueryService(&IAccessible::IID))?;
                        let read_only=raw.GetPatternProvider(UIA_TextPatternId).and_then(|p|p.cast::<ITextProvider>()).and_then(|p|p.DocumentRange()).and_then(|r|r.GetAttributeValue(UIA_IsReadOnlyAttributeId)).ok().and_then(|v|bool::try_from(&v).ok());
                        route["uia"]=serde_json::json!({"controlType":raw.GetPropertyValue(UIA_ControlTypePropertyId).ok().and_then(|v|i32::try_from(&v).ok()),"keyboardFocus":raw.GetPropertyValue(UIA_HasKeyboardFocusPropertyId).ok().and_then(|v|bool::try_from(&v).ok()),"focusable":raw.GetPropertyValue(UIA_IsKeyboardFocusablePropertyId).ok().and_then(|v|bool::try_from(&v).ok()),"textReadOnly":read_only});
                        let array=raw.cast::<IRawElementProviderFragment>()?.GetRuntimeId()?;
                        let result=(||{if array.is_null()||SafeArrayGetDim(array)!=1||SafeArrayGetVartype(array)?!=VT_I4{return Err(windows::core::Error::from_hresult(E_NOINTERFACE));}let low=SafeArrayGetLBound(array,1)?;let high=SafeArrayGetUBound(array,1)?;if high<low||high-low>63{return Err(windows::core::Error::from_hresult(E_INVALIDARG));}let mut values=vec![];for index in low..=high {let mut value=0i32;SafeArrayGetElement(array,&index,(&mut value as *mut i32).cast())?;values.push(value);}Ok::<_,windows::core::Error>(values)})();
                        if !array.is_null(){let _=SafeArrayDestroy(array);}result
                    })();runtime_ids.push(runtime_id.ok());
                    if let Ok(identity)=current.cast(){ identities.push(identity); }
                }
                routes.push(route);
            }
            Ok::<_,windows::core::Error>(serde_json::json!({"routes":routes,"sameObject":identities.len()==2&&identities[0]==identities[1],"opaqueIdentityPresent":stable.iter().all(Option::is_some),"opaqueIdentitySame":stable.len()==2&&stable[0].is_some()&&stable[0]==stable[1],"runtimeIds":runtime_ids}))
        })();
        CoUninitialize();result.unwrap_or_else(|e|serde_json::json!({"error":format!("{:08x}",e.code().0 as u32)}))
    }).join().unwrap_or_else(|_|serde_json::json!({"error":"metadataWorker"}))
}
// Read only known static product labels from the explicitly owned application.
// A native PID is not proof that the frontend rendered.
fn product_surface(process:u32)->serde_json::Value {
    if std::env::var("SAYWEAVE_DAILY_OWNED_PID").ok().and_then(|p|p.parse().ok())!=Some(process) {
        return serde_json::json!({"error":"ownedProductRequired"});
    }
    thread::spawn(move || unsafe {
        use windows::Win32::{System::Com::*,UI::Accessibility::*};
        let result=(|| {
            CoInitializeEx(None,COINIT_MULTITHREADED).ok()?;
            let mut target=ProcessWindow{process,window:HWND::default(),overlay:false};
            // The owned-window callback deliberately stops enumeration once
            // matched; FALSE with last-error0 is not a failed lookup.
            let _=EnumWindows(Some(process_window),LPARAM(&mut target as *mut _ as isize));
            if target.window.0.is_null(){return Err(windows::core::Error::from_hresult(E_FAIL));}
            let automation:IUIAutomation=CoCreateInstance(&CUIAutomation,None,CLSCTX_INPROC_SERVER)?;
            let root=automation.ElementFromHandle(target.window)?;
            let elements=root.FindAll(TreeScope_Descendants,&automation.CreateTrueCondition()?)?;
            let wanted=["首页","历史记录","词典","设置","开始口述","本机用量"];
            let mut found=std::collections::BTreeSet::new();let mut connection_error=false;
            for index in 0..elements.Length()?.min(1000) {
                let item=elements.GetElement(index)?;
                if item.CurrentIsOffscreen()?.as_bool(){continue;}
                let name=item.CurrentName()?.to_string();
                for label in wanted {if name==label{found.insert(label);}}
                connection_error|=name.contains("无法访问此页面")||name.contains("ERR_CONNECTION_REFUSED");
            }
            Ok::<_,windows::core::Error>(serde_json::json!({"process":process,"knownLabels":found,"frontendReady":wanted.iter().all(|n|found.contains(n)),"connectionError":connection_error}))
        })();
        CoUninitialize();result.unwrap_or_else(|e|serde_json::json!({"error":format!("{:08x}",e.code().0 as u32)}))
    }).join().unwrap_or_else(|_|serde_json::json!({"error":"surfaceWorker"}))
}
// Read-only evidence from a runner-owned target, on an independent MTA. Never
// emit external text or use these measurements to move a production selection.
fn selection_evidence(process: u32) -> serde_json::Value {
    thread::spawn(move || {
        use sha2::{Digest, Sha256};
        use windows::{core::Interface, Win32::{System::Com::*, UI::Accessibility::*}};
        let result = (|| unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            let result = (|| {
                let auto: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
                let limits: IUIAutomation2 = auto.cast()?;
                limits.SetConnectionTimeout(500)?;
                limits.SetTransactionTimeout(500)?;
                let element = auto.GetFocusedElement()?;
                if element.CurrentProcessId()? != process as i32 || !element.CurrentHasKeyboardFocus()?.as_bool() || element.CurrentIsPassword()?.as_bool() {
                    return Err(windows::core::Error::from_hresult(E_ACCESSDENIED));
                }
                let pattern: IUIAutomationTextPattern = element.GetCurrentPatternAs(UIA_TextPatternId)?;
                let ranges = pattern.GetSelection()?;
                if ranges.Length()? != 1 { return Err(windows::core::Error::from_hresult(E_FAIL)); }
                let selected = ranges.GetElement(0)?;
                let document = pattern.DocumentRange()?;
                let prefix = document.Clone()?;
                prefix.MoveEndpointByRange(TextPatternRangeEndpoint_End, &selected, TextPatternRangeEndpoint_Start)?;
                let suffix = document.Clone()?;
                suffix.MoveEndpointByRange(TextPatternRangeEndpoint_Start, &selected, TextPatternRangeEndpoint_End)?;
                let text = document.GetText(16385)?.to_string();
                let before = prefix.GetText(16385)?.to_string();
                let picked = selected.GetText(16385)?.to_string();
                let after = suffix.GetText(16385)?.to_string();
                let count = |value: &str| value.encode_utf16().count();
                if count(&text) > 16384 || text != format!("{before}{picked}{after}") {
                    return Err(windows::core::Error::from_hresult(E_INVALIDARG));
                }
                let current = pattern.GetSelection()?;
                let stable = current.Length()? == 1 && selected.Compare(&current.GetElement(0)?)?.as_bool()
                    && auto.CompareElements(&element, &auto.GetFocusedElement()?)?.as_bool();
                let msaa = (|| -> windows::core::Result<serde_json::Value> {
                    use windows::Win32::System::Variant::*;
                    let thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
                    let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
                    GetGUIThreadInfo(thread, &mut info)?;
                    unsafe extern "system" fn child(window: HWND, parameter: LPARAM) -> BOOL {
                        unsafe { let windows = &mut *(parameter.0 as *mut Vec<HWND>); if windows.len() < 64 { windows.push(window); BOOL(1) } else { BOOL(0) } }
                    }
                    let mut windows = vec![info.hwndFocus];
                    let _ = EnumChildWindows(Some(info.hwndFocus), Some(child), LPARAM((&mut windows as *mut Vec<HWND>) as isize));
                    let mut evidence = Vec::new();
                    for window in windows {
                    let mut class = [0u16; 128];
                    let class_len = GetClassNameW(window, &mut class);
                    let mut result: Option<IAccessible> = None;
                    if AccessibleObjectFromWindow(window, OBJID_CLIENT.0 as u32, &IAccessible::IID, (&mut result as *mut Option<IAccessible>).cast()).is_err() {continue;}
                    let mut object = result.ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?;
                    let mut steps = Vec::new();
                    for _ in 0..16 {
                        let value = object.accFocus()?;
                        let role = i32::try_from(&object.get_accRole(&VARIANT::from(0i32))?).ok();
                        let service = object.cast::<IServiceProvider>();
                        let service_raw = service.as_ref().ok().is_some_and(|service| service.QueryService::<IRawElementProviderSimple>(&IAccessible::IID).is_ok());
                        steps.push(serde_json::json!({"role":role,"raw":object.cast::<IRawElementProviderSimple>().is_ok(),"ex":object.cast::<IAccessibleEx>().is_ok(),"service":service.is_ok(),"serviceRaw":service_raw,"focusType":value.Anonymous.Anonymous.vt.0,"focusChild":i32::try_from(&value).ok()}));
                        if value.Anonymous.Anonymous.vt != VT_DISPATCH {break;}
                        object = IDispatch::try_from(&value)?.cast()?;
                    }
                    evidence.push(serde_json::json!({"windowClass":String::from_utf16_lossy(&class[..class_len as usize]),"steps":steps}));
                    }
                    Ok(serde_json::json!(evidence))
                })().unwrap_or_else(|e|serde_json::json!({"error":format!("{:08x}",e.code().0 as u32)}));
                Ok::<_, windows::core::Error>(serde_json::json!({"stable":stable,"documentUtf16":count(&text),"startUtf16":count(&before),"endUtf16":count(&before)+count(&picked),"documentSha256":format!("{:x}",Sha256::digest(text.as_bytes())),"msaa":msaa}))
            })();
            CoUninitialize();
            result
        })();
        result.unwrap_or_else(|e| serde_json::json!({"error":format!("{:08x}",e.code().0 as u32)}))
    }).join().unwrap_or_else(|_| serde_json::json!({"error":"selectionWorker"}))
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
    if std::env::args().nth(1).as_deref()==Some("inspect-product") {
        let process=std::env::args().nth(2).and_then(|v|v.parse().ok()).unwrap_or(0);
        println!("{}",product_surface(process));
        return;
    }
    unsafe {
        let composition = std::env::args().nth(1).as_deref() == Some("composition");
        let mut tsf_manager = None;
        let mut rich_library = None;
        use windows::Win32::System::Com::{
            COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
        };
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
        if composition {
            use windows::Win32::System::LibraryLoader::*;
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
        let mut injected_keys = Vec::new();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_APP + 7 {
                let value = r.recv().unwrap();
                if value["action"] == "exit" {
                    let _ = DestroyWindow(top);
                    PostQuitMessage(0);
                    let _ = ok.send(());
                    break;
                }
                if value["action"] == "inspect-context" {
                    println!("{}", context_status());
                    io::stdout().flush().unwrap();
                    let _ = ok.send(());
                    continue;
                }
                if value["action"] == "inspect-selection" {
                    let process = value["process"]
                        .as_u64()
                        .and_then(|p| u32::try_from(p).ok())
                        .unwrap_or(0);
                    let mut foreground = 0;
                    GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut foreground));
                    let evidence = if process != 0 && foreground == process {
                        selection_evidence(process)
                    } else {
                        serde_json::json!({"error":"ownedForegroundMismatch"})
                    };
                    println!("{evidence}");
                    io::stdout().flush().unwrap();
                    let _ = ok.send(());
                    continue;
                }
                if value["action"] == "inspect-focus" {
                    let process = value["process"]
                        .as_u64()
                        .and_then(|p| u32::try_from(p).ok())
                        .unwrap_or(0);
                    println!("{}", focus_metadata(process));
                    io::stdout().flush().unwrap();
                    let _ = ok.send(());
                    continue;
                }
                if value["action"]=="inspect-product" {
                    let process=value["process"].as_u64().and_then(|p|u32::try_from(p).ok()).unwrap_or(0);
                    println!("{}",product_surface(process));
                    io::stdout().flush().unwrap();let _=ok.send(());continue;
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
                if value["action"] == "caret-home" && GetForegroundWindow() == top {
                    // EM_SETSEL can be ignored while TSF owns a composition. Exercise
                    // the editor's normal caret navigation and assert its real selection.
                    message(editor, WM_KEYDOWN, WPARAM(VK_HOME.0 as usize), LPARAM(0));
                    message(editor, WM_KEYUP, WPARAM(VK_HOME.0 as usize), LPARAM(0));
                }
                if value["action"] == "type"
                    && let Some(text) = value["text"].as_str()
                {
                    // Test-only foreign edit, delivered to this owned target's message loop.
                    for unit in text.encode_utf16() {
                        message(editor, WM_CHAR, WPARAM(unit as usize), LPARAM(0));
                    }
                }
                if value["action"] == "fixture-key" {
                    let mut pid = 0;
                    GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
                    let key = match value["key"].as_str() {
                        Some("AltRight") => Some(VK_RMENU),
                        Some("Backspace") => Some(VK_BACK),
                        Some("Delete") => Some(VK_DELETE),
                        _ => None,
                    };
                    if let Some(key) = key {
                        let down = value["down"].as_bool() == Some(true);
                        let allowed = if down {
                            value["process"].as_u64() == Some(pid as u64)
                        } else {
                            injected_keys.contains(&key)
                        };
                        let flags = if value["down"].as_bool() == Some(true) {
                            KEYBD_EVENT_FLAGS(0)
                        } else {
                            KEYEVENTF_KEYUP
                        } | if key == VK_RMENU || key == VK_DELETE {
                            KEYEVENTF_EXTENDEDKEY
                        } else {
                            KEYBD_EVENT_FLAGS(0)
                        };
                        let event = INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wVk: key,
                                    dwFlags: flags,
                                    dwExtraInfo: 0x53595754,
                                    ..Default::default()
                                },
                            },
                        };
                        if allowed && SendInput(&[event], std::mem::size_of::<INPUT>() as i32) == 1
                        {
                            if down && !injected_keys.contains(&key) {
                                injected_keys.push(key);
                            }
                            if !down {
                                injected_keys.retain(|held| *held != key);
                            }
                        }
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
        let mut release_failed = false;
        for key in injected_keys {
            let event = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: key,
                        dwFlags: KEYEVENTF_KEYUP
                            | if key == VK_RMENU || key == VK_DELETE {
                                KEYEVENTF_EXTENDEDKEY
                            } else {
                                KEYBD_EVENT_FLAGS(0)
                            },
                        dwExtraInfo: 0x53595754,
                        ..Default::default()
                    },
                },
            };
            if SendInput(&[event], std::mem::size_of::<INPUT>() as i32) != 1 {
                eprintln!("owned fixture key release failed");
                release_failed = true;
            }
        }
        if let Some(manager) = tsf_manager {
            let _ = manager.Deactivate();
            drop(manager);
        }
        if let Some(library) = rich_library {
            let _ = FreeLibrary(library);
        }
        CoUninitialize();
        if release_failed {
            std::process::exit(1);
        }
    }
}
