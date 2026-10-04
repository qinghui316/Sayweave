use crate::history::{files::AudioFiles, types::Error};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}
pub fn mono<T: cpal::Sample>(frame: &[T]) -> f32
where
    f32: cpal::FromSample<T>,
{
    frame
        .iter()
        .map(|sample| sample.to_sample::<f32>())
        .sum::<f32>()
        / frame.len() as f32
}
pub fn devices() -> Result<Vec<Device>, Error> {
    let host = cpal::default_host();
    let default = host
        .default_input_device()
        .and_then(|d| d.id().ok())
        .map(|i| i.to_string());
    host.input_devices()
        .map_err(|_| Error::new("deviceAccess"))?
        .map(|d| {
            let id = d.id().map_err(|_| Error::new("deviceAccess"))?.to_string();
            Ok(Device {
                is_default: Some(&id) == default.as_ref(),
                id,
                name: d
                    .description()
                    .map_err(|_| Error::new("deviceAccess"))?
                    .name()
                    .into(),
            })
        })
        .collect()
}
pub struct Converter {
    resampler: Fft<f32>,
    pending: VecDeque<f32>,
    rate: usize,
    total: usize,
    emitted: usize,
    skip: usize,
}
impl Converter {
    pub fn new(rate: usize) -> Result<Self, Error> {
        if !(8000..=192000).contains(&rate) {
            return Err(Error::new("deviceFormat"));
        }
        let r = Fft::<f32>::new(rate, 16000, rate / 50, 1, FixedSync::Both)
            .map_err(|_| Error::new("deviceFormat"))?;
        let skip = r.output_delay();
        Ok(Self {
            resampler: r,
            pending: VecDeque::new(),
            rate,
            total: 0,
            emitted: 0,
            skip,
        })
    }
    fn chunk(&mut self, n: usize) -> Result<Vec<i16>, Error> {
        let frames = self.resampler.input_frames_next();
        let mut input = vec![0.0; frames];
        for v in input.iter_mut().take(n) {
            *v = self.pending.pop_front().unwrap_or(0.0);
        }
        let mut output = vec![0.0; self.resampler.output_frames_next()];
        let i = InterleavedSlice::new(&input, 1, frames).map_err(|_| Error::new("deviceFormat"))?;
        let out_frames = output.len();
        let mut o = InterleavedSlice::new_mut(&mut output, 1, out_frames)
            .map_err(|_| Error::new("deviceFormat"))?;
        let (_, written) = self
            .resampler
            .process_into_buffer(&i, &mut o, Some(&Indexing::new().partial_len(n)))
            .map_err(|_| Error::new("deviceFormat"))?;
        let skip = self.skip.min(written);
        self.skip -= skip;
        Ok(output[skip..written]
            .iter()
            .map(|v| ((v.clamp(-1.0, 1.0) * 32767.0).round()) as i16)
            .collect())
    }
    pub fn push(&mut self, samples: &[f32]) -> Result<Vec<i16>, Error> {
        self.total += samples.len();
        self.pending.extend(samples);
        let mut out = vec![];
        while self.pending.len() >= self.resampler.input_frames_next() {
            let n = self.resampler.input_frames_next();
            out.extend(self.chunk(n)?);
        }
        self.emitted += out.len();
        Ok(out)
    }
    pub fn finish(&mut self) -> Result<Vec<i16>, Error> {
        let target = self.total * 16000 / self.rate;
        let mut out = vec![];
        while self.emitted + out.len() < target {
            let n = self.pending.len().min(self.resampler.input_frames_next());
            out.extend(self.chunk(n)?);
        }
        out.truncate(target.saturating_sub(self.emitted));
        self.emitted += out.len();
        Ok(out)
    }
}
pub struct Packet {
    pub pcm: Vec<i16>,
    pub level: f32,
    pub duration_ms: u64,
}
pub struct CaptureResult {
    pub pcm: Vec<i16>,
    pub error: Option<String>,
    pub save_error: bool,
    pub bytes: u64,
    pub peak: usize,
}
pub struct Capture {
    stop: Arc<AtomicBool>,
    pub packets: mpsc::Receiver<Packet>,
    pub done: oneshot::Receiver<CaptureResult>,
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Capture {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
    pub async fn start(
        device_id: Option<String>,
        file: Option<(AudioFiles, String)>,
    ) -> Result<Self, Error> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopper = stop.clone();
        let (sender, packets) = mpsc::channel(100);
        let (done_tx, done) = oneshot::channel();
        let (ready_tx, ready) = oneshot::channel();
        std::thread::Builder::new()
            .name("sayweave-capture".into())
            .spawn(move || {
                let mut ready_tx = Some(ready_tx);
                let result = (|| -> Result<CaptureResult, Error> {
                    let host = cpal::default_host();
                    let device = match device_id {
                        Some(id) => host
                            .device_by_id(&id.parse().map_err(|_| Error::new("deviceMissing"))?)
                            .ok_or(Error::new("deviceMissing"))?,
                        None => host
                            .default_input_device()
                            .ok_or(Error::new("deviceMissing"))?,
                    };
                    let supported = device
                        .default_input_config()
                        .map_err(|_| Error::new("deviceAccess"))?;
                    let config: supported_config::Config = supported.config();
                    let rate = config.sample_rate as usize;
                    let channels = config.channels as usize;
                    log::info!(target:"sayweave","SAYWEAVE_SPEECH operation=capture_prepare input_rate={} channels={} format={:?}",rate,channels,supported.sample_format());
                    if channels == 0 || channels > 32 {
                        return Err(Error::new("deviceFormat"));
                    }
                    let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(rate * 2);
                    let overflow = Arc::new(AtomicBool::new(false));
                    let failure = Arc::new(AtomicBool::new(false));
                    macro_rules! stream {
                        ($t:ty) => {{
                            let o = overflow.clone();
                            let f = failure.clone();
                            device
                                .build_input_stream::<$t, _, _>(
                                    config,
                                    move |data, _| {
                                        for frame in data.chunks_exact(channels) {
                                            let value = mono(frame);
                                            if producer.push(value).is_err() {
                                                o.store(true, Ordering::Release);
                                                break;
                                            }
                                        }
                                    },
                                    move |_| {
                                        f.store(true, Ordering::Release);
                                    },
                                    Some(Duration::from_secs(5)),
                                )
                                .map_err(|_| Error::new("deviceAccess"))?
                        }};
                    }
                    let stream = match supported.sample_format() {
                        cpal::SampleFormat::F32 => stream!(f32),
                        cpal::SampleFormat::I16 => stream!(i16),
                        cpal::SampleFormat::U16 => stream!(u16),
                        cpal::SampleFormat::I32 => stream!(i32),
                        cpal::SampleFormat::U32 => stream!(u32),
                        cpal::SampleFormat::F64 => stream!(f64),
                        cpal::SampleFormat::I8 => stream!(i8),
                        cpal::SampleFormat::U8 => stream!(u8),
                        _ => return Err(Error::new("deviceFormat")),
                    };
                    let mut converter = Converter::new(rate)?;
                    let mut save_error = false;
                    let mut writer = match &file {
                        Some((files, key)) => match files.writer(key) {
                            Ok(w) => Some(w),
                            Err(_) => {
                                save_error = true;
                                None
                            }
                        },
                        None => None,
                    };
                    stream.play().map_err(|_| Error::new("deviceAccess"))?;
                    if ready_tx.take().unwrap().send(Ok(())).is_err() {
                        stopper.store(true, Ordering::Release);
                    }
                    let began = Instant::now();
                    let mut flushed = Instant::now();
                    let mut pcm = Vec::with_capacity(16000 * 10);
                    let mut peak = 0;
                    let mut error = None;
                    let mut accept = |chunk: Vec<i16>| {
                        if chunk.is_empty() {
                            return;
                        }
                        if let Some(w) = writer.as_mut() {
                            for &s in &chunk {
                                if w.write_sample(s).is_err() {
                                    save_error = true;
                                    break;
                                }
                            }
                            if flushed.elapsed() >= Duration::from_secs(1) {
                                if w.flush().is_err() {
                                    save_error = true;
                                }
                                flushed = Instant::now();
                            }
                        }
                        let level = (chunk
                            .iter()
                            .map(|s| (*s as f32 / 32768.0).powi(2))
                            .sum::<f32>()
                            / chunk.len() as f32)
                            .sqrt();
                        pcm.extend_from_slice(&chunk);
                        if sender
                            .try_send(Packet {
                                pcm: chunk,
                                level,
                                duration_ms: pcm.len() as u64 / 16,
                            })
                            .is_err()
                        {
                            overflow.store(true, Ordering::Release);
                        }
                    };
                    loop {
                        peak = peak.max(consumer.slots());
                        let mut input = vec![];
                        while let Ok(v) = consumer.pop() {
                            input.push(v);
                        }
                        accept(converter.push(&input)?);
                        if stopper.load(Ordering::Acquire)
                            || began.elapsed() >= Duration::from_secs(600)
                            || overflow.load(Ordering::Acquire)
                            || failure.load(Ordering::Acquire)
                        {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    drop(stream);
                    let mut tail = vec![];
                    while let Ok(v) = consumer.pop() {
                        tail.push(v);
                    }
                    accept(converter.push(&tail)?);
                    accept(converter.finish()?);
                    drop(accept);
                    if overflow.load(Ordering::Acquire) {
                        error = Some("bufferOverflow".into());
                    } else if failure.load(Ordering::Acquire) {
                        error = Some("deviceAccess".into());
                    }
                    let bytes = pcm.len() as u64 * 2;
                    if let Some(w) = writer {
                        if w.finalize().is_err() {
                            save_error = true;
                        } else if let Some((files, key)) = &file
                            && files.complete(key).is_err()
                        {
                            save_error = true;
                        }
                    }
                    Ok(CaptureResult {
                        pcm,
                        error,
                        save_error,
                        bytes,
                        peak,
                    })
                })();
                if let (Some(tx), Err(e)) = (ready_tx, result.as_ref()) {
                    let _ = tx.send(Err(e.clone()));
                }
                // The exact preparation error is returned without exposing device internals.
                let result = result.unwrap_or_else(|e| CaptureResult {
                    pcm: vec![],
                    error: Some(e.code),
                    save_error: false,
                    bytes: 0,
                    peak: 0,
                });
                let _ = done_tx.send(result);
            })
            .map_err(|_| Error::new("deviceAccess"))?;
        ready.await.map_err(|_| Error::new("deviceAccess"))??;
        Ok(Self {
            stop,
            packets,
            done,
        })
    }
}
mod supported_config {
    pub type Config = cpal::StreamConfig;
}
