//! Developer-only real-time feed probe. Never built into the production worker.
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
use transcribe_cpp::{
    Backend, Model, ModelOptions, ParakeetStreamOptions, RunOptions, StreamExtension, StreamOptions,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let model_path = PathBuf::from(args.next().ok_or("model path required")?);
    let audio_path = PathBuf::from(args.next().ok_or("16kHz mono WAV required")?);
    let backend = match args.next().as_deref() {
        Some("cpu") => Backend::Cpu,
        _ => Backend::Auto,
    };
    let real_time = args.next().as_deref() != Some("fast");
    let mut wav = hound::WavReader::open(audio_path)?;
    let spec = wav.spec();
    if spec.sample_rate != 16_000
        || spec.channels != 1
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        return Err("expected 16kHz mono PCM16".into());
    }
    let pcm = wav
        .samples::<i16>()
        .map(|s| s.map(|s| s as f32 / 32768.0))
        .collect::<Result<Vec<_>, _>>()?;
    let load = Instant::now();
    transcribe_cpp::init_backends_default()?;
    let model = Model::load_with(
        &model_path,
        &ModelOptions {
            backend,
            ..Default::default()
        },
    )?;
    if !model.capabilities().supports_streaming {
        return Err("model lacks streaming".into());
    }
    let mut session = model.session()?;
    // The engine represents automatic language with a missing hint, not the
    // literal locale "auto". Explicit hints must use advertised locale names.
    let options = RunOptions::default();
    let stream_options = StreamOptions {
        family: Some(StreamExtension::ParakeetStream(ParakeetStreamOptions {
            att_context_right: Some(6),
        })),
        ..Default::default()
    };
    let mut stream = session.stream(&options, &stream_options)?;
    let ready_ms = load.elapsed().as_millis();
    let began = Instant::now();
    let mut previous = String::new();
    let mut previous_tokens = Vec::new();
    let mut updates = Vec::new();
    for (index, chunk) in pcm.chunks(1600).enumerate() {
        if real_time {
            let due = Duration::from_millis(index as u64 * 100);
            if let Some(delay) = due.checked_sub(began.elapsed()) {
                std::thread::sleep(delay);
            }
        }
        let decode = Instant::now();
        let update = stream.feed(chunk)?;
        if update.result_changed {
            let text = stream.text();
            let raw = stream.snapshot();
            let tokens = raw.tokens.iter().map(|token| token.id).collect::<Vec<_>>();
            let append_only = tokens.starts_with(&previous_tokens);
            let compatible = text.full.starts_with(&text.committed);
            if !text.full.is_empty() && text.full != previous {
                updates.push(serde_json::json!({"elapsedMs":began.elapsed().as_millis(),"inputReceivedMs":update.input_received_ms,"audioCommittedMs":update.audio_committed_ms,"revision":update.revision,"feedMs":decode.elapsed().as_millis(),"full":text.full,"committed":text.committed,"tentative":text.tentative,"committedIsRawPrefix":compatible,"nativeTokenCount":tokens.len(),"tokensAppendOnly":append_only}));
            }
            if !compatible {
                return Err("display commitment diverged from authoritative hypothesis".into());
            }
            if !append_only {
                return Err("model-native token prefix revised".into());
            }
            previous = text.full;
            previous_tokens = tokens;
        }
    }
    let ending = Instant::now();
    let final_update = stream.finalize()?;
    let final_text = stream.text();
    println!(
        "{}",
        serde_json::to_string_pretty(
            &serde_json::json!({"kind":"real-model-stream-probe","engine":transcribe_cpp::version(),"backend":format!("{:?}",model.backend()),"device":model.device().ok().map(|d|d.name),"realTimeFeed":real_time,"readyMs":ready_ms,"durationMs":pcm.len()*1000/16000,"elapsedMs":began.elapsed().as_millis(),"finalizeMs":ending.elapsed().as_millis(),"isFinal":final_update.is_final,"full":final_text.full,"committed":final_text.committed,"tentative":final_text.tentative,"updates":updates})
        )?
    );
    Ok(())
}
