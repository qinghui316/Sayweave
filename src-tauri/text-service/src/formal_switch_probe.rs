//! Ordinary signed-release proxy, ordinary control/data pipes, owned blank editor.
//! This gate may stage machine components ONLY on a fresh GitHub-hosted runner.
use super::*;
use sayweave_text_service::{deployment, lifecycle::*};
use std::path::Path;
use windows::{
    Win32::{
        Foundation::*,
        System::{LibraryLoader::*, SystemInformation::GetTickCount64, Threading::*},
        UI::Input::KeyboardAndMouse::SetFocus,
    },
    core::{HRESULT, s, w},
};

type Outcome<T> = Result<T, Box<dyn std::error::Error>>;
macro_rules! checked {
    ($stage:literal, $operation:expr) => {{
        println!("formal_core_switch stage={}", $stage);
        $operation.map_err(|error| format!("{}: {error}", $stage))
    }};
}
struct MachineLock(Pipe);
impl Drop for MachineLock {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.0.0);
        }
    }
}
pub(super) fn run(args: &[String]) -> Outcome<()> {
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
        || std::env::var("RUNNER_ENVIRONMENT").as_deref() != Ok("github-hosted")
    {
        return Err(
            "formal gate requires a disposable GitHub-hosted runner; local deployment forbidden"
                .into(),
        );
    }
    checked!("authorize_installer", deployment::authorize_installer())?;
    let lock = Pipe(unsafe { CreateMutexW(None, false, w!("Global\\Sayweave.TSF.Installer.v1"))? });
    if !matches!(
        unsafe { WaitForSingleObject(lock.0, 0) },
        WAIT_OBJECT_0 | WAIT_ABANDONED
    ) {
        return Err("maintenance busy".into());
    }
    let _lock = MachineLock(lock);
    if registration::snapshot_installer_machine()?.is_some()
        || registration::read_host().is_some()
        || deployment::current_engine()?.is_some()
        || deployment::installer_record()?.is_some()
        || deployment::core_update_pending()?
        || deployment::cleanup_pending()?
    {
        return Err("fresh runner required; existing installation preserved".into());
    }
    let proxy = std::fs::read(args.get(2).ok_or("proxy required")?)?;
    let old = std::fs::read(args.get(3).ok_or("old core required")?)?;
    let new = std::fs::read(args.get(4).ok_or("new core required")?)?;
    if deployment::version(&old) == deployment::version(&new) {
        return Err("core must really change".into());
    }
    let (module, first) = checked!("stage_old_core", deployment::prepare_engine(&proxy, &old))?;
    // Existing registration is rejected by register_installer_machine as well.
    let language = unsafe {
        checked!(
            "current_language",
            registration::profiles()?.GetCurrentLanguage()
        )?
    };
    let user_before = registration::installer_user_choice(language)?;
    if user_before.is_some() {
        return Err("fresh runner user profile required".into());
    }
    checked!(
        "register_machine",
        registration::register_installer_machine(&module, language)
    )?;
    let mut current = None;
    let result = (|| {
        if !checked!(
            "enable_runner_profile",
            registration::initialize_installer_user(language, user_before)
        )? {
            return Err("owned runner profile not enabled".into());
        }
        checked!(
            "publish_old_core",
            deployment::publish_engine(None, Some(&first))
        )?;
        current = Some(first.clone());
        let (same, second) = checked!("stage_new_core", deployment::prepare_engine(&proxy, &new))?;
        if same != module {
            return Err("proxy path changed".into());
        }
        exercise(Path::new(&module), &first, &second, &mut current, language)
    })();
    // Compare-and-restore only our descriptor/registration; never overwrite drift.
    let cleanup = registration::restore_installer_user(language, user_before)
        .and_then(|()| deployment::publish_engine(current.as_ref(), None))
        .and_then(|()| registration::restore_installer_machine(None, &module));
    if let Err(error) = cleanup {
        return Err(format!("cleanup failed: {error}; exercise={result:?}").into());
    }
    println!("formal_core_switch machine_restore_confirmed=true");
    result?;
    println!("formal_core_switch cleanup_complete=true");
    Ok(())
}

