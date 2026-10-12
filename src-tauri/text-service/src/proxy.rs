//! Stable COM service and apartment dispatcher; engines never own this window.
#[cfg(feature = "acceptance")]
use crate::wire::Request;
use crate::{
    engine_abi::*,
    registration,
    transport::{Identity, Pipe, identity},
    wire::MAX_FRAME,
};
use std::{
    cell::{Cell, RefCell},
    ffi::c_void,
    fs::File,
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    rc::Rc,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};
use windows::{
    Win32::{
        Foundation::*,
        System::{Com::*, LibraryLoader::*},
        UI::{TextServices::*, WindowsAndMessaging::*},
    },
    core::{GUID, HRESULT, IUnknown, Interface, Ref, implement, s, w},
};

static OBJECTS: AtomicUsize = AtomicUsize::new(0);
static LOCKS: AtomicUsize = AtomicUsize::new(0);
#[path = "proxy_transport.rs"]
mod gateway;
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
fn error(code: i32) -> windows::core::Error {
    windows::core::Error::from_hresult(HRESULT(code))
}
fn checked(code: i32) -> windows::core::Result<()> {
    HRESULT(code).ok()
}
fn boundary(action: impl FnOnce() -> windows::core::Result<()>) -> HRESULT {
    let _frame = Lease::new();
    HRESULT(
        catch_unwind(AssertUnwindSafe(action)).map_or(E_UNEXPECTED.0, |r| {
            r.map_or_else(|e| e.code().0, |_| S_OK.0)
        }),
    )
}
type ModuleMap = std::collections::HashMap<String, std::sync::Weak<Module>>;
static MODULES: OnceLock<Mutex<ModuleMap>> = OnceLock::new();
static RETIRED_MODULES: Mutex<Vec<RetiredModule>> = Mutex::new(Vec::new());
struct Module {
    base: usize,
    file: Option<File>,
    can_unload: unsafe extern "system" fn() -> HRESULT,
}
struct RetiredModule {
    base: usize,
    _file: File,
    can_unload: unsafe extern "system" fn() -> HRESULT,
}
impl Module {
    fn handle(&self) -> HMODULE {
        HMODULE(self.base as *mut c_void)
    }
}
impl Drop for Module {
    fn drop(&mut self) {
        unsafe {
            if (self.can_unload)() == S_OK && FreeLibrary(self.handle()).is_ok() {
                return;
            }
        }
        if let Some(file) = self.file.take() {
            RETIRED_MODULES
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(RetiredModule {
                    base: self.base,
                    _file: file,
                    can_unload: self.can_unload,
                });
        }
    }
}
fn collect_modules() {
    let old = std::mem::take(&mut *RETIRED_MODULES.lock().unwrap_or_else(|e| e.into_inner()));
    let retained = old
        .into_iter()
        .filter(|module| unsafe {
            (module.can_unload)() != S_OK
                || FreeLibrary(HMODULE(module.base as *mut c_void)).is_err()
        })
        .collect::<Vec<_>>();
    RETIRED_MODULES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .extend(retained);
}
fn acquire_module(path: &Path, file: File) -> windows::core::Result<Arc<Module>> {
    use std::os::windows::ffi::OsStrExt;
    let key = path.to_string_lossy().to_lowercase();
    let cache = MODULES.get_or_init(|| Mutex::new(ModuleMap::new()));
    if let Some(module) = cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .and_then(std::sync::Weak::upgrade)
    {
        return Ok(module);
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    unsafe {
        let handle = LoadLibraryExW(
            windows::core::PCWSTR(wide.as_ptr()),
            None,
            LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
        )?;
        let Some(can) = GetProcAddress(handle, s!("DllCanUnloadNow")) else {
            FreeLibrary(handle)?;
            return Err(error(E_NOINTERFACE.0));
        };
        let candidate = Arc::new(Module {
            base: handle.0 as usize,
            file: Some(file),
            can_unload: std::mem::transmute::<
                unsafe extern "system" fn() -> isize,
                unsafe extern "system" fn() -> HRESULT,
            >(can),
        });
        let existing = {
            let mut modules = cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(existing) = modules.get(&key).and_then(std::sync::Weak::upgrade) {
                Some(existing)
            } else {
                modules.insert(key, Arc::downgrade(&candidate));
                None
            }
        };
        if let Some(existing) = existing {
            // Release just this redundant loader reference. Existing owner keeps
            // the module mapped; no engine was created by this candidate.
            FreeLibrary(handle)?;
            let mut candidate = Arc::try_unwrap(candidate).map_err(|_| error(E_UNEXPECTED.0))?;
            candidate.file.take();
            std::mem::forget(candidate);
            Ok(existing)
        } else {
            Ok(candidate)
        }
    }
}
struct Core {
    module: Arc<Module>,
    api: EngineApiV1,
}
impl Core {
    fn status(&self) -> windows::core::Result<EngineStatusV1> {
        let mut state = EngineStatusV1::default();
        unsafe {
            checked((self.api.status)(self.api.handle, &mut state))?;
        }
        Ok(state)
    }
    fn load(
        path: &Path,
        hash: &[u8; 32],
        generation: u64,
        host: &HostGuardV1,
    ) -> windows::core::Result<Self> {
        use sha2::{Digest, Sha256};
        use std::{
            io::Read,
            os::windows::fs::{MetadataExt, OpenOptionsExt},
        };
        if !path.is_absolute() {
            return Err(error(E_ACCESSDENIED.0));
        }
        for ancestor in path.ancestors() {
            let metadata =
                std::fs::symlink_metadata(ancestor).map_err(|_| error(E_ACCESSDENIED.0))?;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(error(E_ACCESSDENIED.0));
            }
        }
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(path)
            .map_err(|_| error(E_ACCESSDENIED.0))?;
        let metadata = file.metadata().map_err(|_| error(E_FAIL.0))?;
        if !metadata.is_file() || metadata.len() > 32 * 1024 * 1024 {
            return Err(error(E_INVALIDARG.0));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).map_err(|_| error(E_FAIL.0))?;
        if <[u8; 32]>::from(Sha256::digest(&bytes)) != *hash {
            return Err(error(E_ACCESSDENIED.0));
        }
        let module = acquire_module(path, file)?;
        unsafe {
            let Some(entry) = GetProcAddress(module.handle(), s!("CreateEngineV1")) else {
                return Err(error(E_NOINTERFACE.0));
            };
            let create: CreateEngineV1 = std::mem::transmute(entry);
            let mut api = std::mem::MaybeUninit::uninit();
            checked(create(
                generation,
                api.as_mut_ptr(),
                std::mem::size_of::<EngineApiV1>(),
            ))?;
            let api = api.assume_init();
            // A malformed trusted ABI is not safe to call again. Retain its module
            // rather than freeing code that may have created unknown objects.
            if api.abi != ENGINE_ABI
                || api.size as usize != std::mem::size_of::<EngineApiV1>()
                || api.handle.is_null()
            {
                std::mem::forget(module);
                return Err(error(E_NOINTERFACE.0));
            }
            let core = Self { module, api };
            checked((core.api.bind_host)(core.api.handle, host))?;
            let identity = core.status()?;
            if identity.abi != ENGINE_ABI
                || identity.generation != generation
                || identity.module_base != core.module.base as u64
                || identity.module_sha256 != *hash
            {
                let _ = (core.api.deactivate)(core.api.handle);
                if (core.api.destroy)(core.api.handle) != S_OK.0 {
                    std::mem::forget(core);
                }
                return Err(error(E_ACCESSDENIED.0));
            }
            Ok(core)
        }
    }
    fn dispose(&mut self) -> windows::core::Result<bool> {
        if self.api.handle.is_null() {
            return Ok(true);
        }
        let status = self.status()?;
        if status.active != 0 || !status.idle() || status.leases != 0 {
            return Ok(false);
        }
        unsafe {
            checked((self.api.destroy)(self.api.handle))?;
            self.api.handle = std::ptr::null_mut();
        }
        Ok(true)
    }
}
// No automatic FreeLibrary: failed or outstanding retirement retains executable code.
struct RetainedReply {
    key: crate::lifecycle::ReceiptKey,
    owner: Option<(Identity, String)>,
    bytes: Vec<u8>,
}
struct State {
    instance: [u8; 16],
    generation: u64,
    serial: u64,
    calls: Rc<Cell<u32>>,
    manager: Option<ITfThreadMgr>,
    client: u32,
    current: Option<Core>,
    retired: Vec<Core>,
    active: bool,
    window: HWND,
    switching: bool,
    epoch: u64,
    shutdown: bool,
    failed: bool,
    retained_replies: Vec<RetainedReply>,
    reply_owner: Option<(Identity, String)>,
    #[cfg(feature = "acceptance")]
    activation_fault: u32,
    acknowledged: Vec<(crate::lifecycle::ReceiptKey, Option<(Identity, String)>)>,
    pipe: Option<Pipe>,
    peer: Option<Identity>,
    capability: Option<String>,
    fixture_connection: bool,
    connected: bool,
    proxy_identity: Option<crate::wire::ModuleIdentity>,
}
type Shared = Rc<RefCell<State>>;
struct Call(Rc<Cell<u32>>, Lease);
impl Drop for Call {
    fn drop(&mut self) {
        self.0.set(self.0.get() - 1);
    }
}
fn enter(shared: &Shared) -> windows::core::Result<(Call, EngineApiV1)> {
    let state = shared.borrow();
    if state.switching || !state.active || state.shutdown || state.failed || state.calls.get() != 0
    {
        return Err(error(BUSY));
    }
    let api = state
        .current
        .as_ref()
        .ok_or_else(|| error(TF_E_DISCONNECTED.0))?
        .api;
    state.calls.set(1);
    Ok((Call(state.calls.clone(), Lease::new()), api))
}
fn lifecycle(shared: &Shared) -> windows::core::Result<Call> {
    let state = shared.borrow();
    if state.calls.get() != 0 || state.switching {
        return Err(error(BUSY));
    }
    state.calls.set(1);
    Ok(Call(state.calls.clone(), Lease::new()))
}
// Only window IDs cross threads; STA-owned COM/Rc state never enters this map.
static ACTIVE_WINDOWS: OnceLock<Mutex<std::collections::HashMap<u32, usize>>> = OnceLock::new();
#[cfg(feature = "acceptance")]
fn current() -> windows::core::Result<Shared> {
    let thread = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
    let window = ACTIVE_WINDOWS
        .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&thread)
        .copied()
        .ok_or_else(|| error(TF_E_DISCONNECTED.0))?;
    unsafe {
        let window = HWND(window as *mut c_void);
        let mut process = 0;
        if GetWindowThreadProcessId(window, Some(&mut process)) != thread
            || process != std::process::id()
        {
            return Err(error(E_ACCESSDENIED.0));
        }
        let bridge = GetWindowLongPtrW(window, GWLP_USERDATA) as *const Bridge;
        let bridge = bridge.as_ref().ok_or_else(|| error(TF_E_DISCONNECTED.0))?;
        Ok(bridge.state.clone())
    }
}
fn own_module() -> windows::core::Result<HINSTANCE> {
    let mut module = HMODULE::default();
    unsafe {
        GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            windows::core::PCWSTR(procedure as *const () as *const u16),
            &mut module,
        )?;
    }
    Ok(HINSTANCE(module.0))
}
fn collect(shared: &Shared) {
    let Ok(_operation) = lifecycle(shared) else {
        return;
    };
    collect_inner(shared);
}
fn collect_inner(shared: &Shared) {
    let mut retired = { std::mem::take(&mut shared.borrow_mut().retired) };
    let mut retained = Vec::new();
    for mut core in retired.drain(..) {
        if shared.borrow().shutdown {
            // Move receipts into proxy ownership before retiring the engine.
            // They remain generation-bound and readable while shutdown is pending.
            loop {
                if shared.borrow().retained_replies.len() >= 16 {
                    break;
                }
                let mut data = vec![0; MAX_FRAME * 4];
                let mut length = 0;
                let result = unsafe {
                    (core.api.read)(core.api.handle, data.as_mut_ptr(), data.len(), &mut length)
                };
                if result != S_OK.0 || length == 0 {
                    break;
                }
                data.truncate(length);
                // Malformed outcomes keep the core pending rather than pretending success.
                let Some(key) = crate::lifecycle::ReceiptKey::from_frame(&data) else {
                    shared.borrow_mut().failed = true;
                    break;
                };
                let owner = shared.borrow().reply_owner.clone();
                shared.borrow_mut().retained_replies.push(RetainedReply {
                    key,
                    owner,
                    bytes: data,
                });
            }
            unsafe {
                let _ = (core.api.retire)(core.api.handle);
            }
        } else if shared.borrow().failed && core.status().is_ok_and(|s| s.active != 0 && s.idle()) {
            unsafe {
                let _ = (core.api.deactivate)(core.api.handle);
            }
        }
        let disposed = core.dispose().unwrap_or(false);
        if !disposed {
            retained.push(core);
        }
    }
    shared.borrow_mut().retired.extend(retained);
    collect_modules();
}
fn session_matches(shared: &Shared, epoch: u64) -> bool {
    let state = shared.borrow();
    state.active && !state.shutdown && state.epoch == epoch
}
fn recover(shared: &Shared) -> windows::core::Result<()> {
    collect(shared);
    let _operation = lifecycle(shared)?;
    let epoch = shared.borrow().epoch;
    let (manager, client, api) = {
        let state = shared.borrow();
        if state.shutdown || !state.active {
            return Err(error(BUSY));
        }
        for core in &state.retired {
            if core.status()?.active != 0 {
                return Err(error(BUSY));
            }
        }
        (
            state.manager.clone().ok_or_else(|| error(E_UNEXPECTED.0))?,
            state.client,
            state.current.as_ref().map(|c| c.api),
        )
    };
    if let Some(api) = api {
        let mut status = EngineStatusV1::default();
        unsafe {
            checked((api.status)(api.handle, &mut status))?;
            if status.active == 0 {
                checked((api.activate)(api.handle, manager.as_raw(), client))?;
            }
        }
    }
    if !session_matches(shared, epoch) {
        return Err(error(TF_E_DISCONNECTED.0));
    }
    shared.borrow_mut().failed = false;
    Ok(())
}
unsafe extern "system" fn validate_channel(
    context: *mut c_void,
    generation: u64,
    pid: u32,
    capability: *const u8,
    length: usize,
) -> i32 {
    boundary(|| {
        if context.is_null() || capability.is_null() || length > 64 {
            return Err(error(E_INVALIDARG.0));
        }
        let state = unsafe { &*context.cast::<RefCell<State>>() }.borrow();
        let cap = std::str::from_utf8(unsafe { std::slice::from_raw_parts(capability, length) })
            .map_err(|_| error(E_INVALIDARG.0))?;
        if !state.active
            || state.shutdown
            || state.failed
            || state.switching
            || state.generation != generation
            || state.capability.as_deref() != Some(cap)
            || state
                .peer
                .is_none_or(|peer| peer.pid != pid || identity(pid).ok() != Some(peer))
            || registration::read_host() != Some((pid, cap.into()))
        {
            return Err(error(E_ACCESSDENIED.0));
        }
        if !(cfg!(feature = "acceptance") && state.fixture_connection && pid == std::process::id())
            && !state
                .pipe
                .as_ref()
                .is_some_and(|pipe| pipe.alive() && pipe.client_pid().ok() == Some(pid))
        {
            return Err(error(TF_E_DISCONNECTED.0));
        }
        Ok(())
    })
    .0
}
fn prepare(shared: &Shared, path: &Path, hash: &[u8; 32]) -> windows::core::Result<()> {
    if shared.borrow().failed {
        recover(shared)?;
    }
    let (manager, client, generation, epoch, old) = {
        let mut state = shared.borrow_mut();
        if !state.active || state.shutdown || state.switching || state.calls.get() != 0 {
            return Err(error(BUSY));
        }
        if !state.retained_replies.is_empty() {
            return Err(error(BUSY));
        }
        if let Some(core) = &state.current {
            let status = core.status()?;
            if status.module_sha256 == *hash && status.active == 1 {
                return Ok(());
            }
            if !status.idle() {
                return Err(error(BUSY));
            }
        }
        let manager = state.manager.clone().ok_or_else(|| error(E_UNEXPECTED.0))?;
        let generation = state
            .serial
            .checked_add(1)
            .ok_or_else(|| error(E_UNEXPECTED.0))?;
        state.switching = true;
        state.serial = generation;
        (
            manager,
            state.client,
            generation,
            state.epoch,
            state.current.take(),
        )
    };
    let _operation = {
        let state = shared.borrow();
        state.calls.set(1);
        Call(state.calls.clone(), Lease::new())
    };
    let mut candidate = None;
    let mut old_stopped = false;
    let result = (|| -> windows::core::Result<()> {
        let host = HostGuardV1 {
            context: Rc::as_ptr(shared).cast_mut().cast(),
            proxy_instance: shared.borrow().instance,
            engine_generation: generation,
            validate: validate_channel,
        };
        candidate = Some(Core::load(path, hash, generation, &host)?);
        if !session_matches(shared, epoch) {
            return Err(error(TF_E_DISCONNECTED.0));
        }
        if let Some(old) = &old {
            unsafe {
                checked((old.api.deactivate)(old.api.handle))?;
            }
            old_stopped = true;
        }
        if !session_matches(shared, epoch) {
            return Err(error(TF_E_DISCONNECTED.0));
        }
        let api = candidate.as_ref().ok_or_else(|| error(E_UNEXPECTED.0))?.api;
        unsafe {
            checked((api.activate)(api.handle, manager.as_raw(), client))?;
        }
        #[cfg(feature = "acceptance")]
        let activation_fault = std::mem::take(&mut shared.borrow_mut().activation_fault);
        #[cfg(feature = "acceptance")]
        match activation_fault {
            1 => return Err(error(E_FAIL.0)),
            2 => {
                let mut state = shared.borrow_mut();
                state.epoch += 1;
                state.shutdown = true;
                state.active = false;
            }
            _ => (),
        }
        if !session_matches(shared, epoch) {
            return Err(error(TF_E_DISCONNECTED.0));
        }
        if candidate
            .as_ref()
            .ok_or_else(|| error(E_UNEXPECTED.0))?
            .status()?
            .active
            != 1
        {
            return Err(error(E_UNEXPECTED.0));
        }
        Ok(())
    })();
    if let Err(cause) = result {
        let candidate_stopped = if let Some(candidate) = candidate.take() {
            let stopped = unsafe { (candidate.api.deactivate)(candidate.api.handle) == S_OK.0 };
            shared.borrow_mut().retired.push(candidate);
            stopped
        } else {
            true
        };
        let may_restore = session_matches(shared, epoch) && candidate_stopped;
        let restored = if may_restore && old_stopped {
            old.as_ref().is_none_or(|old| unsafe {
                (old.api.activate)(old.api.handle, manager.as_raw(), client) == S_OK.0
            })
        } else {
            !old_stopped
        };
        let mut state = shared.borrow_mut();
        state.switching = false;
        if state.shutdown {
            if let Some(old) = old {
                state.retired.push(old);
            }
        } else {
            state.current = old;
            state.failed = !candidate_stopped || !restored;
        }
        drop(state);
        collect(shared);
        let _ = finish_shutdown(shared);
        return Err(cause);
    }
    let mut state = shared.borrow_mut();
    // No COM calls between the last epoch check and publishing this instance.
    state.current = candidate;
    state.generation = generation;
    state.switching = false;
    state.failed = false;
    if let Some(old) = old {
        state.retired.push(old);
    }
    drop(state);
    collect(shared);
    Ok(())
}
fn finish_shutdown(shared: &Shared) -> windows::core::Result<()> {
    if !shared.borrow().shutdown {
        return Ok(());
    }
    let _operation = lifecycle(shared)?;
    let core = shared.borrow_mut().current.take();
    if let Some(core) = core {
        unsafe {
            let _ = (core.api.retire)(core.api.handle);
        }
        shared.borrow_mut().retired.push(core);
    }
    collect_inner(shared);
    if !shared.borrow().retired.is_empty() || !shared.borrow().retained_replies.is_empty() {
        return Err(error(BUSY));
    }
    let window = {
        let mut state = shared.borrow_mut();
        state.manager = None;
        state.active = false;
        std::mem::take(&mut state.window)
    };
    if !window.is_invalid() {
        unsafe {
            DestroyWindow(window)?;
            if let Err(failure) = UnregisterClassW(w!("SayweaveTsfProxy"), Some(own_module()?))
                && failure.code() != HRESULT::from_win32(ERROR_CLASS_HAS_WINDOWS.0)
            {
                return Err(failure);
            }
        }
    }
    Ok(())
}

