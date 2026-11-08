//! Ordinary signed-release proxy, ordinary control/data pipes, owned blank editor.
//! This gate may stage machine components ONLY on a fresh GitHub-hosted runner.
use super::*;
use sayweave_text_service::{deployment, lifecycle::*};
use std::{
    io::{BufRead, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::mpsc,
};
use windows::{
    Win32::{
        Foundation::*,
        System::{LibraryLoader::*, SystemInformation::GetTickCount64, Threading::*},
        UI::Input::KeyboardAndMouse::SetFocus,
    },
    core::{HRESULT, s, w},
};

type Outcome<T> = Result<T, Box<dyn std::error::Error>>;
fn checked<T, E: std::fmt::Display>(phase: &str, call: &str, result: Result<T, E>) -> Outcome<T> {
    result.map_err(|error| format!("formal_target phase={phase} call={call}: {error}").into())
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
    if args.get(1).is_some_and(|v| v == "formal-proxy-target") {
        return target(args);
    }
    let input_fixture = args.get(1).is_some_and(|v| v == "formal-input-fixture");
    if input_fixture && args.len() == 3 {
        let module = Path::new(&args[2]);
        let descriptor = deployment::current_engine()?.ok_or("installed descriptor missing")?;
        deployment::verified_core(&descriptor)?;
        let registration =
            registration::snapshot_installer_machine()?.ok_or("installed registration missing")?;
        if std::fs::canonicalize(&registration.module)? != std::fs::canonicalize(module)?
            || deployment::installer_module_hash(&registration.module)? != descriptor.proxy_sha256
        {
            return Err("installed component identity differs".into());
        }
        let language = unsafe { registration::profiles()?.GetCurrentLanguage()? };
        exercise(
            module,
            &descriptor,
            &descriptor,
            &mut None,
            language,
            "input",
        )?;
        return Ok(());
    }
    deployment::authorize_installer()?;
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
    if args.get(1).is_some_and(|v| v == "formal-proxy-upgrade") {
        return upgrade(args);
    }
    let proxy = std::fs::read(args.get(2).ok_or("proxy required")?)?;
    let old = std::fs::read(args.get(3).ok_or("old core required")?)?;
    let new = args.get(4).map(std::fs::read).transpose()?;
    if !input_fixture && new.as_ref().is_none_or(|new| old == *new) {
        return Err("core must really change".into());
    }
    let (module, first) = deployment::prepare_engine(&proxy, &old)?;
    // Existing registration is rejected by register_installer_machine as well.
    let language = unsafe { registration::profiles()?.GetCurrentLanguage()? };
    let user_before = registration::installer_user_choice(language)?;
    if user_before.is_some() {
        return Err("fresh runner user profile required".into());
    }
    registration::register_installer_machine(&module, language)?;
    let mut current = None;
    let result = (|| {
        if !registration::initialize_installer_user(language, user_before)? {
            return Err("owned runner profile not enabled".into());
        }
        deployment::publish_engine(None, Some(&first))?;
        current = Some(first.clone());
        if input_fixture {
            await_baseline_publication(language, &module)?;
            let mut child = Target::spawn(
                Command::new(std::env::current_exe()?).args(["formal-input-fixture", &module]),
            )?;
            let proof = child.event("formalTargetPassed")?;
            child.finish()?;
            println!("{proof}");
            return Ok(());
        }
        let second = if let Some(new) = &new {
            let (same, second) = deployment::prepare_engine(&proxy, new)?;
            if same != module {
                return Err("proxy path changed".into());
            }
            second
        } else {
            first.clone()
        };
        exercise(
            Path::new(&module),
            &first,
            &second,
            &mut current,
            language,
            "",
        )
        .map(|_| ())
    })();
    // Compare-and-restore only our descriptor/registration; never overwrite drift.
    let cleanup = registration::restore_installer_user(language, user_before)
        .and_then(|()| deployment::publish_engine(current.as_ref(), None))
        .and_then(|()| registration::restore_installer_machine(None, &module));
    if let Err(error) = cleanup {
        return Err(format!("cleanup failed: {error}; exercise={result:?}").into());
    }
    result?;
    println!("formal_core_switch cleanup_complete=true");
    Ok(())
}

fn prepare_reply(pipe: &Pipe, capability: &str, hash: &str) -> Outcome<ControlReply> {
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
    if reply.task != request.task || reply.operation != 1 || reply.turn != 1 {
        return Err(format!("ordinary prepare failed: {}", reply.code).into());
    }
    Ok(reply)
}
fn prepare(pipe: &Pipe, capability: &str, hash: &str) -> Outcome<ControlReply> {
    let reply = prepare_reply(pipe, capability, hash)?;
    if !reply.ready {
        return Err(format!("ordinary prepare rejected: {}", reply.code).into());
    }
    Ok(reply)
}
fn current_payload(expected: &CurrentEngine) -> Outcome<()> {
    let actual = deployment::current_engine()?.ok_or("current descriptor missing")?;
    // install_bundle creates its own revision; never pretend the staged descriptor
    // is the committed transaction. Verify the committed payload and protocols.
    if !actual.valid()
        || actual.proxy_sha256 != expected.proxy_sha256
        || actual.core_sha256 != expected.core_sha256
        || actual.abi != expected.abi
        || actual.input_protocol != expected.input_protocol
        || actual.platform != expected.platform
    {
        return Err("parent transaction payload differs".into());
    }
    Ok(())
}

fn exercise(
    proxy: &Path,
    first: &CurrentEngine,
    second: &CurrentEngine,
    current: &mut Option<CurrentEngine>,
    language: u16,
    phase: &str,
) -> Outcome<serde_json::Value> {
    unsafe {
        let top = checked(
            phase,
            "window.create",
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
            ),
        )?;
        let editor = checked(
            phase,
            "editor.create",
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
            ),
        )?;
        let manager: ITfThreadMgr = checked(
            phase,
            "thread-manager.create",
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER),
        )?;
        checked(phase, "thread-manager.activate", manager.Activate())?;
        let _ = SetForegroundWindow(top);
        let _ = SetFocus(Some(editor));
        pump(Duration::from_millis(150));
        // TSF must create/activate the registered TIP and supply its client ID.
        // A manually created service's GetClientId is not TIP activation.
        let module = checked(phase, "module.load", load_fixture_module(proxy))?;
        let keyboard_before = checked(phase, "keyboard.snapshot", keyboard())?;
        let result = (|| -> Outcome<serde_json::Value> {
            let input_fixture = phase == "input";
            if input_fixture {
                let core = deployment::verified_core(first)?;
                let core_module = load_fixture_module(&core)?;
                let has_fault_export = [module, core_module].into_iter().any(|handle| {
                    GetProcAddress(handle, s!("SayweaveSettleFixtureFault")).is_some()
                        || GetProcAddress(handle, s!("SayweaveInitialFixtureFault")).is_some()
                        || GetProcAddress(handle, s!("SayweaveBlockFixtureFault")).is_some()
                });
                FreeLibrary(core_module)?;
                if has_fault_export {
                    return Err("ordinary input component exposes a test fault hook".into());
                }
                println!("owned production DLL fault export absent=true");
            }
            let activated = checked(phase, "profile.activate", registration::activate(false))?;
            if activated != language
                || !checked(
                    phase,
                    "profile.read",
                    sayweave_text_service::client::own_profile_active(language),
                )?
                || checked(phase, "keyboard.compare", keyboard())? != keyboard_before
            {
                return Err("owned process TIP activation identity differs".into());
            }
            pump(Duration::from_millis(150));
            println!("formal_core_switch activation=tsf_process_owned");
            if GetForegroundWindow() != top {
                return Err("owned editor must remain foreground".into());
            }
            let capability = uuid::Uuid::new_v4().to_string();
            let _host = registration::host_capability(std::process::id(), &capability)?;
            let pipe = Pipe::client(std::process::id(), GetCurrentThreadId())?;
            let mut records = Vec::new();
            let mut previous: Option<Runtime> = None;
            let mut expected = String::new();
            let mut steps = vec![(first, "旧核心输入。")];
            if phase != "old" && first.core_sha256 != second.core_sha256 {
                steps.push((second, "新核心输入。"));
            }
            for (descriptor, text) in steps {
                if descriptor != first {
                    if phase.is_empty() {
                        deployment::publish_engine(current.as_ref(), Some(descriptor))?;
                    } else {
                        current_payload(descriptor)?;
                    }
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
                let writes: &[(u64, u64, Action, &str)] = if input_fixture {
                    &[
                        (1, 0, Action::Begin, ""),
                        (2, 1, Action::Update, "原文"),
                        (3, 2, Action::Update, "原文尾🙂"),
                        (4, 3, Action::Settle, "尾🙂"),
                        (5, 4, Action::Update, "完整尾句。"),
                        (6, 4, Action::Commit, ""),
                    ]
                } else {
                    &[
                        (1, 0, Action::Begin, ""),
                        (2, 1, Action::Update, text),
                        (3, 1, Action::Commit, ""),
                    ]
                };
                let mut range_epoch = 0;
                for &(operation, revision, action, body) in writes {
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
                        prefix: (action == Action::Settle).then(|| Prefix {
                            text: "整理前段🙂".into(),
                            expected_revision: revision - 1,
                            range_epoch,
                        }),
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
                        return Err(
                            format!("ordinary write {operation} failed: {}", reply.code).into()
                        );
                    }
                    let key = ReceiptKey::from_frame(&frame).ok_or("receipt missing")?;
                    core_switch_probe::acknowledge_pipe(&pipe, &capability, &key)?;
                    if action == Action::Begin {
                        range_epoch = reply.range_epoch;
                    }
                    if input_fixture
                        && action == Action::Update
                        && reply.mutation != Mutation::Changed
                    {
                        return Err(
                            format!("ordinary update {operation} did not change text").into()
                        );
                    }
                    if action == Action::Settle
                        && !reply.settlement.is_some_and(|value| {
                            value.prefix_operation == operation
                                && value.prefix == Mutation::Changed
                                && value.tail_owned
                        })
                    {
                        return Err("ordinary prefix did not settle with a writable tail".into());
                    }
                }
                expected.push_str(if input_fixture {
                    "整理前段🙂完整尾句。"
                } else {
                    text
                });
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
                if matches!(phase, "old" | "new") && descriptor == first {
                    println!(
                        "{}",
                        serde_json::json!({"stageReady":phase,"pid":std::process::id(),"records":records})
                    );
                    std::io::stdout().flush()?;
                    wait_parent(if phase == "old" {
                        "upgraded"
                    } else {
                        "switched"
                    })?;
                }
            }
            let mut refusal = None;
            if phase == "old" {
                current_payload(second)?;
                // New admission must fail; do not send business text on the old binding.
                let reply = prepare_reply(&pipe, &capability, &second.core_sha256)?;
                if reply.ready
                    || reply.runtime.is_some()
                    || reply.engine.is_some()
                    || reply.code != "enginePrepare:80070005"
                {
                    return Err(format!("old proxy admitted new task: {}", reply.code).into());
                }
                pump(Duration::from_millis(250));
                if editor_text(editor)? != expected {
                    return Err("old target text changed after rejected admission".into());
                }
                refusal = Some(reply.code);
            }
            let evidence = serde_json::json!({"formalProxySwitchPassed":!input_fixture,"formalTargetPassed":true,"phase":phase,"processIdentity":sayweave_text_service::transport::identity(std::process::id())?,"pid":std::process::id(),"sameService":true,"ordinaryTransport":true,"records":records,"oldAdmissionRejected":refusal,"finalBody":editor_text(editor)?});
            Ok(evidence)
        })();
        let stopped = registration::deactivate(language, false);
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
        let mut evidence = result?;
        stopped?;
        deactivated?;
        destroyed?;
        if !unloaded && phase != "old" {
            return Err("proxy resources remain".into());
        }
        // The old release can retain its known shutdown pipe until process exit.
        // Never force-unload it; the parent separately proves physical exit.
        evidence["moduleUnloadReady"] = serde_json::json!(unloaded);
        evidence["cleanupComplete"] = serde_json::json!(true);
        if phase == "input" {
            println!("owned installed production DLL actual text=true");
            println!("owned COM DLL unloaded with no live leases=true");
        }
        println!("{evidence}");
        Ok(evidence)
    }
}

