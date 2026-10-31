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
        state.handoff_fence = true;
        state.recovery_receiver = None;
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
    if matches!(request.action, ControlAction::PrepareEngine) {
        shared.borrow_mut().prepare_busy = None;
    }
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
    if matches!(request.action, ControlAction::DrainRetired) {
        return offer_retired(shared, &request);
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
        authorized_owner(shared, &request.capability)?;
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
                format!(
                    "engineBusy:{}",
                    if matches!(request.action, ControlAction::PrepareEngine) {
                        shared.borrow().prepare_busy.unwrap_or("control")
                    } else {
                        "control"
                    }
                )
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
pub(super) fn retain(shared: &Shared, bytes: Vec<u8>) -> windows::core::Result<()> {
    let key = ReceiptKey::from_frame(&bytes).ok_or_else(|| error(E_FAIL.0))?;
    let mut state = shared.borrow_mut();
    if state.retained_replies.iter().any(|reply| reply.key == key) {
        return Ok(());
    }
    if state.retained_replies.len() >= 16 {
        return Err(error(BUSY));
    }
    let owner = state
        .reply_owners
        .iter()
        .find(|(original, _)| original.matches(&key))
        .map(|(_, owner)| owner.clone())
        .or_else(|| {
            state
                .retained_replies
                .iter()
                .find(|reply| reply.key == key)
                .and_then(|reply| reply.owner.clone())
        })
        .or_else(|| {
            state
                .acknowledged
                .iter()
                .find(|reply| reply.key == key)
                .and_then(|reply| reply.owner.clone())
        });
    state
        .reply_owners
        .retain(|(original, _)| !original.matches(&key));
    state.retained_replies.push(RetainedReply {
        key,
        owner,
        receiver: None,
        bytes,
    });
    Ok(())
}
fn current_owner(shared: &Shared) -> Option<ReplyOwner> {
    let state = shared.borrow();
    state.peer.zip(state.capability.clone())
}
fn authorized_owner(shared: &Shared, capability: &str) -> windows::core::Result<ReplyOwner> {
    let owner = current_owner(shared).ok_or_else(|| error(E_ACCESSDENIED.0))?;
    if owner.1 != capability
        || registration::read_host() != Some((owner.0.pid, owner.1.clone()))
        || identity(owner.0.pid)? != owner.0
        || !shared
            .borrow()
            .pipe
            .as_ref()
            .is_some_and(|pipe| pipe.alive() && pipe.client_pid().ok() == Some(owner.0.pid))
    {
        return Err(error(E_ACCESSDENIED.0));
    }
    Ok(owner)
}
fn offered_to(reply: &RetainedReply, owner: &Option<ReplyOwner>) -> bool {
    reply.owner.is_some()
        && &reply.owner != owner
        && reply.receiver.is_some()
        && &reply.receiver == owner
}
fn offer_retired(shared: &Shared, request: &ControlRequest) -> windows::core::Result<()> {
    collect(shared);
    if !request.valid(unsafe { GetTickCount64() }) {
        return Err(error(E_ACCESSDENIED.0));
    }
    let receiver = authorized_owner(shared, &request.capability)?;
    let candidate = {
        let state = shared.borrow();
        state
            .retained_replies
            .iter()
            .find(|reply| reply.owner.as_ref().is_some_and(|owner| owner != &receiver))
            .map(|reply| (reply.key.clone(), reply.owner.clone().unwrap()))
    };
    if let Some((key, original)) = candidate {
        use sha2::{Digest, Sha256};
        let offer = ReceiptRecovery {
            control_protocol: 1,
            task: request.task.clone(),
            turn: request.turn,
            operation: request.operation,
            code: "receiptRecovery".into(),
            receipt: key.clone(),
            owner: ReceiptOwner {
                process: original.0.pid,
                session: original.0.session,
                created: original.0.created,
                capability_sha256: format!("{:x}", Sha256::digest(original.1.as_bytes())),
            },
        };
        send(
            shared,
            &serde_json::to_vec(&offer).map_err(|_| error(E_FAIL.0))?,
        )?;
        // Offering result data never restores the original publication or write permission.
        if authorized_owner(shared, &request.capability)? != receiver {
            return Err(error(E_ACCESSDENIED.0));
        }
        if !request.valid(unsafe { GetTickCount64() }) {
            return Err(error(E_ACCESSDENIED.0));
        }
        let mut state = shared.borrow_mut();
        let reply = state
            .retained_replies
            .iter_mut()
            .find(|reply| reply.key == key && reply.owner.as_ref() == Some(&original))
            .ok_or_else(|| error(E_UNEXPECTED.0))?;
        reply.receiver = Some(receiver);
        state.recovery_receiver = state.peer.zip(state.capability.clone());
        return Ok(());
    }
    let state = shared.borrow();
    let blocked = !state.retained_replies.is_empty();
    let pending = state.shutdown && !state.retired.is_empty();
    let reply = ControlReply {
        control_protocol: 1,
        task: request.task.clone(),
        turn: request.turn,
        operation: request.operation,
        ready: !blocked && !pending,
        code: if blocked {
            "receiptOwnerUnavailable"
        } else if pending {
            "receiptsPending"
        } else {
            "receiptsDrained"
        }
        .into(),
        engine: None,
        runtime: None,
        retired: state.retired.len() as u32,
    };
    drop(state);
    send(
        shared,
        &serde_json::to_vec(&reply).map_err(|_| error(E_FAIL.0))?,
    )?;
    if reply.ready {
        if !request.valid(unsafe { GetTickCount64() })
            || authorized_owner(shared, &request.capability)? != receiver
        {
            return Err(error(E_ACCESSDENIED.0));
        }
        shared.borrow_mut().recovery_receiver = None;
    }
    Ok(())
}
fn receipt_control(shared: &Shared, request: &ControlRequest) -> windows::core::Result<()> {
    // The current authenticated publication is also required by incoming().
    // Another process or a reused PID cannot acknowledge the original owner's effect.
    collect(shared);
    if !request.valid(unsafe { GetTickCount64() }) {
        return Err(error(E_ACCESSDENIED.0));
    }
    let authenticated = authorized_owner(shared, &request.capability)?;
    let key = request
        .receipt
        .as_ref()
        .ok_or_else(|| error(E_INVALIDARG.0))?;
    let index = {
        let state = shared.borrow();
        let owner = Some(authenticated.clone());
        state.retained_replies.iter().position(|r| {
            r.owner.is_some() && (r.owner == owner || offered_to(r, &owner)) && key.matches(&r.key)
        })
    };
    let ack = matches!(request.action, ControlAction::AcknowledgeRetired);
    let previously_acknowledged = {
        let state = shared.borrow();
        let owner = Some(authenticated.clone());
        ack && state.acknowledged.iter().any(|old| {
            (old.owner == owner || old.receiver == owner)
                && owner.is_some()
                && key.matches(&old.key)
        })
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
    if !request.valid(unsafe { GetTickCount64() })
        || authorized_owner(shared, &request.capability)? != authenticated
    {
        return Err(error(E_ACCESSDENIED.0));
    }
    if ack && let Some(index) = index {
        let mut state = shared.borrow_mut();
        let receipt = state.retained_replies.remove(index);
        if state.acknowledged.len() >= 16 {
            state.acknowledged.remove(0);
        }
        state.acknowledged.push(AcknowledgedReply {
            key: receipt.key,
            owner: receipt.owner,
            receiver: receipt.receiver,
        });
    }
    let _ = finish_shutdown(shared);
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
    dispatch_owned(shared, bytes, header)
}
pub(super) fn dispatch_owned(
    shared: &Shared,
    bytes: &[u8],
    header: InputHeader,
) -> windows::core::Result<()> {
    let owner = current_owner(shared).ok_or_else(|| error(E_ACCESSDENIED.0))?;
    owner_slot(shared, &header, &owner)?;
    // Record before a native call can reenter and publish a result. Even a
    // failed call can have an uncertain effect; keep its original identity.
    remember_header(shared, header, owner)?;
    let (_call, api) = enter(shared)?;
    unsafe {
        checked((api.dispatch)(api.handle, bytes.as_ptr(), bytes.len()))?;
    }
    Ok(())
}
fn owner_slot(
    shared: &Shared,
    header: &InputHeader,
    owner: &ReplyOwner,
) -> windows::core::Result<()> {
    let key = ReceiptKey {
        engine: header.engine.clone(),
        task: header.task.clone(),
        turn: header.turn,
        operation: header.operation,
        revision: header.revision,
        sha256: None,
    };
    let state = shared.borrow();
    if state.reply_owners.len() >= 32 {
        return Err(error(BUSY));
    }
    if state
        .reply_owners
        .iter()
        .any(|(original, old)| original.matches(&key) && old != owner)
        || state
            .retained_replies
            .iter()
            .any(|reply| key.matches(&reply.key) && reply.owner.as_ref() != Some(owner))
        || state
            .acknowledged
            .iter()
            .any(|reply| key.matches(&reply.key) && reply.owner.as_ref() != Some(owner))
    {
        return Err(error(E_ACCESSDENIED.0));
    }
    Ok(())
}
fn remember_header(
    shared: &Shared,
    header: InputHeader,
    owner: ReplyOwner,
) -> windows::core::Result<()> {
    let key = ReceiptKey {
        engine: header.engine,
        task: header.task,
        turn: header.turn,
        operation: header.operation,
        revision: header.revision,
        sha256: None,
    };
    let mut state = shared.borrow_mut();
    if let Some((_, original)) = state
        .reply_owners
        .iter()
        .find(|(original, _)| original.matches(&key))
    {
        return if *original == owner {
            Ok(())
        } else {
            Err(error(E_ACCESSDENIED.0))
        };
    }
    if state.reply_owners.len() >= 32 {
        return Err(error(BUSY));
    }
    state.reply_owners.push((key, owner));
    Ok(())
}
fn outgoing(shared: &Shared) -> windows::core::Result<()> {
    if shared.borrow().fixture_connection || shared.borrow().retained_replies.len() >= 16 {
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
    // Delayed old output stays assigned to its original request, never the latest host.
    let current = current_owner(shared);
    let key = ReceiptKey::from_frame(&bytes).ok_or_else(|| error(E_FAIL.0))?;
    let state = shared.borrow();
    let send_current = state.connected
        && state
            .retained_replies
            .iter()
            .find(|reply| reply.key == key)
            .is_some_and(|reply| reply.owner.is_some() && reply.owner == current);
    drop(state);
    if send_current {
        send(shared, &bytes)?;
    }
    Ok(())
}
pub(super) fn poll(shared: &Shared) {
    // WM_TIMER can reenter during a core COM call. Busy is not a broken pipe;
    // keep the authenticated peer and let the original owner finish its call.
    if shared.borrow().calls.get() != 0 || shared.borrow().switching {
        return;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    fn owner(pid: u32, capability: &str) -> ReplyOwner {
        (
            Identity {
                pid,
                session: 1,
                created: pid as u64 + 100,
            },
            capability.into(),
        )
    }
    fn shared() -> Shared {
        Rc::new(RefCell::new(State {
            instance: [1; 16],
            generation: 1,
            serial: 1,
            calls: Rc::new(Cell::new(0)),
            manager: None,
            client: 0,
            current: None,
            retired: Vec::new(),
            active: true,
            window: HWND::default(),
            switching: false,
            epoch: 1,
            shutdown: false,
            failed: false,
            retained_replies: Vec::new(),
            reply_owners: Vec::new(),
            prepare_busy: None,
            handoff_fence: false,
            recovery_receiver: None,
            acknowledged: Vec::new(),
            pipe: None,
            peer: None,
            capability: None,
            fixture_connection: false,
            connected: false,
            proxy_identity: None,
            #[cfg(feature = "acceptance")]
            activation_fault: 0,
        }))
    }
    fn header(operation: u64) -> InputHeader {
        InputHeader {
            protocol: 10,
            engine: EngineBinding {
                proxy_instance: "11111111-1111-4111-8111-111111111111".into(),
                engine_generation: 1,
            },
            capability: uuid::Uuid::new_v4().to_string(),
            task: "22222222-2222-4222-8222-222222222222".into(),
            turn: 1,
            operation,
            revision: 1,
            deadline_tick: 1000,
        }
    }
    fn frame(header: &InputHeader) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"engine":header.engine,"task":header.task,"turn":header.turn,"operation":header.operation,"revision":header.revision,"applied":true,"text":"已写入"})).unwrap()
    }
    #[test]
    fn reentrant_timer_does_not_disconnect_or_revoke_current_publication() {
        let s = shared();
        let original = owner(1, "old");
        s.borrow_mut().peer = Some(original.0);
        s.borrow_mut().capability = Some(original.1.clone());
        s.borrow_mut().connected = true;
        s.borrow().calls.set(1);
        poll(&s);
        assert_eq!(current_owner(&s), Some(original));
        assert!(s.borrow().connected);
        assert!(!s.borrow().handoff_fence);
    }
    #[test]
    fn shutdown_keeps_receipt_channel_until_the_receiver_gets_terminal_state() {
        let s = shared();
        s.borrow_mut().shutdown = true;
        s.borrow_mut().recovery_receiver = Some(owner(2, "new"));
        assert_eq!(finish_shutdown(&s).unwrap_err().code().0, BUSY);
        assert!(s.borrow().recovery_receiver.is_some());
        s.borrow_mut().recovery_receiver = None;
        finish_shutdown(&s).unwrap();
        assert!(!s.borrow().active);
    }
    #[test]
    fn late_result_keeps_original_owner_after_a_new_host_publishes() {
        let s = shared();
        let original = owner(1, "old");
        let receiver = owner(2, "new");
        let h = header(1);
        let bytes = frame(&h);
        remember_header(&s, h, original.clone()).unwrap();
        s.borrow_mut().peer = Some(receiver.0);
        s.borrow_mut().capability = Some(receiver.1.clone());
        retain(&s, bytes.clone()).unwrap();
        let state = s.borrow();
        let reply = &state.retained_replies[0];
        assert_eq!(reply.owner.as_ref(), Some(&original));
        assert_eq!(reply.bytes, bytes);
        assert!(!offered_to(reply, &Some(receiver)));
        assert!(reply.receiver.is_none());
    }
    #[test]
    fn only_a_separately_offered_receiver_can_read_without_relabelling() {
        let original = owner(1, "old");
        let new_cap = owner(1, "new");
        let stranger = owner(2, "new");
        let bytes = frame(&header(1));
        let mut reply = RetainedReply {
            key: ReceiptKey::from_frame(&bytes).unwrap(),
            owner: Some(original.clone()),
            receiver: None,
            bytes,
        };
        assert!(!offered_to(&reply, &Some(new_cap.clone())));
        reply.receiver = Some(new_cap.clone());
        assert!(offered_to(&reply, &Some(new_cap)));
        assert!(!offered_to(&reply, &Some(stranger)));
        assert!(!offered_to(&reply, &Some(original.clone())));
        assert_eq!(reply.owner, Some(original));
        reply.owner = None;
        assert!(!offered_to(&reply, &reply.receiver));
    }
    #[test]
    fn duplicate_frames_do_not_consume_capacity_or_change_original_bytes() {
        let s = shared();
        let h = header(1);
        let bytes = frame(&h);
        let original = owner(1, "old");
        remember_header(&s, h, original.clone()).unwrap();
        retain(&s, bytes.clone()).unwrap();
        for _ in 0..20 {
            retain(&s, bytes.clone()).unwrap();
        }
        assert_eq!(s.borrow().retained_replies.len(), 1);
        assert_eq!(
            s.borrow().retained_replies[0].owner.as_ref(),
            Some(&original)
        );
        assert_eq!(s.borrow().retained_replies[0].bytes, bytes);
    }
    #[test]
    fn unknown_owner_is_never_adopted_by_current_host() {
        let s = shared();
        s.borrow_mut().peer = Some(owner(3, "new").0);
        s.borrow_mut().capability = Some("new".into());
        retain(&s, frame(&header(1))).unwrap();
        assert!(s.borrow().retained_replies[0].owner.is_none());
        assert!(s.borrow().retained_replies[0].receiver.is_none());
    }
    #[test]
    fn capacity_and_cross_owner_identity_collision_are_refused_before_dispatch() {
        let s = shared();
        let a = owner(1, "old");
        let b = owner(1, "new");
        remember_header(&s, header(1), a.clone()).unwrap();
        assert!(owner_slot(&s, &header(1), &b).is_err());
        for op in 2..=32 {
            remember_header(&s, header(op), a.clone()).unwrap();
        }
        assert_eq!(owner_slot(&s, &header(33), &a).unwrap_err().code().0, BUSY);
        assert_eq!(s.borrow().reply_owners.len(), 32);
    }
    #[test]
    fn exact_acknowledged_identity_recovers_duplicate_ownership_without_new_host() {
        let s = shared();
        let original = owner(1, "old");
        let bytes = frame(&header(1));
        s.borrow_mut().acknowledged.push(AcknowledgedReply {
            key: ReceiptKey::from_frame(&bytes).unwrap(),
            owner: Some(original.clone()),
            receiver: Some(owner(2, "receiver")),
        });
        retain(&s, bytes).unwrap();
        assert_eq!(
            s.borrow().retained_replies[0].owner.as_ref(),
            Some(&original)
        );
        assert!(s.borrow().retained_replies[0].receiver.is_none());
    }
}
