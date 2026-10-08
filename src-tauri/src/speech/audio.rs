use crate::history::{files::AudioFiles, types::Error};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Fft, FixedSync, Indexing, Resampler};
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot, watch};
#[derive(Default)]
struct CallbackState {
    fatal: AtomicU8,
    xruns: AtomicU32,
}
impl CallbackState {
    fn record(&self, kind: cpal::ErrorKind) {
        if kind == cpal::ErrorKind::Xrun {
            let _ = self
                .xruns
                .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    Some(n.saturating_add(1))
                });
            return;
        }
        let category = match kind {
            cpal::ErrorKind::DeviceNotAvailable => 1,
            cpal::ErrorKind::StreamInvalidated => 2,
            cpal::ErrorKind::BackendError => 4,
            cpal::ErrorKind::DeviceBusy => 5,
            cpal::ErrorKind::DeviceChanged => 6,
            cpal::ErrorKind::HostUnavailable => 7,
            cpal::ErrorKind::InvalidInput => 8,
            cpal::ErrorKind::PermissionDenied => 9,
            cpal::ErrorKind::RealtimeDenied => 10,
            cpal::ErrorKind::ResourceExhausted => 11,
            cpal::ErrorKind::UnsupportedConfig => 12,
            cpal::ErrorKind::UnsupportedOperation => 13,
            _ => 14,
        };
        // Later nonfatal notifications cannot erase the first fatal callback.
        let _ = self
            .fatal
            .compare_exchange(0, category, Ordering::AcqRel, Ordering::Acquire);
    }
}
fn watch_stop(
    stop: Arc<AtomicBool>,
    mut signal: watch::Receiver<u8>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let finished = *signal.borrow() >= 1;
            if finished || signal.changed().await.is_err() {
                stop.store(true, Ordering::Release);
                break;
            }
        }
    })
}
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
    pub duration_ms: u64,
    pub turn: u64,
}
#[derive(Clone, Copy)]
struct CapturedSample {
    value: f32,
    turn: u64,
}
/// Independent from the continuous WAV converter: no FFT history or pending
/// capture samples can cross an editing boundary into a newly acquired range.
struct TurnConverter {
    rate: usize,
    turn: u64,
    converter: Option<Converter>,
}
impl TurnConverter {
    fn new(rate: usize) -> Self {
        Self {
            rate,
            turn: 0,
            converter: None,
        }
    }
    fn push(
        &mut self,
        samples: &[CapturedSample],
        route: &super::continuous::AudioRoute,
    ) -> Result<Vec<(u64, Vec<i16>)>, Error> {
        let mut output = Vec::new();
        let mut begin = 0;
        while begin < samples.len() {
            let stamp = samples[begin].turn;
            let mut end = begin + 1;
            while end < samples.len() && samples[end].turn == stamp {
                end += 1;
            }
            let turn = if stamp > 0 && route.current() == stamp {
                stamp
            } else {
                0
            };
            if turn != self.turn {
                self.turn = turn;
                self.converter = if turn > 0 {
                    Some(Converter::new(self.rate)?)
                } else {
                    None
                };
            }
            if let Some(converter) = &mut self.converter {
                let input: Vec<_> = samples[begin..end].iter().map(|s| s.value).collect();
                let pcm = converter.push(&input)?;
                if !pcm.is_empty() && route.current() == turn {
                    output.push((turn, pcm));
                }
            }
            begin = end;
        }
        Ok(output)
    }
    fn finish(
        &mut self,
        route: &super::continuous::AudioRoute,
    ) -> Result<Option<(u64, Vec<i16>)>, Error> {
        if self.turn == 0 || route.current() != self.turn {
            return Ok(None);
        }
        match &mut self.converter {
            Some(converter) => Ok(Some((self.turn, converter.finish()?))),
            None => Ok(None),
        }
    }
}
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalMeter {
    pub level: f32,
    pub energies: [f32; 15],
    pub capture_sequence: u64,
    pub duration_ms: u64,
}
// Fixed 50ms windows at the existing 16kHz capture rate. These are temporal
// energies, not frequency energies. Packet boundaries do not change the features.
#[derive(Default)]
struct MeterWindow {
    energy: [f64; 15],
    counts: [usize; 15],
    samples: usize,
    frames: u64,
}
impl MeterWindow {
    fn push(&mut self, pcm: &[i16], mut publish: impl FnMut(LocalMeter)) {
        for sample in pcm {
            let bin = self.samples * 15 / 800;
            self.energy[bin] += (*sample as f64 / 32768.0).powi(2);
            self.counts[bin] += 1;
            self.samples += 1;
            if self.samples == 800 {
                self.frames += 1;
                let level = (self.energy.iter().sum::<f64>() / 800.0).sqrt() as f32;
                let energies =
                    std::array::from_fn(|i| (self.energy[i] / self.counts[i] as f64).sqrt() as f32);
                publish(LocalMeter {
                    level,
                    energies,
                    capture_sequence: self.frames,
                    duration_ms: self.frames * 50,
                });
                self.samples = 0;
                self.energy.fill(0.0);
                self.counts.fill(0);
            }
        }
    }
}
pub fn publish_meter(
    mut reading: watch::Receiver<LocalMeter>,
    mut signal: watch::Receiver<u8>,
    mut publish: impl FnMut(LocalMeter) + Send + 'static,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(50));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if *signal.borrow() >= 1 {
                break;
            }
            tokio::select! {
                biased;
                _=signal.changed()=>{if *signal.borrow()>=1 {break;}},
                _=tick.tick()=>{
                    let sample=*reading.borrow_and_update();
                    publish(sample);
                    if reading.has_changed().is_err(){break;}
                }
            }
        }
    })
}
pub struct CaptureResult {
    pub pcm: Vec<i16>,
    pub error: Option<String>,
    pub save_error: bool,
    pub bytes: u64,
    pub peak: usize,
    pub archived: Option<oneshot::Receiver<bool>>,
}
/// File I/O has a separate bounded owner. A slow disk can fail saving, but
/// cannot hold the audio/network drain or microphone callback hostage.
struct FileSink {
    sender: std::sync::mpsc::SyncSender<Vec<i16>>,
    failed: Arc<AtomicBool>,
}
impl FileSink {
    fn start(files: AudioFiles, key: String) -> Result<(Self, oneshot::Receiver<bool>), Error> {
        let (sender, receiver) = std::sync::mpsc::sync_channel::<Vec<i16>>(64);
        let (complete, archived) = oneshot::channel();
        let failed = Arc::new(AtomicBool::new(false));
        let failure = failed.clone();
        std::thread::Builder::new()
            .name("sayweave-audio-archive".into())
            .spawn(move || {
                let result = (|| -> Result<(), Error> {
                    let mut writer = files.writer(&key)?;
                    let mut flushed = Instant::now();
                    for chunk in receiver {
                        if failure.load(Ordering::Acquire) {
                            return Err(Error::new("saveFailed"));
                        }
                        for sample in chunk {
                            writer
                                .write_sample(sample)
                                .map_err(|_| Error::new("saveFailed"))?;
                        }
                        if flushed.elapsed() >= Duration::from_secs(1) {
                            writer.flush().map_err(|_| Error::new("saveFailed"))?;
                            flushed = Instant::now();
                        }
                    }
                    writer.finalize().map_err(|_| Error::new("saveFailed"))?;
                    if failure.load(Ordering::Acquire) {
                        return Err(Error::new("saveFailed"));
                    }
                    files.complete(&key)?;
                    Ok(())
                })();
                let _ = complete.send(result.is_ok());
            })
            .map_err(|_| Error::new("saveFailed"))?;
        Ok((Self { sender, failed }, archived))
    }
    fn push(&self, pcm: &[i16]) {
        if self.failed.load(Ordering::Acquire) {
            return;
        }
        // Bound bytes as well as slots, including the final resampler drain.
        for chunk in pcm.chunks(1600) {
            if self.sender.try_send(chunk.to_vec()).is_err() {
                self.failed.store(true, Ordering::Release);
                break;
            }
        }
    }
}
pub struct Capture {
    stop: Arc<AtomicBool>,
    stop_watcher: tokio::task::JoinHandle<()>,
    pub packets: mpsc::Receiver<Packet>,
    pub meter: watch::Receiver<LocalMeter>,
    pub done: oneshot::Receiver<CaptureResult>,
    ready: oneshot::Receiver<Result<(), Error>>,
}
impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.stop_watcher.abort();
    }
}
impl Capture {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
    #[cfg(test)]
    pub async fn start(
        device_id: Option<String>,
        file: Option<(AudioFiles, String)>,
        signal: watch::Receiver<u8>,
        route: Option<Arc<super::continuous::AudioRoute>>,
    ) -> Result<Self, Error> {
        let mut capture = Self::prepare(device_id, file, signal, route)?;
        capture.wait_ready().await?;
        Ok(capture)
    }
    pub async fn wait_ready(&mut self) -> Result<(), Error> {
        (&mut self.ready)
            .await
            .map_err(|_| Error::new("deviceAccess"))?
    }
    pub fn prepare(
        device_id: Option<String>,
        file: Option<(AudioFiles, String)>,
        signal: watch::Receiver<u8>,
        route: Option<Arc<super::continuous::AudioRoute>>,
    ) -> Result<Self, Error> {
        let stop = Arc::new(AtomicBool::new(false));
        let stopper = stop.clone();
        let (sender, packets) = mpsc::channel(100);
        let (done_tx, done) = oneshot::channel();
        let (ready_tx, ready) = oneshot::channel();
        let (meter_tx, meter) = watch::channel(LocalMeter::default());
        // Construct the owner before awaiting device readiness: dropping the future stops capture.
        let capture = Self {
            stop_watcher: watch_stop(stop.clone(), signal),
            stop,
            packets,
            meter,
            done,
            ready,
        };
        std::thread::Builder::new()
            .name("sayweave-capture".into())
            .spawn(move || {
                let mut ready_tx = Some(ready_tx);
                let mut archived = None;
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
                    let (mut producer, mut consumer) = rtrb::RingBuffer::<CapturedSample>::new(rate * 2);
                    let overflow = Arc::new(AtomicBool::new(false));
                    let failure = Arc::new(CallbackState::default());
                    macro_rules! stream {
                        ($t:ty) => {{
                            let o = overflow.clone();
                            let f = failure.clone();
                            let capture_route = route.clone();
                            device
                                .build_input_stream::<$t, _, _>(
                                    config,
                                    move |data, _| {
                                        // One nonblocking route read per callback, before buffering.
                                        let turn = capture_route.as_ref().map_or(1, |r|r.current());
                                        for frame in data.chunks_exact(channels) {
                                            let value = mono(frame);
                                            if producer.push(CapturedSample {value,turn}).is_err() {
                                                o.store(true, Ordering::Release);
                                                break;
                                            }
                                        }
                                    },
                                    move |error| {
                                        // Stable category only: no backend/device text is logged.
                                        f.record(error.kind());
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
                    let mut turn_converter = TurnConverter::new(rate);
                    let mut save_error = false;
                    let writer = match &file {
                        Some((files, key)) => match FileSink::start(files.clone(), key.clone()) {
                            Ok((w, completion)) => { archived = Some(completion); Some(w) },
                            Err(_) => {
                                save_error = true;
                                None
                            }
                        },
                        None => None,
                    };
                    if stopper.load(Ordering::Acquire) {return Err(Error::new("cancelled"));}
                    stream.play().map_err(|_| Error::new("deviceAccess"))?;
                    if ready_tx.take().unwrap().send(Ok(())).is_err() {
                        stopper.store(true, Ordering::Release);
                    }
                    let began = Instant::now();
                    let mut pcm = Vec::with_capacity(16000 * 10);
                    let mut peak = 0;
                    let mut error = None;
                    let mut meter_window=MeterWindow::default();
                    let mut accept = |chunk: Vec<i16>| {
                        if chunk.is_empty() {
                            return;
                        }
                        if let Some(w) = &writer { w.push(&chunk); }
                        meter_window.push(&chunk, |sample| { meter_tx.send_replace(sample); });
                        pcm.extend_from_slice(&chunk);
                        if route.is_none() && sender
                            .try_send(Packet {
                                pcm: chunk,
                                duration_ms: pcm.len() as u64 / 16,
                                turn: 1,
                            })
                            .is_err()
                        {
                            overflow.store(true, Ordering::Release);
                        }
                    };
                    let mut captured_frames = 0u64;
                    let transmit = |turn: u64, chunk: Vec<i16>, duration_ms: u64| {
                        if !chunk.is_empty() && sender.try_send(Packet {turn,pcm:chunk,duration_ms}).is_err() {
                            overflow.store(true, Ordering::Release);
                        }
                    };
                    loop {
                        peak = peak.max(consumer.slots());
                        let mut input = vec![];
                        while let Ok(v) = consumer.pop() {
                            input.push(v);
                        }
                        captured_frames += input.len() as u64;
                        let local: Vec<_> = input.iter().map(|s: &CapturedSample|s.value).collect();
                        accept(converter.push(&local)?);
                        if let Some(route) = &route {
                            for (turn,pcm) in turn_converter.push(&input,route)? { transmit(turn,pcm,captured_frames*1000/rate as u64); }
                        }
                        if stopper.load(Ordering::Acquire)
                            || began.elapsed() >= Duration::from_secs(600)
                            || overflow.load(Ordering::Acquire)
                            || failure.fatal.load(Ordering::Acquire) != 0
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
                    captured_frames += tail.len() as u64;
                    let local: Vec<_> = tail.iter().map(|s: &CapturedSample|s.value).collect();
                    accept(converter.push(&local)?);
                    accept(converter.finish()?);
                    if let Some(route) = &route {
                        for (turn,pcm) in turn_converter.push(&tail,route)? { transmit(turn,pcm,captured_frames*1000/rate as u64); }
                        if let Some((turn,pcm))=turn_converter.finish(route)? { transmit(turn,pcm,captured_frames*1000/rate as u64); }
                    }
                    drop(accept);
                    if overflow.load(Ordering::Acquire) {
                        error = Some("bufferOverflow".into());
                    } else if failure.fatal.load(Ordering::Acquire) != 0 {
                        log::info!(target:"sayweave", "operation=capture_callback result=failed category={}", failure.fatal.load(Ordering::Acquire));
                        #[cfg(test)]
                        eprintln!("CAPTURE_CALLBACK category={}", failure.fatal.load(Ordering::Acquire));
                        error = Some("deviceAccess".into());
                    }
                    let xruns = failure.xruns.load(Ordering::Acquire);
                    if xruns > 0 {
                        log::warn!(target:"sayweave", "operation=capture_callback result=continued category=xrun count={xruns}");
                        #[cfg(test)]
                        eprintln!("CAPTURE_CALLBACK category=xrun count={xruns} continued=true");
                    }
                    let bytes = pcm.len() as u64 * 2;
                    drop(writer); // Archive worker owns flushing and finalization.
                    Ok(CaptureResult {
                        pcm,
                        error,
                        save_error,
                        bytes,
                        peak,
                        archived: None,
                    })
                })();
                if let (Some(tx), Err(e)) = (ready_tx, result.as_ref()) {
                    let _ = tx.send(Err(e.clone()));
                }
                // The exact preparation error is returned without exposing device internals.
                let mut result = result.unwrap_or_else(|e| CaptureResult {
                    pcm: vec![],
                    error: Some(e.code),
                    save_error: false,
                    bytes: 0,
                    peak: 0,
                    archived: None,
                });
                result.archived = archived;
                drop(sender); // End of captured audio is independent of file I/O.
                let _ = done_tx.send(result);
            })
            .map_err(|_| Error::new("deviceAccess"))?;
        Ok(capture)
    }
}
mod supported_config {
    pub type Config = cpal::StreamConfig;
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn failed_readiness_keeps_capture_and_file_cleanup_owned() {
        let root = tempfile::tempdir().unwrap();
        let files = AudioFiles::new(root.path()).unwrap();
        let key = uuid::Uuid::new_v4().to_string();
        let (sink, archived) = FileSink::start(files.clone(), key.clone()).unwrap();
        let (ready_tx, ready) = oneshot::channel();
        let (done_tx, done) = oneshot::channel();
        let (_packets, packets) = mpsc::channel(1);
        let (_meter, meter) = watch::channel(LocalMeter::default());
        let stop = Arc::new(AtomicBool::new(false));
        let mut capture = Capture {
            stop,
            stop_watcher: tokio::spawn(std::future::pending()),
            packets,
            meter,
            done,
            ready,
        };
        ready_tx.send(Err(Error::new("deviceAccess"))).unwrap();
        assert_eq!(capture.wait_ready().await.unwrap_err().code, "deviceAccess");
        // Readiness failure does not consume the owner. The common drain can
        // still collect the file worker, even when it finishes after the error.
        sink.push(&[123; 16]);
        drop(sink);
        done_tx
            .send(CaptureResult {
                pcm: vec![],
                error: Some("deviceAccess".into()),
                save_error: false,
                bytes: 32,
                peak: 0,
                archived: Some(archived),
            })
            .ok()
            .unwrap();
        capture.stop();
        let result = (&mut capture.done).await.unwrap();
        assert!(result.archived.unwrap().await.unwrap());
        assert!(files.path(&key, false).unwrap().exists());
    }
    #[tokio::test]
    async fn file_sink_saves_pcm_and_finalizes_after_capture_owner_is_released() {
        let root = tempfile::tempdir().unwrap();
        let files = AudioFiles::new(root.path()).unwrap();
        let key = uuid::Uuid::new_v4().to_string();
        let (sink, done) = FileSink::start(files.clone(), key.clone()).unwrap();
        let samples = vec![123i16; 3200];
        sink.push(&samples);
        drop(sink);
        assert!(done.await.unwrap());
        let saved = hound::WavReader::open(files.path(&key, false).unwrap())
            .unwrap()
            .into_samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(saved, samples);
    }
    #[test]
    fn stalled_file_writer_has_bounded_memory_and_never_blocks_audio_producer() {
        let (sender, receiver) = std::sync::mpsc::sync_channel(64);
        let failed = Arc::new(AtomicBool::new(false));
        let sink = FileSink {
            sender,
            failed: failed.clone(),
        };
        sink.push(&vec![0; 1600 * 100]);
        assert!(failed.load(Ordering::Acquire));
        let chunks: Vec<_> = receiver.try_iter().collect();
        assert_eq!(chunks.len(), 64);
        assert!(chunks.iter().all(|c| c.len() <= 1600));
    }
    #[test]
    fn callback_xrun_continues_but_cannot_clear_device_failure() {
        let callbacks = CallbackState::default();
        callbacks.record(cpal::ErrorKind::Xrun);
        callbacks.record(cpal::ErrorKind::Xrun);
        assert_eq!(callbacks.xruns.load(Ordering::Acquire), 2);
        assert_eq!(callbacks.fatal.load(Ordering::Acquire), 0);
        callbacks.record(cpal::ErrorKind::DeviceNotAvailable);
        callbacks.record(cpal::ErrorKind::Xrun);
        callbacks.record(cpal::ErrorKind::BackendError);
        assert_eq!(callbacks.fatal.load(Ordering::Acquire), 1);
        assert_eq!(callbacks.xruns.load(Ordering::Acquire), 3);
    }
    #[test]
    fn delayed_capture_and_fft_tail_cannot_be_relabelled_as_the_next_turn() {
        use super::super::continuous::{Control, Route};
        use std::sync::atomic::AtomicU64;
        let evidence = Arc::new(AtomicU64::new(0));
        let control = Control::new(evidence.clone());
        control.resume(1, 0);
        control.enabled(Route {
            turn: 1,
            activity: 0,
        });
        let mut encoder = TurnConverter::new(48000);
        let old = vec![
            CapturedSample {
                value: 1.0,
                turn: 1
            };
            200
        ];
        assert!(encoder.push(&old, &control.audio).unwrap().is_empty());
        evidence.fetch_add(1, Ordering::AcqRel);
        control.pause();
        let mut backlog = vec![
            CapturedSample {
                value: 1.0,
                turn: 1
            };
            600
        ];
        backlog.extend(vec![
            CapturedSample {
                value: 1.0,
                turn: 0
            };
            600
        ]);
        control.resume(2, 1);
        control.enabled(Route {
            turn: 2,
            activity: 1,
        });
        // Consumer was delayed until after resume: only originally stamped turn2 is eligible.
        backlog.extend(vec![
            CapturedSample {
                value: 0.0,
                turn: 2
            };
            4800
        ]);
        let mut encoded = encoder.push(&backlog, &control.audio).unwrap();
        encoded.extend(encoder.finish(&control.audio).unwrap());
        assert!(!encoded.is_empty());
        assert!(
            encoded
                .iter()
                .all(|(turn, pcm)| *turn == 2 && pcm.iter().all(|s| *s == 0))
        );
        let mut local = Converter::new(48000).unwrap();
        let mut recording = local
            .push(
                &old.iter()
                    .chain(&backlog)
                    .map(|s| s.value)
                    .collect::<Vec<_>>(),
            )
            .unwrap();
        recording.extend(local.finish().unwrap());
        assert!(recording.iter().any(|s| *s > 10000));
        assert_eq!(recording.len(), (old.len() + backlog.len()) / 3);
    }
    #[test]
    fn meter_features_are_packet_independent_and_empty_packets_do_not_advance_frames() {
        let pcm: Vec<i16> = (0..2400)
            .map(|i| if i % 400 < 200 { 16384 } else { 0 })
            .collect();
        let collect = |size| {
            let mut window = MeterWindow::default();
            let mut frames = Vec::new();
            for chunk in pcm.chunks(size) {
                window.push(chunk, |m| frames.push(m));
                window.push(&[], |_| panic!("empty packet emitted a frame"));
            }
            frames
        };
        let expected = collect(2400);
        assert_eq!(expected.len(), 3);
        for size in [1, 53, 511, 800, 1024] {
            let actual = collect(size);
            for (a, b) in actual.iter().zip(&expected) {
                assert_eq!(a.energies, b.energies);
                assert_eq!(a.level, b.level);
                assert_eq!(a.capture_sequence, b.capture_sequence);
                assert_eq!(a.duration_ms, b.duration_ms);
            }
        }
        assert_eq!(expected[0].energies[0], 0.5);
        assert_eq!(expected[0].energies[4], 0.0);
    }
    #[tokio::test]
    async fn local_meter_keeps_publishing_during_network_backpressure_and_stops_on_finish() {
        let (tx, rx) = watch::channel(LocalMeter {
            level: 0.2,
            duration_ms: 50,
            capture_sequence: 1,
            ..Default::default()
        });
        let (control, signal) = watch::channel(0);
        let (seen, mut samples) = mpsc::unbounded_channel();
        let task = publish_meter(rx, signal, move |v| {
            let _ = seen.send(v);
        });
        let first = tokio::time::timeout(Duration::from_millis(100), samples.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(first.level, 0.2);
        tx.send_replace(LocalMeter {
            level: 0.8,
            duration_ms: 100,
            capture_sequence: 2,
            ..Default::default()
        });
        let next = tokio::time::timeout(Duration::from_millis(150), samples.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(next.level, 0.8);
        control.send(1).unwrap();
        task.await.unwrap();
        while samples.try_recv().is_ok() {}
        assert!(samples.recv().await.is_none());
    }
    #[tokio::test]
    async fn finish_stops_capture_while_the_audio_sender_is_blocked() {
        let (tx, _rx) = mpsc::channel(1);
        tx.send(()).await.unwrap();
        let blocked = tokio::spawn(async move { tx.send(()).await });
        let (control, signal) = watch::channel(0);
        let stop = Arc::new(AtomicBool::new(false));
        let watcher = watch_stop(stop.clone(), signal);
        control.send(1).unwrap();
        tokio::time::timeout(Duration::from_millis(100), watcher)
            .await
            .unwrap()
            .unwrap();
        assert!(stop.load(Ordering::Acquire));
        assert!(!blocked.is_finished());
        blocked.abort();
    }
    #[tokio::test]
    #[ignore = "Requires explicit local microphone acceptance; no file or network output"]
    async fn real_microphone_meter_acceptance() {
        let (control, signal) = watch::channel(0);
        let mut capture = Capture::start(None, None, signal, None).await.unwrap();
        let mut previous = 0;
        let mut frames = 0;
        let mut peak = 0.0f32;
        let mut varied = 0;
        let deadline = tokio::time::sleep(Duration::from_secs(6));
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _=&mut deadline=>break,
                packet=capture.packets.recv()=>{assert!(packet.is_some());},
                changed=capture.meter.changed()=>{
                    changed.unwrap();let value=*capture.meter.borrow_and_update();
                    assert!(value.capture_sequence>previous);previous=value.capture_sequence;
                    assert!(value.energies.iter().all(|v|v.is_finite()&&*v>=0.0&&*v<=1.0));
                    peak=peak.max(value.level);frames+=1;
                    if value.energies.windows(2).any(|w|(w[0]-w[1]).abs()>0.00001){varied+=1;}
                }
            }
        }
        control.send(1).unwrap();
        let result = tokio::time::timeout(Duration::from_secs(3), &mut capture.done)
            .await
            .unwrap()
            .unwrap();
        assert!(result.error.is_none(), "capture error: {:?}", result.error);
        assert!(frames >= 40);
        assert!(result.pcm.len() >= 16000);
        println!(
            "REAL_METER_PASS frames={frames} independent_frames={varied} peak_rms={peak:.6} duration_ms={}",
            previous * 50
        );
        drop(result);
        drop(capture);
    }
}
