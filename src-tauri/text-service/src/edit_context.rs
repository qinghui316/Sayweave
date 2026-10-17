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
    #[cfg(test)]
    pub(super) unsafe fn from_document(carrier: &ITfDocumentMgr) -> windows::core::Result<Self> {
        unsafe { Self::discover(carrier, &mut String::new()) }
    }

    pub(super) unsafe fn discover(
        carrier: &ITfDocumentMgr,
        probe: &mut String,
    ) -> windows::core::Result<Self> {
        unsafe {
            probe.push_str("carrierTop");
            let carrier_context = carrier.GetTop()?;
            probe.push_str("/carrierOwner");
            match carrier_context.GetDocumentMgr() {
                Ok(owner) if owner != *carrier => return Err(failure(TF_E_DISCONNECTED)),
                Err(error) if error.code() == E_NOINTERFACE => probe.push_str("=80004002"),
                Err(error) => return Err(error),
                _ => (),
            }
            probe.push_str("/carrierStatus");
            let status = carrier_context.GetStatus()?;
            probe.push_str(&format!(
                ".static={}.dynamic={}",
                status.dwStaticFlags, status.dwDynamicFlags
            ));
            match carrier.GetBase() {
                Ok(base) => probe.push_str(&format!("/base.same={}", base == carrier_context)),
                Err(error) => probe.push_str(&format!("/base.error={:08x}", error.code().0 as u32)),
            }
            let document = if status.dwStaticFlags & TF_SS_TRANSITORY != 0 {
                parent_document(carrier, probe)?.unwrap_or_else(|| carrier.clone())
            } else {
                probe.push_str("/direct");
                carrier.clone()
            };
            probe.push_str("/documentTop");
            let context = document.GetTop()?;
            probe.push_str("/documentOwner");
            let owner = context.GetDocumentMgr();
            if owner.as_ref().is_ok_and(|owner| *owner != document) {
                return Err(failure(TF_E_DISCONNECTED));
            }
            if let Err(error) = owner {
                if error.code() != E_NOINTERFACE || document != *carrier {
                    return Err(error);
                }
                probe.push_str("=80004002");
            }
            probe.push_str("/documentStatus");
            if document != *carrier
                && (context == carrier_context
                    || context.GetStatus()?.dwStaticFlags & TF_SS_TRANSITORY != 0)
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

    // Verify the route chosen by Begin. Availability changes never select a
    // different document for a live write or a retained block.
    pub(super) unsafe fn matches_document(
        &self,
        carrier: &ITfDocumentMgr,
    ) -> windows::core::Result<bool> {
        unsafe {
            if *carrier != self.carrier || carrier.GetTop()? != self.carrier_context {
                return Ok(false);
            }
            if self.document != self.carrier
                && parent_document(carrier, &mut String::new())?.as_ref() != Some(&self.document)
            {
                return Ok(false);
            }
            Ok(self.document.GetTop()? == self.context)
        }
    }

    pub(super) unsafe fn current(&self, manager: &ITfThreadMgr) -> windows::core::Result<bool> {
        unsafe { Ok(self.matches_document(&manager.GetFocus()?)? && self.focus_matches()) }
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

    // Callback-time facts supplement the first-refusal record. They do not
    // grant a scope or change the saved document selected by Begin.
    pub(super) fn focus_diagnostic(
        &self,
        document: Option<&ITfDocumentMgr>,
        previous: Option<&ITfDocumentMgr>,
        matched: &Option<windows::core::Result<bool>>,
    ) -> serde_json::Value {
        unsafe {
            let window = windows::Win32::UI::Input::KeyboardAndMouse::GetFocus();
            serde_json::json!({
                "document_present": document.is_some(),
                "document_is_carrier": document == Some(&self.carrier),
                "document_is_previous": document == previous,
                "previous_is_carrier": previous == Some(&self.carrier),
                "document_matches": matched.as_ref().and_then(|value| value.as_ref().ok()).copied(),
                "document_error": matched.as_ref().and_then(|value| value.as_ref().err()).map(|error| error.code().0 as u32),
                "window_alive": IsWindow(Some(self.window)).as_bool(),
                "window_matches": window == self.window,
                "root_matches": GetAncestor(self.window, GA_ROOT) == self.root,
                "foreground_matches": GetForegroundWindow() == self.root,
            })
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

// Absence/type/interface availability is optional discovery evidence. Other
// failures, notably explicit permission denial, keep their original HRESULT.
unsafe fn parent_document(
    carrier: &ITfDocumentMgr,
    probe: &mut String,
) -> windows::core::Result<Option<ITfDocumentMgr>> {
    unsafe {
        probe.push_str("/compartmentInterface");
        let compartments: ITfCompartmentMgr = match carrier.cast() {
            Ok(value) => value,
            Err(error) if error.code() == E_NOINTERFACE => {
                probe.push_str("=80004002/direct");
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        probe.push_str("/enumerate");
        let ids = compartments.EnumCompartments()?;
        for _ in 0..64 {
            let mut id = [GUID::zeroed()];
            let mut fetched = 0;
            ids.Next(&mut id, Some(&mut fetched)).ok()?;
            if fetched == 0 {
                probe.push_str("/absent/direct");
                return Ok(None);
            }
            if id[0] == GUID_COMPARTMENT_TRANSITORYEXTENSION_PARENT {
                probe.push_str("/value");
                let value = compartments.GetCompartment(&id[0])?.GetValue()?;
                probe.push_str(&format!(".vt={}", value.Anonymous.Anonymous.vt.0));
                return parent_value(&value, probe);
            }
        }
        Err(failure(E_UNEXPECTED))
    }
}

fn parent_value(
    value: &windows::Win32::System::Variant::VARIANT,
    probe: &mut String,
) -> windows::core::Result<Option<ITfDocumentMgr>> {
    let unknown = match IUnknown::try_from(value) {
        Ok(value) => value,
        Err(error) if error.code() == windows::Win32::Foundation::TYPE_E_TYPEMISMATCH => {
            probe.push_str("/notObject/direct");
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    probe.push_str("/documentInterface");
    match unknown.cast() {
        Ok(document) => {
            probe.push_str("/parent");
            Ok(Some(document))
        }
        Err(error) if error.code() == E_NOINTERFACE => {
            probe.push_str("=80004002");
            // Some transitory providers expose an object without the documented
            // document-manager interface. Observe its actual standard interfaces
            // without treating one of them as authority for another document.
            let context = unknown.cast::<ITfContext>();
            probe.push_str(&format!(
                "/contextInterface={:08x}",
                context.as_ref().err().map_or(0, |e| e.code().0 as u32)
            ));
            if let Ok(context) = context {
                match unsafe { context.GetDocumentMgr() } {
                    Ok(_) => probe.push_str("/contextOwner=00000000"),
                    Err(error) => {
                        probe.push_str(&format!("/contextOwner={:08x}", error.code().0 as u32))
                    }
                }
            }
            probe.push_str("/direct");
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

pub(super) unsafe fn retained(
    shared: &Shared,
    manager: &ITfThreadMgr,
) -> windows::core::Result<Binding> {
    let native = { shared.borrow().native_context.clone() }
        .or_else(|| block_tracking::binding(shared))
        .ok_or_else(|| failure(TF_E_DISCONNECTED))?;
    if unsafe { native.current(manager)? } {
        Ok(native)
    } else {
        Err(failure(TF_E_DISCONNECTED))
    }
}

pub(super) unsafe fn focused(
    shared: &Shared,
    manager: &ITfThreadMgr,
) -> windows::core::Result<ITfContext> {
    unsafe { Ok(retained(shared, manager)?.context) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optional_parent_values_preserve_direct_input_and_pinned_routes() {
        use windows::Win32::System::Variant::VARIANT;
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
            let result = (|| -> windows::core::Result<()> {
                let manager: ITfThreadMgr =
                    CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
                let client = manager.Activate()?;
                let result = (|| -> windows::core::Result<()> {
                    let carrier = manager.CreateDocumentMgr()?;
                    let parent = manager.CreateDocumentMgr()?;
                    for document in [&carrier, &parent] {
                        let (mut context, mut cookie) = (None, 0);
                        document.CreateContext(client, 0, None, &mut context, &mut cookie)?;
                        document.Push(&context.ok_or_else(|| failure(E_POINTER))?)?;
                    }
                    let direct = Binding::from_document(&carrier)?;
                    let mut probe = String::new();
                    assert!(parent_document(&carrier, &mut probe)?.is_none());
                    assert!(probe.contains("absent/direct"));
                    let compartments: ITfCompartmentMgr = carrier.cast()?;
                    let slot = compartments
                        .GetCompartment(&GUID_COMPARTMENT_TRANSITORYEXTENSION_PARENT)?;
                    // The enumerated slot exists, but GetValue returns VT_EMPTY.
                    probe.clear();
                    assert!(parent_document(&carrier, &mut probe)?.is_none());
                    assert!(probe.contains("vt=0/notObject/direct"));
                    for value in [VARIANT::from(7i32), VARIANT::from("owned value")] {
                        slot.SetValue(client, &value)?;
                        assert!(parent_document(&carrier, &mut String::new())?.is_none());
                    }
                    let non_document: IUnknown = manager.cast()?;
                    slot.SetValue(client, &VARIANT::from(non_document))?;
                    probe.clear();
                    assert!(parent_document(&carrier, &mut probe)?.is_none());
                    assert!(probe.contains("documentInterface=80004002"));
                    let parent_unknown: IUnknown = parent.cast()?;
                    slot.SetValue(client, &VARIANT::from(parent_unknown))?;
                    assert_eq!(
                        parent_document(&carrier, &mut String::new())?,
                        Some(parent.clone())
                    );
                    // Direct admission does not change when a parent appears.
                    assert!(direct.matches_document(&carrier)?);
                    assert_eq!(direct.document, carrier);
                    let selected = Binding {
                        document: parent.clone(),
                        context: parent.GetTop()?,
                        ..direct.clone()
                    };
                    assert!(selected.matches_document(&carrier)?);
                    compartments
                        .ClearCompartment(client, &GUID_COMPARTMENT_TRANSITORYEXTENSION_PARENT)?;
                    // A selected parent cannot silently become Direct.
                    assert!(!selected.matches_document(&carrier)?);
                    assert!(direct.matches_document(&carrier)?);
                    parent.Pop(TF_POPF_ALL)?;
                    carrier.Pop(TF_POPF_ALL)?;
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
