//! All TSF state stays on the target apartment. IPC never carries COM pointers.
use crate::{
    composition_text::{CompositionText, TextPhase, equivalent_native_text},
    registration,
    transport::{Identity, Pipe, identity},
    wire::*,
};
use std::{
    cell::RefCell,
    ffi::c_void,
    mem::ManuallyDrop,
    rc::Rc,
    sync::atomic::{AtomicU64, AtomicUsize, Ordering},
};
use windows::{
    Win32::{
        Foundation::*,
        System::{Com::*, LibraryLoader::*, SystemInformation::GetTickCount64},
        UI::{TextServices::*, WindowsAndMessaging::*},
    },
    core::{BOOL, GUID, HRESULT, IUnknown, Interface, Ref, implement, w},
};
static OBJECTS: AtomicUsize = AtomicUsize::new(0);
static LOCKS: AtomicUsize = AtomicUsize::new(0);
static COMPOSITION_EPOCH: AtomicU64 = AtomicU64::new(0);
static LEASE_GENERATION: AtomicU64 = AtomicU64::new(0);
fn next_generation() -> u64 {
    LEASE_GENERATION.fetch_add(1, Ordering::AcqRel) + 1
}
struct Lease;
impl Lease {
    fn new() -> Self {
        OBJECTS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        OBJECTS.fetch_sub(1, Ordering::AcqRel);
    }
}
fn failure(code: HRESULT) -> windows::core::Error {
    windows::core::Error::from_hresult(code)
}
fn module() -> windows::core::Result<HINSTANCE> {
    let mut h = HMODULE::default();
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            windows::core::PCWSTR(procedure as *const () as *const u16),
            &mut h,
        )?;
    }
    Ok(HINSTANCE(h.0))
}
#[derive(Clone)]
struct Pending {
    operation: u64,
    revision: u64,
    task: String,
    turn: u64,
    connection: u64,
    capability: String,
    settlement: Option<Settlement>,
}
#[derive(Default)]
struct State {
    manager: Option<ITfThreadMgr>,
    client: u32,
    window: usize,
    pipe: Option<Pipe>,
    peer: Option<Identity>,
    connected: bool,
    connection: u64,
    capability: Option<String>,
    active: bool,
    generation: u64,
    pending: Option<Pending>,
    epoch: u64,
    own_change: Option<OwnChange>,
    cleanup: bool,
    cleanup_id: u64,
    materialize_allowed: bool,
    version: DraftVersion,
    context: Option<ITfContext>,
    composition: Option<ITfComposition>,
    // A fully settled prefix may leave an empty insertion lease. There is no
    // dummy character and no live TSF composition until the next actual update.
    empty_range: Option<ITfRange>,
    native_position: Option<NativeEditPosition>,
    backup: Option<Rollback>,
    expected: String,
    text: Option<CompositionText>,
    selection: Option<ITfRange>,
    preserved_attributes: Vec<(GUID, i32)>,
    text_cookie: Option<u32>,
    key_sink: bool,
    key_handoff: bool,
    key_failure: bool,
}
// A single post-edit notification for an exact write we just performed. This is
// range evidence, not permission to ignore arbitrary provider notifications.
#[derive(Clone)]
struct OwnChange {
    epoch: u64,
    generation: u64,
    revision: u64,
    range: ITfRange,
    expected: String,
}
#[derive(Clone)]
enum Rollback {
    OriginalSelection(ITfRangeBackup),
    GeneratedTail,
}
#[derive(Clone, Copy, PartialEq, Eq)]
struct NativeEditPosition {
    window: usize,
    offset: u32,
    end: u32,
}
fn native_edit_position(context: &ITfContext) -> windows::core::Result<Option<NativeEditPosition>> {
    unsafe {
        let window = context.GetActiveView()?.GetWnd()?;
        let mut class = [0u16; 64];
        let size = GetClassNameW(window, &mut class);
        if size == 0 || String::from_utf16_lossy(&class[..size as usize]) != "Edit" {
            return Ok(None);
        }
        let (start, end) = edit_selection(window);
        Ok(Some(NativeEditPosition {
            window: window.0 as usize,
            offset: start,
            end,
        }))
    }
}
fn edit_selection(window: HWND) -> (u32, u32) {
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
type Shared = Rc<RefCell<State>>;
#[implement(ITfTextInputProcessor)]
struct Service {
    state: Shared,
    _lease: Lease,
}
impl ITfTextInputProcessor_Impl for Service_Impl {
    fn Activate(&self, manager: Ref<ITfThreadMgr>, client: u32) -> windows::core::Result<()> {
        unsafe {
            if self.state.borrow().active
                || self.state.borrow().composition.is_some()
                || self.state.borrow().cleanup
            {
                return Err(failure(E_UNEXPECTED));
            }
            let manager = manager.as_ref().ok_or_else(|| failure(E_POINTER))?.clone();
            let pipe = Pipe::server()?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: module()?,
                lpszClassName: w!("SayweaveTsfBridge"),
                ..Default::default()
            };
            let _ = RegisterClassW(&class);
            let window = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("SayweaveTsfBridge"),
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(module()?),
                None,
            )?;
            let data = Box::new(self.state.clone());
            SetWindowLongPtrW(window, GWLP_USERDATA, Box::into_raw(data) as isize);
            {
                let mut s = self.state.borrow_mut();
                s.manager = Some(manager);
                s.client = client;
                s.window = window.0 as usize;
                s.active = true;
                s.key_failure = false;
                s.key_handoff = false;
                s.generation = next_generation();
                s.pipe = Some(pipe);
            }
            if SetTimer(Some(window), 1, 50, None) == 0 {
                self.Deactivate()?;
                return Err(failure(E_FAIL));
            }
            let (manager, generation) = {
                let s = self.state.borrow();
                (
                    s.manager.clone().ok_or_else(|| failure(E_UNEXPECTED))?,
                    s.generation,
                )
            };
            let keys = manager.cast::<ITfKeystrokeMgr>();
            let sink: ITfKeyEventSink = KeySink {
                state: Rc::downgrade(&self.state),
                _lease: Lease::new(),
            }
            .into();
            if let Err(error) = keys.and_then(|keys| keys.AdviseKeyEventSink(client, &sink, false))
            {
                self.Deactivate()?;
                return Err(error);
            }
            if !self.state.borrow().active || self.state.borrow().generation != generation {
                if let Ok(keys) = manager.cast::<ITfKeystrokeMgr>() {
                    let _ = keys.UnadviseKeyEventSink(client);
                }
                return Err(failure(E_UNEXPECTED));
            }
            self.state.borrow_mut().key_sink = true;
            Ok(())
        }
    }
    fn Deactivate(&self) -> windows::core::Result<()> {
        unsafe {
            end_owned(&self.state);
            let (manager, client, key_sink, window) = {
                let mut s = self.state.borrow_mut();
                s.active = false;
                s.connected = false;
                s.pending = None;
                s.pipe.take();
                s.peer = None;
                s.capability = None;
                let values = (s.manager.clone(), s.client, s.key_sink, s.window);
                s.key_sink = false;
                s.window = 0;
                values
            };
            if key_sink
                && let Some(manager) = &manager
                && let Ok(keys) = manager.cast::<ITfKeystrokeMgr>()
            {
                let _ = keys.UnadviseKeyEventSink(client);
            }
            let _ = DestroyWindow(HWND(window as *mut _));
            let _ = UnregisterClassW(w!("SayweaveTsfBridge"), Some(module()?));
            Ok(())
        }
    }
}
unsafe extern "system" fn procedure(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe {
        let data = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Shared;
        if !data.is_null() {
            if message == WM_TIMER {
                poll(&*data);
                return LRESULT(0);
            }
            if message == WM_NCDESTROY {
                let _ = KillTimer(Some(window), 1);
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                drop(Box::from_raw(data));
            }
        }
        DefWindowProcW(window, message, w, l)
    }
}
fn disconnect(shared: &Shared) {
    {
        let mut s = shared.borrow_mut();
        s.generation = next_generation();
        s.pending = None;
        s.peer = None;
        s.connected = false;
        s.capability = None;
        if let Some(p) = s.pipe.as_ref() {
            p.disconnect();
        }
    }
    end_owned(shared);
}
fn read_request(shared: &Shared) -> Result<Option<(Request, u64)>, ()> {
    let mut s = shared.borrow_mut();
    if !s.active {
        return Ok(None);
    }
    let Some(pipe) = s.pipe.as_ref() else {
        return Ok(None);
    };
    if !s.connected {
        if !pipe.connect() {
            return Ok(None);
        }
        s.connected = true;
        s.connection = next_generation();
    }
    let Some((host, capability)) = registration::read_host() else {
        return Err(());
    };
    if s.capability.as_ref().is_some_and(|old| old != &capability) {
        return Err(());
    }
    s.capability = Some(capability.clone());
    let pipe = s.pipe.as_ref().unwrap();
    if pipe.client_pid().ok() != Some(host) {
        return Err(());
    }
    let current = identity(host).map_err(|_| ())?;
    if current.session != crate::transport::session(std::process::id()).map_err(|_| ())? {
        return Err(());
    }
    if s.peer.is_some_and(|p| p != current) {
        return Err(());
    }
    s.peer = Some(current);
    match s.pipe.as_ref().unwrap().receive() {
        Ok(Some(bytes)) => {
            let r = serde_json::from_slice::<Request>(&bytes).map_err(|_| ())?;
            if r.capability != capability || !r.valid() {
                return Err(());
            }
            Ok(Some((r, s.connection)))
        }
        Ok(None) => Ok(None),
        Err(_) => Err(()),
    }
}
fn poll(shared: &Shared) {
    let (r, connection) = match read_request(shared) {
        Ok(Some(r)) => r,
        Ok(None) => return,
        Err(()) => {
            disconnect(shared);
            return;
        }
    };
    dispatch(shared, r, connection);
}
fn dispatch(shared: &Shared, r: Request, connection: u64) {
    if r.action == Action::Inspect {
        // Authenticated connection only; no context, selection or edit session is touched.
        let idle = {
            let s = shared.borrow();
            s.pending.is_none()
                && !s.cleanup
                && !s.version.owned
                && r.text.is_empty()
                && r.live(unsafe { GetTickCount64() })
        };
        reply(
            shared,
            &r,
            connection,
            idle,
            "runtime",
            Mutation::NotAttempted,
        );
        return;
    }
    let pending = shared.borrow().pending.clone();
    let inherited_settlement = pending.as_ref().and_then(|p| p.settlement);
    if let Some(p) = pending {
        if r.action != Action::Abort
            || r.task != p.task
            || r.turn != p.turn
            || r.operation <= p.operation
            || connection != p.connection
            || r.capability != p.capability
        {
            reply(
                shared,
                &r,
                connection,
                false,
                "busy",
                Mutation::NotAttempted,
            );
            return;
        }
        {
            let mut s = shared.borrow_mut();
            s.generation = next_generation();
            s.pending = None;
        }
        if shared.borrow().composition.is_none() {
            // Finalize cancellation before retiring ranges/sinks, which can
            // reenter and establish a new task. No old version write follows COM.
            apply_cancel_then_retire(shared, &r, |epoch| {
                if let Some(epoch) = epoch {
                    release_composition(shared, epoch, None);
                }
            });
            reply_outcome(
                shared,
                &r,
                connection,
                true,
                if inherited_settlement.is_some() {
                    "cancelledPrefixContinuation"
                } else {
                    "cancelledBeforeBegin"
                },
                Mutation::NotAttempted,
                inherited_settlement,
            );
            return;
        }
    }
    if !r.safe_prefix_boundary() {
        reply(
            shared,
            &r,
            connection,
            false,
            "prefixBoundaryUnsupported",
            Mutation::NotAttempted,
        );
        return;
    }
    let already_handed_off = {
        let s = shared.borrow();
        s.pending.is_none()
            && !s.cleanup
            && s.composition.is_none()
            && s.context.is_none()
            && s.version.acknowledges_handoff(&r)
            && r.live(unsafe { GetTickCount64() })
    };
    if shared.borrow().key_failure {
        reply_outcome(
            shared,
            &r,
            connection,
            false,
            "keyboardHandoffFailed",
            Mutation::NotAttempted,
            inherited_settlement,
        );
        return;
    }
    if already_handed_off {
        let code = if shared.borrow().key_handoff {
            "keyboardHandedOff"
        } else {
            "alreadyHandedOff"
        };
        shared.borrow_mut().version.applied(&r);
        reply(shared, &r, connection, true, code, Mutation::NotAttempted);
        return;
    }
    if let Err((stage, e)) = schedule(shared, &r, connection, inherited_settlement) {
        reply_outcome(
            shared,
            &r,
            connection,
            false,
            &format!("{stage}:{:08x}", e.code().0 as u32),
            Mutation::NotAttempted,
            inherited_settlement,
        );
    }
}
fn apply_cancel_then_retire(shared: &Shared, request: &Request, retire: impl FnOnce(Option<u64>)) {
    let epoch = {
        let mut state = shared.borrow_mut();
        state.version.applied(request);
        state.empty_range.as_ref().map(|_| state.epoch)
    };
    retire(epoch);
}
fn reply(
    shared: &Shared,
    r: &Request,
    connection: u64,
    applied: bool,
    code: &str,
    mutation: Mutation,
) {
    send_reply(shared, r, connection, applied, code, mutation, None);
}
fn reply_outcome(
    shared: &Shared,
    r: &Request,
    connection: u64,
    applied: bool,
    code: &str,
    mutation: Mutation,
    settlement: Option<Settlement>,
) {
    let mut s = shared.borrow_mut();
    if !s.connected || s.connection != connection || s.capability.as_ref() != Some(&r.capability) {
        return;
    }
    if s.pending.as_ref().is_some_and(|p| {
        p.operation == r.operation
            && p.task == r.task
            && p.turn == r.turn
            && p.connection == connection
            && p.capability == r.capability
    }) {
        s.pending = None;
    }
    drop(s);
    send_reply(shared, r, connection, applied, code, mutation, settlement);
}
fn send_reply(
    shared: &Shared,
    r: &Request,
    connection: u64,
    applied: bool,
    code: &str,
    mutation: Mutation,
    settlement: Option<Settlement>,
) {
    let s = shared.borrow();
    if !s.connected || s.connection != connection || s.capability.as_ref() != Some(&r.capability) {
        return;
    }
    if let Some(pipe) = s.pipe.as_ref() {
        let reply = Reply {
            operation: r.operation,
            revision: r.revision,
            applied,
            code: code.into(),
            mutation,
            range_epoch: if s.composition.is_some() || s.empty_range.is_some() {
                s.epoch
            } else {
                0
            },
            settlement,
            runtime: (r.action == Action::Inspect).then(|| Runtime {
                protocol: PROTOCOL,
                process: std::process::id(),
                thread: unsafe { windows::Win32::System::Threading::GetCurrentThreadId() },
            }),
        };
        if let Ok(data) = serde_json::to_vec(&reply) {
            let _ = pipe.send(&data);
        }
    }
}
fn lease(
    shared: &Shared,
    r: &Request,
    generation: u64,
    context: &ITfContext,
) -> windows::core::Result<()> {
    unsafe {
        if !r.live(GetTickCount64()) {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let (manager, peer) = {
            let s = shared.borrow();
            if !s.active || s.generation != generation || !s.version.accepts(r) || s.cleanup {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let pipe = s.pipe.as_ref().ok_or_else(|| failure(TF_E_DISCONNECTED))?;
            let peer = s.peer.ok_or_else(|| failure(E_ACCESSDENIED))?;
            if !pipe.alive() || pipe.client_pid().ok() != Some(peer.pid) {
                return Err(failure(TF_E_DISCONNECTED));
            }
            (
                s.manager
                    .as_ref()
                    .ok_or_else(|| failure(TF_E_DISCONNECTED))?
                    .clone(),
                peer,
            )
        };
        if registration::read_host() != Some((peer.pid, r.capability.clone()))
            || identity(peer.pid).ok() != Some(peer)
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        if manager.GetFocus()?.GetTop()? != *context {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let mut pid = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
        if pid != std::process::id() {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let view = context.GetActiveView()?.GetWnd()?;
        if !view.0.is_null() && GetAncestor(view, GA_ROOT) != GetForegroundWindow() {
            return Err(failure(TF_E_DISCONNECTED));
        }
        if context.GetStatus()?.dwDynamicFlags & TF_SD_READONLY != 0 {
            return Err(failure(TF_E_READONLY));
        }
        ensure_enabled_context(context)?;
        // COM providers may reenter the apartment. Recheck after the last COM call.
        let s = shared.borrow();
        if !s.active
            || s.generation != generation
            || !s.version.accepts(r)
            || s.cleanup
            || !r.live(GetTickCount64())
            || s.capability.as_ref() != Some(&r.capability)
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        Ok(())
    }
}
// No COM calls: use immediately before/after a mutation when a provider may
// have reentered while resolving the property or range.
fn edit_identity(
    shared: &Shared,
    r: &Request,
    generation: u64,
    epoch: u64,
) -> windows::core::Result<()> {
    let s = shared.borrow();
    if !s.active
        || s.generation != generation
        || s.epoch != epoch
        || !s.version.owned
        || !s.version.accepts(r)
        || s.cleanup
        || s.capability.as_ref() != Some(&r.capability)
        || !r.live(unsafe { GetTickCount64() })
    {
        return Err(failure(TF_E_DISCONNECTED));
    }
    Ok(())
}
fn schedule(
    shared: &Shared,
    r: &Request,
    connection: u64,
    inherited_settlement: Option<Settlement>,
) -> Result<(), (&'static str, windows::core::Error)> {
    unsafe {
        let (manager, client, generation) = {
            let s = shared.borrow();
            if !s.version.accepts(r) {
                return Err(("draftOwnership", failure(E_INVALIDARG)));
            }
            if s.cleanup || r.action == Action::Begin && s.composition.is_some() {
                return Err(("draftCleanup", failure(E_INVALIDARG)));
            }
            (
                s.manager
                    .as_ref()
                    .ok_or_else(|| ("editManager", failure(E_FAIL)))?
                    .clone(),
                s.client,
                s.generation,
            )
        };
        let context = manager
            .GetFocus()
            .and_then(|d| d.GetTop())
            .map_err(|e| ("editFocus", e))?;
        lease(shared, r, generation, &context).map_err(|e| ("editLease", e))?;
        if r.action != Action::Begin && shared.borrow().context.as_ref() != Some(&context) {
            return Err(("editContext", failure(TF_E_DISCONNECTED)));
        }
        let edit: ITfEditSession = Edit {
            state: shared.clone(),
            context: context.clone(),
            request: r.clone(),
            generation,
            connection,
            _lease: Lease::new(),
        }
        .into();
        shared.borrow_mut().pending = Some(Pending {
            operation: r.operation,
            revision: r.revision,
            task: r.task.clone(),
            turn: r.turn,
            connection,
            capability: r.capability.clone(),
            settlement: inherited_settlement,
        });
        let session = context
            .RequestEditSession(client, &edit, TF_ES_ASYNCDONTCARE | TF_ES_READWRITE)
            .map_err(|e| ("requestEdit", e))?;
        // Synchronous DoEditSession owns its acknowledgement, including failure.
        // Its HRESULT must not produce a second reply for the same operation.
        if shared
            .borrow()
            .pending
            .as_ref()
            .is_some_and(|p| p.operation == r.operation)
        {
            session.ok().map_err(|e| ("requestEditResult", e))?;
        }
        Ok(())
    }
}
fn unwatch(context: Option<&ITfContext>, cookie: Option<u32>) {
    if let (Some(context), Some(cookie)) = (context, cookie) {
        unsafe {
            if let Ok(source) = context.cast::<ITfSource>() {
                let _ = source.UnadviseSink(cookie);
            }
        }
    }
}
fn release_composition(shared: &Shared, epoch: u64, composition: Option<&ITfComposition>) {
    release_composition_with_failure(shared, epoch, composition, false);
}
fn release_composition_with_failure(
    shared: &Shared,
    epoch: u64,
    composition: Option<&ITfComposition>,
    semantic_failed: bool,
) {
    let old = {
        let mut s = shared.borrow_mut();
        if s.epoch != epoch || composition.is_some_and(|c| s.composition.as_ref() != Some(c)) {
            return;
        }
        s.generation = next_generation();
        s.version.owned = false;
        s.cleanup = false;
        s.cleanup_id = 0;
        s.materialize_allowed = false;
        s.own_change = None;
        s.empty_range = None;
        s.native_position = None;
        s.key_failure = semantic_failed;
        (
            s.context.take(),
            s.text_cookie.take(),
            s.composition.take(),
            s.backup.take(),
            s.selection.take(),
            s.text.take(),
            s.pending
                .as_ref()
                .filter(|p| {
                    p.settlement
                        .is_some_and(|progress| progress.prefix_operation == p.operation)
                })
                .cloned(),
        )
    };
    // Settle must terminate when editing retires its gap, even if its queued
    // tail has not started. Send before COM cleanup can admit another task.
    if let Some(pending) = &old.6 {
        reply_interrupted_prefix(shared, pending);
    }
    unwatch(old.0.as_ref(), old.1);
    drop(old);
}
fn reply_interrupted_prefix(shared: &Shared, pending: &Pending) {
    let request = Request {
        protocol: PROTOCOL,
        capability: pending.capability.clone(),
        task: pending.task.clone(),
        turn: pending.turn,
        operation: pending.operation,
        revision: pending.revision,
        deadline_tick: unsafe { GetTickCount64() },
        action: Action::Handoff,
        text: String::new(),
        prefix: None,
    };
    let progress = pending.settlement;
    reply_outcome(
        shared,
        &request,
        pending.connection,
        false,
        "compositionInterrupted",
        if progress.is_some_and(|s| s.prefix == Mutation::Uncertain) {
            Mutation::Uncertain
        } else {
            Mutation::Changed
        },
        progress,
    );
}
/// Retire only the captured range. Semantic conversion has a separate guarded authority;
/// it never searches a new target or restores a backup over user changes.
fn end_owned(shared: &Shared) {
    end_owned_with_flags(shared, TF_ES_ASYNCDONTCARE);
}
fn end_owned_with_flags(shared: &Shared, flags: TF_CONTEXT_EDIT_CONTEXT_FLAGS) -> bool {
    let empty_epoch = {
        let state = shared.borrow();
        state.empty_range.as_ref().map(|_| state.epoch)
    };
    if let Some(epoch) = empty_epoch {
        release_composition(shared, epoch, None);
        return true;
    }
    let (data, backup) = {
        let mut s = shared.borrow_mut();
        if s.cleanup {
            return true;
        }
        s.version.owned = false;
        s.generation = next_generation();
        let backup = s.backup.take();
        let data = if let (Some(context), Some(composition)) = (&s.context, &s.composition) {
            Some((context.clone(), composition.clone(), s.client, s.epoch))
        } else {
            None
        };
        if data.is_some() {
            s.cleanup = true;
            s.cleanup_id = next_generation();
        }
        let data = data.map(|(context, composition, client, epoch)| {
            (context, composition, client, epoch, s.cleanup_id)
        });
        (data, backup)
    };
    drop(backup);
    if let Some((context, composition, client, epoch, cleanup_id)) = data {
        unsafe {
            let edit: ITfEditSession = EndOwned {
                state: shared.clone(),
                context: Some(context.clone()),
                composition,
                epoch,
                cleanup_id,
                _lease: Lease::new(),
            }
            .into();
            if context
                .RequestEditSession(client, &edit, flags | TF_ES_READWRITE)
                .and_then(|h| h.ok())
                .is_err()
            {
                mark_handoff_failed(shared, epoch);
                return false;
            }
        }
        return true;
    }
    false
}
fn mark_handoff_failed(shared: &Shared, epoch: u64) {
    let mut s = shared.borrow_mut();
    if s.epoch == epoch && s.composition.is_some() {
        // Neither lock refusal nor EndComposition failure proves termination.
        // Preserve the real range, block writes/resume and await actual teardown.
        s.key_failure = true;
        s.cleanup = true;
        s.version.owned = false;
    }
}
// Cleanup authority is limited to the captured composition, not the live host's next task.
fn cleanup_identity(
    shared: &Shared,
    context: &Option<ITfContext>,
    composition: &ITfComposition,
    epoch: u64,
    cleanup_id: u64,
) -> windows::core::Result<()> {
    let s = shared.borrow();
    if s.epoch != epoch
        || s.cleanup_id != cleanup_id
        || !s.cleanup
        || s.context != *context
        || s.composition.as_ref() != Some(composition)
    {
        return Err(failure(TF_E_DISCONNECTED));
    }
    Ok(())
}
fn revoke_cleanup_conversion(shared: &Shared, epoch: u64) {
    let mut s = shared.borrow_mut();
    if s.epoch == epoch && s.cleanup {
        s.materialize_allowed = false;
    }
}
// Only the already-visible representation may be finalized. Never discover another range.
#[allow(clippy::too_many_arguments)]
fn materialize(
    shared: &Shared,
    context: &ITfContext,
    composition: &ITfComposition,
    epoch: u64,
    ec: u32,
    guard: &impl Fn() -> windows::core::Result<()>,
    preserve_selection: bool,
    mutation: &mut Mutation,
) -> windows::core::Result<()> {
    let (text, expected, manager, attributes) = {
        let s = shared.borrow();
        (
            s.text.clone(),
            s.expected.clone(),
            s.manager.clone(),
            s.preserved_attributes.clone(),
        )
    };
    let Some(text) = text else {
        return Ok(());
    };
    if text.phase == TextPhase::Semantic
        || !text.needs_materialization() && text.phase == TextPhase::Marked
    {
        return Ok(());
    }
    if text.phase == TextPhase::Materializing {
        return Err(failure(TF_E_DISCONNECTED));
    }
    let permitted = || {
        guard()?;
        if !shared.borrow().materialize_allowed {
            return Err(failure(TF_E_DISCONNECTED));
        }
        Ok(())
    };
    let manager = manager.ok_or_else(|| failure(TF_E_DISCONNECTED))?;
    unsafe {
        permitted()?;
        if manager.GetFocus()?.GetTop()? != *context {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let mut process = 0;
        GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut process));
        let view = context.GetActiveView()?.GetWnd()?;
        if process != std::process::id()
            || !view.0.is_null() && GetAncestor(view, GA_ROOT) != GetForegroundWindow()
            || context.GetStatus()?.dwDynamicFlags & TF_SD_READONLY != 0
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        ensure_enabled_context(context)?;
        let range = composition.GetRange()?;
        if password(context, &range, ec)? || range_text(&range, ec)? != expected {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let selected = if preserve_selection {
            Some(selection(context, ec)?.Clone()?)
        } else {
            None
        };
        permitted()?;
        {
            let mut s = shared.borrow_mut();
            if s.epoch != epoch {
                return Err(failure(TF_E_DISCONNECTED));
            }
            s.text
                .as_mut()
                .ok_or_else(|| failure(TF_E_DISCONNECTED))?
                .phase = TextPhase::Materializing;
        }
        *mutation = Mutation::Uncertain;
        let semantic = text.commit_text();
        range.SetText(
            ec,
            TF_ST_CORRECTION,
            &semantic.encode_utf16().collect::<Vec<_>>(),
        )?;
        *mutation = Mutation::Changed;
        permitted()?;
        let actual = range_text(&range, ec)?;
        permitted()?;
        if !equivalent_native_text(&actual, &semantic) {
            return Err(failure(TF_E_DISCONNECTED));
        }
        // Record the successful conversion before another COM call. End failures cannot replay it.
        {
            let mut s = shared.borrow_mut();
            s.expected = actual;
            s.text
                .as_mut()
                .ok_or_else(|| failure(TF_E_DISCONNECTED))?
                .phase = TextPhase::Semantic;
        }
        for (id, value) in attributes {
            let property = context.GetProperty(&id)?;
            permitted()?;
            property.SetValue(
                ec,
                &range,
                &windows::Win32::System::Variant::VARIANT::from(value),
            )?;
            permitted()?;
            let actual = property.GetValue(ec, &range)?;
            permitted()?;
            if actual.vt() != windows::Win32::System::Variant::VT_I4
                || i32::try_from(&actual)? != value
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
        }
        let caret = match selected {
            Some(selected) => selected,
            None => {
                let caret = range.Clone()?;
                caret.Collapse(ec, TF_ANCHOR_END)?;
                caret
            }
        };
        permitted()?;
        let mut selected = TF_SELECTION {
            range: ManuallyDrop::new(Some(caret)),
            style: TF_SELECTIONSTYLE {
                ase: TF_AE_NONE,
                fInterimChar: false.into(),
            },
        };
        let result = context.SetSelection(ec, std::slice::from_ref(&selected));
        ManuallyDrop::drop(&mut selected.range);
        result?;
        let current = selection(context, ec)?;
        permitted()?;
        shared.borrow_mut().selection = Some(current);
    }
    Ok(())
}
// A non-foreground sink only participates while this service owns text. Hand off
// before the application consumes the key: ending a Chromium composition after
// Backspace can restore its cached pre-edit text. The original key is never eaten
// or replayed, and OnTestKeyDown has no side effects.
#[implement(ITfKeyEventSink)]
struct KeySink {
    state: std::rc::Weak<RefCell<State>>,
    _lease: Lease,
}
impl KeySink {
    fn owns_key(&self, context: Ref<ITfContext>, key: WPARAM) -> Option<Shared> {
        if matches!(key.0, 0x10..=0x14 | 0x1b | 0x5b..=0x5c | 0xa0..=0xa5) {
            return None;
        }
        let shared = self.state.upgrade()?;
        let owns = {
            let s = shared.borrow();
            s.active && s.version.owned && !s.cleanup && s.context.as_ref() == context.as_ref()
        };
        owns.then_some(shared)
    }
}
impl ITfKeyEventSink_Impl for KeySink_Impl {
    fn OnSetFocus(&self, _: BOOL) -> windows::core::Result<()> {
        Ok(())
    }
    fn OnTestKeyDown(
        &self,
        context: Ref<ITfContext>,
        key: WPARAM,
        _: LPARAM,
    ) -> windows::core::Result<BOOL> {
        Ok(self.owns_key(context, key).is_some().into())
    }
    fn OnKeyDown(
        &self,
        context: Ref<ITfContext>,
        key: WPARAM,
        _: LPARAM,
    ) -> windows::core::Result<BOOL> {
        if let Some(shared) = self.owns_key(context, key) {
            let ended = end_owned_with_flags(&shared, TF_ES_SYNC);
            shared.borrow_mut().key_handoff = ended;
        }
        Ok(false.into())
    }
    fn OnTestKeyUp(&self, _: Ref<ITfContext>, _: WPARAM, _: LPARAM) -> windows::core::Result<BOOL> {
        Ok(false.into())
    }
    fn OnKeyUp(&self, _: Ref<ITfContext>, _: WPARAM, _: LPARAM) -> windows::core::Result<BOOL> {
        Ok(false.into())
    }
    fn OnPreservedKey(&self, _: Ref<ITfContext>, _: *const GUID) -> windows::core::Result<BOOL> {
        Ok(false.into())
    }
}
#[implement(ITfEditSession)]
struct EndOwned {
    state: Shared,
    context: Option<ITfContext>,
    composition: ITfComposition,
    epoch: u64,
    cleanup_id: u64,
    _lease: Lease,
}
impl ITfEditSession_Impl for EndOwned_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        let guard = || {
            cleanup_identity(
                &self.state,
                &self.context,
                &self.composition,
                self.epoch,
                self.cleanup_id,
            )
        };
        guard()?;
        let mut mutation = Mutation::NotAttempted;
        let semantic = if let Some(context) = &self.context {
            materialize(
                &self.state,
                context,
                &self.composition,
                self.epoch,
                ec,
                &guard,
                true,
                &mut mutation,
            )
        } else if self.state.borrow().text.is_none() {
            Ok(())
        } else {
            Err(failure(TF_E_DISCONNECTED))
        };
        guard()?;
        let result = unsafe { self.composition.EndComposition(ec) };
        if result.is_ok() {
            release_composition_with_failure(
                &self.state,
                self.epoch,
                Some(&self.composition),
                semantic.is_err(),
            );
        } else {
            mark_handoff_failed(&self.state, self.epoch);
        }
        semantic.and(result)
    }
}
#[implement(ITfCompositionSink)]
struct CompositionSink {
    state: Shared,
    epoch: u64,
    _lease: Lease,
}
impl ITfCompositionSink_Impl for CompositionSink_Impl {
    fn OnCompositionTerminated(
        &self,
        ec: u32,
        composition: Ref<ITfComposition>,
    ) -> windows::core::Result<()> {
        #[cfg(feature = "acceptance")]
        eprintln!(
            "composition terminated: epoch={} generation={} owned={}",
            self.epoch,
            self.state.borrow().generation,
            self.state.borrow().version.owned
        );
        let binding = {
            let s = self.state.borrow();
            (s.epoch == self.epoch && s.composition.as_ref() == composition.as_ref())
                .then(|| (s.context.clone(), s.generation))
        };
        let owned = binding.is_some();
        let mut semantic = Ok(());
        if let Some((Some(context), generation)) = binding
            && let Some(composition) = composition.as_ref()
        {
            let guard = || {
                let s = self.state.borrow();
                if s.epoch != self.epoch
                    || s.generation != generation
                    || s.context.as_ref() != Some(&context)
                    || s.composition.as_ref() != Some(composition)
                {
                    Err(failure(TF_E_DISCONNECTED))
                } else {
                    Ok(())
                }
            };
            semantic = materialize(
                &self.state,
                &context,
                composition,
                self.epoch,
                ec,
                &guard,
                true,
                &mut Mutation::NotAttempted,
            );
        }
        release_composition_with_failure(
            &self.state,
            self.epoch,
            composition.as_ref(),
            owned && semantic.is_err(),
        );
        semantic
    }
}
// Uniform scalar attributes can be preserved through a cumulative text correction.
// Mixed/partial or opaque properties cannot be mapped to revised text: hand off instead.
unsafe fn changed_attributes(
    context: &ITfContext,
    record: &ITfEditRecord,
    owned: &ITfRange,
    ec: u32,
) -> windows::core::Result<Vec<(GUID, i32)>> {
    unsafe {
        let properties = context.EnumProperties()?;
        let mut result = Vec::new();
        for _ in 0..64 {
            let mut entries = [None];
            let mut fetched = 0;
            properties.Next(&mut entries, &mut fetched)?;
            if fetched == 0 {
                return Ok(result);
            }
            let property = entries[0].as_ref().ok_or_else(|| failure(E_FAIL))?;
            let id = property.GetType()?;
            let updates =
                record.GetTextAndPropertyUpdates(GET_TEXT_AND_PROPERTY_UPDATES_FLAGS(0), &[&id])?;
            let mut relevant = false;
            let mut ended = false;
            for _ in 0..64 {
                let mut ranges = [None];
                let mut count = 0;
                updates.Next(&mut ranges, &mut count)?;
                if count == 0 {
                    ended = true;
                    break;
                }
                let range = ranges[0].as_ref().ok_or_else(|| failure(E_FAIL))?;
                if range.CompareStart(ec, owned, TF_ANCHOR_END)? >= 0
                    || range.CompareEnd(ec, owned, TF_ANCHOR_START)? <= 0
                {
                    continue;
                }
                if range.CompareStart(ec, owned, TF_ANCHOR_START)? > 0
                    || range.CompareEnd(ec, owned, TF_ANCHOR_END)? < 0
                {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                relevant = true;
            }
            if !ended {
                return Err(failure(E_FAIL));
            }
            if relevant {
                let value = property.GetValue(ec, owned)?;
                if value.vt() != windows::Win32::System::Variant::VT_I4 {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                result.push((id, i32::try_from(&value)?));
            }
        }
        Err(failure(E_FAIL))
    }
}
#[implement(ITfTextEditSink)]
struct TextSink {
    state: Shared,
    epoch: u64,
    _lease: Lease,
}
#[derive(Clone)]
struct EditBinding {
    epoch: u64,
    generation: u64,
    revision: u64,
    composition: Option<ITfComposition>,
    empty_range: Option<ITfRange>,
}
impl EditBinding {
    fn matches(&self, state: &State) -> bool {
        state.active
            && state.epoch == self.epoch
            && state.generation == self.generation
            && state.version.revision == self.revision
            && state.composition == self.composition
            && state.empty_range == self.empty_range
    }
    fn receipt_matches(&self, own: &OwnChange) -> bool {
        own.epoch == self.epoch
            && own.generation == self.generation
            && own.revision == self.revision
    }
}
impl ITfTextEditSink_Impl for TextSink_Impl {
    fn OnEndEdit(
        &self,
        context: Ref<ITfContext>,
        ec: u32,
        record: Ref<ITfEditRecord>,
    ) -> windows::core::Result<()> {
        let (client, binding, expected, expected_selection) = {
            let s = self.state.borrow();
            if s.epoch != self.epoch || !s.version.owned && !s.cleanup {
                return Ok(());
            }
            (
                s.client,
                EditBinding {
                    epoch: s.epoch,
                    generation: s.generation,
                    revision: s.version.revision,
                    composition: s.composition.clone(),
                    empty_range: s.empty_range.clone(),
                },
                s.expected.clone(),
                s.selection.clone(),
            )
        };
        if self.state.borrow().cleanup {
            let changed = (|| unsafe {
                let context = context.as_ref().ok_or_else(|| failure(E_POINTER))?;
                if context.InWriteSession(client)?.as_bool() {
                    return Ok(false);
                }
                let record = record.as_ref().ok_or_else(|| failure(E_POINTER))?;
                let range = binding
                    .composition
                    .as_ref()
                    .ok_or_else(|| failure(TF_E_DISCONNECTED))?
                    .GetRange()?;
                let text = record.GetTextAndPropertyUpdates(TF_GTP_INCL_TEXT, &[])?;
                let mut ranges = [None];
                let mut count = 0;
                text.Next(&mut ranges, &mut count)?;
                Ok::<_, windows::core::Error>(
                    record.GetSelectionStatus()?.as_bool()
                        || count != 0
                        || !changed_attributes(context, record, &range, ec)?.is_empty(),
                )
            })();
            if binding.matches(&self.state.borrow()) && !matches!(changed, Ok(false)) {
                revoke_cleanup_conversion(&self.state, self.epoch);
            }
            return Ok(());
        }
        let result: windows::core::Result<bool> = (|| unsafe {
            let context = context.as_ref().ok_or_else(|| failure(E_POINTER))?;
            if context.InWriteSession(client)?.as_bool() {
                return Ok(false);
            }
            let record = record.as_ref().ok_or_else(|| failure(E_POINTER))?;
            let text_updates = record.GetTextAndPropertyUpdates(TF_GTP_INCL_TEXT, &[])?;
            let mut text_ranges = [None];
            let mut changed_text = 0;
            text_updates.Next(&mut text_ranges, &mut changed_text)?;
            let range = match &binding.composition {
                Some(composition) => composition.GetRange()?,
                None => binding
                    .empty_range
                    .clone()
                    .ok_or_else(|| failure(TF_E_DISCONNECTED))?,
            };
            let selected = selection(context, ec)?;
            let expected_selection =
                expected_selection.ok_or_else(|| failure(TF_E_DISCONNECTED))?;
            if !binding.matches(&self.state.borrow()) {
                return Ok(false);
            }
            let notified = record.GetSelectionStatus()?.as_bool() || changed_text != 0;
            let own_change = {
                self.state
                    .borrow()
                    .own_change
                    .as_ref()
                    .filter(|own| binding.receipt_matches(own))
                    .cloned()
            };
            let own_feedback = if let Some(own) = own_change.filter(|_| notified) {
                let receipt_text = range_text(&own.range, ec)?;
                let receipt_matches = receipt_text == own.expected
                    || (own.range.CompareStart(ec, &range, TF_ANCHOR_START)? == 0
                        && own.range.CompareEnd(ec, &range, TF_ANCHOR_END)? == 0
                        && receipt_text == expected);
                receipt_matches && updates_within(record, &own.range, ec)?
            } else {
                false
            };
            if !binding.matches(&self.state.borrow()) {
                return Ok(false);
            }
            if own_feedback {
                let mut state = self.state.borrow_mut();
                if state
                    .own_change
                    .as_ref()
                    .is_some_and(|own| binding.receipt_matches(own))
                {
                    state.own_change = None;
                }
            }
            if (!own_feedback && (record.GetSelectionStatus()?.as_bool() || changed_text != 0))
                || range_text(&range, ec)? != expected
                || selected.CompareStart(ec, &expected_selection, TF_ANCHOR_START)? != 0
                || selected.CompareEnd(ec, &expected_selection, TF_ANCHOR_END)? != 0
            {
                return Ok(true);
            }
            let mut attributes = changed_attributes(context, record, &range, ec)?;
            if !binding.matches(&self.state.borrow()) {
                return Ok(false);
            }
            // SetText/partial commit may re-emit existing scalar properties.
            // Preserve a user change, not a notification of an unchanged value.
            attributes.retain(|(id, value)| {
                !self
                    .state
                    .borrow()
                    .preserved_attributes
                    .iter()
                    .any(|(known, old)| known == id && old == value)
            });
            if attributes.is_empty() {
                return Ok(false);
            }
            let backup = {
                let mut s = self.state.borrow_mut();
                if !binding.matches(&s) || !s.version.owned {
                    return Ok(false);
                }
                let added = attributes
                    .iter()
                    .filter(|(id, _)| !s.preserved_attributes.iter().any(|(known, _)| known == id))
                    .count();
                if s.preserved_attributes.len() + added > 64 {
                    return Ok(true);
                }
                for (id, value) in attributes {
                    if let Some((_, old)) = s
                        .preserved_attributes
                        .iter_mut()
                        .find(|(known, _)| *known == id)
                    {
                        *old = value;
                    } else {
                        s.preserved_attributes.push((id, value));
                    }
                }
                s.backup.take()
            };
            drop(backup);
            Ok(false)
        })();
        let current = {
            let s = self.state.borrow();
            binding.matches(&s) && s.version.owned
        };
        if current && !matches!(&result, Ok(false)) {
            end_owned(&self.state);
        }
        result?;
        Ok(())
    }
}
// Reject unknown/oversized records and every update outside the captured write.
fn updates_within(
    record: &ITfEditRecord,
    owned: &ITfRange,
    ec: u32,
) -> windows::core::Result<bool> {
    unsafe {
        let updates = record.GetTextAndPropertyUpdates(TF_GTP_INCL_TEXT, &[])?;
        for _ in 0..64 {
            let mut ranges = [None];
            let mut count = 0;
            updates.Next(&mut ranges, &mut count)?;
            if count == 0 {
                return Ok(true);
            }
            let changed = ranges[0].as_ref().ok_or_else(|| failure(E_FAIL))?;
            if changed.CompareStart(ec, owned, TF_ANCHOR_START)? < 0
                || changed.CompareEnd(ec, owned, TF_ANCHOR_END)? > 0
            {
                return Ok(false);
            }
        }
        Ok(false)
    }
}
#[implement(ITfEditSession)]
struct Edit {
    state: Shared,
    context: ITfContext,
    request: Request,
    generation: u64,
    connection: u64,
    _lease: Lease,
}
impl ITfEditSession_Impl for Edit_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        let mut mutation = Mutation::NotAttempted;
        let mut settlement = (self.request.action == Action::Settle).then(Settlement::default);
        let mut phase = "lease";

        let result =
            lease(&self.state, &self.request, self.generation, &self.context).and_then(|()| {
                apply(
                    &self.state,
                    &self.context,
                    &self.request,
                    self.generation,
                    ec,
                    &mut mutation,
                    &mut phase,
                    &mut settlement,
                )
            });
        match &result {
            Ok(()) => complete(
                &self.state,
                &self.request,
                self.connection,
                true,
                "applied",
                mutation,
                settlement,
            ),
            Err(e) => {
                if let Some(progress) = &mut settlement {
                    progress.tail_owned = false;
                }
                complete(
                    &self.state,
                    &self.request,
                    self.connection,
                    false,
                    &format!("edit:{phase}:{:08x}", e.code().0 as u32),
                    mutation,
                    settlement,
                );
                // A stale callback must not hand off a newer draft on the same service.
                if self.state.borrow().generation == self.generation {
                    end_owned(&self.state);
                }
            }
        };
        result
    }
}
fn complete(
    shared: &Shared,
    r: &Request,
    connection: u64,
    applied: bool,
    code: &str,
    mutation: Mutation,
    settlement: Option<Settlement>,
) {
    let current = shared.borrow().pending.as_ref().is_some_and(|p| {
        p.operation == r.operation
            && p.task == r.task
            && p.turn == r.turn
            && p.connection == connection
            && p.capability == r.capability
    });
    if current {
        let settlement =
            settlement.or_else(|| shared.borrow().pending.as_ref().and_then(|p| p.settlement));
        reply_outcome(shared, r, connection, applied, code, mutation, settlement);
    }
}
fn selection(context: &ITfContext, ec: u32) -> windows::core::Result<ITfRange> {
    unsafe {
        let mut selected = [TF_SELECTION::default()];
        let mut fetched = 0;
        let result = context.GetSelection(ec, TF_DEFAULT_SELECTION, &mut selected, &mut fetched);
        let range = ManuallyDrop::take(&mut selected[0].range);
        result?;
        if fetched != 1 {
            return Err(failure(E_FAIL));
        }
        range.ok_or_else(|| failure(E_FAIL))
    }
}
fn range_text(range: &ITfRange, ec: u32) -> windows::core::Result<String> {
    let mut data = vec![0u16; MAX_TEXT_UTF16 + 1];
    let mut fetched = 0;
    unsafe {
        range.GetText(ec, 0, &mut data, &mut fetched)?;
    }
    if fetched as usize > MAX_TEXT_UTF16 {
        return Err(failure(E_INVALIDARG));
    }
    String::from_utf16(&data[..fetched as usize]).map_err(|_| failure(E_FAIL))
}
// Disabled/empty contexts can still expose writable TSF ranges (for example
// browser password fields). InputScope alone is not an admission decision.
fn ensure_enabled_context(context: &ITfContext) -> windows::core::Result<()> {
    use windows::Win32::System::Variant::{VT_EMPTY, VT_I4};
    unsafe {
        let compartments: ITfCompartmentMgr = context.cast()?;
        for id in [
            GUID_COMPARTMENT_KEYBOARD_DISABLED,
            GUID_COMPARTMENT_EMPTYCONTEXT,
        ] {
            let value = compartments.GetCompartment(&id)?.GetValue()?;
            match value.vt() {
                VT_EMPTY => (), // Not set by the context owner.
                VT_I4 if i32::try_from(&value)? == 0 => (),
                VT_I4 => return Err(failure(E_ACCESSDENIED)),
                _ => return Err(failure(E_INVALIDARG)),
            }
        }
        Ok(())
    }
}
fn password(context: &ITfContext, range: &ITfRange, ec: u32) -> windows::core::Result<bool> {
    unsafe {
        let window = context.GetActiveView()?.GetWnd()?;
        if !window.0.is_null() && GetWindowLongPtrW(window, GWL_STYLE) as u32 & 0x20 != 0 {
            return Ok(true);
        }
        if let Ok(property) = context.GetAppProperty(&GUID_PROP_INPUTSCOPE)
            && let Ok(value) = property.GetValue(ec, range)
            && let Ok(unknown) = IUnknown::try_from(&value)
            && let Ok(scope) = unknown.cast::<ITfInputScope>()
        {
            let mut scopes = std::ptr::null_mut();
            let mut count = 0;
            scope.GetInputScopes(&mut scopes, &mut count)?;
            let found = if scopes.is_null() {
                false
            } else {
                std::slice::from_raw_parts(scopes, count as usize).contains(&IS_PASSWORD)
            };
            CoTaskMemFree(Some(scopes.cast()));
            return Ok(found);
        }
        Ok(false)
    }
}
#[allow(clippy::too_many_arguments)]
fn settle_prefix(
    shared: &Shared,
    context: &ITfContext,
    composition: &ITfComposition,
    range: &ITfRange,
    r: &Request,
    generation: u64,
    epoch: u64,
    ec: u32,
    mutation: &mut Mutation,
    phase: &mut &'static str,
    outcome: &mut Settlement,
) -> windows::core::Result<()> {
    unsafe {
        let prefix = r.prefix.as_ref().ok_or_else(|| failure(E_INVALIDARG))?;
        *phase = "settleIdentity";
        if prefix.range_epoch != epoch
            || shared.borrow().version.revision != prefix.expected_revision
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        lease(shared, r, generation, context)?;
        let committed = CompositionText::new(&prefix.text).commit_text();
        let tail = CompositionText::new(&r.text);
        let combined = format!("{committed}{}", tail.marked);
        *phase = "settleText";
        *mutation = Mutation::Uncertain;
        range.SetText(
            ec,
            TF_ST_CORRECTION,
            &combined.encode_utf16().collect::<Vec<_>>(),
        )?;
        *mutation = Mutation::Changed;
        edit_identity(shared, r, generation, epoch)?;
        preserve_attributes(shared, context, range, r, generation, epoch, ec)?;
        *phase = "settleReadback";
        let actual = range_text(range, ec)?;
        edit_identity(shared, r, generation, epoch)?;
        let written = range.Clone()?;
        edit_identity(shared, r, generation, epoch)?;
        #[cfg(feature = "acceptance")]
        eprintln!(
            "prefix readback expected_units={} actual_units={} prefix_units={} matched={}",
            combined.encode_utf16().count(),
            actual.encode_utf16().count(),
            committed.encode_utf16().count(),
            crate::prefix::boundary(&actual, &committed).is_some()
        );
        let (bytes, units) = crate::prefix::boundary(&actual, &committed)
            .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
        if !tail.matches_marked(&actual[bytes..]) {
            *phase = "settleTailReadback";
            return Err(failure(TF_E_DISCONNECTED));
        }
        let boundary = range.Clone()?;
        edit_identity(shared, r, generation, epoch)?;
        let mut shifted = 0;
        boundary.ShiftStart(ec, units, &mut shifted, std::ptr::null())?;
        edit_identity(shared, r, generation, epoch)?;
        if shifted != units {
            return Err(failure(TF_E_DISCONNECTED));
        }
        boundary.Collapse(ec, TF_ANCHOR_START)?;
        edit_identity(shared, r, generation, epoch)?;
        *phase = "settlePrefix";
        // From this point prefix submission may be irreversible. Reentrant
        // Abort must never restore the original selection into the remaining tail.
        let retired_backup = { shared.borrow_mut().backup.take() };
        drop(retired_backup);
        edit_identity(shared, r, generation, epoch)?;
        outcome.prefix_operation = r.operation;
        outcome.prefix = Mutation::Uncertain;
        note_prefix_progress(shared, r, generation, epoch, outcome.prefix)?;
        let remaining = if r.text.is_empty() {
            // An empty composition is terminated by IMM providers. End it
            // intentionally and retain only a guarded empty insertion lease.
            let empty = range.Clone()?;
            empty.Collapse(ec, TF_ANCHOR_END)?;
            edit_identity(shared, r, generation, epoch)?;
            composition.EndComposition(ec)?;
            empty
        } else {
            composition.ShiftStart(ec, &boundary)?;
            composition.GetRange()?
        };
        outcome.prefix = Mutation::Changed;
        note_prefix_progress(shared, r, generation, epoch, outcome.prefix)?;
        edit_identity(shared, r, generation, epoch)?;
        *phase = "settleTail";
        edit_identity(shared, r, generation, epoch)?;
        let actual_tail = range_text(&remaining, ec)?;
        edit_identity(shared, r, generation, epoch)?;
        if !tail.matches_marked(&actual_tail) {
            return Err(failure(TF_E_DISCONNECTED));
        }
        // Install verified range truth before more COM calls can fail/reenter.
        // Cleanup must never compare this shrunken range to the old ASR buffer.
        let retired = {
            let mut state = shared.borrow_mut();
            state.expected = actual_tail;
            state.text = (!r.text.is_empty()).then_some(tail);
            state.backup = Some(Rollback::GeneratedTail);
            state.selection = None;
            if let Some(position) = &mut state.native_position {
                position.offset = position
                    .offset
                    .checked_add(committed.replace('\n', "\r\n").encode_utf16().count() as u32)
                    .ok_or_else(|| failure(E_INVALIDARG))?;
            }
            if r.text.is_empty() {
                state.empty_range = Some(remaining.clone());
                state.materialize_allowed = false;
                state.composition.take()
            } else {
                None
            }
        };
        drop(retired);
        edit_identity(shared, r, generation, epoch)?;
        #[cfg(feature = "acceptance")]
        match SETTLE_FAULT.load(Ordering::Acquire) {
            4 => {
                SETTLE_FAULT.store(0, Ordering::Release);
                return Err(failure(E_FAIL));
            }
            5 => {
                SETTLE_FAULT.store(0, Ordering::Release);
                let connection = shared.borrow().connection;
                dispatch(
                    shared,
                    Request {
                        action: Action::Abort,
                        operation: r.operation + 1,
                        text: String::new(),
                        prefix: None,
                        ..r.clone()
                    },
                    connection,
                );
                edit_identity(shared, r, generation, epoch)?;
            }
            _ => (),
        }
        // The remaining tail is generated text; cancellation erases only this
        // owned range. Never restore an old selection across committed text.
        let caret = remaining.Clone()?;
        caret.Collapse(ec, TF_ANCHOR_END)?;
        edit_identity(shared, r, generation, epoch)?;
        let mut selected = TF_SELECTION {
            range: ManuallyDrop::new(Some(caret)),
            style: TF_SELECTIONSTYLE {
                ase: TF_AE_NONE,
                fInterimChar: false.into(),
            },
        };
        let result = context.SetSelection(ec, std::slice::from_ref(&selected));
        ManuallyDrop::drop(&mut selected.range);
        result?;
        edit_identity(shared, r, generation, epoch)?;
        let selected = selection(context, ec)?;
        edit_identity(shared, r, generation, epoch)?;
        // An IMM-backed provider can rebase ranges after prefix materialization.
        // Bind the receipt to its post-operation range representation, not to
        // the earlier combined buffer, and still require the current tail/caret.
        let receipt_text = range_text(&written, ec)?;
        edit_identity(shared, r, generation, epoch)?;
        let mut state = shared.borrow_mut();
        state.selection = Some(selected);
        state.own_change = Some(OwnChange {
            epoch,
            generation,
            revision: state.version.revision,
            range: written,
            expected: receipt_text,
        });
        outcome.tail_owned = true;
        drop(state);
        #[cfg(feature = "acceptance")]
        settle_checkpoint(
            shared,
            r,
            context,
            composition,
            epoch,
            ec,
            generation,
            mutation,
        )?;
        edit_identity(shared, r, generation, epoch)?;
        {
            let mut state = shared.borrow_mut();
            state.version.applied(r);
            if let Some(receipt) = &mut state.own_change {
                receipt.revision = r.revision;
            }
        }
        Ok(())
    }
}

fn note_prefix_progress(
    shared: &Shared,
    request: &Request,
    generation: u64,
    epoch: u64,
    mutation: Mutation,
) -> windows::core::Result<()> {
    edit_identity(shared, request, generation, epoch)?;
    let mut state = shared.borrow_mut();
    let pending = state
        .pending
        .as_mut()
        .filter(|p| {
            p.operation == request.operation
                && p.task == request.task
                && p.turn == request.turn
                && p.capability == request.capability
        })
        .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
    pending.settlement = Some(Settlement {
        prefix_operation: request.operation,
        prefix: mutation,
        tail_owned: false,
    });
    Ok(())
}
#[allow(clippy::too_many_arguments)]
fn preserve_attributes(
    shared: &Shared,
    context: &ITfContext,
    range: &ITfRange,
    r: &Request,
    generation: u64,
    epoch: u64,
    ec: u32,
) -> windows::core::Result<()> {
    unsafe {
        let attributes = { shared.borrow().preserved_attributes.clone() };
        for (id, value) in attributes {
            lease(shared, r, generation, context)?;
            let property = context.GetProperty(&id)?;
            edit_identity(shared, r, generation, epoch)?;
            property.SetValue(
                ec,
                range,
                &windows::Win32::System::Variant::VARIANT::from(value),
            )?;
            edit_identity(shared, r, generation, epoch)?;
            let actual = property.GetValue(ec, range)?;
            edit_identity(shared, r, generation, epoch)?;
            if actual.vt() != windows::Win32::System::Variant::VT_I4
                || i32::try_from(&actual)? != value
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
        }
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn write_composing(
    shared: &Shared,
    context: &ITfContext,
    range: &ITfRange,
    r: &Request,
    generation: u64,
    epoch: u64,
    ec: u32,
    mutation: &mut Mutation,
    phase: &mut &'static str,
    text: &str,
    advance: bool,
) -> windows::core::Result<()> {
    unsafe {
        *phase = "updateSafety";
        if password(context, range, ec)? {
            return Err(failure(E_ACCESSDENIED));
        }
        *phase = "updateLease";
        lease(shared, r, generation, context)?;
        *mutation = Mutation::Uncertain;
        *phase = "updateText";
        let text = CompositionText::new(text);
        range.SetText(
            ec,
            TF_ST_CORRECTION,
            &text.marked.encode_utf16().collect::<Vec<_>>(),
        )?;
        *mutation = Mutation::Changed;
        *phase = "updateAttributes";
        preserve_attributes(shared, context, range, r, generation, epoch, ec)?;
        *phase = "updateReadback";
        let actual = range_text(range, ec)?;
        // RichEdit and browser text stores can use different newline representations.
        // Preserve the provider's range text; permit only equivalent line breaks.
        if !text.matches_marked(&actual) {
            #[cfg(feature = "acceptance")]
            eprintln!(
                "marked readback mismatch: expected_units={} actual_units={} break_codes={:?}",
                text.marked.encode_utf16().count(),
                actual.encode_utf16().count(),
                actual
                    .chars()
                    .filter(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
                    .map(|c| c as u32)
                    .take(16)
                    .collect::<Vec<_>>()
            );
            return Err(failure(TF_E_DISCONNECTED));
        }
        {
            let mut s = shared.borrow_mut();
            if s.generation != generation || s.epoch != epoch || !s.version.owned {
                return Err(failure(TF_E_DISCONNECTED));
            }
            s.expected = actual;
            s.text = Some(text);
            if advance {
                s.version.applied(r);
            }
            s.selection = None;
        }
        *phase = "updateCaret";
        let caret = range.Clone()?;
        caret.Collapse(ec, TF_ANCHOR_END)?;
        {
            let s = shared.borrow();
            if !s.active || s.generation != generation || s.epoch != epoch || !s.version.owned {
                return Err(failure(TF_E_DISCONNECTED));
            }
        }
        let mut selected = TF_SELECTION {
            range: ManuallyDrop::new(Some(caret)),
            style: TF_SELECTIONSTYLE {
                ase: TF_AE_NONE,
                fInterimChar: false.into(),
            },
        };
        *phase = "updateSelection";
        let result = context.SetSelection(ec, std::slice::from_ref(&selected));
        ManuallyDrop::drop(&mut selected.range);
        result?;
        let next_selection = selection(context, ec)?;
        let mut s = shared.borrow_mut();
        if s.generation != generation || s.epoch != epoch || !s.version.owned {
            return Err(failure(TF_E_DISCONNECTED));
        }
        s.selection = Some(next_selection);
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)]
fn apply(
    shared: &Shared,
    context: &ITfContext,
    r: &Request,
    generation: u64,
    ec: u32,
    mutation: &mut Mutation,
    phase: &mut &'static str,
    settlement: &mut Option<Settlement>,
) -> windows::core::Result<()> {
    shared.borrow_mut().own_change = None;
    unsafe {
        if r.action == Action::Begin {
            shared.borrow_mut().key_handoff = false;
            *phase = "beginSelection";
            let range = selection(context, ec)?;
            *phase = "beginProperties";
            if password(context, &range, ec)? {
                return Err(failure(E_ACCESSDENIED));
            }
            let composition_context: ITfContextComposition = context.cast()?;
            let enumerator = composition_context.EnumCompositions()?;
            let mut compositions = [None];
            let mut fetched = 0;
            enumerator.Next(&mut compositions, &mut fetched)?;
            if fetched != 0 {
                return Err(failure(TF_E_LOCKED));
            }
            let backup = context.CreateRangeBackup(ec, &range)?;
            *phase = "beginText";
            let expected = range_text(&range, ec)?;
            let original_selection = range.Clone()?;
            let native_position = native_edit_position(context)?;
            lease(shared, r, generation, context)?;
            let epoch = COMPOSITION_EPOCH.fetch_add(1, Ordering::AcqRel) + 1;
            let sink: ITfCompositionSink = CompositionSink {
                state: shared.clone(),
                epoch,
                _lease: Lease::new(),
            }
            .into();
            let text_sink: ITfTextEditSink = TextSink {
                state: shared.clone(),
                epoch,
                _lease: Lease::new(),
            }
            .into();
            let source: ITfSource = context.cast()?;
            *phase = "beginWatch";
            let cookie = source.AdviseSink(&ITfTextEditSink::IID, &text_sink)?;
            if let Err(e) = lease(shared, r, generation, context) {
                let _ = source.UnadviseSink(cookie);
                return Err(e);
            }
            *phase = "beginComposition";
            let composition = match composition_context.StartComposition(ec, &range, &sink) {
                Ok(c) => c,
                Err(e) => {
                    let _ = source.UnadviseSink(cookie);
                    return Err(e);
                }
            };
            if shared.borrow().generation != generation || !shared.borrow().active {
                let _ = composition.EndComposition(ec);
                let _ = source.UnadviseSink(cookie);
                return Err(failure(TF_E_DISCONNECTED));
            }
            let mut s = shared.borrow_mut();
            s.epoch = epoch;
            s.composition = Some(composition);
            s.preserved_attributes.clear();
            s.backup = Some(Rollback::OriginalSelection(backup));
            s.context = Some(context.clone());
            s.expected = expected;
            s.text = None;
            s.materialize_allowed = true;
            s.selection = Some(original_selection);
            s.native_position = native_position;
            s.text_cookie = Some(cookie);
            s.version.applied(r);
            return Ok(());
        }
        *phase = "ownership";
        let (mut composition, expected, old_selection, epoch, empty_range, cancel_generated_tail) = {
            let s = shared.borrow();
            (
                s.composition.clone(),
                s.expected.clone(),
                s.selection.clone(),
                s.epoch,
                s.empty_range.clone(),
                r.action == Action::Abort && matches!(s.backup, Some(Rollback::GeneratedTail)),
            )
        };
        *phase = "range";
        let mut range = match &composition {
            Some(composition) => composition.GetRange()?,
            None => empty_range.ok_or_else(|| failure(TF_E_DISCONNECTED))?,
        };
        *phase = "position";
        if password(context, &range, ec)? || range_text(&range, ec)? != expected {
            return Err(failure(TF_E_DISCONNECTED));
        }
        // Cancel erases only a still-owned, unchanged generated range. It does
        // not restore a selection or discover a new insertion position. Writes
        // and restoration of OriginalSelection retain the exact caret guard.
        if !cancel_generated_tail {
            *phase = "selection";
            let old_selection = old_selection.ok_or_else(|| failure(TF_E_DISCONNECTED))?;
            let current_selection = selection(context, ec)?;
            if current_selection.CompareStart(ec, &old_selection, TF_ANCHOR_START)? != 0
                || current_selection.CompareEnd(ec, &old_selection, TF_ANCHOR_END)? != 0
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
        }
        // Reopen the same logical draft only at the still-owned empty caret.
        // User edits revoke this lease through the existing key/text sinks.
        if composition.is_none() {
            edit_identity(shared, r, generation, epoch)?;
            // Standard EDIT's TSF virtual selection can be cached outside a
            // composition. Check its documented UTF-16 selection capability;
            // this is a control adapter, never an application-name exception.
            *phase = "gapPosition";
            let proof = { shared.borrow().native_position };
            if let Some(proof) = proof {
                let actual = edit_selection(HWND(proof.window as *mut _));
                #[cfg(feature = "acceptance")]
                eprintln!(
                    "ordered gap selection expected={} actual_start={} actual_end={}",
                    proof.offset, actual.0, actual.1
                );
                edit_identity(shared, r, generation, epoch)?;
                if actual != (proof.offset, proof.offset) {
                    return Err(failure(TF_E_DISCONNECTED));
                }
            }
            if matches!(r.action, Action::Abort | Action::Commit | Action::Handoff) {
                lease(shared, r, generation, context)?;
                finish_empty_range(shared, r, generation, epoch)?;
                return Ok(());
            }
            *phase = "resumeEmptyTail";
            lease(shared, r, generation, context)?;
            let owner: ITfContextComposition = context.cast()?;
            let sink: ITfCompositionSink = CompositionSink {
                state: shared.clone(),
                epoch,
                _lease: Lease::new(),
            }
            .into();
            let next = owner.StartComposition(ec, &range, &sink)?;
            if let Err(error) = edit_identity(shared, r, generation, epoch) {
                let _ = next.EndComposition(ec);
                return Err(error);
            }
            {
                let mut state = shared.borrow_mut();
                state.composition = Some(next.clone());
                state.empty_range = None;

                state.materialize_allowed = true;
            }
            composition = Some(next);
            range = composition.as_ref().unwrap().GetRange()?;
            edit_identity(shared, r, generation, epoch)?;
        }
        let composition = composition.ok_or_else(|| failure(TF_E_DISCONNECTED))?;
        match r.action {
            Action::Update => {
                write_composing(
                    shared, context, &range, r, generation, epoch, ec, mutation, phase, &r.text,
                    true,
                )?;
            }
            Action::Settle => {
                let outcome = settlement.as_mut().ok_or_else(|| failure(E_INVALIDARG))?;
                settle_prefix(
                    shared,
                    context,
                    &composition,
                    &range,
                    r,
                    generation,
                    epoch,
                    ec,
                    mutation,
                    phase,
                    outcome,
                )?;
            }
            Action::Abort => {
                *phase = "abort";
                let backup = { shared.borrow().backup.clone() };
                let Some(backup) = backup else {
                    // Finish even when property changes made rollback unsafe.
                    materialize(
                        shared,
                        context,
                        &composition,
                        epoch,
                        ec,
                        &|| edit_identity(shared, r, generation, epoch),
                        true,
                        mutation,
                    )?;
                    composition.EndComposition(ec)?;
                    release_composition(shared, epoch, Some(&composition));
                    return Err(failure(TF_E_DISCONNECTED));
                };
                *mutation = Mutation::Uncertain;
                lease(shared, r, generation, context)?;
                *phase = "abortRestore";
                match backup {
                    Rollback::OriginalSelection(backup) => backup.Restore(ec, &range)?,
                    Rollback::GeneratedTail => range.SetText(ec, TF_ST_CORRECTION, &[])?,
                }
                *mutation = Mutation::Changed;
                if shared.borrow().generation == generation {
                    shared.borrow_mut().version.applied(r);
                }
                *phase = "abortEnd";
                composition.EndComposition(ec)?;
                release_composition(shared, epoch, Some(&composition));
            }
            Action::Commit | Action::Handoff => {
                *phase = "commit";
                lease(shared, r, generation, context)?;
                materialize(
                    shared,
                    context,
                    &composition,
                    epoch,
                    ec,
                    &|| edit_identity(shared, r, generation, epoch),
                    r.action == Action::Handoff,
                    mutation,
                )?;
                edit_identity(shared, r, generation, epoch)?;
                shared.borrow_mut().version.applied(r);
                composition.EndComposition(ec)?;
                release_composition(shared, epoch, Some(&composition));
            }
            Action::Begin | Action::Inspect => unreachable!(),
        }
        Ok(())
    }
}
fn finish_empty_range(
    shared: &Shared,
    request: &Request,
    generation: u64,
    epoch: u64,
) -> windows::core::Result<()> {
    edit_identity(shared, request, generation, epoch)?;
    {
        let mut state = shared.borrow_mut();
        if state.composition.is_some() || state.empty_range.is_none() {
            return Err(failure(TF_E_DISCONNECTED));
        }
        state.version.applied(request);
    }
    release_composition(shared, epoch, None);
    Ok(())
}
#[cfg(feature = "acceptance")]
static SETTLE_FAULT: AtomicUsize = AtomicUsize::new(0);
#[cfg(feature = "acceptance")]
#[unsafe(no_mangle)]
pub extern "system" fn SayweaveSettleFixtureFault(mode: usize) {
    SETTLE_FAULT.store(mode, Ordering::Release);
}
#[cfg(feature = "acceptance")]
#[allow(clippy::too_many_arguments)]
fn settle_checkpoint(
    shared: &Shared,
    request: &Request,
    context: &ITfContext,
    composition: &ITfComposition,
    epoch: u64,
    ec: u32,
    generation: u64,
    mutation: &mut Mutation,
) -> windows::core::Result<()> {
    match SETTLE_FAULT.swap(0, Ordering::AcqRel) {
        1 => {
            let connection = shared.borrow().connection;
            let abort = Request {
                action: Action::Abort,
                operation: request.operation + 1,
                text: String::new(),
                prefix: None,
                ..request.clone()
            };
            dispatch(shared, abort, connection);
            Ok(())
        }
        2 => Err(failure(E_FAIL)),
        3 => {
            // Provider termination is allowed to reenter inside a write lock.
            // Preserve the already-visible raw tail before relinquishing it.
            materialize(
                shared,
                context,
                composition,
                epoch,
                ec,
                &|| edit_identity(shared, request, generation, epoch),
                true,
                mutation,
            )?;
            unsafe {
                composition.EndComposition(ec)?;
            }
            release_composition(shared, epoch, Some(composition));
            shared.borrow_mut().key_handoff = true;
            Ok(())
        }
        _ => Ok(()),
    }
}
#[implement(IClassFactory)]
struct Factory {
    _lease: Lease,
}
impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(
        &self,
        outer: Ref<IUnknown>,
        iid: *const GUID,
        out: *mut *mut c_void,
    ) -> windows::core::Result<()> {
        if outer.as_ref().is_some() {
            return Err(failure(CLASS_E_NOAGGREGATION));
        }
        if iid.is_null() || out.is_null() {
            return Err(failure(E_POINTER));
        }
        let service: ITfTextInputProcessor = Service {
            state: Rc::new(RefCell::new(State::default())),
            _lease: Lease::new(),
        }
        .into();
        unsafe { service.query(iid, out).ok() }
    }
    fn LockServer(&self, lock: BOOL) -> windows::core::Result<()> {
        if lock.as_bool() {
            LOCKS.fetch_add(1, Ordering::AcqRel);
        } else {
            let _ = LOCKS.try_update(Ordering::AcqRel, Ordering::Acquire, |n| n.checked_sub(1));
        }
        Ok(())
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    class: *const GUID,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    if class.is_null() || iid.is_null() || out.is_null() {
        return E_POINTER;
    }
    unsafe {
        *out = std::ptr::null_mut();
        if *class != registration::CLASS {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IClassFactory = Factory {
            _lease: Lease::new(),
        }
        .into();
        factory.query(iid, out)
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if OBJECTS.load(Ordering::Acquire) == 0 && LOCKS.load(Ordering::Acquire) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancel_version_precedes_reentrant_range_retirement() {
        let old = uuid::Uuid::new_v4().to_string();
        let newer = uuid::Uuid::new_v4().to_string();
        let request = Request {
            protocol: PROTOCOL,
            capability: uuid::Uuid::new_v4().to_string(),
            task: old.clone(),
            turn: 1,
            operation: 4,
            revision: 2,
            deadline_tick: 1000,
            action: Action::Abort,
            text: String::new(),
            prefix: None,
        };
        let shared = Rc::new(RefCell::new(State::default()));
        apply_cancel_then_retire(&shared, &request, |_| {
            assert_eq!(shared.borrow().version.task, old);
            assert_eq!(shared.borrow().version.operation, 4);
            // UnadviseSink/range Release can reenter and establish a new task.
            let mut state = shared.borrow_mut();
            state.epoch = 9;
            state.version = DraftVersion {
                task: newer.clone(),
                turn: 1,
                operation: 1,
                revision: 0,
                owned: true,
            };
        });
        let state = shared.borrow();
        assert_eq!(state.epoch, 9);
        assert_eq!(state.version.task, newer);
        assert_eq!(
            (
                state.version.operation,
                state.version.revision,
                state.version.owned
            ),
            (1, 0, true)
        );
    }
    #[test]
    fn duplicate_rejections_cannot_retire_an_in_flight_prefix() {
        let server = Pipe::server().unwrap();
        let client = Pipe::client(std::process::id(), unsafe {
            windows::Win32::System::Threading::GetCurrentThreadId()
        })
        .unwrap();
        assert!(server.connect());
        let request = Request {
            protocol: PROTOCOL,
            task: uuid::Uuid::new_v4().to_string(),
            capability: uuid::Uuid::new_v4().to_string(),
            turn: 1,
            operation: 3,
            revision: 2,
            deadline_tick: unsafe { GetTickCount64() } + 1000,
            action: Action::Settle,
            text: "tail".into(),
            prefix: Some(crate::wire::Prefix {
                text: "prefix".into(),
                expected_revision: 1,
                range_epoch: 7,
            }),
        };
        let shared = Rc::new(RefCell::new(State {
            connected: true,
            connection: 17,
            pipe: Some(server),
            capability: Some(request.capability.clone()),
            version: DraftVersion {
                task: request.task.clone(),
                turn: 1,
                operation: 2,
                revision: 1,
                owned: true,
            },
            pending: Some(Pending {
                task: request.task.clone(),
                turn: 1,
                operation: 3,
                revision: 2,
                connection: 17,
                capability: request.capability.clone(),
                settlement: Some(Settlement {
                    prefix_operation: 3,
                    prefix: Mutation::Changed,
                    tail_owned: false,
                }),
            }),
            ..Default::default()
        }));
        for action in [Action::Settle, Action::Inspect] {
            let duplicate = Request {
                action,
                prefix: if action == Action::Inspect {
                    None
                } else {
                    request.prefix.clone()
                },
                text: if action == Action::Inspect {
                    String::new()
                } else {
                    request.text.clone()
                },
                ..request.clone()
            };
            assert!(duplicate.valid());
            dispatch(&shared, duplicate, 17);
            let frame = client.receive().unwrap().unwrap();
            let answer: Reply = serde_json::from_slice(&frame).unwrap();
            assert!(!answer.applied);
            assert!(
                shared
                    .borrow()
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.operation == 3)
            );
            assert_eq!(shared.borrow().version.revision, 1);
        }
        reply_outcome(
            &shared,
            &request,
            17,
            true,
            "applied",
            Mutation::Changed,
            Some(Settlement {
                prefix_operation: 3,
                prefix: Mutation::Changed,
                tail_owned: true,
            }),
        );
        let frame = client.receive().unwrap().unwrap();
        let answer: Reply = serde_json::from_slice(&frame).unwrap();
        assert!(answer.applied);
        assert!(shared.borrow().pending.is_none());
        assert!(client.receive().unwrap().is_none());
    }
    #[test]
    fn abort_rejections_keep_the_exact_in_flight_prefix_progress() {
        for key_failure in [false, true] {
            let server = Pipe::server().unwrap();
            let client = Pipe::client(std::process::id(), unsafe {
                windows::Win32::System::Threading::GetCurrentThreadId()
            })
            .unwrap();
            assert!(server.connect());
            let task = uuid::Uuid::new_v4().to_string();
            let capability = uuid::Uuid::new_v4().to_string();
            let shared = Rc::new(RefCell::new(State {
                active: true,
                connected: true,
                connection: 17,
                pipe: Some(server),
                capability: Some(capability.clone()),
                key_failure,
                composition: Some(TestComposition { refuse_end: false }.into()),
                version: DraftVersion {
                    task: task.clone(),
                    turn: 1,
                    operation: 2,
                    revision: 1,
                    owned: true,
                },
                pending: Some(Pending {
                    task: task.clone(),
                    turn: 1,
                    operation: 3,
                    revision: 2,
                    connection: 17,
                    capability: capability.clone(),
                    settlement: Some(Settlement {
                        prefix_operation: 3,
                        prefix: Mutation::Changed,
                        tail_owned: false,
                    }),
                }),
                ..Default::default()
            }));
            dispatch(
                &shared,
                Request {
                    protocol: PROTOCOL,
                    task,
                    capability,
                    turn: 1,
                    operation: 4,
                    revision: 2,
                    deadline_tick: unsafe { GetTickCount64() } + 1000,
                    action: Action::Abort,
                    text: String::new(),
                    prefix: None,
                },
                17,
            );
            let frame = client
                .receive()
                .unwrap()
                .expect("Abort failure must terminate its request");
            let reply: Reply = serde_json::from_slice(&frame).unwrap();
            assert!(!reply.applied);
            assert_eq!(reply.operation, 4);
            assert_eq!(reply.mutation, Mutation::NotAttempted);
            assert!(reply.settlement.is_some_and(|s| s.prefix_operation == 3
                && s.prefix == Mutation::Changed
                && !s.tail_owned));
            if key_failure {
                assert_eq!(reply.code, "keyboardHandoffFailed");
            } else {
                assert!(reply.code.starts_with("editManager:"));
            }
        }
    }
    #[test]
    fn old_edit_notice_cannot_own_a_new_write_on_the_same_range() {
        let mut state = State {
            active: true,
            epoch: 7,
            generation: 12,
            version: DraftVersion {
                revision: 4,
                owned: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let notice = EditBinding {
            epoch: 7,
            generation: 12,
            revision: 4,
            composition: None,
            empty_range: None,
        };
        assert!(notice.matches(&state));
        // A provider reenters while an old notice reads selection/properties.
        // The same range/epoch survives but the writer has applied revision5.
        state.version.revision = 5;
        assert!(!notice.matches(&state));
        state.version.revision = 4;
        state.generation = 13;
        assert!(!notice.matches(&state));
        state.generation = 12;
        state.active = false;
        assert!(!notice.matches(&state));
    }
    #[test]
    fn a_reentered_empty_terminal_cannot_overwrite_the_new_task_version() {
        let capability = uuid::Uuid::new_v4().to_string();
        let old_task = uuid::Uuid::new_v4().to_string();
        let new_task = uuid::Uuid::new_v4().to_string();
        let request = Request {
            protocol: PROTOCOL,
            capability: capability.clone(),
            task: old_task,
            turn: 1,
            operation: 4,
            revision: 2,
            action: Action::Commit,
            text: String::new(),
            prefix: None,
            deadline_tick: unsafe { GetTickCount64() } + 1000,
        };
        let shared = Rc::new(RefCell::new(State {
            active: true,
            capability: Some(capability),
            epoch: 9,
            generation: 18,
            version: DraftVersion {
                task: new_task.clone(),
                turn: 1,
                operation: 1,
                revision: 0,
                owned: true,
            },
            ..Default::default()
        }));
        assert!(finish_empty_range(&shared, &request, 17, 8).is_err());
        let state = shared.borrow();
        assert_eq!(state.version.task, new_task);
        assert_eq!(
            (
                state.version.operation,
                state.version.revision,
                state.version.owned
            ),
            (1, 0, true)
        );
        assert_eq!((state.epoch, state.generation), (9, 18));
    }
    #[implement(ITfComposition)]
    struct TestComposition {
        refuse_end: bool,
    }
    impl ITfComposition_Impl for TestComposition_Impl {
        fn GetRange(&self) -> windows::core::Result<ITfRange> {
            Err(failure(E_NOTIMPL))
        }
        fn ShiftStart(&self, _: u32, _: Ref<ITfRange>) -> windows::core::Result<()> {
            Err(failure(E_NOTIMPL))
        }
        fn ShiftEnd(&self, _: u32, _: Ref<ITfRange>) -> windows::core::Result<()> {
            Err(failure(E_NOTIMPL))
        }
        fn EndComposition(&self, _: u32) -> windows::core::Result<()> {
            if self.refuse_end {
                Err(failure(E_FAIL))
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn external_edit_revokes_conversion_but_not_range_cleanup() {
        let composition: ITfComposition = TestComposition { refuse_end: false }.into();
        let state = Rc::new(RefCell::new(State {
            epoch: 7,
            cleanup: true,
            cleanup_id: 42,
            materialize_allowed: true,
            text: Some(CompositionText::new("前\n后")),
            composition: Some(composition.clone()),
            ..Default::default()
        }));
        revoke_cleanup_conversion(&state, 6);
        assert!(state.borrow().materialize_allowed);
        revoke_cleanup_conversion(&state, 7);
        assert!(!state.borrow().materialize_allowed);
        assert!(cleanup_identity(&state, &None, &composition, 7, 42).is_ok());
        let edit: ITfEditSession = EndOwned {
            state: state.clone(),
            context: None,
            composition,
            epoch: 7,
            cleanup_id: 42,
            _lease: Lease::new(),
        }
        .into();
        assert!(unsafe { edit.DoEditSession(0) }.is_err());
        assert!(state.borrow().composition.is_none());
        assert!(state.borrow().key_failure);
    }
    #[test]
    fn repeated_disconnect_preserves_the_pending_cleanup_identity() {
        let composition: ITfComposition = TestComposition { refuse_end: false }.into();
        let state = Rc::new(RefCell::new(State {
            epoch: 7,
            cleanup: true,
            cleanup_id: 42,
            generation: 3,
            composition: Some(composition.clone()),
            ..Default::default()
        }));
        disconnect(&state);
        disconnect(&state);
        assert!(end_owned_with_flags(&state, TF_ES_ASYNCDONTCARE));
        assert_eq!(state.borrow().cleanup_id, 42);
        let edit: ITfEditSession = EndOwned {
            state: state.clone(),
            context: None,
            composition,
            epoch: 7,
            cleanup_id: 42,
            _lease: Lease::new(),
        }
        .into();
        unsafe {
            edit.DoEditSession(0).unwrap();
        }
        assert!(state.borrow().composition.is_none());
        assert!(!state.borrow().cleanup);
    }
    #[test]
    fn old_cleanup_cannot_terminate_a_new_range() {
        let old: ITfComposition = TestComposition { refuse_end: false }.into();
        let current: ITfComposition = TestComposition { refuse_end: false }.into();
        let state = Rc::new(RefCell::new(State {
            epoch: 8,
            cleanup: true,
            cleanup_id: 43,
            composition: Some(current.clone()),
            ..Default::default()
        }));
        let edit: ITfEditSession = EndOwned {
            state: state.clone(),
            context: None,
            composition: old,
            epoch: 7,
            cleanup_id: 42,
            _lease: Lease::new(),
        }
        .into();
        assert!(unsafe { edit.DoEditSession(0) }.is_err());
        assert_eq!(state.borrow().composition.as_ref(), Some(&current));
        assert_eq!(state.borrow().cleanup_id, 43);
    }
    #[test]
    fn failed_end_keeps_the_real_range_and_cannot_acknowledge_handoff() {
        let composition: ITfComposition = TestComposition { refuse_end: true }.into();
        let state = Rc::new(RefCell::new(State {
            epoch: 7,
            cleanup: true,
            composition: Some(composition.clone()),
            ..Default::default()
        }));
        let edit: ITfEditSession = EndOwned {
            state: state.clone(),
            context: None,
            composition: composition.clone(),
            epoch: 7,
            cleanup_id: 0,
            _lease: Lease::new(),
        }
        .into();
        assert!(unsafe { edit.DoEditSession(0) }.is_err());
        let s = state.borrow();
        assert_eq!(s.composition.as_ref(), Some(&composition));
        assert!(s.key_failure && s.cleanup && !s.version.owned);
        drop(s);
        // A termination callback, rather than a failed API return, releases it.
        release_composition(&state, 7, Some(&composition));
        assert!(state.borrow().composition.is_none());
        assert!(!state.borrow().key_failure);
    }
    #[test]
    fn refused_lock_retains_its_range_but_cannot_poison_a_new_epoch() {
        let composition: ITfComposition = TestComposition { refuse_end: true }.into();
        let state = Rc::new(RefCell::new(State {
            epoch: 2,
            composition: Some(composition.clone()),
            ..Default::default()
        }));
        mark_handoff_failed(&state, 1);
        assert!(!state.borrow().key_failure);
        mark_handoff_failed(&state, 2);
        assert!(state.borrow().key_failure && state.borrow().cleanup);
        assert_eq!(state.borrow().composition.as_ref(), Some(&composition));
        release_composition(&state, 2, Some(&composition));
    }
    #[test]
    fn owned_context_compartments_enforce_native_admission() {
        // An application owns these flags. Test a real, isolated TSF context,
        // rather than trying to overwrite the flags of an existing edit control.
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            let result = (|| -> windows::core::Result<()> {
                let manager: ITfThreadMgr =
                    CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
                let client = manager.Activate()?;
                let result = (|| -> windows::core::Result<()> {
                    let document = manager.CreateDocumentMgr()?;
                    let mut context = None;
                    let mut cookie = 0;
                    document.CreateContext(client, 0, None, &mut context, &mut cookie)?;
                    let context = context.ok_or_else(|| failure(E_FAIL))?;
                    let compartments: ITfCompartmentMgr = context.cast()?;
                    ensure_enabled_context(&context)?;
                    for id in [
                        GUID_COMPARTMENT_KEYBOARD_DISABLED,
                        GUID_COMPARTMENT_EMPTYCONTEXT,
                    ] {
                        let compartment = compartments.GetCompartment(&id)?;
                        compartment.SetValue(client, &1i32.into())?;
                        assert_eq!(
                            ensure_enabled_context(&context).unwrap_err().code(),
                            E_ACCESSDENIED
                        );
                        compartment.SetValue(client, &0i32.into())?;
                        ensure_enabled_context(&context)?;
                        compartments.ClearCompartment(client, &id)?;
                        ensure_enabled_context(&context)?;
                    }
                    Ok(())
                })();
                manager.Deactivate()?;
                result
            })();
            CoUninitialize();
            result.unwrap();
        }
    }
    #[test]
    fn a_late_reply_cannot_clear_a_new_connections_operation() {
        let capability = uuid::Uuid::new_v4().to_string();
        let task = uuid::Uuid::new_v4().to_string();
        let shared = Rc::new(RefCell::new(State {
            connected: true,
            connection: 42,
            capability: Some(capability.clone()),
            pending: Some(Pending {
                operation: 1,
                revision: 0,
                task: task.clone(),
                turn: 1,
                connection: 42,
                capability,
                settlement: None,
            }),
            ..Default::default()
        }));
        let old = Request {
            protocol: crate::wire::PROTOCOL,
            capability: uuid::Uuid::new_v4().to_string(),
            task,
            turn: 1,
            operation: 1,
            revision: 0,
            deadline_tick: 2000,
            action: Action::Begin,
            text: String::new(),
            prefix: None,
        };
        reply(&shared, &old, 41, false, "old", Mutation::NotAttempted);
        assert!(
            shared
                .borrow()
                .pending
                .as_ref()
                .is_some_and(|p| p.connection == 42 && p.operation == 1)
        );
    }
}
