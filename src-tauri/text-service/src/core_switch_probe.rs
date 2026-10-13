//! Owned blank-editor core switch gate. No registry or user preference writes.
use super::*;
use sayweave_text_service::engine_abi::ProxyStatusV1;
use std::{
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};
use windows::{
    Win32::{Foundation::*, System::LibraryLoader::*, UI::Input::KeyboardAndMouse::*},
    core::{HRESULT, s, w},
};
type Prepare = unsafe extern "system" fn(*const u16, usize, *const u8) -> HRESULT;
type Dispatch = unsafe extern "system" fn(*const u8, usize, u64) -> HRESULT;
type Read = unsafe extern "system" fn(*mut u8, usize, *mut usize) -> HRESULT;
type Inspect = unsafe extern "system" fn(*mut ProxyStatusV1) -> HRESULT;
type Pin = unsafe extern "system" fn() -> HRESULT;
type Release = unsafe extern "system" fn(u64) -> HRESULT;

struct Control {
    prepare: Prepare,
    dispatch: Dispatch,
    read: Read,
    inspect: Inspect,
    pin: Pin,
    release: Release,
}
impl Control {
    unsafe fn load(module: HMODULE) -> Result<Self, Box<dyn std::error::Error>> {
        unsafe {
            Ok(Self {
                prepare: std::mem::transmute::<unsafe extern "system" fn() -> isize, Prepare>(
                    GetProcAddress(module, s!("SayweaveProxyPrepareFixture"))
                        .ok_or("prepare missing")?,
                ),
                dispatch: std::mem::transmute::<unsafe extern "system" fn() -> isize, Dispatch>(
                    GetProcAddress(module, s!("SayweaveProxyDispatchFixture"))
                        .ok_or("dispatch missing")?,
                ),
                read: std::mem::transmute::<unsafe extern "system" fn() -> isize, Read>(
                    GetProcAddress(module, s!("SayweaveProxyReadFixture")).ok_or("read missing")?,
                ),
                inspect: std::mem::transmute::<unsafe extern "system" fn() -> isize, Inspect>(
                    GetProcAddress(module, s!("SayweaveProxyInspectFixture"))
                        .ok_or("inspect missing")?,
                ),
                pin: std::mem::transmute::<unsafe extern "system" fn() -> isize, Pin>(
                    GetProcAddress(module, s!("SayweaveProxyPinFixture")).ok_or("pin missing")?,
                ),
                release: std::mem::transmute::<unsafe extern "system" fn() -> isize, Release>(
                    GetProcAddress(module, s!("SayweaveProxyReleaseFixture"))
                        .ok_or("release missing")?,
                ),
            })
        }
    }
    fn prepare(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        use sha2::{Digest, Sha256};
        let hash: [u8; 32] = Sha256::digest(std::fs::read(path)?).into();
        let path: Vec<u16> = path.as_os_str().encode_wide().collect();
        unsafe {
            (self.prepare)(path.as_ptr(), path.len(), hash.as_ptr()).ok()?;
        }
        Ok(())
    }
    fn status(&self) -> Result<ProxyStatusV1, Box<dyn std::error::Error>> {
        let mut status = ProxyStatusV1::default();
        unsafe {
            (self.inspect)(&mut status).ok()?;
        }
        Ok(status)
    }
    #[allow(clippy::too_many_arguments)]
    fn send_only(
        &self,
        capability: &str,
        task: &str,
        generation: u64,
        op: u64,
        revision: u64,
        action: Action,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let request = Request {
            revoke: None,
            block: None,
            engine: EngineBinding {
                proxy_instance: uuid::Uuid::from_bytes(self.status()?.instance).to_string(),
                engine_generation: generation,
            },
            protocol: sayweave_text_service::wire::PROTOCOL,
            capability: capability.into(),
            task: task.into(),
            turn: 1,
            operation: op,
            revision,
            deadline_tick: unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
                + 2000,
            action,
            text: text.into(),
            prefix: None,
        };
        let bytes = serde_json::to_vec(&request)?;
        unsafe {
            (self.dispatch)(bytes.as_ptr(), bytes.len(), generation).ok()?;
        }
        Ok(())
    }
    fn receive(
        &self,
        task: &str,
        generation: u64,
        op: u64,
        revision: u64,
    ) -> Result<Reply, Box<dyn std::error::Error>> {
        let started = Instant::now();
        let mut buffer = vec![0; MAX_FRAME];
        loop {
            pump(Duration::from_millis(10));
            let mut length = 0;
            unsafe {
                (self.read)(buffer.as_mut_ptr(), buffer.len(), &mut length).ok()?;
            }
            if length > 0 {
                let reply: Reply = serde_json::from_slice(&buffer[..length])?;
                if reply.operation != op
                    || reply.revision != revision
                    || reply.task != task
                    || reply.turn != 1
                    || reply.engine.engine_generation != generation
                    || !reply.applied
                {
                    return Err(format!("core edit rejected: {}", reply.code).into());
                }
                return Ok(reply);
            }
            if started.elapsed() > Duration::from_secs(2) {
                return Err("core reply timeout".into());
            }
        }
    }
    #[allow(clippy::too_many_arguments)]
    fn exchange(
        &self,
        capability: &str,
        task: &str,
        generation: u64,
        op: u64,
        revision: u64,
        action: Action,
        text: &str,
    ) -> Result<Reply, Box<dyn std::error::Error>> {
        self.send_only(capability, task, generation, op, revision, action, text)?;
        self.receive(task, generation, op, revision)
    }
}
fn receive_pipe(
    pipe: &sayweave_text_service::transport::Pipe,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let at = Instant::now();
    loop {
        pump(Duration::from_millis(5));
        if let Some(bytes) = pipe.receive()? {
            return Ok(bytes);
        }
        if at.elapsed() > Duration::from_secs(2) {
            return Err("ordinary pipe timeout".into());
        }
    }
}
fn receipt_request(
    capability: &str,
    key: &sayweave_text_service::lifecycle::ReceiptKey,
    action: sayweave_text_service::lifecycle::ControlAction,
) -> sayweave_text_service::lifecycle::ControlRequest {
    sayweave_text_service::lifecycle::ControlRequest {
        control_protocol: 1,
        capability: capability.into(),
        task: key.task.clone(),
        turn: key.turn,
        operation: key.operation,
        deadline_tick: unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
            + 1000,
        action,
        expected_core: "0".repeat(64),
        receipt: Some(key.clone()),
    }
}
fn acknowledge_pipe(
    pipe: &sayweave_text_service::transport::Pipe,
    capability: &str,
    key: &sayweave_text_service::lifecycle::ReceiptKey,
) -> Result<(), Box<dyn std::error::Error>> {
    let request = receipt_request(
        capability,
        key,
        sayweave_text_service::lifecycle::ControlAction::AcknowledgeRetired,
    );
    pipe.send(&serde_json::to_vec(&request)?)?;
    let reply: sayweave_text_service::lifecycle::ControlReply =
        serde_json::from_slice(&receive_pipe(pipe)?)?;
    if !reply.ready
        || reply.engine.as_ref() != Some(&key.engine)
        || reply.task != key.task
        || reply.operation != key.operation
    {
        return Err("receipt ack rejected".into());
    }
    Ok(())
}
pub(super) fn run(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let proxy = PathBuf::from(args.get(2).ok_or("proxy required")?);
    let cores: Vec<PathBuf> = args.iter().skip(3).map(PathBuf::from).collect();
    if cores.len() != 3 {
        return Err("three distinct real core builds A/B/C required".into());
    }
    use sha2::{Digest, Sha256};
    let hashes: Vec<[u8; 32]> = cores
        .iter()
        .map(|p| std::fs::read(p).map(|b| Sha256::digest(b).into()))
        .collect::<Result<_, _>>()?;
    if hashes[0] == hashes[1] || hashes[1] == hashes[2] || hashes[0] == hashes[2] {
        return Err("core bytes must differ, not just reported versions".into());
    }
    unsafe {
        let top = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("STATIC"),
            w!("Sayweave owned core-switch gate"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            50,
            50,
            520,
            240,
            None,
            None,
            None,
            None,
        )?;
        let editor = CreateWindowExW(
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
        )?;
        let manager: ITfThreadMgr =
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
        manager.Activate()?;
        let _ = SetForegroundWindow(top);
        let _ = SetFocus(Some(editor));
        pump(Duration::from_millis(150));
        if GetForegroundWindow() != top {
            // Only activate this newly-created blank test editor, never a user document.
            let mut rect = RECT::default();
            GetWindowRect(editor, &mut rect)?;
            SetCursorPos(rect.left + 25, rect.top + 25)?;
            let events = [MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP].map(|flag| INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dwFlags: flag,
                        ..Default::default()
                    },
                },
            });
            if SendInput(&events, std::mem::size_of::<INPUT>() as i32) != 2 {
                return Err("owned focus click failed".into());
            }
            pump(Duration::from_millis(150));
        }
        if GetForegroundWindow() != top {
            return Err("owned fixture did not become foreground; no write".into());
        }
        // Deactivate only this process's already active speech service. User/global state unchanged.
        let (activation, language) = sayweave_text_service::client::speech_activation()?;
        if activation == sayweave_text_service::client::SpeechActivation::Own {
            registration::deactivate(language, false)?;
            pump(Duration::from_millis(100));
        }
        let (module, processor) = load_processor(&proxy)?;
        let client = manager
            .cast::<ITfClientId>()?
            .GetClientId(&registration::CLASS)?;
        processor.Activate(&manager, client)?;
        let control = Control::load(module)?;
        let service_identity = processor.as_raw() as usize;
        let capability = uuid::Uuid::new_v4().to_string();
        let _operator = registration::host_capability(std::process::id(), &capability)?;
        let mut original = None;
        let mut a_generation = 0;
        let mut expected = String::new();
        let mut records = Vec::new();
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            for (index, path) in cores.iter().enumerate() {
                eprintln!(
                    "core_switch stage=prepare core={} pid={}",
                    ['A', 'B', 'C'][index],
                    std::process::id()
                );
                if index == 1 {
                    let before = control.status()?;
                    let fault: unsafe extern "system" fn(u32) -> HRESULT = std::mem::transmute(
                        GetProcAddress(module, s!("SayweaveProxyActivationFaultFixture"))
                            .ok_or("activation fault fixture missing")?,
                    );
                    fault(1).ok()?;
                    if control.prepare(path).is_ok() {
                        return Err("candidate failure falsely ready".into());
                    }
                    pump(Duration::from_millis(50));
                    let restored = control.status()?;
                    if restored.ready != 1
                        || restored.generation != before.generation
                        || restored.engine.module_sha256 != before.engine.module_sha256
                        || editor_text(editor)? != expected
                    {
                        return Err(
                            "activation failure did not restore exact prior core/body".into()
                        );
                    }
                }
                control.prepare(path)?;
                let status = control.status()?;
                if status.engine.module_sha256 != hashes[index]
                    || status.generation != status.engine.generation
                {
                    return Err("actual core identity mismatch".into());
                }
                if let Some((nonce, previous)) = original
                    && (status.instance != nonce || status.generation <= previous)
                {
                    return Err("proxy changed or generation reused".into());
                }
                original = Some((status.instance, status.generation));
                if processor.as_raw() as usize != service_identity {
                    return Err("COM service replaced".into());
                }
                if index == 0 {
                    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
                    let mut wrong_hash = hashes[index];
                    wrong_hash[0] ^= 1;
                    if (control.prepare)(wide.as_ptr(), wide.len(), wrong_hash.as_ptr()).is_ok() {
                        return Err("core identity mismatch accepted".into());
                    }
                    let retained = control.status()?;
                    if retained.generation != status.generation || retained.ready != 1 {
                        return Err("rejected identity disturbed old core".into());
                    }
                    a_generation = status.generation;
                    (control.pin)().ok()?;
                }
                if index == 1 {
                    if status.retired != 1 {
                        return Err("held A core was not retained".into());
                    }
                    (control.release)(a_generation).ok()?;
                    pump(Duration::from_millis(75));
                    if control.status()?.retired != 0 {
                        return Err("released A core failed to unload".into());
                    }
                }
                let task = uuid::Uuid::new_v4().to_string();
                let part = format!("核心{}，🙂。", ['A', 'B', 'C'][index]);
                control.exchange(
                    &capability,
                    &task,
                    status.generation,
                    1,
                    0,
                    Action::Begin,
                    "",
                )?;
                // Active input must block candidate change without disturbing the range.
                eprintln!(
                    "core_switch stage=begun core={} generation={}",
                    ['A', 'B', 'C'][index],
                    status.generation
                );
                if index < 2 {
                    let next: Vec<u16> = cores[index + 1].as_os_str().encode_wide().collect();
                    if (control.prepare)(next.as_ptr(), next.len(), hashes[index + 1].as_ptr())
                        .is_ok()
                    {
                        return Err("active range admitted core switch".into());
                    }
                }
                control.exchange(
                    &capability,
                    &task,
                    status.generation,
                    2,
                    1,
                    Action::Update,
                    &part,
                )?;
                control.exchange(
                    &capability,
                    &task,
                    status.generation,
                    3,
                    1,
                    Action::Commit,
                    "",
                )?;
                expected.push_str(&part);
                let actual = editor_text(editor)?;
                eprintln!(
                    "core_switch stage=body core={} actual={actual:?}",
                    ['A', 'B', 'C'][index]
                );
                if actual != expected {
                    return Err(format!(
                        "actual body mismatch: expected={expected:?} got={actual:?}"
                    )
                    .into());
                }
                let stale = Request {
                    revoke: None,
                    block: None,
                    engine: EngineBinding {
                        proxy_instance: uuid::Uuid::from_bytes(status.instance).to_string(),
                        engine_generation: status.generation.saturating_sub(1),
                    },
                    protocol: sayweave_text_service::wire::PROTOCOL,
                    capability: capability.clone(),
                    task,
                    turn: 1,
                    operation: 4,
                    revision: 2,
                    deadline_tick: windows::Win32::System::SystemInformation::GetTickCount64()
                        + 1000,
                    action: Action::Update,
                    text: "stale".into(),
                    prefix: None,
                };
                let bytes = serde_json::to_vec(&stale)?;
                if (control.dispatch)(bytes.as_ptr(), bytes.len(), status.generation - 1).is_ok() {
                    return Err("old generation executed".into());
                }
                if editor_text(editor)? != expected {
                    return Err("stale request changed body".into());
                }
                records.push(serde_json::json!({"core":(['A','B','C'][index]),"generation":status.generation,"moduleBase":status.engine.module_base,
                    "sha256":format!("{:x}",Sha256::digest(std::fs::read(path)?)),"body":actual}));
            }
            // Use the ordinary authenticated pipe and control handlers for receipt recovery.
            let use_transport: Pin = std::mem::transmute(
                GetProcAddress(module, s!("SayweaveProxyUseTransportFixture"))
                    .ok_or("transport fixture missing")?,
            );
            use_transport().ok()?;
            let last = control.status()?;
            let task = uuid::Uuid::new_v4().to_string();
            let binding = EngineBinding {
                proxy_instance: uuid::Uuid::from_bytes(last.instance).to_string(),
                engine_generation: last.generation,
            };
            let pipe = sayweave_text_service::transport::Pipe::client(
                std::process::id(),
                windows::Win32::System::Threading::GetCurrentThreadId(),
            )?;
            let request = |operation, revision, action, text: &str| Request {
                revoke: None,
                block: None,
                engine: binding.clone(),
                protocol: sayweave_text_service::wire::PROTOCOL,
                capability: capability.clone(),
                task: task.clone(),
                turn: 1,
                operation,
                revision,
                deadline_tick: windows::Win32::System::SystemInformation::GetTickCount64() + 2000,
                action,
                text: text.into(),
                prefix: None,
            };
            for (op, revision, action, text) in
                [(1, 0, Action::Begin, ""), (2, 1, Action::Update, "收尾。")]
            {
                pipe.send(&serde_json::to_vec(&request(op, revision, action, text))?)?;
                let bytes = receive_pipe(&pipe)?;
                let reply: Reply = serde_json::from_slice(&bytes)?;
                if !reply.applied || reply.engine != binding || reply.task != task {
                    return Err("ordinary pipe edit failed".into());
                }
                let key = sayweave_text_service::lifecycle::ReceiptKey::from_frame(&bytes)
                    .ok_or("receipt identity")?;
                acknowledge_pipe(&pipe, &capability, &key)?;
            }
            pipe.send(&serde_json::to_vec(&request(3, 1, Action::Commit, ""))?)?;
            pump(Duration::from_millis(80));
            if control.prepare(&cores[0]).is_ok() {
                return Err("switch discarded unacknowledged commit receipt".into());
            }
            let _ = processor.Deactivate();
            drop(pipe);
            pump(Duration::from_millis(80));
            let pipe = sayweave_text_service::transport::Pipe::client(
                std::process::id(),
                windows::Win32::System::Threading::GetCurrentThreadId(),
            )?;
            let key = sayweave_text_service::lifecycle::ReceiptKey {
                engine: binding.clone(),
                task: task.clone(),
                turn: 1,
                operation: 3,
                revision: 1,
                sha256: None,
            };
            let read = receipt_request(
                &capability,
                &key,
                sayweave_text_service::lifecycle::ControlAction::ReadRetired,
            );
            pipe.send(&serde_json::to_vec(&read)?)?;
            let bytes = receive_pipe(&pipe)?;
            let receipt: Reply = serde_json::from_slice(&bytes)?;
            if !receipt.applied || receipt.task != task || receipt.engine != binding {
                return Err("closing lost original text receipt".into());
            }
            let key = sayweave_text_service::lifecycle::ReceiptKey::from_frame(&bytes)
                .ok_or("closing identity")?;
            acknowledge_pipe(&pipe, &capability, &key)?;
            if editor_text(editor)? != format!("{expected}收尾。") {
                return Err("closing changed body".into());
            }
            println!(
                "{}",
                serde_json::json!({"prototypePassed":true,"pid":std::process::id(),"thread":windows::Win32::System::Threading::GetCurrentThreadId(),
                "sameService":true,"sameProxy":true,"heldLeaseThenUnloaded":true,"staleRejected":true,"unreadReceiptRetainedAndDrained":true,"productionReceiptTransport":true,"candidateActivationFailureRestored":true,"wrongCoreHashRejectedBeforeActivation":true,"records":records})
            );
            Ok(())
        })();
        if let Err(cause) = &result {
            eprintln!("core_switch first_failure={cause}");
        }
        let stopped = processor.Deactivate();
        drop(processor);
        let _ = manager.Deactivate();
        let _ = DestroyWindow(top);
        pump(Duration::from_millis(75));
        let can: unsafe extern "system" fn() -> HRESULT = std::mem::transmute(
            GetProcAddress(module, s!("DllCanUnloadNow")).ok_or("proxy unload export missing")?,
        );
        let unloaded = can() == S_OK;
        if unloaded {
            FreeLibrary(module)?;
        }
        result?;
        stopped?;
        if !unloaded {
            return Err("proxy has outstanding resources after cleanup".into());
        }
        println!("core_switch cleanup_complete=true");
        Ok(())
    }
}

