use super::*;
use serde_json::json;
fn event(id: u64, text: &str, end: bool) -> serde_json::Value {
    json!({"header":{"task_id":"task","event":"result-generated"},"payload":{"output":{"sentence":{"sentence_id":id,"text":text,"sentence_end":end,"begin_time":0,"end_time":1000}}}})
}
#[test]
fn partial_replacement_final_deduplication_heartbeat_and_task_identity() {
    let mut t = qwen::Transcript::default();
    t.event(&event(1, "你", false), "task").unwrap();
    t.event(&event(1, "你好", false), "task").unwrap();
    assert_eq!(t.text(false), "你好");
    assert_eq!(t.text(true), "");
    t.event(&event(1, "你好。", true), "task").unwrap();
    t.event(&event(1, "旧暂定", false), "task").unwrap();
    t.event(&event(1, "重复", true), "task").unwrap();
    t.event(&event(2, "世界。", true), "task").unwrap();
    assert_eq!(t.text(true), "你好。世界。");
    assert_eq!(t.segments().len(), 2);
    let heartbeat = json!({"header":{"task_id":"task","event":"result-generated"},"payload":{"output":{"sentence":{"heartbeat":true,"sentence_id":0}}}});
    assert!(matches!(
        t.event(&heartbeat, "task").unwrap(),
        qwen::Event::Heartbeat
    ));
    assert!(t.event(&event(0, "非法序号", false), "task").is_err());
    assert!(t.event(&event(3, "旧任务", true), "different").is_err());
    assert!(matches!(
        t.event(
            &json!({"header":{"task_id":"task","event":"task-finished"},"payload":{"output":{}}}),
            "task"
        )
        .unwrap(),
        qwen::Event::Finished
    ));
    assert!(
        t.event(&event(3, &"x".repeat(65537), true), "task")
            .is_err()
    );
}
#[test]
fn protocol_address_is_origin_only_and_errors_do_not_leak_service_body() {
    assert_eq!(
        qwen::endpoint("https://workspace.cn-beijing.maas.aliyuncs.com").unwrap(),
        "wss://workspace.cn-beijing.maas.aliyuncs.com/api-ws/v1/inference"
    );
    for base in [
        "http://localhost",
        "https://user:secret@host/",
        "https://host/v1",
        "https://host/?token=key",
        "https://host/#fragment",
    ] {
        assert!(qwen::endpoint(base).is_err());
    }
    let mut t = qwen::Transcript::default();
    let e=t.event(&json!({"header":{"task_id":"task","event":"task-failed","error_code":"InvalidApiKey","error_message":"PRIVATE SECRET"}}),"task").err().unwrap();
    assert_eq!(e.code, "auth");
    assert!(!serde_json::to_string(&e).unwrap().contains("PRIVATE"));
}
#[test]
fn resampling_preserves_first_and_last_audio_and_exact_duration() {
    for rate in [8000usize, 16000, 44100, 48000, 96000] {
        for frames in [1usize, rate / 10 + 3, rate] {
            let samples = (0..frames)
                .map(|n| 0.4 * (2.0 * std::f32::consts::PI * 440.0 * n as f32 / rate as f32).sin())
                .collect::<Vec<_>>();
            let mut converter = audio::Converter::new(rate).unwrap();
            let mut out = vec![];
            for block in samples.chunks(137) {
                out.extend(converter.push(block).unwrap());
            }
            out.extend(converter.finish().unwrap());
            assert_eq!(
                out.len(),
                frames * 16000 / rate,
                "rate={rate}, frames={frames}"
            );
            if frames >= rate / 10 {
                assert!(out.iter().take(160).any(|s| s.abs() > 2000));
                assert!(out.iter().rev().take(160).any(|s| s.abs() > 2000));
                assert!(out.iter().all(|s| s.abs() < 20000));
            }
        }
    }
}
#[tokio::test]
async fn bounded_buffers_and_capture_failure_are_explicit() {
    let err = audio::Capture::start(Some("invalid-device-id".into()), None)
        .await
        .err()
        .unwrap();
    assert_eq!(err.code, "deviceMissing");
    let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(2);
    producer.push(1.).unwrap();
    producer.push(2.).unwrap();
    assert!(producer.push(3.).is_err());
    assert_eq!(consumer.pop().unwrap(), 1.);
    assert_eq!(consumer.pop().unwrap(), 2.);
}

#[test]
fn stereo_and_integer_sample_formats_downmix_without_clipping() {
    assert!((audio::mono(&[0.8f32, -0.4]) - 0.2).abs() < 0.0001);
    assert_eq!(audio::mono(&[32767i16, -32767]), 0.0);
    assert!(audio::mono(&[u16::MAX, 32768u16]) > 0.49);
    assert!(audio::mono(&[i32::MAX, 0i32]) > 0.49);
}

#[tokio::test]
async fn stopping_rejects_fresh_operations_but_drains_existing_admission() {
    let root = tempfile::tempdir().unwrap();
    let worker = crate::data::DataWorker::start(root.path().to_path_buf()).unwrap();
    let service =
        super::SpeechService::new(crate::history::HistoryService::new(worker, root.path()));
    let admitted = service.admit().unwrap();
    service.shutdown();
    assert_eq!(service.admit().err().unwrap().code, "stopping");
    assert_eq!(service.play("unused").await.unwrap_err().code, "stopping");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(30), service.drain())
            .await
            .is_err()
    );
    // Work admitted before shutdown may still finish its database transaction.
    service.history.preferences().await.unwrap();
    drop(admitted);
    tokio::time::timeout(std::time::Duration::from_secs(1), service.drain())
        .await
        .unwrap();
}

#[tokio::test]
async fn cancellation_clears_temporary_audio_even_if_database_is_unavailable() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("dictionary.sqlite"), b"invalid sqlite").unwrap();
    let worker = crate::data::DataWorker::start(root.path().to_path_buf()).unwrap();
    let service =
        super::SpeechService::new(crate::history::HistoryService::new(worker, root.path()));
    *service.last_audio.lock().unwrap() = Some(("cancelled-record".into(), vec![1, 2, 3]));
    assert!(service.discard_new("cancelled-record", true).await);
    assert!(service.last_audio.lock().unwrap().is_none());
}

#[tokio::test]
async fn shutdown_waits_for_admitted_background_cleanup() {
    let root = tempfile::tempdir().unwrap();
    let worker = crate::data::DataWorker::start(root.path().to_path_buf()).unwrap();
    let service = Arc::new(super::SpeechService::new(
        crate::history::HistoryService::new(worker, root.path()),
    ));
    service.initialize().await.unwrap();
    // Pause the actual cleanup query with another SQLite connection, not a fake counter.
    let lock = rusqlite::Connection::open(root.path().join("dictionary.sqlite")).unwrap();
    lock.execute_batch("BEGIN EXCLUSIVE").unwrap();
    let pending = service.clone();
    let cleanup = tokio::spawn(async move { pending.maintain().await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while service.operations.load(Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    service.shutdown();
    assert_eq!(service.maintain().await.unwrap_err().code, "stopping");
    assert!(
        tokio::time::timeout(Duration::from_millis(30), service.drain())
            .await
            .is_err()
    );
    lock.execute_batch("ROLLBACK").unwrap();
    cleanup.await.unwrap().unwrap();
    tokio::time::timeout(Duration::from_secs(1), service.drain())
        .await
        .unwrap();
}
