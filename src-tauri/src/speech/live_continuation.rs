//! Explicit synthetic-audio acceptance through production recognition, draft
//! actor, real selected LLM and an owned editor. No daily history/config writes.
use super::{
    NativeTranscript,
    continuous::Control,
    recognition::{Event, RecognitionSession},
};
use crate::{
    dictation::{drafts::SessionDraft, processing_context::Context},
    models::{
        ExecutableConfiguration,
        provider::Provider,
        types::{ConfigInput, Usage},
    },
    windows_input::{
        DeliveryGate, WindowsInputService,
        composition::{Draft, Finish},
    },
};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Child, ChildStdin, Command, Stdio},
    sync::{Arc, mpsc},
    time::{Duration, Instant},
};

struct Editor {
    child: Child,
    input: ChildStdin,
    output: mpsc::Receiver<String>,
}
struct InputLease {
    service: WindowsInputService,
    task: String,
    closed: bool,
}
impl InputLease {
    async fn close(&mut self) {
        self.service.forget(self.task.clone());
        self.service.shutdown();
        self.closed = true;
        tokio::time::timeout(Duration::from_secs(5), self.service.drain())
            .await
            .expect("Input worker cleanup deadline");
    }
}
impl Drop for InputLease {
    fn drop(&mut self) {
        if !self.closed {
            self.service.forget(self.task.clone());
            self.service.shutdown();
        }
    }
}
struct ActorWorker(tokio::task::JoinHandle<()>);
impl Drop for ActorWorker {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl Editor {
    fn open() -> Self {
        use std::os::windows::process::CommandExt;
        let exe = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/input-fixture.exe");
        let mut child = Command::new(exe)
            .arg("composition")
            .env("SAYWEAVE_INPUT_ACCEPTANCE", "1")
            .creation_flags(0x08000000)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let out = child.stdout.take().unwrap();
        let (tx, output) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        unsafe {
            let _ = windows::Win32::UI::WindowsAndMessaging::AllowSetForegroundWindow(child.id());
        }
        let editor = Self {
            child,
            input,
            output,
        };
        editor.read();
        editor
    }
    fn read(&self) -> Value {
        serde_json::from_str(
            &self
                .output
                .recv_timeout(Duration::from_secs(3))
                .expect("owned editor response"),
        )
        .unwrap()
    }
    fn command(&mut self, value: Value) -> Value {
        writeln!(self.input, "{value}").unwrap();
        self.input.flush().unwrap();
        self.read()
    }
}
impl Drop for Editor {
    fn drop(&mut self) {
        let _ = writeln!(self.input, "{{\"action\":\"exit\"}}");
        let _ = self.input.flush();
        for _ in 0..30 {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn configuration(db: &rusqlite::Connection, usage: &str) -> ExecutableConfiguration {
    let (id,revision,input,reference)=db.query_row("SELECT c.id,c.revision,c.name,c.protocol,c.base_url,c.model_id,c.credential_ref FROM model_defaults d JOIN model_configs c ON c.id=d.config_id WHERE d.usage=?1",[usage],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,ConfigInput{name:r.get(2)?,protocol:r.get(3)?,base_url:r.get(4)?,model_id:r.get(5)?,usages:vec![if usage=="speech"{Usage::Speech}else{Usage::Text}]},r.get::<_,String>(6)?))).unwrap();
    let key = keyring::Entry::new("local.sayweave.skeleton.release", &reference)
        .unwrap()
        .get_password()
        .expect("Daily credential unavailable");
    ExecutableConfiguration {
        id,
        revision,
        input,
        key,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 3)]
#[ignore = "Explicit real ASR/LLM and foreground owned editor; synthetic audio only"]
async fn live_continuation() {
    assert_eq!(
        std::env::var("SAYWEAVE_LIVE_CONTINUATION").as_deref(),
        Ok("synthetic-only")
    );
    assert!(std::env::var_os("SAYWEAVE_DATA_ACCEPTANCE_ID").is_none());
    let profile = PathBuf::from(std::env::var("APPDATA").unwrap())
        .join("local.sayweave.skeleton/data/dictionary.sqlite");
    let db =
        rusqlite::Connection::open_with_flags(profile, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let speech = configuration(&db, "speech");
    let text = configuration(&db, "text");
    let qwen_count: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM model_configs WHERE protocol='qwen-audio-streaming'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(db);
    let mut wave =
        hound::WavReader::open(std::env::var("SAYWEAVE_SEGMENTATION_WAV").unwrap()).unwrap();
    assert_eq!(
        (
            wave.spec().sample_rate,
            wave.spec().channels,
            wave.spec().bits_per_sample
        ),
        (16000, 1, 16)
    );
    let pcm = wave
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(pcm.len() < 90 * 16000);
    let task = uuid::Uuid::new_v4().to_string();
    let root = tempfile::tempdir().unwrap();
    let data = crate::data::DataWorker::start(root.path().to_path_buf()).unwrap();
    let provider = Provider::new(Arc::new(crate::network::RequestCoordinator::default())).unwrap();
    let mut context = Context::fixture(Arc::new(provider), text.input.clone(), data.clone());
    context.task_id = task.clone();
    context.configuration = Arc::new(text);
    context.attach(None, None).await.unwrap();
    let mut recognition = RecognitionSession::open(&speech, &task).await.unwrap();
    let mut editor = Editor::open();
    editor.command(json!({"action":"focus","text":"","start":0,"end":0}));
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut input = InputLease {
        service: WindowsInputService::new(),
        task: task.clone(),
        closed: false,
    };
    let anchor = WindowsInputService::anchor().expect("owned target foreground");
    assert_eq!(
        anchor.process,
        editor.child.id(),
        "Never bind a personal foreground application in acceptance"
    );
    input.service.bind_target(&task, &anchor, None);
    let draft = Draft::begin(
        task.clone(),
        1,
        anchor.clone(),
        Arc::new(DeliveryGate::default()),
    )
    .await
    .unwrap();
    let control = Control::new(crate::input_state::activity_evidence());
    let (cancel, _) = tokio::sync::watch::channel(0);
    let actor = SessionDraft::new(
        task.clone(),
        anchor,
        draft,
        control.clone(),
        cancel,
        Some(context),
        None,
    );
    actor.activate();
    let mut worker = ActorWorker(tokio::spawn(actor.clone().maintain(|_| {})));
    let started = Instant::now();
    let mut audio = tokio::time::interval(Duration::from_millis(60));
    audio.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut observe = tokio::time::interval(Duration::from_millis(100));
    observe.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut at = 0;
    let mut finish_ms = None;
    let mut latest = NativeTranscript {
        task_id: task.clone(),
        turn: 1,
        ..Default::default()
    };
    let mut samples = vec![];
    let mut boundaries = vec![];
    let outcome=tokio::time::timeout(Duration::from_secs(110),async {
        loop {tokio::select!{
            _=audio.tick(),if finish_ms.is_none()=>{
                if at<pcm.len(){let end=(at+960).min(pcm.len());recognition.send_pcm(&pcm[at..end]).await?;at=end;}
                else{finish_ms=Some(started.elapsed().as_millis());control.request_finish();recognition.finish().await?;}
            },
            _=observe.tick()=>{let value=editor.command(json!({"action":"read"}));samples.push(json!({"ms":started.elapsed().as_millis(),"beforeFinish":finish_ms.is_none(),"text":value["text"]}));},
            event=recognition.next_event()=>{match event?{
                Event::Update(value)=>{
                    let segments=value.segments.iter().map(|s|{let previous=latest.segments.iter().find(|v|v.id==s.id);crate::contracts::processing::SourceSegment{id:s.id,text:s.text.clone(),confirmed:s.confirmed,revision:previous.map(|v|v.revision+u64::from(v.text!=s.text)).unwrap_or(1)}}).collect();
                    latest=NativeTranscript{task_id:task.clone(),turn:1,sequence:latest.sequence+1,text:value.text,segments};
                    boundaries.push(json!({"ms":started.elapsed().as_millis(),"beforeFinish":finish_ms.is_none(),"confirmed":latest.segments.iter().filter(|s|s.confirmed).count()}));actor.update(&latest);
                },
                Event::Finished=>break,
                Event::Heartbeat=>{}
            }}
        }}
        Ok::<_,crate::history::types::Error>(())
    }).await;
    let stream_ok = matches!(outcome, Ok(Ok(())));
    let delivery = tokio::time::timeout(
        Duration::from_secs(30),
        actor.finish(
            &latest,
            if stream_ok {
                Finish::Commit
            } else {
                Finish::Abort
            },
        ),
    )
    .await
    .expect("Draft finish deadline");
    tokio::time::timeout(Duration::from_secs(5), &mut worker.0)
        .await
        .expect("Actor cleanup deadline")
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let actual = editor.command(json!({"action":"read"}))["text"]
        .as_str()
        .unwrap()
        .to_owned();
    let report = actor.report().unwrap();
    let mut prefix = String::new();
    let mut cleaned_before_finish = 0;
    let units=report.units.iter().map(|u|{if u.application=="applied"{prefix.push_str(u.applied_text.as_deref().unwrap_or(""));}let observed=u.generation=="success"&&u.application=="applied"&&u.candidate.as_deref()!=Some(u.unit.text.as_str())&&samples.iter().any(|s|s["beforeFinish"]==true&&s["text"].as_str().is_some_and(|v|v.starts_with(&prefix)));if observed{cleaned_before_finish+=1;}json!({"id":u.unit.id,"generation":u.generation,"application":u.application,"source":u.unit.text,"candidate":u.candidate,"applied":u.applied_text,"observedBeforeFinish":observed,"error":u.error})}).collect::<Vec<_>>();
    let result = json!({"taskId":task,"syntheticOnly":true,"dailyDataReadOnly":true,"qwenConfigurations":qwen_count,"finishMs":finish_ms,"delivery":delivery,"streamOk":stream_ok,"units":units,"samples":samples,"boundaries":boundaries,"actual":actual,"expected":report.text,"cleanedBeforeFinish":cleaned_before_finish});
    std::fs::write(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../.local/live-continuation.json"),
        serde_json::to_vec_pretty(&result).unwrap(),
    )
    .unwrap();
    input.close().await;
    drop(editor);
    drop(actor);
    data.shutdown().await;
    assert!(stream_ok, "Recognition failed");
    assert_eq!(
        actual, report.text,
        "Actual target text must match settled report"
    );
    assert!(
        cleaned_before_finish >= 2,
        "Two real cleaned prefixes must be visible before Finish"
    );
}
