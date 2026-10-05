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
struct Host {
    language: Option<u16>,
    manager: Option<ITfThreadMgr>,
    _lease: registration::HostLease,
}
impl Host {
    fn close(&mut self) -> windows::core::Result<()> {
        if let Some(language) = self.language {
            if own_profile_active(language)? {
                registration::deactivate(language, true)?;
                if own_profile_active(language)? {
                    return Err(windows::core::Error::new(
                        windows::Win32::Foundation::E_FAIL,
                        "speech profile remains active",
                    ));
                }
            }
            self.language = None;
        }
        if let Some(manager) = &self.manager {
            unsafe {
                manager.Deactivate()?;
            }
            self.manager = None;
        }
        Ok(())
    }
}
impl Drop for Host {
    fn drop(&mut self) {
        if let Err(error) = self.close() {
            eprintln!("probe_cleanup_failed hresult={:08x}", error.code().0 as u32);
        }
    }
}
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
                if input_profiles()?.iter().any(|p| {
                    p.catid == GUID_TFCAT_TIP_SPEECH && p.dwFlags & TF_IPP_FLAG_ACTIVE != 0
                }) {
                    return Err(
                        "an existing speech profile is active; probe made no changes".into(),
                    );
                }
                let capability = uuid::Uuid::new_v4().to_string();
                let mut host = Host {
                    language: None,
                    manager: None,
                    _lease: registration::host_capability(std::process::id(), &capability)?,
                };
                let keyboard_before = keyboard()?;
                let manager: ITfThreadMgr =
                    CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
                let _client = manager.Activate()?;
                host.manager = Some(manager);
                host.language = Some(registration::activate(true)?);
                if !own_profile_active(host.language.unwrap())? {
                    return Err("speech profile was not activated; no input attempted".into());
                }
                if keyboard()? != keyboard_before {
                    return Err(
                        "speech activation changed keyboard profile; acceptance failed".into(),
                    );
                }
                if args[1] == "activation-check" {
                    host.close()?;
                    if keyboard()? != keyboard_before {
                        return Err("keyboard profile changed during cleanup".into());
                    }
                    println!(
                        "speech activation/deactivation verified; keyboard unchanged; no input attempted"
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
                for (op, revision, action, text) in [
                    (1, 0, Action::Begin, ""),
                    (2, 1, Action::Update, "Sayweave 原位"),
                    (3, 2, Action::Update, "Sayweave 原位验证，🙂"),
                    (4, 2, Action::Commit, ""),
                ] {
                    if GetForegroundWindow() != window {
                        return Err("target foreground changed; stopped without retry".into());
                    }
                    let r = Request {
                        protocol: 1,
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
                    pump(Duration::from_millis(700));
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
fn own_profile_active(language: u16) -> windows::core::Result<bool> {
    unsafe {
        let mut active_language = 0;
        let mut active_profile = windows::core::GUID::zeroed();
        registration::profiles()?.GetActiveLanguageProfile(
            &registration::CLASS,
            &mut active_language,
            &mut active_profile,
        )?;
        Ok(active_language == language && active_profile == registration::PROFILE)
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
            w!("RICHEDIT50W"),
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
        let client = manager.Activate()?;
        let _ = SendMessageW(
            editor,
            WM_USER + 204,
            Some(WPARAM(0x10000)),
            Some(LPARAM(0x10000)),
        );
        let _ = SetForegroundWindow(top);
        let _ = SetFocus(Some(editor));
        pump(Duration::from_millis(200));
        let module_path = std::env::args()
            .nth(2)
            .map(std::path::PathBuf::from)
            .unwrap_or(std::env::current_exe()?.with_file_name("sayweave_text_service.dll"));
        let (component, processor) = load_processor(&module_path)?;
        processor.Activate(&manager, client)?;
        let capability = uuid::Uuid::new_v4().to_string();
        let host = registration::host_capability(std::process::id(), &capability)?;
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let context = manager.GetFocus()?.GetTop()?;
            println!(
                "owned TSF context available; readonly={}",
                context.GetStatus()?.dwStaticFlags & TF_SD_READONLY != 0
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
                    protocol: 1,
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
                if editor_text(editor)? != expected {
                    return Err("owned target text assertion failed".into());
                }
                println!("owned operation={operation} applied=true actual_text_assertion=true");
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
            let handed_off_text = editor_text(editor)?;
            let user_edit: ITfEditSession = UserPropertyEdit {
                context: context.clone(),
            }
            .into();
            context
                .RequestEditSession(0, &user_edit, TF_ES_SYNC | TF_ES_READWRITE)?
                .ok()?;
            pump(Duration::from_millis(100));
            let abort = exchange(
                &pipe,
                &capability,
                &task,
                4,
                11,
                1,
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
            println!("owned property-only user edit revoked rollback ownership=true");
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
                (12, 0, Action::Begin, ""),
                (13, 1, Action::Update, multiline),
                (14, 1, Action::Commit, ""),
            ] {
                let reply = exchange(
                    &pipe,
                    &capability,
                    &task,
                    5,
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
}
#[cfg(feature = "acceptance")]
impl ITfEditSession_Impl for UserPropertyEdit_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        unsafe {
            let range = self.context.GetStart(ec)?;
            range.ShiftEndToRange(ec, &self.context.GetEnd(ec)?, TF_ANCHOR_END)?;
            self.context.GetProperty(&GUID_PROP_LANGID)?.SetValue(
                ec,
                &range,
                &windows::Win32::System::Variant::VARIANT::from(0x409i32),
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
        protocol: 1,
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
