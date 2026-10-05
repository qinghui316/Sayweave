//! Independent application observations. COM objects and private text never leave the MTA.
use super::{Anchor, Error};
use std::time::Instant;
use windows::Win32::UI::Accessibility::*;

macro_rules! call {
    ($r:expr, $value:expr) => {{
        live($r)?;
        let result = $value.map_err(|_| Error::new("targetProbeFailed"));
        live($r)?;
        result?
    }};
}
fn live(r: &Request) -> Result<(), Error> {
    if Instant::now() >= r.expires {
        Err(Error::new("targetTimeout"))
    } else {
        Ok(())
    }
}

pub(super) struct Request {
    pub task: String,
    pub turn: u64,
    pub anchor: Anchor,
    pub expires: Instant,
    pub action: Action,
}
pub(super) enum Action {
    Begin,
    Check(Option<String>),
    Confirm(String),
}
#[derive(Clone, PartialEq, Eq)]
struct Position {
    before: String,
    selected: String,
    after: String,
}
impl Position {
    fn replaced(&self, text: &str) -> Self {
        Self {
            before: self.before.clone() + &normalize(text),
            selected: String::new(),
            after: self.after.clone(),
        }
    }
}
fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}
struct Bound {
    task: String,
    turn: u64,
    capture: u64,
    element: IUIAutomationElement,
    initial: Position,
    observed: Position,
}
#[derive(Default)]
pub(super) struct Evidence(Option<Bound>);
impl Evidence {
    pub fn forget(&mut self, task: &str) {
        if self.0.as_ref().is_some_and(|p| p.task == task) {
            self.0 = None;
        }
    }
    pub fn execute(&mut self, auto: Option<&IUIAutomation>, r: Request) -> Result<bool, Error> {
        if Instant::now() >= r.expires {
            return Err(Error::new("targetTimeout"));
        }
        if !super::native::composition_target(&r.task, &r.anchor)? {
            return Err(Error::new("targetChanged"));
        }
        if matches!(r.action, Action::Begin) {
            self.0 = None;
            if let Some(auto) = auto {
                let (element, initial) = match observe(auto, &r) {
                    Ok(value) => value,
                    Err(error)
                        if matches!(
                            error.code.as_str(),
                            "targetProbeFailed" | "targetSelectionUnavailable" | "targetTextLimit"
                        ) =>
                    {
                        return Ok(false);
                    }
                    Err(error) => return Err(error),
                };
                self.0 = Some(Bound {
                    task: r.task,
                    turn: r.turn,
                    capture: r.anchor.capture_id,
                    element,
                    observed: initial.clone(),
                    initial,
                });
                return Ok(true);
            }
            return Ok(false); // TSF admission does not require UIA readback.
        }
        let Some(bound) = self
            .0
            .as_mut()
            .filter(|b| b.task == r.task && b.turn == r.turn && b.capture == r.anchor.capture_id)
        else {
            return Ok(false);
        };
        let auto = auto.ok_or_else(|| Error::new("targetProbeFailed"))?;
        let (element, actual) = observe(auto, &r)?;
        let same = call!(&r, unsafe {
            auto.CompareElements(&bound.element, &element)
        })
        .as_bool();
        if !same {
            return Err(Error::new("targetChanged"));
        }
        let expected = match &r.action {
            Action::Check(None) => bound.initial.clone(),
            Action::Check(Some(text)) | Action::Confirm(text) => bound.initial.replaced(text),
            Action::Begin => unreachable!(),
        };
        if actual != expected {
            log::info!(target:"sayweave", "operation=draft_evidence task_id={} turn={} phase={} before_match={} selection_match={} after_match={} actual_lengths={},{},{} expected_lengths={},{},{} actual_newlines={},{},{} expected_newlines={},{},{}", r.task,r.turn,if matches!(r.action,Action::Confirm(_)){"confirm"}else{"check"},actual.before==expected.before,actual.selected==expected.selected,actual.after==expected.after,actual.before.encode_utf16().count(),actual.selected.encode_utf16().count(),actual.after.encode_utf16().count(),expected.before.encode_utf16().count(),expected.selected.encode_utf16().count(),expected.after.encode_utf16().count(),actual.before.matches('\n').count(),actual.selected.matches('\n').count(),actual.after.matches('\n').count(),expected.before.matches('\n').count(),expected.selected.matches('\n').count(),expected.after.matches('\n').count());
        }
        if actual == expected {
            bound.observed = actual;
            Ok(true)
        } else if matches!(r.action, Action::Confirm(_)) {
            Ok(false)
        } else if actual == bound.observed && expected != bound.observed {
            // UIA may publish the preceding application snapshot after the TSF edit
            // acknowledgement. Wait for the exact new position; never accept this old
            // observation as permission to write or adopt a different caret.
            Err(Error::new("targetObservationPending"))
        } else {
            Err(Error::new("targetChanged"))
        }
    }
}
fn observe(auto: &IUIAutomation, r: &Request) -> Result<(IUIAutomationElement, Position), Error> {
    unsafe {
        let element = call!(r, auto.GetFocusedElement());
        if call!(r, element.CurrentProcessId()) != r.anchor.process as i32 {
            return Err(Error::new("targetChanged"));
        }
        live(r)?;
        let password = element
            .CurrentIsPassword()
            .map_err(|_| Error::new("targetSafetyUnknown"))?;
        live(r)?;
        if password.as_bool() {
            return Err(Error::new("targetPassword"));
        }
        let pattern = call!(
            r,
            element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
        );
        let ranges = call!(r, pattern.GetSelection());
        if call!(r, ranges.Length()) != 1 {
            return Err(Error::new("targetSelectionUnavailable"));
        }
        let selected = call!(r, ranges.GetElement(0));
        let before = call!(r, pattern.DocumentRange());
        call!(
            r,
            before.MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                &selected,
                TextPatternRangeEndpoint_Start,
            )
        );
        let after = call!(r, pattern.DocumentRange());
        call!(
            r,
            after.MoveEndpointByRange(
                TextPatternRangeEndpoint_Start,
                &selected,
                TextPatternRangeEndpoint_End,
            )
        );
        // Bounded reads, including the prefix length: equal nearby characters cannot
        // hide a moved caret or a maxlength refusal. Oversize observations are unavailable.
        let result = Position {
            before: read(r, &before, 65536)?,
            selected: read(r, &selected, 16384)?,
            after: read(r, &after, 65536)?,
        };
        if Instant::now() >= r.expires || !super::native::composition_target(&r.task, &r.anchor)? {
            return Err(Error::new("targetTimeout"));
        }
        Ok((element, result))
    }
}
fn read(r: &Request, range: &IUIAutomationTextRange, limit: i32) -> Result<String, Error> {
    let text = call!(r, unsafe { range.GetText(limit + 1) });
    if text.len() > limit as usize {
        return Err(Error::new("targetTextLimit"));
    }
    Ok(normalize(&text.to_string()))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_proof_requires_position_text_and_untouched_surroundings() {
        let initial = Position {
            before: "a".repeat(3000),
            selected: "old🙂".into(),
            after: "tail".into(),
        };
        let expected = initial.replaced("a");
        assert!(expected != initial); // maxlength rejects even identical repeated characters
        let mut moved = expected.clone();
        moved.before.push('a');
        assert!(moved != expected);
        let mut edited = expected.clone();
        edited.after.push('x');
        assert!(edited != expected);
        assert_eq!(
            initial.replaced("中\r\n🙂").before,
            "a".repeat(3000) + "中\n🙂"
        );
    }
}
