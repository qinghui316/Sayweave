use sayweave_local_asr_protocol::{
    Command, DevicePolicy, Event, Frame, RunId, VERSION, decode_json, read_frame, write_json,
};
use sayweave_local_asr_worker::{
    error_code, load, proof::NativeAppendProof, run_options, snapshot, warm_stream,
};
use sha2::{Digest, Sha256};
use std::{
    collections::VecDeque,
    io::{self, BufWriter, Read},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
};
use transcribe_cpp::{CancelToken, Model, ParakeetStreamOptions, StreamExtension, StreamOptions};

type Output = Arc<Mutex<BufWriter<io::Stdout>>>;
type Cancellation = Arc<Mutex<Option<(RunId, CancelToken)>>>;
struct Prepared {
    path: String,
    sha256: String,
    device: DevicePolicy,
    model: Model,
    warmup_ms: u64,
}

fn emit(out: &Output, event: &Event) -> Result<(), &'static str> {
    write_json(&mut *out.lock().map_err(|_| "localProtocol")?, event).map_err(|_| "localPipeClosed")
}
fn fail(out: &Output, id: Option<RunId>, code: &str) {
    let _ = emit(
        out,
        &Event::Error {
            id,
            code: code.into(),
        },
    );
}
fn cancel_current(current: &Cancellation, id: Option<&RunId>) {
    if let Ok(guard) = current.lock()
        && let Some((active, token)) = guard.as_ref()
        && id.is_none_or(|id| id == active)
    {
        token.cancel();
    }
}
fn valid_id(id: &RunId) -> bool {
    id.is_valid()
}
fn idle_command(frame: Frame) -> Result<Command, (Option<RunId>, &'static str)> {
    // Cooperative cancellation can finish before the reader queue is drained.
    // Retired PCM remains owned by its original run, never by the next stream.
    match frame {
        Frame::Audio { id, .. } => Err((Some(id), "localStaleOperation")),
        frame => decode_json(frame).map_err(|_| (None, "localProtocol")),
    }
}
fn prepare(
    path: String,
    sha256: String,
    device: DevicePolicy,
    token: &CancelToken,
) -> Result<Prepared, &'static str> {
    let file_path = PathBuf::from(&path);
    if !file_path.is_absolute()
        || sha256.len() != 64
        || !sha256.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("localProtocol");
    }
    let mut file = std::fs::File::open(&file_path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "localModelMissing"
        } else {
            "localModelUnreadable"
        }
    })?;
    let mut digest = Sha256::new();
    let mut buf = [0; 64 * 1024];
    loop {
        if token.is_cancelled() {
            return Err("cancelled");
        }
        let n = file.read(&mut buf).map_err(|_| "localModelUnreadable")?;
        if n == 0 {
            break;
        }
        digest.update(&buf[..n]);
    }
    if format!("{:x}", digest.finalize()) != sha256.to_ascii_lowercase() {
        return Err("localModelCorrupt");
    }
    let mut model = load(&file_path, device)?;
    let warmup_ms = match warm_stream(&model, token) {
        Err("localBackendUnavailable" | "localMemoryUnavailable")
            if device == DevicePolicy::Auto && model.device().is_ok_and(|d| d.kind != "cpu") =>
        {
            // Initialization includes first compute. This retry is still before
            // Ready or any accepted microphone audio; never replay user audio.
            drop(model);
            model = load(&file_path, DevicePolicy::Cpu)?;
            warm_stream(&model, token)?
        }
        result => result?,
    };
    Ok(Prepared {
        path,
        sha256,
        device,
        model,
        warmup_ms,
    })
}

