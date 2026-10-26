//! Serialized native compute leases, independent of recording and native-input permissions.
mod job;
use crate::build_info::local_runtime as payload;
use crate::model_assets::ModelLease;
use sayweave_local_asr_protocol::{
    Command, DevicePolicy, Event, MAX_JSON, RunId, VERSION, write_audio, write_json,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    process::{Child, ChildStdout},
    sync::{Mutex, Notify, OwnedMutexGuard, mpsc, oneshot},
};

fn child_environment(
    source: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<(std::ffi::OsString, std::ffi::OsString)> {
    const ALLOWED: &[&str] = &[
        "SYSTEMROOT",
        "WINDIR",
        "TEMP",
        "TMP",
        "LOCALAPPDATA",
        "APPDATA",
        "USERPROFILE",
        "PROGRAMDATA",
        "PROGRAMFILES",
        "PROGRAMFILES(X86)",
    ];
    source
        .into_iter()
        .filter(|(key, _)| {
            key.to_str()
                .is_some_and(|key| ALLOWED.contains(&key.to_ascii_uppercase().as_str()))
        })
        .collect()
}

#[derive(Clone)]
pub struct Runtime(Arc<Inner>);
struct Inner {
    program: PathBuf,
    payload: Vec<payload::RuntimeFile>,
    worker: Arc<Mutex<Option<Worker>>>,
    users: AtomicUsize,
    selected: AtomicBool,
    stopping: AtomicBool,
    changed: Notify,
}
pub struct TaskLease(Arc<Inner>);
#[derive(Clone)]
pub struct StreamSettings {
    pub language: String,
    pub right_context: i32,
}
impl Default for StreamSettings {
    fn default() -> Self {
        Self {
            language: "auto".into(),
            right_context: 6,
        }
    }
}
impl Drop for TaskLease {
    fn drop(&mut self) {
        self.0.users.fetch_sub(1, Ordering::AcqRel);
        self.0.changed.notify_waiters();
    }
}
struct Outgoing {
    bytes: Vec<u8>,
    done: oneshot::Sender<Result<(), &'static str>>,
}
struct Worker {
    child: Child,
    job: job::Job,
    input: mpsc::Sender<Outgoing>,
    writer: tokio::task::JoinHandle<()>,
    output: ChildStdout,
    buffered: Vec<u8>,
    model: Arc<ModelLease>,
    device: DevicePolicy,
    instance: String,
    prepared: Option<Event>,
    _files: Vec<std::fs::File>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.job.terminate();
        self.writer.abort();
    }
}

