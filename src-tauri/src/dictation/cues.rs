//! Short, optional output on the default playback device, separate from microphone and history.
use crate::{coordinator::Phase, speech::Snapshot as SpeechSnapshot};
use rodio::cpal::{
    self,
    traits::{DeviceTrait, HostTrait},
};
use std::{
    io::Cursor,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{RecvTimeoutError, SyncSender, sync_channel},
    },
    thread,
    time::{Duration, Instant},
};
use tokio::sync::watch;

const START_FRESHNESS: Duration = Duration::from_millis(300);
// The default device may have been switched or gone idle during processing.
const RESULT_FRESHNESS: Duration = Duration::from_millis(1_200);
const OUTPUT_IDLE: Duration = Duration::from_millis(500);
const START_WAV: &[u8] = include_bytes!("../../assets/interaction-sounds/recording-start.wav");
const RESULT_WAV: &[u8] = include_bytes!("../../assets/interaction-sounds/result-ready.wav");

pub(super) struct Ticket {
    task: String,
    generation: u64,
    stop: watch::Receiver<u8>,
    offered: Instant,
    output_live: Option<Arc<AtomicBool>>,
}

impl Ticket {
    pub fn new(task: String, generation: u64, stop: watch::Receiver<u8>) -> Self {
        Self {
            task,
            generation,
            stop,
            offered: Instant::now(),
            output_live: None,
        }
    }

    pub fn retain_output(mut self, live: Arc<AtomicBool>) -> Self {
        self.output_live = Some(live);
        self
    }
}

pub(super) struct ReadyTicket {
    pub ticket: Ticket,
    pub still_recording: Arc<AtomicBool>,
}

enum Kind {
    Prepare(watch::Receiver<SpeechSnapshot>),
    Ready {
        speech: watch::Receiver<SpeechSnapshot>,
        still_recording: Arc<AtomicBool>,
    },
    Completed,
}

impl Kind {
    fn name(&self) -> &'static str {
        match self {
            Self::Prepare(_) => "prepare",
            Self::Ready { .. } => "start",
            Self::Completed => "complete",
        }
    }

    fn wav(&self) -> Option<&'static [u8]> {
        match self {
            Self::Prepare(_) => None,
            Self::Ready { .. } => Some(START_WAV),
            Self::Completed => Some(RESULT_WAV),
        }
    }

    fn freshness(&self) -> Option<Duration> {
        match self {
            Self::Prepare(_) => None,
            Self::Ready { .. } => Some(START_FRESHNESS),
            Self::Completed => Some(RESULT_FRESHNESS),
        }
    }
}

struct Request {
    ticket: Ticket,
    kind: Kind,
}

impl Request {
    fn capture_active(&self, speech: &watch::Receiver<SpeechSnapshot>) -> bool {
        let speech = speech.borrow();
        speech.task_id.as_deref() == Some(self.ticket.task.as_str())
            && matches!(speech.phase, Phase::Connecting | Phase::Recording)
    }

    fn keeps_output(&self, generation: u64) -> bool {
        self.ticket.generation == generation
            && *self.ticket.stop.borrow() < 2
            && self
                .ticket
                .output_live
                .as_ref()
                .is_some_and(|live| live.load(Ordering::Acquire))
    }

    fn current(&self, generation: u64) -> bool {
        let stop = *self.ticket.stop.borrow();
        self.ticket.generation == generation
            && match &self.kind {
                Kind::Prepare(speech) => stop == 0 && self.capture_active(speech),
                Kind::Ready {
                    speech,
                    still_recording,
                } => {
                    let speech = speech.borrow();
                    still_recording.load(Ordering::Acquire)
                        && stop == 0
                        && speech.task_id.as_deref() == Some(self.ticket.task.as_str())
                        && speech.phase == Phase::Recording
                }
                Kind::Completed => stop == 1,
            }
    }

    fn rejection(&self, generation: u64, now: Instant) -> Option<&'static str> {
        if !self.current(generation) {
            Some("superseded")
        } else if self
            .kind
            .freshness()
            .is_some_and(|limit| now.duration_since(self.ticket.offered) > limit)
        {
            Some("expired")
        } else {
            None
        }
    }

    fn log(&self, result: &str) {
        log::info!(target:"sayweave", "operation=interaction_sound task_id={} cue={} result={result} elapsed_ms={}", self.ticket.task, self.kind.name(), self.ticket.offered.elapsed().as_millis());
    }
}

#[derive(Default)]
pub(super) struct Player {
    generation: Arc<AtomicU64>,
    worker: OnceLock<Option<Worker>>,
}