fn serve(
    prepared: &Prepared,
    id: &RunId,
    language: &str,
    right_context: i32,
    rx: &mpsc::Receiver<Frame>,
    out: &Output,
    current: &Cancellation,
) -> Result<bool, &'static str> {
    let mut session = prepared.model.session().map_err(|e| error_code(&e))?;
    let token = CancelToken::new();
    session.set_cancel_token(&token);
    *current.lock().map_err(|_| "localProtocol")? = Some((id.clone(), token.clone()));
    let options = run_options(&prepared.model, language)?;
    let stream_options = StreamOptions {
        family: Some(StreamExtension::ParakeetStream(ParakeetStreamOptions {
            att_context_right: Some(right_context),
        })),
        ..Default::default()
    };
    let mut stream = session
        .stream(&options, &stream_options)
        .map_err(|e| error_code(&e))?;
    let mut proof = NativeAppendProof::default();
    let mut sequence = 0;
    emit(out, &Event::Ready { id: id.clone() })?;
    loop {
        if token.is_cancelled() {
            return Err("cancelled");
        }
        let frame = rx.recv().map_err(|_| "cancelled")?;
        match frame {
            Frame::Audio {
                id: audio_id,
                sequence: next,
                pcm,
            } => {
                if audio_id != *id {
                    fail(out, Some(audio_id), "localStaleOperation");
                    continue;
                }
                if next != sequence + 1 {
                    return Err("localAudioSequence");
                }
                sequence = next;
                let samples = pcm.iter().map(|s| *s as f32 / 32768.0).collect::<Vec<_>>();
                let update = stream.feed(&samples).map_err(|e| error_code(&e))?;
                if update.result_changed && !token.is_cancelled() {
                    emit(out, &snapshot(&stream, id, update, &mut proof)?)?;
                }
            }
            frame => match decode_json::<Command>(frame).map_err(|_| "localProtocol")? {
                Command::Finish { id: finish_id } if finish_id == *id => {
                    let update = stream.finalize().map_err(|e| error_code(&e))?;
                    if token.is_cancelled() {
                        return Err("cancelled");
                    }
                    emit(out, &snapshot(&stream, id, update, &mut proof)?)?;
                    emit(out, &Event::Finished { id: id.clone() })?;
                    return Ok(false);
                }
                Command::Cancel { id: cancel_id } if cancel_id == *id => return Err("cancelled"),
                Command::Shutdown => return Ok(true),
                Command::Cancel { .. } | Command::Finish { .. } => {}
                Command::Start { id: other, .. } => fail(out, Some(other), "localBusy"),
                _ => return Err("localProtocol"),
            },
        }
    }
}

