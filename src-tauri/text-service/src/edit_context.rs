//! Resolve a focused TSF carrier to its existing editable document.
//! A transitory store owns preedit text, not the host's committed block.
use super::*;

#[derive(Clone)]
pub(super) struct Binding {
    pub carrier: ITfDocumentMgr,
    pub carrier_context: ITfContext,
    pub document: ITfDocumentMgr,
    pub context: ITfContext,
    window: HWND,
    root: HWND,
}

impl Binding {
    pub(super) unsafe fn focused(manager: &ITfThreadMgr) -> windows::core::Result<Self> {
        unsafe { Self::from_document(&manager.GetFocus()?) }
    }

    pub(super) unsafe fn from_document(carrier: &ITfDocumentMgr) -> windows::core::Result<Self> {
        unsafe {
            let carrier_context = carrier.GetTop()?;
            if carrier_context.GetDocumentMgr()? != *carrier {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let document = if carrier_context.GetStatus()?.dwStaticFlags & TF_SS_TRANSITORY != 0 {
                // GetCompartment creates missing entries. Inspect only a native
                // relation which already exists; never create or retarget it.
                let compartments: ITfCompartmentMgr = carrier.cast()?;
                let ids = compartments.EnumCompartments()?;
                let mut parent = None;
                for _ in 0..64 {
                    let mut id = [GUID::zeroed()];
                    let mut fetched = 0;
                    ids.Next(&mut id, Some(&mut fetched)).ok()?;
                    if fetched == 0 {
                        break;
                    }
                    if id[0] == GUID_COMPARTMENT_TRANSITORYEXTENSION_PARENT {
                        let value = compartments.GetCompartment(&id[0])?.GetValue()?;
                        parent = Some(IUnknown::try_from(&value)?.cast::<ITfDocumentMgr>()?);
                        break;
                    }
                }
                // Without a parent, retain the original carrier path. A range
                // which later disappears still fails the existing block proof.
                parent.unwrap_or_else(|| carrier.clone())
            } else {
                carrier.clone()
            };
            let context = document.GetTop()?;
            if context.GetDocumentMgr()? != document
                || (document != *carrier
                    && (context == carrier_context
                        || context.GetStatus()?.dwStaticFlags & TF_SS_TRANSITORY != 0))
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            let window = windows::Win32::UI::Input::KeyboardAndMouse::GetFocus();
            Ok(Self {
                carrier: carrier.clone(),
                carrier_context,
                document,
                context,
                window,
                root: GetAncestor(window, GA_ROOT),
            })
        }
    }

    pub(super) fn same(&self, other: &Self) -> bool {
        self.carrier == other.carrier
            && self.carrier_context == other.carrier_context
            && self.document == other.document
            && self.context == other.context
            && self.window == other.window
            && self.root == other.root
    }

    pub(super) fn focus_matches(&self) -> bool {
        unsafe {
            !self.window.0.is_null()
                && IsWindow(Some(self.window)).as_bool()
                && windows::Win32::UI::Input::KeyboardAndMouse::GetFocus() == self.window
                && GetAncestor(self.window, GA_ROOT) == self.root
                && GetForegroundWindow() == self.root
        }
    }

    pub(super) fn permits(&self) -> windows::core::Result<()> {
        unsafe {
            for context in [&self.carrier_context, &self.context] {
                if context.GetStatus()?.dwDynamicFlags & TF_SD_READONLY != 0 {
                    return Err(failure(TF_E_READONLY));
                }
                ensure_enabled_context(context)?;
            }
            Ok(())
        }
    }

    pub(super) fn owns_document(&self, document: &ITfDocumentMgr) -> bool {
        document == &self.carrier || document == &self.document
    }

    pub(super) fn owns_context(&self, context: &ITfContext) -> bool {
        context == &self.carrier_context || context == &self.context
    }

    pub(super) fn check_carrier(&self, client: u32) -> windows::core::Result<()> {
        self.with_carrier(client, || Ok(()))
    }

    pub(super) fn with_carrier<T: 'static>(
        &self,
        client: u32,
        work: impl FnOnce() -> windows::core::Result<T> + 'static,
    ) -> windows::core::Result<T> {
        if self.carrier_context == self.context {
            return work();
        }
        let result = Rc::new(Cell::new(None));
        let output = Rc::new(RefCell::new(None));
        let written = output.clone();
        let open = Rc::new(Cell::new(true));
        let edit: ITfEditSession = CarrierRead {
            context: self.carrier_context.clone(),
            result: result.clone(),
            work: RefCell::new(Some(Box::new(move || {
                let value = work()?;
                *written.borrow_mut() = Some(value);
                Ok(())
            }))),
            open: open.clone(),
            _lease: Lease::new(),
        }
        .into();
        // A cookie from the parent cannot inspect a carrier's properties.
        // Use the carrier's own read session, including while its parent is
        // locked. An unavailable read never grants mutation permission.
        let requested = unsafe {
            self.carrier_context
                .RequestEditSession(client, &edit, TF_ES_SYNC | TF_ES_READ)
        };
        open.set(false);
        requested?.ok()?;
        result.get().ok_or_else(|| failure(TF_E_LOCKED))?.ok()?;
        output
            .borrow_mut()
            .take()
            .ok_or_else(|| failure(TF_E_DISCONNECTED))
    }
}

