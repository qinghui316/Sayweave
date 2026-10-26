//! Test-only pipe peer. No model, microphone, UI or input permission.
use sayweave_local_asr_protocol::*;
use std::{
    io::{self, Write},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
fn emit(event: &Event) {
    write_json(&mut io::stdout().lock(), event).unwrap();
}
fn log(path: &std::path::Path, event: &str, id: &RunId) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path.with_extension("events"))
        .unwrap();
    writeln!(
        file,
        "{} {event} {} {}",
        std::process::id(),
        id.turn,
        id.request
    )
    .unwrap();
}
fn wait(path: &std::path::Path, suffix: &str, cancel: &AtomicBool, ignore: bool) {
    while !path.with_extension(suffix).exists() && (ignore || !cancel.load(Ordering::Acquire)) {
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn main() {
    let active = Arc::new(Mutex::new(None::<RunId>));
    let cancel = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::channel();
    let current = active.clone();
    let stopped = cancel.clone();
    std::thread::spawn(move || {
        let mut input = io::stdin().lock();
        while let Ok(Some(frame)) = read_frame(&mut input) {
            if let Frame::Json(bytes) = &frame
                && let Ok(command) = serde_json::from_slice::<Command>(bytes)
            {
                if let Command::Cancel { id } = &command
                    && current.lock().unwrap().as_ref() == Some(id)
                {
                    stopped.store(true, Ordering::Release);
                }
                if matches!(command, Command::Shutdown) {
                    stopped.store(true, Ordering::Release);
                }
            }
            if tx.send(frame).is_err() {
                return;
            }
        }
        stopped.store(true, Ordering::Release);
    });
    let mut path = PathBuf::new();
    let mut run = None;
    let mut retired: Option<RunId> = None;
    let mut sequence = 0;
    while let Ok(frame) = rx.recv() {
        if let Frame::Audio {
            id, sequence: seq, ..
        } = frame
        {
            if run.as_ref() != Some(&id) {
                emit(&Event::Error {
                    id: Some(id),
                    code: "localStaleOperation".into(),
                });
                continue;
            }
            sequence = seq;
            log(&path, "audio", &id);
            emit(&Event::Update {
                id,
                revision: seq as i32,
                full: "字".repeat(seq as usize),
                confirmed: "字".repeat(seq as usize),
                committed: String::new(),
                tentative: String::new(),
                native_token_count: seq as usize,
                audio_received_ms: seq as i64 * 100,
                audio_committed_ms: seq as i64 * 100,
                final_result: false,
            });
            continue;
        }
        match decode_json::<Command>(frame).unwrap() {
            Command::Hello { version, instance } => emit(&Event::Hello {
                version,
                instance,
                engine: "0.1.3".into(),
            }),
            Command::Prepare {
                id, path: model, ..
            } => {
                path = PathBuf::from(model);
                *active.lock().unwrap() = Some(id.clone());
                cancel.store(false, Ordering::Release);
                log(&path, "prepare", &id);
                if path.file_name().unwrap().to_string_lossy().contains("cold") {
                    wait(&path, "ready", &cancel, false);
                }
                if cancel.load(Ordering::Acquire) {
                    emit(&Event::Cancelled { id });
                    continue;
                }
                emit(&Event::Prepared {
                    id,
                    backend: "cpu".into(),
                    device: Some("test peer".into()),
                    device_id: None,
                    device_memory_total: None,
                    device_memory_free: None,
                    warmup_ms: 0,
                    streaming: true,
                });
            }
            Command::Start { id, .. } => {
                *active.lock().unwrap() = Some(id.clone());
                cancel.store(false, Ordering::Release);
                sequence = 0;
                log(&path, "start", &id);
                run = Some(id.clone());
                if path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains("stale")
                    && let Some(old) = &retired
                {
                    emit(&Event::Ready { id: old.clone() });
                    emit(&Event::Error {
                        id: Some(old.clone()),
                        code: "localInferenceFailed".into(),
                    });
                }
                emit(&Event::Ready { id });
                if path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains("stale")
                    && let Some(old) = &retired
                {
                    emit(&Event::Update {
                        id: old.clone(),
                        revision: 99,
                        full: "旧字".into(),
                        confirmed: "旧字".into(),
                        committed: String::new(),
                        tentative: String::new(),
                        native_token_count: 1,
                        audio_received_ms: 100,
                        audio_committed_ms: 100,
                        final_result: true,
                    });
                    emit(&Event::Finished { id: old.clone() });
                }
            }
            Command::Finish { id } if run.as_ref() == Some(&id) => {
                log(&path, "finishWaiting", &id);
                if path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .contains("finish")
                {
                    wait(&path, "finished", &cancel, false);
                }
                if cancel.load(Ordering::Acquire) {
                    emit(&Event::Cancelled { id });
                    run = None;
                    continue;
                }
                emit(&Event::Update {
                    id: id.clone(),
                    revision: sequence as i32,
                    full: "字".repeat(sequence as usize),
                    confirmed: "字".repeat(sequence as usize),
                    committed: String::new(),
                    tentative: String::new(),
                    native_token_count: sequence as usize,
                    audio_received_ms: sequence as i64 * 100,
                    audio_committed_ms: sequence as i64 * 100,
                    final_result: true,
                });
                log(&path, "finished", &id);
                emit(&Event::Finished { id: id.clone() });
                retired = Some(id);
                run = None;
            }
            Command::Cancel { id } => {
                if path.file_name().unwrap().to_string_lossy().contains("hang") {
                    loop {
                        std::thread::sleep(Duration::from_secs(1));
                    }
                }
                log(&path, "cancelled", &id);
                emit(&Event::Cancelled { id });
                run = None;
            }
            Command::Shutdown => return,
            _ => emit(&Event::Error {
                id: None,
                code: "localProtocol".into(),
            }),
        }
    }
}