impl Worker {
    fn spawn(
        program: &PathBuf,
        model: Arc<ModelLease>,
        device: DevicePolicy,
        inventory: &[payload::RuntimeFile],
    ) -> Result<Self, &'static str> {
        let files = payload::hold(program, inventory)?;
        let mut command = tokio::process::Command::new(program);
        command
            .arg("--stdio")
            .current_dir(program.parent().ok_or("localRuntimeMissing")?)
            .env_clear()
            .envs(child_environment(std::env::vars_os()))
            .env("PATH", program.parent().ok_or("localRuntimeMissing")?)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x08000000);
        let mut child = command.spawn().map_err(|_| "localRuntimeMissing")?;
        let job = job::Job::attach(&child)?;
        let mut stdin = child.stdin.take().ok_or("localProcessUnavailable")?;
        let output = child.stdout.take().ok_or("localProcessUnavailable")?;
        let (input, mut commands) = mpsc::channel::<Outgoing>(16);
        let writer = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            while let Some(message) = commands.recv().await {
                let result = stdin
                    .write_all(&message.bytes)
                    .await
                    .map_err(|_| "localPipeClosed");
                let failed = result.is_err();
                let _ = message.done.send(result);
                if failed {
                    break;
                }
            }
        });
        Ok(Self {
            child,
            job,
            input,
            writer,
            output,
            buffered: Vec::with_capacity(8192),
            model,
            device,
            instance: uuid::Uuid::new_v4().to_string(),
            prepared: None,
            _files: files,
        })
    }
    async fn bytes(&self, bytes: Vec<u8>) -> Result<(), &'static str> {
        let (done, reply) = oneshot::channel();
        tokio::time::timeout(
            Duration::from_millis(500),
            self.input.send(Outgoing { bytes, done }),
        )
        .await
        .map_err(|_| "localBufferOverflow")?
        .map_err(|_| "localPipeClosed")?;
        tokio::time::timeout(Duration::from_secs(5), reply)
            .await
            .map_err(|_| "localPipeClosed")?
            .map_err(|_| "localPipeClosed")?
    }
    async fn command(&self, command: &Command) -> Result<(), &'static str> {
        let mut bytes = Vec::new();
        write_json(&mut bytes, command).map_err(|_| "localProtocol")?;
        self.bytes(bytes).await
    }
    async fn event(&mut self) -> Result<Event, &'static str> {
        loop {
            if self.buffered.len() >= 4 {
                let size = u32::from_le_bytes(self.buffered[..4].try_into().unwrap()) as usize;
                if size == 0 || size > MAX_JSON + 1 {
                    return Err("localProtocol");
                }
                if self.buffered.len() >= size + 4 {
                    if self.buffered[4] != 0 {
                        return Err("localProtocol");
                    }
                    let value = serde_json::from_slice(&self.buffered[5..size + 4])
                        .map_err(|_| "localProtocol")?;
                    self.buffered.drain(..size + 4);
                    return Ok(value);
                }
            }
            if self.buffered.len() > MAX_JSON + 5 {
                return Err("localProtocol");
            }
            // read_buf is cancellation-safe: bytes already read stay in Worker,
            // including when a select/Drop interrupts waiting for a full frame.
            if self
                .output
                .read_buf(&mut self.buffered)
                .await
                .map_err(|_| "localPipeClosed")?
                == 0
            {
                return Err("localPipeClosed");
            }
        }
    }
    async fn terminate_and_wait(&mut self) -> Result<(), &'static str> {
        self.job.terminate();
        self.child
            .wait()
            .await
            .map(|_| ())
            .map_err(|_| "localProcessUnavailable")
    }
    async fn close(&mut self) -> Result<(), &'static str> {
        self.prepared = None;
        let _ = self.command(&Command::Shutdown).await;
        match tokio::time::timeout(Duration::from_secs(1), self.child.wait()).await {
            Ok(Ok(_)) => Ok(()),
            _ => self.terminate_and_wait().await,
        }
    }
    async fn event_for(&mut self, expected: &RunId) -> Result<Event, &'static str> {
        loop {
            let event = self.event().await?;
            let id = match &event {
                Event::Prepared { id, .. }
                | Event::Ready { id }
                | Event::Update { id, .. }
                | Event::Finished { id }
                | Event::Cancelled { id } => Some(id),
                Event::Error { id, .. } => id.as_ref(),
                _ => None,
            };
            if id.is_some_and(|id| id != expected) {
                continue;
            }
            return Ok(event);
        }
    }
}
async fn close_slot(slot: &mut Option<Worker>) -> Result<(), &'static str> {
    if let Some(worker) = slot.as_mut() {
        worker.close().await?;
    }
    // Cancellation or an uncertain OS wait leaves the poisoned worker and its
    // file lease in this owner. Only confirmed exit permits their removal.
    *slot = None;
    Ok(())
}