fn prepare(pipe: &Pipe, capability: &str, hash: &str) -> Outcome<ControlReply> {
    let request = ControlRequest {
        control_protocol: 1,
        capability: capability.into(),
        task: uuid::Uuid::new_v4().to_string(),
        turn: 1,
        operation: 1,
        deadline_tick: unsafe { GetTickCount64() } + 2000,
        action: ControlAction::PrepareEngine,
        expected_core: hash.into(),
        receipt: None,
    };
    pipe.send(&serde_json::to_vec(&request)?)?;
    let reply: ControlReply = serde_json::from_slice(&core_switch_probe::receive_pipe(pipe)?)?;
    if !reply.ready || reply.task != request.task || reply.operation != 1 || reply.turn != 1 {
        return Err(format!("ordinary prepare failed: {}", reply.code).into());
    }
    Ok(reply)
}

fn exercise(
    proxy: &Path,
    first: &CurrentEngine,
    second: &CurrentEngine,
    current: &mut Option<CurrentEngine>,
    language: u16,
) -> Outcome<()> {
    unsafe {
        let top = checked!(
            "create_window",
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("STATIC"),
                w!("Sayweave formal core switch"),
                WS_OVERLAPPEDWINDOW | WS_VISIBLE,
                50,
                50,
                520,
                240,
                None,
                None,
                None,
                None,
            )
        )?;
        let editor = checked!(
            "create_editor",
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
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
            )
        )?;
        let manager: ITfThreadMgr = checked!(
            "create_thread_manager",
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)
        )?;
        checked!("activate_thread_manager", manager.Activate())?;
        let _ = SetForegroundWindow(top);
        let _ = SetFocus(Some(editor));
        pump(Duration::from_millis(150));
        // TSF must create/activate the registered TIP and supply its client ID.
        // A manually created service's GetClientId is not TIP activation.
        let module = checked!("load_proxy", load_fixture_module(proxy))?;
        let keyboard_before = checked!("keyboard_before", keyboard())?;
        let context_error = manager
            .GetFocus()
            .and_then(|document| document.GetTop())
            .err()
            .map(|e| e.code());
        let class_result: windows::core::Result<ITfTextInputProcessor> =
            CoCreateInstance(&registration::CLASS, None, CLSCTX_INPROC_SERVER);
        let class_error = class_result.as_ref().err().map(|e| e.code());
        println!(
            "formal_core_switch context_error={context_error:?} class_error={class_error:?} foreground={} language={language}",
            GetForegroundWindow() == top
        );
        drop(class_result);
        let capability = uuid::Uuid::new_v4().to_string();
        let _host = checked!(
            "host_capability",
            registration::host_capability(std::process::id(), &capability)
        )?;
        let result = (|| -> Outcome<()> {
            // Match ProfileLease's production activation scope. This fresh CI
            // session has no user installation or other Sayweave host.
            let activated = checked!("activate_session_tip", registration::activate(true))?;
            if activated != language
                || !checked!(
                    "verify_active_tip",
                    sayweave_text_service::client::own_profile_active(language)
                )?
                || checked!("keyboard_after", keyboard())? != keyboard_before
            {
                return Err("owned process TIP activation identity differs".into());
            }
            pump(Duration::from_millis(150));
            println!("formal_core_switch activation=tsf_session_owned");
            if GetForegroundWindow() != top {
                return Err("owned editor must remain foreground".into());
            }
            let pipe = checked!(
                "connect_pipe",
                Pipe::client(std::process::id(), GetCurrentThreadId())
            )?;
            let mut records = Vec::new();
            let mut previous: Option<Runtime> = None;
            let mut expected = String::new();
            for (descriptor, text) in [(first, "旧核心输入。"), (second, "新核心输入。")]
            {
                if descriptor != first {
                    deployment::publish_engine(current.as_ref(), Some(descriptor))?;
                    *current = Some(descriptor.clone());
                }
                let ready = prepare(&pipe, &capability, &descriptor.core_sha256)?;
                let runtime = ready.runtime.ok_or("runtime missing")?;
                let binding = ready.engine.ok_or("binding missing")?;
                if runtime.engine != binding
                    || runtime.process != std::process::id()
                    || runtime.thread != GetCurrentThreadId()
                    || runtime
                        .core
                        .as_ref()
                        .is_none_or(|m| m.sha256 != descriptor.core_sha256)
                    || runtime
                        .proxy
                        .as_ref()
                        .is_none_or(|m| m.sha256 != descriptor.proxy_sha256)
                {
                    return Err("executing identity differs".into());
                }
                if let Some(before) = &previous
                    && (before.engine.proxy_instance != binding.proxy_instance
                        || before.engine.engine_generation >= binding.engine_generation
                        || before.proxy.as_ref().unwrap().base
                            != runtime.proxy.as_ref().unwrap().base)
                {
                    return Err("proxy instance changed or core did not switch".into());
                }
                let task = uuid::Uuid::new_v4().to_string();
                for (operation, revision, action, body) in [
                    (1, 0, Action::Begin, ""),
                    (2, 1, Action::Update, text),
                    (3, 1, Action::Commit, ""),
                ] {
                    if GetForegroundWindow() != top {
                        return Err("owned focus lost; no write".into());
                    }
                    let request = Request {
                        revoke: None,
                        block: None,
                        engine: binding.clone(),
                        protocol: PROTOCOL,
                        capability: capability.clone(),
                        task: task.clone(),
                        turn: 1,
                        operation,
                        revision,
                        deadline_tick: GetTickCount64() + 2000,
                        action,
                        text: body.into(),
                        prefix: None,
                    };
                    pipe.send(&serde_json::to_vec(&request)?)?;
                    let frame = core_switch_probe::receive_pipe(&pipe)?;
                    let reply: Reply = serde_json::from_slice(&frame)?;
                    if !reply.applied
                        || reply.engine != binding
                        || reply.task != task
                        || reply.operation != operation
                        || reply.revision != revision
                    {
                        return Err(format!("ordinary write failed: {}", reply.code).into());
                    }
                    let key = ReceiptKey::from_frame(&frame).ok_or("receipt missing")?;
                    core_switch_probe::acknowledge_pipe(&pipe, &capability, &key)?;
                }
                expected.push_str(text);
                // Check after pumping all host events, and continuously before the next switch.
                let until = Instant::now() + Duration::from_millis(250);
                while Instant::now() < until {
                    pump(Duration::from_millis(10));
                    if editor_text(editor)? != expected {
                        return Err("host body changed after receipt".into());
                    }
                }
                records.push(serde_json::json!({"runtime":runtime,"body":editor_text(editor)?}));
                previous = Some(runtime);
            }
            println!(
                "{}",
                serde_json::json!({"formalProxySwitchPassed":true,"processIdentity":sayweave_text_service::transport::identity(std::process::id())?,"pid":std::process::id(),"sameService":true,"ordinaryTransport":true,"records":records})
            );
            Ok(())
        })();
        let stopped = match sayweave_text_service::client::own_profile_active(language) {
            Ok(true) => registration::deactivate(language, true),
            Ok(false) => Ok(()),
            Err(error) => Err(error),
        };
        let deactivated = manager.Deactivate();
        let destroyed = DestroyWindow(top);
        pump(Duration::from_millis(100));
        let can: unsafe extern "system" fn() -> HRESULT = std::mem::transmute(
            GetProcAddress(module, s!("DllCanUnloadNow")).ok_or("unload export")?,
        );
        let unloaded = can() == S_OK;
        if unloaded {
            FreeLibrary(module)?;
        }
        if stopped.is_err() || deactivated.is_err() || destroyed.is_err() || !unloaded {
            return Err(format!("host cleanup failed: tip={stopped:?}, manager={deactivated:?}, window={destroyed:?}, unloaded={unloaded}; exercise={result:?}").into());
        }
        println!("formal_core_switch host_restore_confirmed=true");
        result
    }
}
