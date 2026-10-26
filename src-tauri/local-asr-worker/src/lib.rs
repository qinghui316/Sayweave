//! Native inference implementation. Only the isolated worker depends on this crate.
pub mod pause;
pub mod proof;

use sayweave_local_asr_protocol::{DevicePolicy, Event, RunId};
use std::path::Path;
use transcribe_cpp::{
    Backend, CancelToken, Model, ModelOptions, ParakeetStreamOptions, RunOptions, Stream,
    StreamExtension, StreamOptions, StreamUpdate,
};

/// Required production preparation, not an acceptance-only warmup. Exercise
/// first/subsequent/final streaming geometry before admitting microphone audio.
/// Results from this synthetic silence never leave the compute owner.
pub fn warm_stream(model: &Model, token: &CancelToken) -> Result<u64, &'static str> {
    let began = std::time::Instant::now();
    let mut session = model.session().map_err(|e| error_code(&e))?;
    session.set_cancel_token(token);
    let options = run_options(model, "auto")?;
    let parameters = StreamOptions {
        family: Some(StreamExtension::ParakeetStream(ParakeetStreamOptions {
            att_context_right: Some(6),
        })),
        ..Default::default()
    };
    let mut stream = session
        .stream(&options, &parameters)
        .map_err(|e| error_code(&e))?;
    for _ in 0..30 {
        stream.feed(&[0.0; 1600]).map_err(|e| error_code(&e))?;
    }
    stream.finalize().map_err(|e| error_code(&e))?;
    Ok(began.elapsed().as_millis() as u64)
}

pub fn language_hint(
    requested: &str,
    supported: &[String],
) -> Result<Option<String>, &'static str> {
    if requested.is_empty() || requested == "auto" {
        return Ok(None);
    }
    supported
        .iter()
        .find(|s| s.eq_ignore_ascii_case(requested))
        .or_else(|| {
            supported.iter().find(|s| {
                s.to_ascii_lowercase()
                    .starts_with(&format!("{}-", requested.to_ascii_lowercase()))
            })
        })
        .cloned()
        .map(Some)
        .ok_or("localLanguageUnsupported")
}

pub fn load(path: &Path, device: DevicePolicy) -> Result<Model, &'static str> {
    transcribe_cpp::init_backends_default().map_err(|_| "localBackendUnavailable")?;
    // Only initialization failures may retry once on CPU, before any audio is accepted.
    let model = load_with_fallback(device, |choice| {
        Model::load_with(
            path,
            &ModelOptions {
                backend: match choice {
                    DevicePolicy::Auto => Backend::Auto,
                    DevicePolicy::Cpu => Backend::Cpu,
                },
                ..Default::default()
            },
        )
    })
    .map_err(|e| error_code(&e))?;
    if !model.capabilities().supports_streaming || model.capabilities().native_sample_rate != 16000
    {
        return Err("localStreamingUnsupported");
    }
    // This release's native confirmation contract was audited against the
    // irreversible-token Parakeet implementation in exactly this engine version.
    if transcribe_cpp::version() != "0.1.3" || model.arch() != "parakeet" {
        return Err("localConfirmationUnsupported");
    }
    Ok(model)
}
fn load_with_fallback<T>(
    device: DevicePolicy,
    mut attempt: impl FnMut(DevicePolicy) -> transcribe_cpp::Result<T>,
) -> transcribe_cpp::Result<T> {
    match attempt(device) {
        Err(transcribe_cpp::Error::Backend(_) | transcribe_cpp::Error::OutOfMemory(_))
            if device == DevicePolicy::Auto =>
        {
            attempt(DevicePolicy::Cpu)
        }
        result => result,
    }
}

pub fn run_options(model: &Model, language: &str) -> Result<RunOptions, &'static str> {
    Ok(RunOptions {
        language: language_hint(language, &model.capabilities().languages)?,
        ..Default::default()
    })
}

pub fn snapshot(
    stream: &Stream<'_>,
    id: &RunId,
    update: StreamUpdate,
    proof: &mut proof::NativeAppendProof,
) -> Result<Event, &'static str> {
    let text = stream.text();
    let raw = stream.snapshot();
    let tokens = raw.tokens.iter().map(|t| t.id).collect::<Vec<_>>();
    let confirmed = proof.observe(&tokens, &raw.text, &text.full, &text.committed)?;
    Ok(Event::Update {
        id: id.clone(),
        revision: update.revision,
        full: text.full,
        confirmed,
        committed: text.committed,
        tentative: text.tentative,
        native_token_count: tokens.len(),
        audio_received_ms: update.input_received_ms,
        audio_committed_ms: update.audio_committed_ms,
        pause: None,
        final_result: update.is_final,
    })
}

pub fn error_code(error: &transcribe_cpp::Error) -> &'static str {
    match error {
        transcribe_cpp::Error::Aborted { .. } => "cancelled",
        transcribe_cpp::Error::Backend(_) => "localBackendUnavailable",
        transcribe_cpp::Error::OutOfMemory(_) => "localMemoryUnavailable",
        transcribe_cpp::Error::ModelFileNotFound(_) => "localModelMissing",
        transcribe_cpp::Error::Unsupported(_) | transcribe_cpp::Error::NotImplemented(_) => {
            "localStreamingUnsupported"
        }
        transcribe_cpp::Error::BadStructSize(_) | transcribe_cpp::Error::VersionMismatch(_) => {
            "localRuntimeMismatch"
        }
        _ => "localInferenceFailed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn automatic_initialization_tries_cpu_once_for_backend_or_memory_failure_only() {
        for error in [
            transcribe_cpp::Error::Backend("fixture".into()),
            transcribe_cpp::Error::OutOfMemory("fixture".into()),
        ] {
            let mut first = Some(error);
            let mut calls = Vec::new();
            let result = load_with_fallback(DevicePolicy::Auto, |device| {
                calls.push(device);
                first.take().map_or(Ok(()), Err)
            });
            assert!(result.is_ok());
            assert_eq!(calls, [DevicePolicy::Auto, DevicePolicy::Cpu]);
        }
        for (device, error, count) in [
            (
                DevicePolicy::Cpu,
                transcribe_cpp::Error::Backend("fixture".into()),
                1,
            ),
            (
                DevicePolicy::Auto,
                transcribe_cpp::Error::Unsupported("fixture".into()),
                1,
            ),
            (
                DevicePolicy::Auto,
                transcribe_cpp::Error::Backend("fixture".into()),
                2,
            ),
        ] {
            let mut initial = Some(error);
            let mut calls = 0;
            let result: transcribe_cpp::Result<()> = load_with_fallback(device, |_| {
                calls += 1;
                Err(initial.take().unwrap_or_else(|| {
                    transcribe_cpp::Error::Backend("CPU also unavailable".into())
                }))
            });
            assert!(result.is_err());
            assert_eq!(calls, count);
        }
    }
    #[test]
    fn an_engine_version_mismatch_is_not_reported_as_a_network_or_inference_error() {
        assert_eq!(
            error_code(&transcribe_cpp::Error::VersionMismatch("fixture".into())),
            "localRuntimeMismatch"
        );
    }
    #[test]
    fn auto_is_an_absent_hint_and_explicit_languages_use_advertised_locales() {
        let locales = vec!["en-US".into(), "zh-CN".into()];
        assert_eq!(language_hint("auto", &locales), Ok(None));
        assert_eq!(language_hint("zh", &locales), Ok(Some("zh-CN".into())));
        assert_eq!(language_hint("EN-us", &locales), Ok(Some("en-US".into())));
        assert!(language_hint("invalid", &locales).is_err());
    }
}