struct Owner {
    guard: Option<OwnedMutexGuard<Option<Worker>>>,
    id: RunId,
    armed: bool,
    handle: tokio::runtime::Handle,
}
impl Owner {
    fn worker(&mut self) -> Result<&mut Worker, &'static str> {
        self.guard
            .as_mut()
            .and_then(|g| g.as_mut())
            .ok_or("localProcessUnavailable")
    }
    fn settled(&mut self) {
        self.armed = false;
    }
}
impl Drop for Owner {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Some(mut guard) = self.guard.take() else {
            return;
        };
        let Some(mut worker) = guard.take() else {
            return;
        };
        let id = self.id.clone();
        // Keep BOTH the compute mutex and file lease until native cancellation
        // has actually settled, or the killed process has actually exited.
        self.handle.spawn(async move {
            let cooperative = tokio::time::timeout(Duration::from_secs(1), async {
                worker.command(&Command::Cancel { id: id.clone() }).await?;
                loop {
                    match worker.event_for(&id).await? {
                        Event::Cancelled { id: done } | Event::Finished { id: done }
                            if done == id =>
                        {
                            return Ok::<_, &'static str>(());
                        }
                        Event::Error { .. } => return Err("localInferenceFailed"),
                        _ => {}
                    }
                }
            })
            .await;
            if matches!(cooperative, Ok(Ok(()))) && worker.prepared.is_some() {
                *guard = Some(worker);
            } else {
                worker.prepared = None;
                if worker.terminate_and_wait().await.is_err() {
                    *guard = Some(worker);
                }
            }
        });
    }
}

