//! The existing owned rich-text fixture drives the real actor and TSF adapter.
//! Only ASR/model responses and edit notifications are controlled here.
use super::*;
use std::io::{BufRead, Write};

// Hold only the actor's completion, after the authenticated input worker has
// returned Handoff. The same worker can service a read-only runtime Inspect;
// no ThreadMgr/profile activation or Block Read runs to manufacture a premise.
static SOURCE_RELEASE: (std::sync::Mutex<bool>, std::sync::Condvar) =
    (std::sync::Mutex::new(false), std::sync::Condvar::new());
static SOURCE_TIMEOUT: AtomicBool = AtomicBool::new(false);
pub(super) fn readonly_handoff_barrier() {
    if std::env::var("SAYWEAVE_ACTOR_SOURCE_INSPECT").as_deref() != Ok("1") {
        return;
    }
    eprintln!("owned_source_barrier=waiting");
    let released = SOURCE_RELEASE.0.lock().unwrap_or_else(|e| e.into_inner());
    let (released, timeout) = SOURCE_RELEASE
        .1
        .wait_timeout_while(released, Duration::from_secs(10), |done| !*done)
        .unwrap_or_else(|e| e.into_inner());
    SOURCE_TIMEOUT.store(!*released || timeout.timed_out(), Ordering::Release);
    eprintln!("owned_source_barrier=released");
}
fn release_source_barrier() {
    *SOURCE_RELEASE.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
    SOURCE_RELEASE.1.notify_all();
}

