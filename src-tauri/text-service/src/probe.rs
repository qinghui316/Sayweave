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
                if !(5..=30).contains(&focus_seconds) {
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
                let interval = args
                    .get(5)
                    .map(|s| s.parse::<u64>())
                    .transpose()?
                    .unwrap_or(700);
                if interval > 1000 {
                    return Err("probe interval exceeds limit".into());
                }
                for (op, revision, action, text) in [
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
                ] {
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
                    if reply.operation != op || !reply.applied {
                        return Err("target did not apply edit".into());
                    }
                    if action == Action::Inspect && !reply.compatible_runtime(pid, thread) {
                        return Err("target runtime mismatch; no input attempted".into());
                    }
                    pump(Duration::from_millis(interval));
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
        let library = LoadLibraryExW(w!("Msftedit.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)?;
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
        let manager: ITfThreadMgr =
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
        manager.Activate()?;
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
        pump(Duration::from_millis(200));
        if GetForegroundWindow() != top {
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
            )?;
            let mut rect = RECT::default();
            GetWindowRect(editor, &mut rect)?;
            SetCursorPos(rect.left + 30, rect.top + 30)?;
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
        let (activation, language) = sayweave_text_service::client::speech_activation()?;
        if activation == sayweave_text_service::client::SpeechActivation::Own {
            registration::deactivate(language, false)?;
            pump(Duration::from_millis(100));
        }
        let (component, processor) = load_processor(&module_path)?;
        let service_client = manager
            .cast::<ITfClientId>()?
            .GetClientId(&registration::CLASS)?;
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
            if plain {
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
                    return Err("plain property-affected cancellation lost visible text".into());
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
