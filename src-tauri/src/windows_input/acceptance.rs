//! Explicit desktop test against owned TSF targets. Normal input leaves the clipboard untouched.
use super::{native::*, *};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use windows::Win32::{Foundation::*, System::DataExchange::*, UI::WindowsAndMessaging::*};
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
    let input = WindowsInputService::new();
    struct InputLease(WindowsInputService);
    impl Drop for InputLease {
        fn drop(&mut self) {
            self.0.shutdown();
        }
    }
    let _input_lease = InputLease(input.clone());
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
        runtime
            .block_on(draft.replace(1, prefix.into()))
            .map_err(|e| e.code)?;
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
        runtime
            .block_on(draft.replace(2, text.into()))
            .map_err(|e| e.code)?;
        if runtime
            .block_on(draft.replace(1, "STALE".into()))
            .err()
            .is_none_or(|e| e.code != "staleSnapshot")
        {
            return Err("old source snapshot accepted".into());
        }
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let value = fixture.command(serde_json::json!({"action":"read"}))?;
            if value["text"]
                .as_str()
                .map(|v| v.replace("\r\n", "\n"))
                .as_deref()
                == Some(expected)
            {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(
                    "second live update did not reach same target range before finish".into(),
                );
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let delivery = runtime.block_on(draft.finish(text.into(), Finish::Commit));
        if delivery.verification != Verification::Unavailable
            || delivery.submission != Submission::Submitted
            || delivery.error.as_deref() != Some("inputUnconfirmed")
        {
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
    // A fully settled prefix already ended its composition. Finishing must
    // release resources, not require continued ownership of an empty caret.
    for (move_caret, resume_tail, cancel_after) in [
        (false, false, false),
        (true, false, false),
        (false, true, false),
        (false, false, true),
    ] {
        fixture.command(serde_json::json!({"action":"focus","text":"前后","start":1,"end":1}))?;
        thread::sleep(Duration::from_millis(60));
        let id = uuid::Uuid::new_v4().to_string();
        let anchor = WindowsInputService::anchor().ok_or("settled fixture not foreground")?;
        bind_watch(&id, &anchor, None).map_err(|e| e.code)?;
        let gate = Arc::new(DeliveryGate::default());
        let draft = runtime
            .block_on(Draft::begin(id.clone(), 1, anchor, gate.clone()))
            .map_err(|e| e.code)?;
        runtime
            .block_on(draft.replace(1, "今天10月8号，不对，今天10月7号。".into()))
            .map_err(|e| e.code)?;
        let settled = runtime.block_on(draft.settle(
            2,
            "今天10月7号。".into(),
            String::new(),
            Arc::new(AtomicBool::new(true)),
        ));
        if settled.prefix != sayweave_text_service::wire::Mutation::Changed
            || !settled.tail_owned
            || settled.error.is_some()
        {
            return Err(format!("empty-tail settlement failed: {:?}", settled.error));
        }
        thread::sleep(Duration::from_millis(80));
        if move_caret {
            fixture.command(serde_json::json!({"action":"caret-home"}))?;
        }
        let tail = if resume_tail { "继续输入🙂" } else { "" };
        if resume_tail {
            runtime
                .block_on(draft.replace(3, tail.into()))
                .map_err(|e| e.code)?;
        }
        if cancel_after {
            gate.cancel();
        }
        let outcome = runtime.block_on(draft.finish_receipt(tail.into(), Finish::Commit));
        let after = fixture.command(serde_json::json!({"action":"read"}))?;
        if outcome.committed != resume_tail
            || outcome.delivery.error.as_deref()
                != Some(if cancel_after {
                    "cancelled"
                } else {
                    "inputUnconfirmed"
                })
            || after["text"] != format!("前今天10月7号。{tail}后")
            || move_caret && (after["start"] != 0 || after["end"] != 0)
        {
            return Err(format!(
                "settled prefix finalization failed: moved={move_caret} resume={resume_tail} cancel={cancel_after} committed={} error={:?} actual={}",
                outcome.committed, outcome.delivery.error, after
            ));
        }
        release_watch(&id);
        println!(
            "SAYWEAVE_SETTLED_PREFIX_FINISH_PASS moved_caret={move_caret} resumed={resume_tail} cancelled={cancel_after} actual_text=true no_repeated_input=true"
        );
    }
    // Editor caret navigation is not keyboard-hook evidence in this fixture. The target
    // range must independently reject a later revision at the changed position.
    {
        fixture
            .command(serde_json::json!({"action":"focus","text":"protected","start":9,"end":9}))?;
        let id = uuid::Uuid::new_v4().to_string();
        let anchor = WindowsInputService::anchor().ok_or("caret fixture target")?;
        bind_watch(&id, &anchor, None).map_err(|e| e.code)?;
        let draft = runtime
            .block_on(Draft::begin(
                id.clone(),
                1,
                anchor,
                Arc::new(DeliveryGate::default()),
            ))
            .map_err(|e| e.code)?;
        runtime
            .block_on(draft.replace(1, "正在识别".into()))
            .map_err(|e| e.code)?;
        let until = Instant::now() + Duration::from_secs(3);
        while fixture.command(serde_json::json!({"action":"read"}))?["text"] != "protected正在识别"
        {
            if Instant::now() >= until {
                return Err("caret setup partial absent".into());
            }
            thread::sleep(Duration::from_millis(20));
        }
        let moved = fixture.command(serde_json::json!({"action":"caret-home"}))?;
        println!(
            "CARET_SELECTION_OBSERVED start={} end={}",
            moved["start"], moved["end"]
        );
        if moved["start"] != 0 || moved["end"] != 0 {
            return Err("fixture caret did not move".into());
        }
        let _ = runtime.block_on(draft.replace(2, "不应覆盖".into()));
        let delivery = runtime.block_on(draft.finish("不应覆盖".into(), Finish::Commit));
        let after = fixture.command(serde_json::json!({"action":"read"}))?;
        if delivery.state == "confirmed" || after["text"] != "protected正在识别" {
            return Err(format!(
                "caret protection failed: state={} error={:?}",
                delivery.state, delivery.error
            ));
        }
        release_watch(&id);
        println!(
            "SAYWEAVE_COMPOSITION_CARET_PASS actual_selection_changed=true no_late_write=true"
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
            flow.clone(),
            cancel.clone(),
            None,
            None,
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
            ..NativeTranscript::default()
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
            ..NativeTranscript::default()
        });
        let latest = NativeTranscript {
            task_id: id.clone(),
            turn: 2,
            sequence: 3,
            text: "新🙂".into(),
            ..NativeTranscript::default()
        };
        manager.update(&latest);
        flow.request_finish();
        cancel.send_replace(1);
        let delivery = runtime.block_on(manager.finish(&latest, Finish::Commit));
        runtime.block_on(actor).map_err(|e| e.to_string())?;
        let actual = fixture.command(serde_json::json!({"action":"read"}))?;
        if delivery.verification != Verification::Unavailable
            || delivery.submission != Submission::Submitted
            || delivery.error.as_deref() != Some("inputUnconfirmed")
            || actual["text"] != "a你好X新🙂bc"
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
    runtime
        .block_on(draft.replace(1, "已有草稿".into()))
        .map_err(|e| e.code)?;
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
pub(super) fn run(exe: &std::path::Path) -> Result<(), String> {
    // The product uses TSF. No automatic paste or clipboard mutation is an input fallback.
    let sequence = unsafe { GetClipboardSequenceNumber() };
    composition_run(exe)?;
    if unsafe { GetClipboardSequenceNumber() } != sequence {
        return Err("clipboard changed during composition acceptance".into());
    }
    Ok(())
}

/// Scripted operator for a strictly owned target, using the registered production
/// host and its optional receipt sink. It never directly loads a component DLL.
pub(super) fn observation_run(process: u32) -> Result<(), String> {
    use super::composition::{Draft, Finish, ObservationSink, stop_observer};
    use std::sync::atomic::{AtomicBool, AtomicU64};
    if std::env::var("SAYWEAVE_TSF_OWNED_PID")
        .ok()
        .and_then(|p| p.parse().ok())
        != Some(process)
        || process == std::process::id()
    {
        return Err("owned observation target required".into());
    }
    let input = WindowsInputService::new();
    let _evidence = EvidenceLease::new()?;
    let runtime = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    let (send, evidence) = std::sync::mpsc::channel();
    let mut draft = None;
    let mut task = String::new();
    let result = (|| {
        println!("{}", serde_json::json!({"ready":true}));
        std::io::stdout().flush().map_err(|e| e.to_string())?;
        for line in std::io::stdin().lock().lines() {
            let value: serde_json::Value = serde_json::from_str(&line.map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            let action = value["action"].as_str().ok_or("action required")?;
            let mut response = serde_json::json!({"action":action});
            match action {
                "begin" => {
                    let anchor =
                        WindowsInputService::anchor().ok_or("owned foreground required")?;
                    if anchor.process != process || draft.is_some() {
                        return Err("owned foreground/session mismatch".into());
                    }
                    let application = super::application::resolve(&anchor);
                    task = uuid::Uuid::new_v4().to_string();
                    bind_watch(&task, &anchor, None).map_err(|e| e.code)?;
                    let current = runtime
                        .block_on(Draft::begin(
                            task.clone(),
                            1,
                            anchor,
                            Arc::new(DeliveryGate::default()),
                        ))
                        .map_err(|e| e.code)?;
                    let sink = Arc::new(ObservationSink {
                        snapshot: crate::contracts::language::Snapshot {
                            application: application.clone(),
                            settings: crate::contracts::language::Settings {
                                enabled: true,
                                revision: 0,
                            },
                            profile_version: 0,
                            shared_version: 0,
                            preferences: vec![],
                        },
                        permit: Arc::new(AtomicBool::new(true)),
                        current_revision: Arc::new(AtomicU64::new(0)),
                        emit: Arc::new({
                            let send = send.clone();
                            move |value| {
                                let _ = send.send(value);
                            }
                        }),
                    });
                    response["observing"] = runtime.block_on(current.observe(sink)).into();
                    response["application"] =
                        serde_json::to_value(application).map_err(|e| e.to_string())?;
                    draft = Some(current);
                }
                "replace" => {
                    runtime
                        .block_on(draft.as_ref().ok_or("begin first")?.replace_with_source(
                            value["sequence"].as_u64().ok_or("sequence required")?,
                            value["text"].as_str().ok_or("text required")?.into(),
                            Some(value["raw"].as_str().ok_or("raw required")?.into()),
                        ))
                        .map_err(|e| e.code)?;
                }
                "commit" | "handoff" => {
                    let outcome = runtime.block_on(
                        draft
                            .as_ref()
                            .ok_or("begin first")?
                            .finish_receipt_with_source(
                                value["text"].as_str().unwrap_or("").into(),
                                if action == "commit" {
                                    Finish::Commit
                                } else {
                                    Finish::Handoff("editing".into())
                                },
                                value["raw"].as_str().unwrap_or("").into(),
                            ),
                    );
                    response["committed"] = outcome.committed.into();
                    response["handedOff"] = outcome.handed_off.into();
                    response["delivery"] =
                        serde_json::to_value(outcome.delivery).map_err(|e| e.to_string())?;
                }
                "drain" => {}
                "stop" => {
                    runtime.block_on(stop_observer()).map_err(|e| e.code)?;
                }
                "exit" => break,
                _ => return Err("unknown observation fixture action".into()),
            }
            response["edits"] = serde_json::to_value(evidence.try_iter().collect::<Vec<_>>())
                .map_err(|e| e.to_string())?;
            println!("{response}");
            std::io::stdout().flush().map_err(|e| e.to_string())?;
        }
        Ok(())
    })();
    if let Some(draft) = draft {
        let _ = runtime.block_on(draft.finish(String::new(), Finish::Abort));
    }
    let _ = runtime.block_on(stop_observer());
    release_watch(&task);
    input.shutdown();
    result
}
