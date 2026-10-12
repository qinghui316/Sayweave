//! Optional read-only output observation, independent of composition ownership.
//! COM ranges and sinks never leave their target apartment.
use super::{Callback, Lease, range_text};
#[path = "native_observation.rs"]
mod native_observation;
#[cfg(feature = "acceptance")]
pub(super) fn native_read_count() -> u64 {
    native_observation::read_count()
}
use crate::wire::{ObservedEdit, Request};
use std::{cell::RefCell, rc::Rc};
use windows::{
    Win32::{
        Foundation::E_POINTER, System::SystemInformation::GetTickCount64, UI::TextServices::*,
    },
    core::{Interface, Ref, implement},
};
struct Reading {
    value: ObservedEdit,
    start: u64,
    changed: u64,
    valid: bool,
    continuation: bool,
    epoch: u64,
    original: Option<Rc<str>>,
    end_reason: &'static str,
}
impl Reading {
    // A host may clear its field after submitting. Keep only the latest
    // completed, already-verified edit; emptying itself grants no evidence.
    fn freeze_before_clear(&mut self) -> bool {
        if !self.valid
            || self.value.composing
            || self.value.sequence == 0
            || self.value.after.is_empty()
            || self.value.before == self.value.after
            || self.original.as_deref().is_some_and(|original| {
                crate::composition_text::equivalent_native_text(original, &self.value.after)
            })
        {
            return false;
        }
        self.value.ended = true;
        self.continuation = false;
        self.end_reason = "fieldCleared";
        true
    }
}
unsafe fn field_is_empty(context: &ITfContext, ec: u32) -> windows::core::Result<bool> {
    unsafe {
        if context
            .GetStart(ec)?
            .CompareStart(ec, &context.GetEnd(ec)?, TF_ANCHOR_START)?
            == 0
        {
            return Ok(true);
        }
        // RichEdit can retain a structural terminal after its visible text is
        // empty. Its own native length is an independent metadata proof. A
        // browser/top-level window's zero caption length is never a field proof.
        use windows::Win32::{
            System::Threading::GetCurrentProcessId,
            UI::WindowsAndMessaging::{
                GetClassNameW, GetWindowTextLengthW, GetWindowThreadProcessId,
            },
        };
        let window = context.GetActiveView()?.GetWnd()?;
        let mut process = 0;
        GetWindowThreadProcessId(window, Some(&mut process));
        let mut buffer = [0u16; 64];
        let count = GetClassNameW(window, &mut buffer);
        let name = String::from_utf16_lossy(&buffer[..count.max(0) as usize]).to_ascii_uppercase();
        Ok(process == GetCurrentProcessId()
            && (name == "EDIT" || name.starts_with("RICHEDIT"))
            && GetWindowTextLengthW(window) == 0)
    }
}
pub(super) struct Watch {
    field: super::initial_selection::FocusIdentity,
    context: ITfContext,
    data: Rc<RefCell<Reading>>,
    cookie: Option<u32>,
    notifications: Option<Rc<super::TextNotifications>>,
    _native: Option<native_observation::BoundEdit>,
}
impl Drop for Watch {
    fn drop(&mut self) {
        // Providers may call the sink again during/after Unadvise. Retire its
        // read permission before entering COM or removing the native subclass.
        let mut data = self.data.borrow_mut();
        data.valid = false;
        data.value.ended = true;
        data.epoch = data.epoch.wrapping_add(1);
        drop(data);
        unsafe {
            if let Some(cookie) = self.cookie {
                if let Some(notifications) = &self.notifications {
                    notifications.retire(self.context.clone(), cookie);
                } else if let Ok(source) = self.context.cast::<ITfSource>() {
                    let _ = source.UnadviseSink(cookie);
                }
            }
        }
    }
}
impl Watch {
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn attach(
        context: &ITfContext,
        range: &ITfRange,
        client: u32,
        request: &Request,
        ec: u32,
        completed_tick: u64,
        field: super::initial_selection::FocusIdentity,
        original: Option<Rc<str>>,
        notifications: Rc<super::TextNotifications>,
    ) -> windows::core::Result<Self> {
        unsafe {
            if context.GetStatus()?.dwDynamicFlags & TF_SD_READONLY != 0
                || super::password(context, range, ec)?
            {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_ACCESSDENIED,
                ));
            }
            let before = range_text(range, ec)?;
            let now = completed_tick;
            let range = range.Clone()?;
            // Appending another ASR tail at this boundary must stay outside this receipt.
            range.SetGravity(ec, TF_GRAVITY_BACKWARD, TF_GRAVITY_BACKWARD)?;
            let data = Rc::new(RefCell::new(Reading {
                value: ObservedEdit {
                    engine: request.engine.clone(),
                    task: request.task.clone(),
                    turn: request.turn,
                    operation: request.operation,
                    revision: request.revision,
                    sequence: 0,
                    changed_tick: 0,
                    before: before.clone(),
                    after: before,
                    ended: false,
                    composing: false,
                    region: if request.action == crate::wire::Action::Settle {
                        crate::wire::OutputRegion::Prefix
                    } else {
                        crate::wire::OutputRegion::Tail
                    },
                },
                start: now,
                changed: now,
                valid: true,
                continuation: false,
                epoch: 0,
                original,
                end_reason: "none",
            }));
            let sink: ITfTextEditSink = Sink {
                field: field.clone(),
                range,
                client,
                data: data.clone(),
                notifications: notifications.clone(),
                _lease: Lease::new(),
            }
            .into();
            let source: ITfSource = context.cast()?;
            let cookie = source.AdviseSink(&ITfTextEditSink::IID, &sink)?;
            if !field.matches().unwrap_or(false) {
                notifications.retire(context.clone(), cookie);
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_ACCESSDENIED,
                ));
            }
            Ok(Self {
                field,
                context: context.clone(),
                data,
                cookie: Some(cookie),
                notifications: Some(notifications),
                _native: None,
            })
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub(super) unsafe fn native(
        context: &ITfContext,
        position: (usize, u32),
        client: u32,
        request: &Request,
        expected: &str,
        completed_tick: u64,
        field: super::initial_selection::FocusIdentity,
        original: Option<Rc<str>>,
    ) -> windows::core::Result<Self> {
        unsafe {
            let (window, start) = position;
            let now = completed_tick;
            if context.GetActiveView()?.GetWnd()?.0 as usize != window
                || context.GetStatus()?.dwDynamicFlags & TF_SD_READONLY != 0
            {
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_ACCESSDENIED,
                ));
            }
            let data = Rc::new(RefCell::new(Reading {
                value: ObservedEdit {
                    engine: request.engine.clone(),
                    task: request.task.clone(),
                    turn: request.turn,
                    operation: request.operation,
                    revision: request.revision,
                    sequence: 0,
                    changed_tick: 0,
                    before: expected.into(),
                    after: expected.into(),
                    ended: false,
                    composing: false,
                    region: if request.action == crate::wire::Action::Settle {
                        crate::wire::OutputRegion::Prefix
                    } else {
                        crate::wire::OutputRegion::Tail
                    },
                },
                start: now,
                changed: now,
                valid: true,
                continuation: false,
                epoch: 0,
                original,
                end_reason: "none",
            }));
            let native = native_observation::BoundEdit::attach(
                windows::Win32::Foundation::HWND(window as *mut _),
                start,
                expected,
                data.clone(),
                context.clone(),
                client,
            )?;
            Ok(Self {
                field,
                context: context.clone(),
                data,
                cookie: None,
                notifications: None,
                _native: Some(native),
            })
        }
    }
    pub(super) unsafe fn snapshot(
        &self,
        manager: Option<&ITfThreadMgr>,
        stop: bool,
    ) -> Option<ObservedEdit> {
        unsafe {
            let focused = manager
                .and_then(|m| m.GetFocus().ok())
                .and_then(|m| m.GetTop().ok())
                .is_some_and(|c| c == self.context);
            let now = GetTickCount64();
            let field = self.field.matches();
            let complete = self._native.as_ref().is_none_or(|native| native.complete());
            let mut data = self.data.borrow_mut();
            if field.is_err() {
                data.valid = false;
                data.end_reason = "fieldUnavailable";
            }
            if data.value.before != data.value.after
                && data.original.as_deref().is_some_and(|original| {
                    crate::composition_text::equivalent_native_text(original, &data.value.after)
                })
            {
                // Restoring the known original selection is not new correction
                // evidence. This negative check never grants read/write ownership.
                data.valid = false;
                data.end_reason = "originalRestored";
            }
            if data.end_reason == "none" {
                data.end_reason = if stop {
                    "stopped"
                } else if field == Ok(false) {
                    "fieldChanged"
                } else if !focused {
                    "contextChanged"
                } else if now.saturating_sub(data.start) >= 60000 {
                    "expired"
                } else if now.saturating_sub(data.changed) >= 15000 {
                    "quiet"
                } else if !data.valid {
                    "invalidated"
                } else {
                    "none"
                };
            }
            data.value.ended |= stop
                || field == Ok(false)
                || !focused
                || now.saturating_sub(data.start) >= 60000
                || now.saturating_sub(data.changed) >= 15000;
            #[cfg(feature = "acceptance")]
            if data.value.ended {
                eprintln!(
                    "owned observation end stop={} field_match={} context_match={} age_ms={} quiet_ms={} valid={} sequence={}",
                    stop,
                    field == Ok(true),
                    focused,
                    now.saturating_sub(data.start),
                    now.saturating_sub(data.changed),
                    data.valid,
                    data.value.sequence
                );
            }
            (data.valid
                && complete
                && !data.value.composing
                && data.value.before != data.value.after)
                .then(|| data.value.clone())
        }
    }
    pub(super) fn ended(&self) -> bool {
        self.data.borrow().value.ended || !self.data.borrow().valid
    }
    pub(super) fn diagnostic(&self) -> (bool, bool, u64, &'static str) {
        let data = self.data.borrow();
        (
            data.valid,
            data.value.composing,
            data.value.sequence,
            data.end_reason,
        )
    }
}
#[implement(ITfTextEditSink)]
struct Sink {
    field: super::initial_selection::FocusIdentity,
    range: ITfRange,
    client: u32,
    data: Rc<RefCell<Reading>>,
    notifications: Rc<super::TextNotifications>,
    _lease: Lease,
}
impl ITfTextEditSink_Impl for Sink_Impl {
    fn OnEndEdit(
        &self,
        context: Ref<ITfContext>,
        ec: u32,
        record: Ref<ITfEditRecord>,
    ) -> windows::core::Result<()> {
        let _callback = Callback::enter();
        let _notification = super::TextNotification::enter(self.notifications.clone());
        #[cfg(feature = "acceptance")]
        super::observation_fixture_reentry()?;
        unsafe {
            let Some(context) = context.as_ref() else {
                return Ok(());
            };
            let (stopped, epoch) = {
                let mut data = self.data.borrow_mut();
                data.epoch += 1;
                (data.value.ended || !data.valid, data.epoch)
            };
            if stopped {
                #[cfg(feature = "acceptance")]
                eprintln!("owned observer callback stopped=true epoch={epoch}");
                return Ok(());
            }
            match context.InWriteSession(self.client) {
                Ok(own) if own.as_bool() => {
                    #[cfg(feature = "acceptance")]
                    eprintln!("owned observer callback own=true epoch={epoch}");
                    self.data.borrow_mut().continuation = false;
                    return Ok(());
                }
                Err(_) => {
                    let mut data = self.data.borrow_mut();
                    data.valid = false;
                    data.end_reason = "writeQueryFailed";
                    return Ok(());
                }
                _ => (),
            }
            match self.field.matches() {
                Ok(true) => (),
                Ok(false) => {
                    let mut data = self.data.borrow_mut();
                    data.value.ended = true;
                    data.end_reason = "fieldChanged";
                    return Ok(());
                }
                Err(_) => {
                    let mut data = self.data.borrow_mut();
                    data.valid = false;
                    data.end_reason = "fieldUnavailable";
                    return Ok(());
                }
            }
            let result = (|| -> windows::core::Result<(String, bool, bool, bool)> {
                if context.GetStatus()?.dwDynamicFlags & TF_SD_READONLY != 0
                    || super::password(context, &self.range, ec)?
                {
                    return Err(windows::core::Error::from_hresult(
                        windows::Win32::Foundation::E_ACCESSDENIED,
                    ));
                }
                let record = record
                    .as_ref()
                    .ok_or_else(|| windows::core::Error::from_hresult(E_POINTER))?;
                let changes = record.GetTextAndPropertyUpdates(TF_GTP_INCL_TEXT, &[])?;
                let mut ranges = [None];
                let mut count = 0;
                let mut changed_text = false;
                let mut exhausted = false;
                // Some hosts clear while ending composition and notify only a
                // property change. The field's own empty metadata remains the
                // proof; notification kind alone must not revive interim text.
                let cleared = field_is_empty(context, ec)?;
                for _ in 0..64 {
                    ranges[0] = None;
                    changes.Next(&mut ranges, &mut count)?;
                    if count == 0 {
                        exhausted = true;
                        break;
                    }
                    changed_text = true;
                    // Check whole-document structure, not just an empty receipt.
                    // This reads no unrelated text and does not infer a send key.
                    if cleared {
                        exhausted = true;
                        break;
                    }
                    let changed = ranges[0]
                        .as_ref()
                        .ok_or_else(|| windows::core::Error::from_hresult(E_POINTER))?;
                    if changed.CompareStart(ec, &self.range, TF_ANCHOR_END)? == 0
                        && changed.CompareEnd(ec, &self.range, TF_ANCHOR_END)? > 0
                    {
                        let caret = super::selection(context, ec)?;
                        let data = self.data.borrow();
                        let continuation = data.continuation;
                        let edited = data.value.sequence > 0;
                        drop(data);
                        if continuation
                            && caret.IsEmpty(ec)?.as_bool()
                            && caret.CompareStart(ec, changed, TF_ANCHOR_END)? == 0
                        {
                            self.range.ShiftEndToRange(ec, changed, TF_ANCHOR_END)?;
                        } else if edited {
                            return Err(windows::core::Error::from_hresult(
                                windows::Win32::Foundation::E_ACCESSDENIED,
                            ));
                        }
                    }
                    if changed.CompareStart(ec, &self.range, TF_ANCHOR_START)? < 0
                        && changed.CompareEnd(ec, &self.range, TF_ANCHOR_START)? > 0
                        || changed.CompareStart(ec, &self.range, TF_ANCHOR_END)? < 0
                            && changed.CompareEnd(ec, &self.range, TF_ANCHOR_END)? > 0
                    {
                        return Err(windows::core::Error::from_hresult(
                            windows::Win32::Foundation::E_ACCESSDENIED,
                        ));
                    }
                    if changed.CompareStart(ec, &self.range, TF_ANCHOR_START)? >= 0
                        && changed.CompareEnd(ec, &self.range, TF_ANCHOR_END)? <= 0
                    {
                        continue;
                    }
                }
                if !exhausted {
                    return Err(windows::core::Error::from_hresult(
                        windows::Win32::Foundation::E_ACCESSDENIED,
                    ));
                }
                // Native ranges track edits inside the receipt; unrelated text stays outside.
                let after = if cleared {
                    String::new()
                } else {
                    range_text(&self.range, ec)?
                };
                #[cfg(feature = "acceptance")]
                eprintln!(
                    "owned observer callback own=false epoch={epoch} changed_text={changed_text} after_units={}",
                    after.encode_utf16().count()
                );
                if !self.field.matches()? {
                    return Err(windows::core::Error::from_hresult(
                        windows::Win32::Foundation::E_ACCESSDENIED,
                    ));
                }
                let caret = super::selection(context, ec)?;
                let continuation = changed_text
                    && caret.IsEmpty(ec)?.as_bool()
                    && caret.CompareStart(ec, &self.range, TF_ANCHOR_END)? == 0;
                let owner: ITfContextComposition = context.cast()?;
                let mut compositions = [None];
                let mut fetched = 0;
                owner
                    .EnumCompositions()?
                    .Next(&mut compositions, &mut fetched)?;
                Ok((after, fetched != 0, continuation, cleared))
            })();
            let field = self.field.matches();
            let mut data = self.data.borrow_mut();
            if data.epoch != epoch || data.value.ended || !data.valid {
                return Ok(());
            }
            match field {
                Ok(true) => (),
                Ok(false) => {
                    data.value.ended = true;
                    data.end_reason = "fieldChanged";
                    return Ok(());
                }
                Err(_) => {
                    data.valid = false;
                    data.end_reason = "fieldUnavailable";
                    return Ok(());
                }
            }
            match result {
                Ok((after, composing, continuation, cleared)) => {
                    data.continuation = continuation;
                    // Both the previous captured edit and this clearing session
                    // must be complete; ending IME cannot promote an interim edit.
                    if (cleared || after.is_empty()) && !composing && data.freeze_before_clear() {
                        return Ok(());
                    }
                    data.value.composing = composing;
                    if after != data.value.after {
                        data.value.after = after;
                        data.value.sequence += 1;
                        data.changed = GetTickCount64();
                        data.value.changed_tick = data.changed;
                    }
                }
                Err(error) => {
                    #[cfg(feature = "acceptance")]
                    eprintln!(
                        "owned observer callback failed epoch={epoch} hr={:08x}",
                        error.code().0 as u32
                    );
                    let _ = error;
                    data.valid = false;
                    data.end_reason = "readFailed";
                }
            }
            Ok(())
        }
    }
}
