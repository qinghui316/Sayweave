//! All UIA interfaces are created, queried and released on the single reader MTA.
use super::{Anchor, LIMIT};
use std::time::{Duration, Instant};
use windows::{
    Win32::{System::Com::*, UI::Accessibility::*},
    core::Interface,
};

struct Apartment;
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}
pub(super) struct Reader {
    automation: IUIAutomation,
    _apartment: Apartment,
}
pub(super) struct Field {
    element: IUIAutomationElement,
}
pub(super) struct Snapshot {
    pub text: String,
    #[cfg(test)]
    pub composing: Option<bool>,
}
impl Reader {
    pub fn new() -> Option<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok().ok()?;
            let apartment = Apartment;
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).ok()?;
            if let Ok(options) = automation.cast::<IUIAutomation2>() {
                let _ = options.SetConnectionTimeout(50);
                let _ = options.SetTransactionTimeout(50);
            }
            Some(Self {
                automation,
                _apartment: apartment,
            })
        }
    }
    fn focused(&self, anchor: &Anchor) -> Result<IUIAutomationElement, &'static str> {
        super::super::native::learning_target(anchor).map_err(|_| "targetChanged")?;
        unsafe {
            let focused = self
                .automation
                .GetFocusedElement()
                .map_err(|_| "focusedElementUnavailable")?;
            let mut element = focused.clone();
            // TextChild is an explicit container relationship. No sibling or
            // window-wide search may select an unrelated editable field.
            if !text_capability(&element)
                && let Ok(child) = focused
                    .GetCurrentPatternAs::<IUIAutomationTextChildPattern>(UIA_TextChildPatternId)
            {
                element = child
                    .TextContainer()
                    .map_err(|_| "textContainerUnavailable")?;
            }
            if !text_capability(&element) {
                return Err("textUnavailable");
            }
            safety(&element, anchor)?;
            Ok(element)
        }
    }
    pub fn capture(&self, anchor: &Anchor) -> Result<Field, &'static str> {
        self.focused(anchor).map(|element| Field { element })
    }
    pub fn read(&self, anchor: &Anchor, field: &Field) -> Result<Snapshot, &'static str> {
        let began = Instant::now();
        #[cfg(test)]
        {
            static STALLED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if std::env::var("SAYWEAVE_ACTOR_LEARNING_SLOW").as_deref() == Ok("1")
                && !STALLED.swap(true, std::sync::atomic::Ordering::AcqRel)
            {
                eprintln!("owned_learning_read_stall=waiting");
                std::thread::sleep(Duration::from_secs(3));
                eprintln!("owned_learning_read_stall=finished");
            }
        }
        let check = || -> Result<(), &'static str> {
            if began.elapsed() >= Duration::from_millis(500) {
                return Err("readTimeout");
            }
            let current = self.focused(anchor)?;
            unsafe {
                if !self
                    .automation
                    .CompareElements(&current, &field.element)
                    .map_err(|_| "identityUnavailable")?
                    .as_bool()
                {
                    return Err("fieldChanged");
                }
                safety(&field.element, anchor)?;
            }
            Ok(())
        };
        check()?;
        let activity = crate::input_state::activity();
        #[cfg(test)]
        let composing = unsafe { composition(&field.element, anchor).ok() };
        let text = unsafe {
            if let Ok(pattern) = text_pattern(&field.element) {
                let range = pattern.DocumentRange().map_err(|_| "textUnavailable")?;
                let walker = self
                    .automation
                    .RawViewWalker()
                    .map_err(|_| "rangeOwnerUnknown")?;
                let mut owner = range
                    .GetEnclosingElement()
                    .map_err(|_| "rangeOwnerUnknown")?;
                let mut within = false;
                for _ in 0..16 {
                    if self
                        .automation
                        .CompareElements(&owner, &field.element)
                        .map_err(|_| "rangeOwnerUnknown")?
                        .as_bool()
                    {
                        within = true;
                        break;
                    }
                    if owner.CurrentProcessId().map_err(|_| "rangeOwnerUnknown")?
                        != anchor.editor.process as i32
                        || [UIA_WindowControlTypeId, UIA_DocumentControlTypeId].contains(
                            &owner
                                .CurrentControlType()
                                .map_err(|_| "rangeOwnerUnknown")?,
                        )
                    {
                        break;
                    }
                    owner = walker
                        .GetParentElement(&owner)
                        .map_err(|_| "rangeOwnerUnknown")?;
                }
                if !within {
                    return Err("rangeOutsideField");
                }
                let value = range
                    .GetText(LIMIT as i32 + 1)
                    .map_err(|_| "textUnavailable")?;
                if value.len() >= LIMIT {
                    return Err("incompleteText");
                }
                String::from_utf16(&value).map_err(|_| "invalidText")?
            } else {
                let value = field
                    .element
                    .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                    .map_err(|_| "textUnavailable")?
                    .CurrentValue()
                    .map_err(|_| "textUnavailable")?;
                if value.len() >= LIMIT {
                    return Err("incompleteText");
                }
                String::from_utf16(&value).map_err(|_| "invalidText")?
            }
        };
        if text.len() >= LIMIT {
            return Err("incompleteText");
        }
        check()?;
        #[cfg(test)]
        let composing_after = unsafe { composition(&field.element, anchor).ok() };
        if activity != crate::input_state::activity() {
            return Err("readRacedEdit");
        }
        Ok(Snapshot {
            text,
            #[cfg(test)]
            composing: composing
                .zip(composing_after)
                .map(|(before, after)| before || after),
        })
    }
}
fn text_capability(element: &IUIAutomationElement) -> bool {
    unsafe {
        text_pattern(element).is_ok()
            || element
                .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .is_ok()
    }
}
fn text_pattern(element: &IUIAutomationElement) -> windows::core::Result<IUIAutomationTextPattern> {
    unsafe {
        element
            .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            .or_else(|_| {
                element
                    .GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id)?
                    .cast()
            })
    }
}
unsafe fn safety(element: &IUIAutomationElement, anchor: &Anchor) -> Result<(), &'static str> {
    unsafe {
        if element
            .CurrentProcessId()
            .map_err(|_| "identityUnavailable")?
            != anchor.editor.process as i32
        {
            return Err("fieldProcessChanged");
        }
        if element
            .CurrentIsPassword()
            .map_err(|_| "safetyUnknown")?
            .as_bool()
        {
            return Err("password");
        }
        if !element
            .CurrentIsEnabled()
            .map_err(|_| "safetyUnknown")?
            .as_bool()
        {
            return Err("disabled");
        }
        let readonly = if let Ok(value) =
            element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        {
            value
                .CurrentIsReadOnly()
                .map_err(|_| "readonlyUnknown")?
                .as_bool()
        } else {
            let pattern = text_pattern(element).map_err(|_| "readonlyUnknown")?;
            let selected = pattern.GetSelection().map_err(|_| "readonlyUnknown")?;
            if selected.Length().map_err(|_| "readonlyUnknown")? != 1 {
                return Err("readonlyUnknown");
            }
            let value = selected
                .GetElement(0)
                .map_err(|_| "readonlyUnknown")?
                .GetAttributeValue(UIA_IsReadOnlyAttributeId)
                .map_err(|_| "readonlyUnknown")?;
            if value.vt() != windows::Win32::System::Variant::VT_BOOL {
                return Err("readonlyUnknown");
            }
            bool::try_from(&value).map_err(|_| "readonlyUnknown")?
        };
        if readonly {
            return Err("readonly");
        }
        Ok(())
    }
}
#[cfg(test)]
unsafe fn composition(
    element: &IUIAutomationElement,
    anchor: &Anchor,
) -> Result<bool, &'static str> {
    unsafe {
        if let Ok(edit) =
            element.GetCurrentPatternAs::<IUIAutomationTextEditPattern>(UIA_TextEditPatternId)
        {
            // S_OK with a null range explicitly means no active composition;
            // preserve that fact rather than treating every failed QI as false.
            let mut range = std::ptr::null_mut();
            (Interface::vtable(&edit).GetActiveComposition)(Interface::as_raw(&edit), &mut range)
                .ok()
                .map_err(|_| "compositionUnknown")?;
            if range.is_null() {
                return Ok(false);
            }
            drop(IUIAutomationTextRange::from_raw(range));
            return Ok(true);
        }
        super::super::native::learning_composition(anchor).ok_or("compositionUnknown")
    }
}