fn wait_parent(expected: &'static str) -> Outcome<()> {
    let (send, receive) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = std::io::stdin().read_line(&mut line).map(|_| line);
        let _ = send.send(result);
    });
    let until = Instant::now() + Duration::from_secs(30);
    loop {
        if let Ok(line) = receive.try_recv() {
            if line?.trim() != expected {
                return Err("parent event differs".into());
            }
            return Ok(());
        }
        if Instant::now() >= until {
            return Err("parent event timeout".into());
        }
        pump(Duration::from_millis(10));
    }
}

struct Target {
    child: Child,
    lines: mpsc::Receiver<String>,
}
impl Drop for Target {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}
impl Target {
    fn spawn(command: &mut Command) -> Outcome<Self> {
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let stdout = child.stdout.take().ok_or("child stdout")?;
        let (send, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(stdout).lines() {
                match line {
                    Ok(line) => {
                        if send.send(line).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        Ok(Self { child, lines })
    }
    fn launch(
        module: &str,
        first: &CurrentEngine,
        second: &CurrentEngine,
        phase: &str,
        language: u16,
    ) -> Outcome<Self> {
        Self::spawn(Command::new(std::env::current_exe()?).args([
            "formal-proxy-target",
            module,
            &serde_json::to_string(first)?,
            &serde_json::to_string(second)?,
            phase,
            &language.to_string(),
        ]))
    }
    fn event(&mut self, key: &str) -> Outcome<serde_json::Value> {
        self.event_before(key, Instant::now() + Duration::from_secs(40))
    }
    fn event_before(&mut self, key: &str, until: Instant) -> Outcome<serde_json::Value> {
        loop {
            let line = self
                .lines
                .recv_timeout(until.saturating_duration_since(Instant::now()))?;
            println!("target: {line}");
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line)
                && value.get(key).is_some()
            {
                return Ok(value);
            }
        }
    }
    fn signal(&mut self, value: &str) -> Outcome<()> {
        let stdin = self.child.stdin.as_mut().ok_or("child stdin")?;
        writeln!(stdin, "{value}")?;
        stdin.flush()?;
        Ok(())
    }
    fn finish(&mut self) -> Outcome<()> {
        self.finish_before(Instant::now() + Duration::from_secs(10))
    }
    fn finish_before(&mut self, until: Instant) -> Outcome<()> {
        loop {
            if let Some(status) = self.child.try_wait()? {
                if !status.success() {
                    return Err("owned target failed cleanup".into());
                }
                return Ok(());
            }
            if Instant::now() >= until {
                return Err("owned target exit timeout".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

fn await_baseline_publication(language: u16, module: &str) -> Outcome<()> {
    // Reuse the existing registration-query-detail probe and the installation
    // acceptance readiness contract. Registration success is not publication.
    let until = Instant::now() + Duration::from_secs(10);
    let class = format!("{:?}", registration::CLASS);
    let profile = format!("{:?}", registration::PROFILE);
    let category = format!("{GUID_TFCAT_TIP_SPEECH:?}");
    let mut stable = 0;
    loop {
        let mut query =
            Target::spawn(Command::new(std::env::current_exe()?).arg("registration-query-detail"))?;
        let reading = query.event_before("managerProfiles", until)?;
        query.finish_before(until)?;
        let profiles = reading["managerProfiles"]
            .as_array()
            .ok_or("profile query shape")?;
        let categories = reading["categories"]
            .as_array()
            .ok_or("category query shape")?;
        let ready = reading["class"] == class
            && reading["expectedProfile"] == profile
            && reading["legacyLanguage"] == language
            && profiles.len() == 1
            && profiles[0]["clsid"] == class
            && profiles[0]["profile"] == profile
            && profiles[0]["category"] == category
            && profiles[0]["language"] == language
            && profiles[0]["type"] == 1
            && profiles[0]["flags"]
                .as_u64()
                .is_some_and(|flags| flags & TF_IPP_FLAG_ENABLED as u64 != 0)
            && categories.len() == 1
            && categories[0] == category
            && reading["roots"]["machine"]["com"] == true
            && reading["roots"]["machine"]["tip"] == true
            && reading["roots"]["user"]["com"] == false;
        // Root presence alone is not identity. Preserve the existing exact
        // protected-module anchor in addition to the published profile proof.
        if registration::owned_module()?.as_deref() != Some(module) {
            return Err("baseline registration changed".into());
        }
        stable = if ready { stable + 1 } else { 0 };
        println!(
            "{}",
            serde_json::json!({"baselinePublication":true,"ready":ready,"consecutive":stable,"reading":reading})
        );
        if Instant::now() >= until {
            return Err("baseline profile publication timeout".into());
        }
        if stable == 2 {
            return Ok(());
        }
        std::thread::sleep(
            Duration::from_millis(250).min(until.saturating_duration_since(Instant::now())),
        );
    }
}

fn target(args: &[String]) -> Outcome<()> {
    let module = args.get(2).ok_or("target module")?;
    let first: CurrentEngine = serde_json::from_str(args.get(3).ok_or("target first")?)?;
    let second: CurrentEngine = serde_json::from_str(args.get(4).ok_or("target second")?)?;
    let phase = args.get(5).ok_or("target phase")?;
    if !first.valid() || !second.valid() || !matches!(phase.as_str(), "old" | "new") {
        return Err("target identities".into());
    }
    let language = args.get(6).ok_or("target language")?.parse()?;
    exercise(
        Path::new(module),
        &first,
        &second,
        &mut Some(first.clone()),
        language,
        phase,
    )?;
    println!("formal_proxy_target cleanup_complete=true");
    Ok(())
}

fn upgrade(args: &[String]) -> Outcome<()> {
    let proxy = std::fs::read(args.get(2).ok_or("new proxy")?)?;
    let old = std::fs::read(args.get(3).ok_or("old core")?)?;
    let new = std::fs::read(args.get(4).ok_or("new core")?)?;
    let old_proxy = std::fs::read(args.get(5).ok_or("old proxy")?)?;
    if deployment::version(&proxy) == deployment::version(&old_proxy) {
        return Err("proxy must really change".into());
    }
    let (old_module, old_descriptor) = deployment::prepare_engine(&old_proxy, &old)?;
    let (new_module, first) = deployment::prepare_engine(&proxy, &old)?;
    let (_, second) = deployment::prepare_engine(&proxy, &new)?;
    let language = unsafe { registration::profiles()?.GetCurrentLanguage()? };
    let user_before = registration::installer_user_choice(language)?;
    if user_before.is_some() {
        return Err("fresh runner user profile required".into());
    }
    registration::register_installer_machine(&old_module, language)?;
    let sid = sayweave_text_service::transport::current_user_sid()?;
    let mut installed = false;
    let mut baseline_published = false;
    let result = (|| -> Outcome<()> {
        if !registration::initialize_installer_user(language, user_before)? {
            return Err("owned runner profile not enabled".into());
        }
        await_baseline_publication(language, &old_module)?;
        deployment::publish_engine(None, Some(&old_descriptor))?;
        baseline_published = true;
        let mut previous = Target::launch(&old_module, &old_descriptor, &first, "old", language)?;
        let old_ready = previous.event("stageReady")?;
        deployment::install_bundle(
            &proxy,
            &old,
            &sid,
            Default::default(),
            Instant::now() + Duration::from_secs(10),
        )?;
        installed = true;
        previous.signal("upgraded")?;
        let old_result = previous.event("formalTargetPassed")?;
        previous.finish()?;
        drop(previous);
        let first = deployment::current_engine()?.ok_or("committed proxy descriptor")?;
        let mut fresh = Target::launch(&new_module, &first, &second, "new", language)?;
        let new_ready = fresh.event("stageReady")?;
        if old_ready["pid"] == new_ready["pid"] {
            return Err("target was not reopened".into());
        }
        if old != new {
            deployment::install_bundle(
                &proxy,
                &new,
                &sid,
                Default::default(),
                Instant::now() + Duration::from_secs(10),
            )?;
        }
        fresh.signal("switched")?;
        let new_result = fresh.event("formalTargetPassed")?;
        fresh.finish()?;
        println!(
            "{}",
            serde_json::json!({"formalProxySwitchPassed":true,"proxyUpgradePassed":true,"oldTargetExited":true,"newTargetExited":true,"coreChanged":old!=new,"oldTarget":old_result,"newTarget":new_result})
        );
        Ok(())
    })();
    // The transaction retains its immutable original old-proxy anchor across core-only changes.
    let cleanup = (|| -> Outcome<()> {
        if installed {
            let current = deployment::current_engine()?.ok_or("cleanup descriptor")?;
            let core = if current.core_sha256 == deployment::version(&new) {
                &new
            } else {
                &old
            };
            deployment::rollback_bundle(&proxy, core, &sid)?;
        }
        registration::restore_installer_user(language, user_before)?;
        deployment::publish_engine(baseline_published.then_some(&old_descriptor), None)?;
        registration::restore_installer_machine(None, &old_module)?;
        if registration::snapshot_installer_machine()?.is_some()
            || deployment::current_engine()?.is_some()
            || registration::read_host().is_some()
            || deployment::core_update_pending()?
        {
            return Err("upgrade resources remain".into());
        }
        Ok(())
    })();
    cleanup.map_err(|e| format!("upgrade cleanup failed: {e}; exercise={result:?}"))?;
    result?;
    println!("formal_proxy_upgrade cleanup_complete=true");
    Ok(())
}
