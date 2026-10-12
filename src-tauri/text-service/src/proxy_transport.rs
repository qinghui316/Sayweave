//! Stable control/auth shell. Native business frames stay opaque to the proxy.
use super::*;
use crate::{
    deployment,
    lifecycle::*,
    wire::{EngineBinding, ModuleIdentity, Runtime},
};
use windows::Win32::System::SystemInformation::GetTickCount64;
pub(super) fn own_identity() -> windows::core::Result<ModuleIdentity> {
    let module = own_module()?;
    let mut path = vec![0; 32768];
    let length = unsafe { GetModuleFileNameW(Some(HMODULE(module.0)), &mut path) } as usize;
    if length == 0 || length >= path.len() {
        return Err(error(E_FAIL.0));
    }
    let path = String::from_utf16(&path[..length]).map_err(|_| error(E_FAIL.0))?;
    let bytes = std::fs::read(path).map_err(|_| error(E_FAIL.0))?;
    use sha2::{Digest, Sha256};
    Ok(ModuleIdentity {
        base: module.0 as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    })
}
fn disconnect_peer(shared: &Shared) {
    let api = {
        let mut state = shared.borrow_mut();
        if let Some(pipe) = &state.pipe {
            pipe.disconnect();
        }
        state.connected = false;
        state.peer = None;
        state.capability = None;
        state.current.as_ref().map(|c| c.api)
    };
    if let Some(api) = api {
        unsafe {
            let _ = (api.disconnect)(api.handle);
        }
    }
}
fn incoming(shared: &Shared) -> windows::core::Result<Option<Vec<u8>>> {
    let mut state = shared.borrow_mut();
    if state.window.is_invalid()
        || state.switching
        || state.fixture_connection
        || state.calls.get() != 0
    {
        return Ok(None);
    }
    let pipe = state
        .pipe
        .as_ref()
        .ok_or_else(|| error(TF_E_DISCONNECTED.0))?;
    if !state.connected {
        if !pipe.connect() {
            return Ok(None);
        }
        state.connected = true;
    }
    let (host, capability) = registration::read_host().ok_or_else(|| error(E_ACCESSDENIED.0))?;
    let peer = identity(host)?;
    if peer.session != crate::transport::session(std::process::id())?
        || state
            .pipe
            .as_ref()
            .ok_or_else(|| error(E_UNEXPECTED.0))?
            .client_pid()?
            != host
        || state.peer.is_some_and(|old| old != peer)
        || state
            .capability
            .as_ref()
            .is_some_and(|old| old != &capability)
    {
        return Err(error(E_ACCESSDENIED.0));
    }
    state.peer = Some(peer);
    state.capability = Some(capability);
    state
        .pipe
        .as_ref()
        .ok_or_else(|| error(E_UNEXPECTED.0))?
        .receive()
}
fn send(shared: &Shared, data: &[u8]) -> windows::core::Result<()> {
    if data.len() > MAX_FRAME {
        return Err(error(E_INVALIDARG.0));
    }
    let state = shared.borrow();
    state
        .pipe
        .as_ref()
        .ok_or_else(|| error(TF_E_DISCONNECTED.0))?
        .send(data)
}
fn runtime(shared: &Shared) -> windows::core::Result<Runtime> {
    let state = shared.borrow();
    let status = state
        .current
        .as_ref()
        .ok_or_else(|| error(TF_E_DISCONNECTED.0))?
        .status()?;
    Ok(Runtime {
        protocol: status.input_protocol,
        process: std::process::id(),
        thread: unsafe { windows::Win32::System::Threading::GetCurrentThreadId() },
        engine: EngineBinding {
            proxy_instance: uuid::Uuid::from_bytes(state.instance).to_string(),
            engine_generation: state.generation,
        },
        core: Some(ModuleIdentity {
            base: status.module_base,
            sha256: status
                .module_sha256
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        }),
        proxy: state.proxy_identity.clone(),
    })
}
fn control(shared: &Shared, request: ControlRequest) -> windows::core::Result<()> {
    let now = unsafe { GetTickCount64() };
    if !request.valid(now) || shared.borrow().capability.as_deref() != Some(&request.capability) {
        return Err(error(E_ACCESSDENIED.0));
    }
    if matches!(
        request.action,
        ControlAction::ReadRetired | ControlAction::AcknowledgeRetired
    ) {
        return receipt_control(shared, &request);
    }
    let result = (|| -> windows::core::Result<Runtime> {
        let descriptor = deployment::current_engine()?.ok_or_else(|| error(E_NOINTERFACE.0))?;
        if descriptor.core_sha256 != request.expected_core
            || shared
                .borrow()
                .proxy_identity
                .as_ref()
                .is_none_or(|v| v.sha256 != descriptor.proxy_sha256)
        {
            return Err(error(E_ACCESSDENIED.0));
        }
        let path = deployment::verified_core(&descriptor)?;
        match request.action {
            ControlAction::PrepareEngine => {
                let hash: [u8; 32] = hex(&descriptor.core_sha256)?;
                prepare(shared, &path, &hash)?;
            }
            ControlAction::QueryEngine => (),
            ControlAction::DrainRetired => {
                collect(shared);
            }
            ControlAction::ReadRetired | ControlAction::AcknowledgeRetired => unreachable!(),
        }
        if unsafe { GetTickCount64() } > request.deadline_tick
            || deployment::current_engine()?.as_ref() != Some(&descriptor)
        {
            return Err(error(TF_E_DISCONNECTED.0));
        }
        let runtime = runtime(shared)?;
        if runtime.protocol != descriptor.input_protocol
            || runtime
                .core
                .as_ref()
                .is_none_or(|core| core.sha256 != request.expected_core)
            || !shared.borrow().active
            || shared.borrow().shutdown
            || shared.borrow().failed
        {
            return Err(error(E_UNEXPECTED.0));
        }
        Ok(runtime)
    })();
    let reply = match result {
        Ok(runtime) => ControlReply {
            control_protocol: 1,
            task: request.task,
            turn: request.turn,
            operation: request.operation,
            ready: true,
            code: "ready".into(),
            engine: Some(runtime.engine.clone()),
            runtime: Some(runtime),
            retired: shared.borrow().retired.len() as u32,
        },
        Err(cause) => ControlReply {
            control_protocol: 1,
            task: request.task,
            turn: request.turn,
            operation: request.operation,
            ready: false,
            code: if cause.code().0 == BUSY {
                "engineBusy".into()
            } else {
                format!("enginePrepare:{:08x}", cause.code().0 as u32)
            },
            engine: None,
            runtime: None,
            retired: shared.borrow().retired.len() as u32,
        },
    };
    send(
        shared,
        &serde_json::to_vec(&reply).map_err(|_| error(E_FAIL.0))?,
    )
}
fn retain(shared: &Shared, bytes: Vec<u8>) -> windows::core::Result<()> {
    let key = ReceiptKey::from_frame(&bytes).ok_or_else(|| error(E_FAIL.0))?;
    let mut state = shared.borrow_mut();
    if state.retained_replies.len() >= 16 {
        return Err(error(BUSY));
    }
    let owner = state.reply_owner.clone();
    state
        .retained_replies
        .push(RetainedReply { key, owner, bytes });
    Ok(())
}
fn receipt_control(shared: &Shared, request: &ControlRequest) -> windows::core::Result<()> {
    // The current authenticated publication is also required by incoming().
    // Another process or a reused PID cannot acknowledge the original owner's effect.
    collect(shared);
    let key = request
        .receipt
        .as_ref()
        .ok_or_else(|| error(E_INVALIDARG.0))?;
    let index = {
        let state = shared.borrow();
        let owner = state.peer.zip(state.capability.clone());
        state
            .retained_replies
            .iter()
            .position(|r| r.owner.is_some() && r.owner == owner && key.matches(&r.key))
    };
    let ack = matches!(request.action, ControlAction::AcknowledgeRetired);
    let previously_acknowledged = {
        let state = shared.borrow();
        let owner = state.peer.zip(state.capability.clone());
        ack && state
            .acknowledged
            .iter()
            .any(|(old, old_owner)| *old_owner == owner && key.matches(old))
    };
    if !ack && let Some(index) = index {
        let bytes = shared.borrow().retained_replies[index].bytes.clone();
        return send(shared, &bytes);
    }
    let reply = ControlReply {
        control_protocol: 1,
        task: request.task.clone(),
        turn: request.turn,
        operation: request.operation,
        ready: index.is_some() || previously_acknowledged,
        code: if index.is_some() || previously_acknowledged {
            "receiptAcknowledged"
        } else {
            "receiptUnavailable"
        }
        .into(),
        engine: Some(key.engine.clone()),
        runtime: None,
        retired: shared.borrow().retired.len() as u32,
    };
    // Sending the confirmation must succeed before consuming the retained bytes.
    send(
        shared,
        &serde_json::to_vec(&reply).map_err(|_| error(E_FAIL.0))?,
    )?;
    if ack && let Some(index) = index {
        let mut state = shared.borrow_mut();
        let receipt = state.retained_replies.remove(index);
        if state.acknowledged.len() >= 16 {
            state.acknowledged.remove(0);
        }
        state.acknowledged.push((receipt.key, receipt.owner));
    }
    Ok(())
}
fn hex(value: &str) -> windows::core::Result<[u8; 32]> {
    if value.len() != 64 {
        return Err(error(E_INVALIDARG.0));
    }
    let mut bytes = [0; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| error(E_INVALIDARG.0))?;
    }
    Ok(bytes)
}
fn business(shared: &Shared, bytes: &[u8]) -> windows::core::Result<()> {
    if shared.borrow().retained_replies.len() >= 16 {
        return Err(error(BUSY));
    }
    let header: InputHeader = serde_json::from_slice(bytes).map_err(|_| error(E_INVALIDARG.0))?;
    let expected = runtime(shared)?;
    if !header.valid(unsafe { GetTickCount64() })
        || header.engine != expected.engine
        || header.protocol != expected.protocol
        || shared.borrow().capability.as_deref() != Some(&header.capability)
    {
        return Err(error(E_ACCESSDENIED.0));
    }
    let (_call, api) = enter(shared)?;
    unsafe {
        checked((api.dispatch)(api.handle, bytes.as_ptr(), bytes.len()))?;
    }
    let owner = {
        let state = shared.borrow();
        state.peer.zip(state.capability.clone())
    };
    shared.borrow_mut().reply_owner = owner;
    Ok(())
}
fn outgoing(shared: &Shared) -> windows::core::Result<()> {
    if !shared.borrow().connected
        || shared.borrow().fixture_connection
        || shared.borrow().retained_replies.len() >= 16
    {
        return Ok(());
    }
    let mut data = vec![0; MAX_FRAME];
    let mut length = 0;
    {
        let (_call, api) = enter(shared)?;
        unsafe {
            checked((api.read)(
                api.handle,
                data.as_mut_ptr(),
                data.len(),
                &mut length,
            ))?;
        }
    }
    if length == 0 {
        return Ok(());
    }
    data.truncate(length);
    let mut reply: serde_json::Value =
        serde_json::from_slice(&data).map_err(|_| error(E_FAIL.0))?;
    let engine: EngineBinding =
        serde_json::from_value(reply["engine"].clone()).map_err(|_| error(E_FAIL.0))?;
    if engine != runtime(shared)?.engine {
        retain(shared, data)?;
        return Err(error(E_ACCESSDENIED.0));
    }
    if reply["runtime"].is_object() {
        reply["runtime"]["proxy"] = serde_json::to_value(shared.borrow().proxy_identity.clone())
            .map_err(|_| error(E_FAIL.0))?;
    }
    let bytes = serde_json::to_vec(&reply).map_err(|_| error(E_FAIL.0))?;
    // Sending to the OS pipe is not proof the controller received the effect.
    // Keep every business outcome until its exact hash is acknowledged.
    retain(shared, bytes.clone())?;
    send(shared, &bytes)?;
    Ok(())
}
pub(super) fn poll(shared: &Shared) {
    let result = (|| -> windows::core::Result<()> {
        if let Some(bytes) = incoming(shared)? {
            let value: serde_json::Value =
                serde_json::from_slice(&bytes).map_err(|_| error(E_INVALIDARG.0))?;
            if value.get("control_protocol").is_some() {
                control(
                    shared,
                    serde_json::from_slice(&bytes).map_err(|_| error(E_INVALIDARG.0))?,
                )?;
            } else {
                business(shared, &bytes)?;
            }
        }
        if shared.borrow().current.is_some()
            && shared.borrow().active
            && !shared.borrow().switching
            && !shared.borrow().failed
            && !shared.borrow().shutdown
        {
            outgoing(shared)?;
        }
        Ok(())
    })();
    if result.is_err() {
        disconnect_peer(shared);
    }
}
