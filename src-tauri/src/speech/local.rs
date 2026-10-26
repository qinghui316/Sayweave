//! Native model facts enter the existing recognition contract here only.
use super::recognition::{Command, Event, RecognitionSession, Segment, Transcript};
use crate::{history::types::Error, models::LocalConfiguration};
use sayweave_local_asr_protocol::{Event as NativeEvent, RunId};
use tokio::sync::mpsc;

#[derive(Default)]
struct Projection {
    confirmed: String,
    segments: Vec<Segment>,
    next_id: u64,
    revision: i32,
    tail_revision: u64,
    final_seen: bool,
}
impl Projection {
    fn update(&mut self, event: NativeEvent) -> Result<Transcript, Error> {
        let NativeEvent::Update {
            revision,
            full,
            confirmed,
            final_result,
            ..
        } = event
        else {
            return Err(Error::new("localProtocol"));
        };
        // Only verified native confirmation is allowed to advance source IDs.
        if self.final_seen
            || revision < self.revision
            || !confirmed.starts_with(&self.confirmed)
            || !full.starts_with(&confirmed)
            || (final_result && full != confirmed)
        {
            return Err(Error::new("localSourceChanged"));
        }
        self.revision = revision;
        if self.next_id == 0 {
            self.next_id = 1;
        }
        if confirmed.len() > self.confirmed.len() {
            self.tail_revision += 1;
            self.segments.push(Segment {
                id: self.next_id,
                sequence: self.tail_revision,
                text: confirmed[self.confirmed.len()..].into(),
                confirmed: true,
                begin_ms: None,
                end_ms: None,
            });
            self.next_id += 1;
            self.tail_revision = 0;
            self.confirmed = confirmed.clone();
        }
        let mut segments = self.segments.clone();
        if full.len() > confirmed.len() {
            self.tail_revision += 1;
            segments.push(Segment {
                id: self.next_id,
                sequence: self.tail_revision,
                text: full[confirmed.len()..].into(),
                confirmed: false,
                begin_ms: None,
                end_ms: None,
            });
        }
        self.final_seen = final_result;
        Ok(Transcript {
            text: full,
            confirmed_text: confirmed,
            segments,
        })
    }
}