#[implement(ITfEditSession)]
struct CarrierRead {
    context: ITfContext,
    result: Rc<Cell<Option<HRESULT>>>,
    work: RefCell<Option<CarrierWork>>,
    open: Rc<Cell<bool>>,
    _lease: Lease,
}
type CarrierWork = Box<dyn FnOnce() -> windows::core::Result<()>>;
impl ITfEditSession_Impl for CarrierRead_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        let _callback = Callback::enter();
        if !self.open.get() {
            return Err(failure(TF_E_DISCONNECTED));
        }
        let result = (|| unsafe {
            if self.context.GetStatus()?.dwDynamicFlags & TF_SD_READONLY != 0 {
                return Err(failure(TF_E_READONLY));
            }
            ensure_enabled_context(&self.context)?;
            if password(&self.context, &selection(&self.context, ec)?, ec)? {
                return Err(failure(E_ACCESSDENIED));
            }
            let owner: ITfContextComposition = self.context.cast()?;
            let mut entries = [None];
            let mut count = 0;
            owner.EnumCompositions()?.Next(&mut entries, &mut count)?;
            if count != 0 {
                return Err(failure(TF_E_LOCKED));
            }
            let work = self
                .work
                .borrow_mut()
                .take()
                .ok_or_else(|| failure(E_UNEXPECTED))?;
            work()
        })();
        self.result
            .set(Some(result.map_or_else(|e| e.code(), |()| S_OK)));
        Ok(())
    }
}

pub(super) unsafe fn focused(manager: &ITfThreadMgr) -> windows::core::Result<ITfContext> {
    unsafe { Ok(Binding::focused(manager)?.context) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_carrier_callback_cannot_use_a_parent_cookie() {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            let result = (|| -> windows::core::Result<()> {
                let manager: ITfThreadMgr =
                    CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
                let client = manager.Activate()?;
                let result = (|| -> windows::core::Result<()> {
                    let document = manager.CreateDocumentMgr()?;
                    let (mut context, mut cookie) = (None, 0);
                    document.CreateContext(client, 0, None, &mut context, &mut cookie)?;
                    let calls = Rc::new(Cell::new(0));
                    let ran = calls.clone();
                    let edit: ITfEditSession = CarrierRead {
                        context: context.ok_or_else(|| failure(E_POINTER))?,
                        result: Rc::new(Cell::new(None)),
                        open: Rc::new(Cell::new(false)),
                        work: RefCell::new(Some(Box::new(move || {
                            ran.set(ran.get() + 1);
                            Ok(())
                        }))),
                        _lease: Lease::new(),
                    }
                    .into();
                    assert_eq!(
                        edit.DoEditSession(cookie).unwrap_err().code(),
                        TF_E_DISCONNECTED
                    );
                    assert_eq!(
                        edit.DoEditSession(cookie).unwrap_err().code(),
                        TF_E_DISCONNECTED
                    );
                    assert_eq!(calls.get(), 0);
                    Ok(())
                })();
                manager.Deactivate()?;
                result
            })();
            CoUninitialize();
            result.unwrap();
        }
    }
}