impl Runtime {
    pub fn new(program: PathBuf, payload: Vec<payload::RuntimeFile>) -> Self {
        Self(Arc::new(Inner {
            program,
            payload,
            worker: Arc::new(Mutex::new(None)),
            users: AtomicUsize::new(0),
            selected: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
            changed: Notify::new(),
        }))
    }
    pub fn task_lease(&self) -> Result<Arc<TaskLease>, &'static str> {
        if self.0.stopping.load(Ordering::Acquire) {
            return Err("stopping");
        }
        self.0.users.fetch_add(1, Ordering::AcqRel);
        Ok(Arc::new(TaskLease(self.0.clone())))
    }
    pub fn is_in_use(&self) -> bool {
        self.0.users.load(Ordering::Acquire) > 0
    }
    pub async fn unload_idle(&self) -> Result<(), &'static str> {
        if self.is_in_use() {
            return Err("busy");
        }
        let mut slot = self.0.worker.try_lock().map_err(|_| "busy")?;
        if self.is_in_use() {
            return Err("busy");
        }
        close_slot(&mut slot).await
    }
    pub fn select(&self, local: bool) {
        self.0.selected.store(local, Ordering::Release);
        self.0.changed.notify_waiters();
        if !local {
            let state = self.0.clone();
            tokio::spawn(async move {
                loop {
                    let changed = state.changed.notified();
                    tokio::pin!(changed);
                    changed.as_mut().enable();
                    if state.selected.load(Ordering::Acquire) {
                        return;
                    }
                    if state.users.load(Ordering::Acquire) == 0 {
                        let mut slot = state.worker.lock().await;
                        if !state.selected.load(Ordering::Acquire)
                            && state.users.load(Ordering::Acquire) == 0
                        {
                            if let Err(code) = close_slot(&mut slot).await {
                                log::warn!("local runtime release: {code}");
                            }
                            return;
                        }
                    }
                    changed.await;
                }
            });
        }
    }
    async fn owner(
        &self,
        id: RunId,
        model: Arc<ModelLease>,
        device: DevicePolicy,
    ) -> Result<Owner, &'static str> {
        if !id.is_valid() {
            return Err("localProtocol");
        }
        if self.0.stopping.load(Ordering::Acquire) {
            return Err("stopping");
        }
        let guard = self.0.worker.clone().lock_owned().await;
        let mut owner = Owner {
            guard: Some(guard),
            id: id.clone(),
            armed: true,
            handle: tokio::runtime::Handle::current(),
        };
        if self.0.stopping.load(Ordering::Acquire) {
            return Err("stopping");
        }
        let same = owner
            .guard
            .as_ref()
            .unwrap()
            .as_ref()
            .is_some_and(|worker| {
                worker.model.sha256 == model.sha256
                    && worker.model.path == model.path
                    && worker.device == device
                    && worker.prepared.is_some()
            });
        if !same {
            if let Err(code) = close_slot(owner.guard.as_mut().unwrap()).await {
                owner.settled();
                return Err(code);
            }
            **owner.guard.as_mut().unwrap() = Some(Worker::spawn(
                &self.0.program,
                model,
                device,
                &self.0.payload,
            )?);
        }
        let worker = owner.worker()?;
        if worker.prepared.is_none() {
            worker
                .command(&Command::Hello {
                    version: VERSION,
                    instance: worker.instance.clone(),
                })
                .await?;
            match worker.event().await? {
                Event::Hello {
                    version,
                    instance,
                    engine,
                } if version == VERSION && instance == worker.instance && engine == "0.1.3" => {}
                _ => return Err("localRuntimeMismatch"),
            }
            worker
                .command(&Command::Prepare {
                    id: id.clone(),
                    path: worker
                        .model
                        .path
                        .to_str()
                        .ok_or("localModelMissing")?
                        .into(),
                    sha256: worker.model.sha256.clone(),
                    device,
                })
                .await?;
            let event = worker.event_for(&id).await?;
            match &event {
                Event::Prepared {
                    id: done,
                    streaming: true,
                    ..
                } if done == &id => worker.prepared = Some(event),
                Event::Error { code, .. } => {
                    return Err(sayweave_local_asr_protocol::failure_code(code));
                }
                _ => return Err("localProtocol"),
            }
        }
        Ok(owner)
    }
    #[cfg(test)]
    pub(crate) async fn preparation(&self) -> Option<Event> {
        self.0
            .worker
            .lock()
            .await
            .as_ref()
            .and_then(|w| w.prepared.clone())
    }
    #[cfg(all(test, windows))]
    pub(crate) async fn process_memory(&self) -> Option<serde_json::Value> {
        use windows::Win32::{
            Foundation::HANDLE,
            System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS},
        };
        let owner = self.0.worker.lock().await;
        let worker = owner.as_ref()?;
        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        // SAFETY: the compute owner holds the live Child and the correctly sized output buffer.
        unsafe {
            GetProcessMemoryInfo(
                HANDLE(worker.child.raw_handle()?),
                &mut counters,
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            )
            .ok()?;
        }
        Some(
            serde_json::json!({"pid":worker.child.id(),"workingSetBytes":counters.WorkingSetSize,"peakWorkingSetBytes":counters.PeakWorkingSetSize,"pagefileBytes":counters.PagefileUsage,"peakPagefileBytes":counters.PeakPagefileUsage}),
        )
    }
    pub async fn prepare(
        &self,
        id: RunId,
        model: Arc<ModelLease>,
        device: DevicePolicy,
    ) -> Result<(), &'static str> {
        let mut owner = self.owner(id, model, device).await?;
        owner.settled();
        Ok(())
    }
    #[cfg(test)]
    pub async fn open(
        &self,
        id: RunId,
        model: Arc<ModelLease>,
        device: DevicePolicy,
    ) -> Result<Session, &'static str> {
        self.open_with_settings(id, model, device, &StreamSettings::default())
            .await
    }
    pub async fn open_with_settings(
        &self,
        id: RunId,
        model: Arc<ModelLease>,
        device: DevicePolicy,
        settings: &StreamSettings,
    ) -> Result<Session, &'static str> {
        let mut owner = self.owner(id.clone(), model, device).await?;
        owner
            .worker()?
            .command(&Command::Start {
                id: id.clone(),
                language: settings.language.clone(),
                right_context: settings.right_context,
            })
            .await?;
        loop {
            match owner.worker()?.event_for(&id).await? {
                Event::Ready { id: ready } if ready == id => break,
                Event::Error { code, .. } => {
                    return Err(sayweave_local_asr_protocol::failure_code(&code));
                }
                Event::Cancelled { id: old } | Event::Finished { id: old } if old != id => {}
                _ => return Err("localProtocol"),
            }
        }
        let (commands, mut receive) = mpsc::channel(32);
        let (send, events) = mpsc::channel(33);
        // Reserve the terminal error independently of the bounded update queue.
        // A stalled consumer must never retain the compute/file lease forever.
        let failure = send
            .clone()
            .reserve_owned()
            .await
            .map_err(|_| "localPipeClosed")?;
        let bridge = tokio::spawn(async move {
            let mut sequence = 0;
            let mut finishing = false;
            let result=async {
                loop {tokio::select!{
                    command=receive.recv()=>match command.ok_or("cancelled")? {
                        SessionCommand::Audio(pcm)=>{sequence+=1;let mut bytes=Vec::new();write_audio(&mut bytes,&id,sequence,&pcm).map_err(|_|"localBufferOverflow")?;owner.worker()?.bytes(bytes).await?;},
                        SessionCommand::Finish(ack)=>{let result=if !finishing{owner.worker()?.command(&Command::Finish{id:id.clone()}).await}else{Ok(())};if result.is_ok(){finishing=true;}let _=ack.send(result);result?;}
                    },
                    event=async{owner.worker()?.event_for(&id).await}=>{
                        let event=event?;
                        match &event {
                            Event::Update{id:current,..} if current==&id=>{},
                            Event::Finished{id:current} if current==&id&&finishing=>{publish(&send,event).await?;owner.settled();return Ok(());},
                            Event::Cancelled{id:old}|Event::Finished{id:old} if old!=&id=>continue,
                            Event::Error{code,..}=>return Err(sayweave_local_asr_protocol::failure_code(code)),
                            _=>return Err("localProtocol")
                        }
                        publish(&send,event).await?;
                    }
                }}
            }.await;
            if let Err(code) = result {
                log::warn!(
                    "local ASR bridge task={} turn={} request={}: {code}",
                    id.task,
                    id.turn,
                    id.request
                );
                failure.send(Err(code));
            }
        });
        Ok(Session {
            commands,
            events,
            bridge,
        })
    }
    pub async fn shutdown(&self) -> Result<(), &'static str> {
        self.0.stopping.store(true, Ordering::Release);
        self.select(false);
        let mut guard = self.0.worker.lock().await;
        close_slot(&mut guard).await
    }
}
enum SessionCommand {
    Audio(Vec<i16>),
    Finish(oneshot::Sender<Result<(), &'static str>>),
}
async fn publish(
    send: &mpsc::Sender<Result<Event, &'static str>>,
    event: Event,
) -> Result<(), &'static str> {
    tokio::time::timeout(Duration::from_millis(500), send.send(Ok(event)))
        .await
        .map_err(|_| "localBufferOverflow")?
        .map_err(|_| "cancelled")
}
pub struct Session {
    commands: mpsc::Sender<SessionCommand>,
    events: mpsc::Receiver<Result<Event, &'static str>>,
    bridge: tokio::task::JoinHandle<()>,
}
impl Session {
    pub async fn send_pcm(&self, pcm: &[i16]) -> Result<(), &'static str> {
        if pcm.len() > sayweave_local_asr_protocol::MAX_PCM_SAMPLES {
            return Err("localBufferOverflow");
        }
        tokio::time::timeout(
            Duration::from_millis(500),
            self.commands.send(SessionCommand::Audio(pcm.to_vec())),
        )
        .await
        .map_err(|_| "localBufferOverflow")?
        .map_err(|_| "localPipeClosed")
    }
    pub async fn finish(&self) -> Result<(), &'static str> {
        let (done, reply) = oneshot::channel();
        self.commands
            .send(SessionCommand::Finish(done))
            .await
            .map_err(|_| "localPipeClosed")?;
        reply.await.map_err(|_| "localPipeClosed")?
    }
    pub async fn next_event(&mut self) -> Result<Event, &'static str> {
        self.events.recv().await.ok_or("localPipeClosed")?
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.bridge.abort();
    }
}
#[cfg(test)]
mod tests;
#[cfg(test)]
pub(crate) use tests::setup as test_runtime;
