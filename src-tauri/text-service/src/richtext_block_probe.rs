//! Bounded script for the existing owned rich-text fixture. No registry writes.
use super::*;
use sayweave_text_service::{deployment, lifecycle::*, transport};
use std::io::{BufRead, Write};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    action: Action,
    #[serde(default)]
    block: Option<BlockCommand>,
    #[serde(default)]
    text: String,
    turn: u64,
    revision: u64,
}

fn receive<T: serde::de::DeserializeOwned>(pipe: &Pipe, timeout: Duration) -> Result<T> {
    Ok(serde_json::from_slice(&receive_frame(pipe, timeout)?)?)
}
fn receive_frame(pipe: &Pipe, timeout: Duration) -> Result<Vec<u8>> {
    let start = Instant::now();
    loop {
        pump(Duration::from_millis(5));
        if let Some(frame) = pipe.receive()? {
            return Ok(frame);
        }
        if start.elapsed() >= timeout {
            return Err("owned rich-text acknowledgement timeout; no replay".into());
        }
    }
}

pub(super) fn run(args: &[String]) -> Result<()> {
    let expected: u32 = args.get(2).ok_or("owned PID required")?.parse()?;
    if expected == 0
        || std::env::var("SAYWEAVE_TSF_OWNED_PID").ok().as_deref()
            != Some(expected.to_string().as_str())
        || std::env::var("SAYWEAVE_INPUT_ACCEPTANCE").as_deref() != Ok("1")
    {
        return Err("owned fixture identity required".into());
    }
    let identity = transport::identity(expected)?;
    let capability = uuid::Uuid::new_v4().to_string();
    let _host = registration::host_capability(std::process::id(), &capability)?;
    let before = keyboard()?;
    let mut profile = sayweave_text_service::client::ProfileLease::open()?;
    let result = script(expected, identity, &capability);
    profile.close()?;
    if keyboard()? != before {
        return Err("owned rich-text script changed keyboard".into());
    }
    result
}

fn script(expected: u32, identity: transport::Identity, capability: &str) -> Result<()> {
    let window = unsafe { GetForegroundWindow() };
    let mut process = 0;
    let thread = unsafe { GetWindowThreadProcessId(window, Some(&mut process)) };
    if process != expected {
        return Err("owned foreground differs; no input attempted".into());
    }
    let started = Instant::now();
    let pipe = loop {
        if let Ok(pipe) = Pipe::client(process, thread) {
            break pipe;
        }
        if started.elapsed() >= Duration::from_secs(3) {
            return Err("owned bridge unavailable".into());
        }
        pump(Duration::from_millis(20));
    };
    let descriptor = deployment::current_engine()?.ok_or("no installed engine")?;
    let task = uuid::Uuid::new_v4().to_string();
    let control = ControlRequest {
        control_protocol: 1,
        capability: capability.into(),
        task: task.clone(),
        turn: 1,
        operation: 1,
        deadline_tick: unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
            + 15_000,
        action: ControlAction::PrepareEngine,
        expected_core: descriptor.core_sha256.clone(),
        receipt: None,
    };
    pipe.send(&serde_json::to_vec(&control)?)?;
    let ready: ControlReply = receive(&pipe, Duration::from_secs(15))?;
    let runtime = ready.runtime.as_ref().ok_or("missing execution identity")?;
    let engine = ready.engine.as_ref().ok_or("missing engine binding")?;
    if !ready.ready
        || ready.task != task
        || ready.operation != 1
        || runtime.process != process
        || runtime.thread != thread
        || runtime.engine != *engine
        || runtime.protocol != PROTOCOL
        || runtime.core.as_ref().map(|m| &m.sha256) != Some(&descriptor.core_sha256)
        || runtime.proxy.as_ref().map(|m| &m.sha256) != Some(&descriptor.proxy_sha256)
    {
        return Err(format!("owned engine identity rejected: {}", ready.code).into());
    }
    println!(
        "{}",
        serde_json::json!({"ready":ready,"processIdentity":identity})
    );
    std::io::stdout().flush()?;
    let (send, commands) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if send.send(line).is_err() {
                break;
            }
        }
    });
    let start = Instant::now();
    let mut operation = 0;
    loop {
        pump(Duration::from_millis(5));
        if start.elapsed() >= Duration::from_secs(60) {
            return Err("owned rich-text script total deadline".into());
        }
        let line = match commands.try_recv() {
            Ok(line) => line,
            Err(std::sync::mpsc::TryRecvError::Empty) => continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
        };
        if line == "exit" {
            break;
        }
        if unsafe { GetForegroundWindow() } != window || !transport::same_process_alive(identity)? {
            return Err("owned target changed; no further write".into());
        }
        let step: Step = serde_json::from_str(&line)?;
        operation += 1;
        let request = Request {
            revoke: None,
            prefix: None,
            block: step.block,
            engine: engine.clone(),
            protocol: PROTOCOL,
            capability: capability.into(),
            task: task.clone(),
            turn: step.turn,
            operation,
            revision: step.revision,
            deadline_tick: unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
                + 2000,
            action: step.action,
            text: step.text,
        };
        pipe.send(&serde_json::to_vec(&request)?)?;
        let frame = receive_frame(&pipe, Duration::from_secs(2))?;
        let reply: Reply = serde_json::from_slice(&frame)?;
        if reply.task != task
            || reply.engine != *engine
            || reply.operation != operation
            || reply.turn != request.turn
            || reply.revision != request.revision
        {
            return Err("owned acknowledgement identity differs".into());
        }
        // Reading a frame does not settle the proxy's retained effect record.
        // Use its exact serialized hash, as the ordinary input client does.
        let receipt = ReceiptKey::from_frame(&frame).ok_or("owned receipt identity missing")?;
        let acknowledge = ControlRequest {
            control_protocol: 1,
            capability: capability.into(),
            task: task.clone(),
            turn: request.turn,
            operation,
            deadline_tick: unsafe { windows::Win32::System::SystemInformation::GetTickCount64() }
                + 2000,
            action: ControlAction::AcknowledgeRetired,
            expected_core: descriptor.core_sha256.clone(),
            receipt: Some(receipt),
        };
        pipe.send(&serde_json::to_vec(&acknowledge)?)?;
        let acknowledged: ControlReply = receive(&pipe, Duration::from_secs(2))?;
        if !acknowledged.ready
            || acknowledged.task != task
            || acknowledged.turn != request.turn
            || acknowledged.operation != operation
            || acknowledged.engine.as_ref() != Some(engine)
            || acknowledged.code != "receiptAcknowledged"
        {
            return Err("owned effect receipt acknowledgement failed".into());
        }
        println!("{}", serde_json::json!({"request":request,"reply":reply}));
        std::io::stdout().flush()?;
    }
    Ok(())
}