fn main() {
    // The worker has no standalone dictation mode or network listener.
    if std::env::args().nth(1).as_deref() != Some("--stdio") {
        std::process::exit(2);
    }
    let out = Arc::new(Mutex::new(BufWriter::new(io::stdout())));
    let current: Cancellation = Arc::new(Mutex::new(None));
    let (tx, rx) = mpsc::sync_channel(32);
    let reader_current = current.clone();
    let reader_out = out.clone();
    thread::spawn(move || {
        let mut input = io::stdin().lock();
        loop {
            let frame = match read_frame(&mut input) {
                Ok(Some(frame)) => frame,
                Ok(None) => break,
                Err(_) => {
                    fail(&reader_out, None, "localProtocol");
                    break;
                }
            };
            let mut shutdown = false;
            if let Frame::Json(ref bytes) = frame {
                match serde_json::from_slice::<Command>(bytes) {
                    Ok(Command::Cancel { ref id }) => cancel_current(&reader_current, Some(id)),
                    Ok(Command::Shutdown) => {
                        cancel_current(&reader_current, None);
                        shutdown = true;
                    }
                    Ok(_) => {}
                    Err(_) => {
                        fail(&reader_out, None, "localProtocol");
                        break;
                    }
                }
            }
            // A full queue is a terminal overload, never a silently discarded frame.
            if tx.try_send(frame).is_err() {
                fail(&reader_out, None, "localBufferOverflow");
                break;
            }
            if shutdown {
                break;
            }
        }
        cancel_current(&reader_current, None);
    });
    let mut prepared: Option<Prepared> = None;
    let mut hello = false;
    let mut retired = VecDeque::new();
    while let Ok(frame) = rx.recv() {
        let command = match idle_command(frame) {
            Ok(command) => command,
            Err((id, code)) => {
                fail(&out, id, code);
                continue;
            }
        };
        if !hello {
            match command {
                Command::Hello { version, instance }
                    if version == VERSION && !instance.is_empty() && instance.len() <= 128 =>
                {
                    hello = true;
                    let _ = emit(
                        &out,
                        &Event::Hello {
                            version: VERSION,
                            instance,
                            engine: transcribe_cpp::version(),
                        },
                    );
                    continue;
                }
                _ => {
                    fail(&out, None, "localRuntimeMismatch");
                    break;
                }
            }
        }
        match command {
            Command::Prepare {
                id,
                path,
                sha256,
                device,
            } => {
                if !valid_id(&id) || retired.contains(&id) {
                    fail(&out, Some(id), "localStaleOperation");
                    continue;
                }
                let token = CancelToken::new();
                if let Ok(mut guard) = current.lock() {
                    *guard = Some((id.clone(), token.clone()));
                }
                if !prepared
                    .as_ref()
                    .is_some_and(|p| p.path == path && p.sha256 == sha256 && p.device == device)
                {
                    prepared = None;
                    match prepare(path, sha256, device, &token) {
                        Ok(model) => prepared = Some(model),
                        Err(code) => {
                            if let Ok(mut guard) = current.lock() {
                                *guard = None;
                            }
                            if code == "cancelled" {
                                let _ = emit(&out, &Event::Cancelled { id: id.clone() });
                                retired.push_back(id);
                                if retired.len() > 64 {
                                    retired.pop_front();
                                }
                            } else {
                                fail(&out, Some(id), code);
                            }
                            continue;
                        }
                    }
                }
                if let Ok(mut guard) = current.lock() {
                    *guard = None;
                }
                if token.is_cancelled() {
                    let _ = emit(&out, &Event::Cancelled { id: id.clone() });
                    retired.push_back(id);
                    if retired.len() > 64 {
                        retired.pop_front();
                    }
                    continue;
                }
                let model = &prepared.as_ref().unwrap().model;
                let actual = model.device().ok();
                let _ = emit(
                    &out,
                    &Event::Prepared {
                        id,
                        backend: actual
                            .as_ref()
                            .map(|d| d.kind.clone())
                            .unwrap_or_else(|| "unknown".into()),
                        device: actual.as_ref().map(|d| {
                            if d.description.is_empty() {
                                d.name.clone()
                            } else {
                                d.description.clone()
                            }
                        }),
                        device_id: actual.as_ref().and_then(|d| d.device_id.clone()),
                        device_memory_total: actual
                            .as_ref()
                            .and_then(|d| (d.memory_total > 0).then_some(d.memory_total)),
                        device_memory_free: actual
                            .as_ref()
                            .and_then(|d| (d.memory_free > 0).then_some(d.memory_free)),
                        streaming: true,
                        warmup_ms: prepared.as_ref().unwrap().warmup_ms,
                    },
                );
            }
            Command::Start {
                id,
                language,
                right_context,
            } => {
                if !valid_id(&id) || retired.contains(&id) {
                    fail(&out, Some(id), "localStaleOperation");
                    continue;
                }
                let Some(model) = &prepared else {
                    fail(&out, Some(id), "localModelMissing");
                    continue;
                };
                let result = serve(model, &id, &language, right_context, &rx, &out, &current);
                if let Ok(mut guard) = current.lock() {
                    *guard = None;
                }
                match result {
                    Ok(true) => break,
                    Ok(false) => {}
                    Err("cancelled") => {
                        let _ = emit(&out, &Event::Cancelled { id: id.clone() });
                    }
                    Err(code) => fail(&out, Some(id.clone()), code),
                }
                retired.push_back(id);
                if retired.len() > 64 {
                    retired.pop_front();
                }
            }
            Command::Cancel { id } if !retired.contains(&id) => {
                let _ = emit(&out, &Event::Cancelled { id: id.clone() });
                retired.push_back(id);
                if retired.len() > 64 {
                    retired.pop_front();
                }
            }
            Command::Cancel { .. } | Command::Finish { .. } => {}
            Command::Unload => {
                prepared = None;
                let _ = emit(&out, &Event::Unloaded);
            }
            Command::Shutdown => break,
            _ => {
                fail(&out, None, "localProtocol");
                break;
            }
        }
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    #[test]
    fn cancelled_pcm_backlog_keeps_its_run_identity_before_next_start() {
        let old = RunId {
            task: "same-task".into(),
            turn: 1,
            request: "old-request".into(),
        };
        let next = RunId {
            turn: 2,
            request: "next-request".into(),
            ..old.clone()
        };
        let mut queue = VecDeque::new();
        for sequence in 1..=4 {
            queue.push_back(Frame::Audio {
                id: old.clone(),
                sequence,
                pcm: vec![1; 1600],
            });
        }
        queue.push_back(Frame::Json(
            serde_json::to_vec(&Command::Cancel { id: old.clone() }).unwrap(),
        ));
        queue.push_back(Frame::Json(
            serde_json::to_vec(&Command::Start {
                id: next.clone(),
                language: "auto".into(),
                right_context: 6,
            })
            .unwrap(),
        ));
        for _ in 0..4 {
            let (id, code) = idle_command(queue.pop_front().unwrap()).unwrap_err();
            assert_eq!(id, Some(old.clone()));
            assert_eq!(code, "localStaleOperation");
        }
        assert!(
            matches!(idle_command(queue.pop_front().unwrap()).unwrap(),Command::Cancel{id} if id==old)
        );
        assert!(
            matches!(idle_command(queue.pop_front().unwrap()).unwrap(),Command::Start{id,..} if id==next)
        );
    }
}
