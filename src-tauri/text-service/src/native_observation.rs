//! A native EDIT receipt uses native UTF-16 coordinates and native edit messages.
//! It never interprets the IMM composition's shadow offsets as document offsets.
use super::{Lease, Reading};
use crate::composition_text::equivalent_native_text;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use windows::{
    Win32::{
        Foundation::*,
        System::{SystemInformation::GetTickCount64, Threading::*},
        UI::{Input::KeyboardAndMouse::GetFocus, TextServices::ITfContext, WindowsAndMessaging::*},
    },
    core::Error,
};
type Proc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM, usize, usize) -> LRESULT;
#[cfg(feature = "acceptance")]
static READS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "acceptance")]
pub(super) fn read_count() -> u64 {
    READS.load(std::sync::atomic::Ordering::Relaxed)
}
#[link(name = "comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(window: HWND, callback: Option<Proc>, id: usize, data: usize) -> i32;
    fn RemoveWindowSubclass(window: HWND, callback: Option<Proc>, id: usize) -> i32;
    fn DefSubclassProc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT;
}
struct Hook {
    reading: Rc<RefCell<Reading>>,
    context: ITfContext,
    client: u32,
    start: Cell<u32>,
    end: Cell<u32>,
    depth: Cell<u32>,
    incomplete: Cell<bool>,
    continuation: Cell<bool>,
    gone: Rc<Cell<bool>>,
    _lease: Lease,
}
pub(super) struct BoundEdit {
    window: HWND,
    raw: *mut Rc<Hook>,
    gone: Rc<Cell<bool>>,
}
impl Drop for BoundEdit {
    fn drop(&mut self) {
        unsafe {
            if !self.gone.get()
                && RemoveWindowSubclass(self.window, Some(procedure), self.raw as usize) != 0
            {
                self.gone.set(true);
                drop(Box::from_raw(self.raw));
            }
        }
    }
}
unsafe fn capable(window: HWND) -> bool {
    unsafe {
        let mut pid = 0;
        let thread = GetWindowThreadProcessId(window, Some(&mut pid));
        let style = GetWindowLongPtrW(window, GWL_STYLE) as u32;
        let mut class = [0u16; 64];
        let length = GetClassNameW(window, &mut class);
        IsWindow(Some(window)).as_bool()
            && pid == GetCurrentProcessId()
            && thread == GetCurrentThreadId()
            && GetFocus() == window
            && style & (ES_PASSWORD as u32 | ES_READONLY as u32) == 0
            && length > 0
            && String::from_utf16_lossy(&class[..length as usize]) == "Edit"
    }
}
unsafe fn length(window: HWND) -> u32 {
    unsafe { SendMessageW(window, WM_GETTEXTLENGTH, None, None).0.max(0) as u32 }
}
unsafe fn selection(window: HWND) -> (u32, u32) {
    unsafe {
        let (mut start, mut end) = (0u32, 0u32);
        SendMessageW(
            window,
            0x00B0,
            Some(WPARAM((&mut start as *mut u32) as usize)),
            Some(LPARAM((&mut end as *mut u32) as isize)),
        );
        (start, end)
    }
}
/// Line prefixes exist only in a bounded scratch buffer to address the span;
/// only the receipt's interval is returned, retained or sent to the host.
unsafe fn text(window: HWND, start: u32, end: u32) -> windows::core::Result<String> {
    #[cfg(feature = "acceptance")]
    READS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    unsafe {
        if end < start || end - start > 16384 || end > length(window) {
            return Err(Error::from_hresult(E_INVALIDARG));
        }
        if start == end {
            return Ok(String::new());
        }
        let first = SendMessageW(window, 0x00C9, Some(WPARAM(start as usize)), None).0;
        let last = SendMessageW(window, 0x00C9, Some(WPARAM((end - 1) as usize)), None).0;
        if first < 0 || last < first || last - first > 16384 {
            return Err(Error::from_hresult(E_FAIL));
        }
        let mut owned = Vec::with_capacity((end - start) as usize);
        for line in first..=last {
            let origin = SendMessageW(window, 0x00BB, Some(WPARAM(line as usize)), None).0;
            if origin < 0 {
                return Err(Error::from_hresult(E_FAIL));
            }
            let origin = origin as u32;
            let line_length = SendMessageW(window, 0x00C1, Some(WPARAM(origin as usize)), None)
                .0
                .max(0) as u32;
            let through = end.min(origin + line_length).saturating_sub(origin);
            if through > 65535 {
                return Err(Error::from_hresult(E_INVALIDARG));
            }
            if through > 0 {
                let mut buffer = vec![0u16; through as usize + 1];
                buffer[0] = through as u16;
                let read = SendMessageW(
                    window,
                    0x00C4,
                    Some(WPARAM(line as usize)),
                    Some(LPARAM(buffer.as_mut_ptr() as isize)),
                )
                .0
                .max(0) as u32;
                let from = start.saturating_sub(origin).min(through);
                if read < through {
                    return Err(Error::from_hresult(E_FAIL));
                }
                owned.extend_from_slice(&buffer[from as usize..through as usize]);
            }
            if line < last {
                let next = SendMessageW(window, 0x00BB, Some(WPARAM((line + 1) as usize)), None).0;
                let gap = next - (origin + line_length) as isize;
                if !(0..=2).contains(&gap) {
                    return Err(Error::from_hresult(E_FAIL));
                }
                let separator = if gap == 2 {
                    vec![13u16, 10]
                } else if gap == 1 {
                    vec![10u16]
                } else {
                    vec![]
                };
                for (index, unit) in separator.into_iter().enumerate() {
                    let at = origin + line_length + index as u32;
                    if at >= start && at < end {
                        owned.push(unit);
                    }
                }
            }
        }
        String::from_utf16(&owned).map_err(|_| Error::from_hresult(E_INVALIDARG))
    }
}
impl BoundEdit {
    pub(super) fn complete(&self) -> bool {
        unsafe {
            if self.gone.get() {
                return false;
            }
            let hook = (&*self.raw).clone();
            let reading = hook.reading.borrow();
            if !reading.valid {
                return false;
            }
            if reading.value.ended {
                return !hook.incomplete.get();
            }
            drop(reading);
            let style = GetWindowLongPtrW(self.window, GWL_STYLE) as u32;
            if style & (ES_PASSWORD as u32 | ES_READONLY as u32) != 0 {
                hook.reading.borrow_mut().valid = false;
                // Retire read permission before the span verification below.
                return false;
            }
            if GetFocus() != self.window {
                hook.reading.borrow_mut().value.ended = true;
            } else if !hook.incomplete.get()
                && !matches!(text(self.window,hook.start.get(),hook.end.get()),Ok(value) if value==hook.reading.borrow().value.after)
            {
                hook.reading.borrow_mut().valid = false;
            }
            !hook.incomplete.get()
        }
    }
    pub(super) unsafe fn attach(
        window: HWND,
        start: u32,
        expected: &str,
        reading: Rc<RefCell<Reading>>,
        context: ITfContext,
        client: u32,
    ) -> windows::core::Result<Self> {
        unsafe {
            if !capable(window) {
                return Err(Error::from_hresult(E_ACCESSDENIED));
            }
            let native = expected
                .replace("\r\n", "\n")
                .replace('\r', "\n")
                .replace('\n', "\r\n");
            let end = start
                .checked_add(native.encode_utf16().count() as u32)
                .ok_or_else(|| Error::from_hresult(E_INVALIDARG))?;
            let actual = text(window, start, end)?;
            if !equivalent_native_text(&actual, expected) {
                return Err(Error::from_hresult(E_FAIL));
            }
            {
                let mut data = reading.borrow_mut();
                data.value.before = actual.clone();
                data.value.after = actual;
            }
            let gone = Rc::new(Cell::new(false));
            let raw = Box::into_raw(Box::new(Rc::new(Hook {
                reading,
                context,
                client,
                start: Cell::new(start),
                end: Cell::new(end),
                depth: Cell::new(0),
                incomplete: Cell::new(false),
                continuation: Cell::new(false),
                gone: gone.clone(),
                _lease: Lease::new(),
            })));
            if SetWindowSubclass(window, Some(procedure), raw as usize, raw as usize) == 0 {
                drop(Box::from_raw(raw));
                return Err(Error::from_hresult(E_FAIL));
            }
            Ok(Self { window, raw, gone })
        }
    }
}
unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    unsafe {
        let hook = (*(data as *mut Rc<Hook>)).clone();
        if message == WM_NCDESTROY {
            hook.gone.set(true);
            hook.reading.borrow_mut().valid = false;
            RemoveWindowSubclass(window, Some(procedure), id);
            let owned = Box::from_raw(data as *mut Rc<Hook>);
            let result = DefSubclassProc(window, message, w, l);
            drop(owned);
            return result;
        }
        let reading = hook.reading.clone();
        let stopped = {
            let record = reading.borrow();
            !record.valid || record.value.ended
        };
        if message == 0x00B1
            || matches!(message, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_KILLFOCUS)
            || message == WM_KEYDOWN && matches!(w.0, 0x21..=0x28)
        {
            hook.continuation.set(false);
        }
        if message == WM_KILLFOCUS {
            reading.borrow_mut().value.ended = true;
        }
        if !stopped && matches!(message, WM_IME_STARTCOMPOSITION | WM_IME_ENDCOMPOSITION) {
            reading.borrow_mut().value.composing = message == WM_IME_STARTCOMPOSITION;
            reading.borrow_mut().changed = GetTickCount64();
        }
        let changing = matches!(
            message,
            WM_CHAR
                | WM_PASTE
                | WM_CUT
                | WM_CLEAR
                | WM_IME_COMPOSITION
                | WM_IME_CHAR
                | 0x00C2
                | WM_UNDO
                | 0x00C7
                | WM_SETTEXT
        ) || message == WM_KEYDOWN && w.0 == 0x2E;
        if stopped || !changing {
            return DefSubclassProc(window, message, w, l);
        }
        if !capable(window) {
            reading.borrow_mut().valid = false;
            return DefSubclassProc(window, message, w, l);
        }
        if hook.depth.get() != 0 {
            reading.borrow_mut().valid = false;
            return DefSubclassProc(window, message, w, l);
        }
        hook.depth.set(1);
        let own = match hook.context.InWriteSession(hook.client) {
            Ok(value) => value.as_bool(),
            Err(_) => {
                reading.borrow_mut().valid = false;
                hook.depth.set(0);
                return DefSubclassProc(window, message, w, l);
            }
        };
        let before_selection = selection(window);
        let before_length = length(window);
        let start = hook.start.get();
        let end = hook.end.get();
        if !hook.incomplete.get()
            && !matches!(text(window,start,end),Ok(value) if value==reading.borrow().value.after)
        {
            reading.borrow_mut().valid = false;
        }
        let result = DefSubclassProc(window, message, w, l);
        if hook.gone.get() {
            return result;
        }
        hook.depth.set(0);
        if !reading.borrow().valid {
            return result;
        }
        if own {
            hook.continuation.set(false);
            if !matches!(text(window,start,end),Ok(value) if value==reading.borrow().value.after) {
                reading.borrow_mut().valid = false;
            }
            return result;
        }
        let after_length = length(window);
        let delta = after_length as i64 - before_length as i64;
        if after_length == 0
            && !hook.incomplete.get()
            && message != WM_UNDO
            && message != 0x00C7
            && capable(window)
            && reading.borrow_mut().freeze_before_clear()
        {
            return result;
        }
        if message == WM_UNDO || message == 0x00C7 || message == WM_SETTEXT || !capable(window) {
            reading.borrow_mut().valid = false;
            return result;
        }
        let mut from = before_selection.0;
        let mut to = before_selection.1;
        if from == to && delta < 0 {
            if message == WM_CHAR && w.0 == 8 {
                from = (from as i64 + delta).max(0) as u32;
            } else if message == WM_KEYDOWN && w.0 == 0x2E {
                to = (to as i64 - delta) as u32;
            } else {
                reading.borrow_mut().valid = false;
                return result;
            }
        }
        if delta == 0 && from == to {
            return result;
        }
        let continuing = hook.continuation.get() && from == to && from == end;
        if to <= start && (from < start || from == to) && !continuing {
            hook.start.set((start as i64 + delta).max(0) as u32);
            hook.end.set((end as i64 + delta).max(0) as u32);
            return result;
        }
        if from >= end && !continuing {
            if from == end && reading.borrow().value.sequence > 0 {
                reading.borrow_mut().valid = false;
            }
            return result;
        }
        if from < start || to > end {
            reading.borrow_mut().valid = false;
            return result;
        }
        let next = (end as i64 + delta).max(start as i64) as u32;
        hook.end.set(next);
        hook.continuation.set(selection(window) == (next, next));
        if let Ok(after) = text(window, start, next) {
            hook.incomplete.set(false);
            let mut record = reading.borrow_mut();
            if after != record.value.after {
                record.value.after = after;
                record.value.sequence += 1;
                record.changed = GetTickCount64();
                record.value.changed_tick = record.changed;
            }
        } else if message == WM_CHAR && (0xd800..=0xdbff).contains(&w.0) {
            // A UTF-16 pair arrives in two messages; do not publish its interim scalar.
            hook.incomplete.set(true);
        } else {
            reading.borrow_mut().valid = false;
        }
        result
    }
}
