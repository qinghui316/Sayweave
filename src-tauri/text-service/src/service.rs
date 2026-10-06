//! All TSF state stays on the target apartment. IPC never carries COM pointers.
use crate::{
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
    task: String,
    turn: u64,
    connection: u64,
    capability: String,
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
    cleanup: bool,
    version: DraftVersion,
    context: Option<ITfContext>,
    composition: Option<ITfComposition>,
    backup: Option<ITfRangeBackup>,
    expected: String,
    selection: Option<ITfRange>,
    preserved_attributes: Vec<(GUID, i32)>,
    text_cookie: Option<u32>,
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
            if self.state.borrow().active {
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
                s.generation = next_generation();
                s.pipe = Some(pipe);
            }
            if SetTimer(Some(window), 1, 50, None) == 0 {
                self.Deactivate()?;
                return Err(failure(E_FAIL));
            }
            Ok(())
        }
    }
    fn Deactivate(&self) -> windows::core::Result<()> {
        unsafe {
            end_owned(&self.state);
            let old = {
                let mut s = self.state.borrow_mut();
                std::mem::take(&mut *s)
            };
            unwatch(old.context.as_ref(), old.text_cookie);
            let _ = DestroyWindow(HWND(old.window as *mut _));
            drop(old);
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
            shared.borrow_mut().version.applied(&r);
            reply(
                shared,
                &r,
                connection,
                true,
                "cancelledBeforeBegin",
                Mutation::NotAttempted,
            );
            return;
        }
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
    if already_handed_off {
        shared.borrow_mut().version.applied(&r);
        reply(
            shared,
            &r,
            connection,
            true,
            "alreadyHandedOff",
            Mutation::NotAttempted,
        );
        return;
    }
    if let Err((stage, e)) = schedule(shared, &r, connection) {
        reply(
            shared,
            &r,
            connection,
            false,
            &format!("{stage}:{:08x}", e.code().0 as u32),
            Mutation::NotAttempted,
        );
    }
}
fn reply(
    shared: &Shared,
    r: &Request,
    connection: u64,
    applied: bool,
    code: &str,
    mutation: Mutation,
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
    if let Some(pipe) = s.pipe.as_ref() {
        let reply = Reply {
            operation: r.operation,
            revision: r.revision,
            applied,
            code: code.into(),
            mutation,
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
            task: r.task.clone(),
            turn: r.turn,
            connection,
            capability: r.capability.clone(),
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
    let old = {
        let mut s = shared.borrow_mut();
        if s.epoch != epoch || composition.is_some_and(|c| s.composition.as_ref() != Some(c)) {
            return;
        }
        s.generation = next_generation();
        s.version.owned = false;
        s.cleanup = false;
        (
            s.context.take(),
            s.text_cookie.take(),
            s.composition.take(),
            s.backup.take(),
            s.selection.take(),
        )
    };
    unwatch(old.0.as_ref(), old.1);
    drop(old);
}
/// End only this service's existing composition. No text, selection or backup restoration.
/// This also runs after disconnect; it is resource handoff, not a new external write.
fn end_owned(shared: &Shared) {
    let (data, backup) = {
        let mut s = shared.borrow_mut();
        s.version.owned = false;
        s.generation = next_generation();
        let backup = s.backup.take();
        let data = if s.cleanup {
            None
        } else if let (Some(context), Some(composition)) = (&s.context, &s.composition) {
            Some((context.clone(), composition.clone(), s.client, s.epoch))
        } else {
            None
        };
        if data.is_some() {
            s.cleanup = true;
        }
        (data, backup)
    };
    drop(backup);
    if let Some((context, composition, client, epoch)) = data {
        unsafe {
            let edit: ITfEditSession = EndOwned {
                state: shared.clone(),
                composition,
                epoch,
                _lease: Lease::new(),
            }
            .into();
            if context
                .RequestEditSession(client, &edit, TF_ES_ASYNCDONTCARE | TF_ES_READWRITE)
                .and_then(|h| h.ok())
                .is_err()
            {
                release_composition(shared, epoch, None);
            }
        }
    }
}
#[implement(ITfEditSession)]
struct EndOwned {
    state: Shared,
    composition: ITfComposition,
    epoch: u64,
    _lease: Lease,
}
impl ITfEditSession_Impl for EndOwned_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        let result = unsafe { self.composition.EndComposition(ec) };
        release_composition(&self.state, self.epoch, Some(&self.composition));
        result
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
        _: u32,
        composition: Ref<ITfComposition>,
    ) -> windows::core::Result<()> {
        #[cfg(feature = "acceptance")]
        eprintln!(
            "composition terminated: epoch={} generation={} owned={}",
            self.epoch,
            self.state.borrow().generation,
            self.state.borrow().version.owned
        );
        release_composition(&self.state, self.epoch, composition.as_ref());
        Ok(())
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
impl ITfTextEditSink_Impl for TextSink_Impl {
    fn OnEndEdit(
        &self,
        context: Ref<ITfContext>,
        ec: u32,
        record: Ref<ITfEditRecord>,
    ) -> windows::core::Result<()> {
        let (client, generation, composition, expected, expected_selection) = {
            let s = self.state.borrow();
            if s.epoch != self.epoch || !s.version.owned {
                return Ok(());
            }
            (
                s.client,
                s.generation,
                s.composition.clone(),
                s.expected.clone(),
                s.selection.clone(),
            )
        };
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
            let range = composition
                .as_ref()
                .ok_or_else(|| failure(TF_E_DISCONNECTED))?
                .GetRange()?;
            let selected = selection(context, ec)?;
            let expected_selection =
                expected_selection.ok_or_else(|| failure(TF_E_DISCONNECTED))?;
            if record.GetSelectionStatus()?.as_bool()
                || changed_text != 0
                || range_text(&range, ec)? != expected
                || selected.CompareStart(ec, &expected_selection, TF_ANCHOR_START)? != 0
                || selected.CompareEnd(ec, &expected_selection, TF_ANCHOR_END)? != 0
            {
                return Ok(true);
            }
            let attributes = changed_attributes(context, record, &range, ec)?;
            if attributes.is_empty() {
                return Ok(false);
            }
            let backup = {
                let mut s = self.state.borrow_mut();
                if s.epoch != self.epoch || s.generation != generation || !s.version.owned {
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
            s.epoch == self.epoch
                && s.generation == generation
                && s.version.owned
                && s.composition == composition
        };
        if current && !matches!(&result, Ok(false)) {
            end_owned(&self.state);
        }
        result?;
        Ok(())
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
            ),
            Err(e) => {
                complete(
                    &self.state,
                    &self.request,
                    self.connection,
                    false,
                    &format!("edit:{phase}:{:08x}", e.code().0 as u32),
                    mutation,
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
) {
    let current = shared.borrow().pending.as_ref().is_some_and(|p| {
        p.operation == r.operation
            && p.task == r.task
            && p.turn == r.turn
            && p.connection == connection
            && p.capability == r.capability
    });
    if current {
        reply(shared, r, connection, applied, code, mutation);
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
fn apply(
    shared: &Shared,
    context: &ITfContext,
    r: &Request,
    generation: u64,
    ec: u32,
    mutation: &mut Mutation,
    phase: &mut &'static str,
) -> windows::core::Result<()> {
    unsafe {
        if r.action == Action::Begin {
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
            s.backup = Some(backup);
            s.context = Some(context.clone());
            s.expected = expected;
            s.selection = Some(original_selection);
            s.text_cookie = Some(cookie);
            s.version.applied(r);
            return Ok(());
        }
        *phase = "ownership";
        let (composition, expected, old_selection, epoch) = {
            let s = shared.borrow();
            (
                s.composition
                    .as_ref()
                    .ok_or_else(|| failure(TF_E_DISCONNECTED))?
                    .clone(),
                s.expected.clone(),
                s.selection
                    .clone()
                    .ok_or_else(|| failure(TF_E_DISCONNECTED))?,
                s.epoch,
            )
        };
        *phase = "range";
        let range = composition.GetRange()?;
        *phase = "selection";
        let current_selection = selection(context, ec)?;
        *phase = "position";
        if password(context, &range, ec)?
            || range_text(&range, ec)? != expected
            || current_selection.CompareStart(ec, &old_selection, TF_ANCHOR_START)? != 0
            || current_selection.CompareEnd(ec, &old_selection, TF_ANCHOR_END)? != 0
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        match r.action {
            Action::Update => {
                *phase = "updateSafety";
                if password(context, &range, ec)? {
                    return Err(failure(E_ACCESSDENIED));
                }
                *phase = "updateLease";
                lease(shared, r, generation, context)?;
                *mutation = Mutation::Uncertain;
                *phase = "updateText";
                range.SetText(
                    ec,
                    TF_ST_CORRECTION,
                    &r.text.encode_utf16().collect::<Vec<_>>(),
                )?;
                *mutation = Mutation::Changed;
                *phase = "updateAttributes";
                let attributes = { shared.borrow().preserved_attributes.clone() };
                for (id, value) in attributes {
                    lease(shared, r, generation, context)?;
                    let property = context.GetProperty(&id)?;
                    edit_identity(shared, r, generation, epoch)?;
                    property.SetValue(
                        ec,
                        &range,
                        &windows::Win32::System::Variant::VARIANT::from(value),
                    )?;
                    edit_identity(shared, r, generation, epoch)?;
                    let actual = property.GetValue(ec, &range)?;
                    edit_identity(shared, r, generation, epoch)?;
                    if actual.vt() != windows::Win32::System::Variant::VT_I4
                        || i32::try_from(&actual)? != value
                    {
                        return Err(failure(TF_E_DISCONNECTED));
                    }
                }
                *phase = "updateReadback";
                let actual = range_text(&range, ec)?;
                // RichEdit and browser text stores can use different newline representations.
                // Preserve the provider's range text; permit only equivalent line breaks.
                if actual.replace("\r\n", "\n").replace('\r', "\n")
                    != r.text.replace("\r\n", "\n").replace('\r', "\n")
                {
                    return Err(failure(TF_E_DISCONNECTED));
                }
                {
                    let mut s = shared.borrow_mut();
                    if s.generation != generation || s.epoch != epoch || !s.version.owned {
                        return Err(failure(TF_E_DISCONNECTED));
                    }
                    s.expected = actual;
                    s.version.applied(r);
                    s.selection = None;
                }
                *phase = "updateCaret";
                let caret = range.Clone()?;
                caret.Collapse(ec, TF_ANCHOR_END)?;
                {
                    let s = shared.borrow();
                    if !s.active
                        || s.generation != generation
                        || s.epoch != epoch
                        || !s.version.owned
                    {
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
            }
            Action::Abort => {
                *phase = "abort";
                let backup = { shared.borrow().backup.clone() };
                let Some(backup) = backup else {
                    // Finish even when property changes made rollback unsafe.
                    composition.EndComposition(ec)?;
                    release_composition(shared, epoch, Some(&composition));
                    return Err(failure(TF_E_DISCONNECTED));
                };
                *mutation = Mutation::Uncertain;
                lease(shared, r, generation, context)?;
                backup.Restore(ec, &range)?;
                *mutation = Mutation::Changed;
                if shared.borrow().generation == generation {
                    shared.borrow_mut().version.applied(r);
                }
                composition.EndComposition(ec)?;
                release_composition(shared, epoch, Some(&composition));
            }
            Action::Commit | Action::Handoff => {
                *phase = "commit";
                lease(shared, r, generation, context)?;
                shared.borrow_mut().version.applied(r);
                composition.EndComposition(ec)?;
                release_composition(shared, epoch, Some(&composition));
            }
            Action::Begin | Action::Inspect => unreachable!(),
        }
        Ok(())
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
                task: task.clone(),
                turn: 1,
                connection: 42,
                capability,
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
