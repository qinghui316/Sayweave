//! Bounded first-refusal telemetry. Never used to grant or restore a writer.
use super::*;
use serde::Serialize;

#[derive(Default)]
pub(super) struct Record {
    baseline: Option<Rc<Frame>>,
    ui_baseline: Option<Rc<Frame>>,
    first: Option<Rc<Frame>>,
    collecting: bool,
}
#[derive(Serialize)]
struct Frame {
    tick: u64,
    origin: &'static str,
    phase: String,
    error: Option<u32>,
    block: String,
    epoch: u64,
    version: u64,
    lease: u64,
    valid: bool,
    writable: bool,
    writing: bool,
    end_version: u64,
    end_lease: u64,
    end_valid: bool,
    expected_units: usize,
    field_matches: bool,
    range: Option<[i32; 2]>,
    selected: Option<[i32; 2]>,
    document_end: Option<i32>,
    native_units: Option<usize>,
    native_read_error: Option<u32>,
    projection: Option<initial_selection::BlockProjection>,
    projection_error: Option<String>,
    provider_summary: Option<ProviderSummary>,
}
#[derive(Default, Serialize)]
struct ProviderSummary {
    raw_units: Option<usize>,
    tsf_units: Option<usize>,
    selected: [Option<usize>; 2],
    equal: Option<bool>,
    mapped: Option<bool>,
    stable: Option<bool>,
    proxy: Option<bool>,
    stage: Option<String>,
    error: Option<u32>,
}
impl ProviderSummary {
    // inspect() also returns metadata when its full representation proof is
    // unavailable. Keep only these generated numeric/boolean/fixed-stage keys.
    fn read(input: &str) -> Self {
        let mut out = Self::default();
        for part in input.split_ascii_whitespace() {
            let Some((key, value)) = part.split_once('=') else {
                continue;
            };
            match key {
                "raw_len" => out.raw_units = value.parse().ok(),
                "tsf_len" => out.tsf_units = value.parse().ok(),
                "raw_start" => out.selected[0] = value.parse().ok(),
                "raw_end" => out.selected[1] = value.parse().ok(),
                "raw_tsf_equal" => out.equal = value.parse().ok(),
                "raw_tsf_mapped" => out.mapped = value.parse().ok(),
                "raw_stable" => out.stable = value.parse().ok(),
                "raw_proxy_interface" => out.proxy = value.parse().ok(),
                "raw_stage"
                    if value.len() <= 32 && value.bytes().all(|b| b.is_ascii_alphanumeric()) =>
                {
                    out.stage = Some(value.into())
                }
                "raw_error" if value.len() == 8 => out.error = u32::from_str_radix(value, 16).ok(),
                _ => (),
            }
        }
        out
    }
}
pub(super) unsafe fn capture(
    context: &ITfContext,
    range: &ITfRange,
    field: &initial_selection::FocusIdentity,
    data: &Rc<RefCell<Data>>,
    ec: u32,
    cause: (&'static str, &str, Option<HRESULT>),
) {
    let (origin, phase, error) = cause;
    let (mut frame, baseline) = {
        let mut d = data.borrow_mut();
        if error.is_some() && d.evidence.first.is_some() {
            return;
        }
        let frame = Frame {
            tick: unsafe { GetTickCount64() },
            origin,
            phase: phase.into(),
            error: error.map(|e| e.0 as u32),
            block: d.id.clone(),
            epoch: d.epoch,
            version: d.version,
            lease: d.lease,
            valid: d.valid,
            writable: d.writable,
            writing: d.writing,
            end_version: d.version,
            end_lease: d.lease,
            end_valid: d.valid,
            expected_units: d.text.encode_utf16().count(),
            field_matches: false,
            range: None,
            selected: None,
            document_end: None,
            native_units: None,
            native_read_error: None,
            projection: None,
            projection_error: None,
            provider_summary: None,
        };
        if error.is_some() {
            // Reserve the first header before any COM call. A reentrant refusal
            // during baseline capture still keeps its own tick and origin.
            let header = Frame {
                tick: frame.tick,
                origin,
                phase: phase.into(),
                error: frame.error,
                block: frame.block.clone(),
                epoch: frame.epoch,
                version: frame.version,
                lease: frame.lease,
                valid: frame.valid,
                writable: frame.writable,
                writing: frame.writing,
                end_version: frame.version,
                end_lease: frame.lease,
                end_valid: frame.valid,
                expected_units: frame.expected_units,
                field_matches: false,
                range: None,
                selected: None,
                document_end: None,
                native_units: None,
                native_read_error: None,
                projection: None,
                projection_error: Some("capturePending".into()),
                provider_summary: None,
            };
            d.evidence.first = Some(Rc::new(header));
        }
        if d.evidence.collecting {
            return;
        }
        d.evidence.collecting = true;
        (frame, d.evidence.ui_baseline.clone())
    };
    // The collection flag suppresses telemetry recursion only. Existing edit,
    // focus, revocation and retirement callbacks continue to run normally.
    let result = (|| unsafe {
        frame.field_matches = field.matches()?;
        if !frame.field_matches {
            return Ok(());
        }
        let position = range_position(context, range, ec).ok();
        frame.range = position.map(|p| [p.start, p.end]);
        frame.selected = selection(context, ec)
            .ok()
            .and_then(|r| range_position(context, &r, ec).ok())
            .map(|p| [p.start, p.end]);
        let end = context.GetEnd(ec)?;
        frame.document_end = range_position(context, &end, ec).ok().map(|p| p.end);
        match range_text(range, ec) {
            Ok(text) => frame.native_units = Some(text.encode_utf16().count()),
            Err(e) => frame.native_read_error = Some(e.code().0 as u32),
        }
        let whole = context.GetStart(ec)?;
        whole.ShiftEndToRange(ec, &end, TF_ANCHOR_END)?;
        if password(context, &whole, ec)? {
            return Err(failure(E_ACCESSDENIED));
        }
        let native = range_text(&whole, ec)?;
        let (summary, evidence) = initial_selection::inspect(&native);
        frame.provider_summary = Some(ProviderSummary::read(&summary));
        let Some(evidence) = evidence else {
            frame.projection_error = Some("providerUnavailable".into());
            return Ok(());
        };
        evidence.revalidate(&native)?;
        if native != range_text(&whole, ec)?
            || !field.matches()?
            || position != range_position(context, range, ec).ok()
        {
            return Err(failure(TF_E_DISCONNECTED));
        }
        if let Some(position) = position {
            let mut projection = evidence.block_projection(position.start, position.end);
            if let Some(previous) = baseline.as_ref().and_then(|v| v.projection.as_ref()) {
                projection.compare(previous);
            }
            evidence.revalidate(&native)?;
            if native != range_text(&whole, ec)?
                || !field.matches()?
                || Some(position) != range_position(context, range, ec).ok()
            {
                return Err(failure(TF_E_DISCONNECTED));
            }
            frame.projection = Some(projection);
        }
        Ok::<_, windows::core::Error>(())
    })();
    if let Err(e) = result {
        frame.projection_error = Some(format!("probe:{:08x}", e.code().0 as u32));
    }
    let (end_version, end_lease, end_valid, raced) = {
        let d = data.borrow();
        (
            d.version,
            d.lease,
            d.valid,
            d.epoch != frame.epoch
                || d.version != frame.version
                || d.lease != frame.lease
                || d.valid != frame.valid,
        )
    };
    frame.end_version = end_version;
    frame.end_lease = end_lease;
    frame.end_valid = end_valid;
    if raced {
        // Releasing the provider can reenter Data; no RefCell borrow is alive.
        frame.projection = None;
        frame.projection_error = Some("captureRaced".into());
    }
    let frame = Rc::new(frame);
    let (retired, retired_ui) = {
        let mut d = data.borrow_mut();
        d.evidence.collecting = false;
        if error.is_some() {
            if d.evidence
                .first
                .as_ref()
                .is_some_and(|v| v.tick == frame.tick && v.origin == frame.origin)
            {
                (d.evidence.first.replace(frame), None)
            } else {
                (None, None)
            }
        } else if d.evidence.first.is_none() && d.valid {
            let ui = if frame.projection.as_ref().is_some_and(|v| v.mapped()) {
                d.evidence.ui_baseline.replace(frame.clone())
            } else {
                None
            };
            (d.evidence.baseline.replace(frame), ui)
        } else {
            (None, None)
        }
    };
    // Provider references are released outside RefCell borrows.
    drop(retired);
    drop(retired_ui);
}
pub(super) fn suffix(data: &Rc<RefCell<Data>>) -> String {
    let (baseline, ui_baseline, first) = {
        let d = data.borrow();
        (
            d.evidence.baseline.clone(),
            d.evidence.ui_baseline.clone(),
            d.evidence.first.clone(),
        )
    };
    if first.is_none() {
        return String::new();
    }
    let value = serde_json::json!({"before":baseline.as_deref(),"ui_before":ui_baseline.as_deref(),"first":first.as_deref()});
    let encoded = value.to_string();
    if encoded.len() > 4096 {
        return ":structure=oversize".into();
    }
    format!(":structure={encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unavailable_projection_keeps_structural_numbers_but_no_external_strings() {
        let summary = ProviderSummary::read(
            "raw_len=5 tsf_len=0 raw_start=0 raw_end=0 raw_tsf_equal=false raw_tsf_mapped=false raw_stable=true raw_proxy_interface=false raw_module=private.dll body=secret path=C:/private hash=abcdef raw_stage=secret/path raw_us=99",
        );
        let value = serde_json::to_value(summary).unwrap();
        assert_eq!(value["raw_units"], 5);
        assert_eq!(value["tsf_units"], 0);
        assert_eq!(value["mapped"], false);
        assert_eq!(value["stable"], true);
        let encoded = value.to_string();
        for forbidden in ["private", "secret", "path", "module", "hash", "abcdef"] {
            assert!(!encoded.contains(forbidden));
        }
        assert_eq!(
            ProviderSummary::read("raw_stage=textPattern raw_error=80004002").error,
            Some(0x80004002)
        );
    }
}
