//! A recording can span several ASR turns. Only the active turn publishes a writable draft.
//! Retired connections drain final results into the raw transcript, never into a new input range.
use super::recognition::{Event, RecognitionSession, Transcript};
use crate::{
    history::types::Error,
    models::SpeechConfiguration,
    network::{RequestCoordinator, RequestGuard},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Route {
    pub turn: u64,
    pub activity: u64,
}
/// Capture stamps PCM at the callback boundary; conversion and dispatch recheck its turn.
pub struct AudioRoute {
    evidence: Arc<AtomicU64>,
    route: Mutex<Route>,
    editing_gate: AtomicBool,
}
impl AudioRoute {
    pub fn new(evidence: Arc<AtomicU64>) -> Self {
        Self {
            evidence,
            route: Mutex::new(Route::default()),
            editing_gate: AtomicBool::new(false),
        }
    }
    pub fn current(&self) -> u64 {
        self.route
            .try_lock()
            .ok()
            .filter(|r| {
                !self.editing_gate.load(Ordering::Acquire)
                    || r.activity == self.evidence.load(Ordering::Acquire)
            })
            .map(|r| r.turn)
            .unwrap_or(0)
    }
    fn set(&self, route: Route) {
        if let Ok(mut current) = self.route.lock() {
            *current = route;
        }
    }
}
#[derive(Clone)]
pub struct Control {
    pub audio: Arc<AudioRoute>,
    requested: watch::Sender<Route>,
    ready: watch::Sender<u64>,
    stopping: Arc<AtomicBool>,
    transition: Arc<Mutex<()>>,
    draft_activity: Arc<AtomicU64>,
    draft_armed: Arc<AtomicBool>,
    handed_off: Arc<AtomicBool>,
    raw: Arc<Mutex<RawTurns>>,
    task: Arc<OnceLock<String>>,
}
impl Control {
    pub fn new(evidence: Arc<AtomicU64>) -> Self {
        let (requested, _) = watch::channel(Route::default());
        let (ready, _) = watch::channel(0);
        Self {
            audio: Arc::new(AudioRoute::new(evidence)),
            requested,
            ready,
            stopping: Arc::new(AtomicBool::new(false)),
            transition: Arc::new(Mutex::new(())),
            draft_activity: Arc::new(AtomicU64::new(0)),
            draft_armed: Arc::new(AtomicBool::new(false)),
            handed_off: Arc::new(AtomicBool::new(false)),
            raw: Arc::new(Mutex::new(RawTurns::default())),
            task: Arc::new(OnceLock::new()),
        }
    }
    pub fn pause(&self) {
        self.audio.set(Route::default());
        self.requested.send_replace(Route::default());
        self.ready.send_replace(0);
    }
    pub fn resume(&self, turn: u64, activity: u64) {
        let Ok(_transition) = self.transition.lock() else {
            return;
        };
        if self.stopping() {
            return;
        }
        self.audio.set(Route::default());
        self.requested.send_replace(Route { turn, activity });
    }
    /// Synchronously forbids reopening, without dropping accepted audio still being drained.
    pub fn request_finish(&self) {
        if let Ok(_transition) = self.transition.lock() {
            self.stopping.store(true, Ordering::Release);
            self.requested.send_modify(|_| {});
        }
    }
    pub fn stopping(&self) -> bool {
        self.stopping.load(Ordering::Acquire)
    }
    pub fn draft_started(&self, activity: u64) {
        self.draft_activity.store(activity, Ordering::Release);
        self.draft_armed.store(true, Ordering::Release);
        self.audio.editing_gate.store(true, Ordering::Release);
    }
    pub fn handoff_started(&self) {
        self.handed_off.store(true, Ordering::Release);
    }
    /// Physical editing can hand the TSF range back before the async draft actor observes it.
    pub fn preserve_history(&self) -> bool {
        self.handed_off.load(Ordering::Acquire)
            || self.draft_armed.load(Ordering::Acquire)
                && self.draft_activity.load(Ordering::Acquire)
                    != self.audio.evidence.load(Ordering::Acquire)
    }
    fn record(&self, turn: u64, transcript: Transcript) -> Result<(), Error> {
        self.raw
            .lock()
            .map_err(|_| Error::new("unavailable"))?
            .set(turn, transcript)
    }
    fn previous(&self, turn: u64) -> Transcript {
        self.raw
            .lock()
            .ok()
            .and_then(|r| r.0.get(&turn).cloned())
            .unwrap_or_default()
    }
    /// Captures parsed retired tails even if hard cancellation drops their pending sockets.
    pub fn latest_raw(&self) -> Option<Transcript> {
        self.raw
            .lock()
            .ok()
            .filter(|r| !r.0.is_empty())
            .map(|r| r.combined())
    }
    pub fn readiness(&self) -> watch::Receiver<u64> {
        self.ready.subscribe()
    }
    fn trace(&self, turn: u64, phase: &str) {
        if let Some(task) = self.task.get() {
            log::info!(target:"sayweave", "operation=speech_turn task_id={task} turn={turn} phase={phase} audio_route={}", self.audio.current());
        }
    }
    pub(super) fn enabled(&self, route: Route) {
        let Ok(_transition) = self.transition.lock() else {
            return;
        };
        if !self.stopping() && *self.requested.borrow() == route {
            self.audio.set(route);
            self.ready.send_replace(route.turn);
            self.trace(route.turn, "ready");
        }
    }
}
pub struct Update {
    pub raw: Transcript,
    pub draft: Option<(u64, String)>,
}
pub enum StreamEvent {
    Update(Update),
    Finished,
    Heartbeat,
}
pub enum Stream {
    Single(RecognitionSession),
    Continuous(Continuous),
}
impl Stream {
    pub async fn open(
        cfg: &SpeechConfiguration,
        task: &str,
        control: Option<Control>,
        requests: Arc<RequestCoordinator>,
    ) -> Result<Self, Error> {
        let first = RecognitionSession::open(cfg, task).await?;
        Ok(if let Some(control) = control {
            let _ = control.task.set(task.into());
            let route = *control.requested.borrow();
            control.enabled(route);
            Self::Continuous(Continuous::spawn(first, cfg, control, route, requests))
        } else {
            Self::Single(first)
        })
    }
    pub async fn send_pcm(&self, turn: u64, pcm: &[i16]) -> Result<(), Error> {
        match self {
            Self::Single(session) => session.send_pcm(pcm).await,
            Self::Continuous(session) => session.send_pcm(turn, pcm).await,
        }
    }
    pub async fn finish(&self) -> Result<(), Error> {
        match self {
            Self::Single(s) => s.finish().await,
            Self::Continuous(s) => s.finish().await,
        }
    }
    pub async fn next_event(&mut self) -> Result<StreamEvent, Error> {
        match self {
            Self::Single(s) => Ok(match s.next_event().await? {
                Event::Update(raw) => StreamEvent::Update(Update {
                    draft: Some((1, raw.text(false))),
                    raw,
                }),
                Event::Finished => StreamEvent::Finished,
                Event::Heartbeat => StreamEvent::Heartbeat,
            }),
            Self::Continuous(s) => s.events.recv().await.ok_or_else(|| Error::new("network"))?,
        }
    }
}
enum Command {
    Audio(u64, Vec<i16>),
    Finish(oneshot::Sender<Result<(), Error>>),
}
pub struct Continuous {
    commands: mpsc::Sender<Command>,
    events: mpsc::Receiver<Result<StreamEvent, Error>>,
    worker: tokio::task::JoinHandle<()>,
}
struct Connection {
    turn: u64,
    session: RecognitionSession,
    _slot: Option<RequestGuard>,
}
#[derive(Default)]
struct RawTurns(BTreeMap<u64, Transcript>);
impl RawTurns {
    fn set(&mut self, turn: u64, transcript: Transcript) -> Result<(), Error> {
        let mut text = transcript.text.len();
        let mut confirmed = transcript.confirmed_text.len();
        let mut segments = transcript.segments.len();
        let mut segment_text = transcript
            .segments
            .iter()
            .map(|s| s.text.len())
            .sum::<usize>();
        for (id, value) in &self.0 {
            if *id == turn {
                continue;
            }
            text += value.text.len() + usize::from(!value.text.is_empty());
            confirmed += value.confirmed_text.len() + usize::from(!value.confirmed_text.is_empty());
            segments += value.segments.len();
            segment_text += value.segments.iter().map(|s| s.text.len()).sum::<usize>();
        }
        // Bound the whole recording, not each individually legal provider turn.
        if text > 1024 * 1024
            || confirmed > 1024 * 1024
            || segment_text > 1024 * 1024
            || segments > 10000
        {
            return Err(Error::new("responseTooLarge"));
        }
        self.0.insert(turn, transcript);
        Ok(())
    }
    fn combined(&self) -> Transcript {
        let join = |confirmed: bool| {
            self.0
                .values()
                .map(|t| {
                    if confirmed {
                        t.confirmed_text.as_str()
                    } else {
                        t.text.as_str()
                    }
                })
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        };
        Transcript {
            text: join(false),
            confirmed_text: join(true),
            segments: self
                .0
                .iter()
                .flat_map(|(turn, t)| {
                    t.segments.iter().cloned().map(move |mut segment| {
                        segment.id = (turn << 48) | (segment.id & ((1 << 48) - 1));
                        if self.0.len() > 1 {
                            segment.begin_ms = None;
                            segment.end_ms = None;
                        }
                        segment
                    })
                })
                .collect(),
        }
    }
}
impl Continuous {
    fn spawn(
        first: RecognitionSession,
        cfg: &SpeechConfiguration,
        control: Control,
        route: Route,
        requests: Arc<RequestCoordinator>,
    ) -> Self {
        let cfg = Arc::new(SpeechConfiguration {
            id: cfg.id.clone(),
            input: cfg.input.clone(),
            revision: cfg.revision,
            key: cfg.key.clone(),
        });
        Self::run(first, control, route, move |turn| {
            let cfg = cfg.clone();
            let requests = requests.clone();
            async move {
                let id = uuid::Uuid::new_v4().to_string();
                let slot = requests.register(&id).map_err(|e| Error::new(&e.code))?;
                let session = RecognitionSession::open(&cfg, &id).await?;
                Ok(Connection {
                    turn,
                    session,
                    _slot: Some(slot),
                })
            }
        })
    }
    fn run<F, Fut>(first: RecognitionSession, control: Control, route: Route, open: F) -> Self
    where
        F: Fn(u64) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = Result<Connection, Error>> + Send + 'static,
    {
        let (commands, mut receive) = mpsc::channel(32);
        let (send, events) = mpsc::channel(32);
        // Subscribe before spawning: an immediate edit/finish cannot be lost at startup.
        let mut changes = control.requested.subscribe();
        let worker = tokio::spawn(async move {
            let mut active = Some(Connection {
                turn: route.turn.max(1),
                session: first,
                _slot: None,
            });
            // JoinSet aborts its children on drop, including cancellation of this worker.
            let mut opening = tokio::task::JoinSet::<Result<Connection, Error>>::new();
            let mut opening_turn = None;
            let mut retiring = tokio::task::JoinSet::<Retired>::new();
            let (retired_updates, mut raw_updates) = mpsc::channel(16);
            let mut ended = false;
            let mut final_route = Route::default();
            let mut failure = None;
            let mut first_audio = 0;
            let mut first_text = 0;
            let outcome = async {
                loop {
                    let desired = *changes.borrow_and_update();
                    if !ended {
                        if control.stopping() || opening_turn.is_some_and(|turn|turn!=desired.turn) {
                            opening = tokio::task::JoinSet::new();
                            opening_turn = None;
                        }
                        if active.as_ref().is_some_and(|c|c.turn!=desired.turn) {
                            control.audio.set(Route::default());
                            let connection = active.take().unwrap();
                            let previous = control.previous(connection.turn);
                            retire(connection, previous, control.clone(),retired_updates.clone(),&mut retiring);
                        }
                        if desired.turn == 0 { control.ready.send_replace(0); }
                    }
                    if ended && active.is_none() && opening.is_empty() && retiring.is_empty() {
                        if let Some(error) = failure { return Err(error); }
                        send.send(Ok(StreamEvent::Finished)).await.map_err(|_|Error::new("cancelled"))?;
                        return Ok(());
                    }
                    if !ended && !control.stopping() && active.is_none() && opening.is_empty() && desired.turn > 0 && retiring.len() < 2 {
                        if desired.turn > 256 { return Err(Error::new("turnLimit")); }
                        opening_turn = Some(desired.turn);
                        opening.spawn(open(desired.turn));
                    }
                    tokio::select! {
                        biased;
                        _=changes.changed(), if !ended => {},
                        result=opening.join_next(), if !opening.is_empty() => {
                            opening_turn = None;
                            let connection = result.ok_or_else(||Error::new("network"))?.map_err(|_|Error::new("network"))??;
                            let desired = *changes.borrow();
                            if !ended && !control.stopping() && desired.turn == connection.turn {
                                control.enabled(desired);
                                active = Some(connection);
                            }
                        },
                        command=receive.recv()=>match command.ok_or_else(||Error::new("cancelled"))? {
                            Command::Audio(turn,pcm)=> {
                                if !ended && turn > 0 && control.audio.current() == turn
                                    && let Some(connection) = active.as_ref().filter(|c| c.turn == turn)
                                {
                                    connection.session.send_pcm(&pcm).await?;
                                    if first_audio != turn { first_audio = turn; control.trace(turn,"first_audio_sent"); }
                                }
                            },
                            Command::Finish(ack)=> {
                                final_route = *changes.borrow();
                                ended = true;
                                control.request_finish();
                                control.pause();
                                opening = tokio::task::JoinSet::new();
                                opening_turn = None;
                                if let Some(connection) = active.take() {
                                    let old = control.previous(connection.turn);
                                    retire(connection, old,control.clone(),retired_updates.clone(),&mut retiring);
                                }
                                let _ = ack.send(Ok(()));
                            }
                        },
                        value=async { match active.as_mut() {Some(c)=>c.session.next_event().await, None=>std::future::pending().await} }=> {
                            let turn = active.as_ref().unwrap().turn;
                            match value? {
                                Event::Update(value)=> {
                                    let text = value.text(false);
                                    if first_text != turn && !text.is_empty() { first_text = turn; control.trace(turn,"first_transcript"); }
                                    control.record(turn,value)?;
                                    // Editing evidence immediately closes the write gate, even before Pause arrives.
                                    let draft = (control.audio.current() == turn).then_some((turn,text));
                                    send.send(Ok(StreamEvent::Update(Update {raw:control.latest_raw().unwrap_or_default(),draft}))).await.map_err(|_|Error::new("cancelled"))?;
                                },
                                Event::Finished=>return Err(Error::new("protocol")),
                                Event::Heartbeat=>{}
                            }
                        },
                        _=raw_updates.recv(), if !retiring.is_empty()=> {
                            send.send(Ok(StreamEvent::Update(Update {raw:control.latest_raw().unwrap_or_default(),draft:None}))).await.map_err(|_|Error::new("cancelled"))?;
                        },
                        value=retiring.join_next(), if !retiring.is_empty()=>match value {
                            Some(Ok(Retired {turn,transcript,error}))=> {
                                let draft = (error.is_none() && ended && final_route.turn == turn && final_route.activity == control.audio.evidence.load(Ordering::Acquire))
                                    .then(|| (turn, transcript.text(false)));
                                control.record(turn,transcript)?;
                                send.send(Ok(StreamEvent::Update(Update { raw:control.latest_raw().unwrap_or_default(),draft }))).await.map_err(|_|Error::new("cancelled"))?;
                                if error.is_some() { failure=error; }
                            },
                            Some(Err(_))=>failure=Some(Error::new("network")), None=>{}
                        }
                    }
                }
            }.await;
            control.pause();
            if let Err(error) = outcome {
                let _ = send.send(Err(error)).await;
            }
        });
        Self {
            commands,
            events,
            worker,
        }
    }
    async fn send_pcm(&self, turn: u64, pcm: &[i16]) -> Result<(), Error> {
        if turn == 0 {
            return Ok(());
        }
        tokio::time::timeout(
            Duration::from_millis(500),
            self.commands.send(Command::Audio(turn, pcm.to_vec())),
        )
        .await
        .map_err(|_| Error::new("bufferOverflow"))?
        .map_err(|_| Error::new("network"))
    }
    async fn finish(&self) -> Result<(), Error> {
        let (ack, result) = oneshot::channel();
        tokio::time::timeout(Duration::from_secs(6), async {
            self.commands
                .send(Command::Finish(ack))
                .await
                .map_err(|_| Error::new("network"))?;
            result.await.map_err(|_| Error::new("network"))?
        })
        .await
        .map_err(|_| Error::new("timeout"))?
    }
}
struct Retired {
    turn: u64,
    transcript: Transcript,
    error: Option<Error>,
}
fn retire(
    mut connection: Connection,
    previous: Transcript,
    control: Control,
    updates: mpsc::Sender<()>,
    retired: &mut tokio::task::JoinSet<Retired>,
) {
    retired.spawn(async move {
        let mut transcript = previous;
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            connection.session.finish().await?;
            loop {
                match connection.session.next_event().await? {
                    Event::Update(value) => {
                        control.record(connection.turn, value.clone())?;
                        transcript = value;
                        // The snapshot is synchronous; notification is coalesced when full.
                        let _ = updates.try_send(());
                    }
                    Event::Finished => return Ok::<_, Error>(()),
                    Event::Heartbeat => {}
                }
            }
        })
        .await
        .unwrap_or_else(|_| Err(Error::new("timeout")));
        Retired {
            turn: connection.turn,
            transcript,
            error: result.err(),
        }
    });
}
impl Drop for Continuous {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edit_evidence_retains_raw_history_before_async_handoff() {
        let evidence = Arc::new(AtomicU64::new(3));
        let control = Control::new(evidence.clone());
        control.resume(1, 3);
        control.enabled(Route {
            turn: 1,
            activity: 3,
        });
        control.draft_started(3);
        assert!(!control.preserve_history());
        evidence.fetch_add(1, Ordering::AcqRel);
        assert!(control.preserve_history());
        assert_eq!(control.audio.current(), 0);
        control.request_finish();
        control.resume(2, 4);
        control.enabled(Route {
            turn: 2,
            activity: 4,
        });
        assert_ne!(*control.readiness().borrow(), 2);
        assert_eq!(control.audio.current(), 0); // editing stopped this turn
    }
    #[tokio::test]
    async fn cancelling_worker_also_drops_the_pending_connection_future() {
        struct OnDrop(Option<oneshot::Sender<()>>);
        impl Drop for OnDrop {
            fn drop(&mut self) {
                if let Some(tx) = self.0.take() {
                    let _ = tx.send(());
                }
            }
        }
        let control = Control::new(Arc::new(AtomicU64::new(0)));
        let route = Route {
            turn: 1,
            activity: 0,
        };
        control.resume(1, 0);
        control.enabled(route);
        let (first, mut old) = RecognitionSession::controlled();
        let (opened, mut opening) = mpsc::channel(1);
        let (dropped, finished) = oneshot::channel();
        let dropped = Mutex::new(Some(dropped));
        let stream = Continuous::run(first, control.clone(), route, move |_| {
            let opened = opened.clone();
            let guard = OnDrop(dropped.lock().unwrap().take());
            async move {
                let _guard = guard;
                opened.send(()).await.unwrap();
                std::future::pending().await
            }
        });
        control.pause();
        old.finished_audio().await;
        old.events.send(Ok(Event::Finished)).await.unwrap();
        control.resume(2, 0);
        tokio::time::timeout(Duration::from_secs(2), opening.recv())
            .await
            .unwrap()
            .unwrap();
        drop(stream);
        tokio::time::timeout(Duration::from_secs(2), finished)
            .await
            .unwrap()
            .unwrap();
    }
    #[tokio::test]
    async fn failed_retired_turn_preserves_received_tail_without_writing_a_draft() {
        let control = Control::new(Arc::new(AtomicU64::new(0)));
        let route = Route {
            turn: 1,
            activity: 0,
        };
        control.resume(1, 0);
        control.enabled(route);
        let (first, mut old) = RecognitionSession::controlled();
        let mut stream = Continuous::run(first, control.clone(), route, |_| async {
            Err(Error::new("unexpectedOpen"))
        });
        control.pause();
        old.finished_audio().await;
        old.events
            .send(Ok(Event::Update(Transcript {
                text: "已收到的尾字".into(),
                ..Transcript::default()
            })))
            .await
            .unwrap();
        old.events.send(Err(Error::new("network"))).await.unwrap();
        let value = tokio::time::timeout(Duration::from_secs(2), stream.events.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(
            matches!(value,StreamEvent::Update(Update {draft:None,ref raw}) if raw.text=="已收到的尾字")
        );
        stream.finish().await.unwrap();
        let error = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                match stream.events.recv().await.unwrap() {
                    Err(error) => break error,
                    Ok(StreamEvent::Update(value)) => {
                        assert!(value.draft.is_none());
                        assert_eq!(value.raw.text, "已收到的尾字");
                    }
                    _ => panic!("failed turn cannot report completion"),
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(error.code, "network");
    }
    #[test]
    fn physical_edit_closes_pcm_before_pause_and_old_turns_do_not_reenter() {
        let evidence = Arc::new(AtomicU64::new(3));
        let control = Control::new(evidence.clone());
        control.resume(1, 3);
        control.enabled(Route {
            turn: 1,
            activity: 3,
        });
        control.draft_started(3);
        assert_eq!(control.audio.current(), 1);
        evidence.fetch_add(1, Ordering::AcqRel);
        assert_eq!(control.audio.current(), 0);
        control.pause();
        control.resume(2, 4);
        control.enabled(Route {
            turn: 1,
            activity: 3,
        });
        assert_eq!(control.audio.current(), 0);
        control.enabled(Route {
            turn: 2,
            activity: 4,
        });
        assert_eq!(control.audio.current(), 2);
    }
    #[test]
    fn late_raw_turn_replaces_only_its_original_slot() {
        let mut turns = RawTurns::default();
        turns
            .set(
                1,
                Transcript {
                    text: "旧临时".into(),
                    ..Transcript::default()
                },
            )
            .unwrap();
        turns
            .set(
                2,
                Transcript {
                    text: "新位置".into(),
                    ..Transcript::default()
                },
            )
            .unwrap();
        turns
            .set(
                1,
                Transcript {
                    text: "旧最终".into(),
                    confirmed_text: "旧最终".into(),
                    ..Transcript::default()
                },
            )
            .unwrap();
        assert_eq!(turns.combined().text, "旧最终\n新位置");
        assert_eq!(turns.combined().confirmed_text, "旧最终");
    }
    #[test]
    fn individually_legal_turns_cannot_exceed_the_recording_budget() {
        let mut turns = RawTurns::default();
        let first = "字".repeat(200000);
        turns
            .set(
                1,
                Transcript {
                    text: first.clone(),
                    ..Transcript::default()
                },
            )
            .unwrap();
        assert_eq!(
            turns
                .set(
                    2,
                    Transcript {
                        text: first.clone(),
                        ..Transcript::default()
                    }
                )
                .unwrap_err()
                .code,
            "responseTooLarge"
        );
        assert_eq!(turns.combined().text, first);
        let segments = |n| {
            (0..n)
                .map(|id| super::super::recognition::Segment {
                    id,
                    sequence: 1,
                    text: "x".into(),
                    confirmed: true,
                    begin_ms: None,
                    end_ms: None,
                })
                .collect()
        };
        turns
            .set(
                1,
                Transcript {
                    segments: segments(6000),
                    ..Transcript::default()
                },
            )
            .unwrap();
        assert_eq!(
            turns
                .set(
                    2,
                    Transcript {
                        segments: segments(6000),
                        ..Transcript::default()
                    }
                )
                .unwrap_err()
                .code,
            "responseTooLarge"
        );
        assert_eq!(turns.combined().segments.len(), 6000);
    }
    #[tokio::test]
    async fn editing_turns_discard_paused_pcm_and_only_current_tail_is_writable() {
        tokio::time::timeout(Duration::from_secs(6), async {
            let evidence = Arc::new(AtomicU64::new(7));
            let control = Control::new(evidence.clone());
            let route = Route { turn: 1, activity: 7 };
            control.resume(1, 7); control.enabled(route);
            control.draft_started(7);
            let (first, mut old) = RecognitionSession::controlled();
            let (next, mut current) = RecognitionSession::controlled();
            let available = Mutex::new(Some(next));
            let mut stream = Continuous::run(first, control.clone(), route, move |turn| {
                let next = available.lock().unwrap().take().unwrap();
                async move { Ok(Connection { turn, session: next, _slot: None }) }
            });
            stream.send_pcm(1, &[1, 2]).await.unwrap();
            assert_eq!(old.audio().await, [1, 2]);
            old.events.send(Ok(Event::Update(Transcript {text:"旧草稿".into(), ..Transcript::default()}))).await.unwrap();
            let value = stream.events.recv().await.unwrap().unwrap();
            assert!(matches!(value,StreamEvent::Update(Update { draft: Some((1,ref text)), .. }) if text=="旧草稿"));
            evidence.fetch_add(1, Ordering::AcqRel);
            control.pause();
            stream.send_pcm(1, &[99]).await.unwrap();
            old.finished_audio().await;
            control.resume(2, 8);
            control.readiness().wait_for(|turn|*turn==2).await.unwrap();
            stream.send_pcm(2, &[3, 4]).await.unwrap();
            assert_eq!(current.audio().await, [3, 4]);
            current.events.send(Ok(Event::Update(Transcript {text:"新草稿".into(), ..Transcript::default()}))).await.unwrap();
            old.events.send(Ok(Event::Update(Transcript {text:"旧尾字".into(),confirmed_text:"旧尾字".into(),..Transcript::default()}))).await.unwrap();
            old.events.send(Ok(Event::Finished)).await.unwrap();
            let (finished, ()) = tokio::join!(stream.finish(), current.finished_audio());
            finished.unwrap();
            current.events.send(Ok(Event::Update(Transcript {text:"新尾字🙂".into(),confirmed_text:"新尾字🙂".into(),..Transcript::default()}))).await.unwrap();
            current.events.send(Ok(Event::Finished)).await.unwrap();
            let mut saw_tail = false;
            let mut raw = String::new();
            loop {
                match stream.events.recv().await.unwrap().unwrap() {
                    StreamEvent::Update(value) => {
                        raw = value.raw.text;
                        if let Some((turn,text)) = value.draft {
                            assert_eq!(turn,2);
                            assert!(!text.contains("旧"));
                            saw_tail |= text=="新尾字🙂";
                        }
                    },
                    StreamEvent::Finished=>break, StreamEvent::Heartbeat=>{}
                }
            }
            assert!(saw_tail);
            assert_eq!(raw,"旧尾字\n新尾字🙂");
        }).await.unwrap();
    }
    #[tokio::test]
    async fn finish_cancels_pending_turn_preparation_without_restarting_audio() {
        tokio::time::timeout(Duration::from_secs(6), async {
            let control = Control::new(Arc::new(AtomicU64::new(0)));
            let route = Route {
                turn: 1,
                activity: 0,
            };
            control.resume(1, 0);
            control.enabled(route);
            let (first, mut old) = RecognitionSession::controlled();
            let (opened, mut requests) = mpsc::channel(1);
            let mut stream = Continuous::run(first, control.clone(), route, move |turn| {
                let opened = opened.clone();
                async move {
                    opened.send(turn).await.unwrap();
                    std::future::pending::<Result<Connection, Error>>().await
                }
            });
            control.pause();
            old.finished_audio().await;
            old.events.send(Ok(Event::Finished)).await.unwrap();
            control.resume(2, 0);
            assert_eq!(requests.recv().await, Some(2));
            stream.finish().await.unwrap();
            while !matches!(
                stream.events.recv().await.unwrap().unwrap(),
                StreamEvent::Finished
            ) {}
            assert_eq!(control.audio.current(), 0);
            assert_eq!(*control.readiness().borrow(), 0);
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn hard_cancellation_keeps_parsed_retired_tail_before_the_final_packet() {
        let control = Control::new(Arc::new(AtomicU64::new(0)));
        let route = Route {
            turn: 1,
            activity: 0,
        };
        control.resume(1, 0);
        control.enabled(route);
        control.draft_started(0);
        let (first, mut old) = RecognitionSession::controlled();
        let mut stream = Continuous::run(first, control.clone(), route, |_| async {
            Err(Error::new("unexpectedOpen"))
        });
        control.handoff_started();
        control.pause();
        old.finished_audio().await;
        old.events
            .send(Ok(Event::Update(Transcript {
                text: "已解析但尚未终结的尾字".into(),
                ..Transcript::default()
            })))
            .await
            .unwrap();
        let value = tokio::time::timeout(Duration::from_secs(2), stream.events.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(matches!(
            value,
            StreamEvent::Update(Update { draft: None, .. })
        ));
        drop(stream); // no Finished; mirrors cancellation during a retiring socket
        assert!(control.preserve_history());
        assert_eq!(control.latest_raw().unwrap().text, "已解析但尚未终结的尾字");
    }
}
