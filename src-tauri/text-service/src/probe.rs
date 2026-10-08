//! Stage-A operator probe. Not a renderer command and not part of the production dictation path.
use sayweave_text_service::{registration, transport::Pipe, wire::*};
use std::time::{Duration, Instant};
use windows::{
    Win32::{
        System::Com::*,
        UI::{TextServices::*, WindowsAndMessaging::*},
    },
    core::Interface,
};
struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
fn main() {
    if let Err(error) = run() {
        eprintln!("TSF probe failed: {error}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let _apartment = Apartment;
        let args: Vec<_> = std::env::args().collect();
        match args.get(1).map(String::as_str) {
            #[cfg(feature = "acceptance")]
            Some("host-identity-hold") => {
                let capability = uuid::Uuid::new_v4().to_string();
                let host = registration::host_capability(std::process::id(), &capability)?;
                if registration::host_capability(std::process::id(), &capability).is_ok()
                    || registration::read_host() != Some((std::process::id(), capability))
                {
                    return Err("host identity/reentry guard failed".into());
                }
                println!("host_identity_ready pid={}", std::process::id());
                pump(Duration::from_secs(20));
                drop(host);
                if registration::read_host().is_some() {
                    return Err("released host identity remained visible".into());
                }
                println!("host_identity_cleanup=true");
            }
            #[cfg(feature = "acceptance")]
            Some("host-identity-absent") => {
                if registration::read_host().is_some() {
                    return Err("inactive host identity remained visible".into());
                }
                println!("host_identity_absent=true");
            }
            #[cfg(feature = "acceptance")]
            Some("host-identity-read") => {
                let expected: u32 = args.get(2).ok_or("expected PID required")?.parse()?;
                let actual = registration::read_host().map(|p| p.0);
                println!("host_identity_match={}", actual == Some(expected));
                if actual != Some(expected) {
                    return Err("host identity was not visible in reader context".into());
                }
            }
            #[cfg(feature = "acceptance")]
            Some("fixture") => fixture()?,
            Some("register") => {
                let path = std::fs::canonicalize(args.get(2).ok_or("DLL path required")?)?;
                registration::register(&path.to_string_lossy())?;
                println!("Registered current-user speech profile; default keyboard unchanged.");
            }
            Some("register-machine") => {
                registration::register_machine(args.get(2).ok_or("DLL path required")?)?;
                println!("Installed speech component; activation remains operator-controlled.");
            }
            Some("remove-machine") => {
                registration::unregister_machine()?;
                println!("Removed owned machine speech component.");
            }
            Some("remove") => {
                registration::unregister()?;
                println!("Removed Sayweave speech profile.");
            }
            Some("probe" | "activation-check") => {
                let capability = uuid::Uuid::new_v4().to_string();
                let _host = registration::host_capability(std::process::id(), &capability)?;
                let keyboard_before = keyboard()?;
                let activation_before = sayweave_text_service::client::speech_activation()?;
                let mut host = sayweave_text_service::client::ProfileLease::open()?;
                if args[1] == "activation-check" {
                    host.close()?;
                    if keyboard()? != keyboard_before {
                        return Err("keyboard profile changed during cleanup".into());
                    }
                    if sayweave_text_service::client::speech_activation()? != activation_before {
                        return Err("speech activation was not restored".into());
                    }
                    println!(
                        "speech lease and prior activation restored ({:?}); keyboard unchanged; no input attempted",
                        activation_before.0
                    );
                    return Ok(());
                }
                let focus_seconds = args
                    .get(3)
                    .map(|s| s.parse::<u64>())
                    .transpose()?
                    .unwrap_or(5);
                let owned_immediate = focus_seconds == 0
                    && std::env::var("SAYWEAVE_INPUT_ACCEPTANCE").as_deref() == Ok("1")
                    && args
                        .get(2)
                        .and_then(|pid| pid.parse::<u32>().ok())
                        .is_some();
                if !owned_immediate && !(5..=30).contains(&focus_seconds) {
                    return Err("operator focus countdown must be between5and30seconds".into());
                }
                println!(
                    "Focus the intended blank input within{focus_seconds}seconds. This probe writes two cumulative snapshots, then commits once."
                );
                pump(Duration::from_secs(focus_seconds));
                let window = GetForegroundWindow();
                let mut pid = 0;
                let thread = GetWindowThreadProcessId(window, Some(&mut pid));
                if let Some(expected) = args.get(2)
                    && pid != expected.parse::<u32>()?
                {
                    return Err("foreground is not the owned fixture; no input attempted".into());
                }
                if pid == std::process::id() {
                    return Err("probe cannot target its own window".into());
                }
                let start = Instant::now();
                let pipe = loop {
                    if let Ok(pipe) = Pipe::client(pid, thread) {
                        break pipe;
                    }
                    if start.elapsed() > Duration::from_secs(3) {
                        return Err(
                            "speech profile activated but target thread has no TSF bridge".into(),
                        );
                    }
                    pump(Duration::from_millis(50));
                };
                let task = uuid::Uuid::new_v4().to_string();
                let multiline = args.get(4).map(String::as_str) == Some("multiline");
                let prefix_probe = matches!(
                    args.get(4).map(String::as_str),
                    Some("prefix" | "prefix-multiline" | "prefix-gap" | "prefix-gap-abort")
                );
                let prefix_gap = matches!(
                    args.get(4).map(String::as_str),
                    Some("prefix-gap" | "prefix-gap-abort")
                );
                let prefix_abort = args.get(4).map(String::as_str) == Some("prefix-gap-abort");
                let prefix_multiline = args.get(4).map(String::as_str) == Some("prefix-multiline");
                let interval = args
                    .get(5)
                    .map(|s| s.parse::<u64>())
                    .transpose()?
                    .unwrap_or(700);
                // Operator observation delay only; native request deadlines and
                // production recording/processing latency are unchanged.
                if interval > 3000 {
                    return Err("probe interval exceeds limit".into());
                }
                let begin_interval = args
                    .get(6)
                    .map(|s| s.parse::<u64>())
                    .transpose()?
                    .unwrap_or(interval);
                let first_write_interval = args
                    .get(7)
                    .map(|s| s.parse::<u64>())
                    .transpose()?
                    .unwrap_or(interval);
                if (args.len() > 6 && !owned_immediate)
                    || begin_interval > 3000
                    || first_write_interval > 3000
                {
                    return Err("independent intervals require a bounded owned fixture".into());
                }
                let first_boundary =
                    sayweave_text_service::settlement_boundary::SettlementBoundary::prepare(
                        "",
                        if prefix_multiline {
                            "第一段。\n"
                        } else {
                            "第一段。"
                        },
                    )
                    .unwrap();
                let second_boundary =
                    sayweave_text_service::settlement_boundary::SettlementBoundary::prepare(
                        &first_boundary.retained_separator,
                        if prefix_multiline {
                            "第二段。\n"
                        } else {
                            "第二段。"
                        },
                    )
                    .unwrap();
                let first_tail =
                    first_boundary.tail(if prefix_gap { "" } else { "第二段临时" });
                let growing_tail = first_boundary.tail("第二段续写🙂");
                let second_tail = second_boundary.tail(if prefix_gap { "" } else { "尾部🙂" });
                let final_tail = second_boundary.tail("尾部完成🙂");
                let mut steps = if prefix_probe {
                    vec![
                        (1, 0, Action::Inspect, "", None),
                        (2, 0, Action::Begin, "", None),
                        (
                            3,
                            1,
                            Action::Update,
                            if prefix_gap {
                                "ASR 第一段"
                            } else {
                                "ASR 第一段，第二段临时"
                            },
                            None,
                        ),
                        (
                            4,
                            2,
                            Action::Settle,
                            first_tail.as_str(),
                            Some(first_boundary.committed.as_str()),
                        ),
                        (5, 3, Action::Update, growing_tail.as_str(), None),
                        (
                            6,
                            4,
                            Action::Settle,
                            second_tail.as_str(),
                            Some(second_boundary.committed.as_str()),
                        ),
                        (7, 5, Action::Update, final_tail.as_str(), None),
                        (8, 5, Action::Commit, "", None),
                    ]
                } else {
                    vec![
                        (1, 0, Action::Inspect, ""),
                        (2, 0, Action::Begin, ""),
                        (3, 1, Action::Update, "Sayweave 原位"),
                        (
                            4,
                            2,
                            Action::Update,
                            if multiline {
                                "中文，🙂\n第二行 e\u{0301}。"
                            } else {
                                "Sayweave 原位验证，🙂"
                            },
                        ),
                        (5, 2, Action::Commit, ""),
                    ]
                    .into_iter()
                    .map(|(op, revision, action, text)| (op, revision, action, text, None))
                    .collect()
                };
                if prefix_abort {
                    steps.truncate(5);
                    steps.push((6, 3, Action::Abort, "", None));
                }
                let mut range_epoch = 0;
                let mut previous_revision = 0;
                for (op, revision, action, text, processed) in steps {
                    if GetForegroundWindow() != window {
                        return Err("target foreground changed; stopped without retry".into());
                    }
                    let r = Request {
                        protocol: sayweave_text_service::wire::PROTOCOL,
                        capability: capability.clone(),
                        task: task.clone(),
                        turn: 1,
                        operation: op,
                        revision,
                        deadline_tick: windows::Win32::System::SystemInformation::GetTickCount64()
                            + 2000,
                        action,
                        text: text.into(),
                        prefix: processed.map(|text| Prefix {
                            text: text.into(),
                            expected_revision: previous_revision,
                            range_epoch,
                        }),
                    };
                    pipe.send(&serde_json::to_vec(&r)?)?;
                    let wait = Instant::now();
                    let reply = loop {
                        if let Some(data) = pipe.receive()? {
                            break serde_json::from_slice::<Reply>(&data)?;
                        }
                        if wait.elapsed() > Duration::from_secs(2) {
                            return Err(
                                "edit acknowledgement timeout; no automatic reinsertion".into()
                            );
                        }
                        pump(Duration::from_millis(10));
                    };
                    println!(
                        "task={task} operation={} revision={} applied={} code={}",
                        reply.operation, reply.revision, reply.applied, reply.code
                    );
                    if reply.operation != op || reply.revision != revision || !reply.applied {
                        return Err("target did not apply edit".into());
                    }
                    if action == Action::Inspect && !reply.compatible_runtime(pid, thread) {
                        return Err("target runtime mismatch; no input attempted".into());
                    }
                    if action == Action::Settle
                        && !reply.settlement.is_some_and(|s| {
                            s.prefix_operation == op
                                && s.prefix == Mutation::Changed
                                && s.tail_owned
                        })
                    {
                        return Err("target did not retain a writable settled tail".into());
                    }
                    range_epoch = reply.range_epoch;
                    previous_revision = revision;
                    pump(Duration::from_millis(match op {
                        2 => begin_interval,
                        3 => first_write_interval,
                        _ => interval,
                    }));
                }
                if keyboard()? != keyboard_before {
                    return Err("keyboard profile changed during probe".into());
                }
                host.close()?;
                if keyboard()? != keyboard_before {
                    return Err("keyboard profile changed during cleanup".into());
                }
                println!(
                    "Composition updates acknowledged. Actual target text still requires visual/readback acceptance."
                );
            }
            Some("status") => {
                for profile in input_profiles()?
                    .iter()
                    .filter(|p| p.clsid == registration::CLASS)
                {
                    println!(
                        "sayweave_speech_profile={} language={} enabled={} active={}",
                        profile.catid == GUID_TFCAT_TIP_SPEECH,
                        profile.langid,
                        registration::profiles()?
                            .IsEnabledLanguageProfile(
                                &profile.clsid,
                                profile.langid,
                                &profile.guidProfile
                            )?
                            .as_bool(),
                        profile.dwFlags & TF_IPP_FLAG_ACTIVE != 0
                    );
                }
            }
            _ => return Err("Usage: tsf-probe register DLL | remove | status | probe".into()),
        }
        Ok(())
    }
}
fn input_profiles() -> windows::core::Result<Vec<TF_INPUTPROCESSORPROFILE>> {
    unsafe {
        let manager: ITfInputProcessorProfileMgr = registration::profiles()?.cast()?;
        let enumerator = manager.EnumProfiles(0)?;
        let mut result = Vec::new();
        loop {
            let mut batch = [TF_INPUTPROCESSORPROFILE::default(); 16];
            let mut fetched = 0;
            enumerator.Next(&mut batch, &mut fetched)?;
            if fetched as usize > batch.len() || result.len() + fetched as usize > 512 {
                return Err(windows::core::Error::new(
                    windows::Win32::Foundation::E_FAIL,
                    "profile enumeration exceeded bounds",
                ));
            }
            result.extend_from_slice(&batch[..fetched as usize]);
            if fetched == 0 {
                return Ok(result);
            }
        }
    }
}
fn keyboard() -> windows::core::Result<(u32, u16, windows::core::GUID, windows::core::GUID, isize)>
{
    unsafe {
        let manager: ITfInputProcessorProfileMgr = registration::profiles()?.cast()?;
        let mut profile = TF_INPUTPROCESSORPROFILE::default();
        manager.GetActiveProfile(&GUID_TFCAT_TIP_KEYBOARD, &mut profile)?;
        Ok((
            profile.dwProfileType,
            profile.langid,
            profile.clsid,
            profile.guidProfile,
            profile.hkl.0 as isize,
        ))
    }
}
#[cfg(feature = "acceptance")]
fn fixture() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        use windows::{
            Win32::{
                Foundation::*,
                System::{LibraryLoader::*, Threading::GetCurrentThreadId},
                UI::Input::KeyboardAndMouse::*,
            },
            core::w,
        };
        let plain = std::env::args().nth(3).as_deref() == Some("plain");
        println!("owned fixture stage=library");
        let library = LoadLibraryExW(w!("Msftedit.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)?;
        println!("owned fixture stage=window");
        let top = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("Sayweave TSF isolated fixture"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            20,
            20,
            520,
            240,
            None,
            None,
            None,
            None,
        )?;
        println!("owned fixture stage=editor");
        let editor = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            if plain { w!("EDIT") } else { w!("RICHEDIT50W") },
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_MULTILINE as u32),
            10,
            10,
            480,
            170,
            Some(top),
            None,
            None,
            None,
        )?;
        println!("owned fixture stage=manager");
        let manager: ITfThreadMgr =
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)
                .map_err(|e| format!("owned manager create: {e}"))?;
        manager
            .Activate()
            .map_err(|e| format!("owned manager activate: {e}"))?;
        println!("owned fixture stage=manager-active");
        if !plain {
            let _ = SendMessageW(
                editor,
                WM_USER + 204,
                Some(WPARAM(0x10000)),
                Some(LPARAM(0x10000)),
            );
        }
        let _ = SetForegroundWindow(top);
        let _ = SetFocus(Some(editor));
        // Test-only operator/controller lease. Allows the desktop adapter to
        // focus this exact newly-created window; no production capture delay.
        if std::env::var_os("SAYWEAVE_TSF_FIXTURE_SUPERVISED").is_some() {
            println!("owned fixture awaiting controller foreground=true");
            let deadline = Instant::now() + Duration::from_secs(30);
            while GetForegroundWindow() != top && Instant::now() < deadline {
                pump(Duration::from_millis(25));
            }
        }
        pump(Duration::from_millis(200));
        println!(
            "owned fixture stage=focus foreground_matches={}",
            GetForegroundWindow() == top
        );
        if GetForegroundWindow() != top {
            println!(
                "owned fixture focus fallback top_valid={} editor_valid={}",
                IsWindow(Some(top)).as_bool(),
                IsWindow(Some(editor)).as_bool()
            );
            // A background test launcher may be denied SetForegroundWindow.
            // Exercise only the blank window this fixture owns, then verify focus.
            SetWindowPos(
                top,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .map_err(|e| format!("owned focus topmost: {e}"))?;
            let mut rect = RECT::default();
            GetWindowRect(editor, &mut rect).map_err(|e| format!("owned focus rect: {e}"))?;
            SetCursorPos(rect.left + 30, rect.top + 30)
                .map_err(|e| format!("owned focus cursor: {e}"))?;
            let events = [MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP].map(|flags| INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dwFlags: flags,
                        ..Default::default()
                    },
                },
            });
            if SendInput(&events, std::mem::size_of::<INPUT>() as i32) != 2 {
                return Err("owned fixture focus click was not submitted".into());
            }
            pump(Duration::from_millis(200));
            SetWindowPos(
                top,
                Some(HWND_NOTOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )?;
            println!("owned fixture required a real activation click=true");
        }
        if GetForegroundWindow() != top {
            return Err("owned fixture is not the actual foreground window".into());
        }
        let module_path = std::env::args()
            .nth(2)
            .map(std::path::PathBuf::from)
            .unwrap_or(std::env::current_exe()?.with_file_name("sayweave_text_service.dll"));
        // The installed speech profile can already be active in this fixture process.
        // Isolate the explicitly loaded component here only; leave the desktop/user
        // profile and keyboard unchanged.
        println!("owned fixture stage=activation");
        let (activation, language) = sayweave_text_service::client::speech_activation()?;
        if activation == sayweave_text_service::client::SpeechActivation::Own {
            registration::deactivate(language, false)?;
            pump(Duration::from_millis(100));
        }
        println!("owned fixture stage=component");
        let (component, processor) = load_processor(&module_path)?;
        let service_client = manager
            .cast::<ITfClientId>()?
            .GetClientId(&registration::CLASS)?;
        println!("owned fixture stage=service");
        processor.Activate(&manager, service_client)?;
        let capability = uuid::Uuid::new_v4().to_string();
        let host = registration::host_capability(std::process::id(), &capability)?;
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let context = manager.GetFocus()?.GetTop()?;
            println!(
                "owned TSF context available; readonly={}",
                context.GetStatus()?.dwDynamicFlags & TF_SD_READONLY != 0
            );
            let pipe = Pipe::client(std::process::id(), GetCurrentThreadId())?;
            let task = uuid::Uuid::new_v4().to_string();
            let mut expected = String::new();
            for (operation, revision, action, text) in [
                (1, 0, Action::Begin, ""),
                (2, 1, Action::Update, "Sayweave 原位"),
                (3, 2, Action::Update, "Sayweave 原位验证，🙂"),
                (4, 2, Action::Commit, ""),
            ] {
                let request = Request {
                    protocol: sayweave_text_service::wire::PROTOCOL,
                    capability: capability.clone(),
                    task: task.clone(),
                    turn: 1,
                    operation,
                    revision,
                    deadline_tick: windows::Win32::System::SystemInformation::GetTickCount64()
                        + 2000,
                    action,
                    text: text.into(),
                    prefix: None,
                };
                pipe.send(&serde_json::to_vec(&request)?)?;
                let started = Instant::now();
                let reply = loop {
                    pump(Duration::from_millis(10));
                    if let Some(data) = pipe.receive()? {
                        break serde_json::from_slice::<Reply>(&data)?;
                    }
                    if started.elapsed() > Duration::from_secs(2) {
                        return Err("owned edit acknowledgement timeout".into());
                    }
                };
                if reply.operation != operation || !reply.applied {
                    return Err(format!("owned edit rejected: {}", reply.code).into());
                }
                if action == Action::Update {
                    expected = text.into();
                }
                // A legacy IMM-backed EDIT keeps active composition outside its
                // committed WM_GETTEXT buffer. Read the actual TSF range while
                // composing, and require real control text after Commit.
                let composing = plain && action == Action::Update;
                let actual = if composing {
                    composition_text(&context)?
                } else {
                    editor_text(editor)?
                };
                if actual != expected {
                    return Err(format!("owned target text assertion failed: operation={operation} expected_utf16={} actual_utf16={} actual_hash={}", expected.encode_utf16().count(), actual.encode_utf16().count(), sayweave_text_service::deployment::version(actual.as_bytes())).into());
                }
                println!(
                    "owned operation={operation} applied=true actual_text_assertion=true source={}",
                    if composing {
                        "compositionRange"
                    } else {
                        "controlText"
                    }
                );
            }
            prefix_fixture(editor, &manager, &pipe, &capability)?;
            prefix_fault_fixture(component, editor, &pipe, &capability)?;
            if plain {
                let end = editor_text(editor)?.encode_utf16().count();
                SendMessageW(
                    editor,
                    0x00B1,
                    Some(WPARAM(end)),
                    Some(LPARAM(end as isize)),
                );
                pump(Duration::from_millis(20));
                println!(
                    "owned plain EDIT cumulative replacement and single commit actual text=true"
                );
                let before = editor_text(editor)?;
                if !exchange(&pipe, &capability, &task, 2, 1, 0, Action::Begin, "", false)?.applied
                    || !exchange(
                        &pipe,
                        &capability,
                        &task,
                        2,
                        2,
                        1,
                        Action::Update,
                        "取消保留",
                        false,
                    )?
                    .applied
                {
                    return Err("plain cancel setup rejected".into());
                }
                pump(Duration::from_millis(100));
                let abort = exchange(&pipe, &capability, &task, 2, 3, 1, Action::Abort, "", false)?;
                if abort.applied
                    || abort.mutation != Mutation::NotAttempted
                    || editor_text(editor)? != format!("{before}取消保留")
                {
                    return Err(format!("plain property-affected cancellation lost visible text: code={} applied={} mutation={:?} owned_fixture_expected={:?} owned_fixture_actual={:?}", abort.code, abort.applied, abort.mutation, format!("{before}取消保留"), editor_text(editor)?).into());
                }
                println!(
                    "owned plain EDIT cancellation retains property-affected text and ends composition=true"
                );
                return Ok(());
            }
            let original = "前 被选🙂 后";
            SendMessageW(
                editor,
                WM_SETTEXT,
                None,
                Some(LPARAM(
                    original
                        .encode_utf16()
                        .chain(Some(0))
                        .collect::<Vec<_>>()
                        .as_ptr() as isize,
                )),
            );
            SendMessageW(editor, 0xB1, Some(WPARAM(2)), Some(LPARAM(6))); // EM_SETSEL uses UTF16 offsets.
            pump(Duration::from_millis(50));
            for (operation, revision, action, text, expected) in [
                (5, 0, Action::Begin, "", original),
                (6, 1, Action::Update, "替换🙂", "前 替换🙂 后"),
                (7, 1, Action::Abort, "", original),
            ] {
                let reply = exchange(
                    &pipe,
                    &capability,
                    &task,
                    2,
                    operation,
                    revision,
                    action,
                    text,
                    false,
                )?;
                if !reply.applied || editor_text(editor)? != expected {
                    return Err("selection replacement/rollback assertion failed".into());
                }
            }
            println!("owned selection replacement and abort restore actual text=true");
            let before_expired = editor_text(editor)?;
            let expired = exchange(&pipe, &capability, &task, 3, 8, 0, Action::Begin, "", true)?;
            if expired.applied || editor_text(editor)? != before_expired {
                return Err("expired request modified target".into());
            }
            println!("owned expired request rejected without text mutation=true");
            exchange(&pipe, &capability, &task, 4, 9, 0, Action::Begin, "", false)?;
            let update = exchange(
                &pipe,
                &capability,
                &task,
                4,
                10,
                1,
                Action::Update,
                "交接文字",
                false,
            )?;
            if !update.applied {
                return Err("property case setup rejected".into());
            }
            let user_edit: ITfEditSession = UserPropertyEdit {
                context: context.clone(),
                mode: 0,
            }
            .into();
            context
                .RequestEditSession(0, &user_edit, TF_ES_SYNC | TF_ES_READWRITE)?
                .ok()?;
            pump(Duration::from_millis(100));
            verify_language(&context, 0x409)?;
            let corrected = exchange(
                &pipe,
                &capability,
                &task,
                4,
                11,
                2,
                Action::Update,
                "更新文字",
                false,
            )?;
            if !corrected.applied {
                return Err("property-preserving correction rejected".into());
            }
            verify_language(&context, 0x409)?;
            let handed_off_text = editor_text(editor)?;
            let abort = exchange(
                &pipe,
                &capability,
                &task,
                4,
                12,
                2,
                Action::Abort,
                "",
                false,
            )?;
            if abort.applied
                || abort.mutation != Mutation::NotAttempted
                || editor_text(editor)? != handed_off_text
            {
                return Err("external property edit was overwritten by rollback".into());
            }
            verify_language(&context, 0x409)?;
            println!("owned property-only user edit revoked rollback ownership=true");
            let handoff = exchange(
                &pipe,
                &capability,
                &task,
                4,
                13,
                2,
                Action::Handoff,
                "",
                false,
            )?;
            if !handoff.applied
                || handoff.code != "alreadyHandedOff"
                || handoff.mutation != Mutation::NotAttempted
                || editor_text(editor)? != handed_off_text
            {
                return Err("same-turn released range handoff acknowledgement changed text".into());
            }
            println!("owned released-range handoff acknowledged without mutation=true");
            // A property affecting only part of the owned text cannot be mapped
            // onto a replacement snapshot; deleting a property is likewise not
            // a scalar value that can be preserved.
            for (turn, mode) in [(6, 1), (7, 2)] {
                SendMessageW(
                    editor,
                    WM_SETTEXT,
                    None,
                    Some(LPARAM([0u16].as_ptr() as isize)),
                );
                SendMessageW(editor, 0xB1, Some(WPARAM(0)), Some(LPARAM(0)));
                pump(Duration::from_millis(50));
                for (operation, revision, action, text) in [
                    (1, 0, Action::Begin, ""),
                    (2, 1, Action::Update, "保留原文字"),
                ] {
                    if !exchange(
                        &pipe,
                        &capability,
                        &task,
                        turn,
                        operation,
                        revision,
                        action,
                        text,
                        false,
                    )?
                    .applied
                    {
                        return Err("attribute rejection setup failed".into());
                    }
                }
                let full: ITfEditSession = UserPropertyEdit {
                    context: context.clone(),
                    mode: 0,
                }
                .into();
                context
                    .RequestEditSession(0, &full, TF_ES_SYNC | TF_ES_READWRITE)?
                    .ok()?;
                let changed: ITfEditSession = UserPropertyEdit {
                    context: context.clone(),
                    mode,
                }
                .into();
                context
                    .RequestEditSession(0, &changed, TF_ES_SYNC | TF_ES_READWRITE)?
                    .ok()?;
                pump(Duration::from_millis(100));
                let preserved = editor_text(editor)?;
                let reply = exchange(
                    &pipe,
                    &capability,
                    &task,
                    turn,
                    3,
                    2,
                    Action::Update,
                    "不得覆盖",
                    false,
                )?;
                if reply.applied
                    || reply.mutation != Mutation::NotAttempted
                    || editor_text(editor)? != preserved
                {
                    return Err("partial/cleared attribute allowed later overwrite".into());
                }
                println!(
                    "owned attribute mode={mode} hands off and rejects stale replacement=true"
                );
            }
            SendMessageW(
                editor,
                WM_SETTEXT,
                None,
                Some(LPARAM([0u16].as_ptr() as isize)),
            );
            SendMessageW(editor, 0xB1, Some(WPARAM(0)), Some(LPARAM(0)));
            pump(Duration::from_millis(50));
            let multiline = "中文，\n换行🙂";
            for (operation, revision, action, text) in [
                (13, 0, Action::Begin, ""),
                (14, 1, Action::Update, multiline),
                (15, 1, Action::Commit, ""),
            ] {
                let reply = exchange(
                    &pipe,
                    &capability,
                    &task,
                    8,
                    operation,
                    revision,
                    action,
                    text,
                    false,
                )?;
                if !reply.applied {
                    return Err(format!("multiline edit rejected: {}", reply.code).into());
                }
                if action != Action::Begin
                    && editor_text(editor)?
                        .replace("\r\n", "\n")
                        .replace('\r', "\n")
                        != multiline
                {
                    return Err("multiline actual target text incorrect".into());
                }
            }
            println!("owned multiline and emoji update/commit actual text=true");
            // Exercise TSF's actual key dispatcher, not a direct call to our sink.
            // It must hand off before the application edits, without eating the key.
            for (op, revision, action, text) in [
                (16, 0, Action::Begin, ""),
                (17, 1, Action::Update, "删除字"),
            ] {
                if !exchange(
                    &pipe,
                    &capability,
                    &task,
                    9,
                    op,
                    revision,
                    action,
                    text,
                    false,
                )?
                .applied
                {
                    return Err("keyboard handoff setup rejected".into());
                }
            }
            let keys = manager.cast::<ITfKeystrokeMgr>()?;
            if !keys
                .TestKeyDown(WPARAM(VK_BACK.0 as usize), LPARAM(1))?
                .as_bool()
            {
                return Err("owned composition did not receive Backspace test".into());
            }
            if keys
                .KeyDown(WPARAM(VK_BACK.0 as usize), LPARAM(1))?
                .as_bool()
            {
                return Err("composition handoff swallowed Backspace".into());
            }
            SendMessageW(
                editor,
                WM_KEYDOWN,
                Some(WPARAM(VK_BACK.0 as usize)),
                Some(LPARAM(1)),
            );
            pump(Duration::from_millis(100));
            let handoff = exchange(
                &pipe,
                &capability,
                &task,
                9,
                18,
                1,
                Action::Handoff,
                "",
                false,
            )?;
            if !handoff.applied
                || handoff.code != "keyboardHandedOff"
                || !editor_text(editor)?.ends_with("删除")
            {
                return Err(format!("keyboard handoff restored deleted text or missed pre-edit dispatch: code={} applied={} fixture_text={:?}", handoff.code, handoff.applied, editor_text(editor)?).into());
            }
            println!("owned pre-edit Backspace handoff preserves actual deletion=true");
            for semantic in [
                "\n开头🙂",
                "结尾\n",
                "前\n\n后",
                "前\r\n中\r后",
                "前🙂\n组合 e\u{301}",
            ] {
                let original = "前 被选🙂 后";
                SendMessageW(
                    editor,
                    WM_SETTEXT,
                    None,
                    Some(LPARAM(
                        original
                            .encode_utf16()
                            .chain(Some(0))
                            .collect::<Vec<_>>()
                            .as_ptr() as isize,
                    )),
                );
                SendMessageW(editor, 0xB1, Some(WPARAM(2)), Some(LPARAM(6)));
                pump(Duration::from_millis(30));
                let sample_task = uuid::Uuid::new_v4().to_string();
                for (operation, revision, action, text) in [
                    (1, 0, Action::Begin, ""),
                    (2, 1, Action::Update, "临时\n片段"),
                    (3, 2, Action::Update, semantic),
                    (4, 2, Action::Commit, ""),
                ] {
                    let reply = exchange(
                        &pipe,
                        &capability,
                        &sample_task,
                        1,
                        operation,
                        revision,
                        action,
                        text,
                        false,
                    )?;
                    if !reply.applied {
                        return Err(
                            format!("multiline boundary case rejected: {}", reply.code).into()
                        );
                    }
                }
                let actual = editor_text(editor)?
                    .replace("\r\n", "\n")
                    .replace('\r', "\n");
                let expected = format!(
                    "前 {} 后",
                    semantic.replace("\r\n", "\n").replace('\r', "\n")
                );
                if actual != expected {
                    return Err(
                        "multiline boundary commit changed original surroundings or text".into(),
                    );
                }
            }
            println!(
                "owned leading/trailing/repeated/mixed breaks, revised snapshots and Unicode exact text=true"
            );
            Ok(())
        })();
        drop(host);
        let _ = processor.Deactivate();
        drop(processor);
        let _ = manager.Deactivate();
        let _ = DestroyWindow(top);
        let _ = FreeLibrary(library);
        pump(Duration::from_millis(100));
        let entry = GetProcAddress(component, windows::core::s!("DllCanUnloadNow"))
            .ok_or("missing unload export")?;
        let can_unload: unsafe extern "system" fn() -> windows::core::HRESULT =
            std::mem::transmute(entry);
        if can_unload() != windows::Win32::Foundation::S_OK {
            return Err("component still owns resources after fixture cleanup".into());
        }
        FreeLibrary(component)?;
        println!("owned COM DLL unloaded with no live leases=true");
        result
    }
}
#[cfg(feature = "acceptance")]
fn composition_text(context: &ITfContext) -> Result<String, Box<dyn std::error::Error>> {
    let output = std::rc::Rc::new(std::cell::RefCell::new(None));
    let read: ITfEditSession = ReadComposition {
        context: context.clone(),
        output: output.clone(),
    }
    .into();
    unsafe {
        context
            .RequestEditSession(0, &read, TF_ES_ASYNCDONTCARE | TF_ES_READ)?
            .ok()?;
    }
    let started = Instant::now();
    loop {
        if let Some(value) = output.borrow_mut().take() {
            return Ok(value?);
        }
        if started.elapsed() > Duration::from_secs(2) {
            return Err("owned composition read timeout".into());
        }
        pump(Duration::from_millis(10));
    }
}
#[cfg(feature = "acceptance")]
#[windows::core::implement(ITfEditSession)]
struct ReadComposition {
    context: ITfContext,
    output: std::rc::Rc<std::cell::RefCell<Option<windows::core::Result<String>>>>,
}
#[cfg(feature = "acceptance")]
impl ITfEditSession_Impl for ReadComposition_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        let result = (|| unsafe {
            let compositions: ITfContextComposition = self.context.cast()?;
            let mut composition = [None];
            let mut count = 0;
            compositions
                .EnumCompositions()?
                .Next(&mut composition, &mut count)?;
            if count != 1 {
                return Err(windows::core::Error::new(
                    windows::Win32::Foundation::E_FAIL,
                    "owned composition missing",
                ));
            }
            let range = composition[0].as_ref().unwrap().GetRange()?;
            let mut text = [0u16; 256];
            let mut fetched = 0;
            range.GetText(ec, 0, &mut text, &mut fetched)?;
            String::from_utf16(&text[..fetched as usize])
                .map_err(|_| windows::core::Error::from(windows::Win32::Foundation::E_FAIL))
        })();
        *self.output.borrow_mut() = Some(result);
        Ok(())
    }
}
#[cfg(feature = "acceptance")]
fn load_processor(
    path: &std::path::Path,
) -> Result<(windows::Win32::Foundation::HMODULE, ITfTextInputProcessor), Box<dyn std::error::Error>>
{
    use windows::{
        Win32::{Foundation::FreeLibrary, System::LibraryLoader::*},
        core::{GUID, HRESULT, Interface, PCWSTR, s},
    };
    unsafe {
        let path: Vec<_> = std::fs::canonicalize(path)?
            .to_string_lossy()
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let module = LoadLibraryExW(
            PCWSTR(path.as_ptr()),
            None,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )?;
        let result = (|| -> Result<ITfTextInputProcessor, Box<dyn std::error::Error>> {
            let entry = GetProcAddress(module, s!("DllGetClassObject"))
                .ok_or("missing COM factory export")?;
            let factory_entry: unsafe extern "system" fn(
                *const GUID,
                *const GUID,
                *mut *mut std::ffi::c_void,
            ) -> HRESULT = std::mem::transmute(entry);
            let mut raw = std::ptr::null_mut();
            factory_entry(&registration::CLASS, &IClassFactory::IID, &mut raw).ok()?;
            let factory = IClassFactory::from_raw(raw);
            Ok(factory.CreateInstance(None)?)
        })();
        match result {
            Ok(processor) => Ok((module, processor)),
            Err(e) => {
                let _ = FreeLibrary(module);
                Err(e)
            }
        }
    }
}
#[cfg(feature = "acceptance")]
#[windows::core::implement(ITfEditSession)]
struct UserPropertyEdit {
    context: ITfContext,
    mode: u8,
}
#[cfg(feature = "acceptance")]
#[windows::core::implement(ITfEditSession)]
struct ReadLanguage {
    context: ITfContext,
    result: std::rc::Rc<std::cell::Cell<Option<i32>>>,
}
#[cfg(feature = "acceptance")]
impl ITfEditSession_Impl for ReadLanguage_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        unsafe {
            let range = self.context.GetStart(ec)?;
            range.ShiftEndToRange(ec, &self.context.GetEnd(ec)?, TF_ANCHOR_END)?;
            let value = self
                .context
                .GetProperty(&GUID_PROP_LANGID)?
                .GetValue(ec, &range)?;
            eprintln!(
                "language proof type={} value={:?}",
                value.vt().0,
                i32::try_from(&value).ok()
            );
            if value.vt() != windows::Win32::System::Variant::VT_I4 {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_FAIL,
                ));
            }
            self.result.set(Some(i32::try_from(&value)?));
            Ok(())
        }
    }
}
#[cfg(feature = "acceptance")]
fn verify_language(context: &ITfContext, expected: i32) -> Result<(), Box<dyn std::error::Error>> {
    let result = std::rc::Rc::new(std::cell::Cell::new(None));
    let read: ITfEditSession = ReadLanguage {
        context: context.clone(),
        result: result.clone(),
    }
    .into();
    unsafe {
        context
            .RequestEditSession(0, &read, TF_ES_SYNC | TF_ES_READ)?
            .ok()?;
    }
    if result.get() != Some(expected) {
        return Err("user LANGID changed during correction/rollback".into());
    }
    Ok(())
}
#[cfg(feature = "acceptance")]
impl ITfEditSession_Impl for UserPropertyEdit_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        unsafe {
            let range = self.context.GetStart(ec)?;
            range.ShiftEndToRange(ec, &self.context.GetEnd(ec)?, TF_ANCHOR_END)?;
            if self.mode == 1 {
                range.Collapse(ec, TF_ANCHOR_START)?;
                let mut shifted = 0;
                range.ShiftEnd(ec, 1, &mut shifted, std::ptr::null())?;
            }
            if self.mode == 2 {
                return self
                    .context
                    .GetProperty(&GUID_PROP_LANGID)?
                    .Clear(ec, &range);
            }
            self.context.GetProperty(&GUID_PROP_LANGID)?.SetValue(
                ec,
                &range,
                &windows::Win32::System::Variant::VARIANT::from(if self.mode == 1 {
                    0x804i32
                } else {
                    0x409i32
                }),
            )
        }
    }
}
#[cfg(feature = "acceptance")]
fn editor_text(
    editor: windows::Win32::Foundation::HWND,
) -> Result<String, Box<dyn std::error::Error>> {
    use windows::Win32::Foundation::*;
    unsafe {
        let length = SendMessageW(editor, WM_GETTEXTLENGTH, None, None).0 as usize;
        let mut actual = vec![0u16; length + 1];
        SendMessageW(
            editor,
            WM_GETTEXT,
            Some(WPARAM(actual.len())),
            Some(LPARAM(actual.as_mut_ptr() as isize)),
        );
        Ok(String::from_utf16(&actual[..length])?)
    }
}
#[cfg(feature = "acceptance")]
#[allow(clippy::too_many_arguments)]
fn exchange(
    pipe: &Pipe,
    capability: &str,
    task: &str,
    turn: u64,
    operation: u64,
    revision: u64,
    action: Action,
    text: &str,
    expired: bool,
) -> Result<Reply, Box<dyn std::error::Error>> {
    let now = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
    let r = Request {
        protocol: sayweave_text_service::wire::PROTOCOL,
        capability: capability.into(),
        task: task.into(),
        turn,
        operation,
        revision,
        deadline_tick: if expired { now - 1 } else { now + 2000 },
        action,
        text: text.into(),
        prefix: None,
    };
    pipe.send(&serde_json::to_vec(&r)?)?;
    let start = Instant::now();
    loop {
        pump(Duration::from_millis(10));
        if let Some(data) = pipe.receive()? {
            let reply: Reply = serde_json::from_slice(&data)?;
            if reply.operation != operation {
                return Err("acknowledgement operation mismatch".into());
            }
            return Ok(reply);
        }
        if start.elapsed() > Duration::from_secs(2) {
            return Err("owned acknowledgement timeout".into());
        }
    }
}
#[cfg(feature = "acceptance")]
fn generated_text_matches(actual: &str, expected: &str) -> bool {
    actual
        .replace("\r\n", "\n")
        .replace(['\r', '\u{2028}', '\u{b}'], "\n")
        == expected.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(feature = "acceptance")]
