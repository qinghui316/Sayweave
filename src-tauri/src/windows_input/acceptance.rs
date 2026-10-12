//! Explicit desktop test against owned TSF targets. Normal input leaves the clipboard untouched.
use super::{native::*, *};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, ChildStdin, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use windows::Win32::{Foundation::*, System::DataExchange::*, UI::WindowsAndMessaging::*};
static TRACKED_AUXILIARY: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static AUXILIARY_EVENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
struct AuxiliaryEvents(usize);
impl AuxiliaryEvents {
    fn track(window: usize) -> Self {
        TRACKED_AUXILIARY.store(window, std::sync::atomic::Ordering::Release);
        Self(window)
    }
    fn count(&self) -> u64 {
        AUXILIARY_EVENTS.load(std::sync::atomic::Ordering::Acquire)
    }
    fn wait_after(&self, previous: u64) -> Result<(), String> {
        let until = Instant::now() + Duration::from_secs(2);
        while self.count() <= previous {
            if Instant::now() >= until {
                return Err("owned auxiliary foreground event was not received".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }
}
impl Drop for AuxiliaryEvents {
    fn drop(&mut self) {
        let _ = TRACKED_AUXILIARY.compare_exchange(
            self.0,
            0,
            std::sync::atomic::Ordering::AcqRel,
            std::sync::atomic::Ordering::Acquire,
        );
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
    if event == EVENT_SYSTEM_FOREGROUND
        && window.0 as usize == TRACKED_AUXILIARY.load(std::sync::atomic::Ordering::Acquire)
    {
        AUXILIARY_EVENTS.fetch_add(1, std::sync::atomic::Ordering::AcqRel);
    }
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
    // Initialize the provider in this same owned editor before capturing test ranges.
    // First activation may emit selection events; production admission must reject them.
    fixture.command(serde_json::json!({"action":"focus","text":"初始化正文","start":1,"end":1}))?;
    let before = fixture.command(serde_json::json!({"action":"read"}))?;
    let initialization = uuid::Uuid::new_v4().to_string();
    let anchor = WindowsInputService::anchor().ok_or("fixture not foreground")?;
    bind_watch(&initialization, &anchor, None).map_err(|e| e.code)?;
    match runtime.block_on(Draft::begin(
        initialization.clone(),
        1,
        anchor,
        Arc::new(DeliveryGate::default()),
    )) {
        Ok(draft) => {
            let outcome = runtime.block_on(draft.finish(String::new(), Finish::Commit));
            if outcome.restore_error || outcome.submission != Submission::NotSubmitted {
                return Err("owned initialization did not retire without mutation".into());
            }
        }
        Err(error) if error.code == "targetChanged" => {
            println!("SAYWEAVE_EDITOR_INITIALIZATION admission=rejected_before_Begin");
        }
        Err(error) => return Err(error.code),
    }
    release_watch(&initialization);
    let after = fixture.command(serde_json::json!({"action":"read"}))?;
    if !after["foreground"].as_bool().unwrap_or(false)
        || ["text", "start", "end"]
            .iter()
            .any(|key| before[key] != after[key])
    {
        return Err("owned initialization changed body, selection or foreground".into());
    }
    println!("SAYWEAVE_EDITOR_INITIALIZATION body_selection_unchanged=true");
    for (start, end) in [(1, 1), (1, 3)] {
        for mode in [
            Finish::Commit,
            Finish::Abort,
            Finish::Handoff("editing".into()),
        ] {
            fixture.command(serde_json::json!({"action":"readonly","enabled":false}))?;
            fixture.command(
                serde_json::json!({"action":"focus","text":"原有正文","start":start,"end":end}),
            )?;
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
            fixture.command(serde_json::json!({"action":"readonly","enabled":true}))?;
            let result = runtime.block_on(draft.finish(String::new(), mode));
            if result.submission != Submission::NotSubmitted
                || result.restore_error
                || !matches!(
                    result.error.as_deref(),
                    Some("none" | "cancelled" | "editing")
                )
            {
                return Err(format!(
                    "unwritten readonly terminal failed: {:?}",
                    result.error
                ));
            }
            let actual = fixture.command(serde_json::json!({"action":"read"}))?;
            if actual["text"].as_str() != Some("原有正文") {
                return Err("unwritten release changed existing body".into());
            }
            release_watch(&id);
            println!(
                "SAYWEAVE_PREPARED_READONLY_PASS selected={} body_unchanged=true",
                start != end
            );
        }
    }
    fixture.command(serde_json::json!({"action":"readonly","enabled":false}))?;
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
        println!(
            "SAYWEAVE_COMPOSITION_TARGET pid={} phase=first_applied",
            fixture.child.id()
        );
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
        thread::sleep(Duration::from_millis(if resume_tail { 1500 } else { 80 }));
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
            anchor.clone(),
            draft,
            flow.clone(),
            cancel.clone(),
            None,
            None,
        );
        manager.activate();
        let ongoing = manager.clone();
        let mut actor = runtime.spawn(async move {
            ongoing.maintain(|_| {}).await;
        });
        let mut latest = NativeTranscript {
            task_id: id.clone(),
            turn: 1,
            sequence: 1,
            text: "第1轮你好".into(),
            ..NativeTranscript::default()
        };
        let result = (|| -> Result<(), String> {
            let active = || -> Result<(), String> {
                if *cancel.borrow() != 0 || flow.stopping() {
                    return Err(format!(
                        "six-edit Session stopped: cancel={} stopping={}",
                        *cancel.borrow(),
                        flow.stopping()
                    ));
                }
                Ok(())
            };
            let wait_text = |fixture: &mut Fixture, expected: &str| -> Result<(), String> {
                let until = Instant::now() + Duration::from_secs(4);
                loop {
                    active()?;
                    let actual = fixture.command(serde_json::json!({"action":"read"}))?;
                    if actual["foreground"] != true {
                        return Err("six-edit owned target lost foreground".into());
                    }
                    if actual["text"] == expected {
                        return Ok(());
                    }
                    if Instant::now() >= until {
                        return Err(format!(
                            "six-edit complete text mismatch expected={expected:?} actual={actual}"
                        ));
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            };
            let mut edited_prefix = String::new();
            manager.update(&latest);
            wait_text(&mut fixture, "a第1轮你好bc")?;
            let auxiliary = fixture.command(serde_json::json!({"action":"auxiliary-info"}))?;
            if auxiliary["foreground"] != true
                || auxiliary["auxiliary"]["error"] != serde_json::Value::Null
                || auxiliary["auxiliary"]["noActivate"] != true
                || auxiliary["auxiliary"]["toolWindow"] != true
                || auxiliary["auxiliary"]["foreground"] != false
                || auxiliary["auxiliary"]["text"] != "owned helper sentinel"
            {
                return Err(format!("nonactivating helper initialization {auxiliary}"));
            }
            let auxiliary_window = auxiliary["auxiliary"]["window"]
                .as_u64()
                .filter(|window| *window != 0 && *window as usize != fixture.window)
                .ok_or("nonactivating helper window missing")?
                as usize;
            let notices = AuxiliaryEvents::track(auxiliary_window);
            for round in 1..=6 {
                active()?;
                let previous = notices.count();
                let notice = fixture.command(serde_json::json!({"action":"auxiliary-notice"}))?;
                if notice["foreground"] != true
                    || notice["auxiliary"]["error"] != serde_json::Value::Null
                    || notice["auxiliary"]["foreground"] != false
                    || notice["auxiliary"]["text"] != "owned helper sentinel"
                {
                    return Err(format!(
                        "synthetic auxiliary notification changed target {notice}"
                    ));
                }
                notices.wait_after(previous)?;
                if !composition_target(&id, &anchor).map_err(|e| e.code)? {
                    return Err("synthetic auxiliary notification blocked original target".into());
                }
                let before = fixture.command(serde_json::json!({"action":"read"}))?;
                let expected_caret =
                    1 + edited_prefix.encode_utf16().count() + latest.text.encode_utf16().count();
                if before["text"] != format!("a{edited_prefix}{}bc", latest.text)
                    || before["start"].as_u64() != Some(expected_caret as u64)
                    || before["end"].as_u64() != Some(expected_caret as u64)
                {
                    return Err(format!("six-edit round {round} initial position {before}"));
                }
                // Two BMP characters, a fixed X and explicit editing evidence.
                // The helper dispatches TSF keys before Win32 edit messages; no physical
                // keyboard or main-program low-level hook coverage is claimed.
                let before_edit_prefix = edited_prefix.clone();
                edited_prefix.push_str(latest.text.strip_suffix("你好").ok_or("BMP tail")?);
                edited_prefix.push('X');
                crate::input_state::edited();
                let edited = fixture.command(serde_json::json!({"action":"controlled-edit"}))?;
                let expected = format!("a{edited_prefix}bc");
                if edited["controlledEdit"] != "completed"
                    || edited["text"] != expected
                    || edited["foreground"] != true
                {
                    return Err(format!("six-edit round {round} actual deletion {edited}"));
                }
                let steps = edited["controlledEditSteps"]
                    .as_array()
                    .ok_or("delete steps missing")?;
                let expected_steps = [
                    format!(
                        "a{before_edit_prefix}{}bc",
                        latest.text.strip_suffix('好').ok_or("first BMP tail")?
                    ),
                    format!(
                        "a{before_edit_prefix}{}bc",
                        latest.text.strip_suffix("你好").ok_or("second BMP tail")?
                    ),
                ];
                if steps.len() != expected_steps.len() {
                    return Err(format!(
                        "six-edit round {round} did not read both deletions"
                    ));
                }
                for (index, (step, expected)) in steps.iter().zip(&expected_steps).enumerate() {
                    let caret = (expected_caret - index - 1) as u64;
                    if step["text"] != *expected
                        || step["start"].as_u64() != Some(caret)
                        || step["end"].as_u64() != Some(caret)
                        || step["foreground"] != true
                    {
                        return Err(format!(
                            "six-edit round {round} deletion {} mismatch {step}",
                            index + 1
                        ));
                    }
                }
                let until = Instant::now() + Duration::from_secs(4);
                while manager.presentation() != Presentation::Editing {
                    active()?;
                    if Instant::now() >= until {
                        return Err(format!("six-edit round {round} handoff was not observed"));
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                while manager.presentation() != Presentation::Resuming {
                    active()?;
                    if Instant::now() >= until {
                        return Err(format!("six-edit round {round} new turn was not acquired"));
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                manager.update(&NativeTranscript {
                    task_id: id.clone(),
                    turn: latest.turn,
                    sequence: latest.sequence + 1,
                    text: "旧轮禁止覆盖".into(),
                    ..NativeTranscript::default()
                });
                thread::sleep(Duration::from_millis(60));
                wait_text(&mut fixture, &expected)?;
                latest.turn += 1;
                latest.sequence += 2;
                latest.text = format!("第{}轮你好", round + 1);
                manager.update(&latest);
                wait_text(&mut fixture, &format!("a{edited_prefix}{}bc", latest.text))?;
                active()?;
                println!(
                    "SAYWEAVE_COMPOSITION_EDIT_ROUND_PASS round={round} turn={} same_session=true controlled_edit=true synthetic_accessibility_notice=true actual_text=true cancel=0 stopping=false",
                    latest.turn
                );
            }
            flow.request_finish();
            cancel.send_replace(1);
            let delivery = runtime.block_on(manager.finish(&latest, Finish::Commit));
            runtime
                .block_on(async { tokio::time::timeout(Duration::from_secs(5), &mut actor).await })
                .map_err(|_| "six-edit actor did not finish")?
                .map_err(|e| e.to_string())?;
            let actual = fixture.command(serde_json::json!({"action":"read"}))?;
            let report = manager.report().ok_or("six-edit final report missing")?;
            if latest.turn != 7
                || report.error.is_some()
                || delivery.verification != Verification::Unavailable
                || delivery.submission != Submission::Submitted
                || delivery.error.as_deref() != Some("inputUnconfirmed")
                || actual["text"] != format!("a{edited_prefix}{}bc", latest.text)
            {
                return Err(format!(
                    "six-edit final assertion turn={} delivery={:?} report_error={:?} actual={actual}",
                    latest.turn, delivery.error, report.error
                ));
            }
            let helper = fixture.command(serde_json::json!({"action":"auxiliary-read"}))?;
            if helper["auxiliary"]["text"] != "owned helper sentinel" {
                return Err(format!("six-edit workflow wrote the helper {helper}"));
            }
            Ok(())
        })();
        if result.is_err() && !actor.is_finished() {
            flow.request_finish();
            cancel.send_replace(2);
            let _ = runtime.block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(5),
                    manager.finish(&latest, Finish::Abort),
                )
                .await
            });
            if !actor.is_finished() {
                actor.abort();
            }
            let _ = runtime
                .block_on(async { tokio::time::timeout(Duration::from_secs(1), &mut actor).await });
        }
        release_watch(&id);
        result?;
        println!(
            "SAYWEAVE_COMPOSITION_TURNS_PASS rounds=6 final_turn=7 controlled_edit=true actual_handoff=true resumed_range=true old_turn_ignored=true final_single_copy=true"
        );
    }
    // User edits followed by Finish on the same production actor. Delay cases
    // exercise real handoff/empty-resume timing; controlled unit replies cover
    // the exact in-flight Begin success/failure boundaries independently.
    for (delay, continue_speaking, late_result) in [
        (0, false, false),
        (100, false, false),
        (300, false, false),
        (800, false, false),
        (800, true, false),
        (0, false, true),
    ] {
        use crate::{
            dictation::drafts::SessionDraft,
            speech::{NativeTranscript, continuous::Control},
        };
        fixture.command(serde_json::json!({"action":"focus","text":"abc","start":1,"end":1}))?;
        thread::sleep(Duration::from_millis(60));
        let id = uuid::Uuid::new_v4().to_string();
        let anchor = WindowsInputService::anchor().ok_or("edit-finish target missing")?;
        bind_watch(&id, &anchor, None).map_err(|e| e.code)?;
        let draft = runtime
            .block_on(Draft::begin(
                id.clone(),
                1,
                anchor.clone(),
                Arc::new(DeliveryGate::default()),
            ))
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
        let mut actor = runtime.spawn(async move {
            ongoing.maintain(|_| {}).await;
        });
        let mut latest = NativeTranscript {
            task_id: id.clone(),
            turn: 1,
            sequence: 1,
            text: "用户你好".into(),
            ..NativeTranscript::default()
        };
        let result = (|| -> Result<(), String> {
            let wait_text = |fixture: &mut Fixture, expected: &str| -> Result<(), String> {
                let until = Instant::now() + Duration::from_secs(4);
                loop {
                    let read = fixture.command(serde_json::json!({"action":"read"}))?;
                    if read["text"] == expected {
                        return Ok(());
                    }
                    if Instant::now() >= until {
                        return Err(format!("edit-finish actual text mismatch: {read}"));
                    }
                    thread::sleep(Duration::from_millis(20));
                }
            };
            manager.update(&latest);
            wait_text(&mut fixture, "a用户你好bc")?;
            crate::input_state::edited();
            let edited = fixture.command(serde_json::json!({"action":"controlled-edit"}))?;
            if edited["text"] != "a用户Xbc" {
                return Err(format!("edit-finish edit missing: {edited}"));
            }
            thread::sleep(Duration::from_millis(delay));
            let mut expected = "a用户Xbc".to_string();
            if continue_speaking {
                latest.turn = 2;
                latest.sequence = 2;
                latest.text = "继续说话".into();
                manager.update(&latest);
                expected = "a用户X继续说话bc".into();
                wait_text(&mut fixture, &expected)?;
            }
            flow.request_finish();
            cancel.send_replace(1);
            if late_result {
                let mut late = latest.clone();
                late.sequence += 100;
                late.text = "旧结果不得覆盖用户正文".into();
                manager.update(&late);
            }
            let delivery = runtime
                .block_on(async {
                    tokio::time::timeout(
                        Duration::from_secs(5),
                        manager.finish(&latest, Finish::Commit),
                    )
                    .await
                })
                .map_err(|_| "edit-finish settlement timeout")?;
            runtime
                .block_on(async { tokio::time::timeout(Duration::from_secs(5), &mut actor).await })
                .map_err(|_| "edit-finish actor timeout")?
                .map_err(|e| e.to_string())?;
            let read = fixture.command(serde_json::json!({"action":"read"}))?;
            let report = manager.report().ok_or("edit-finish report missing")?;
            if read["text"] != expected
                || delivery.restore_error
                || delivery.submission == Submission::Uncertain
                || report.error.is_some()
            {
                return Err(format!(
                    "edit-finish failed delay={delay} continuation={continue_speaking} late={late_result} delivery={:?} report={:?} read={read}",
                    delivery.error, report.error
                ));
            }
            println!(
                "SAYWEAVE_EDIT_FINISH_PASS delay_ms={delay} continued={continue_speaking} late_result={late_result} actual={expected:?} user_text_preserved=true actor_settled=true"
            );
            Ok(())
        })();
        if result.is_err() && !actor.is_finished() {
            flow.request_finish();
            cancel.send_replace(2);
            let _ = runtime.block_on(async {
                tokio::time::timeout(
                    Duration::from_secs(5),
                    manager.finish(&latest, Finish::Abort),
                )
                .await
            });
            if !actor.is_finished() {
                actor.abort();
            }
            let _ = runtime
                .block_on(async { tokio::time::timeout(Duration::from_secs(1), &mut actor).await });
        }
        release_watch(&id);
        result?;
    }
    // The same styled helper is still a real external window if explicitly
    // activated. This uses actual foreground, not a synthetic notice, and must
    // remain invalidating after the original window regains foreground.
    {
        fixture
            .command(serde_json::json!({"action":"focus","text":"target ","start":7,"end":7}))?;
        thread::sleep(Duration::from_millis(60));
        let id = uuid::Uuid::new_v4().to_string();
        let anchor = WindowsInputService::anchor().ok_or("auxiliary negative target missing")?;
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
            .block_on(draft.replace(1, "原范围".into()))
            .map_err(|e| e.code)?;
        let helper = fixture.command(serde_json::json!({"action":"auxiliary-info"}))?;
        let window = helper["auxiliary"]["window"]
            .as_u64()
            .ok_or("helper missing")? as usize;
        let notices = AuxiliaryEvents::track(window);
        let previous = notices.count();
        let activated = fixture.command(serde_json::json!({"action":"auxiliary-activate"}))?;
        if activated["auxiliary"]["error"] != serde_json::Value::Null
            || activated["foreground"] != false
            || activated["auxiliary"]["foreground"] != true
        {
            return Err(format!("helper was not actual foreground {activated}"));
        }
        notices.wait_after(previous)?;
        fixture.command(serde_json::json!({"action":"focus"}))?;
        thread::sleep(Duration::from_millis(60));
        let result = runtime.block_on(draft.finish("禁止追加".into(), Finish::Commit));
        let actual = fixture.command(serde_json::json!({"action":"auxiliary-read"}))?;
        release_watch(&id);
        if result.error.as_deref() != Some("targetChanged")
            || actual["text"] != "target 原范围"
            || actual["auxiliary"]["text"] != "owned helper sentinel"
        {
            return Err(format!(
                "explicitly activated helper bypassed protection: {:?} {actual}",
                result.error
            ));
        }
        println!(
            "SAYWEAVE_NONACTIVATING_HELPER_PASS synthetic_notice_preserved_target=true actual_activation_sticky=true helper_untouched=true ime_identity_proven=false"
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
    use std::sync::atomic::AtomicBool;
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
    let mut gate: Option<Arc<DeliveryGate>> = None;
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
                    let current_gate = Arc::new(DeliveryGate::default());
                    let current = runtime
                        .block_on(Draft::begin(task.clone(), 1, anchor, current_gate.clone()))
                        .map_err(|e| e.code)?;
                    let sink = Arc::new(ObservationSink {
                        snapshot: crate::contracts::language::Snapshot {
                            application: application.clone(),
                            settings: crate::contracts::language::LanguageSettings {
                                automatic_terms: crate::contracts::language::Settings {
                                    enabled: true,
                                    revision: 0,
                                },
                                personalization: crate::contracts::language::Settings {
                                    enabled: true,
                                    revision: 0,
                                },
                            },
                            profile_version: 0,
                            shared_version: 0,
                            preferences: vec![],
                        },
                        control: Arc::new(crate::language_policy::LearningControl::new(
                            crate::contracts::language::LanguageSettings {
                                automatic_terms: crate::contracts::language::Settings {
                                    enabled: true,
                                    revision: 0,
                                },
                                personalization: crate::contracts::language::Settings {
                                    enabled: true,
                                    revision: 0,
                                },
                            },
                        )),
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
                    gate = Some(current_gate);
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
                "settle" => {
                    let outcome =
                        runtime.block_on(draft.as_ref().ok_or("begin first")?.settle_with_source(
                            value["sequence"].as_u64().ok_or("sequence required")?,
                            value["text"].as_str().ok_or("text required")?.into(),
                            value["tail"].as_str().unwrap_or("").into(),
                            Some((
                                value["raw"].as_str().ok_or("raw required")?.into(),
                                value["tailRaw"].as_str().unwrap_or("").into(),
                            )),
                            Arc::new(AtomicBool::new(true)),
                        ));
                    response["prefixChanged"] =
                        (outcome.prefix == sayweave_text_service::wire::Mutation::Changed).into();
                    response["tailOwned"] = outcome.tail_owned.into();
                    response["error"] =
                        serde_json::to_value(outcome.error).map_err(|e| e.to_string())?;
                }
                "commit" | "handoff" | "abort" => {
                    let outcome = runtime.block_on(
                        draft
                            .as_ref()
                            .ok_or("begin first")?
                            .finish_receipt_with_source(
                                value["text"].as_str().unwrap_or("").into(),
                                if action == "commit" {
                                    Finish::Commit
                                } else if action == "abort" {
                                    Finish::Abort
                                } else {
                                    Finish::Handoff("editing".into())
                                },
                                value["raw"].as_str().unwrap_or("").into(),
                            ),
                    );
                    response["committed"] = outcome.committed.into();
                    response["handedOff"] = outcome.handed_off.into();
                    response["cancelled"] = outcome.cancelled.into();
                    response["rollbackFailed"] = outcome.rollback_failed.into();
                    response["delivery"] =
                        serde_json::to_value(outcome.delivery).map_err(|e| e.to_string())?;
                }
                "cancel-gate" => {
                    gate.as_ref().ok_or("begin first")?.cancel();
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