struct Bridge {
    state: Shared,
    _lease: Lease,
}
#[implement(ITfTextInputProcessor)]
struct Service {
    state: Shared,
    _lease: Lease,
}
impl ITfTextInputProcessor_Impl for Service_Impl {
    fn Activate(&self, manager: Ref<ITfThreadMgr>, client: u32) -> windows::core::Result<()> {
        if self.state.borrow().active {
            return Err(error(E_UNEXPECTED.0));
        }
        let manager = manager.as_ref().ok_or_else(|| error(E_POINTER.0))?.clone();
        unsafe {
            let module = own_module()?;
            let pipe = Pipe::server()?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: module,
                lpszClassName: w!("SayweaveTsfProxy"),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(windows::core::Error::from_thread());
            }
            let window = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("SayweaveTsfProxy"),
                w!(""),
                WINDOW_STYLE(0),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(module),
                None,
            )?;
            let bridge = Box::new(Bridge {
                state: self.state.clone(),
                _lease: Lease::new(),
            });
            SetWindowLongPtrW(window, GWLP_USERDATA, Box::into_raw(bridge) as isize);
            let mut state = self.state.borrow_mut();
            state.epoch = state
                .epoch
                .checked_add(1)
                .ok_or_else(|| error(E_UNEXPECTED.0))?;
            state.shutdown = false;
            state.failed = false;
            state.manager = Some(manager);
            state.pipe = Some(pipe);
            state.connected = false;
            state.peer = None;
            state.capability = None;
            state.fixture_connection = false;
            state.proxy_identity = Some(gateway::own_identity()?);
            state.client = client;
            state.active = true;
            state.window = window;
            drop(state);
            ACTIVE_WINDOWS
                .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(
                    windows::Win32::System::Threading::GetCurrentThreadId(),
                    window.0 as usize,
                );
            if SetTimer(Some(window), 1, 25, None) == 0 {
                self.Deactivate()?;
                return Err(error(E_FAIL.0));
            }
        }
        Ok(())
    }
    fn Deactivate(&self) -> windows::core::Result<()> {
        {
            let mut state = self.state.borrow_mut();
            if !state.shutdown {
                state.epoch = state
                    .epoch
                    .checked_add(1)
                    .ok_or_else(|| error(E_UNEXPECTED.0))?;
            }
            state.shutdown = true;
            state.active = false;
        }
        finish_shutdown(&self.state)
    }
}
unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let _frame = Lease::new();
    unsafe {
        let bridge = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Bridge;
        if !bridge.is_null() && message == WM_TIMER {
            let state = (*bridge).state.clone();
            if let Ok((_call, api)) = enter(&state) {
                let _ = (api.poll)(api.handle);
            }
            gateway::poll(&state);
            collect(&state);
            let _ = finish_shutdown(&state);
            return LRESULT(0);
        }
        if !bridge.is_null() && message == WM_NCDESTROY {
            let thread = windows::Win32::System::Threading::GetCurrentThreadId();
            let mut active = ACTIVE_WINDOWS
                .get_or_init(|| Mutex::new(std::collections::HashMap::new()))
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if active.get(&thread) == Some(&(window.0 as usize)) {
                active.remove(&thread);
            }
            drop(active);
            let _ = KillTimer(Some(window), 1);
            SetWindowLongPtrW(window, GWLP_USERDATA, 0);
            drop(Box::from_raw(bridge));
        }
        DefWindowProcW(window, message, wparam, lparam)
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
            return Err(error(CLASS_E_NOAGGREGATION.0));
        }
        if iid.is_null() || out.is_null() {
            return Err(error(E_POINTER.0));
        }
        let state = Rc::new(RefCell::new(State {
            instance: *uuid::Uuid::new_v4().as_bytes(),
            generation: 0,
            serial: 0,
            calls: Rc::new(Cell::new(0)),
            manager: None,
            client: 0,
            current: None,
            retired: Vec::new(),
            active: false,
            window: HWND::default(),
            switching: false,
            epoch: 0,
            shutdown: false,
            failed: false,
            retained_replies: Vec::new(),
            reply_owner: None,
            acknowledged: Vec::new(),
            #[cfg(feature = "acceptance")]
            activation_fault: 0,
            pipe: None,
            peer: None,
            capability: None,
            fixture_connection: false,
            connected: false,
            proxy_identity: None,
        }));
        let service: ITfTextInputProcessor = Service {
            state,
            _lease: Lease::new(),
        }
        .into();
        unsafe { service.query(iid, out).ok() }
    }
    fn LockServer(&self, lock: windows::core::BOOL) -> windows::core::Result<()> {
        if lock.as_bool() {
            LOCKS.fetch_add(1, Ordering::AcqRel);
        } else {
            LOCKS
                .try_update(Ordering::AcqRel, Ordering::Acquire, |v| v.checked_sub(1))
                .map_err(|_| error(E_UNEXPECTED.0))?;
        }
        Ok(())
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    out: *mut *mut c_void,
) -> HRESULT {
    boundary(|| {
        if clsid.is_null() || iid.is_null() || out.is_null() {
            return Err(error(E_POINTER.0));
        }
        unsafe {
            *out = std::ptr::null_mut();
            if *clsid != registration::CLASS {
                return Err(error(CLASS_E_CLASSNOTAVAILABLE.0));
            }
            let factory: IClassFactory = Factory {
                _lease: Lease::new(),
            }
            .into();
            factory.query(iid, out).ok()
        }
    })
}
#[unsafe(no_mangle)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    if OBJECTS.load(Ordering::Acquire) == 0
        && LOCKS.load(Ordering::Acquire) == 0
        && RETIRED_MODULES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty()
    {
        S_OK
    } else {
        S_FALSE
    }
}