fn prefix_fixture(
    editor: windows::Win32::Foundation::HWND,
    manager: &ITfThreadMgr,
    pipe: &Pipe,
    capability: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use windows::{Win32::Foundation::*, core::w};
    for (committed, tail_text, move_caret) in [
        ("整理前段🙂", "尾🙂", false),
        ("整理前段🙂", "\n尾🙂", false),
        ("整理前段🙂", "\n", false),
        ("整理\n前段🙂", "尾🙂", false),
        ("整理前段🙂", "", false),
        ("整理前段🙂", "", true),
    ] {
        unsafe {
            SendMessageW(
                editor,
                WM_SETTEXT,
                None,
                Some(LPARAM(w!("左原选区右").as_ptr() as isize)),
            );
            SendMessageW(editor, 0x00B1, Some(WPARAM(1)), Some(LPARAM(4)));
        }
        // Drain the fixture's own external reset before claiming a new range.
        pump(Duration::from_millis(50));
        let task = uuid::Uuid::new_v4().to_string();
        let begin = exchange(pipe, capability, &task, 1, 1, 0, Action::Begin, "", false)?;
        if !begin.applied {
            return Err(format!("prefix begin rejected: {}", begin.code).into());
        }
        let context = unsafe { manager.GetFocus()?.GetTop()? };
        println!(
            "prefix fixture begin epoch={} context_selected=true",
            begin.range_epoch
        );
        let initial = exchange(
            pipe,
            capability,
            &task,
            1,
            2,
            1,
            Action::Update,
            "原文尾🙂",
            false,
        )?;
        if !initial.applied {
            return Err("prefix initial update failed".into());
        }
        println!(
            "prefix initial epoch={} applied={}",
            initial.range_epoch, initial.applied
        );
        if composition_text(&context).is_err() {
            return Err(
                "prefix fixture initial composition already missing before settlement".into(),
            );
        }
        let settle = Request {
            protocol: PROTOCOL,
            capability: capability.into(),
            task: task.clone(),
            turn: 1,
            operation: 3,
            revision: 2,
            deadline_tick: unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
                + 2000,
            action: Action::Settle,
            text: tail_text.into(),
            prefix: Some(Prefix {
                text: committed.into(),
                expected_revision: 1,
                range_epoch: begin.range_epoch,
            }),
        };
        // The old trailing-LF request must be rejected without touching the
        // live composition, target text, caret or operation version.
        let rejected_request = Request {
            prefix: Some(Prefix {
                text: format!("{committed}\n"),
                expected_revision: 1,
                range_epoch: begin.range_epoch,
            }),
            ..settle.clone()
        };
        let before_rejection = editor_text(editor)?;
        let caret_before_rejection = editor_selection(editor);
        pipe.send(&serde_json::to_vec(&rejected_request)?)?;
        let rejection_started = Instant::now();
        let rejected = loop {
            pump(Duration::from_millis(10));
            if let Some(data) = pipe.receive()? {
                break serde_json::from_slice::<Reply>(&data)?;
            }
            if rejection_started.elapsed() > Duration::from_secs(2) {
                return Err("boundary rejection timeout".into());
            }
        };
        if rejected.applied
            || rejected.code != "prefixBoundaryUnsupported"
            || rejected.mutation != Mutation::NotAttempted
            || editor_text(editor)? != before_rejection
            || editor_selection(editor) != caret_before_rejection
            || composition_text(&context).is_err()
        {
            return Err("invalid boundary modified or retired the live draft".into());
        }
        println!("owned trailing-LF boundary rejected with actual text/caret unchanged=true");
        pipe.send(&serde_json::to_vec(&settle)?)?;
        let started = Instant::now();
        let reply = loop {
            pump(Duration::from_millis(10));
            if let Some(data) = pipe.receive()? {
                break serde_json::from_slice::<Reply>(&data)?;
            }
            if started.elapsed() > Duration::from_secs(2) {
                return Err("prefix response timeout".into());
            }
        };
        if reply.operation != 3
            || !reply.applied
            || !reply.settlement.is_some_and(|s| {
                s.prefix_operation == 3 && s.prefix == Mutation::Changed && s.tail_owned
            })
        {
            return Err(format!(
                "prefix settle rejected: {} mutation={:?} settlement={:?} owned_fixture_text={:?} owned_selection={:?}",
                reply.code, reply.mutation, reply.settlement, editor_text(editor)?, editor_selection(editor)
            )
            .into());
        }
        println!(
            "prefix reply epoch={} same_context={} settlement={:?}",
            reply.range_epoch,
            unsafe { manager.GetFocus()?.GetTop()? } == context,
            reply.settlement
        );
        if !tail_text.is_empty() && !generated_text_matches(&composition_text(&context)?, tail_text)
        {
            return Err(format!(
                "owned prefix range expected={tail_text:?} actual={:?}",
                composition_text(&context)?
            )
            .into());
        }
        if tail_text.is_empty() && composition_text(&context).is_ok() {
            return Err("empty tail retained a dummy composition".into());
        }
        let prefix = format!("左{committed}");
        if !editor_text(editor)?
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .starts_with(&prefix)
        {
            return Err("settled prefix is not actual committed control text".into());
        }
        if move_caret {
            let before = editor_text(editor)?;
            let before_selection = editor_selection(editor);
            unsafe {
                SendMessageW(editor, 0x00B1, Some(WPARAM(0)), Some(LPARAM(0)));
            }
            pump(Duration::from_millis(50));
            if before_selection == (0, 0) || editor_selection(editor) != (0, 0) {
                return Err("owned fixture did not move the actual caret to0".into());
            }
            let rejected = exchange(
                pipe,
                capability,
                &task,
                1,
                4,
                3,
                Action::Update,
                "不得写入",
                false,
            )?;
            if rejected.applied
                || rejected.mutation != Mutation::NotAttempted
                || editor_text(editor)? != before
            {
                return Err("empty insertion lease survived a real caret move".into());
            }
            println!(
                "owned empty-tail caret change revokes continuation with unchanged actual text=true"
            );
            continue;
        }
        if tail_text == "\n" {
            let finished = exchange(pipe, capability, &task, 1, 4, 2, Action::Commit, "", false)?;
            if !finished.applied
                || editor_text(editor)?
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    != format!("{prefix}\n右")
            {
                return Err("separator-only final draft lost its real paragraph".into());
            }
            println!("owned separator-only final draft commits exact paragraph=true");
            continue;
        }
        let carried_separator = if tail_text.starts_with('\n') {
            "\n"
        } else {
            ""
        };
        let continued = format!("{carried_separator}尾部继续🙂");
        let next_owned_tail = if tail_text.starts_with('\n') {
            "\n待撤销尾部"
        } else {
            "待撤销尾部"
        };
        let updated = exchange(
            pipe,
            capability,
            &task,
            1,
            4,
            3,
            Action::Update,
            &continued,
            false,
        )?;
        if !updated.applied || !generated_text_matches(&composition_text(&context)?, &continued) {
            return Err(format!(
                "prefix tail update failed: code={} applied={} mutation={:?}",
                updated.code, updated.applied, updated.mutation
            )
            .into());
        }
        let next = Request {
            operation: 5,
            revision: 4,
            deadline_tick: unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
                + 2000,
            text: next_owned_tail.into(),
            prefix: Some(Prefix {
                text: continued.clone(),
                expected_revision: 3,
                range_epoch: updated.range_epoch,
            }),
            ..settle
        };
        pipe.send(&serde_json::to_vec(&next)?)?;
        let started = Instant::now();
        let second = loop {
            pump(Duration::from_millis(10));
            if let Some(data) = pipe.receive()? {
                break serde_json::from_slice::<Reply>(&data)?;
            }
            if started.elapsed() > Duration::from_secs(2) {
                return Err("second prefix response timeout".into());
            }
        };
        if second.operation != 5
            || !second.applied
            || !second.settlement.is_some_and(|s| {
                s.prefix_operation == 5 && s.prefix == Mutation::Changed && s.tail_owned
            })
        {
            return Err(format!("second prefix rejected: {}", second.code).into());
        }
        if !generated_text_matches(&composition_text(&context)?, next_owned_tail) {
            return Err("second prefix did not retain latest tail".into());
        }
        let aborted = exchange(pipe, capability, &task, 1, 6, 4, Action::Abort, "", false)?;
        if !aborted.applied {
            return Err(format!("prefix tail abort rejected: {}", aborted.code).into());
        }
        if editor_text(editor)?
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            != format!("{prefix}{continued}右")
        {
            return Err(
                "prefix cancellation erased committed text or restored replaced selection".into(),
            );
        }
    }
    println!("owned prefix settle, live tail, Unicode/newline and cancel actual text=true");
    Ok(())
}
#[cfg(feature = "acceptance")]
fn editor_selection(window: windows::Win32::Foundation::HWND) -> (u32, u32) {
    use windows::Win32::Foundation::*;
    let (mut start, mut end) = (0u32, 0u32);
    unsafe {
        SendMessageW(
            window,
            0x00B0,
            Some(WPARAM((&mut start as *mut u32) as usize)),
            Some(LPARAM((&mut end as *mut u32) as isize)),
        );
    }
    (start, end)
}
#[cfg(feature = "acceptance")]
fn prefix_fault_fixture(
    component: windows::Win32::Foundation::HMODULE,
    editor: windows::Win32::Foundation::HWND,
    pipe: &Pipe,
    capability: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    use windows::{
        Win32::{Foundation::*, System::LibraryLoader::*},
        core::{s, w},
    };
    unsafe {
        let entry = GetProcAddress(component, s!("SayweaveSettleFixtureFault"))
            .ok_or("missing owned acceptance-only checkpoint")?;
        let inject: unsafe extern "system" fn(usize) = std::mem::transmute(entry);
        for mode in [1, 2, 3, 4, 5] {
            SendMessageW(
                editor,
                WM_SETTEXT,
                None,
                Some(LPARAM(w!("左原选区右").as_ptr() as isize)),
            );
            SendMessageW(editor, 0x00B1, Some(WPARAM(1)), Some(LPARAM(4)));
            // The fixture reset is an external edit. Deliver its notifications
            // before claiming a new range; this is not a product start delay.
            pump(Duration::from_millis(50));
            let task = uuid::Uuid::new_v4().to_string();
            let begin = exchange(pipe, capability, &task, 1, 1, 0, Action::Begin, "", false)?;
            if !begin.applied
                || !exchange(
                    pipe,
                    capability,
                    &task,
                    1,
                    2,
                    1,
                    Action::Update,
                    "原文尾🙂",
                    false,
                )?
                .applied
            {
                return Err("prefix fault fixture setup failed".into());
            }
            inject(mode);
            let raw_tail = if matches!(mode, 4 | 5) {
                "原文\n尾🙂"
            } else {
                "原文尾🙂"
            };
            let request = Request {
                protocol: PROTOCOL,
                capability: capability.into(),
                task: task.clone(),
                turn: 1,
                operation: 3,
                revision: 2,
                deadline_tick: windows::Win32::System::SystemInformation::GetTickCount64() + 2000,
                action: Action::Settle,
                text: raw_tail.into(),
                prefix: Some(Prefix {
                    text: "提交前段🙂".into(),
                    expected_revision: 1,
                    range_epoch: begin.range_epoch,
                }),
            };
            pipe.send(&serde_json::to_vec(&request)?)?;
            let started = Instant::now();
            let reply = loop {
                pump(Duration::from_millis(10));
                if let Some(frame) = pipe.receive()? {
                    break serde_json::from_slice::<Reply>(&frame)?;
                }
                if started.elapsed() > Duration::from_secs(2) {
                    return Err("prefix fault reply missing".into());
                }
            };
            let cancelled = matches!(mode, 1 | 5);
            if (cancelled && (!reply.applied || reply.code != "applied"))
                || (!cancelled && reply.applied)
                || reply.operation != if cancelled { 4 } else { 3 }
            {
                return Err(format!("prefix fault mode={mode} reply applied={} operation={} code={} settlement={:?}",reply.applied,reply.operation,reply.code,reply.settlement).into());
            }
            if !reply.settlement.is_some_and(|s| {
                s.prefix_operation == 3 && s.prefix == Mutation::Changed && !s.tail_owned
            }) {
                return Err("partial prefix outcome lost its irreversible effect".into());
            }
            pump(Duration::from_millis(30));
            let actual = editor_text(editor)?
                .replace("\r\n", "\n")
                .replace('\r', "\n");
            let expected_actual = if cancelled {
                "左提交前段🙂右".to_string()
            } else {
                format!("左提交前段🙂{raw_tail}右")
            };
            if actual != expected_actual {
                return Err(
                    "post-prefix cancellation/failure erased prefix or restored original selection"
                        .into(),
                );
            }
            let stale = exchange(
                pipe,
                capability,
                &task,
                1,
                5,
                3,
                Action::Update,
                "不得重复",
                false,
            )?;
            if stale.applied
                || stale.mutation != Mutation::NotAttempted
                || editor_text(editor)?
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    != actual
            {
                return Err("partial prefix failure allowed replay into target".into());
            }
            println!(
                "owned prefix checkpoint={mode} preserves actual prefix and denies replay=true"
            );
            if mode == 3 {
                if !exchange(pipe, capability, &task, 2, 1, 0, Action::Begin, "", false)?.applied {
                    return Err(
                        "interrupted prefix left native transaction busy for the next turn".into(),
                    );
                }
                if !exchange(
                    pipe,
                    capability,
                    &task,
                    2,
                    2,
                    1,
                    Action::Update,
                    "新轮次",
                    false,
                )?
                .applied
                    || !exchange(pipe, capability, &task, 2, 3, 1, Action::Commit, "", false)?
                        .applied
                {
                    return Err("interrupted prefix next turn could not write".into());
                }
                if editor_text(editor)?
                    .replace("\r\n", "\n")
                    .replace('\r', "\n")
                    != "左提交前段🙂原文尾🙂新轮次右"
                {
                    return Err("interrupted prefix old tail wrote over the new turn".into());
                }
            }
        }
    }
    Ok(())
}
fn pump(duration: Duration) {
    let until = Instant::now() + duration;
    while Instant::now() < until {
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
