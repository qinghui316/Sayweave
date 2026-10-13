//! The existing owned rich-text fixture drives the real actor and TSF adapter.
//! Only ASR/model responses and edit notifications are controlled here.
use super::*;
use std::io::{BufRead, Write};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires the owned rich-text fixture and matching installed core"]
async fn richtext_actor() {
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
        for _ in 0..80 {
            let line = receiver.recv_timeout(Duration::from_secs(15)).map_err(|e|e.to_string())?;
            let command: serde_json::Value = serde_json::from_str(&line).map_err(|e|e.to_string())?;
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