pub async fn open(config: &LocalConfiguration, id: RunId) -> Result<RecognitionSession, Error> {
    let mut native = config
        .runtime
        .open_with_settings(
            id,
            config.model.clone(),
            config.record.input.device,
            &config.settings,
        )
        .await
        .map_err(Error::new)?;
    let (audio, mut commands) = mpsc::channel(32);
    let (send, events) = mpsc::channel(33);
    let failure = send
        .clone()
        .reserve_owned()
        .await
        .map_err(|_| Error::new("localPipeClosed"))?;
    let worker = tokio::spawn(async move {
        let mut projection = Projection::default();
        let mut finishing = false;
        let result=async {
            loop {tokio::select! {
                command=commands.recv()=>match command.ok_or(Error::new("cancelled"))? {
                    Command::Audio(pcm) if !finishing=>native.send_pcm(&pcm).await.map_err(Error::new)?,
                    Command::Finish(ack) if !finishing=>{let result=native.finish().await.map_err(Error::new);finishing=result.is_ok();let _=ack.send(result.clone());result?;},
                    _=>return Err(Error::new("localProtocol")),
                },
                event=native.next_event()=>match event.map_err(Error::new)? {
                    event @ NativeEvent::Update{..}=>{publish(&send,Event::Update(projection.update(event)?)).await?;},
                    NativeEvent::Finished{..} if finishing && projection.final_seen=>{publish(&send,Event::Finished).await?;return Ok(());},
                    _=>return Err(Error::new("localProtocol")),
                }
            }}
        }.await;
        if let Err(error) = result {
            log::warn!("local recognition bridge: {}", error.code);
            failure.send(Err(error));
        }
    });
    Ok(RecognitionSession {
        audio,
        events,
        worker,
        closed_error: "localPipeClosed",
    })
}
async fn publish(send: &mpsc::Sender<Result<Event, Error>>, event: Event) -> Result<(), Error> {
    tokio::time::timeout(std::time::Duration::from_millis(500), send.send(Ok(event)))
        .await
        .map_err(|_| Error::new("localBufferOverflow"))?
        .map_err(|_| Error::new("cancelled"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "explicit isolated pipe-peer binary, built by local-ASR validation"]
    async fn stalled_adapter_consumer_releases_the_stream_and_preserves_local_errors() {
        use std::time::Duration;
        let (_root, runtime, model) = crate::local_asr::test_runtime("ordinary");
        let config = LocalConfiguration::fixture(
            crate::models::types::LocalConfigRecord {
                id: "bounded-local".into(),
                input: crate::models::types::LocalConfigInput {
                    name: "bounded local".into(),
                    asset_id: "fixture".into(),
                    engine: "transcribe-cpp-0.1.3".into(),
                    device: sayweave_local_asr_protocol::DevicePolicy::Cpu,
                },
                revision: 1,
                created_at: 1,
                updated_at: 1,
            },
            model,
            runtime.clone(),
        );
        let id = |turn| RunId {
            task: "same-local-task".into(),
            turn,
            request: uuid::Uuid::new_v4().to_string(),
        };
        let mut session = open(&config, id(1)).await.unwrap();
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
        let settled = tokio::time::timeout(Duration::from_secs(2), async {
            let _ = session.finish().await;
            while !session.worker.is_finished() {
                tokio::task::yield_now().await;
            }
        })
        .await;
        let mut terminal = None;
        if settled.is_ok() {
            while let Some(event) = session.events.recv().await {
                if let Err(error) = event {
                    terminal = Some(error.code);
                }
            }
            assert_eq!(
                session.send_pcm(&[1]).await.unwrap_err().code,
                "localPipeClosed"
            );
        }
        drop(session);
        let mut next = tokio::time::timeout(Duration::from_secs(3), open(&config, id(2)))
            .await
            .unwrap()
            .unwrap();
        next.send_pcm(&[1; 1600]).await.unwrap();
        next.finish().await.unwrap();
        while !matches!(next.next_event().await.unwrap(), Event::Finished) {}
        drop(next);
        drop(config);
        runtime.shutdown().await.unwrap();
        assert!(
            settled.is_ok(),
            "A stalled adapter retained the previous stream"
        );
        assert_eq!(terminal.as_deref(), Some("localBufferOverflow"));
    }
    fn update(revision: i32, full: &str, confirmed: &str, final_result: bool) -> NativeEvent {
        NativeEvent::Update {
            id: RunId {
                task: "t".into(),
                turn: 1,
                request: "r".into(),
            },
            revision,
            full: full.into(),
            confirmed: confirmed.into(),
            committed: full.into(),
            tentative: String::new(),
            native_token_count: 1,
            audio_received_ms: 100,
            audio_committed_ms: 100,
            final_result,
        }
    }
    #[test]
    fn display_text_does_not_confirm_sources_and_verified_ids_remain_immutable() {
        let mut p = Projection::default();
        let a = p.update(update(1, "项目使用SQLit", "", false)).unwrap();
        assert_eq!(a.confirmed_text, "");
        assert!(!a.segments[0].confirmed);
        let b = p
            .update(update(2, "项目使用SQLite", "项目使用SQLite", false))
            .unwrap();
        assert_eq!(b.segments[0].id, a.segments[0].id);
        assert!(b.segments[0].confirmed);
        let c = p
            .update(update(
                3,
                "项目使用SQLite。下一句",
                "项目使用SQLite。",
                false,
            ))
            .unwrap();
        assert_eq!(c.segments[0].text, b.segments[0].text);
        assert_eq!(c.segments[2].text, "下一句");
        assert!(p.update(update(4, "其他正文", "其他正文", true)).is_err());
    }
    #[test]
    fn final_authority_must_include_all_visible_text_and_cannot_revive() {
        let mut p = Projection::default();
        assert!(p.update(update(1, "尾字", "", true)).is_err());
        p.update(update(2, "尾字", "尾字", true)).unwrap();
        assert!(p.update(update(3, "迟到文字", "迟到文字", false)).is_err());
    }
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[ignore = "explicit fixed-audio real model, CPU/Vulkan and worker identities recorded separately"]
    #[cfg(windows)]
    async fn real_worker_through_main_recognition_adapter() {
        use crate::{
            build_info::local_runtime::RuntimeFile, local_asr::Runtime, model_assets,
            models::SpeechConfiguration,
        };
        use sha2::{Digest, Sha256};
        use std::{
            sync::Arc,
            time::{Duration, Instant},
        };
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap();
        let report_id = std::env::var("SAYWEAVE_LOCAL_REPORT")
            .unwrap_or_else(|_| uuid::Uuid::new_v4().to_string());
        assert!(
            !report_id.is_empty()
                && report_id.len() <= 100
                && report_id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c))
        );
        let output = root.join(format!(".local/local-asr/reports/main-{report_id}.json"));
        assert!(
            !output.exists(),
            "Never overwrite earlier benchmark evidence"
        );
        let rounds = std::env::var("SAYWEAVE_LOCAL_ROUNDS")
            .ok()
            .map(|n| n.parse::<u64>().unwrap())
            .unwrap_or(2);
        assert!((1..=8).contains(&rounds));
        let directory = std::env::var_os("SAYWEAVE_LOCAL_RUNTIME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join("src-tauri/target/debug/local-asr"));
        let program = directory.join("sayweave-local-asr-worker.exe");
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap())
                .unwrap();
        let inventory = manifest["files"]
            .as_object()
            .unwrap()
            .iter()
            .map(|(name, item)| RuntimeFile {
                name: name.clone(),
                bytes: item["bytes"].as_u64().unwrap(),
                sha256: item["sha256"].as_str().unwrap().into(),
            })
            .collect();
        let runtime = Runtime::new(program.clone(), inventory);
        runtime.select(true);
        let model = model_assets::ModelLease::acquire(
            &root
                .join(".local/local-asr/models")
                .join(model_assets::MODEL_FILE),
            model_assets::MODEL_BYTES,
            model_assets::MODEL_SHA256,
        )
        .unwrap();
        let requested = std::env::var("SAYWEAVE_LOCAL_DEVICE").unwrap_or_else(|_| "cpu".into());
        let device = if requested == "auto" {
            sayweave_local_asr_protocol::DevicePolicy::Auto
        } else {
            sayweave_local_asr_protocol::DevicePolicy::Cpu
        };
        let config: SpeechConfiguration =
            SpeechConfiguration::Local(Arc::new(crate::models::LocalConfiguration::fixture(
                crate::models::types::LocalConfigRecord {
                    id: uuid::Uuid::new_v4().to_string(),
                    input: crate::models::types::LocalConfigInput {
                        name: "native acceptance".into(),
                        asset_id: model_assets::MODEL_ID.into(),
                        engine: "transcribe-cpp-0.1.3".into(),
                        device,
                    },
                    revision: 1,
                    created_at: 1,
                    updated_at: 1,
                },
                model,
                runtime.clone(),
            )));
        let task = uuid::Uuid::new_v4().to_string();
        let preparing = Instant::now();
        tokio::time::timeout(Duration::from_secs(60), config.prepare(&task))
            .await
            .unwrap()
            .unwrap();
        let prepare_ms = preparing.elapsed().as_secs_f64() * 1000.0;
        let metadata = runtime.preparation().await.unwrap();
        if let Ok(expected) = std::env::var("SAYWEAVE_LOCAL_EXPECT_BACKEND") {
            assert!(matches!(&metadata,NativeEvent::Prepared{backend,..} if backend==&expected));
        }
        let audio = std::env::var_os("SAYWEAVE_LOCAL_AUDIO")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| root.join(".local/local-asr/fixtures/zh-mixed-sapi.wav"));
        let mut wav = hound::WavReader::open(&audio).unwrap();
        assert_eq!(wav.spec().sample_rate, 16000);
        assert_eq!(wav.spec().channels, 1);
        let pcm = wav.samples::<i16>().collect::<Result<Vec<_>, _>>().unwrap();
        let audio_seconds = pcm.len() as f64 / 16000.0;
        assert!(audio_seconds > 1.0 && audio_seconds <= 120.0);
        let mut runs = Vec::new();
        for turn in 1..=rounds {
            let mut session = RecognitionSession::open_for_turn(
                &config,
                &task,
                turn,
                &uuid::Uuid::new_v4().to_string(),
                &[],
            )
            .await
            .unwrap();
            let began = Instant::now();
            let mut at = 0;
            let mut final_at = None;
            let mut updates = 0;
            let mut first = None;
            let mut text = String::new();
            let mut progress = Vec::new();
            let mut confirmed = String::new();
            let mut last_pcm_ms = 0.0;
            let start = tokio::time::Instant::now() + Duration::from_millis(100);
            let mut tick = tokio::time::interval_at(start, Duration::from_millis(100));
            // Keep capture deadlines anchored. Windows timer granularity must
            // not accumulate a delay on every packet and slow the test audio.
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
            tokio::time::timeout(Duration::from_secs_f64(audio_seconds + 15.0),async {
                loop {tokio::select! {
                    event=session.next_event()=>match event.unwrap() {
                        Event::Update(value)=>{
                            assert!(value.confirmed_text.starts_with(&confirmed));confirmed=value.confirmed_text.clone();
                            assert_eq!(value.segments.iter().filter(|s|s.confirmed).map(|s|s.text.as_str()).collect::<String>(),value.confirmed_text);
                            assert_eq!(value.segments.iter().map(|s|s.text.as_str()).collect::<String>(),value.text);
                            if !value.text.is_empty()&&first.is_none(){assert!(final_at.is_none());first=Some(began.elapsed().as_secs_f64()*1000.0);}
                            if text!=value.text {progress.push(began.elapsed().as_secs_f64()*1000.0);}text=value.text;updates+=1;
                        },
                        Event::Finished=>{assert!(final_at.is_some());break;},Event::Heartbeat=>{},
                    },
                    _=tick.tick(),if final_at.is_none()=>{
                        if at<pcm.len(){let end=(at+1600).min(pcm.len());session.send_pcm(&pcm[at..end]).await.unwrap();at=end;last_pcm_ms=began.elapsed().as_secs_f64()*1000.0;}
                        else {final_at=Some(Instant::now());session.finish().await.unwrap();}
                    },
                }}
            }).await.unwrap();
            assert!(updates > 2);
            assert_eq!(text, confirmed);
            assert_eq!(at, pcm.len());
            runs.push(serde_json::json!({"turn":turn,"updates":updates,"firstTextMs":first.unwrap(),"finalizeMs":final_at.unwrap().elapsed().as_secs_f64()*1000.0,"lastPcmMs":last_pcm_ms,"elapsedMs":began.elapsed().as_secs_f64()*1000.0,"full":text,"progressMs":progress}));
            drop(session);
        }
        // Exercise the production dispatcher with queued PCM at cooperative
        // cancellation, then reuse the same model/runtime for the next turn.
        // This runs AFTER timing samples and cannot warm their first decode.
        let cancel_began = Instant::now();
        tokio::time::timeout(Duration::from_secs(60), async {
            let mut old = RecognitionSession::open_for_turn(
                &config,
                &task,
                rounds + 1,
                &uuid::Uuid::new_v4().to_string(),
                &[],
            )
            .await
            .unwrap();
            for chunk in pcm.chunks(1600).take(24) {
                old.send_pcm(chunk).await.unwrap();
            }
            eprintln!("local_cancel_acceptance=waiting_first_update_after_2400ms_audio");
            assert!(matches!(old.next_event().await.unwrap(), Event::Update(_)));
            eprintln!("local_cancel_acceptance=first_update_received");
            for chunk in pcm.chunks(1600).take(8) {
                old.send_pcm(chunk).await.unwrap();
            }
            drop(old);
            let mut next = RecognitionSession::open_for_turn(
                &config,
                &task,
                rounds + 2,
                &uuid::Uuid::new_v4().to_string(),
                &[],
            )
            .await
            .unwrap();
            for chunk in pcm.chunks(1600).take(16) {
                next.send_pcm(chunk).await.unwrap();
            }
            next.finish().await.unwrap();
            while !matches!(next.next_event().await.unwrap(), Event::Finished) {}
        })
        .await
        .unwrap();
        let cancellation_ms = cancel_began.elapsed().as_secs_f64() * 1000.0;
        drop(config);
        let memory = runtime.process_memory().await;
        runtime.shutdown().await.unwrap();
        let worker_sha = format!("{:x}", Sha256::digest(std::fs::read(program).unwrap()));
        let report = serde_json::json!({"kind":"main-recognition-real-worker-fixed-audio","audioPacing":"absolute-100ms","audioFile":audio,"audioSha256":format!("{:x}", Sha256::digest(std::fs::read(&audio).unwrap())),"durationSeconds":audio_seconds,"syntheticAudio":audio.file_name().is_some_and(|n|n.to_string_lossy().contains("sapi")),"requestedDevice":requested,"prepared":metadata,"prepareMs":prepare_ms,"workerSha256":worker_sha,"testDriverSha256":format!("{:x}", Sha256::digest(std::fs::read(std::env::current_exe().unwrap()).unwrap())),"runtimeManifest":manifest,"modelSha256":model_assets::MODEL_SHA256,"processMemory":memory,"runs":runs,"cancellationNextTurnPassed":true,"cancellationRecoveryMs":cancellation_ms,"processAndComputeClosed":true});
        std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