enum MultiCommand {
    Switch(PathBuf),
    CloseDuringActivation(PathBuf),
    Inspect,
    Stop,
}
pub(super) fn multi(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::mpsc;
    let proxy = PathBuf::from(args.get(2).ok_or("proxy required")?);
    let core = PathBuf::from(args.get(3).ok_or("core A required")?);
    let next = PathBuf::from(args.get(4).ok_or("core B required")?);
    let mut workers = Vec::new();
    for _ in 0..2 {
        let (send, receive) = mpsc::channel();
        let (report, events) = mpsc::channel::<Result<ProxyStatusV1, String>>();
        let proxy = proxy.clone();
        let core = core.clone();
        let thread = std::thread::spawn(move || {
            let outcome = (|| -> Result<(), Box<dyn std::error::Error>> {
                unsafe {
                    CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
                    let _apartment = Apartment;
                    let manager: ITfThreadMgr =
                        CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
                    manager.Activate()?;
                    let (activation, language) =
                        sayweave_text_service::client::speech_activation()?;
                    if activation == sayweave_text_service::client::SpeechActivation::Own {
                        registration::deactivate(language, false)?;
                        pump(Duration::from_millis(50));
                    }
                    let (module, processor) = load_processor(&proxy)?;
                    let client = manager
                        .cast::<ITfClientId>()?
                        .GetClientId(&registration::CLASS)?;
                    processor.Activate(&manager, client)?;
                    let control = Control::load(module)?;
                    control.prepare(&core)?;
                    report.send(Ok(control.status()?))?;
                    loop {
                        pump(Duration::from_millis(10));
                        match receive.try_recv() {
                            Ok(MultiCommand::Switch(path)) => {
                                control.prepare(&path)?;
                                report.send(Ok(control.status()?))?;
                            }
                            Ok(MultiCommand::CloseDuringActivation(path)) => {
                                let fault: unsafe extern "system" fn(u32) -> HRESULT =
                                    std::mem::transmute(
                                        GetProcAddress(
                                            module,
                                            s!("SayweaveProxyActivationFaultFixture"),
                                        )
                                        .ok_or("activation reentry fixture missing")?,
                                    );
                                fault(2).ok()?;
                                if control.prepare(&path).is_ok() {
                                    return Err("shutdown epoch published candidate".into());
                                }
                                pump(Duration::from_millis(75));
                                if control.status().is_ok() {
                                    return Err("shutdown retained STA window/engine".into());
                                }
                                report.send(Ok(ProxyStatusV1::default()))?;
                            }
                            Ok(MultiCommand::Inspect) => {
                                report.send(Ok(control.status()?))?;
                            }
                            Ok(MultiCommand::Stop) | Err(mpsc::TryRecvError::Disconnected) => break,
                            Err(mpsc::TryRecvError::Empty) => (),
                        }
                    }
                    processor.Deactivate()?;
                    drop(processor);
                    manager.Deactivate()?;
                    // Each thread drops only its own loader reference; the other
                    // STA's independently held reference keeps the proxy mapped.
                    FreeLibrary(module)?;
                    Ok(())
                }
            })();
            if let Err(error) = outcome {
                let _ = report.send(Err(error.to_string()));
            }
        });
        workers.push((send, events, thread));
    }
    let status = |index: usize| -> Result<ProxyStatusV1, Box<dyn std::error::Error>> {
        workers[index]
            .1
            .recv_timeout(Duration::from_secs(5))?
            .map_err(Into::into)
    };
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let a = status(0)?;
        let b = status(1)?;
        if a.instance == b.instance || a.engine.module_base != b.engine.module_base {
            return Err("STA identities or shared module leases mismatched".into());
        }
        workers[0].0.send(MultiCommand::Switch(next))?;
        let switched = status(0)?;
        workers[1].0.send(MultiCommand::Inspect)?;
        let retained = status(1)?;
        if switched.engine.module_sha256 == a.engine.module_sha256
            || retained.engine.module_sha256 != b.engine.module_sha256
            || retained.engine.active != 1
        {
            return Err("switch on one STA invalidated another STA".into());
        }
        workers[0]
            .0
            .send(MultiCommand::CloseDuringActivation(core.clone()))?;
        if status(0)?.ready != 0 {
            return Err("reentry core falsely ready".into());
        }
        workers[1].0.send(MultiCommand::Inspect)?;
        if status(1)?.engine.active != 1 {
            return Err("reentry affected another STA".into());
        }
        println!(
            "{}",
            serde_json::json!({"multiSta":true,"oneSwitchedOtherRetained":true,"activationShutdownEpochRejected":true,"pid":std::process::id(),
            "firstGeneration":switched.generation,"secondGeneration":retained.generation})
        );
        Ok(())
    })();
    for (send, _, _) in &workers {
        let _ = send.send(MultiCommand::Stop);
    }
    for (_, events, thread) in workers {
        thread.join().map_err(|_| "STA worker panicked")?;
        if let Ok(Err(error)) = events.try_recv() {
            return Err(error.into());
        }
    }
    result
}
