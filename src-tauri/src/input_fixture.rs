//! An owned blank Win32 editor for opt-in input acceptance. No personal application is reused.
use std::{
    io::{self, BufRead, Write},
    sync::mpsc,
    thread,
};
use windows::{
    Win32::{
        Foundation::*,
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    },
    core::{BOOL, w},
};
unsafe extern "system" fn fixture_proc(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if m == WM_RENDERFORMAT {
        std::thread::sleep(std::time::Duration::from_millis(1200));
        unsafe {
            use windows::Win32::System::{DataExchange::*, Memory::*};
            let text = b"{\\rtf1 delayed original}\0";
            if let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, text.len()) {
                let data = GlobalLock(memory);
                if !data.is_null() {
                    std::ptr::copy_nonoverlapping(text.as_ptr(), data.cast(), text.len());
                    let _ = GlobalUnlock(memory);
                    let _ = SetClipboardData(w.0 as u32, Some(HANDLE(memory.0)));
                }
            }
        }
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(h, m, w, l) }
}
fn message(h: HWND, m: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    unsafe { SendMessageW(h, m, Some(w), Some(l)) }
}
#[windows::core::implement(windows::Win32::UI::TextServices::ITfEditSession)]
struct ContextReader {
    context: windows::Win32::UI::TextServices::ITfContext,
    result: std::rc::Rc<std::cell::RefCell<Option<serde_json::Value>>>,
}
impl windows::Win32::UI::TextServices::ITfEditSession_Impl for ContextReader_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        use windows::Win32::UI::TextServices::TF_ANCHOR_END;
        let mut phase = "getStart";
        let result = (|| unsafe {
            let range = self.context.GetStart(ec)?;
            phase = "getEnd";
            let end = self.context.GetEnd(ec)?;
            phase = "extend";
            range.ShiftEndToRange(ec, &end, TF_ANCHOR_END)?;
            phase = "getText";
            let mut buffer = [0u16; 1024];
            let mut fetched = 0;
            range.GetText(ec, 0, &mut buffer, &mut fetched)?;
            Ok::<_, windows::core::Error>(fetched)
        })();
        *self.result.borrow_mut() = Some(serde_json::json!({
            "units":result.as_ref().ok(), "limit":1024, "phase":phase,
            "error":result.err().map(|e|format!("{:08x}",e.code().0 as u32)),
        }));
        Ok(())
    }
}
fn context_read(
    manager: &windows::Win32::UI::TextServices::ITfThreadMgr,
    context: &windows::Win32::UI::TextServices::ITfContext,
) -> serde_json::Value {
    use windows::Win32::UI::TextServices::*;
    unsafe {
        let client = match manager.Activate() {
            Ok(client) => client,
            Err(e) => {
                return serde_json::json!({"activationError":format!("{:08x}",e.code().0 as u32)});
            }
        };
        let value = std::rc::Rc::new(std::cell::RefCell::new(None));
        let edit: ITfEditSession = ContextReader {
            context: context.clone(),
            result: value.clone(),
        }
        .into();
        let request = context.RequestEditSession(client, &edit, TF_ES_SYNC | TF_ES_READ);
        drop(edit);
        let deactivate = manager.Deactivate();
        let read = value.borrow().clone();
        serde_json::json!({"requestError":request.as_ref().err().map(|e|format!("{:08x}",e.code().0 as u32)),
            "sessionResult":request.ok().map(|r|format!("{:08x}",r.0 as u32)),
            "deactivateError":deactivate.err().map(|e|format!("{:08x}",e.code().0 as u32)),"read":read})
    }
}
fn context_status() -> serde_json::Value {
    use windows::Win32::{System::Com::*, UI::TextServices::*};
    use windows::core::Interface;
    let result = (|| unsafe {
        let manager: ITfThreadMgr =
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
        let document = manager.GetFocus()?;
        let context = document.GetTop()?;
        let status = context.GetStatus()?;
        let mut relations = vec![];
        let mut complete = false;
        let topology = (|| -> windows::core::Result<()> {
            let compartments = document.cast::<ITfCompartmentMgr>()?;
            // Enumerate first: GetCompartment would create an absent entry.
            let ids = compartments.EnumCompartments()?;
            for _ in 0..64 {
                let mut id = [windows::core::GUID::zeroed()];
                let mut fetched = 0;
                ids.Next(&mut id, Some(&mut fetched)).ok()?;
                if fetched == 0 {
                    complete = true;
                    break;
                }
                let relation = if id[0] == GUID_COMPARTMENT_TRANSITORYEXTENSION_PARENT {
                    "parent"
                } else if id[0] == GUID_COMPARTMENT_TRANSITORYEXTENSION_DOCUMENTMANAGER {
                    "transitory"
                } else {
                    continue;
                };
                let related = (|| {
                    let value = compartments.GetCompartment(&id[0])?.GetValue()?;
                    let related =
                        windows::core::IUnknown::try_from(&value)?.cast::<ITfDocumentMgr>()?;
                    let top = related.GetTop()?;
                    let status = top.GetStatus()?;
                    Ok::<_, windows::core::Error>(serde_json::json!({
                        "sameDocument": related == document,
                        "sameContext": top == context,
                        "topIsBase": related.GetBase()? == top,
                        "staticFlags": status.dwStaticFlags,
                        "dynamicFlags": status.dwDynamicFlags,
                        "read": context_read(&manager, &top),
                    }))
                })();
                relations.push(serde_json::json!({"relation":relation,
                    "document":related.as_ref().ok(),
                    "error":related.err().map(|e|format!("{:08x}",e.code().0 as u32))}));
            }
            Ok(())
        })();
        Ok::<_, windows::core::Error>(serde_json::json!({
            "staticFlags": status.dwStaticFlags,
            "dynamicFlags": status.dwDynamicFlags,
            "transitory": status.dwStaticFlags & TF_SS_TRANSITORY != 0,
            "topIsBase": document.GetBase()? == context,
            "compositions": context_compositions(&context),
            "relations": relations,
            "relationsComplete": complete,
            "relationsError": topology.err().map(|e|format!("{:08x}",e.code().0 as u32)),
            "read": context_read(&manager, &context),
        }))
    })();
    result.unwrap_or_else(|e| serde_json::json!({"error":format!("{:08x}",e.code().0 as u32)}))
}
fn context_compositions(
    context: &windows::Win32::UI::TextServices::ITfContext,
) -> serde_json::Value {
    use windows::{Win32::UI::TextServices::*, core::Interface};
    let result = (|| unsafe {
        let owner: ITfContextComposition = context.cast()?;
        let enumeration = owner.EnumCompositions()?;
        let mut values = vec![];
        for _ in 0..8 {
            let mut next = [None];
            let mut fetched = 0;
            enumeration.Next(&mut next, &mut fetched)?;
            if fetched == 0 {
                break;
            }
            let composition = next[0].as_ref().ok_or(E_POINTER)?;
            values.push(
                serde_json::json!({"owner":format!("{:?}",composition.GetOwnerClsid()?),
                "sameContext":composition.GetRange()?.GetContext()? == *context}),
            );
        }
        Ok::<_, windows::core::Error>(values)
    })();
    serde_json::json!({"values":result.as_ref().ok(),"error":result.err().map(|e|format!("{:08x}",e.code().0 as u32))})
}
#[windows::core::implement(windows::Win32::UI::TextServices::ITfCompositionSink)]
struct ParentProbeSink;
impl windows::Win32::UI::TextServices::ITfCompositionSink_Impl for ParentProbeSink_Impl {
    fn OnCompositionTerminated(
        &self,
        _: u32,
        _: windows::core::Ref<windows::Win32::UI::TextServices::ITfComposition>,
    ) -> windows::core::Result<()> {
        Ok(())
    }
}
#[windows::core::implement(windows::Win32::UI::TextServices::ITfEditSession)]
struct ParentWriteGuard {
    delegate: windows::Win32::UI::TextServices::ITfEditSession,
    parent_cookie: u32,
}
impl windows::Win32::UI::TextServices::ITfEditSession_Impl for ParentWriteGuard_Impl {
    fn DoEditSession(&self, _: u32) -> windows::core::Result<()> {
        unsafe { self.delegate.DoEditSession(self.parent_cookie) }
    }
}
#[windows::core::implement(windows::Win32::UI::TextServices::ITfEditSession)]
struct ParentWriteProbe {
    context: windows::Win32::UI::TextServices::ITfContext,
    carrier: windows::Win32::UI::TextServices::ITfContext,
    client: u32,
    guarded: bool,
    result: std::rc::Rc<std::cell::RefCell<Option<serde_json::Value>>>,
}
impl windows::Win32::UI::TextServices::ITfEditSession_Impl for ParentWriteProbe_Impl {
    fn DoEditSession(&self, ec: u32) -> windows::core::Result<()> {
        use windows::{Win32::UI::TextServices::*, core::Interface};
        if !self.guarded {
            let delegate: ITfEditSession = ParentWriteProbe {
                context: self.context.clone(),
                carrier: self.carrier.clone(),
                client: self.client,
                guarded: true,
                result: self.result.clone(),
            }
            .into();
            let read: ITfEditSession = ParentWriteGuard {
                delegate,
                parent_cookie: ec,
            }
            .into();
            return unsafe {
                self.carrier
                    .RequestEditSession(self.client, &read, TF_ES_SYNC | TF_ES_READ)?
                    .ok()
            };
        }
        let mut phase = "selection";
        let mut changed = false;
        let before = context_compositions(&self.carrier);
        let child_read = std::rc::Rc::new(std::cell::RefCell::new(None));
        let child_session: ITfEditSession = ContextReader {
            context: self.carrier.clone(),
            result: child_read.clone(),
        }
        .into();
        let child_request = unsafe {
            self.carrier
                .RequestEditSession(self.client, &child_session, TF_ES_SYNC | TF_ES_READ)
        };
        let nested_read = serde_json::json!({"requestError":child_request.as_ref().err().map(|e|format!("{:08x}",e.code().0 as u32)),
            "sessionResult":child_request.ok().map(|r|format!("{:08x}",r.0 as u32)),"read":child_read.borrow().clone()});
        let result = (|| unsafe {
            let mut selected = [TF_SELECTION::default()];
            let mut fetched = 0;
            let read =
                self.context
                    .GetSelection(ec, TF_DEFAULT_SELECTION, &mut selected, &mut fetched);
            let range = std::mem::ManuallyDrop::take(&mut selected[0].range);
            read?;
            if fetched != 1 {
                return Err(E_FAIL.into());
            }
            let range = range.ok_or_else(|| windows::core::Error::from_hresult(E_POINTER))?;
            phase = "composition";
            let owner = self.context.cast::<ITfContextComposition>()?;
            let sink: ITfCompositionSink = ParentProbeSink.into();
            let composition = owner.StartComposition(ec, &range, &sink)?;
            phase = "write";
            let owned = composition.GetRange()?;
            owned.SetText(ec, 0, &"父文档探针".encode_utf16().collect::<Vec<_>>())?;
            changed = true;
            phase = "end";
            composition.EndComposition(ec)
        })();
        *self.result.borrow_mut() = Some(
            serde_json::json!({"phase":phase,"changed":changed,"carrierBefore":before,"carrierAfter":context_compositions(&self.carrier),"nestedCarrierRead":nested_read,
            "error":result.err().map(|e|format!("{:08x}",e.code().0 as u32))}),
        );
        Ok(())
    }
}
fn parent_write_probe() -> serde_json::Value {
    use windows::{
        Win32::{System::Com::*, UI::TextServices::*},
        core::Interface,
    };
    let result = (|| unsafe {
        let manager: ITfThreadMgr =
            CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER)?;
        let document = manager.GetFocus()?;
        let compartments = document.cast::<ITfCompartmentMgr>()?;
        let ids = compartments.EnumCompartments()?;
        let mut parent = None;
        for _ in 0..64 {
            let mut id = [windows::core::GUID::zeroed()];
            let mut fetched = 0;
            ids.Next(&mut id, Some(&mut fetched)).ok()?;
            if fetched == 0 {
                break;
            }
            if id[0] == GUID_COMPARTMENT_TRANSITORYEXTENSION_PARENT {
                let value = compartments.GetCompartment(&id[0])?.GetValue()?;
                parent = Some(
                    windows::core::IUnknown::try_from(&value)?
                        .cast::<ITfDocumentMgr>()?
                        .GetTop()?,
                );
                break;
            }
        }
        let context = parent.ok_or_else(|| windows::core::Error::from_hresult(E_NOINTERFACE))?;
        if context.GetStatus()?.dwDynamicFlags & TF_SD_READONLY != 0 {
            return Err(E_ACCESSDENIED.into());
        }
        let client = manager.Activate()?;
        let value = std::rc::Rc::new(std::cell::RefCell::new(None));
        let edit: ITfEditSession = ParentWriteProbe {
            context: context.clone(),
            carrier: document.GetTop()?,
            client,
            guarded: false,
            result: value.clone(),
        }
        .into();
        let request = context.RequestEditSession(client, &edit, TF_ES_SYNC | TF_ES_READWRITE);
        drop(edit);
        let deactivate = manager.Deactivate();
        let proof = value.borrow().clone();
        Ok::<_, windows::core::Error>(serde_json::json!({
            "requestError":request.as_ref().err().map(|e|format!("{:08x}",e.code().0 as u32)),
            "sessionResult":request.ok().map(|r|format!("{:08x}",r.0 as u32)),
            "deactivateError":deactivate.err().map(|e|format!("{:08x}",e.code().0 as u32)),"write":proof}))
    })();
    result.unwrap_or_else(|e| serde_json::json!({"error":format!("{:08x}",e.code().0 as u32)}))
}
// Metadata-only diagnostic for an explicitly owned foreground fixture. Never
// reads a field name/value/body and never supplies production range authority.
fn focus_metadata(process: u32) -> serde_json::Value {
    focus_metadata_at(process, None)
}
fn focus_metadata_at(process: u32, expected: Option<usize>) -> serde_json::Value {
    std::thread::spawn(move || unsafe {
        use windows::{core::Interface, Win32::{System::{Com::*, Ole::*, Variant::*}, UI::Accessibility::*}};
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let result = (|| {
            let window=expected.map(|v|HWND(v as *mut _)).unwrap_or_else(||GetForegroundWindow());let mut actual=0;
            let thread=GetWindowThreadProcessId(window,Some(&mut actual));
            if actual!=process { return Err(windows::core::Error::from_hresult(E_ACCESSDENIED)); }
            let mut info=GUITHREADINFO { cbSize:std::mem::size_of::<GUITHREADINFO>() as u32,..Default::default() };
            GetGUIThreadInfo(thread,&mut info)?;
            let mut routes=vec![];let mut identities:Vec<windows::core::IUnknown>=vec![];
            let mut stable=vec![];let mut runtime_ids=vec![];
            for target in [info.hwndFocus,window] {
                let mut accessible: Option<IAccessible>=None;
                let value=AccessibleObjectFromWindow(target,OBJID_CLIENT.0 as u32,&IAccessible::IID,(&mut accessible as *mut Option<IAccessible>).cast());
                let mut route=serde_json::json!({"window":target.0 as usize,"error":value.as_ref().err().map(|e|format!("{:08x}",e.code().0 as u32)),"chain":[]});
                if value.is_ok() && let Some(mut current)=accessible {
                    for _ in 0..16 {
                        let child=VARIANT::from(0i32);
                        let focus=current.accFocus();
                        route["chain"].as_array_mut().unwrap().push(serde_json::json!({"focusVt":focus.as_ref().ok().map(|f|f.vt().0),"focusChild":focus.as_ref().ok().and_then(|f|i32::try_from(f).ok()),"children":current.accChildCount().ok(),"role":current.get_accRole(&child).ok().and_then(|r|i32::try_from(&r).ok()),"state":current.get_accState(&child).ok().and_then(|r|i32::try_from(&r).ok()),"focusError":focus.as_ref().err().map(|e|format!("{:08x}",e.code().0 as u32))}));
                        let Ok(focus)=focus else { break; };
                        let next=if focus.vt()==VT_DISPATCH { IDispatch::try_from(&focus).and_then(|v|v.cast()) } else if focus.vt()==VT_I4 && i32::try_from(&focus).unwrap_or(0)!=0 { current.get_accChild(&focus).and_then(|v|v.cast()) } else { break; };
                        let Ok(next)=next else { break; };current=next;
                    }
                    let mut tree=vec![];let mut queue=vec![(current.clone(),0u32)];
                    while let Some((node,depth))=queue.pop() {
                        if depth>=4 || tree.len()>=64 {continue;}
                        for index in 1..=node.accChildCount().unwrap_or(0).clamp(0,32) {
                            if tree.len()>=64 {break;}
                            let child=VARIANT::from(index);let next=node.get_accChild(&child).and_then(|v|v.cast::<IAccessible>());
                            tree.push(serde_json::json!({"depth":depth,"child":index,
                                "role":node.get_accRole(&child).ok().and_then(|v|i32::try_from(&v).ok()),
                                "state":node.get_accState(&child).ok().and_then(|v|i32::try_from(&v).ok()),
                                "object":next.is_ok(),"error":next.as_ref().err().map(|e|format!("{:08x}",e.code().0 as u32))}));
                            if let Ok(next)=next {queue.push((next,depth+1));}
                        }
                    }
                    route["descendants"]=serde_json::json!(tree);
                    let mut opaque=std::ptr::null_mut();let mut length=0;
                    let bytes=if let Ok(identity)=current.cast::<IAccIdentity>() {
                        let result=identity.GetIdentityString(0,&mut opaque,&mut length);
                        route["opaqueIdentityHresult"]=serde_json::json!(result.as_ref().err().map(|e|format!("{:08x}",e.code().0 as u32)));
                        let bytes=if result.is_ok()&&!opaque.is_null()&&length>0&&length<=1024 {Some(std::slice::from_raw_parts(opaque,length as usize).to_vec())}else{None};
                        if !opaque.is_null(){CoTaskMemFree(Some(opaque.cast()));}bytes
                    }else{route["opaqueIdentityInterface"]=serde_json::json!(false);None};
                    route["accessibleEx"]=serde_json::json!(current.cast::<IAccessibleEx>().is_ok());
                    stable.push(bytes);
                    let runtime_id=(|| {
                        let raw=current.cast::<IRawElementProviderSimple>().or_else(|_|current.cast::<IServiceProvider>()?.QueryService(&IAccessible::IID))?;
                        let read_only=raw.GetPatternProvider(UIA_TextPatternId).and_then(|p|p.cast::<ITextProvider>()).and_then(|p|p.DocumentRange()).and_then(|r|r.GetAttributeValue(UIA_IsReadOnlyAttributeId)).ok().and_then(|v|bool::try_from(&v).ok());
                        route["uia"]=serde_json::json!({"controlType":raw.GetPropertyValue(UIA_ControlTypePropertyId).ok().and_then(|v|i32::try_from(&v).ok()),"keyboardFocus":raw.GetPropertyValue(UIA_HasKeyboardFocusPropertyId).ok().and_then(|v|bool::try_from(&v).ok()),"focusable":raw.GetPropertyValue(UIA_IsKeyboardFocusablePropertyId).ok().and_then(|v|bool::try_from(&v).ok()),"textReadOnly":read_only});
                        let array=raw.cast::<IRawElementProviderFragment>()?.GetRuntimeId()?;
                        let result=(||{if array.is_null()||SafeArrayGetDim(array)!=1||SafeArrayGetVartype(array)?!=VT_I4{return Err(windows::core::Error::from_hresult(E_NOINTERFACE));}let low=SafeArrayGetLBound(array,1)?;let high=SafeArrayGetUBound(array,1)?;if high<low||high-low>63{return Err(windows::core::Error::from_hresult(E_INVALIDARG));}let mut values=vec![];for index in low..=high {let mut value=0i32;SafeArrayGetElement(array,&index,(&mut value as *mut i32).cast())?;values.push(value);}Ok::<_,windows::core::Error>(values)})();
                        if !array.is_null(){let _=SafeArrayDestroy(array);}result
                    })();runtime_ids.push(runtime_id.ok());
                    if let Ok(identity)=current.cast(){ identities.push(identity); }
                }
                routes.push(route);
            }
            Ok::<_,windows::core::Error>(serde_json::json!({"routes":routes,"sameObject":identities.len()==2&&identities[0]==identities[1],"opaqueIdentityPresent":stable.iter().all(Option::is_some),"opaqueIdentitySame":stable.len()==2&&stable[0].is_some()&&stable[0]==stable[1],"runtimeIds":runtime_ids}))
        })();
        CoUninitialize();result.unwrap_or_else(|e|serde_json::json!({"error":format!("{:08x}",e.code().0 as u32)}))
    }).join().unwrap_or_else(|_|serde_json::json!({"error":"metadataWorker"}))
}
// A read-only acceptance observer for the explicitly selected native endpoint.
// This uses the host's existing MSAA/UIA provider when the UIA client bridge
// returns a null TextPattern. It never supplies production write authority.
fn owned_body_snapshot(
    process: u32,
    window: usize,
    expected: String,
    expected_runtime: Option<Vec<i32>>,
) -> serde_json::Value {
    if std::env::var("SAYWEAVE_TSF_OWNED_PID")
        .ok()
        .and_then(|v| v.parse().ok())
        != Some(process)
    {
        return serde_json::json!({"error":"explicitOwnedProcessRequired"});
    }
    thread::spawn(move || unsafe {
        use windows::{core::Interface, Win32::{System::{Com::*, Ole::*, Variant::*}, UI::Accessibility::*}};
        if let Err(e)=CoInitializeEx(None,COINIT_APARTMENTTHREADED).ok() {
            return serde_json::json!({"error":format!("{:08x}",e.code().0 as u32)});
        }
        let result=(|| {
            let root=HWND(window as *mut _);
            let focused=|| {
                if GetForegroundWindow()!=root {return Err(windows::core::Error::from_hresult(E_ACCESSDENIED));}
                let mut actual=0;let tid=GetWindowThreadProcessId(root,Some(&mut actual));
                if actual!=process {return Err(windows::core::Error::from_hresult(E_ACCESSDENIED));}
                let mut gui=GUITHREADINFO {cbSize:std::mem::size_of::<GUITHREADINFO>() as u32,..Default::default()};
                GetGUIThreadInfo(tid,&mut gui)?;
                if GetAncestor(gui.hwndFocus,GA_ROOT)!=root {return Err(windows::core::Error::from_hresult(E_ACCESSDENIED));}
                let mut accessible:Option<IAccessible>=None;
                AccessibleObjectFromWindow(gui.hwndFocus,OBJID_CLIENT.0 as u32,&IAccessible::IID,(&mut accessible as *mut Option<IAccessible>).cast())?;
                let mut accessible=accessible.ok_or_else(||windows::core::Error::from_hresult(E_NOINTERFACE))?;
                for _ in 0..16 {
                    let focus=accessible.accFocus()?;
                    if focus.vt()==VT_DISPATCH {accessible=IDispatch::try_from(&focus)?.cast()?;}
                    else if focus.vt()==VT_I4 && i32::try_from(&focus)?!=0 {accessible=accessible.get_accChild(&focus)?.cast()?;}
                    else {
                        let raw:IRawElementProviderSimple=accessible.cast().or_else(|_|accessible.cast::<IServiceProvider>()?.QueryService(&IAccessible::IID))?;
                        let kind=i32::try_from(&raw.GetPropertyValue(UIA_ControlTypePropertyId)?)?;
                        if ![UIA_EditControlTypeId.0,UIA_GroupControlTypeId.0].contains(&kind)
                            || !bool::try_from(&raw.GetPropertyValue(UIA_HasKeyboardFocusPropertyId)?)?
                            || !bool::try_from(&raw.GetPropertyValue(UIA_IsEnabledPropertyId)?)?
                            || bool::try_from(&raw.GetPropertyValue(UIA_IsPasswordPropertyId)?)? {
                            return Err(windows::core::Error::from_hresult(E_ACCESSDENIED));
                        }
                        return Ok(raw);
                    }
                }
                Err(windows::core::Error::from_hresult(E_FAIL))
            };
            let identity=|raw:&IRawElementProviderSimple| {
                let array=raw.cast::<IRawElementProviderFragment>()?.GetRuntimeId()?;
                let result=(|| {
                    if array.is_null()||SafeArrayGetDim(array)!=1||SafeArrayGetVartype(array)?!=VT_I4 {return Err(windows::core::Error::from_hresult(E_NOINTERFACE));}
                    let low=SafeArrayGetLBound(array,1)?;let high=SafeArrayGetUBound(array,1)?;
                    if high<low||high-low>63 {return Err(windows::core::Error::from_hresult(E_INVALIDARG));}
                    let mut values=vec![];for i in low..=high {let mut value=0i32;SafeArrayGetElement(array,&i,(&mut value as *mut i32).cast())?;values.push(value);}
                    Ok::<_,windows::core::Error>(values)
                })();
                if !array.is_null(){let _=SafeArrayDestroy(array);}result
            };
            let raw=focused()?;let before=identity(&raw)?;
            if expected_runtime.as_ref().is_some_and(|id|id!=&before) {return Err(windows::core::Error::from_hresult(E_ACCESSDENIED));}
            let pattern:ITextProvider=raw.GetPatternProvider(UIA_TextPatternId)?.cast()?;
            let range=pattern.DocumentRange()?;
            let readonly=range.GetAttributeValue(UIA_IsReadOnlyAttributeId)?;
            if readonly.vt()!=VT_BOOL||bool::try_from(&readonly)? {return Err(windows::core::Error::from_hresult(E_ACCESSDENIED));}
            let mut owner=range.GetEnclosingElement()?;let mut within=false;
            for _ in 0..16 {
                if identity(&owner).is_ok_and(|id|id==before){within=true;break;}
                owner=owner.cast::<IRawElementProviderFragment>()?.Navigate(NavigateDirection_Parent)?.cast()?;
            }
            if !within {return Err(windows::core::Error::from_hresult(E_ACCESSDENIED));}
            let body=range.GetText(1025)?.to_string();
            let after=identity(&focused()?)?;
            if before!=after||body.encode_utf16().count()>1024 {return Err(windows::core::Error::from_hresult(E_ACCESSDENIED));}
            // An explicitly retained field identity also admits diagnostic
            // representation before input (for example a host suggestion).
            let owned=expected_runtime.is_some()||body.starts_with("测试：")||body==expected||body==format!("{expected}\r\n");
            Ok::<_,windows::core::Error>(serde_json::json!({"process":process,"window":window,"runtimeId":before,"actualBody":owned.then_some(&body),"units":body.encode_utf16().count(),"matches":body==expected,"reader":"nativeFocusedProvider","readonly":false}))
        })();
        CoUninitialize();
        result.unwrap_or_else(|e|serde_json::json!({"error":format!("{:08x}",e.code().0 as u32)}))
    }).join().unwrap_or_else(|_|serde_json::json!({"error":"ownedBodyWorker"}))
}
// Read only known static product labels from the explicitly owned application.
// A native PID is not proof that the frontend rendered.
fn product_surface(process: u32) -> serde_json::Value {
    if std::env::var("SAYWEAVE_DAILY_OWNED_PID")
        .ok()
        .and_then(|p| p.parse().ok())
        != Some(process)
    {
        return serde_json::json!({"error":"ownedProductRequired"});
    }
    thread::spawn(move || unsafe {
        use windows::Win32::{System::Com::*,UI::Accessibility::*};
        let result=(|| {
            CoInitializeEx(None,COINIT_MULTITHREADED).ok()?;
            let mut target=ProcessWindow{process,window:HWND::default(),overlay:false};
            // The owned-window callback deliberately stops enumeration once
            // matched; FALSE with last-error0 is not a failed lookup.
            let _=EnumWindows(Some(process_window),LPARAM(&mut target as *mut _ as isize));
            if target.window.0.is_null(){return Err(windows::core::Error::from_hresult(E_FAIL));}
            let automation:IUIAutomation=CoCreateInstance(&CUIAutomation,None,CLSCTX_INPROC_SERVER)?;
            let root=automation.ElementFromHandle(target.window)?;
            let elements=root.FindAll(TreeScope_Descendants,&automation.CreateTrueCondition()?)?;
            let wanted=["首页","历史记录","词典","设置","开始口述","本机用量"];
            let mut found=std::collections::BTreeSet::new();let mut connection_error=false;
            for index in 0..elements.Length()?.min(1000) {
                let item=elements.GetElement(index)?;
                if item.CurrentIsOffscreen()?.as_bool(){continue;}
                let name=item.CurrentName()?.to_string();
                for label in wanted {if name==label{found.insert(label);}}
                connection_error|=name.contains("无法访问此页面")||name.contains("ERR_CONNECTION_REFUSED");
            }
            Ok::<_,windows::core::Error>(serde_json::json!({"process":process,"knownLabels":found,"frontendReady":wanted.iter().all(|n|found.contains(n)),"connectionError":connection_error}))
        })();
        CoUninitialize();result.unwrap_or_else(|e|serde_json::json!({"error":format!("{:08x}",e.code().0 as u32)}))
    }).join().unwrap_or_else(|_|serde_json::json!({"error":"surfaceWorker"}))
}
// Read-only evidence from a runner-owned target, on an independent MTA. Never
// emit external text or use these measurements to move a production selection.
fn selection_evidence(process: u32) -> serde_json::Value {
    thread::spawn(move || {
        use sha2::{Digest, Sha256};
        use windows::{core::Interface, Win32::{System::Com::*, UI::Accessibility::*}};
        let result = (|| unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            let result = (|| {
                let auto: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
                let limits: IUIAutomation2 = auto.cast()?;
                limits.SetConnectionTimeout(500)?;
                limits.SetTransactionTimeout(500)?;
                let element = auto.GetFocusedElement()?;
                if element.CurrentProcessId()? != process as i32 || !element.CurrentHasKeyboardFocus()?.as_bool() || element.CurrentIsPassword()?.as_bool() {
                    return Err(windows::core::Error::from_hresult(E_ACCESSDENIED));
                }
                let pattern: IUIAutomationTextPattern = element.GetCurrentPatternAs(UIA_TextPatternId)?;
                let ranges = pattern.GetSelection()?;
                if ranges.Length()? != 1 { return Err(windows::core::Error::from_hresult(E_FAIL)); }
                let selected = ranges.GetElement(0)?;
                let document = pattern.DocumentRange()?;
                let prefix = document.Clone()?;
                prefix.MoveEndpointByRange(TextPatternRangeEndpoint_End, &selected, TextPatternRangeEndpoint_Start)?;
                let suffix = document.Clone()?;
                suffix.MoveEndpointByRange(TextPatternRangeEndpoint_Start, &selected, TextPatternRangeEndpoint_End)?;
                let text = document.GetText(16385)?.to_string();
                let before = prefix.GetText(16385)?.to_string();
                let picked = selected.GetText(16385)?.to_string();
                let after = suffix.GetText(16385)?.to_string();
                let count = |value: &str| value.encode_utf16().count();
                if count(&text) > 16384 || text != format!("{before}{picked}{after}") {
                    return Err(windows::core::Error::from_hresult(E_INVALIDARG));
                }
                let current = pattern.GetSelection()?;
                let stable = current.Length()? == 1 && selected.Compare(&current.GetElement(0)?)?.as_bool()
                    && auto.CompareElements(&element, &auto.GetFocusedElement()?)?.as_bool();
                let msaa = (|| -> windows::core::Result<serde_json::Value> {
                    use windows::Win32::System::Variant::*;
                    let thread = GetWindowThreadProcessId(GetForegroundWindow(), None);
                    let mut info = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
                    GetGUIThreadInfo(thread, &mut info)?;
                    unsafe extern "system" fn child(window: HWND, parameter: LPARAM) -> BOOL {
                        unsafe { let windows = &mut *(parameter.0 as *mut Vec<HWND>); if windows.len() < 64 { windows.push(window); BOOL(1) } else { BOOL(0) } }
                    }
                    let mut windows = vec![info.hwndFocus];
                    let _ = EnumChildWindows(Some(info.hwndFocus), Some(child), LPARAM((&mut windows as *mut Vec<HWND>) as isize));
                    let mut evidence = Vec::new();
                    for window in windows {
                    let mut class = [0u16; 128];
                    let class_len = GetClassNameW(window, &mut class);
                    let mut result: Option<IAccessible> = None;
                    if AccessibleObjectFromWindow(window, OBJID_CLIENT.0 as u32, &IAccessible::IID, (&mut result as *mut Option<IAccessible>).cast()).is_err() {continue;}
                    let mut object = result.ok_or_else(|| windows::core::Error::from_hresult(E_FAIL))?;
                    let mut steps = Vec::new();
                    for _ in 0..16 {
                        let value = object.accFocus()?;
                        let role = i32::try_from(&object.get_accRole(&VARIANT::from(0i32))?).ok();
                        let service = object.cast::<IServiceProvider>();
                        let service_raw = service.as_ref().ok().is_some_and(|service| service.QueryService::<IRawElementProviderSimple>(&IAccessible::IID).is_ok());
                        steps.push(serde_json::json!({"role":role,"raw":object.cast::<IRawElementProviderSimple>().is_ok(),"ex":object.cast::<IAccessibleEx>().is_ok(),"service":service.is_ok(),"serviceRaw":service_raw,"focusType":value.Anonymous.Anonymous.vt.0,"focusChild":i32::try_from(&value).ok()}));
                        if value.Anonymous.Anonymous.vt != VT_DISPATCH {break;}
                        object = IDispatch::try_from(&value)?.cast()?;
                    }
                    evidence.push(serde_json::json!({"windowClass":String::from_utf16_lossy(&class[..class_len as usize]),"steps":steps}));
                    }
                    Ok(serde_json::json!(evidence))
                })().unwrap_or_else(|e|serde_json::json!({"error":format!("{:08x}",e.code().0 as u32)}));
                Ok::<_, windows::core::Error>(serde_json::json!({"stable":stable,"documentUtf16":count(&text),"startUtf16":count(&before),"endUtf16":count(&before)+count(&picked),"documentSha256":format!("{:x}",Sha256::digest(text.as_bytes())),"msaa":msaa}))
            })();
            CoUninitialize();
            result
        })();
        result.unwrap_or_else(|e| serde_json::json!({"error":format!("{:08x}",e.code().0 as u32)}))
    }).join().unwrap_or_else(|_| serde_json::json!({"error":"selectionWorker"}))
}
unsafe fn focus_window(window: HWND) {
    unsafe {
        use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
        let current = GetCurrentThreadId();
        let foreground = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let attached =
            current != foreground && AttachThreadInput(current, foreground, true).as_bool();
        let target_thread = GetWindowThreadProcessId(window, None);
        let target_attached = target_thread != current
            && target_thread != foreground
            && AttachThreadInput(current, target_thread, true).as_bool();
        let _ = ShowWindow(window, SW_RESTORE);
        let _ = BringWindowToTop(window);
        let _ = SetForegroundWindow(window);
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetActiveWindow(window);
        if target_attached {
            let _ = AttachThreadInput(current, target_thread, false);
        }
        if attached {
            let _ = AttachThreadInput(current, foreground, false);
        }
    }
}
struct ProcessWindow {
    process: u32,
    window: HWND,
    overlay: bool,
}
unsafe extern "system" fn process_window(window: HWND, parameter: LPARAM) -> BOOL {
    unsafe {
        let target = &mut *(parameter.0 as *mut ProcessWindow);
        let mut process = 0;
        GetWindowThreadProcessId(window, Some(&mut process));
        if process == target.process
            && IsWindowVisible(window).as_bool()
            && GetWindow(window, GW_OWNER).unwrap_or_default().0.is_null()
            && ((GetWindowLongPtrW(window, GWL_EXSTYLE) as u32 & WS_EX_NOACTIVATE.0 != 0)
                == target.overlay)
        {
            target.window = window;
            return BOOL(0);
        }
        BOOL(1)
    }
}
fn main() {
    if std::env::var("SAYWEAVE_INPUT_ACCEPTANCE").as_deref() != Ok("1") {
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("inspect-owned-body") {
        let process = std::env::args()
            .nth(2)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let window = std::env::args()
            .nth(3)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let expected = std::env::args().nth(4).unwrap_or_default();
        let runtime = std::env::args().nth(5);
        let expected_runtime = runtime
            .as_ref()
            .and_then(|raw| serde_json::from_str::<Vec<i32>>(raw).ok())
            .filter(|id| !id.is_empty() && id.len() <= 64);
        if runtime.is_some() && expected_runtime.is_none() {
            println!("{{\"error\":\"invalidOwnedFieldIdentity\"}}");
            return;
        }
        if process != 0 && window != 0 && expected.len() <= 1024 {
            println!(
                "{}",
                owned_body_snapshot(process, window, expected, expected_runtime)
            );
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("inspect-focus-window") {
        let process = std::env::args()
            .nth(2)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let window = std::env::args().nth(3).and_then(|v| v.parse().ok());
        if process != 0 && window.is_some() {
            println!("{}", focus_metadata_at(process, window));
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("inspect-focus-watch") {
        let process = std::env::args()
            .nth(2)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if process == 0 {
            return;
        }
        // Opt-in, metadata-only observation. Do not move foreground or read text.
        for _ in 0..60 {
            let mut foreground = 0;
            unsafe {
                GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut foreground));
            }
            if foreground == process {
                println!("{}", focus_metadata(process));
                let _ = io::stdout().flush();
            }
            thread::sleep(std::time::Duration::from_millis(500));
        }
        return;
    }
    if std::env::args().nth(1).as_deref() == Some("inspect-product") {
        let process = std::env::args()
            .nth(2)
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        println!("{}", product_surface(process));
        return;
    }
    unsafe {
        let composition = std::env::args().nth(1).as_deref() == Some("composition");
        let mut tsf_manager = None;
        let mut rich_library = None;
        use windows::Win32::System::Com::{
            COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize,
        };
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap();
        if composition {
            use windows::Win32::System::LibraryLoader::*;
            rich_library = Some(
                LoadLibraryExW(w!("Msftedit.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32).unwrap(),
            );
        }
        use windows::Win32::UI::HiDpi::{
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
        };
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let class = WNDCLASSW {
            lpfnWndProc: Some(fixture_proc),
            lpszClassName: w!("SayweaveInputFixture"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let top = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("SayweaveInputFixture"),
            w!("Sayweave isolated input"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            30,
            30,
            480,
            260,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let editor = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            if composition {
                w!("RICHEDIT50W")
            } else {
                w!("EDIT")
            },
            w!(""),
            WS_CHILD | WS_VISIBLE | WS_TABSTOP | WINDOW_STYLE(ES_MULTILINE as u32),
            10,
            10,
            440,
            180,
            Some(top),
            None,
            None,
            None,
        )
        .unwrap();
        if composition {
            use windows::Win32::{System::Com::*, UI::TextServices::*};
            let manager: ITfThreadMgr =
                CoCreateInstance(&CLSID_TF_ThreadMgr, None, CLSCTX_INPROC_SERVER).unwrap();
            manager.Activate().unwrap();
            message(editor, WM_USER + 204, WPARAM(0x10000), LPARAM(0x10000));
            tsf_manager = Some(manager);
        }
        let (t, r) = mpsc::channel::<serde_json::Value>();
        let (ok, ready) = mpsc::channel::<()>();
        let h = top.0 as usize;
        thread::spawn(move || {
            for line in io::stdin().lock().lines() {
                let Ok(line) = line else {
                    break;
                };
                let Ok(value) = serde_json::from_str(&line) else {
                    break;
                };
                if t.send(value).is_err() {
                    break;
                }
                let _ = PostMessageW(Some(HWND(h as *mut _)), WM_APP + 7, WPARAM(0), LPARAM(0));
                if ready.recv().is_err() {
                    break;
                }
            }
        });
        println!(
            "{{\"window\":{},\"focus\":{}}}",
            top.0 as usize, editor.0 as usize
        );
        io::stdout().flush().unwrap();
        let mut msg = MSG::default();
        let mut injected_keys = Vec::new();
        let mut auxiliary = HWND::default();
        let mut auxiliary_editor = HWND::default();
        let mut sibling = HWND::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_APP + 7 {
                let value = r.recv().unwrap();
                if value["action"] == "exit" {
                    if !auxiliary.0.is_null() {
                        let _ = DestroyWindow(auxiliary);
                    }
                    let _ = DestroyWindow(top);
                    PostQuitMessage(0);
                    let _ = ok.send(());
                    break;
                }
                if value["action"] == "inspect-context" {
                    println!("{}", context_status());
                    io::stdout().flush().unwrap();
                    let _ = ok.send(());
                    continue;
                }
                if value["action"] == "parent-write-probe" {
                    let result = if GetForegroundWindow() == top && GetFocus() == editor {
                        parent_write_probe()
                    } else {
                        serde_json::json!({"error":"ownedForegroundRequired"})
                    };
                    println!("{result}");
                    io::stdout().flush().unwrap();
                    let _ = ok.send(());
                    continue;
                }
                if value["action"] == "terminate-composition" {
                    use windows::{Win32::UI::TextServices::*, core::Interface};
                    let result = (|| -> windows::core::Result<()> {
                        if GetForegroundWindow() != top || GetFocus() != editor {
                            return Err(E_ACCESSDENIED.into());
                        }
                        let context = tsf_manager
                            .as_ref()
                            .ok_or_else(|| windows::core::Error::from_hresult(E_NOINTERFACE))?
                            .GetFocus()?
                            .GetTop()?;
                        let owner: ITfContextOwnerCompositionServices = context.cast()?;
                        // The host's real TSF operation, on its own STA/context.
                        // No application/proxy test hook and no fabricated reply.
                        owner.TerminateComposition(None)
                    })();
                    println!(
                        "{}",
                        serde_json::json!({"terminated":result.is_ok(),"error":result.err().map(|e|format!("{:08x}",e.code().0 as u32))})
                    );
                    io::stdout().flush().unwrap();
                    let _ = ok.send(());
                    continue;
                }
                if value["action"] == "inspect-selection" {
                    let process = value["process"]
                        .as_u64()
                        .and_then(|p| u32::try_from(p).ok())
                        .unwrap_or(0);
                    let mut foreground = 0;
                    GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut foreground));
                    let evidence = if process != 0 && foreground == process {
                        selection_evidence(process)
                    } else {
                        serde_json::json!({"error":"ownedForegroundMismatch"})
                    };
                    println!("{evidence}");
                    io::stdout().flush().unwrap();
                    let _ = ok.send(());
                    continue;
                }
                if value["action"] == "inspect-focus" {
                    let process = value["process"]
                        .as_u64()
                        .and_then(|p| u32::try_from(p).ok())
                        .unwrap_or(0);
                    println!("{}", focus_metadata(process));
                    io::stdout().flush().unwrap();
                    let _ = ok.send(());
                    continue;
                }
                if value["action"] == "inspect-product" {
                    let process = value["process"]
                        .as_u64()
                        .and_then(|p| u32::try_from(p).ok())
                        .unwrap_or(0);
                    println!("{}", product_surface(process));
                    io::stdout().flush().unwrap();
                    let _ = ok.send(());
                    continue;
                }
                let mut auxiliary_result = None;
                let mut key_result = None;
                if matches!(
                    value["action"].as_str(),
                    Some("auxiliary-info" | "auxiliary-notice" | "auxiliary-activate")
                ) {
                    let result = (|| -> windows::core::Result<()> {
                        if GetForegroundWindow() != top || GetFocus() != editor {
                            return Err(E_ACCESSDENIED.into());
                        }
                        if !IsWindow(Some(auxiliary)).as_bool() {
                            auxiliary = CreateWindowExW(
                                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                                w!("SayweaveInputFixture"),
                                w!("Owned auxiliary fixture"),
                                WS_POPUP,
                                530,
                                30,
                                200,
                                80,
                                None,
                                None,
                                None,
                                None,
                            )?;
                            auxiliary_editor = CreateWindowExW(
                                WINDOW_EX_STYLE(0),
                                w!("EDIT"),
                                w!("owned helper sentinel"),
                                WS_CHILD | WS_VISIBLE,
                                4,
                                4,
                                190,
                                60,
                                Some(auxiliary),
                                None,
                                None,
                                None,
                            )?;
                            let _ = ShowWindow(auxiliary, SW_SHOWNOACTIVATE);
                        }
                        if value["action"] == "auxiliary-notice" {
                            use windows::Win32::UI::Accessibility::NotifyWinEvent;
                            // An explicit synthetic accessibility notification,
                            // not an assertion that this window became foreground.
                            NotifyWinEvent(EVENT_SYSTEM_FOREGROUND, auxiliary, OBJID_WINDOW.0, 0);
                        } else if value["action"] == "auxiliary-activate" {
                            let _ = SetForegroundWindow(auxiliary);
                            let _ = SetFocus(Some(auxiliary_editor));
                            if GetForegroundWindow() != auxiliary {
                                return Err(E_FAIL.into());
                            }
                        }
                        Ok(())
                    })();
                    let mut text = [0u16; 256];
                    let size = GetWindowTextW(auxiliary_editor, &mut text) as usize;
                    let style = GetWindowLongPtrW(auxiliary, GWL_EXSTYLE) as u32;
                    auxiliary_result = Some(serde_json::json!({
                        "window": auxiliary.0 as usize,
                        "text": String::from_utf16_lossy(&text[..size]),
                        "noActivate": style & WS_EX_NOACTIVATE.0 != 0,
                        "toolWindow": style & WS_EX_TOOLWINDOW.0 != 0,
                        "foreground": GetForegroundWindow() == auxiliary,
                        "error": result.err().map(|error| format!("{:08x}", error.code().0 as u32))
                    }));
                }
                if value["action"] == "auxiliary-read" {
                    let mut text = [0u16; 256];
                    let size = GetWindowTextW(auxiliary_editor, &mut text) as usize;
                    auxiliary_result = Some(serde_json::json!({
                        "window": auxiliary.0 as usize,
                        "text": String::from_utf16_lossy(&text[..size]),
                        "foreground": GetForegroundWindow() == auxiliary
                    }));
                }
                if value["action"] != "type"
                    && let Some(text) = value["text"].as_str()
                {
                    let wide: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
                    let _ = message(
                        editor,
                        WM_SETTEXT,
                        WPARAM(0),
                        LPARAM(wide.as_ptr() as isize),
                    );
                    let _ = message(
                        editor,
                        0x00B1,
                        WPARAM(value["start"].as_u64().unwrap_or(0) as usize),
                        LPARAM(value["end"].as_i64().unwrap_or(0) as isize),
                    );
                }
                if value["action"] == "readonly" {
                    let _ = message(
                        editor,
                        0x00CF,
                        WPARAM(usize::from(value["enabled"].as_bool().unwrap_or(true))),
                        LPARAM(0),
                    );
                }
                if value["action"] == "selection" {
                    let _ = message(
                        editor,
                        0x00B1,
                        WPARAM(value["start"].as_u64().unwrap_or(0) as usize),
                        LPARAM(value["end"].as_i64().unwrap_or(0) as isize),
                    );
                }
                if value["action"] == "caret-home" && GetForegroundWindow() == top {
                    // EM_SETSEL can be ignored while TSF owns a composition. Exercise
                    // the editor's normal caret navigation and assert its real selection.
                    message(editor, WM_KEYDOWN, WPARAM(VK_HOME.0 as usize), LPARAM(0));
                    message(editor, WM_KEYUP, WPARAM(VK_HOME.0 as usize), LPARAM(0));
                }
                if value["action"] == "type"
                    && let Some(text) = value["text"].as_str()
                {
                    // Test-only foreign edit, delivered to this owned target's message loop.
                    for unit in text.encode_utf16() {
                        message(editor, WM_CHAR, WPARAM(unit as usize), LPARAM(0));
                    }
                }
                let mut controlled_edit = None;
                let mut controlled_edit_steps = Vec::new();
                if value["action"] == "controlled-edit" {
                    // Fixed, owned-target edit for the SessionDraft acceptance test.
                    // Dispatch TSF before RichEdit's two Backspace key messages;
                    // only the fixed X is delivered with WM_CHAR.
                    // this is neither physical input nor a main-program hook test.
                    let result = (|| -> windows::core::Result<()> {
                        use windows::{Win32::UI::TextServices::*, core::Interface};
                        if GetForegroundWindow() != top || GetFocus() != editor {
                            return Err(E_ACCESSDENIED.into());
                        }
                        let manager = tsf_manager.as_ref().ok_or(E_FAIL)?;
                        let keys: ITfKeystrokeMgr = manager.cast()?;
                        for step in 1..=2 {
                            if GetForegroundWindow() != top || GetFocus() != editor {
                                return Err(E_ACCESSDENIED.into());
                            }
                            if keys
                                .TestKeyDown(WPARAM(VK_BACK.0 as usize), LPARAM(1))?
                                .as_bool()
                                && keys
                                    .KeyDown(WPARAM(VK_BACK.0 as usize), LPARAM(1))?
                                    .as_bool()
                            {
                                return Err(E_FAIL.into());
                            }
                            message(editor, WM_KEYDOWN, WPARAM(VK_BACK.0 as usize), LPARAM(1));
                            if keys
                                .TestKeyUp(WPARAM(VK_BACK.0 as usize), LPARAM(1))?
                                .as_bool()
                            {
                                let _ = keys.KeyUp(WPARAM(VK_BACK.0 as usize), LPARAM(1))?;
                            }
                            message(editor, WM_KEYUP, WPARAM(VK_BACK.0 as usize), LPARAM(1));
                            let mut text = [0u16; 4096];
                            let size = GetWindowTextW(editor, &mut text) as usize;
                            let selected = message(editor, 0x00B0, WPARAM(0), LPARAM(0)).0 as usize;
                            controlled_edit_steps.push(serde_json::json!({
                                "step": step,
                                "text": String::from_utf16_lossy(&text[..size]),
                                "start": selected & 65535,
                                "end": (selected >> 16) & 65535,
                                "foreground": GetForegroundWindow() == top
                            }));
                        }
                        if GetForegroundWindow() != top || GetFocus() != editor {
                            return Err(E_ACCESSDENIED.into());
                        }
                        message(editor, WM_CHAR, WPARAM('X' as usize), LPARAM(0));
                        Ok(())
                    })();
                    controlled_edit = Some(match result {
                        Ok(()) => "completed".to_string(),
                        Err(error) => format!("failed:{:08x}", error.code().0 as u32),
                    });
                }
                if value["action"] == "fixture-text" {
                    // Match fixture-key's acceptance tag so the real edit
                    // observer sees test text as well as navigation/deletion.
                    let mut pid = 0;
                    GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
                    let text = value["text"].as_str().unwrap_or_default();
                    let units: Vec<u16> = text.encode_utf16().collect();
                    let allowed = value["process"].as_u64() == Some(pid as u64)
                        && !units.is_empty()
                        && units.len() <= 8
                        && !text.chars().any(char::is_control);
                    let mut sent = allowed;
                    if allowed {
                        for unit in &units {
                            let event = |up| INPUT {
                                r#type: INPUT_KEYBOARD,
                                Anonymous: INPUT_0 {
                                    ki: KEYBDINPUT {
                                        wScan: *unit,
                                        dwFlags: KEYEVENTF_UNICODE
                                            | if up {
                                                KEYEVENTF_KEYUP
                                            } else {
                                                KEYBD_EVENT_FLAGS(0)
                                            },
                                        dwExtraInfo: 0x53595754,
                                        ..Default::default()
                                    },
                                },
                            };
                            let count = SendInput(
                                &[event(false), event(true)],
                                std::mem::size_of::<INPUT>() as i32,
                            );
                            if count != 2 {
                                if count == 1 {
                                    let _ = SendInput(
                                        &[event(true)],
                                        std::mem::size_of::<INPUT>() as i32,
                                    );
                                }
                                sent = false;
                                break;
                            }
                        }
                    }
                    key_result = Some(
                        serde_json::json!({"accepted":sent,"process":pid,"unicodeUnits":units.len()}),
                    );
                }
                if value["action"] == "fixture-key" {
                    let mut pid = 0;
                    GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut pid));
                    let key = match value["key"].as_str() {
                        Some("AltRight") => Some(VK_RMENU),
                        Some("Backspace") => Some(VK_BACK),
                        Some("Delete") => Some(VK_DELETE),
                        Some("ArrowLeft") => Some(VK_LEFT),
                        Some("End") => Some(VK_END),
                        Some("ShiftLeft") => Some(VK_LSHIFT),
                        Some("ControlLeft") => Some(VK_LCONTROL),
                        Some("ShiftRight") => Some(VK_RSHIFT),
                        Some("Escape") => Some(VK_ESCAPE),
                        Some("Enter") => Some(VK_RETURN),
                        Some("Space") => Some(VK_SPACE),
                        Some(key) if key.len() == 4 && key.starts_with("Key") => key
                            .as_bytes()
                            .get(3)
                            .filter(|key| key.is_ascii_uppercase())
                            .map(|key| VIRTUAL_KEY(*key as u16)),
                        _ => None,
                    };
                    if let Some(key) = key {
                        let down = value["down"].as_bool() == Some(true);
                        let allowed = if down {
                            value["process"].as_u64() == Some(pid as u64)
                        } else {
                            injected_keys.contains(&key)
                        };
                        let flags = if value["down"].as_bool() == Some(true) {
                            KEYBD_EVENT_FLAGS(0)
                        } else {
                            KEYEVENTF_KEYUP
                        } | if key == VK_RMENU
                            || key == VK_DELETE
                            || key == VK_LEFT
                            || key == VK_END
                        {
                            KEYEVENTF_EXTENDEDKEY
                        } else {
                            KEYBD_EVENT_FLAGS(0)
                        };
                        let event = INPUT {
                            r#type: INPUT_KEYBOARD,
                            Anonymous: INPUT_0 {
                                ki: KEYBDINPUT {
                                    wVk: key,
                                    dwFlags: flags,
                                    dwExtraInfo: 0x53595754,
                                    ..Default::default()
                                },
                            },
                        };
                        let sent = allowed
                            && SendInput(&[event], std::mem::size_of::<INPUT>() as i32) == 1;
                        key_result = Some(
                            serde_json::json!({"accepted":sent,"process":pid,"key":key.0,"down":down}),
                        );
                        if sent {
                            if down && !injected_keys.contains(&key) {
                                injected_keys.push(key);
                            }
                            if !down {
                                injected_keys.retain(|held| *held != key);
                            }
                        }
                    }
                }
                if value["action"] == "delayClipboard" {
                    use windows::Win32::System::DataExchange::*;
                    if OpenClipboard(Some(top)).is_ok() {
                        let _ = EmptyClipboard();
                        let name: Vec<u16> =
                            "Rich Text Format".encode_utf16().chain(Some(0)).collect();
                        let kind = RegisterClipboardFormatW(windows::core::PCWSTR(name.as_ptr()));
                        let _ = SetClipboardData(kind, None);
                        let _ = CloseClipboard();
                    }
                }
                if value["action"] == "focus-desktop" {
                    let desktop = GetShellWindow();
                    if !desktop.0.is_null() {
                        focus_window(desktop);
                    }
                }
                if value["action"] == "focus" {
                    // Only this owned test fixture may establish focus; product delivery never does so.
                    focus_window(top);
                    let _ = SetFocus(Some(editor));
                }
                if value["action"] == "focus-sibling" && GetForegroundWindow() == top {
                    if sibling.0.is_null() {
                        sibling = CreateWindowExW(
                            WINDOW_EX_STYLE(0),
                            w!("EDIT"),
                            w!(""),
                            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
                            20,
                            80,
                            400,
                            80,
                            Some(top),
                            None,
                            None,
                            None,
                        )
                        .unwrap();
                        let mut text = [0u16; 4096];
                        GetWindowTextW(editor, &mut text);
                        message(
                            sibling,
                            WM_SETTEXT,
                            WPARAM(0),
                            LPARAM(text.as_ptr() as isize),
                        );
                    }
                    let _ = SetFocus(Some(sibling));
                }
                // The test runner passes the PID of its own launched browser/editor. This command
                // is absent from production and never receives a personal application's identity.
                if value["action"] == "focus-process"
                    && let Some(process) = value["process"]
                        .as_u64()
                        .and_then(|p| u32::try_from(p).ok())
                {
                    let mut target = ProcessWindow {
                        process,
                        window: HWND::default(),
                        overlay: false,
                    };
                    let _ =
                        EnumWindows(Some(process_window), LPARAM(&mut target as *mut _ as isize));
                    if !target.window.0.is_null() {
                        focus_window(target.window);
                    }
                }
                let mut clicked = None;
                let mut click_check = None;
                let mut overlay_window = None;
                if value["action"] == "inspect-overlay" {
                    let mut target = ProcessWindow {
                        process: value["process"].as_u64().unwrap_or(0) as u32,
                        window: HWND::default(),
                        overlay: true,
                    };
                    let _ =
                        EnumWindows(Some(process_window), LPARAM(&mut target as *mut _ as isize));
                    if !target.window.0.is_null() {
                        overlay_window = Some(target.window.0 as usize);
                    }
                }
                if value["action"] == "click-window" {
                    let window = HWND(value["window"].as_u64().unwrap_or(0) as usize as *mut _);
                    let mut process = 0;
                    GetWindowThreadProcessId(window, Some(&mut process));
                    if process == value["process"].as_u64().unwrap_or(0) as u32
                        && IsWindow(Some(window)).as_bool()
                    {
                        let mut point = POINT {
                            x: value["x"].as_i64().unwrap_or(0) as i32,
                            y: value["y"].as_i64().unwrap_or(0) as i32,
                        };
                        use windows::Win32::Graphics::Gdi::ClientToScreen;
                        let converted = ClientToScreen(window, &mut point).as_bool();
                        let mut client = RECT::default();
                        let _ = GetClientRect(window, &mut client);
                        click_check = Some(
                            serde_json::json!({"converted":converted,"width":client.right,"height":client.bottom,"pointRootMatches":GetAncestor(WindowFromPoint(point), GA_ROOT) == window,"x":point.x,"y":point.y}),
                        );
                        if converted && GetAncestor(WindowFromPoint(point), GA_ROOT) == window {
                            let mut old = POINT::default();
                            let _ = GetCursorPos(&mut old);
                            let _ = SetCursorPos(point.x, point.y);
                            let events =
                                [MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP].map(|flags| INPUT {
                                    r#type: INPUT_MOUSE,
                                    Anonymous: INPUT_0 {
                                        mi: MOUSEINPUT {
                                            dwFlags: flags,
                                            ..Default::default()
                                        },
                                    },
                                });
                            clicked = Some(SendInput(&events, std::mem::size_of::<INPUT>() as i32));
                            let _ = SetCursorPos(old.x, old.y);
                        }
                    }
                }
                let mut foreground_process = 0;
                GetWindowThreadProcessId(GetForegroundWindow(), Some(&mut foreground_process));
                let mut buf = [0u16; 4096];
                let len = GetWindowTextW(editor, &mut buf) as usize;
                let selection = message(editor, 0x00B0, WPARAM(0), LPARAM(0)).0 as usize;
                let sibling_value = if sibling.0.is_null() {
                    None
                } else {
                    let mut text = [0u16; 4096];
                    let size = GetWindowTextW(sibling, &mut text) as usize;
                    Some(
                        serde_json::json!({"window":sibling.0 as usize,"focused":GetFocus()==sibling,"text":String::from_utf16_lossy(&text[..size])}),
                    )
                };
                println!(
                    "{}",
                    serde_json::json!({"foreground":GetForegroundWindow()==top,"foregroundProcess":foreground_process,"foregroundWindow":GetForegroundWindow().0 as usize,"nativeFocus":GetFocus().0 as usize,"sibling":sibling_value,"desktopWindow":GetShellWindow().0 as usize,"clipboardSequence":windows::Win32::System::DataExchange::GetClipboardSequenceNumber(),"text":String::from_utf16_lossy(&buf[..len]),"start":selection&65535,"end":(selection>>16)&65535,"clicked":clicked,"clickCheck":click_check,"overlayWindow":overlay_window,"controlledEdit":controlled_edit,"controlledEditSteps":controlled_edit_steps,"auxiliary":auxiliary_result,"keyResult":key_result})
                );
                io::stdout().flush().unwrap();
                let _ = ok.send(());
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let mut release_failed = false;
        for key in injected_keys {
            let event = INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: key,
                        dwFlags: KEYEVENTF_KEYUP
                            | if key == VK_RMENU || key == VK_DELETE {
                                KEYEVENTF_EXTENDEDKEY
                            } else {
                                KEYBD_EVENT_FLAGS(0)
                            },
                        dwExtraInfo: 0x53595754,
                        ..Default::default()
                    },
                },
            };
            if SendInput(&[event], std::mem::size_of::<INPUT>() as i32) != 1 {
                eprintln!("owned fixture key release failed");
                release_failed = true;
            }
        }
        if let Some(manager) = tsf_manager {
            let _ = manager.Deactivate();
            drop(manager);
        }
        if let Some(library) = rich_library {
            let _ = FreeLibrary(library);
        }
        CoUninitialize();
        if release_failed {
            std::process::exit(1);
        }
    }
}
