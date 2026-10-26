//! Selected speech supplier, identical caller-created synthetic audio, read-only daily DB.
use super::recognition::{Event, RecognitionSession};
use crate::models::{
    ExecutableConfiguration,
    types::{ConfigInput, Usage},
};
use serde_json::json;
use std::{path::PathBuf, time::Duration};
/// Integration-fixture helper: the speech owner keeps the supplier connection
/// and decoding private. This is compiled only in the existing test module.
pub(crate) async fn learned_term_followup(
    cfg: &ExecutableConfiguration,
    path: &std::path::Path,
    words: &[String],
) -> String {
    let mut wav = hound::WavReader::open(path).unwrap();
    assert_eq!(
        (
            wav.spec().sample_rate,
            wav.spec().channels,
            wav.spec().bits_per_sample
        ),
        (16000, 1, 16)
    );
    let pcm = wav.samples::<i16>().collect::<Result<Vec<_>, _>>().unwrap();
    assert!(pcm.len() < 16000 * 30);
    let mut speech =
        RecognitionSession::open_with_hotwords(cfg, &uuid::Uuid::new_v4().to_string(), words)
            .await
            .unwrap();
    let mut position = 0;
    let mut finished = false;
    let mut actual = String::new();
    let mut timer = tokio::time::interval(Duration::from_millis(60));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    tokio::time::timeout(Duration::from_secs(45),async {loop{tokio::select!{
        _=timer.tick(),if !finished=>{if position<pcm.len(){let end=(position+960).min(pcm.len());speech.send_pcm(&pcm[position..end]).await.unwrap();position=end;}else{speech.finish().await.unwrap();finished=true;}},
        event=speech.next_event()=>match event.unwrap(){Event::Update(v)=>actual=v.text(false),Event::Finished=>break,Event::Heartbeat=>(),}
    }}}).await.unwrap();
    drop(speech);
    actual
}
#[tokio::test]
#[ignore = "Explicit authorized ASR hotword comparison, synthetic audio only"]
async fn selected_hotword_comparison() {
    crate::tls::initialize();
    assert_eq!(
        std::env::var("SAYWEAVE_HOTWORDS_ACCEPTANCE").as_deref(),
        Ok("synthetic-only")
    );
    assert!(std::env::var_os("SAYWEAVE_DATA_ACCEPTANCE_ID").is_none());
    let profile = PathBuf::from(std::env::var("APPDATA").unwrap())
        .join("local.sayweave.skeleton/data/dictionary.sqlite");
    let db =
        rusqlite::Connection::open_with_flags(profile, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let (id,revision,input,reference)=db.query_row("SELECT c.id,c.revision,c.name,c.protocol,c.base_url,c.model_id,c.credential_ref FROM model_configs c JOIN model_defaults d ON d.config_id=c.id WHERE d.usage='speech'",[],|r|Ok((r.get::<_,String>(0)?,r.get::<_,i64>(1)?,ConfigInput{name:r.get(2)?,protocol:r.get(3)?,base_url:r.get(4)?,model_id:r.get(5)?,usages:vec![Usage::Speech]},r.get::<_,String>(6)?))).unwrap();
    let qwen: i64 = db
        .query_row(
            "SELECT count(*) FROM model_configs WHERE protocol='qwen-audio-streaming'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    let key = keyring::Entry::new("local.sayweave.skeleton.release", &reference)
        .unwrap()
        .get_password()
        .expect("Selected release speech credential unavailable");
    let cfg = ExecutableConfiguration {
        id: id.clone(),
        revision,
        input: input.validate().unwrap(),
        key,
    };
    let mut wav = hound::WavReader::open(std::env::var("SAYWEAVE_HOTWORDS_WAV").unwrap()).unwrap();
    assert_eq!(
        (
            wav.spec().sample_rate,
            wav.spec().channels,
            wav.spec().bits_per_sample
        ),
        (16000, 1, 16)
    );
    let pcm = wav.samples::<i16>().collect::<Result<Vec<_>, _>>().unwrap();
    assert!(pcm.len() < 16000 * 30);
    let mut report = json!({"syntheticOnly":true,"dailyProfileReadOnly":true,"configId":id,"configRevision":revision,"protocol":cfg.input.protocol,"qwenConfigurations":qwen,"samples":pcm.len(),"variants":[]});
    let chinese = std::env::var("SAYWEAVE_HOTWORDS_CASE").as_deref() == Ok("chinese");
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(if chinese {
        "../.local/language-hotwords-chinese.json"
    } else {
        "../.local/language-hotwords-live.json"
    });
    let hints = if chinese {
        vec!["绮蓝引擎".into()]
    } else {
        vec!["Sayweave".into()]
    };
    for (variant, words) in [("baseline", vec![]), ("hotwords", hints)] {
        let began = std::time::Instant::now();
        let task = uuid::Uuid::new_v4().to_string();
        let mut session = RecognitionSession::open_with_hotwords(&cfg, &task, &words)
            .await
            .map_err(|e| e.code)
            .unwrap();
        let ready = began.elapsed().as_millis();
        let mut position = 0;
        let mut finished = false;
        let mut text = String::new();
        let mut clock = tokio::time::interval(Duration::from_millis(60));
        clock.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut confirmed_before_finish = false;
        let result=tokio::time::timeout(Duration::from_secs(45),async {
            loop {tokio::select! {
                _=clock.tick(),if !finished=>{
                    if position<pcm.len(){let end=(position+960).min(pcm.len()); session.send_pcm(&pcm[position..end]).await?;position=end;}
                    else {session.finish().await?;finished=true;}
                },
                event=session.next_event()=>match event? {
                    Event::Update(value)=>{text=value.text(false);confirmed_before_finish|=!finished && value.segments.iter().any(|s|s.confirmed);},
                    Event::Finished=>return Ok::<_,crate::history::types::Error>(()),
                    Event::Heartbeat=>(),
                }
            }}
        }).await;
        report["variants"].as_array_mut().unwrap().push(json!({"variant":variant,"hints":words,"readyMs":ready,"elapsedMs":began.elapsed().as_millis(),"text":text,"confirmedBeforeFinish":confirmed_before_finish,"passed":matches!(result,Ok(Ok(())))}));
        std::fs::write(&output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        drop(session);
        result
            .expect("Supplier deadline")
            .map_err(|e| e.code)
            .unwrap();
    }
}
