//! Native-owned update transaction. Renderer calls carry the exact candidate identity.
use crate::{
    build_info::{self, BuildInfo},
    network::{RequestCoordinator, RequestError},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha512};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{Emitter, Manager, State, WebviewWindow};

const MAX_PACKAGE: usize = 128 * 1024 * 1024;
const MAX_METADATA: usize = 64 * 1024;
const FEED: &str = "https://github.com/qinghui316/Sayweave/releases/latest/download/latest.json";
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    revision: u64,
    task_id: Option<String>,
    state: String,
    version: Option<String>,
    progress: Option<u8>,
    error: Option<String>,
    failure_stage: Option<String>,
    blocked_reason: Option<String>,
    recovery: bool,
    notes: Option<String>,
    build: BuildInfo,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Candidate {
    version: String,
    signature: String,
    url: String,
    installer_size: usize,
    installer_sha512: String,
    notes: String,
}
#[derive(Deserialize)]
struct Manifest {
    version: String,
    #[serde(default)]
    notes: String,
    platforms: std::collections::HashMap<String, Platform>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Platform {
    signature: String,
    url: String,
    installer_size: usize,
    installer_sha512: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Journal {
    schema_version: u8,
    candidate: Candidate,
    stage: String,
}
struct Inner {
    snapshot: Snapshot,
    candidate: Option<Candidate>,
    last_check: u64,
}
pub struct UpdateService {
    inner: Mutex<Inner>,
    root: PathBuf,
    requests: Arc<RequestCoordinator>,
    key: String,
    feed: String,
}
#[derive(Debug, Serialize)]
pub struct Error {
    code: String,
}
impl Error {
    fn new(code: &str) -> Self {
        Self { code: code.into() }
    }
}
impl From<RequestError> for Error {
    fn from(value: RequestError) -> Self {
        Self::new(&value.code)
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn bounded_read(path: &Path, limit: usize) -> Result<Vec<u8>, Error> {
    let file = fs::File::open(path).map_err(|_| Error::new("cacheRead"))?;
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::new("cacheRead"))?;
    if bytes.len() > limit {
        return Err(Error::new("tooLarge"));
    }
    Ok(bytes)
}
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| Error::new("cacheWrite"))?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| Error::new("cacheWrite"))?;
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
            }
            let from: Vec<_> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
            let to: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            // Owned terminated paths; replace atomically and request durable metadata.
            if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 1 | 8) } == 0 {
                return Err(Error::new("cacheWrite"));
            }
        }
        #[cfg(not(windows))]
        fs::rename(&temporary, path).map_err(|_| Error::new("cacheWrite"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
fn version(value: &str) -> Option<(u64, u64, u64)> {
    let parts: Vec<_> = value.split('.').collect();
    if parts.len() != 3
        || parts.iter().any(|v| {
            v.is_empty()
                || !v.bytes().all(|c| c.is_ascii_digit())
                || (v.len() > 1 && v.starts_with('0'))
        })
    {
        return None;
    }
    Some((
        parts[0].parse().ok()?,
        parts[1].parse().ok()?,
        parts[2].parse().ok()?,
    ))
}
fn validate_candidate(candidate: &Candidate, test: bool) -> Result<(), Error> {
    if version(&candidate.version).is_none()
        || candidate.installer_size == 0
        || candidate.installer_size > MAX_PACKAGE
        || candidate.notes.len() > 16 * 1024
    {
        return Err(Error::new("invalidMetadata"));
    }
    let expected = format!(
        "https://github.com/qinghui316/Sayweave/releases/download/v{}/Sayweave_{}_x64-setup.exe",
        candidate.version, candidate.version
    );
    if (!test && candidate.url != expected)
        || !candidate.url.starts_with("https://")
        || !STANDARD
            .decode(&candidate.installer_sha512)
            .is_ok_and(|v| v.len() == 64)
    {
        return Err(Error::new("invalidMetadata"));
    }
    Ok(())
}
impl UpdateService {
    pub fn new(root: &Path, requests: Arc<RequestCoordinator>) -> Result<Self, Error> {
        let build = build_info::current();
        let config: serde_json::Value = serde_json::from_str(include_str!("../tauri.conf.json"))
            .map_err(|_| Error::new("configuration"))?;
        if config["plugins"]["updater"]["requireSignedVersion"] != true {
            return Err(Error::new("configuration"));
        }
        let (feed, key) = if build.channel == "test" {
            (
                option_env!("SAYWEAVE_TEST_UPDATE_URL")
                    .unwrap_or("")
                    .to_owned(),
                option_env!("SAYWEAVE_TEST_UPDATE_KEY")
                    .unwrap_or("")
                    .to_owned(),
            )
        } else {
            (
                FEED.into(),
                config["plugins"]["updater"]["pubkey"]
                    .as_str()
                    .ok_or(Error::new("configuration"))?
                    .into(),
            )
        };
        let root = root.join("update-cache");
        let cache_available = fs::create_dir_all(&root).is_ok();
        let last_check = bounded_read(&root.join("last-check"), 32)
            .ok()
            .and_then(|v| String::from_utf8(v).ok())
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        let mut snapshot = Snapshot {
            revision: 1,
            task_id: None,
            state: "idle".into(),
            version: None,
            progress: None,
            error: None,
            failure_stage: None,
            blocked_reason: None,
            recovery: false,
            notes: None,
            build,
        };
        let mut candidate = None;
        if !cache_available {
            snapshot.state = "failed".into();
            snapshot.error = Some("无法准备更新缓存，当前程序可以继续使用。".into());
            snapshot.failure_stage = Some("cacheWrite".into());
        }
        if snapshot.build.updates_enabled
            && let Ok(bytes) = bounded_read(&root.join("transaction.json"), MAX_METADATA)
            && let Ok(journal) = serde_json::from_slice::<Journal>(&bytes)
            && journal.schema_version == 1
        {
            let verified_package = bounded_read(&root.join("candidate.exe"), MAX_PACKAGE)
                .ok()
                .filter(|bytes| verify_package(bytes, &journal.candidate, &key).is_ok());
            if journal.candidate.version == snapshot.build.version
                && verified_package.as_ref().is_some_and(|bytes| {
                    crate::installer::acknowledge_running(&crate::installer::package_hash(bytes))
                })
            {
                let _ = fs::remove_file(root.join("candidate.exe"));
                let _ = fs::remove_file(root.join("transaction.json"));
            } else if validate_candidate(&journal.candidate, snapshot.build.channel == "test")
                .is_ok()
                && verified_package.is_some()
                && version(&journal.candidate.version) >= version(snapshot.build.version)
            {
                snapshot.task_id = Some(uuid::Uuid::new_v4().to_string());
                snapshot.state = "readyToInstall".into();
                snapshot.version = Some(journal.candidate.version.clone());
                snapshot.progress = Some(100);
                snapshot.notes = Some(journal.candidate.notes.clone());
                if journal.stage != "ready" {
                    snapshot.error = Some("上次更新未完成，可以重新安装。".into());
                    snapshot.failure_stage = Some(journal.stage);
                }
                candidate = Some(journal.candidate);
            } else if journal.candidate.version == snapshot.build.version {
                snapshot.state = "recovery".into();
                snapshot.recovery = true;
                snapshot.failure_stage = Some("componentDeployment".into());
                snapshot.error = Some("输入组件维护尚未完成，请重新运行安装包恢复。".into());
            }
        }
        Ok(Self {
            inner: Mutex::new(Inner {
                snapshot,
                candidate,
                last_check,
            }),
            root,
            requests,
            key,
            feed,
        })
    }
    fn snapshot(&self) -> Result<Snapshot, Error> {
        Ok(self
            .inner
            .lock()
            .map_err(|_| Error::new("unavailable"))?
            .snapshot
            .clone())
    }
    fn publish(&self, app: &tauri::AppHandle, id: &str, edit: impl FnOnce(&mut Snapshot)) -> bool {
        if let Some(snapshot) = self.transition(id, edit) {
            let _ = app.emit_to("main", "update:snapshot", snapshot);
            true
        } else {
            false
        }
    }
    fn transition(&self, id: &str, edit: impl FnOnce(&mut Snapshot)) -> Option<Snapshot> {
        let Ok(mut inner) = self.inner.lock() else {
            return None;
        };
        if inner.snapshot.task_id.as_deref() != Some(id) {
            return None;
        }
        edit(&mut inner.snapshot);
        inner.snapshot.revision += 1;
        let snapshot = inner.snapshot.clone();
        Some(snapshot)
    }
    fn journal(&self, candidate: &Candidate, stage: &str) -> Result<(), Error> {
        let bytes = serde_json::to_vec(&Journal {
            schema_version: 1,
            candidate: candidate.clone(),
            stage: stage.into(),
        })
        .map_err(|_| Error::new("journalWrite"))?;
        if bytes.len() > MAX_METADATA {
            return Err(Error::new("tooLarge"));
        }
        atomic_write(&self.root.join("transaction.json"), &bytes)
    }
    fn begin_check(&self, manual: bool) -> Result<Option<String>, Error> {
        let mut inner = self.inner.lock().map_err(|_| Error::new("unavailable"))?;
        if !inner.snapshot.build.updates_enabled {
            return Err(Error::new("disabled"));
        }
        if !["idle", "failed"].contains(&inner.snapshot.state.as_str()) {
            return Err(Error::new("busy"));
        }
        if !manual && now().saturating_sub(inner.last_check) < 86400 {
            return Ok(None);
        }
        let id = uuid::Uuid::new_v4().to_string();
        inner.last_check = now();
        atomic_write(
            &self.root.join("last-check"),
            inner.last_check.to_string().as_bytes(),
        )?;
        inner.snapshot.task_id = Some(id.clone());
        inner.snapshot.state = "checking".into();
        inner.snapshot.revision += 1;
        inner.snapshot.error = None;
        inner.snapshot.failure_stage = None;
        inner.snapshot.progress = None;
        Ok(Some(id))
    }
    async fn transfer(&self, app: &tauri::AppHandle, id: &str) -> Result<(), Error> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::custom(|attempt| {
                if attempt.previous().len() >= 10 || attempt.url().scheme() != "https" {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            }))
            .user_agent("Sayweave-updater")
            .build()
            .map_err(|_| Error::new("network"))?;
        let mut guard = self.requests.register(id)?;
        let metadata = guard
            .run(15, fetch(&client, &self.feed, MAX_METADATA, None))
            .await?;
        let manifest: Manifest =
            serde_json::from_slice(&metadata).map_err(|_| Error::new("invalidMetadata"))?;
        if version(&manifest.version).is_none() {
            return Err(Error::new("invalidMetadata"));
        }
        if version(&manifest.version) <= version(build_info::current().version) {
            self.publish(app, id, |s| {
                s.state = "idle".into();
                s.version = None;
            });
            return Ok(());
        }
        let platform = manifest
            .platforms
            .get("windows-x86_64")
            .ok_or(Error::new("invalidMetadata"))?;
        let candidate = Candidate {
            version: manifest.version,
            signature: platform.signature.clone(),
            url: platform.url.clone(),
            installer_size: platform.installer_size,
            installer_sha512: platform.installer_sha512.clone(),
            notes: manifest.notes,
        };
        validate_candidate(&candidate, build_info::current().channel == "test")?;
        if !self.publish(app, id, |s| {
            s.state = "downloading".into();
            s.version = Some(candidate.version.clone());
            s.progress = Some(0);
            s.notes = Some(candidate.notes.clone());
        }) {
            return Err(Error::new("cancelled"));
        }
        let progress = |received: usize| {
            self.publish(app, id, |s| {
                s.progress =
                    Some(((received as u64 * 99) / (candidate.installer_size as u64)).min(99) as u8)
            });
        };
        let bytes = guard
            .run(
                300,
                fetch(&client, &candidate.url, MAX_PACKAGE, Some(&progress)),
            )
            .await?;
        drop(guard);
        if !self.publish(app, id, |s| {
            s.state = "verifying".into();
            s.progress = None;
        }) {
            return Err(Error::new("cancelled"));
        }
        let (payload, verified) = tokio::task::spawn_blocking({
            let candidate = candidate.clone();
            let key = self.key.clone();
            move || {
                let result = verify_package(&bytes, &candidate, &key);
                (bytes, result)
            }
        })
        .await
        .map_err(|_| Error::new("verification"))?;
        verified?;
        // Hold identity lock across publication of the candidate; cancellation cannot promote it.
        let mut inner = self.inner.lock().map_err(|_| Error::new("unavailable"))?;
        if inner.snapshot.task_id.as_deref() != Some(id) || inner.snapshot.state != "verifying" {
            return Err(Error::new("cancelled"));
        }
        atomic_write(&self.root.join("candidate.exe"), &payload)?;
        self.journal(&candidate, "ready")?;
        inner.candidate = Some(candidate);
        drop(inner);
        self.publish(app, id, |s| {
            s.state = "readyToInstall".into();
            s.progress = Some(100);
        });
        Ok(())
    }
    fn cancel(&self, app: &tauri::AppHandle) -> Result<(), Error> {
        let snapshot = self.invalidate_transfer()?;
        let _ = app.emit_to("main", "update:snapshot", snapshot);
        Ok(())
    }
    fn invalidate_transfer(&self) -> Result<Snapshot, Error> {
        let mut inner = self.inner.lock().map_err(|_| Error::new("unavailable"))?;
        if !["checking", "downloading", "verifying"].contains(&inner.snapshot.state.as_str()) {
            return Err(Error::new("busy"));
        }
        if let Some(id) = inner.snapshot.task_id.take() {
            self.requests.cancel(&id)?;
        }
        inner.snapshot.state = "idle".into();
        inner.snapshot.revision += 1;
        inner.snapshot.progress = None;
        inner.snapshot.error = None;
        let snapshot = inner.snapshot.clone();
        Ok(snapshot)
    }
    pub fn foreground(&self, app: &tauri::AppHandle) {
        let _ = self.cancel(app);
    }
    async fn install(&self, app: &tauri::AppHandle, id: &str, revision: u64) -> Result<(), Error> {
        let candidate = self.take_candidate(id, revision)?;
        self.publish(app, id, |s| {
            s.blocked_reason = None;
            s.error = None;
        });
        self.install_candidate(app, id, candidate).await
    }
    fn take_candidate(&self, id: &str, revision: u64) -> Result<Candidate, Error> {
        let mut inner = self.inner.lock().map_err(|_| Error::new("unavailable"))?;
        if inner.snapshot.task_id.as_deref() != Some(id)
            || inner.snapshot.revision != revision
            || inner.snapshot.state != "readyToInstall"
        {
            return Err(Error::new("staleUpdate"));
        }
        let candidate = inner
            .candidate
            .clone()
            .ok_or(Error::new("missingPackage"))?;
        inner.snapshot.state = "preparing".into();
        inner.snapshot.revision += 1;
        Ok(candidate)
    }
    async fn install_candidate(
        &self,
        app: &tauri::AppHandle,
        id: &str,
        candidate: Candidate,
    ) -> Result<(), Error> {
        let lifecycle = app.state::<crate::lifecycle::Lifecycle>();
        if lifecycle.fence(|| foreground_busy(app)).is_err() {
            self.publish(app, id, |s| {
                s.state = "readyToInstall".into();
                s.blocked_reason = Some("请先结束当前口述或输入操作。".into());
            });
            return Ok(());
        }
        let watermark = crate::archive::watermark();
        let prepare = tokio::time::timeout(Duration::from_secs(10), async {
            lifecycle.drain().await.map_err(Error::new)?;
            crate::archive::drain_since(watermark)
                .await
                .map_err(Error::new)?;
            if foreground_busy(app) {
                return Err(Error::new("busy"));
            }
            let bytes = bounded_read(&self.root.join("candidate.exe"), MAX_PACKAGE)?;
            verify_package(&bytes, &candidate, &self.key)?;
            self.journal(&candidate, "preparing")
        })
        .await
        .unwrap_or_else(|_| Err(Error::new("prepareTimeout")));
        if let Err(error) = prepare {
            lifecycle.release();
            let invalid_package = ["cacheRead", "tooLarge", "packageIdentity", "signature"]
                .contains(&error.code.as_str());
            if invalid_package {
                self.discard_candidate(id);
            }
            self.publish(app, id, |s| {
                s.state = if invalid_package {
                    "failed"
                } else {
                    "readyToInstall"
                }
                .into();
                s.error = Some(
                    if invalid_package {
                        "缓存更新包无法验证，请重新下载。"
                    } else {
                        "准备更新失败，当前程序可以继续使用。"
                    }
                    .into(),
                );
                s.failure_stage = Some(error.code);
            });
            return Ok(());
        }
        if let Err(error) = self.journal(&candidate, "stopping") {
            lifecycle.release();
            self.publish(app, id, |s| {
                s.state = "readyToInstall".into();
                s.error = Some("无法保存更新状态，请重试。".into());
                s.failure_stage = Some(error.code);
            });
            return Ok(());
        }
        self.publish(app, id, |s| s.state = "stopping".into());
        let stopped = tokio::time::timeout(
            Duration::from_secs(5),
            crate::application_exit::stop(app, watermark),
        )
        .await;
        if !matches!(stopped, Ok(Ok(()))) {
            self.recover(app, id, "stop", "停止服务未完成，请重启当前版本后再试。");
            return Ok(());
        }
        let launch = (|| {
            let bytes = bounded_read(&self.root.join("candidate.exe"), MAX_PACKAGE)?;
            verify_package(&bytes, &candidate, &self.key)?;
            self.journal(&candidate, "launching")?;
            launch_installer(&self.root.join("candidate.exe"))
        })();
        if launch.is_err() {
            self.recover(app, id, "launch", "安装器未能启动，请重启当前版本后再试。");
            return Ok(());
        }
        // A launched installer is not yet a completed update; the next process confirms its version.
        let _ = self.journal(&candidate, "launched");
        self.publish(app, id, |s| s.state = "installing".into());
        log::logger().flush();
        app.exit(0);
        Ok(())
    }
    fn discard_candidate(&self, id: &str) {
        if let Ok(mut inner) = self.inner.lock()
            && inner.snapshot.task_id.as_deref() == Some(id)
        {
            inner.candidate = None;
            let _ = fs::remove_file(self.root.join("transaction.json"));
            let _ = fs::remove_file(self.root.join("candidate.exe"));
        }
    }
    fn recover(&self, app: &tauri::AppHandle, id: &str, stage: &str, message: &str) {
        self.publish(app, id, |s| {
            s.state = "recovery".into();
            s.recovery = true;
            s.failure_stage = Some(stage.into());
            s.error = Some(message.into());
        });
    }
}
fn foreground_busy(app: &tauri::AppHandle) -> bool {
    app.state::<crate::dictation::DictationCoordinator>().busy()
        || app.state::<crate::speech::SpeechService>().is_active()
        || app
            .state::<crate::windows_input::WindowsInputService>()
            .is_busy()
        || app
            .state::<crate::shortcuts::ShortcutService>()
            .is_capturing()
        || crate::read(app).is_ok_and(|v| v.demo_busy())
}
async fn fetch(
    client: &reqwest::Client,
    url: &str,
    limit: usize,
    progress: Option<&(dyn Fn(usize) + Sync)>,
) -> Result<Vec<u8>, Error> {
    let mut response = client
        .get(url)
        .send()
        .await
        .map_err(|_| Error::new("network"))?
        .error_for_status()
        .map_err(|_| Error::new("network"))?;
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(Error::new("tooLarge"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error::new("network"))? {
        if bytes.len() + chunk.len() > limit {
            return Err(Error::new("tooLarge"));
        }
        bytes.extend_from_slice(&chunk);
        if let Some(progress) = progress {
            progress(bytes.len());
        }
    }
    Ok(bytes)
}
fn verify_package(bytes: &[u8], candidate: &Candidate, key: &str) -> Result<(), Error> {
    if bytes.len() != candidate.installer_size
        || STANDARD.encode(Sha512::digest(bytes)) != candidate.installer_sha512
    {
        return Err(Error::new("packageIdentity"));
    }
    crate::update_signature::verify(bytes, key, &candidate.signature, &candidate.version)
        .map_err(|_| Error::new("signature"))
}
fn launch_installer(path: &Path) -> Result<(), Error> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new(path)
            .args(["/P", "/UPDATE", "/R"])
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|_| Error::new("launch"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(Error::new("unsupported"))
    }
}
fn authorize(window: &WebviewWindow) -> Result<(), Error> {
    if window.label() == "main" {
        Ok(())
    } else {
        Err(Error::new("forbidden"))
    }
}
#[tauri::command]
pub fn update_snapshot(
    window: WebviewWindow,
    service: State<'_, UpdateService>,
) -> Result<Snapshot, Error> {
    authorize(&window)?;
    service.snapshot()
}
#[tauri::command]
pub fn update_check(
    app: tauri::AppHandle,
    window: WebviewWindow,
    service: State<'_, UpdateService>,
    manual: bool,
) -> Result<Snapshot, Error> {
    authorize(&window)?;
    if foreground_busy(&app) {
        return Err(Error::new("busy"));
    }
    if let Some(id) = service.begin_check(manual)? {
        service.publish(&app, &id, |_| {});
        dispatch(app.clone(), id);
    }
    service.snapshot()
}
fn dispatch(app: tauri::AppHandle, id: String) {
    tauri::async_runtime::spawn(async move {
        let service = app.state::<UpdateService>();
        if let Err(error) = service.transfer(&app, &id).await {
            service.publish(&app, &id, |s| {
                s.failure_stage = Some(s.state.clone());
                s.state = "failed".into();
                s.error = Some(
                    match error.code.as_str() {
                        "signature" | "packageIdentity" => "更新包验证失败，请重试。",
                        _ => "暂时无法获取更新，请稍后重试。",
                    }
                    .into(),
                );
                s.progress = None;
            });
        }
    });
}
pub fn schedule(app: tauri::AppHandle) {
    if !build_info::current().updates_enabled {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(20)).await;
        loop {
            if !foreground_busy(&app) {
                let service = app.state::<UpdateService>();
                if let Ok(Some(id)) = service.begin_check(false) {
                    service.publish(&app, &id, |_| {});
                    dispatch(app.clone(), id);
                }
            }
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
}
#[tauri::command]
pub fn update_cancel(
    app: tauri::AppHandle,
    window: WebviewWindow,
    service: State<'_, UpdateService>,
) -> Result<Snapshot, Error> {
    authorize(&window)?;
    service.cancel(&app)?;
    service.snapshot()
}
#[tauri::command]
pub async fn update_install(
    app: tauri::AppHandle,
    window: WebviewWindow,
    service: State<'_, UpdateService>,
    task_id: String,
    revision: u64,
) -> Result<Snapshot, Error> {
    authorize(&window)?;
    service.install(&app, &task_id, revision).await?;
    service.snapshot()
}
#[tauri::command]
pub fn update_restart(
    app: tauri::AppHandle,
    window: WebviewWindow,
    service: State<'_, UpdateService>,
) -> Result<(), Error> {
    authorize(&window)?;
    if !service.snapshot()?.recovery {
        return Err(Error::new("invalidState"));
    }
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;
    fn service() -> (tempfile::TempDir, UpdateService) {
        let root = tempfile::tempdir().unwrap();
        let service =
            UpdateService::new(root.path(), Arc::new(RequestCoordinator::default())).unwrap();
        service.inner.lock().unwrap().snapshot.build.updates_enabled = true;
        (root, service)
    }
    #[test]
    fn cancellation_rejects_late_progress_and_errors_without_touching_the_next_task() {
        let (_root, service) = service();
        let old = service.begin_check(true).unwrap().unwrap();
        assert!(service.begin_check(true).is_err());
        service.invalidate_transfer().unwrap();
        let new = service.begin_check(true).unwrap().unwrap();
        assert_ne!(new, old);
        assert!(
            service
                .transition(&old, |s| s.state = "failed".into())
                .is_none()
        );
        assert_eq!(service.snapshot().unwrap().task_id, Some(new));
        assert_eq!(service.snapshot().unwrap().state, "checking");
    }
    #[test]
    fn exact_install_admission_is_single_use_and_blocks_check_and_cancel() {
        let (_root, service) = service();
        let id = service.begin_check(true).unwrap().unwrap();
        let candidate = Candidate {
            version: "0.1.7".into(),
            signature: String::new(),
            url: String::new(),
            installer_size: 1,
            installer_sha512: String::new(),
            notes: String::new(),
        };
        service.inner.lock().unwrap().candidate = Some(candidate);
        service
            .transition(&id, |s| s.state = "readyToInstall".into())
            .unwrap();
        let revision = service.snapshot().unwrap().revision;
        assert_eq!(
            service.take_candidate(&id, revision - 1).unwrap_err().code,
            "staleUpdate"
        );
        assert_eq!(
            service
                .take_candidate("another-task", revision)
                .unwrap_err()
                .code,
            "staleUpdate"
        );
        assert!(service.take_candidate(&id, revision).is_ok());
        assert!(service.take_candidate(&id, revision).is_err());
        assert!(service.begin_check(true).is_err());
        assert!(service.invalidate_transfer().is_err());
    }
    #[test]
    fn automatic_check_limit_is_persisted_but_manual_check_remains_explicit() {
        let (root, service) = service();
        service.begin_check(true).unwrap();
        service.invalidate_transfer().unwrap();
        assert!(service.begin_check(false).unwrap().is_none());
        let restored =
            UpdateService::new(root.path(), Arc::new(RequestCoordinator::default())).unwrap();
        restored
            .inner
            .lock()
            .unwrap()
            .snapshot
            .build
            .updates_enabled = true;
        assert!(restored.begin_check(false).unwrap().is_none());
        assert!(restored.begin_check(true).unwrap().is_some());
    }
    #[test]
    fn rejects_external_release_and_invalid_version() {
        let mut candidate = Candidate {
            version: "0.1.7".into(),
            url: "https://evil.example/releases/download/v0.1.7/Sayweave_0.1.7_x64-setup.exe"
                .into(),
            signature: "".into(),
            installer_size: 1,
            installer_sha512: STANDARD.encode([0; 64]),
            notes: "".into(),
        };
        assert!(validate_candidate(&candidate, false).is_err());
        candidate.url="https://github.com/qinghui316/Sayweave/releases/download/v0.1.7/Sayweave_0.1.7_x64-setup.exe".into();
        assert!(validate_candidate(&candidate, false).is_ok());
        candidate.version = "00.1.7".into();
        assert!(validate_candidate(&candidate, false).is_err());
    }
    #[test]
    fn atomic_record_replaces_and_bounded_read_rejects_overflow() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("transaction.json");
        atomic_write(&path, b"old").unwrap();
        atomic_write(&path, b"new value").unwrap();
        assert_eq!(bounded_read(&path, 9).unwrap(), b"new value");
        assert!(bounded_read(&path, 8).is_err());
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
    #[test]
    fn inaccessible_update_cache_does_not_prevent_native_application_setup() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("update-cache"), b"blocked directory").unwrap();
        let service =
            UpdateService::new(root.path(), Arc::new(RequestCoordinator::default())).unwrap();
        assert_eq!(service.snapshot().unwrap().state, "failed");
        assert_eq!(
            service.snapshot().unwrap().failure_stage.as_deref(),
            Some("cacheWrite")
        );
    }
}