#[cfg(feature = "acceptance")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SayweaveProxyPrepareFixture(
    path: *const u16,
    length: usize,
    hash: *const u8,
) -> HRESULT {
    boundary(|| {
        if path.is_null() || hash.is_null() || length == 0 || length > 32767 {
            return Err(error(E_INVALIDARG.0));
        }
        let path = String::from_utf16(unsafe { std::slice::from_raw_parts(path, length) })
            .map_err(|_| error(E_INVALIDARG.0))?;
        let hash: &[u8; 32] = unsafe { &*hash.cast() };
        prepare(&current()?, Path::new(&path), hash)
    })
}
#[cfg(feature = "acceptance")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SayweaveProxyDispatchFixture(
    data: *const u8,
    length: usize,
    generation: u64,
) -> HRESULT {
    boundary(|| {
        let shared = current()?;
        if data.is_null() || length == 0 || length > MAX_FRAME {
            return Err(error(E_INVALIDARG.0));
        }
        let request: Request =
            serde_json::from_slice(unsafe { std::slice::from_raw_parts(data, length) })
                .map_err(|_| error(E_INVALIDARG.0))?;
        let (host, capability) =
            registration::read_host().ok_or_else(|| error(E_ACCESSDENIED.0))?;
        if capability != request.capability || host != std::process::id() {
            return Err(error(E_ACCESSDENIED.0));
        }
        {
            let mut state = shared.borrow_mut();
            state.peer = Some(identity(host)?);
            state.capability = Some(capability);
            state.fixture_connection = true;
        }
        if shared.borrow().generation != generation {
            return Err(error(E_ACCESSDENIED.0));
        }
        if request.engine.proxy_instance
            != uuid::Uuid::from_bytes(shared.borrow().instance).to_string()
            || request.engine.engine_generation != generation
        {
            return Err(error(E_ACCESSDENIED.0));
        }
        let (_call, api) = enter(&shared)?;
        unsafe { checked((api.dispatch)(api.handle, data, length)) }
    })
}
#[cfg(feature = "acceptance")]
#[unsafe(no_mangle)]
pub extern "system" fn SayweaveProxyActivationFaultFixture(mode: u32) -> HRESULT {
    boundary(|| {
        if mode > 2 {
            return Err(error(E_INVALIDARG.0));
        }
        current()?.borrow_mut().activation_fault = mode;
        Ok(())
    })
}
#[cfg(feature = "acceptance")]
#[unsafe(no_mangle)]
pub extern "system" fn SayweaveProxyUseTransportFixture() -> HRESULT {
    boundary(|| {
        let shared = current()?;
        let mut state = shared.borrow_mut();
        state.fixture_connection = false;
        state.connected = false;
        state.peer = None;
        state.capability = None;
        Ok(())
    })
}
#[cfg(feature = "acceptance")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SayweaveProxyReadFixture(
    buffer: *mut u8,
    capacity: usize,
    length: *mut usize,
) -> HRESULT {
    boundary(|| {
        if capacity > MAX_FRAME {
            return Err(error(E_INVALIDARG.0));
        }
        let shared = current()?;
        let (_call, api) = enter(&shared)?;
        unsafe { checked((api.read)(api.handle, buffer, capacity, length)) }
    })
}
#[cfg(feature = "acceptance")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SayweaveProxyInspectFixture(output: *mut ProxyStatusV1) -> HRESULT {
    boundary(|| {
        if output.is_null() {
            return Err(error(E_POINTER.0));
        }
        let shared = current()?;
        let state = shared.borrow();
        let engine = state
            .current
            .as_ref()
            .ok_or_else(|| error(TF_E_DISCONNECTED.0))?
            .status()?;
        unsafe {
            *output = ProxyStatusV1 {
                abi: ENGINE_ABI,
                ready: u32::from(state.active && !state.shutdown && !state.failed),
                instance: state.instance,
                generation: state.generation,
                retired: state.retired.len() as u32,
                engine,
            };
        }
        Ok(())
    })
}
#[cfg(feature = "acceptance")]
#[unsafe(no_mangle)]
pub unsafe extern "system" fn SayweaveProxyReadRetiredFixture(
    buffer: *mut u8,
    capacity: usize,
    length: *mut usize,
    generation: u64,
) -> HRESULT {
    boundary(|| {
        if buffer.is_null() || length.is_null() || capacity > MAX_FRAME * 4 {
            return Err(error(E_INVALIDARG.0));
        }
        let shared = current()?;
        {
            let mut state = shared.borrow_mut();
            let Some(index) = state
                .retained_replies
                .iter()
                .position(|r| r.key.engine.engine_generation == generation)
            else {
                unsafe {
                    *length = 0;
                }
                return Ok(());
            };
            let reply = &state.retained_replies[index].bytes;
            unsafe {
                *length = reply.len();
            }
            if reply.len() > capacity {
                return Err(error(HRESULT::from_win32(ERROR_INSUFFICIENT_BUFFER.0).0));
            }
            unsafe {
                std::ptr::copy_nonoverlapping(reply.as_ptr(), buffer, reply.len());
            }
            state.retained_replies.remove(index);
        }
        let _ = finish_shutdown(&shared);
        Ok(())
    })
}
#[cfg(feature = "acceptance")]
#[unsafe(no_mangle)]
pub extern "system" fn SayweaveProxyPinFixture() -> HRESULT {
    boundary(|| {
        let shared = current()?;
        let state = shared.borrow();
        let core = state
            .current
            .as_ref()
            .ok_or_else(|| error(TF_E_DISCONNECTED.0))?;
        unsafe { checked((core.api.retain)(core.api.handle)) }
    })
}
#[cfg(feature = "acceptance")]
#[unsafe(no_mangle)]
pub extern "system" fn SayweaveProxyReleaseFixture(generation: u64) -> HRESULT {
    boundary(|| {
        let shared = current()?;
        {
            let state = shared.borrow();
            let core = state
                .retired
                .iter()
                .find(|c| c.status().is_ok_and(|s| s.generation == generation))
                .ok_or_else(|| error(E_INVALIDARG.0))?;
            unsafe {
                checked((core.api.release)(core.api.handle))?;
            }
        }
        collect(&shared);
        Ok(())
    })
}