/// Independent host-body evidence for explicitly prefixed actual-host trials.
/// The retained field is never used to authorize a production mutation.
struct ActualBodyReader {
    automation: windows::Win32::UI::Accessibility::IUIAutomation,
    field: windows::Win32::UI::Accessibility::IUIAutomationElement,
    anchor: windows_input::Anchor,
    unprefixed: bool,
    _apartment: ActualBodyApartment,
}
struct ActualBodyApartment;
impl Drop for ActualBodyApartment {
    fn drop(&mut self) {
        unsafe { windows::Win32::System::Com::CoUninitialize() }
    }
}
impl ActualBodyReader {
    fn open(anchor: &windows_input::Anchor) -> windows::core::Result<Self> {
        use windows::Win32::{System::Com::*, UI::Accessibility::*};
        use windows::core::Interface;
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            let apartment = ActualBodyApartment;
            let automation: IUIAutomation =
                CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
            let bounded: IUIAutomation2 = automation.cast()?;
            bounded.SetConnectionTimeout(1000)?;
            bounded.SetTransactionTimeout(1000)?;
            let field = automation.GetFocusedElement()?;
            let reader = Self {
                automation,
                field,
                anchor: anchor.clone(),
                unprefixed: std::env::var("SAYWEAVE_ACTOR_EMPTY_FIELD").as_deref() == Ok("1"),
                _apartment: apartment,
            };
            let initial = reader.read()?;
            if reader.unprefixed && !initial.chars().all(|c| matches!(c, '\r' | '\n')) {
                return Err(windows::core::Error::new(
                    windows::Win32::Foundation::E_ACCESSDENIED,
                    "explicit empty test field was not empty at admission",
                ));
            }
            Ok(reader)
        }
    }
    fn check_focus(&self) -> windows::core::Result<()> {
        use windows::Win32::{Foundation::E_ACCESSDENIED, UI::Accessibility::*};
        let reject = |stage: &str| windows::core::Error::new(E_ACCESSDENIED, stage);
        let current = windows_input::WindowsInputService::observed_anchor()
            .ok_or_else(|| reject("actual-body native focus absent"))?;
        if current.window != self.anchor.window
            || current.process != self.anchor.process
            || current.lifetime != self.anchor.lifetime
            || current.editor != self.anchor.editor
        {
            return Err(reject("actual-body native endpoint changed"));
        }
        unsafe {
            let diagnostic = |stage: &str, error: windows::core::Error| {
                windows::core::Error::new(error.code(), format!("actual-body {stage}: {error}"))
            };
            let focused = self
                .automation
                .GetFocusedElement()
                .map_err(|e| diagnostic("focused element", e))?;
            let kind = self
                .field
                .CurrentControlType()
                .map_err(|e| diagnostic("retained control type", e))?;
            if !self
                .automation
                .CompareElements(&self.field, &focused)
                .map_err(|e| diagnostic("retained field comparison", e))?
                .as_bool()
                || self
                    .field
                    .CurrentProcessId()
                    .map_err(|e| diagnostic("retained process", e))?
                    != self.anchor.editor.process as i32
                || !self
                    .field
                    .CurrentHasKeyboardFocus()
                    .map_err(|e| diagnostic("retained keyboard focus", e))?
                    .as_bool()
                || self
                    .field
                    .CurrentIsPassword()
                    .map_err(|e| diagnostic("retained password", e))?
                    .as_bool()
                || !self
                    .field
                    .CurrentIsEnabled()
                    .map_err(|e| diagnostic("retained enabled", e))?
                    .as_bool()
                || ![UIA_EditControlTypeId, UIA_GroupControlTypeId].contains(&kind)
            {
                return Err(reject(&format!(
                    "actual-body field: kind={} process={} focus={} enabled={} password={}",
                    kind.0,
                    self.field.CurrentProcessId()?,
                    self.field.CurrentHasKeyboardFocus()?.as_bool(),
                    self.field.CurrentIsEnabled()?.as_bool(),
                    self.field.CurrentIsPassword()?.as_bool()
                )));
            }
            Ok(())
        }
    }
    fn collapsed_selection(&self) -> windows::core::Result<usize> {
        use windows::Win32::{Foundation::E_ACCESSDENIED, UI::Accessibility::*};
        self.check_focus()?;
        unsafe {
            let pattern = self
                .field
                .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)?;
            let selected = pattern.GetSelection()?;
            if selected.Length()? != 1 {
                return Err(E_ACCESSDENIED.into());
            }
            let selected = selected.GetElement(0)?;
            if selected.CompareEndpoints(
                TextPatternRangeEndpoint_Start,
                &selected,
                TextPatternRangeEndpoint_End,
            )? != 0
                || !selected.GetText(1025)?.is_empty()
            {
                return Err(E_ACCESSDENIED.into());
            }
            let prefix = pattern.DocumentRange()?;
            prefix.MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                &selected,
                TextPatternRangeEndpoint_Start,
            )?;
            let text = prefix.GetText(1025)?.to_string();
            if text.encode_utf16().count() > 1024 {
                return Err(E_ACCESSDENIED.into());
            }
            self.check_focus()?;
            Ok(text.encode_utf16().count())
        }
    }
    fn middle_point(&self) -> windows::core::Result<Option<serde_json::Value>> {
        use windows::Win32::{
            Foundation::*,
            System::{Ole::*, Variant::*},
            UI::{Accessibility::*, HiDpi::*, WindowsAndMessaging::*},
        };
        self.check_focus()?;
        struct RestoreDpi(DPI_AWARENESS_CONTEXT);
        impl Drop for RestoreDpi {
            fn drop(&mut self) {
                unsafe {
                    let _ = SetThreadDpiAwarenessContext(self.0);
                }
            }
        }
        unsafe {
            // UIA rectangles are physical screen coordinates; match the
            // per-monitor-aware fixture before converting them to client units.
            let previous = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            if previous.0.is_null() {
                return Err(E_ACCESSDENIED.into());
            }
            let _dpi = RestoreDpi(previous);
            let range = self
                .field
                .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)?
                .DocumentRange()?;
            range.MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                &range,
                TextPatternRangeEndpoint_Start,
            )?;
            if range.MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, 3)? != 3
                || range.MoveEndpointByUnit(
                    TextPatternRangeEndpoint_Start,
                    TextUnit_Character,
                    2,
                )? != 2
            {
                return Ok(None);
            }
            // Geometry only for the third character of the owned four-character
            // test output. No text read, selection change or native prewarm.
            let array = range.GetBoundingRectangles()?;
            let result = (|| -> windows::core::Result<[f64; 4]> {
                if array.is_null()
                    || SafeArrayGetDim(array) != 1
                    || SafeArrayGetVartype(array)? != VT_R8
                {
                    return Err(windows::core::Error::new(
                        E_ACCESSDENIED,
                        "geometry array type",
                    ));
                }
                let low = SafeArrayGetLBound(array, 1)?;
                let high = SafeArrayGetUBound(array, 1)?;
                if i64::from(high) - i64::from(low) != 3 {
                    return Err(windows::core::Error::new(
                        E_ACCESSDENIED,
                        format!("geometry rectangle bounds {low}..{high}"),
                    ));
                }
                let mut values = [0f64; 4];
                for (index, value) in values.iter_mut().enumerate() {
                    let at = low + index as i32;
                    SafeArrayGetElement(array, &at, (value as *mut f64).cast())?;
                }
                if values.iter().any(|value| !value.is_finite())
                    || values[2] <= 0.
                    || values[3] <= 0.
                {
                    return Err(windows::core::Error::new(
                        E_ACCESSDENIED,
                        format!("geometry rectangle values {values:?}"),
                    ));
                }
                Ok(values)
            })();
            if !array.is_null() {
                SafeArrayDestroy(array)?;
            }
            let [left, top, width, height] = result?;
            let bounds = self.field.CurrentBoundingRectangle()?;
            let x = left + (width * 0.1).min(1.);
            let y = top + height * 0.5;
            if x < f64::from(bounds.left)
                || x >= f64::from(bounds.right)
                || y < f64::from(bounds.top)
                || y >= f64::from(bounds.bottom)
            {
                return Err(windows::core::Error::new(
                    E_ACCESSDENIED,
                    format!("geometry point {x},{y} outside field {bounds:?}"),
                ));
            }
            let window = GetAncestor(HWND(self.anchor.window as *mut _), GA_ROOT);
            let mut point = POINT {
                x: x.round() as i32,
                y: y.round() as i32,
            };
            if point.x < bounds.left
                || point.x >= bounds.right
                || point.y < bounds.top
                || point.y >= bounds.bottom
            {
                return Err(E_ACCESSDENIED.into());
            }
            if !windows::Win32::Graphics::Gdi::ScreenToClient(window, &mut point).as_bool() {
                return Err(E_ACCESSDENIED.into());
            }
            self.check_focus()?;
            Ok(Some(
                serde_json::json!({"window":window.0 as usize,"x":point.x,"y":point.y,"screenX":x.round() as i32,"screenY":y.round() as i32,"previousDpiContext":previous.0 as isize,"offset":2,"geometryOnly":true}),
            ))
        }
    }
    fn read(&self) -> windows::core::Result<String> {
        use windows::Win32::{Foundation::E_ACCESSDENIED, UI::Accessibility::*};
        self.check_focus()?;
        let reject = |stage: &str| windows::core::Error::new(E_ACCESSDENIED, stage);
        unsafe {
            let diagnostic = |stage: &str, error: windows::core::Error| {
                windows::core::Error::new(error.code(), format!("actual-body {stage}: {error}"))
            };
            let pattern: IUIAutomationTextPattern = self
                .field
                .GetCurrentPatternAs(UIA_TextPatternId)
                .map_err(|e| diagnostic("text pattern", e))?;
            let range = pattern
                .DocumentRange()
                .map_err(|e| diagnostic("document range", e))?;
            let owner = range
                .GetEnclosingElement()
                .map_err(|e| diagnostic("range owner", e))?;
            // UIA defines the enclosing element as the innermost element; a
            // paragraph/group inside the retained editable field is valid.
            let walker = self
                .automation
                .RawViewWalker()
                .map_err(|e| diagnostic("raw walker", e))?;
            let mut ancestor = owner.clone();
            let mut owner_within_field = false;
            let mut owner_chain = Vec::new();
            for _ in 0..16 {
                let kind = ancestor
                    .CurrentControlType()
                    .map_err(|e| diagnostic("ancestor control type", e))?;
                owner_chain.push(kind.0);
                if self
                    .automation
                    .CompareElements(&self.field, &ancestor)
                    .map_err(|e| diagnostic("ancestor comparison", e))?
                    .as_bool()
                {
                    owner_within_field = true;
                    break;
                }
                if ancestor
                    .CurrentProcessId()
                    .map_err(|e| diagnostic("ancestor process", e))?
                    != self.anchor.editor.process as i32
                    || [UIA_WindowControlTypeId, UIA_DocumentControlTypeId].contains(&kind)
                {
                    break;
                }
                ancestor = walker
                    .GetParentElement(&ancestor)
                    .map_err(|e| diagnostic("range owner ancestry", e))?;
            }
            let readonly = range
                .GetAttributeValue(UIA_IsReadOnlyAttributeId)
                .map_err(|e| diagnostic("range readonly", e))?;
            if !owner_within_field
                || readonly.vt() != windows::Win32::System::Variant::VT_BOOL
                || bool::try_from(&readonly).map_err(|e| diagnostic("readonly type", e))?
            {
                return Err(reject(&format!(
                    "actual-body range: withinField={owner_within_field} ownerChain={owner_chain:?} readonlyVt={}",
                    readonly.vt().0
                )));
            }
            let text = range
                .GetText(1025)
                .map_err(|e| diagnostic("range text", e))?
                .to_string();
            if text.encode_utf16().count() > 1024 || !self.unprefixed && !text.starts_with("测试：")
            {
                return Err(reject(&format!(
                    "actual-body prefix/size: units={}",
                    text.encode_utf16().count()
                )));
            }
            // Recheck the retained object after the cross-process text request.
            if !self
                .automation
                .CompareElements(
                    &self.field,
                    &self
                        .automation
                        .GetFocusedElement()
                        .map_err(|e| diagnostic("final focus", e))?,
                )
                .map_err(|e| diagnostic("final field comparison", e))?
                .as_bool()
            {
                return Err(reject("actual-body focus changed after read"));
            }
            Ok(text)
        }
    }
}

