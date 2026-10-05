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
struct EvidenceLease {
    thread: Option<std::thread::JoinHandle<()>>,
    id: u32,
}
unsafe extern "system" fn evidence_event(
    _: windows::Win32::UI::Accessibility::HWINEVENTHOOK,
    event: u32,
    window: HWND,
    object: i32,
    _: i32,
    _: u32,
    time: u32,
) {
    super::native::observe(event, window.0 as usize, object, time);
}
impl EvidenceLease {
    fn new() -> Result<Self, String> {
        let (ready, value) = std::sync::mpsc::channel::<Result<u32, String>>();
        let thread = std::thread::spawn(move || unsafe {
            use windows::Win32::{System::Threading::GetCurrentThreadId, UI::Accessibility::*};
            let hooks = [
                EVENT_SYSTEM_FOREGROUND,
                EVENT_OBJECT_FOCUS,
                EVENT_OBJECT_DESTROY,
                EVENT_OBJECT_VALUECHANGE,
                EVENT_OBJECT_TEXTSELECTIONCHANGED,
            ]
            .map(|event| {
                SetWinEventHook(
                    event,
                    event,
                    None,
                    Some(evidence_event),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                )
            });
            let mut message = MSG::default();
            let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
            if hooks.iter().any(|h| h.0.is_null()) {
                for hook in hooks {
                    let _ = UnhookWinEvent(hook);
                }
                let _ = ready.send(Err("event hook unavailable".into()));
                return;
            }
            let _ = ready.send(Ok(GetCurrentThreadId()));
            while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
            for hook in hooks {
                let _ = UnhookWinEvent(hook);
            }
        });
        Ok(Self {
            thread: Some(thread),
            id: value.recv().map_err(|_| "event hook worker")??,
        })
    }
}
impl Drop for EvidenceLease {
    fn drop(&mut self) {
        unsafe {
            let _ = PostThreadMessageW(self.id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Fixture {
    fn new(exe: &std::path::Path) -> Result<Self, String> {
        Self::open(exe, false)
    }
    fn open(exe: &std::path::Path, composition: bool) -> Result<Self, String> {
        let mut command = Command::new(exe);
        if composition {
            command.arg("composition");
        }
        let mut child = command
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
/// Production host against an owned TSF editor. No clipboard reads/writes or supplier requests.
pub(super) fn composition_run(exe: &std::path::Path) -> Result<(), String> {
    use super::composition::{Draft, Finish};
    let mut fixture = Fixture::open(exe, true)?;
    let _evidence = EvidenceLease::new()?;
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    for (before, start, end, prefix, text, expected) in [
        ("abc", 1, 1, "中", "中文，🙂", "a中文，🙂bc"),
        ("前 被选🙂 后", 2, 6, "替", "替换🙂", "前 替换🙂 后"),
        ("", 0, 0, "第一行", "第一行\n第二行🙂", "第一行\n第二行🙂"),
    ] {
        fixture
            .command(serde_json::json!({"action":"focus","text":before,"start":start,"end":end}))?;
        std::thread::sleep(Duration::from_millis(60));
        let id = uuid::Uuid::new_v4().to_string();
        let anchor = WindowsInputService::anchor().ok_or("fixture not foreground")?;
        bind_watch(&id, &anchor, None).map_err(|e| e.code)?;
        let draft = runtime
            .block_on(Draft::begin(
                id.clone(),
                1,
                anchor,
                Arc::new(DeliveryGate::default()),
            ))
            .map_err(|e| e.code)?;
        draft.update(1, 1, prefix);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let value = fixture.command(serde_json::json!({"action":"read"}))?;
            if value["text"].as_str().is_some_and(|v| v.contains(prefix)) {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err("live prefix did not reach actual target".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        draft.update(1, 2, text);
        draft.update(1, 1, "STALE");
        let delivery = runtime.block_on(draft.finish(text.into(), Finish::Commit));
        if delivery.verification != Verification::Confirmed {
            return Err(format!("composition commit failed: {:?}", delivery.error));
        }
        let actual = fixture.command(serde_json::json!({"action":"read"}))?;
        if actual["text"]
            .as_str()
            .map(|t| t.replace("\r\n", "\n"))
            .as_deref()
            != Some(expected)
        {
            return Err("composition actual target assertion failed".into());
        }
        release_watch(&id);
        println!(
            "SAYWEAVE_COMPOSITION_SAMPLE_PASS actual_prefix=true cumulative=true final_single_copy=true"
        );
    }
    // Exact production turn manager, actual TSF target text. Editing evidence is
    // explicitly controlled here; this does not stand in for physical-key/Codex acceptance.
    {
        use crate::{
            dictation::{Presentation, drafts::SessionDraft},
            speech::{NativeTranscript, continuous::Control},
        };
        fixture.command(serde_json::json!({"action":"focus","text":"abc","start":1,"end":1}))?;
        std::thread::sleep(Duration::from_millis(60));
        let id = uuid::Uuid::new_v4().to_string();
        let anchor = WindowsInputService::anchor().ok_or("turn fixture target")?;
        bind_watch(&id, &anchor, None).map_err(|e| e.code)?;
        let gate = Arc::new(DeliveryGate::default());
        let draft = runtime
            .block_on(Draft::begin(id.clone(), 1, anchor.clone(), gate.clone()))
            .map_err(|e| e.code)?;
        let flow = Control::new(crate::input_state::activity_evidence());
        flow.resume(1, anchor.activity);
        let (cancel, _) = tokio::sync::watch::channel(0);
        let manager = SessionDraft::new(
            id.clone(),
            anchor,
            draft,
            gate,
            flow.clone(),
            cancel.clone(),
        );
        manager.activate();
        let ongoing = manager.clone();
        let actor = runtime.spawn(async move {
            ongoing.maintain(|_| {}).await;
        });
        manager.update(&NativeTranscript {
            task_id: id.clone(),
            turn: 1,
            sequence: 1,
            text: "你好".into(),
        });
        let until = std::time::Instant::now() + Duration::from_secs(3);
        while fixture.command(serde_json::json!({"action":"read"}))?["text"] != "a你好bc" {
            if std::time::Instant::now() >= until {
                return Err("turn1 actual text missing".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        crate::input_state::edited();
        fixture.command(serde_json::json!({"action":"type","text":"X"}))?;
        let until = std::time::Instant::now() + Duration::from_secs(4);
        while manager.presentation() != Presentation::Resuming {
            if *cancel.borrow() > 0 || std::time::Instant::now() >= until {
                actor.abort();
                return Err("production turn manager did not resume".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        manager.update(&NativeTranscript {
            task_id: id.clone(),
            turn: 1,
            sequence: 2,
            text: "旧轮禁止覆盖".into(),
        });
        let latest = NativeTranscript {
            task_id: id.clone(),
            turn: 2,
            sequence: 3,
            text: "新🙂".into(),
        };
        manager.update(&latest);
        flow.request_finish();
        cancel.send_replace(1);
        let delivery = runtime.block_on(manager.finish(&latest, Finish::Commit));
        runtime.block_on(actor).map_err(|e| e.to_string())?;
        let actual = fixture.command(serde_json::json!({"action":"read"}))?;
        if delivery.verification != Verification::Confirmed || actual["text"] != "a你好X新🙂bc"
        {
            return Err(format!(
                "handoff owned-fixture assertion failed delivery={:?} actual={}",
                delivery.error, actual["text"]
            ));
        }
        release_watch(&id);
        println!(
            "SAYWEAVE_COMPOSITION_TURNS_PASS controlled_edit=true actual_handoff=true resumed_range=true old_turn_ignored=true final_single_copy=true"
        );
    }
    // Actual foreground notifications must remain sticky after switching back. This is
    // deliberately a second owned process, not a fabricated WinEvent or personal app.
    fixture.command(serde_json::json!({"action":"focus","text":"","start":0,"end":0}))?;
    std::thread::sleep(Duration::from_millis(60));
    let id = uuid::Uuid::new_v4().to_string();
    let anchor = WindowsInputService::anchor().ok_or("fixture not foreground")?;
    bind_watch(&id, &anchor, None).map_err(|e| e.code)?;
    let draft = runtime
        .block_on(Draft::begin(
            id.clone(),
            1,
            anchor,
            Arc::new(DeliveryGate::default()),
        ))
        .map_err(|e| e.code)?;
    draft.update(1, 1, "已有草稿");
    let until = std::time::Instant::now() + Duration::from_secs(3);
    while fixture.command(serde_json::json!({"action":"read"}))?["text"] != "已有草稿" {
        if std::time::Instant::now() >= until {
            return Err("negative fixture prefix missing".into());
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    {
        let mut other = Fixture::open(exe, true)?;
        other.command(serde_json::json!({"action":"focus","text":"other","start":5,"end":5}))?;
        std::thread::sleep(Duration::from_millis(100));
        fixture.command(serde_json::json!({"action":"focus"}))?;
        std::thread::sleep(Duration::from_millis(100));
        let result = runtime.block_on(draft.finish("禁止追加".into(), Finish::Commit));
        if result.error.as_deref() != Some("targetChanged")
            || fixture.command(serde_json::json!({"action":"read"}))?["text"] != "已有草稿"
            || other.command(serde_json::json!({"action":"read"}))?["text"] != "other"
        {
            return Err("real foreground switch was not protected".into());
        }
    }
    release_watch(&id);
    println!("SAYWEAVE_COMPOSITION_PROTECTION_PASS real_switch_return=true no_wrong_write=true");
    Ok(())
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
