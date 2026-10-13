//! The existing owned rich-text fixture drives the real actor and TSF adapter.
//! Only ASR/model responses and edit notifications are controlled here.
use super::*;
use std::io::{BufRead, Write};

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
    let deadline = std::time::Instant::now() + Duration::from_secs(300);
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
                && line.contains("action=Update applied=true")
                && HOLD_FIRST_UPDATE.swap(false, Ordering::AcqRel)
            {
                eprintln!("owned_bind_barrier=waiting");
                let (lock, ready) = &RELEASE_UPDATE;
                let released = lock.lock().unwrap_or_else(|e| e.into_inner());
                let (released, timeout) = ready
                    .wait_timeout_while(released, Duration::from_secs(10), |done| !*done)
                    .unwrap_or_else(|e| e.into_inner());
                let failed = !*released || timeout.timed_out();
                drop(released);
                BARRIER_TIMEOUT.store(failed, Ordering::Release);
                if failed {
                    eprintln!("owned_bind_barrier=timeout");
                }
                eprintln!("owned_bind_barrier=released");
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
    let anchor = windows_input::WindowsInputService::anchor().unwrap();
    assert_eq!(
        anchor.process, pid,
        "only the owned foreground editor is allowed"
    );
    let task = uuid::Uuid::new_v4().to_string();
    input.bind_target(&task, &anchor, None);
    let gate = Arc::new(windows_input::DeliveryGate::default());
    let draft = Draft::begin(task.clone(), 1, anchor.clone(), gate)
        .await
        .unwrap();
    let provider = Fixture::start(vec![
        response("下周一上午开会"),
        response("下周一下午三点开会"),
        response("下周一下午四点线上开会"),
    ]);
    let root = tempfile::tempdir().unwrap();
    let data = crate::data::DataWorker::start(root.path().to_owned()).unwrap();
    let mut context = Context::fixture(provider.provider.clone(), provider.input(), data.clone());
    context.task_id.clone_from(&task);
    let control = Control::new(input_state::activity_evidence());
    control.resume(1, anchor.activity);
    let (cancel, _) = watch::channel(0);
    let manager = SessionDraft::new(
        task.clone(),
        anchor,
        draft,
        control.clone(),
        cancel.clone(),
        Some(context),
        None,
    );
    HOLD_FIRST_UPDATE.store(
        std::env::var("SAYWEAVE_BLOCK_EARLY_EDIT").as_deref() == Ok("1"),
        Ordering::Release,
    );
    manager.activate();
    let actor = tokio::spawn(manager.clone().maintain(|_| {}));
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
    emit(serde_json::json!({"ready":true,"task":task,"process":pid}));
    let mut latest = NativeTranscript::default();
    let mut finished = false;
    let result: Result<(), String> = async {
        let deadline = Instant::now() + Duration::from_secs(90);
        for _ in 0..2000 {
            if Instant::now() >= deadline { return Err("owned actor deadline".into()); }
            let line = receiver.recv_timeout(Duration::from_secs(15)).map_err(|e|e.to_string())?;
            let command: serde_json::Value = serde_json::from_str(&line).map_err(|e|e.to_string())?;
            if BARRIER_TIMEOUT.load(Ordering::Acquire) {
                return Err("owned Bind barrier timeout".into());
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
                }
                Some("status") => {},
                Some("finish") => {
                    control.request_finish();
                    cancel.send_replace(1);
                    let delivery = manager.finish(&latest, Finish::Commit).await;
                    let report = manager.report().ok_or("missing actor report")?;
                    emit(serde_json::json!({"finished":true,"delivery":delivery,"text":report.text,"error":report.error,"requests":provider.seen.lock().unwrap().len()}));
                    finished = true;
                    if report.error.is_some() || delivery.restore_error || delivery.submission == Submission::Uncertain {
                        return Err("actor delivery was not safely settled".into());
                    }
                    return Ok(());
                }
                _ => return Err("unknown owned actor command".into()),
            }
            if *cancel.borrow() > 0 || control.stopping() {
                return Err(format!("actor stopped: {:?}", manager.report().and_then(|r|r.error)));
            }
            emit(serde_json::json!({"accepted":true,"sequence":latest.sequence,"presentation":manager.presentation(),"requests":provider.seen.lock().unwrap().len()}));
        }
        Err("owned actor command limit".into())
    }.await;
    // Also unblock native retirement if the driver exits during the edit.
    *RELEASE_UPDATE.0.lock().unwrap_or_else(|e| e.into_inner()) = true;
    RELEASE_UPDATE.1.notify_one();
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
    drop(manager);
    data.shutdown().await;
    result.unwrap();
    settled.unwrap().unwrap();
}
