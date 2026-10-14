//! Hosted core lifecycle. All handles are created, used and destroyed on one STA.
use super::*;
use crate::engine_abi::*;
use std::{
    collections::VecDeque,
    panic::{AssertUnwindSafe, catch_unwind},
};

struct Engine {
    service: ITfTextInputProcessor,
    state: Shared,
    output: Rc<RefCell<VecDeque<Vec<u8>>>>,
    generation: u64,
    thread: u32,
    pins: Cell<u32>,
    identity: (u64, [u8; 32]),
}
fn guard(action: impl FnOnce() -> windows::core::Result<()>) -> i32 {
    catch_unwind(AssertUnwindSafe(action)).map_or(E_UNEXPECTED.0, |r| {
        r.map_or_else(|e| e.code().0, |_| S_OK.0)
    })
}
unsafe fn instance<'a>(handle: *mut c_void) -> windows::core::Result<&'a Engine> {
    let value = unsafe { handle.cast::<Engine>().as_ref() }.ok_or_else(|| failure(E_POINTER))?;
    if value.thread != unsafe { windows::Win32::System::Threading::GetCurrentThreadId() } {
        return Err(failure(RPC_E_WRONG_THREAD));
    }
    Ok(value)
}
fn status_of(engine: &Engine) -> EngineStatusV1 {
    let s = engine.state.borrow();
    EngineStatusV1 {
        abi: ENGINE_ABI,
        input_protocol: PROTOCOL,
        generation: engine.generation,
        build_id: match env!("SAYWEAVE_ENGINE_BUILD_ID").parse().unwrap_or(0) {
            0 => include_str!("../core-build-id.txt")
                .trim()
                .parse()
                .expect("valid source-owned core build identity"),
            explicit => explicit,
        },
        module_base: engine.identity.0,
        module_sha256: engine.identity.1,
        active: u32::from(s.active),
        ranges: u32::from(
            s.version.owned
                || s.composition.is_some()
                || s.prepared.is_some()
                || s.empty_range.is_some()
                || s.block.is_some(),
        ),
        pending: u32::from(
            s.pending.is_some() || !engine.output.borrow().is_empty() || s.reply_recovery.is_some(),
        ),
        cleanup: u32::from(
            s.cleanup
                || !s.retired_bridges.is_empty()
                || !s.text_notifications.retired.borrow().is_empty(),
        ),
        observers: (s.observations.len() + s.observation_pending.len()) as u32
            + u32::from(s.observation_task.is_some()),
        callbacks: s.text_notifications.depth.get() + CALLBACKS.load(Ordering::Acquire) as u32,
        leases: engine.pins.get(),
    }
}
unsafe extern "system" fn activate(handle: *mut c_void, manager: *mut c_void, client: u32) -> i32 {
    guard(|| {
        let engine = unsafe { instance(handle)? };
        let manager = unsafe { ITfThreadMgr::from_raw_borrowed(&manager) }
            .ok_or_else(|| failure(E_POINTER))?;
        unsafe { engine.service.Activate(manager, client) }
    })
}
unsafe extern "system" fn dispatch_frame(
    handle: *mut c_void,
    data: *const u8,
    length: usize,
) -> i32 {
    guard(|| {
        let engine = unsafe { instance(handle)? };
        if data.is_null() || length == 0 || length > MAX_FRAME {
            return Err(failure(E_INVALIDARG));
        }
        let request: Request =
            serde_json::from_slice(unsafe { std::slice::from_raw_parts(data, length) })
                .map_err(|_| failure(E_INVALIDARG))?;
        if request.action != Action::Revoke
            && (!engine.output.borrow().is_empty()
                || engine.state.borrow().reply_recovery.is_some())
        {
            return Err(failure(HRESULT(BUSY)));
        }
        if !request.valid() || !request.live(unsafe { GetTickCount64() }) {
            return Err(failure(E_INVALIDARG));
        }
        let mut state = engine.state.borrow_mut();
        if request.engine != state.binding {
            return Err(failure(E_ACCESSDENIED));
        }
        let (host, capability) =
            registration::read_host().ok_or_else(|| failure(E_ACCESSDENIED))?;
        if capability != request.capability {
            return Err(failure(E_ACCESSDENIED));
        }
        let peer = identity(host)?;
        if peer.session != crate::transport::session(std::process::id())? {
            return Err(failure(E_ACCESSDENIED));
        }
        if state.peer.is_some_and(|previous| previous != peer) {
            return Err(failure(E_ACCESSDENIED));
        }
        if !state.active {
            return Err(failure(TF_E_DISCONNECTED));
        }
        if state
            .capability
            .as_ref()
            .is_some_and(|v| v != &request.capability)
        {
            return Err(failure(E_ACCESSDENIED));
        }
        state.capability = Some(request.capability.clone());
        state.peer = Some(peer);
        state.connected = true;
        state.connection = engine.generation;
        drop(state);
        dispatch(&engine.state, request, engine.generation);
        Ok(())
    })
}
unsafe extern "system" fn poll_engine(handle: *mut c_void) -> i32 {
    guard(|| {
        let engine = unsafe { instance(handle)? };
        drain_text_watches(&engine.state);
        if engine.state.borrow().active {
            activate_observations(&engine.state);
        }
        Ok(())
    })
}
unsafe extern "system" fn bind_host(handle: *mut c_void, host: *const HostGuardV1) -> i32 {
    guard(|| {
        let engine = unsafe { instance(handle)? };
        let host = unsafe { host.as_ref() }.ok_or_else(|| failure(E_POINTER))?;
        if host.context.is_null()
            || host.engine_generation != engine.generation
            || engine.state.borrow().active
        {
            return Err(failure(E_INVALIDARG));
        }
        let mut state = engine.state.borrow_mut();
        state.host_guard = Some(*host);
        state.binding = EngineBinding {
            proxy_instance: uuid::Uuid::from_bytes(host.proxy_instance).to_string(),
            engine_generation: host.engine_generation,
        };
        Ok(())
    })
}
unsafe extern "system" fn disconnect_engine(handle: *mut c_void) -> i32 {
    guard(|| {
        let engine = unsafe { instance(handle)? };
        disconnect(&engine.state);
        Ok(())
    })
}
unsafe extern "system" fn read(
    handle: *mut c_void,
    buffer: *mut u8,
    capacity: usize,
    size: *mut usize,
) -> i32 {
    guard(|| {
        let engine = unsafe { instance(handle)? };
        if size.is_null() {
            return Err(failure(E_POINTER));
        }
        let mut output = engine.output.borrow_mut();
        let recovery = engine.state.borrow().reply_recovery.clone();
        let Some(frame) = output.front().or(recovery.as_ref()) else {
            unsafe {
                *size = 0;
            }
            return Ok(());
        };
        unsafe {
            *size = frame.len();
        }
        if buffer.is_null() || capacity < frame.len() {
            return Err(failure(HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0)));
        }
        unsafe {
            std::ptr::copy_nonoverlapping(frame.as_ptr(), buffer, frame.len());
        }
        if output.pop_front().is_none() {
            engine.state.borrow_mut().reply_recovery = None;
        }
        Ok(())
    })
}
unsafe extern "system" fn query(handle: *mut c_void, result: *mut EngineStatusV1) -> i32 {
    guard(|| {
        let engine = unsafe { instance(handle)? };
        if result.is_null() {
            return Err(failure(E_POINTER));
        }
        unsafe {
            *result = status_of(engine);
        }
        Ok(())
    })
}
unsafe extern "system" fn deactivate(handle: *mut c_void) -> i32 {
    guard(|| {
        let engine = unsafe { instance(handle)? };
        drain_text_watches(&engine.state);
        if !status_of(engine).idle() {
            return Err(failure(HRESULT(BUSY)));
        }
        let (manager, client, keys) = {
            let s = engine.state.borrow();
            (s.manager.clone(), s.client, s.key_sink)
        };
        if keys {
            unsafe {
                manager
                    .as_ref()
                    .ok_or_else(|| failure(E_UNEXPECTED))?
                    .cast::<ITfKeystrokeMgr>()?
                    .UnadviseKeyEventSink(client)?;
            }
        }
        let mut state = engine.state.borrow_mut();
        state.active = false;
        state.connected = false;
        state.key_sink = false;
        state.peer = None;
        state.capability = None;
        state.manager = None;
        Ok(())
    })
}
unsafe extern "system" fn destroy(handle: *mut c_void) -> i32 {
    guard(|| {
        let engine = unsafe { instance(handle)? };
        let state = status_of(engine);
        if state.active != 0 || !state.idle() || state.leases != 0 {
            return Err(failure(HRESULT(BUSY)));
        }
        unsafe {
            drop(Box::from_raw(handle.cast::<Engine>()));
        }
        Ok(())
    })
}
unsafe extern "system" fn retire(handle: *mut c_void) -> i32 {
    guard(|| {
        let engine = unsafe { instance(handle)? };
        if CALLBACKS.load(Ordering::Acquire) != 0 {
            return Err(failure(HRESULT(BUSY)));
        }
        end_owned(&engine.state);
        stop_observations(&engine.state);
        let (manager, client, keys) = {
            let s = engine.state.borrow();
            (s.manager.clone(), s.client, s.key_sink)
        };
        if keys {
            unsafe {
                manager
                    .as_ref()
                    .ok_or_else(|| failure(E_UNEXPECTED))?
                    .cast::<ITfKeystrokeMgr>()?
                    .UnadviseKeyEventSink(client)?;
            }
        }
        let mut state = engine.state.borrow_mut();
        state.active = false;
        state.connected = false;
        state.key_sink = false;
        state.generation = next_generation();
        state.pending = None;
        drop(state);
        drain_text_watches(&engine.state);
        Ok(())
    })
}
unsafe extern "system" fn retain(handle: *mut c_void) -> i32 {
    guard(|| {
        let e = unsafe { instance(handle)? };
        e.pins.set(
            e.pins
                .get()
                .checked_add(1)
                .ok_or_else(|| failure(E_UNEXPECTED))?,
        );
        Ok(())
    })
}
unsafe extern "system" fn release(handle: *mut c_void) -> i32 {
    guard(|| {
        let e = unsafe { instance(handle)? };
        e.pins.set(
            e.pins
                .get()
                .checked_sub(1)
                .ok_or_else(|| failure(E_UNEXPECTED))?,
        );
        Ok(())
    })
}
fn own_identity() -> windows::core::Result<(u64, [u8; 32])> {
    let h = module()?;
    let mut path = vec![0u16; 32768];
    let length = unsafe { GetModuleFileNameW(Some(HMODULE(h.0)), &mut path) } as usize;
    if length == 0 || length >= path.len() {
        return Err(failure(E_FAIL));
    }
    let file = String::from_utf16(&path[..length]).map_err(|_| failure(E_FAIL))?;
    let bytes = std::fs::read(file).map_err(|_| failure(E_FAIL))?;
    use sha2::{Digest, Sha256};
    Ok((h.0 as u64, Sha256::digest(bytes).into()))
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn CreateEngineV1(
    generation: u64,
    result: *mut EngineApiV1,
    capacity: usize,
) -> i32 {
    guard(|| {
        if generation == 0 || result.is_null() || capacity < std::mem::size_of::<EngineApiV1>() {
            return Err(failure(E_INVALIDARG));
        }
        let output = Rc::new(RefCell::new(VecDeque::new()));
        let identity = own_identity()?;
        let state = Rc::new(RefCell::new(State {
            hosted: true,
            output: Some(output.clone()),
            core_identity: Some(ModuleIdentity {
                base: identity.0,
                sha256: identity.1.iter().map(|b| format!("{b:02x}")).collect(),
            }),
            ..Default::default()
        }));
        let service = Service {
            state: state.clone(),
            _lease: Lease::new(),
        }
        .into();
        let engine = Box::new(Engine {
            service,
            state,
            output,
            generation,
            thread: unsafe { windows::Win32::System::Threading::GetCurrentThreadId() },
            pins: Cell::new(0),
            identity,
        });
        unsafe {
            *result = EngineApiV1 {
                abi: ENGINE_ABI,
                size: std::mem::size_of::<EngineApiV1>() as u32,
                handle: Box::into_raw(engine).cast(),
                bind_host,
                disconnect: disconnect_engine,
                activate,
                dispatch: dispatch_frame,
                poll: poll_engine,
                read,
                status: query,
                deactivate,
                retire,
                destroy,
                retain,
                release,
            };
        }
        Ok(())
    })
}