impl Player {
    // The existing task admission owns replacement; the player never grants admission.
    pub fn next_generation(&self) -> u64 {
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub fn prepare(&self, ticket: Ticket, speech: watch::Receiver<SpeechSnapshot>) {
        self.offer(ticket, Kind::Prepare(speech));
    }

    pub fn ready(&self, ready: ReadyTicket, speech: watch::Receiver<SpeechSnapshot>) {
        self.offer(
            ready.ticket,
            Kind::Ready {
                speech,
                still_recording: ready.still_recording,
            },
        );
    }

    pub fn complete(&self, ticket: Ticket) {
        self.offer(ticket, Kind::Completed);
    }

    fn offer(&self, ticket: Ticket, kind: Kind) {
        let request = Request { ticket, kind };
        if let Some(reason) =
            request.rejection(self.generation.load(Ordering::Acquire), Instant::now())
        {
            request.log(reason);
            return;
        }
        let Some(worker) = self
            .worker
            .get_or_init(|| Worker::new(self.generation.clone()))
        else {
            request.log("worker_unavailable");
            return;
        };
        let Ok(mut pending) = worker.pending.lock() else {
            return;
        };
        if let Some(reason) =
            request.rejection(self.generation.load(Ordering::Acquire), Instant::now())
        {
            request.log(reason);
            return;
        }
        // A delayed prepare can never displace a playable cue for the same task.
        if pending.as_ref().is_some_and(|queued| {
            queued.ticket.generation == request.ticket.generation
                && matches!(request.kind, Kind::Prepare(_))
        }) {
            return;
        }
        let previous = pending.replace(request);
        drop(pending);
        let _ = worker.wake.try_send(());
        if let Some(previous) = previous
            && previous.kind.wav().is_some()
        {
            previous.log("superseded");
        }
    }
}

struct Worker {
    pending: Arc<Mutex<Option<Request>>>,
    wake: SyncSender<()>,
}

impl Worker {
    fn new(generation: Arc<AtomicU64>) -> Option<Self> {
        let pending = Arc::new(Mutex::new(None::<Request>));
        let requests = Arc::clone(&pending);
        let (wake, ready) = sync_channel(1);
        thread::Builder::new()
            .name("sayweave-cues".into())
            .spawn(move || {
                let mut output = None;
                let mut lease: Option<Request> = None;
                loop {
                    if output.is_some() {
                        match ready.recv_timeout(OUTPUT_IDLE) {
                            Ok(()) => {}
                            Err(RecvTimeoutError::Timeout) => {
                                if !lease.as_ref().is_some_and(|request| {
                                    request.keeps_output(generation.load(Ordering::Acquire))
                                }) {
                                    output = None;
                                    lease = None;
                                }
                                continue;
                            }
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                    } else if ready.recv().is_err() {
                        break;
                    }
                    let request = requests.lock().ok().and_then(|mut slot| slot.take());
                    if let Some(request) = request {
                        request.log(play(&request, &generation, &mut output));
                        lease = Some(request);
                    }
                }
            })
            .ok()?;
        Some(Self { pending, wake })
    }
}

struct Output {
    sink: rodio::MixerDeviceSink,
    device: Option<cpal::DeviceId>,
    failed: Arc<AtomicBool>,
}

impl Output {
    fn refresh(slot: &mut Option<Self>) -> Option<&Self> {
        // Resolve the default at the cue boundary as well as on preparation.
        // Never fall back to a different device if opening it fails.
        let Some(default) = cpal::default_host().default_output_device() else {
            *slot = None;
            return None;
        };
        let device = default.id().ok();
        if !slot.as_ref().is_some_and(|output| {
            device.is_some() && output.device == device && !output.failed.load(Ordering::Acquire)
        }) {
            *slot = None;
            let failed = Arc::new(AtomicBool::new(false));
            let fault = failed.clone();
            let mut sink = rodio::DeviceSinkBuilder::from_device(default)
                .ok()?
                .with_error_callback(move |err| {
                    if !matches!(err, cpal::StreamError::BufferUnderrun) {
                        fault.store(true, Ordering::Release);
                    }
                })
                .open_stream()
                .ok()?;
            sink.log_on_drop(false);
            *slot = Some(Self {
                sink,
                device,
                failed,
            });
        }
        slot.as_ref()
    }
}

fn play(request: &Request, generation: &AtomicU64, output: &mut Option<Output>) -> &'static str {
    let rejection = || request.rejection(generation.load(Ordering::Acquire), Instant::now());
    if let Some(reason) = rejection() {
        return reason;
    }
    let Some(output) = Output::refresh(output) else {
        return "output_unavailable";
    };
    let Some(wav) = request.kind.wav() else {
        return "output_ready";
    };
    let Ok(decoder) = rodio::Decoder::try_from(Cursor::new(wav)) else {
        return "invalid_wav";
    };
    let player = rodio::Player::connect_new(output.sink.mixer());
    if let Some(reason) = rejection() {
        return reason;
    }
    player.append(decoder);
    request.log("submitted");
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if output.failed.load(Ordering::Acquire) {
            break "output_lost";
        }
        if player.empty() {
            // The worker keeps the stream open across the hardware buffer's tail.
            break "drained";
        }
        if !request.current(generation.load(Ordering::Acquire)) {
            break "interrupted";
        }
        if Instant::now() >= deadline {
            break "drain_timeout";
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_open_may_not_replay_a_stopped_expired_or_replaced_cue() {
        let (stop, receiver) = watch::channel(0);
        let (speech, state) = watch::channel(SpeechSnapshot {
            task_id: Some("first".into()),
            phase: Phase::Recording,
            ..SpeechSnapshot::default()
        });
        let offered = Instant::now();
        let mut request = Request {
            ticket: Ticket {
                task: "first".into(),
                generation: 1,
                stop: receiver,
                offered,
                output_live: None,
            },
            kind: Kind::Ready {
                speech: state,
                still_recording: Arc::new(AtomicBool::new(true)),
            },
        };
        assert_eq!(request.rejection(1, offered), None);
        assert_eq!(request.rejection(2, offered), Some("superseded"));
        assert_eq!(
            request.rejection(1, offered + Duration::from_millis(301)),
            Some("expired")
        );
        speech.send_modify(|s| s.phase = Phase::Processing);
        assert!(request.rejection(1, offered).is_some());
        speech.send_modify(|s| s.phase = Phase::Recording);
        stop.send_replace(1);
        assert!(request.rejection(1, offered).is_some());
        request.kind = Kind::Completed;
        let cold_open = offered + Duration::from_millis(490);
        assert_eq!(request.rejection(1, cold_open), None);
        assert_eq!(request.rejection(2, cold_open), Some("superseded"));
        assert_eq!(
            request.rejection(1, offered + RESULT_FRESHNESS + Duration::from_millis(1)),
            Some("expired")
        );
        stop.send_replace(2);
        assert!(request.rejection(1, offered).is_some());
        stop.send_replace(0);
        assert!(request.rejection(1, offered).is_some());
        stop.send_replace(3);
        assert!(request.rejection(1, offered).is_some());
    }

    #[test]
    fn prepared_output_remains_available_through_final_processing() {
        let (stop, signal) = watch::channel(0);
        let (speech, state) = watch::channel(SpeechSnapshot {
            task_id: Some("first".into()),
            phase: Phase::Connecting,
            ..SpeechSnapshot::default()
        });
        let live = Arc::new(AtomicBool::new(true));
        let request = Request {
            ticket: Ticket::new("first".into(), 1, signal).retain_output(live.clone()),
            kind: Kind::Prepare(state),
        };
        assert!(request.keeps_output(1));
        assert!(!request.keeps_output(2));
        stop.send_replace(1);
        speech.send_modify(|s| s.phase = Phase::Processing);
        assert!(request.keeps_output(1));
        speech.send_modify(|s| s.phase = Phase::Completed);
        assert!(request.keeps_output(1));
        stop.send_replace(2);
        assert!(!request.keeps_output(1));
        stop.send_replace(1);
        live.store(false, Ordering::Release);
        assert!(!request.keeps_output(1));
    }

    #[test]
    fn embedded_recording_cues_decode_in_the_production_player() {
        use rodio::Source;

        for wav in [START_WAV, RESULT_WAV] {
            let decoder = rodio::Decoder::try_from(Cursor::new(wav)).unwrap();
            let channels = usize::from(decoder.channels().get());
            let rate = decoder.sample_rate().get() as usize;
            let samples = decoder.count();
            assert_eq!(samples % channels, 0);
            assert!(samples > 0 && samples < rate * channels);
        }
    }

    #[test]
    #[ignore = "opens the real default output and plays the two product cues"]
    fn real_default_output_drains_both_recording_cues() {
        let generation = AtomicU64::new(1);
        let (stop, signal) = watch::channel(0);
        let (speech, state) = watch::channel(SpeechSnapshot {
            task_id: Some("device-check".into()),
            phase: Phase::Connecting,
            ..SpeechSnapshot::default()
        });
        let ticket = || Ticket::new("device-check".into(), 1, signal.clone());
        let mut output = None;
        let prepare = Request {
            ticket: ticket(),
            kind: Kind::Prepare(state.clone()),
        };
        assert_eq!(play(&prepare, &generation, &mut output), "output_ready");
        eprintln!(
            "Default output prepared in {} ms",
            prepare.ticket.offered.elapsed().as_millis()
        );
        speech.send_modify(|s| s.phase = Phase::Recording);
        let start = Request {
            ticket: ticket(),
            kind: Kind::Ready {
                speech: state,
                still_recording: Arc::new(AtomicBool::new(true)),
            },
        };
        assert_eq!(play(&start, &generation, &mut output), "drained");
        stop.send_replace(1);
        speech.send_modify(|s| s.phase = Phase::Completed);
        // Exercise the cold path too, as when the default output changes mid-task.
        output = None;
        let end = Request {
            ticket: ticket(),
            kind: Kind::Completed,
        };
        assert_eq!(play(&end, &generation, &mut output), "drained");
        thread::sleep(OUTPUT_IDLE);
    }
}
