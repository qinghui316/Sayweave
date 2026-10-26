use super::*;
use sha2::{Digest, Sha256};
#[test]
fn worker_receives_only_os_environment_and_no_provider_or_backend_overrides() {
    let env = child_environment([
        ("SystemRoot".into(), "C:\\Windows".into()),
        (
            "LOCALAPPDATA".into(),
            "C:\\Users\\fixture\\AppData\\Local".into(),
        ),
        ("QWEN_API_KEY".into(), "private-fixture".into()),
        ("OPENAI_API_KEY".into(), "private-fixture".into()),
        ("VK_ICD_FILENAMES".into(), "foreign-driver".into()),
        ("Path".into(), "foreign-dll-directory".into()),
    ]);
    assert_eq!(env.len(), 2);
    assert_eq!(env[0].0, "SystemRoot");
    assert_eq!(env[1].0, "LOCALAPPDATA");
}
fn id(turn: u64) -> RunId {
    RunId {
        task: "one-voice-task".into(),
        turn,
        request: uuid::Uuid::new_v4().to_string(),
    }
}
pub(crate) fn setup(mode: &str) -> (tempfile::TempDir, Runtime, Arc<ModelLease>) {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join(format!("{mode}.gguf"));
    std::fs::write(&path, b"fixture").unwrap();
    let lease =
        ModelLease::acquire(&path, 7, &format!("{:x}", Sha256::digest(b"fixture"))).unwrap();
    let program = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("local-asr-fixture.exe");
    assert!(
        program.is_file(),
        "Build protocol --features fixture --bin local-asr-fixture using the shared target first"
    );
    let bytes = std::fs::read(&program).unwrap();
    let staged = root.path().join("sayweave-local-asr-worker.exe");
    std::fs::write(&staged, &bytes).unwrap();
    let manifest = vec![payload::RuntimeFile {
        name: "sayweave-local-asr-worker.exe".into(),
        bytes: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&bytes)),
    }];
    (root, Runtime::new(staged, manifest), lease)
}
#[tokio::test]
#[ignore = "explicit isolated pipe-peer binary, built by local-ASR validation"]
async fn stalled_event_consumer_cannot_deadlock_finish_or_the_next_turn() {
    let (_root, runtime, model) = setup("ordinary");
    let mut session = runtime
        .open(id(1), model.clone(), DevicePolicy::Cpu)
        .await
        .unwrap();
    for _ in 0..50 {
        session.send_pcm(&[1; 1600]).await.unwrap();
    }
    tokio::time::timeout(Duration::from_secs(3), async {
        while session.events.len() < 32 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    // A finish acknowledgement is only command acceptance. The bridge must
    // actually settle even when nobody drains its result queue.
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        let _ = session.finish().await;
        while !session.bridge.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let mut terminal = None;
    if result.is_ok() {
        while let Some(event) = session.events.recv().await {
            if let Err(code) = event {
                terminal = Some(code);
            }
        }
    }
    drop(session);
    let mut next = tokio::time::timeout(
        Duration::from_secs(3),
        runtime.open(id(2), model, DevicePolicy::Cpu),
    )
    .await
    .unwrap()
    .unwrap();
    next.send_pcm(&[1; 1600]).await.unwrap();
    next.finish().await.unwrap();
    finished(&mut next).await;
    drop(next);
    runtime.shutdown().await.unwrap();
    assert!(
        result.is_ok(),
        "A stalled result consumer kept the bridge and compute lease alive"
    );
    assert_eq!(terminal, Some("localBufferOverflow"));
}
async fn observed(model: &ModelLease, event: &str) -> String {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let text =
                std::fs::read_to_string(model.path.with_extension("events")).unwrap_or_default();
            if text.contains(event) {
                return text;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
async fn finished(session: &mut Session) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match session.next_event().await.unwrap() {
                Event::Finished { .. } => return,
                Event::Update { .. } => {}
                event => panic!("unexpected {event:?}"),
            }
        }
    })
    .await
    .unwrap();
}
#[tokio::test]
#[ignore = "explicit isolated pipe-peer binary, built by local-ASR validation"]
async fn next_turn_waits_for_actual_old_finalization_and_reuses_one_process() {
    let (_root, runtime, model) = setup("finish");
    let mut old = runtime
        .open(id(1), model.clone(), DevicePolicy::Cpu)
        .await
        .unwrap();
    old.send_pcm(&[1; 1600]).await.unwrap();
    old.finish().await.unwrap();
    observed(&model, "finishWaiting").await;
    let next_runtime = runtime.clone();
    let next_model = model.clone();
    let next = tokio::spawn(async move {
        next_runtime
            .open(id(2), next_model, DevicePolicy::Cpu)
            .await
    });
    tokio::task::yield_now().await;
    assert!(!next.is_finished());
    std::fs::write(model.path.with_extension("finished"), b"release").unwrap();
    finished(&mut old).await;
    let mut next = tokio::time::timeout(Duration::from_secs(5), next)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    next.send_pcm(&[2; 1600]).await.unwrap();
    next.finish().await.unwrap();
    finished(&mut next).await;
    runtime.shutdown().await.unwrap();
    let log = observed(&model, "start 2").await;
    assert_eq!(log.lines().filter(|s| s.contains("prepare")).count(), 1);
    let finish = log.find("finished 1").unwrap();
    let start = log.find("start 2").unwrap();
    assert!(finish < start);
}
#[tokio::test]
#[ignore = "explicit isolated pipe-peer binary, built by local-ASR validation"]
async fn cancelling_queued_turn_never_cancels_active_compute() {
    let (_root, runtime, model) = setup("ordinary");
    let mut active = runtime
        .open(id(1), model.clone(), DevicePolicy::Cpu)
        .await
        .unwrap();
    let r = runtime.clone();
    let m = model.clone();
    let pending = tokio::spawn(async move { r.open(id(2), m, DevicePolicy::Cpu).await });
    tokio::task::yield_now().await;
    pending.abort();
    let _ = pending.await;
    active.send_pcm(&[1; 1600]).await.unwrap();
    assert!(matches!(
        active.next_event().await.unwrap(),
        Event::Update { .. }
    ));
    active.finish().await.unwrap();
    finished(&mut active).await;
    runtime.shutdown().await.unwrap();
    assert!(!observed(&model, "finished").await.contains("cancelled"));
}
#[tokio::test]
#[ignore = "explicit isolated pipe-peer binary, built by local-ASR validation"]
async fn retired_ready_error_text_and_finish_cannot_enter_the_new_turn() {
    let (_root, runtime, model) = setup("stale");
    let mut first = runtime
        .open(id(1), model.clone(), DevicePolicy::Cpu)
        .await
        .unwrap();
    first.send_pcm(&[1; 1600]).await.unwrap();
    first.finish().await.unwrap();
    finished(&mut first).await;
    let current = id(2);
    let mut next = runtime
        .open(current.clone(), model.clone(), DevicePolicy::Cpu)
        .await
        .unwrap();
    next.send_pcm(&[2; 1600]).await.unwrap();
    match next.next_event().await.unwrap() {
        Event::Update { id, full, .. } => {
            assert_eq!(id, current);
            assert_eq!(full, "字");
        }
        event => panic!("unexpected {event:?}"),
    }
    next.finish().await.unwrap();
    finished(&mut next).await;
    runtime.shutdown().await.unwrap();
}
#[tokio::test]
#[ignore = "explicit isolated pipe-peer binary, built by local-ASR validation"]
async fn cancelled_cold_prepare_is_not_reused_with_half_completed_handshake() {
    let (_root, runtime, model) = setup("cold");
    let r = runtime.clone();
    let m = model.clone();
    let preparing = tokio::spawn(async move { r.prepare(id(1), m, DevicePolicy::Cpu).await });
    observed(&model, "prepare 1").await;
    preparing.abort();
    let _ = preparing.await;
    // A future preparation owns a new request. It cannot inherit the old nonce/Hello state.
    std::fs::write(model.path.with_extension("ready"), b"release").unwrap();
    let mut next = tokio::time::timeout(
        Duration::from_secs(5),
        runtime.open(id(2), model.clone(), DevicePolicy::Cpu),
    )
    .await
    .unwrap()
    .unwrap();
    next.send_pcm(&[1; 1600]).await.unwrap();
    next.finish().await.unwrap();
    finished(&mut next).await;
    runtime.shutdown().await.unwrap();
    let log = observed(&model, "finished 2").await;
    let pids = log
        .lines()
        .map(|s| s.split_whitespace().next().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(pids.len(), 2);
}
#[tokio::test]
#[ignore = "explicit isolated pipe-peer binary, built by local-ASR validation"]
async fn unresponsive_cancel_exits_before_the_next_process_gets_compute() {
    let (_root, runtime, model) = setup("hang");
    let old = runtime
        .open(id(1), model.clone(), DevicePolicy::Cpu)
        .await
        .unwrap();
    drop(old);
    let mut next = tokio::time::timeout(
        Duration::from_secs(5),
        runtime.open(id(2), model.clone(), DevicePolicy::Cpu),
    )
    .await
    .unwrap()
    .unwrap();
    next.send_pcm(&[1; 1600]).await.unwrap();
    next.finish().await.unwrap();
    finished(&mut next).await;
    runtime.shutdown().await.unwrap();
    let log = observed(&model, "finished 2").await;
    let pids = log
        .lines()
        .map(|s| s.split_whitespace().next().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(pids.len(), 2);
    assert!(runtime.0.worker.lock().await.is_none());
}
#[tokio::test]
#[ignore = "explicit isolated pipe-peer binary, built by local-ASR validation"]
async fn frozen_task_lease_prevents_cloud_switch_from_unloading_between_turns() {
    let (_root, runtime, model) = setup("ordinary");
    runtime.select(true);
    let task = runtime.task_lease().unwrap();
    let mut first = runtime
        .open(id(1), model.clone(), DevicePolicy::Cpu)
        .await
        .unwrap();
    first.finish().await.unwrap();
    finished(&mut first).await;
    runtime.select(false);
    assert!(runtime.unload_idle().await.is_err());
    let mut next = runtime
        .open(id(2), model.clone(), DevicePolicy::Cpu)
        .await
        .unwrap();
    next.finish().await.unwrap();
    finished(&mut next).await;
    drop(task);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if runtime.0.worker.lock().await.is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(
        observed(&model, "finished 2")
            .await
            .lines()
            .filter(|s| s.contains("prepare"))
            .count(),
        1
    );
    runtime.shutdown().await.unwrap();
}
#[cfg(windows)]
#[tokio::test]
#[ignore = "explicit isolated pipe-peer binary, built by local-ASR validation"]
async fn closing_private_job_terminates_the_child_and_wait_observes_exit() {
    let (_root, runtime, _model) = setup("ordinary");
    let mut child = tokio::process::Command::new(&runtime.0.program)
        .arg("--stdio")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .creation_flags(0x08000000)
        .spawn()
        .unwrap();
    let job = job::Job::attach(&child).unwrap();
    drop(job);
    assert!(
        tokio::time::timeout(Duration::from_secs(2), child.wait())
            .await
            .unwrap()
            .unwrap()
            .code()
            .is_some()
    );
}