#[test]
#[ignore = "requires the explicit prefixed actual-host test field; read-only"]
fn actual_body_readback_probe() {
    assert_eq!(
        std::env::var("SAYWEAVE_INPUT_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let expected: u32 = std::env::var("SAYWEAVE_TSF_OWNED_PID")
        .unwrap()
        .parse()
        .unwrap();
    let anchor = windows_input::WindowsInputService::anchor().unwrap();
    assert_eq!(anchor.process, expected);
    let reader = ActualBodyReader::open(&anchor).expect("actual focused field diagnostics");
    println!(
        "ACTOR_JSON:{}",
        serde_json::json!({"actualBody":reader.read().unwrap()})
    );
}

/// Test-only bounded maintenance window. This never changes COM registration,
/// profile enablement or the daily executable. A durable receipt precedes CAS.
#[test]
#[ignore = "requires explicit temporary deployment authorization and elevation"]
fn richtext_core_window() {
    use sayweave_text_service::deployment as deploy;
    assert_eq!(
        std::env::var("SAYWEAVE_INPUT_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    let directory = std::path::PathBuf::from(std::env::var("SAYWEAVE_CORE_WINDOW").unwrap());
    assert!(directory.is_absolute() && directory.is_dir());
    for name in ["ready", "stop", "result.txt", "receipt.json"] {
        assert!(
            !directory.join(name).exists(),
            "fresh maintenance directory required"
        );
    }
    let proxy = include_bytes!(concat!(env!("OUT_DIR"), "/text-service.dll"));
    let core = include_bytes!(concat!(env!("OUT_DIR"), "/text-service-core.dll"));
    assert!(!proxy.is_empty() && !core.is_empty());
    let _guard = crate::installer::legacy_maintenance_guard(core).unwrap();
    assert!(!deploy::core_update_pending().unwrap());
    let before = deploy::current_engine()
        .unwrap()
        .expect("existing descriptor");
    deploy::verified_core(&before).unwrap();
    assert_eq!(before.proxy_sha256, deploy::version(proxy));
    let (_, target) = deploy::prepare_engine(proxy, core).unwrap();
    let mut receipt = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(directory.join("receipt.json"))
        .unwrap();
    serde_json::to_writer(
        &mut receipt,
        &serde_json::json!({"before":before,"target":target}),
    )
    .unwrap();
    receipt.sync_all().unwrap();
    // Restore on unwind as well as driver completion/timeout. Never overwrite a
    // third-party descriptor; a failed restore remains explicit in the receipt.
    struct Restore {
        before: sayweave_text_service::lifecycle::CurrentEngine,
        target: sayweave_text_service::lifecycle::CurrentEngine,
        directory: std::path::PathBuf,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            let result = match deploy::current_engine() {
                Ok(Some(current)) if current == self.before => Ok(()),
                Ok(Some(current)) if current == self.target => {
                    deploy::publish_engine(Some(&self.target), Some(&self.before))
                }
                Ok(_) => Err(windows::core::Error::from_hresult(
                    windows::Win32::Foundation::E_UNEXPECTED,
                )),
                Err(error) => Err(error),
            };
            let status = match result {
                Ok(()) => "restored".to_owned(),
                Err(e) => format!("restore failed: {e}"),
            };
            let written = (|| -> std::io::Result<()> {
                let mut file = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(self.directory.join("result.txt"))?;
                file.write_all(status.as_bytes())?;
                file.sync_all()
            })();
            if let Err(error) = written {
                eprintln!("maintenance receipt failure: {status}: {error}");
            }
        }
    }
    let restore = Restore {
        before,
        target,
        directory: directory.clone(),
    };
    deploy::publish_engine(Some(&restore.before), Some(&restore.target)).unwrap();
    std::fs::write(directory.join("ready"), b"ready").unwrap();
    let seconds = std::env::var("SAYWEAVE_CORE_WINDOW_SECONDS")
        .ok()
        .map(|value| value.parse::<u64>().expect("owned core window duration"))
        .unwrap_or(300);
    assert!(
        (300..=900).contains(&seconds),
        "bounded core window required"
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(seconds);
    while !directory.join("stop").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    drop(restore);
    assert_eq!(
        std::fs::read_to_string(directory.join("result.txt")).unwrap(),
        "restored"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the owned rich-text fixture and matching installed core"]
async fn richtext_actor() {
    // Hold the first successful native Update receipt before the actor can
    // schedule Bind. This changes scheduling only, not the native result.
    static HOLD_FIRST_UPDATE: AtomicBool = AtomicBool::new(false);
    static HOLD_SCOPE_RELEASE: AtomicBool = AtomicBool::new(false);
    static HOLD_HANDOFF: AtomicBool = AtomicBool::new(false);
    static HELD_INITIAL_REFUSAL: AtomicBool = AtomicBool::new(false);
    static BARRIER_TIMEOUT: AtomicBool = AtomicBool::new(false);
    static RELEASE_UPDATE: (std::sync::Mutex<bool>, std::sync::Condvar) =
        (std::sync::Mutex::new(false), std::sync::Condvar::new());
    struct FixtureLog;
    impl log::Log for FixtureLog {
        fn enabled(&self, _: &log::Metadata<'_>) -> bool {
            true
        }
        fn log(&self, record: &log::Record<'_>) {
            let line = record.args().to_string();
            eprintln!("{line}");
            if line.contains("operation=composition_edit")
                && line.contains("action=Begin applied=false code=edit:beginProperties:80040500")
                && let Ok(path) = std::env::var("SAYWEAVE_ACTOR_INITIAL_REFUSAL_BARRIER")
                && !HELD_INITIAL_REFUSAL.swap(true, Ordering::AcqRel)
            {
                eprintln!("owned_initial_refusal_barrier=waiting");
                let until = std::time::Instant::now() + Duration::from_secs(8);
                while !std::path::Path::new(&path).is_file() {
                    assert!(
                        std::time::Instant::now() < until,
                        "initial refusal barrier timed out"
                    );
                    std::thread::sleep(Duration::from_millis(10));
                }
                eprintln!("owned_initial_refusal_barrier=released");
            }
            let hold_update = line.contains("action=Update applied=true")
                && HOLD_FIRST_UPDATE.swap(false, Ordering::AcqRel);
            let hold_release = line.contains("code=blockReleased")
                && HOLD_SCOPE_RELEASE.swap(false, Ordering::AcqRel);
            let hold_handoff = line.contains("action=Handoff applied=true code=keyboardHandedOff")
                && HOLD_HANDOFF.swap(false, Ordering::AcqRel);
            if line.contains("operation=composition_edit")
                && (hold_update || hold_release || hold_handoff)
            {
                let barrier = if hold_handoff {
                    "owned_handoff_barrier"
                } else {
                    "owned_bind_barrier"
                };
                eprintln!("{barrier}=waiting");
                let (lock, ready) = &RELEASE_UPDATE;
                let released = lock.lock().unwrap_or_else(|e| e.into_inner());
                let (released, timeout) = ready
                    .wait_timeout_while(released, Duration::from_secs(10), |done| !*done)
                    .unwrap_or_else(|e| e.into_inner());
                let failed = !*released || timeout.timed_out();
                drop(released);
                BARRIER_TIMEOUT.store(failed, Ordering::Release);
                if failed {
                    eprintln!("{barrier}=timeout");
                }
                eprintln!("{barrier}=released");
            }
        }
        fn flush(&self) {}
    }
    static LOG: FixtureLog = FixtureLog;
    log::set_logger(&LOG).unwrap();
    log::set_max_level(log::LevelFilter::Info);
    assert_eq!(
        std::env::var("SAYWEAVE_INPUT_ACCEPTANCE").as_deref(),
        Ok("1")
    );
    assert_ne!(
        std::env::var("SAYWEAVE_DRAFT_STRATEGY").as_deref(),
        Ok("segmented")
    );
    let pid: u32 = std::env::var("SAYWEAVE_TSF_OWNED_PID")
        .unwrap()
        .parse()
        .unwrap();
    let input = windows_input::WindowsInputService::new();
    let mut anchor = windows_input::WindowsInputService::anchor().unwrap();
    assert_eq!(
        anchor.process, pid,
        "only the owned foreground editor is allowed"
    );
    let body_reader = if std::env::var("SAYWEAVE_ACTOR_BODY_READBACK").as_deref() == Ok("1") {
        eprintln!("owned_body_barrier=waiting");
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(current) = windows_input::WindowsInputService::anchor()
                && current.process == pid
                && let Ok(reader) = ActualBodyReader::open(&current)
            {
                let before = reader.read().unwrap();
                tokio::time::sleep(Duration::from_millis(500)).await;
                if reader.read().is_ok_and(|after| after == before) {
                    anchor = reader.anchor.clone();
                    break Some(reader);
                }
            }
            assert!(
                Instant::now() < deadline,
                "prefixed body readiness deadline"
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    } else {
        None
    };
    let task = uuid::Uuid::new_v4().to_string();
    let _physical = (std::env::var("SAYWEAVE_ACTOR_PHYSICAL_INPUT").as_deref() == Ok("1"))
        .then(|| crate::shortcuts::FixturePhysicalInput::start(&task).unwrap());
    input.bind_target(&task, &anchor, None);
    let gate = Arc::new(windows_input::DeliveryGate::default());
    let initial = Draft::prepare(task.clone(), 1, anchor.clone(), gate)
        .await
        .unwrap();
    let pending_initial = initial.draft.is_none();
    if std::env::var("SAYWEAVE_ACTOR_INITIAL_IME").as_deref() != Ok("1") {
        assert!(
            !pending_initial,
            "ordinary input must have a ready native range"
        );
    }
    let replies = std::env::var("SAYWEAVE_ACTOR_MODEL_REPLIES")
        .ok()
        .map(|raw| {
            let replies: Vec<String> = serde_json::from_str(&raw).expect("owned model reply list");
            assert!(
                !replies.is_empty()
                    && replies.len() <= 4
                    && replies.iter().all(|text| text.len() <= 1024)
            );
            replies.into_iter().map(|text| response(&text)).collect()
        })
        .unwrap_or_else(|| {
            vec![
                response("下周一上午开会"),
                response("下周一下午三点开会"),
                response("下周一下午四点线上开会"),
            ]
        });
    let provider = Fixture::start(replies);
    let root = tempfile::tempdir().unwrap();
    let data = crate::data::DataWorker::start(root.path().to_owned()).unwrap();
    let learning_enabled = std::env::var("SAYWEAVE_ACTOR_LEARNING").as_deref() == Ok("1");
    let (learning_send, learning_events) = std::sync::mpsc::channel();
    let mut learned = Vec::new();
    if learning_enabled {
        let application = windows_input::application_context(Some(anchor.clone())).await;
        let frozen = crate::language_learning::snapshot(&data, application)
            .await
            .unwrap();
        let sink = Arc::new(crate::windows_input::composition::ObservationSink {
            control: Arc::new(crate::language_policy::LearningControl::new(
                frozen.settings.clone(),
            )),
            snapshot: frozen,
            emit: Arc::new(move |e| {
                let _ = learning_send.send(e);
            }),
        });
        let ready = if std::env::var("SAYWEAVE_ACTOR_LEARNING_FIELD").as_deref() == Ok("1") {
            initial
                .draft
                .as_ref()
                .expect("field-only learning fixture requires initial range")
                .observe_field_fixture(sink)
                .await
        } else {
            initial.observe(sink).await
        };
        assert!(ready, "owned observation admission");
    }
    let mut context = Context::fixture(provider.provider.clone(), provider.input(), data.clone());
    context.task_id.clone_from(&task);
    let control = Control::new(input_state::activity_evidence());
    control.resume(1, anchor.activity);
    control.fixture_ready(1, anchor.activity);
    let (cancel, _) = watch::channel(0);
    let manager = SessionDraft::new_initial(
        task.clone(),
        anchor,
        initial,
        control.clone(),
        cancel.clone(),
        (std::env::var("SAYWEAVE_ACTOR_RAW_ONLY").as_deref() != Ok("1")).then_some(context),
        None,
    );
    HOLD_FIRST_UPDATE.store(
        std::env::var("SAYWEAVE_BLOCK_EARLY_EDIT").as_deref() == Ok("1"),
        Ordering::Release,
    );
    HOLD_SCOPE_RELEASE.store(
        std::env::var("SAYWEAVE_BLOCK_SEAL_BARRIER").as_deref() == Ok("1"),
        Ordering::Release,
    );
    HOLD_HANDOFF.store(
        std::env::var("SAYWEAVE_BLOCK_HANDOFF_BARRIER").as_deref() == Ok("1"),
        Ordering::Release,
    );
    manager.activate();
    let publications = Arc::new(Mutex::new(Vec::new()));
    let events = publications.clone();
    let actor = tokio::spawn(manager.clone().maintain(move |value| {
        events.lock().unwrap().push(value);
    }));
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break };
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let emit = |value: serde_json::Value| {
        println!("ACTOR_JSON:{value}");
        std::io::stdout().flush().unwrap();
    };
    emit(
        serde_json::json!({"ready":true,"task":task,"process":pid,"activity":input_state::activity(),"pendingInitial":pending_initial}),
    );
    let mut latest = NativeTranscript::default();
    let mut finished = false;
    let keep_after_finish = std::env::var("SAYWEAVE_ACTOR_KEEP_AFTER_FINISH").as_deref() == Ok("1");
    let result: Result<(), String> = async {
        let timeout_seconds = std::env::var("SAYWEAVE_ACTOR_TIMEOUT_SECONDS")
            .ok().map(|value| value.parse::<u64>().expect("owned actor timeout"))
            .unwrap_or(90);
        assert!((30..=180).contains(&timeout_seconds), "bounded owned actor timeout required");
        let deadline = Instant::now() + Duration::from_secs(timeout_seconds);
        for _ in 0..2000 {
            if Instant::now() >= deadline { return Err("owned actor deadline".into()); }
            let line = receiver.recv_timeout(Duration::from_secs(15)).map_err(|e|e.to_string())?;
            let command: serde_json::Value = serde_json::from_str(&line).map_err(|e|e.to_string())?;
            if BARRIER_TIMEOUT.load(Ordering::Acquire) {
                return Err("owned Bind barrier timeout".into());
            }
            if SOURCE_TIMEOUT.load(Ordering::Acquire) {
                return Err("owned source barrier timeout".into());
            }
            match command["action"].as_str() {
                Some("asr") => {
                    let segments = command["segments"].as_array().ok_or("source array required")?;
                    if segments.len() > 4 { return Err("owned source limit".into()); }
                    let segments = segments.iter().map(|s| Ok(SourceSegment {
                        id: s["id"].as_u64().ok_or("source id")?,
                        revision: s["revision"].as_u64().ok_or("source revision")?,
                        text: s["text"].as_str().filter(|t|t.len()<1024).ok_or("source text")?.into(),
                        confirmed: s["confirmed"].as_bool().ok_or("source confirmation")?,
                    })).collect::<Result<Vec<_>, &str>>()?;
                    latest.sequence += 1;
                    latest.task_id.clone_from(&task);
                    latest.turn = 1;
                    latest.text = segments.iter().map(|s|s.text.as_str()).collect();
                    latest.segments = segments;
                    manager.update(&latest);
                }
                Some("edited") => { input_state::edited(); }
                Some("release-bind") => {
                    let (lock, ready) = &RELEASE_UPDATE;
                    *lock.lock().unwrap_or_else(|e| e.into_inner()) = true;
                    ready.notify_one();
                    release_source_barrier();
                }
                Some("native-facts") if std::env::var("SAYWEAVE_ACTOR_SOURCE_INSPECT").as_deref()==Ok("1") => {
                    let reply=manager.connection.inspect_fixture().await.map_err(|e|e.code)?;
                    emit(serde_json::json!({"nativeFacts":reply}));
                    continue;
                }
                Some("native-read") if std::env::var("SAYWEAVE_ACTOR_SOURCE_INSPECT").as_deref()==Ok("1") => {
                    let block: sayweave_text_service::wire::BlockCommand=serde_json::from_value(command["block"].clone()).map_err(|e|e.to_string())?;
                    if block.action!=sayweave_text_service::wire::BlockAction::Read {return Err("readonly fixture permits only Read".into());}
                    let reply=manager.connection.block(1,None,block,String::new(),Arc::new(AtomicBool::new(true))).await.map_err(|e|e.code)?;
                    emit(serde_json::json!({"nativeRead":reply}));
                    continue;
                }
                Some("status") => {},
                Some("learning") if learning_enabled => {
                    for evidence in learning_events.try_iter() {
                        learned.push(crate::language_learning::classify_observed_fixture(&data,evidence).await?);
                    }
                    emit(serde_json::json!({"learning":learned}));
                    continue;
                }
                Some("done") if (learning_enabled||keep_after_finish) && finished => {emit(serde_json::json!({"done":true}));return Ok(());}
                Some("body-focus") => {
                    let reader = body_reader.as_ref().ok_or("actual body readback not enabled")?;
                    reader.check_focus().map_err(|error|format!("actual field identity: {error:?}"))?;
                    emit(serde_json::json!({"actualFocus":true,"activity":input_state::activity()}));
                    continue;
                }
                Some("body-middle-point") => {
                    let reader = body_reader.as_ref().ok_or("actual body readback not enabled")?;
                    match reader.middle_point() {
                        Ok(Some(point)) => emit(serde_json::json!({"actualPoint":point})),
                        Ok(None) => emit(serde_json::json!({"actualPoint":null,"geometryPending":true})),
                        Err(error) => return Err(format!("actual field geometry: {error:?}")),
                    }
                    continue;
                }
                Some("body-selection") => {
                    let reader = body_reader.as_ref().ok_or("actual body readback not enabled")?;
                    let offset=reader.collapsed_selection().map_err(|error|format!("actual field selection: {error:?}"))?;
                    emit(serde_json::json!({"actualSelection":[offset,offset]}));
                    continue;
                }
                Some("body") => {
                    let reader = body_reader.as_ref().ok_or("actual body readback not enabled")?;
                    let body = match reader.read() {
                        Ok(body) => body,
                        Err(error) if error.code().0 as u32 == windows::Win32::UI::Accessibility::UIA_E_TIMEOUT => {
                            emit(serde_json::json!({"actualBody":null,"readError":format!("{error}"),"activity":input_state::activity(),"editingKeyHeld":input_state::editing_key_held()}));
                            continue;
                        }
                        Err(error) => return Err(format!("actual body readback: {error:?}")),
                    };
                    emit(serde_json::json!({"actualBody":body,"activity":input_state::activity(),"editingKeyHeld":input_state::editing_key_held()}));
                    continue;
                }
                Some("finish" | "stop") => {
                    let stopped = command["action"] == "stop";
                    control.request_finish();
                    cancel.send_replace(if stopped { 2 } else { 1 });
                    release_source_barrier();
                    let delivery = manager.finish(&latest, if stopped { Finish::Abort } else { Finish::Commit }).await;
                    finished = true;
                    let report = manager.report().ok_or("missing actor report")?;
                    let actual_body = if let Some(reader) = &body_reader {
                        tokio::time::sleep(Duration::from_millis(80)).await;
                        Some(reader.read().map_err(|e|format!("settled actual body readback: {e}"))?)
                    } else { None };
                    emit(serde_json::json!({"finished":true,"delivery":delivery,"text":report.text,"actualBody":actual_body,"error":report.error,"resultOnly":report.result_only,"resultRaw":report.result_raw,"requests":provider.seen.lock().unwrap().len()}));
                    if !report.result_only && (report.error.as_deref().is_some_and(|error| !stopped || error != "cancelled") || delivery.restore_error || delivery.submission == Submission::Uncertain) {
                        return Err("actor delivery was not safely settled".into());
                    }
                    if learning_enabled||keep_after_finish {continue;}
                    return Ok(());
                }
                _ => return Err("unknown owned actor command".into()),
            }
            if !finished && (*cancel.borrow() > 0 || control.stopping()) {
                return Err(format!("actor stopped: {:?}", manager.report().and_then(|r|r.error)));
            }
            emit(serde_json::json!({"accepted":true,"activity":input_state::activity(),"editingKeyHeld":input_state::editing_key_held(),"sequence":latest.sequence,"resultOnly":manager.result_only(),"presentation":manager.presentation(),"published":publications.lock().unwrap().last().copied(),"audioRoute":control.audio.current(),"requests":provider.seen.lock().unwrap().len()}));
        }
        Err("owned actor command limit".into())
    }.await;
    // Also unblock native retirement if the driver exits during the edit.
    *RELEASE_UPDATE.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
    RELEASE_UPDATE.1.notify_one();
    release_source_barrier();
    if !finished {
        control.request_finish();
        cancel.send_replace(2);
        let _ = tokio::time::timeout(
            Duration::from_secs(5),
            manager.finish(&latest, Finish::Abort),
        )
        .await;
    }
    let settled = tokio::time::timeout(Duration::from_secs(5), actor).await;
    input.forget(task);
    input.shutdown();
    crate::windows_input::composition::stop_observer()
        .await
        .unwrap();
    drop(manager);
    data.shutdown().await;
    result.unwrap();
    settled.unwrap().unwrap();
}
