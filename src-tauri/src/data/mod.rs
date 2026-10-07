mod cancellation;
pub mod model;
mod repositories;
pub mod repository;
#[cfg(test)]
mod tests;
use model::*;
use repository::Repository;
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Instant,
};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
pub enum Operation {
    Retry,
    List(Query),
    Get(Kind, String),
    Create(Input),
    Update(Input, String, i64),
    Delete(Kind, String, i64),
    Models(crate::contracts::models::DbOperation),
    History(crate::contracts::history::DbOperation),
    Shortcuts(crate::contracts::shortcuts::DbOperation),
    Processing(crate::contracts::processing::DbOperation),
    Drain,
}
#[derive(Debug)]
pub enum Output {
    Status(Status),
    Page(Page),
    Record(Record),
    Deleted,
    Models(Result<crate::contracts::models::DbOutput, crate::contracts::models::ModelError>),
    History(crate::contracts::history::DbOutput),
    Shortcuts(crate::contracts::shortcuts::DbOutput),
    Processing(crate::contracts::processing::DbOutput),
}
struct Request {
    operation: Operation,
    response: oneshot::Sender<Result<Output, DataError>>,
    queued: Instant,
}
#[derive(Clone)]
pub struct DataWorker {
    sender: mpsc::Sender<Request>,
    status: Arc<Mutex<Status>>,
    stopping: Arc<AtomicBool>,
    dispatch: Arc<Mutex<()>>,
    root: PathBuf,
    journal_io: Arc<Mutex<()>>,
    deferred_cancellations: Arc<Mutex<HashSet<String>>>,
    lease_active: Arc<AtomicBool>,
}
pub fn profile_root(base: &Path, development: bool) -> PathBuf {
    base.join(if development { "data-dev" } else { "data" })
}
pub fn acquire(root: &Path) -> Result<File, DataError> {
    fs::create_dir_all(root).map_err(|_| DataError::new("unavailable"))?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("instance.lock"))
        .map_err(|_| DataError::new("unavailable"))?;
    file.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => DataError::new("alreadyRunning"),
        _ => DataError::new("unavailable"),
    })?;
    Ok(file)
}
impl DataWorker {
    pub fn start(root: PathBuf) -> Result<Self, DataError> {
        let lease = match acquire(&root) {
            Ok(file) => Some(file),
            Err(error) if error.code == "alreadyRunning" => return Err(error),
            Err(_) => None,
        };
        let (sender, mut receiver) = mpsc::channel::<Request>(64);
        let status = Arc::new(Mutex::new(Status {
            state: "preparing".into(),
            error: None,
            schema_version: 0,
            normalization_version: 1,
        }));
        let shared = status.clone();
        let journal_io = Arc::new(Mutex::new(()));
        let thread_journal = journal_io.clone();
        let deferred_cancellations = Arc::new(Mutex::new(HashSet::new()));
        let thread_deferred = deferred_cancellations.clone();
        let worker_root = root.clone();
        let lease_active = Arc::new(AtomicBool::new(lease.is_some()));
        let thread_lease = lease_active.clone();
        std::thread::Builder::new()
            .name("sayweave-data".into())
            .spawn(move || {
                let mut lease = lease;
                let mut repository = if lease.is_some() {
                    match thread_journal.lock() {
                        Ok(_guard) => Repository::open(&root),
                        Err(_) => Err(DataError::new("unavailable")),
                    }
                } else {
                    Err(DataError::new("unavailable"))
                };
                Self::set_status(&shared, &repository);
                while let Some(request) = receiver.blocking_recv() {
                    let queued = request.queued.elapsed();
                    let started = Instant::now();
                    let name = match &request.operation {
                        Operation::Retry => "retry",
                        Operation::List(_) => "list",
                        Operation::Get(_, _) => "get",
                        Operation::Create(_) => "create",
                        Operation::Update(_, _, _) => "update",
                        Operation::Delete(_, _, _) => "delete",
                        Operation::Models(_) => "models",
                        Operation::History(_) => "history",
                        Operation::Shortcuts(_) => "shortcuts",
                        Operation::Processing(_) => "processing",
                        Operation::Drain => "drain",
                    };
                    let result = if matches!(request.operation, Operation::Retry) {
                        if thread_deferred.lock().map(|ids|!ids.is_empty()).unwrap_or(true) {
                            let _=request.response.send(Err(DataError::new("busy")));
                            continue;
                        }
                        if repository.is_err() {
                            if lease.is_none() {
                                lease = acquire(&root).ok();
                                thread_lease.store(lease.is_some(), Ordering::Release);
                            }
                            repository = if lease.is_some() {
                                match thread_journal.lock() {
                                    Ok(_guard) => Repository::open(&root),
                                    Err(_) => Err(DataError::new("unavailable")),
                                }
                            } else {
                                Err(DataError::new("unavailable"))
                            };
                        }
                        let recovery = match repository.as_mut() {
                            Ok(repo) => match thread_journal.lock() {
                                Ok(_guard) => cancellation::replay(repo, &root),
                                Err(_) => Err(DataError::new("unavailable")),
                            },
                            Err(error) => Err(error.clone()),
                        };
                        if let Err(error) = recovery { repository = Err(error); }
                        Self::set_status(&shared, &repository);
                        shared
                            .lock()
                            .map(|s| Output::Status(s.clone()))
                            .map_err(|_| DataError::new("unavailable"))
                    } else {
                        match repository.as_mut() {
                            Err(error) => Err(error.clone()),
                            Ok(repo) => (|| {
                                let _journal = thread_journal
                                    .lock()
                                    .map_err(|_| DataError::new("unavailable"))?;
                                let deferred=thread_deferred.lock().map_err(|_|DataError::new("unavailable"))?;
                                cancellation::replay_except(repo, &root, &deferred)?;
                                if cancellation::blocks_read(repo, &request.operation, &deferred)? { return Err(DataError::new("busy")); }
                                drop(deferred);
                                drop(_journal);
                                match request.operation {
                                    Operation::List(query) => repo.list(query).map(Output::Page),
                                    Operation::Get(kind, id) => {
                                        repo.get(kind, &id).map(Output::Record)
                                    }
                                    Operation::Create(input) => {
                                        repo.save(input, None).map(Output::Record)
                                    }
                                    Operation::Update(input, id, revision) => {
                                        repo.save(input, Some((id, revision))).map(Output::Record)
                                    }
                                    Operation::Delete(kind, id, revision) => {
                                        repo.delete(kind, &id, revision).map(|_| Output::Deleted)
                                    }
                                    Operation::Retry => unreachable!(),
                                    Operation::History(op) => {
                                    if matches!(op,crate::contracts::history::DbOperation::PruneCancelled{..}) {
                                        Ok(Output::History(crate::contracts::history::DbOutput::Done))
                                    } else { repo.history_operation(op).map(Output::History) }
                                    }
                                    Operation::Shortcuts(op) => {
                                        repo.shortcut_operation(op).map(Output::Shortcuts)
                                    }
                                    Operation::Processing(op) => {
                                        if matches!(
                                            op,
                                            crate::contracts::processing::DbOperation::Cancel { .. }
                                        ) {
                                            Ok(Output::Processing(
                                                crate::contracts::processing::DbOutput::Done,
                                            ))
                                        } else {
                                            repo.processing_operation(op).map(Output::Processing)
                                        }
                                    }
                                    Operation::Models(op) => match repo.model_operation(op) {
                                        Err(e) if e.code == "unavailable" => {
                                            Err(DataError::new("unavailable"))
                                        }
                                        result => Ok(Output::Models(result)),
                                    },
                                    Operation::Drain => Ok(Output::Deleted),
                                }
                            })(),
                        }
                    };
                    if let Err(error) = &result
                        && error.code == "unavailable"
                    {
                        repository = Err(error.clone());
                        Self::set_status(&shared, &repository);
                    }
                    log::info!(target:"sayweave",
                        "SAYWEAVE_DATA operation={name} queue_us={} execution_us={} result={}",
                        queued.as_micros(),
                        started.elapsed().as_micros(),
                        result.as_ref().map(|_| "ok").unwrap_or_else(|e| &e.code)
                    );
                    let _ = request.response.send(result);
                }
                thread_lease.store(false, Ordering::Release);
            })
            .map_err(|_| DataError::new("unavailable"))?;
        Ok(Self {
            sender,
            status,
            stopping: Arc::new(AtomicBool::new(false)),
            dispatch: Arc::new(Mutex::new(())),
            root: worker_root,
            journal_io,
            deferred_cancellations,
            lease_active,
        })
    }
    fn set_status(shared: &Mutex<Status>, repository: &Result<Repository, DataError>) {
        if let Ok(mut status) = shared.lock() {
            status.state = if repository.is_ok() {
                "ready"
            } else {
                "unavailable"
            }
            .into();
            status.error = repository.as_ref().err().cloned();
            status.schema_version = repository
                .as_ref()
                .ok()
                .and_then(|r| {
                    r.connection
                        .query_row(
                            "SELECT coalesce(max(version),0) FROM schema_migrations",
                            [],
                            |row| row.get(0),
                        )
                        .ok()
                })
                .unwrap_or(0);
        }
    }
    pub fn status(&self) -> Result<Status, DataError> {
        self.status
            .lock()
            .map(|s| s.clone())
            .map_err(|_| DataError::new("unavailable"))
    }
    pub async fn request(&self, caller: &str, operation: Operation) -> Result<Output, DataError> {
        authorize(caller)?;
        self.request_inner(operation, false).await
    }
    // Model operations already admitted by the service must be allowed to commit their journal during exit.
    pub(crate) async fn request_admitted(&self, operation: Operation) -> Result<Output, DataError> {
        self.request_inner(operation, true).await
    }
    /// Already admitted ordinary archive facts wait for bounded queue capacity
    /// in the background. Cancellation intents still use request_admitted.
    pub(crate) async fn request_archive(&self, operation: Operation) -> Result<Output, DataError> {
        let permit = tokio::time::timeout(std::time::Duration::from_secs(5), self.sender.reserve())
            .await
            .map_err(|_| DataError::new("busy"))?
            .map_err(|_| DataError::new("unavailable"))?;
        let (response, receive) = oneshot::channel();
        permit.send(Request {
            operation,
            response,
            queued: Instant::now(),
        });
        receive.await.map_err(|_| DataError::new("unavailable"))?
    }
    async fn request_inner(
        &self,
        operation: Operation,
        admitted: bool,
    ) -> Result<Output, DataError> {
        if matches!(
            &operation,
            Operation::Processing(crate::contracts::processing::DbOperation::Cancel { .. })
                | Operation::History(crate::contracts::history::DbOperation::PruneCancelled { .. })
        ) {
            if !self.lease_active.load(Ordering::Acquire)
                || (self.stopping.load(Ordering::Acquire) && !admitted)
            {
                return Err(DataError::new("unavailable"));
            }
            self.stage_cancel(&operation).await?;
            if let Some(id) = cancellation::identity(&operation) {
                self.deferred_cancellations
                    .lock()
                    .map_err(|_| DataError::new("unavailable"))?
                    .remove(id);
            }
        }
        let (response, receive) = oneshot::channel();
        {
            let _dispatch = self
                .dispatch
                .lock()
                .map_err(|_| DataError::new("unavailable"))?;
            if self.stopping.load(Ordering::Acquire) && !admitted {
                return Err(DataError::new("unavailable"));
            }
            self.sender
                .try_send(Request {
                    operation,
                    response,
                    queued: Instant::now(),
                })
                .map_err(|e| match e {
                    mpsc::error::TrySendError::Full(_) => DataError::new("busy"),
                    _ => DataError::new("unavailable"),
                })?;
        }
        receive.await.map_err(|_| DataError::new("unavailable"))?
    }
    /// Persist the cancellation barrier before waiting for ordinary archives.
    /// This does not execute SQL; request ordering remains with the archive owner.
    pub(crate) async fn stage_cancel(&self, operation: &Operation) -> Result<(), DataError> {
        if !self.lease_active.load(Ordering::Acquire) {
            return Err(DataError::new("unavailable"));
        }
        let root = self.root.clone();
        let journal = self.journal_io.clone();
        let intent = operation.clone();
        let deferred = self.deferred_cancellations.clone();
        tokio::task::spawn_blocking(move || {
            let _guard = journal.lock().map_err(|_| DataError::new("unavailable"))?;
            let staged = cancellation::stage(&root, &intent)?;
            if let Some(id) = cancellation::identity(&intent) {
                deferred
                    .lock()
                    .map_err(|_| DataError::new("unavailable"))?
                    .insert(id.into());
            }
            Ok::<_, DataError>(staged)
        })
        .await
        .map_err(|_| DataError::new("unavailable"))??;
        Ok(())
    }
    pub fn begin_shutdown(&self) {
        if let Ok(_guard) = self.dispatch.lock() {
            self.stopping.store(true, Ordering::Release);
        }
    }
    pub async fn shutdown(&self) {
        self.begin_shutdown();
        let (response, receive) = oneshot::channel();
        if self
            .sender
            .send(Request {
                operation: Operation::Drain,
                response,
                queued: Instant::now(),
            })
            .await
            .is_ok()
        {
            let _ = receive.await;
        }
        if let Ok(mut status) = self.status.lock() {
            status.state = "stopping".into();
        }
    }
}
