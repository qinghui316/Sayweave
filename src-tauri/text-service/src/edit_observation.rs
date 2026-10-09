//! Optional read-only output observation, independent of composition ownership.
//! COM ranges and sinks never leave their target apartment.
use super::{Lease, range_text};
#[path = "native_observation.rs"]
mod native_observation;
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
}
pub(super) struct Watch {
    field: super::initial_selection::FocusIdentity,
    context: ITfContext,
    data: Rc<RefCell<Reading>>,
    cookie: Option<u32>,
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
            if let Some(cookie) = self.cookie
                && let Ok(source) = self.context.cast::<ITfSource>()
            {
                let _ = source.UnadviseSink(cookie);
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
            }));
            let sink: ITfTextEditSink = Sink {
                field: field.clone(),
                range,
                client,
                data: data.clone(),
                _lease: Lease::new(),
            }
            .into();
            let source: ITfSource = context.cast()?;
            let cookie = source.AdviseSink(&ITfTextEditSink::IID, &sink)?;
            if !field.matches().unwrap_or(false) {
                let _ = source.UnadviseSink(cookie);
                return Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_ACCESSDENIED,
                ));
            }
            Ok(Self {
                field,
                context: context.clone(),
                data,
                cookie: Some(cookie),
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
            }
            if data.value.before != data.value.after
                && data.original.as_deref().is_some_and(|original| {
                    crate::composition_text::equivalent_native_text(original, &data.value.after)
                })
            {
                // Restoring the known original selection is not new correction
                // evidence. This negative check never grants read/write ownership.
                data.valid = false;
            }
            data.value.ended |= stop
                || field == Ok(false)
                || !focused
                || now.saturating_sub(data.start) >= 60000
                || now.saturating_sub(data.changed) >= 15000;
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
}
#[implement(ITfTextEditSink)]
struct Sink {
    field: super::initial_selection::FocusIdentity,
    range: ITfRange,
    client: u32,
    data: Rc<RefCell<Reading>>,
    _lease: Lease,
}
impl ITfTextEditSink_Impl for Sink_Impl {
    fn OnEndEdit(
        &self,
        context: Ref<ITfContext>,
        ec: u32,
        record: Ref<ITfEditRecord>,
    ) -> windows::core::Result<()> {
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
                return Ok(());
            }
            match context.InWriteSession(self.client) {
                Ok(own) if own.as_bool() => {
                    self.data.borrow_mut().continuation = false;
                    return Ok(());
                }
                Err(_) => {
                    self.data.borrow_mut().valid = false;
                    return Ok(());
                }
                _ => (),
            }
            match self.field.matches() {
                Ok(true) => (),
                Ok(false) => {
                    self.data.borrow_mut().value.ended = true;
                    return Ok(());
                }
                Err(_) => {
                    self.data.borrow_mut().valid = false;
                    return Ok(());
                }
            }
            let result = (|| -> windows::core::Result<(String, bool, bool)> {
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
                for _ in 0..64 {
                    ranges[0] = None;
                    changes.Next(&mut ranges, &mut count)?;
                    if count == 0 {
                        exhausted = true;
                        break;
                    }
                    changed_text = true;
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
                let after = range_text(&self.range, ec)?;
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
                Ok((after, fetched != 0, continuation))
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
                    return Ok(());
                }
                Err(_) => {
                    data.valid = false;
                    return Ok(());
                }
            }
            match result {
                Ok((after, composing, continuation)) => {
                    data.continuation = continuation;
                    data.value.composing = composing;
                    if after != data.value.after {
                        data.value.after = after;
                        data.value.sequence += 1;
                        data.changed = GetTickCount64();
                        data.value.changed_tick = data.changed;
                    }
                }
                Err(_) => data.valid = false,
            }
            Ok(())
        }
    }
}
