//! Opt-in, read-only daily configuration acceptance. Only caller-created
//! synthetic audio is uploaded; no microphone, UI or user text is inspected.
use super::*;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

#[tokio::test]
#[ignore = "Explicit supplier acceptance with saved credentials and synthetic audio"]
async fn live_segmentation() {
    assert_eq!(
        std::env::var("SAYWEAVE_SEGMENTATION_ACCEPTANCE").as_deref(),
        Ok("synthetic-only")
    );
    assert!(std::env::var_os("SAYWEAVE_DATA_ACCEPTANCE_ID").is_none());
    let variant = std::env::var("SAYWEAVE_SEGMENTATION_VARIANT").unwrap();
    assert!(["baseline", "window", "two-pass"].contains(&variant.as_str()));
    let profile = PathBuf::from(std::env::var("APPDATA").unwrap())
        .join("local.sayweave.skeleton/data/dictionary.sqlite");
    let db =
        rusqlite::Connection::open_with_flags(&profile, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .unwrap();
    let (base, reference): (String, String) = db.query_row("SELECT c.base_url,c.credential_ref FROM model_configs c JOIN model_defaults d ON d.config_id=c.id WHERE d.usage='speech' AND c.protocol='doubao-asr-streaming'", [], |r| Ok((r.get(0)?,r.get(1)?))).unwrap();
    drop(db);
    let key = keyring::Entry::new("local.sayweave.skeleton.release", &reference)
        .unwrap()
        .get_password()
        .expect("Daily credential unavailable");
    let wav = std::env::var("SAYWEAVE_SEGMENTATION_WAV").unwrap();
    let mut reader = hound::WavReader::open(wav).unwrap();
    assert_eq!(
        (
            reader.spec().sample_rate,
            reader.spec().channels,
            reader.spec().bits_per_sample
        ),
        (16000, 1, 16)
    );
    let pcm = reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(pcm.len() < 16000 * 90);
    let task = uuid::Uuid::new_v4().to_string();
    let mut config = json!({"user":{"uid":"sayweave"},"audio":{"format":"pcm","codec":"raw","rate":16000,"bits":16,"channel":1},"request":{"model_name":"bigmodel","enable_itn":true,"enable_punc":true,"show_utterances":true,"result_type":"full"}});
    if variant != "baseline" {
        config["request"]["end_window_size"] = json!(800);
    }
    if variant == "two-pass" {
        config["request"]["enable_nonstream"] = json!(true);
    }
    let mut socket = tokio::time::timeout(
        Duration::from_secs(15),
        connect(&base, &key, &task, RESOURCES[0]),
    )
    .await
    .unwrap()
    .map_err(|e| e.code)
    .unwrap();
    drop(key);
    tokio::time::timeout(
        Duration::from_secs(5),
        socket.send(Message::Binary(
            encode(0x10, 0, true, config.to_string().as_bytes())
                .unwrap()
                .into(),
        )),
    )
    .await
    .expect("Configuration send timeout")
    .unwrap();
    let (mut writer, mut reader) = socket.split();
    let started = Instant::now();
    let (finish_tx, mut finish_rx) = tokio::sync::watch::channel(false);
    let mut sending = tokio::spawn(async move {
        for chunk in pcm.chunks(960) {
            let bytes = chunk
                .iter()
                .flat_map(|s| s.to_le_bytes())
                .collect::<Vec<_>>();
            tokio::time::timeout(
                Duration::from_secs(5),
                writer.send(Message::Binary(audio_frame(&bytes, false).unwrap().into())),
            )
            .await
            .expect("Audio send timeout")
            .unwrap();
            tokio::time::sleep(Duration::from_millis(60)).await;
        }
        finish_tx.send_replace(true);
        tokio::time::timeout(
            Duration::from_secs(5),
            writer.send(Message::Binary(audio_frame(&[], true).unwrap().into())),
        )
        .await
        .expect("Finish send timeout")
        .unwrap();
        started.elapsed().as_millis()
    });
    let mut parser = Transcript::default();
    let mut rows = vec![];
    let receive = tokio::time::timeout(Duration::from_secs(105), async {
        while let Some(message) = reader.next().await {
            let message = message.map_err(|_| "network")?;
            if let Message::Binary(bytes) = &message {
                let wire = decode(bytes).map_err(|e|e.code)?;
                let utterances = wire.value.pointer("/result/utterances").and_then(Value::as_array);
                let before_finish = !*finish_rx.borrow_and_update();
                let definite = utterances.map(|v|v.iter().filter(|u|u.get("definite").and_then(Value::as_bool)==Some(true)).count()).unwrap_or(0);
                let wire_text = wire.value.pointer("/result/text").and_then(Value::as_str).unwrap_or("").to_owned();
                parser.event(message).map_err(|e|e.code)?;
                rows.push(json!({"ms":started.elapsed().as_millis(),"beforeFinish":before_finish,"wireDefinite":definite,"utterances":utterances.map(Vec::len),"terminal":wire.last,"mappedConfirmed":parser.text.segments.iter().filter(|s|s.confirmed).count(),"syntheticText":wire_text}));
                if parser.finished { break; }
            }
        }
        Ok::<_,String>(())
    }).await;
    let finish_ms = match tokio::time::timeout(Duration::from_secs(2), &mut sending).await {
        Ok(Ok(ms)) => Some(ms),
        Ok(Err(_)) => None,
        Err(_) => {
            sending.abort();
            let _ = sending.await;
            None
        }
    };
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../.local/segmentation-{variant}.json"));
    let report = json!({"taskId":task,"variant":variant,"syntheticOnly":true,"profileReadOnly":true,"finishMs":finish_ms,"terminal":parser.finished,"rows":rows});
    std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    receive.expect("Supplier timeout").unwrap();
    assert!(parser.finished, "Supplier did not terminate");
    assert!(
        finish_ms.is_some(),
        "Audio sender failed or exceeded its deadline"
    );
    println!("segmentation variant={variant} finish_ms={finish_ms:?} terminal=true");
}
