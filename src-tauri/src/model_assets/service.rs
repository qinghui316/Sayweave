use super::{
    ModelLease,
    catalog::{CATALOG, Descriptor},
};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex as AsyncMutex, watch};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub asset_id: String,
    pub state: String,
    pub received: u64,
    pub bytes: u64,
    pub error: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Partial {
    source: String,
    etag: String,
}

/// All registered assets use the same per-asset operation/read-lease contract.
pub struct Assets {
    entries: std::collections::BTreeMap<&'static str, Arc<Asset>>,
}
impl Assets {
    pub fn new(root: PathBuf) -> Result<Arc<Self>, &'static str> {
        Self::with_catalog(root, CATALOG)
    }
    pub(crate) fn with_catalog(
        root: PathBuf,
        catalog: &'static [Descriptor],
    ) -> Result<Arc<Self>, &'static str> {
        let mut entries = std::collections::BTreeMap::new();
        let mut files = std::collections::BTreeSet::new();
        for descriptor in catalog {
            if descriptor.id.is_empty()
                || descriptor.file.is_empty()
                || descriptor.file.contains(['/', '\\', ':'])
                || !descriptor.file.ends_with(".gguf")
                || descriptor.bytes == 0
                || descriptor.sha256.len() != 64
                || !descriptor.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                || !files.insert(descriptor.file.to_ascii_lowercase())
                || entries
                    .insert(descriptor.id, Asset::new(root.clone(), descriptor)?)
                    .is_some()
            {
                return Err("invalid");
            }
        }
        Ok(Arc::new(Self { entries }))
    }
    fn entry(&self, id: &str) -> Result<&Arc<Asset>, &'static str> {
        self.entries.get(id).ok_or("invalid")
    }
    pub fn descriptor(&self, id: &str) -> Result<&'static Descriptor, &'static str> {
        Ok(self.entry(id)?.descriptor)
    }
    pub async fn status(&self, id: &str) -> Result<Status, &'static str> {
        self.entry(id)?.status(id).await
    }
    pub async fn lease(&self, id: &str) -> Result<Arc<ModelLease>, &'static str> {
        self.entry(id)?.lease(id).await
    }
    pub fn cancel(&self, id: &str) -> Result<(), &'static str> {
        self.entry(id)?.cancel(id)
    }
    pub async fn download(&self, id: &str) -> Result<Status, &'static str> {
        self.entry(id)?.download(id).await
    }
    pub async fn delete(&self, id: &str) -> Result<(), &'static str> {
        self.entry(id)?.delete(id).await
    }
    pub async fn shutdown(&self) {
        for entry in self.entries.values() {
            entry.shutdown().await;
        }
    }
}
struct Asset {
    root: PathBuf,
    descriptor: &'static Descriptor,
    client: reqwest::Client,
    gate: Arc<AsyncMutex<()>>,
    state: Mutex<Status>,
    verified: Mutex<Option<Arc<ModelLease>>>,
    cancel: Mutex<Option<watch::Sender<bool>>>,
    jobs: Mutex<Vec<tokio::task::JoinHandle<()>>>,
    stopping: AtomicBool,
}
impl Asset {
    pub fn new(root: PathBuf, descriptor: &'static Descriptor) -> Result<Arc<Self>, &'static str> {
        // Redirects must remain HTTPS. The pinned digest, never a redirected filename,
        // determines identity. Credentials and renderer-supplied URLs are not accepted.
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() > 8 || attempt.url().scheme() != "https" {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .build()
            .map_err(|_| "localDownloadUnavailable")?;
        Ok(Arc::new(Self {
            root,
            descriptor,
            client,
            gate: Arc::new(AsyncMutex::new(())),
            state: Mutex::new(Status {
                asset_id: descriptor.id.into(),
                state: "missing".into(),
                received: 0,
                bytes: descriptor.bytes,
                error: None,
            }),
            verified: Mutex::new(None),
            cancel: Mutex::new(None),
            jobs: Mutex::new(vec![]),
            stopping: AtomicBool::new(false),
        }))
    }
    fn check(&self, id: &str) -> Result<(), &'static str> {
        if id != self.descriptor.id {
            return Err("invalid");
        }
        if self.stopping.load(Ordering::Acquire) {
            return Err("stopping");
        }
        Ok(())
    }
    fn path(&self) -> PathBuf {
        self.root.join(self.descriptor.file)
    }
    fn set(&self, state: &str, received: u64, error: Option<&str>) {
        if let Ok(mut current) = self.state.lock() {
            current.state = state.into();
            current.received = received;
            current.error = error.map(String::from);
        }
    }
    pub async fn status(&self, id: &str) -> Result<Status, &'static str> {
        self.check(id)?;
        super::validate_path(&self.path())?;
        let current = self.state.lock().map_err(|_| "unavailable")?.clone();
        if current.state == "missing" && self.path().is_file() {
            // Status is metadata-only. Cloud users opening settings never scan
            // a large model; explicit local selection verifies actual bytes.
            match std::fs::metadata(self.path()) {
                Ok(metadata) if metadata.len() == self.descriptor.bytes => {
                    self.set("available", self.descriptor.bytes, None)
                }
                _ => self.set("failed", 0, Some("localModelCorrupt")),
            }
        }
        self.state
            .lock()
            .map_err(|_| "unavailable")
            .map(|s| s.clone())
    }
    pub async fn lease(&self, id: &str) -> Result<Arc<ModelLease>, &'static str> {
        self.check(id)?;
        let guard = self.gate.clone().try_lock_owned().map_err(|_| "busy")?;
        if let Some(lease) = self.verified.lock().map_err(|_| "unavailable")?.as_ref() {
            return Ok(lease.clone());
        }
        let path = self.path();
        let descriptor = self.descriptor;
        let result = tokio::task::spawn_blocking(move || {
            ModelLease::acquire(&path, descriptor.bytes, descriptor.sha256)
                .map(|lease| (lease, guard))
        })
        .await
        .map_err(|_| "localModelUnreadable")?;
        let (lease, _guard) = match result {
            Ok(verified) => verified,
            Err(code) => {
                self.set("failed", 0, Some(code));
                return Err(code);
            }
        };
        *self.verified.lock().map_err(|_| "unavailable")? = Some(lease.clone());
        self.set("ready", self.descriptor.bytes, None);
        Ok(lease)
    }
    pub fn cancel(&self, id: &str) -> Result<(), &'static str> {
        self.check(id)?;
        if let Some(cancel) = self.cancel.lock().map_err(|_| "unavailable")?.as_ref() {
            cancel.send_replace(true);
        }
        Ok(())
    }
    pub async fn download(self: &Arc<Self>, id: &str) -> Result<Status, &'static str> {
        self.check(id)?;
        let guard = self.gate.clone().try_lock_owned().map_err(|_| "busy")?;
        if self.verified.lock().map_err(|_| "unavailable")?.is_some() {
            return Err("busy");
        }
        let (tx, rx) = watch::channel(false);
        *self.cancel.lock().map_err(|_| "unavailable")? = Some(tx);
        self.set("downloading", 0, None);
        let this = self.clone();
        let job = tokio::spawn(async move {
            let _guard = guard;
            let result = this.transfer(rx).await;
            match result {
                Ok(()) => this.set("ready", this.descriptor.bytes, None),
                Err("cancelled") => this.set("missing", 0, None),
                Err(code) => this.set("failed", 0, Some(code)),
            }
            if let Ok(mut cancel) = this.cancel.lock() {
                *cancel = None;
            }
        });
        // Remove completed handles so repeated download/retry has bounded bookkeeping.
        let mut jobs = self.jobs.lock().map_err(|_| "unavailable")?;
        jobs.retain(|job| !job.is_finished());
        jobs.push(job);
        self.state
            .lock()
            .map_err(|_| "unavailable")
            .map(|s| s.clone())
    }
    async fn transfer(&self, cancel: watch::Receiver<bool>) -> Result<(), &'static str> {
        let part = self.root.join(format!("{}.part", self.descriptor.file));
        let metadata = part.with_extension("json");
        super::validate_path(&self.path())?;
        super::validate_path(&part)?;
        super::validate_path(&metadata)?;
        tokio::task::block_in_place(|| std::fs::create_dir_all(&self.root)).map_err(io_error)?;
        let mut last = "localDownloadUnavailable";
        for source in self.descriptor.sources {
            if *cancel.borrow() {
                return Err("cancelled");
            }
            let result = tokio::time::timeout(
                Duration::from_secs(900),
                self.fetch(source, &part, &metadata, cancel.clone()),
            )
            .await
            .unwrap_or(Err("localDownloadTimeout"));
            match result {
                Ok(()) => return Ok(()),
                Err(code @ ("localDiskFull" | "localFileAccess" | "cancelled")) => {
                    return Err(code);
                }
                Err(code) => last = code,
            }
        }
        Err(last)
    }
    async fn fetch(
        &self,
        source: &str,
        part: &std::path::Path,
        metadata: &std::path::Path,
        mut cancel: watch::Receiver<bool>,
    ) -> Result<(), &'static str> {
        super::validate_path(part)?;
        super::validate_path(metadata)?;
        let saved = tokio::task::block_in_place(|| {
            use std::io::Read;
            let file = std::fs::File::open(metadata).ok()?;
            if file.metadata().ok()?.len() > 8192 {
                return None;
            }
            let mut bytes = Vec::new();
            file.take(8193).read_to_end(&mut bytes).ok()?;
            (bytes.len() <= 8192).then_some(bytes)
        })
        .and_then(|b| serde_json::from_slice::<Partial>(&b).ok())
        .filter(|s| {
            s.source == source
                && s.etag.len() >= 2
                && s.etag.starts_with('"')
                && s.etag.ends_with('"')
        });
        let offset = if saved.is_some() {
            tokio::fs::metadata(part)
                .await
                .map(|m| m.len())
                .unwrap_or(0)
                .min(self.descriptor.bytes)
        } else {
            0
        };
        let mut request = self
            .client
            .get(source)
            .header(reqwest::header::ACCEPT_ENCODING, "identity");
        if offset > 0 {
            request = request
                .header(reqwest::header::RANGE, format!("bytes={offset}-"))
                .header(reqwest::header::IF_RANGE, &saved.as_ref().unwrap().etag);
        }
        let response = tokio::select! {biased;_=cancel.changed()=>return Err("cancelled"),response=request.send()=>response.map_err(|_|"localDownloadUnavailable")?};
        let resumed = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
        if !response.status().is_success() {
            return Err("localDownloadUnavailable");
        }
        if resumed {
            let expected = format!(
                "bytes {offset}-{}/{}",
                self.descriptor.bytes - 1,
                self.descriptor.bytes
            );
            if offset == 0
                || response
                    .headers()
                    .get(reqwest::header::CONTENT_RANGE)
                    .and_then(|v| v.to_str().ok())
                    != Some(expected.as_str())
            {
                discard_resume(metadata)?;
                return Err("localDownloadRange");
            }
        }
        let start = if resumed { offset } else { 0 };
        if response
            .content_length()
            .is_some_and(|n| n != self.descriptor.bytes - start)
        {
            return Err("localModelCorrupt");
        }
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if resumed && saved.as_ref().is_some_and(|old| old.etag != etag) {
            discard_resume(metadata)?;
            return Err("localDownloadRange");
        }
        let mut file = tokio::task::block_in_place(|| {
            std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(resumed)
                .truncate(!resumed)
                .open(part)
        })
        .map_err(io_error)?;
        let marker = serde_json::to_vec(&Partial {
            source: source.into(),
            etag,
        })
        .map_err(|_| "localDownloadUnavailable")?;
        tokio::task::block_in_place(|| std::fs::write(metadata, marker)).map_err(io_error)?;
        let mut received = start;
        let mut stream = response.bytes_stream();
        loop {
            let bytes = tokio::select! {biased;_=cancel.changed()=>return Err("cancelled"),bytes=stream.next()=>bytes};
            let Some(bytes) = bytes else {
                break;
            };
            let bytes = bytes.map_err(|_| "localDownloadUnavailable")?;
            received = received
                .checked_add(bytes.len() as u64)
                .filter(|n| *n <= self.descriptor.bytes)
                .ok_or("localModelCorrupt")?;
            tokio::task::block_in_place(|| file.write_all(&bytes)).map_err(io_error)?;
            self.set("downloading", received, None);
        }
        tokio::task::block_in_place(|| file.sync_all()).map_err(io_error)?;
        drop(file);
        if received != self.descriptor.bytes {
            return Err("localModelCorrupt");
        }
        self.set("verifying", received, None);
        let path = part.to_path_buf();
        let descriptor = self.descriptor;
        // This is a filesystem transaction on a compensated runtime thread.
        // It cannot detach a hash/rename and release the operation guard early.
        tokio::task::block_in_place(|| {
            super::validate_path(&self.path())?;
            let lease = match ModelLease::acquire(&path, descriptor.bytes, descriptor.sha256) {
                Ok(lease) => lease,
                Err(code) => {
                    if code == "localModelCorrupt" {
                        let _ = std::fs::remove_file(part);
                        let _ = std::fs::remove_file(metadata);
                    }
                    return Err(code);
                }
            };
            drop(lease);
            if *cancel.borrow() {
                return Err("cancelled");
            }
            std::fs::rename(part, self.path()).map_err(io_error)?;
            let _ = std::fs::remove_file(metadata);
            Ok(())
        })
    }
    pub async fn delete(&self, id: &str) -> Result<(), &'static str> {
        self.check(id)?;
        let _guard = self.gate.try_lock().map_err(|_| "busy")?;
        {
            let mut lease = self.verified.lock().map_err(|_| "unavailable")?;
            if lease
                .as_ref()
                .is_some_and(|lease| Arc::strong_count(lease) > 1)
            {
                return Err("busy");
            }
            *lease = None;
        }
        tokio::task::block_in_place(|| {
            for path in [
                self.path(),
                self.root.join(format!("{}.part", self.descriptor.file)),
                self.root.join(format!("{}.json", self.descriptor.file)),
            ] {
                super::validate_path(&path)?;
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(io_error(e)),
                }
            }
            Ok::<_, &'static str>(())
        })?;
        self.set("missing", 0, None);
        Ok(())
    }
    pub async fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        if let Ok(cancel) = self.cancel.lock()
            && let Some(tx) = cancel.as_ref()
        {
            tx.send_replace(true);
        }
        let jobs = self
            .jobs
            .lock()
            .map(|mut jobs| std::mem::take(&mut *jobs))
            .unwrap_or_default();
        for job in jobs {
            let _ = job.await;
        }
        let _guard = self.gate.lock().await;
        if let Ok(mut lease) = self.verified.lock() {
            *lease = None;
        }
    }
}
fn discard_resume(metadata: &std::path::Path) -> Result<(), &'static str> {
    super::validate_path(metadata)?;
    tokio::task::block_in_place(|| match std::fs::remove_file(metadata) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(error)),
    })
}
fn io_error(error: std::io::Error) -> &'static str {
    match error.kind() {
        std::io::ErrorKind::StorageFull => "localDiskFull",
        _ => "localFileAccess",
    }
}
