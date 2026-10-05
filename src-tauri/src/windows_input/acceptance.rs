//! Explicit desktop test, against a process created here. Restores the previous clipboard only while owned.
use super::{native::*, *};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    time::Duration,
};
use windows::Win32::{Foundation::*, System::DataExchange::*, UI::WindowsAndMessaging::*};
// Clipboard ownership belongs to the caller process, not the external editor fixture.
struct ClipboardOwner {
    window: usize,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl ClipboardOwner {
    fn new() -> Result<Self, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let thread = std::thread::spawn(move || unsafe {
            let window = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!("Sayweave input test clipboard"),
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
            let Ok(window) = window else {
                let _ = tx.send(Err("clipboard window"));
                return;
            };
            let _ = tx.send(Ok(window.0 as usize));
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
                if msg.message == WM_APP + 17 {
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let _ = DestroyWindow(window);
        });
        Ok(Self {
            window: rx
                .recv()
                .map_err(|_| "clipboard owner")?
                .map_err(str::to_string)?,
            thread: Some(thread),
        })
    }
}
impl Drop for ClipboardOwner {
    fn drop(&mut self) {
        unsafe {
            let _ = PostMessageW(
                Some(HWND(self.window as *mut _)),
                WM_APP + 17,
                WPARAM(0),
                LPARAM(0),
            );
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
struct Fixture {
    child: Child,
    window: usize,
    input: ChildStdin,
    output: std::sync::mpsc::Receiver<String>,
}
impl Fixture {
    fn new(exe: &std::path::Path) -> Result<Self, String> {
        let mut child = Command::new(exe)
            .env("SAYWEAVE_INPUT_ACCEPTANCE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
        let input = child.stdin.take().unwrap();
        let out = child.stdout.take().unwrap();
        let (t, output) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else {
                    break;
                };
                if t.send(line).is_err() {
                    break;
                }
            }
        });
        unsafe {
            let _ = AllowSetForegroundWindow(child.id());
        }
        let mut fixture = Self {
            child,
            window: 0,
            input,
            output,
        };
        fixture.window = fixture.read()?["window"].as_u64().ok_or("missing window")? as usize;
        Ok(fixture)
    }
    fn read(&self) -> Result<serde_json::Value, String> {
        let line = self
            .output
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| "fixture timeout".to_string())?;
        serde_json::from_str(&line).map_err(|e| e.to_string())
    }
    fn command(&mut self, value: serde_json::Value) -> Result<serde_json::Value, String> {
        writeln!(self.input, "{value}").map_err(|e| e.to_string())?;
        self.input.flush().map_err(|e| e.to_string())?;
        self.read()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = writeln!(self.input, "{{\"action\":\"exit\"}}");
        let _ = self.input.flush();
        for _ in 0..20 {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
struct Restore {
    window: usize,
    backup: Vec<(u32, Vec<u8>)>,
}
impl Drop for Restore {
    fn drop(&mut self) {
        if let Ok(_lock) = open(self.window) {
            unsafe {
                if GetClipboardOwner()
                    .ok()
                    .is_some_and(|h| h.0 as usize == self.window)
                {
                    let _ = install(&self.backup);
                }
            }
        }
    }
}
pub(super) fn run(exe: &std::path::Path) -> Result<(), String> {
    let mut fixture = Fixture::new(exe)?;
    let clipboard_owner = ClipboardOwner::new()?;
    let owner = HWND(clipboard_owner.window as *mut _);
    let previous = {
        let _lock = open(owner.0 as usize).map_err(|e| e.code)?;
        backup().map_err(|e| format!("clipboard protection: {}", e.code))?
    };
    let _restore = Restore {
        window: owner.0 as usize,
        backup: previous,
    };
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    let service = WindowsInputService::new();
    service.set_owner(owner.0 as usize);
    let samples = [
        ("abc", 1, 1, "中文", "a中文bc"),
        ("a😀b", 1, 3, "你好", "a你好b"),
        ("abc", 0, 3, "abc", "abc"),
        ("before", 6, 6, "\nnext", "before\nnext"),
    ];
    for (before, start, end, text, expected) in samples {
        fixture
            .command(serde_json::json!({"action":"focus","text":before,"start":start,"end":end}))?;
        std::thread::sleep(Duration::from_millis(40));
        let id = uuid::Uuid::new_v4().to_string();
        let anchor = WindowsInputService::anchor().ok_or("fixture not foreground")?;
        runtime
            .block_on(service.capture(id.clone(), anchor))
            .map_err(|e| e.code)?;
        let formats = vec![
            (13, text_bytes("clipboard before")),
            (format("HTML Format"), b"Version:1.0\r\nfragment\0".to_vec()),
            (format("Rich Text Format"), b"{\\rtf1 retained}\0".to_vec()),
            (
                format("Chromium internal source URL"),
                b"https://fixture.invalid/clipboard\0".to_vec(),
            ),
            (
                format("Chromium internal source RFH token"),
                vec![0, 255, 11, 38, 42, 9, 0, 17, 128, 3, 7, 99, 0, 4, 8, 12],
            ),
            (
                format("PNG"),
                vec![
                    137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0,
                    0, 1, 8, 4, 0, 0, 0, 181, 28, 12, 2, 0, 0, 0, 11, 73, 68, 65, 84, 120, 218, 99,
                    252, 255, 31, 0, 3, 3, 2, 0, 239, 154, 233, 191, 0, 0, 0, 0, 73, 69, 78, 68,
                    174, 66, 96, 130,
                ],
            ),
            (8, {
                let mut dib = vec![0u8; 44];
                dib[0] = 40;
                dib[4] = 1;
                dib[8] = 1;
                dib[12] = 1;
                dib[14] = 24;
                dib
            }),
            (15, {
                let mut files = vec![0u8; 20];
                files[0] = 20;
                files[16] = 1;
                files.extend(
                    "C:\\Sayweave-fixture.txt\0\0"
                        .encode_utf16()
                        .flat_map(u16::to_ne_bytes),
                );
                files
            }),
        ];
        let formats = {
            let _lock = open(owner.0 as usize).map_err(|e| e.code)?;
            install(&formats).map_err(|e| e.code)?;
            backup().map_err(|e| e.code)?
        };
        let result =
            runtime.block_on(service.deliver(id, text.into(), Arc::new(DeliveryGate::default())));
        if result.state != "confirmed" || result.restore_error {
            let actual = fixture.command(serde_json::json!({"action":"read"}))?;
            return Err(format!("delivery {result:?} fixture {actual}"));
        }
        let actual = fixture.command(serde_json::json!({"action":"read"}))?;
        if normalize(actual["text"].as_str().unwrap_or("")) != expected {
            return Err("actual target mismatch".into());
        }
        {
            let _lock = open(owner.0 as usize).map_err(|e| e.code)?;
            let restored = backup().map_err(|e| e.code)?;
            for (kind, data) in &formats {
                if !restored.iter().any(|(k, d)| k == kind && d == data) {
                    return Err("clipboard format lost".into());
                }
            }
        }
        println!(
            "SAYWEAVE_INPUT_ACCEPTANCE insert_replace_unicode_readback: confirmed {}ms",
            result.elapsed_ms
        );
    }
    // Allocation fails before EmptyClipboard, so explicit copy preserves all original formats.
    FAIL_ALLOCATION.store(true, Ordering::Release);
    if runtime.block_on(service.copy("new".into())).is_ok() {
        return Err("allocation fault ignored".into());
    }
    {
        let _lock = open(owner.0 as usize).map_err(|e| e.code)?;
        if !backup()
            .map_err(|e| e.code)?
            .iter()
            .any(|(kind, _)| *kind == format("PNG"))
        {
            return Err("copy failure destroyed formats".into());
        }
    }
    // Four temporary formats succeed; the first restoration format fails once.
    for fault in 0..3 {
        let user_copy = fault == 2;
        fixture.command(serde_json::json!({"action":"focus","text":"safe","start":4,"end":4}))?;
        let id = uuid::Uuid::new_v4().to_string();
        runtime
            .block_on(service.capture(
                id.clone(),
                WindowsInputService::anchor().ok_or("fixture target")?,
            ))
            .map_err(|e| e.code)?;
        FAIL_PUT_AT.store(5, Ordering::Release);
        FAIL_PUT_CHAIN.store(if fault == 1 { 32 } else { 0 }, Ordering::Release);
        let result = runtime.block_on(service.deliver(
            id,
            " text".into(),
            Arc::new(DeliveryGate::default()),
        ));
        if result.state != "confirmed" || !result.restore_error {
            let actual = fixture.command(serde_json::json!({"action":"read"}))?;
            return Err(format!(
                "restore fault {fault} {result:?} owned target {actual}"
            ));
        }
        FAIL_PUT_LEFT.store(0, Ordering::Release);
        if user_copy {
            let _lock = open(owner.0 as usize).map_err(|e| e.code)?;
            install(&[(13, text_bytes("user new clipboard"))]).map_err(|e| e.code)?;
        }
        runtime.block_on(service.restore()).map_err(|e| e.code)?;
        let _lock = open(owner.0 as usize).map_err(|e| e.code)?;
        let restored = backup().map_err(|e| e.code)?;
        if user_copy {
            if !restored.iter().any(|(kind, data)| {
                *kind == 13 && data.starts_with(&text_bytes("user new clipboard"))
            }) {
                return Err("recovery overwrote user copy".into());
            }
        } else if !restored.iter().any(|(kind, _)| *kind == format("PNG")) {
            return Err("restore retry lost backup".into());
        }
    }
    println!("SAYWEAVE_INPUT_ACCEPTANCE allocation_restore_fault_user_copy: PASS");
    fixture.command(serde_json::json!({"action":"focus","text":"safe","start":4,"end":4}))?;
    let id = uuid::Uuid::new_v4().to_string();
    runtime
        .block_on(service.capture(
            id.clone(),
            WindowsInputService::anchor().ok_or("fixture target")?,
        ))
        .map_err(|e| e.code)?;
    FAIL_PUT_AT.store(1, Ordering::Release);
    FAIL_PUT_CHAIN.store(32, Ordering::Release);
    let failed_write = runtime.block_on(service.deliver(
        id,
        "never pasted".into(),
        Arc::new(DeliveryGate::default()),
    ));
    FAIL_PUT_LEFT.store(0, Ordering::Release);
    if failed_write.state != "copy" || !failed_write.restore_error {
        return Err(format!("missing restore action {failed_write:?}"));
    }
    runtime.block_on(service.restore()).map_err(|e| e.code)?;
    if fixture.command(serde_json::json!({"action":"read"}))?["text"] != "safe" {
        return Err("failed temporary write pasted".into());
    }
    println!("SAYWEAVE_INPUT_ACCEPTANCE temporary_write_restore_fault: PASS");
    fixture.command(serde_json::json!({"action":"focus","text":"safe","start":4,"end":4}))?;
    let id = uuid::Uuid::new_v4().to_string();
    runtime
        .block_on(service.capture(
            id.clone(),
            WindowsInputService::anchor().ok_or("fixture target")?,
        ))
        .map_err(|e| e.code)?;
    fixture.command(serde_json::json!({"action":"delayClipboard"}))?;
    let at = std::time::Instant::now();
    let result = runtime.block_on(service.deliver(
        id,
        "late must not input".into(),
        Arc::new(DeliveryGate::default()),
    ));
    if result.state != "copy" || at.elapsed() > Duration::from_millis(800) {
        return Err(format!("delayed owner wait {result:?}"));
    }
    std::thread::sleep(Duration::from_millis(1300));
    if fixture.command(serde_json::json!({"action":"read"}))?["text"] != "safe" {
        return Err("late clipboard read delivered text".into());
    }
    println!("SAYWEAVE_INPUT_ACCEPTANCE delayed_owner_bounded_no_late_input: PASS");
    fixture.command(serde_json::json!({"action":"focus","text":"safe","start":4,"end":4}))?;
    let id = uuid::Uuid::new_v4().to_string();
    runtime
        .block_on(service.capture(
            id.clone(),
            WindowsInputService::anchor().ok_or("fixture target")?,
        ))
        .map_err(|e| e.code)?;
    {
        let _lock = open(owner.0 as usize).map_err(|e| e.code)?;
        install(&[
            (13, text_bytes("retained")),
            (format("Sayweave.UnknownFixture"), vec![1, 2, 3, 4]),
        ])
        .map_err(|e| e.code)?;
    }
    let sequence = unsafe { GetClipboardSequenceNumber() };
    let result = runtime.block_on(service.deliver(
        id,
        "never pasted".into(),
        Arc::new(DeliveryGate::default()),
    ));
    if result.state != "copy"
        || result.error.as_deref() != Some("clipboardUnsupported")
        || unsafe { GetClipboardSequenceNumber() } != sequence
        || fixture.command(serde_json::json!({"action":"read"}))?["text"] != "safe"
    {
        return Err("unknown clipboard format was changed or delivered".into());
    }
    {
        let _lock = open(owner.0 as usize).map_err(|e| e.code)?;
        install(&[(13, text_bytes("retained"))]).map_err(|e| e.code)?;
    }
    println!("SAYWEAVE_INPUT_ACCEPTANCE unknown_format_preserved_no_input: PASS");
    fixture.command(serde_json::json!({"action":"focus","text":"unchanged","start":9,"end":9}))?;
    let id = uuid::Uuid::new_v4().to_string();
    runtime
        .block_on(service.capture(
            id.clone(),
            WindowsInputService::anchor().ok_or("fixture target")?,
        ))
        .map_err(|e| e.code)?;
    fixture
        .command(serde_json::json!({"action":"focus","text":"user changed","start":0,"end":0}))?;
    let result = runtime.block_on(service.deliver(
        id,
        "do not insert".into(),
        Arc::new(DeliveryGate::default()),
    ));
    if result.state != "copy" {
        return Err("changed target wrote text".into());
    }
    fixture.command(serde_json::json!({"action":"readonly"}))?;
    let result = runtime.block_on(service.capture(
        uuid::Uuid::new_v4().to_string(),
        WindowsInputService::anchor().ok_or("fixture target")?,
    ));
    if !result.is_err_and(|e| e.code == "targetReadonly") {
        return Err("readonly accepted".into());
    }
    service.shutdown();
    runtime.block_on(service.drain());
    println!("SAYWEAVE_INPUT_ACCEPTANCE changed_readonly_shutdown: PASS");
    Ok(())
}
