#[cfg(windows)]
mod backend;
pub use backend::TaskStamp;
pub mod commands;
mod repository;
#[cfg(test)]
mod tests;
pub mod types;
use crate::{
    data::{DataWorker, Operation, Output},
    history::types::Error,
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};
use tauri::{Emitter, Manager};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use types::*;
pub struct ShortcutService {
    data: DataWorker,
    state: Mutex<Status>,
    gate: tokio::sync::Mutex<()>,
    registration: Mutex<()>,
    stopping: AtomicBool,
    changing: AtomicBool,
    capturing: AtomicBool,
    backend: Mutex<Option<backend::Backend>>,
    capture_generation: std::sync::atomic::AtomicU64,
}
impl ShortcutService {
    pub fn task_is_current(&self, task: &TaskStamp) -> bool {
        self.backend
            .lock()
            .ok()
            .and_then(|b| b.as_ref().map(|b| b.task_is_current(task)))
            .unwrap_or(false)
    }
    pub fn new(data: DataWorker) -> Self {
        Self {
            data,
            state: Mutex::new(Status {
                sequence: 0,
                capture_token: None,
                saved: Config::default(),
                active: None,
                capturing: false,
                pending: false,
                error: Some("preparing".into()),
                captured_binding: None,
            }),
            gate: tokio::sync::Mutex::new(()),
            registration: Mutex::new(()),
            stopping: AtomicBool::new(false),
            changing: AtomicBool::new(false),
            capturing: AtomicBool::new(false),
            backend: Mutex::new(None),
            capture_generation: std::sync::atomic::AtomicU64::new(0),
        }
    }
    pub fn block_triggers(&self, value: bool) {
        self.changing.store(value, Ordering::Release);
    }
    pub fn triggers_paused(&self) -> bool {
        self.changing.load(Ordering::Acquire) || self.capturing.load(Ordering::Acquire)
    }
    pub fn is_capturing(&self) -> bool {
        self.capturing.load(Ordering::Acquire)
    }
    pub fn target_tracking_ready(&self) -> bool {
        !self.stopping.load(Ordering::Acquire)
            && self
                .backend
                .lock()
                .ok()
                .and_then(|b| b.as_ref().and_then(|b| b.generation()))
                .is_some()
    }
    fn current_event(&self, generation: u64) -> bool {
        self.backend
            .lock()
            .ok()
            .and_then(|b| b.as_ref().and_then(|b| b.generation()))
            == Some(generation)
    }
    pub fn status(&self) -> Result<Status, Error> {
        let mut state = self.state.lock().map_err(|_| Error::new("unavailable"))?;
        // Snapshot order includes command reads and event publications, under the same state lock.
        state.sequence = state
            .sequence
            .checked_add(1)
            .ok_or_else(|| Error::new("unavailable"))?;
        Ok(state.clone())
    }
    pub async fn db(&self, operation: DbOperation) -> Result<DbOutput, Error> {
        match self
            .data
            .request_admitted(Operation::Shortcuts(operation))
            .await?
        {
            Output::Shortcuts(value) => Ok(value),
            _ => Err(Error::new("unavailable")),
        }
    }
    fn publish(&self, app: &tauri::AppHandle) {
        if let Ok(s) = self.status() {
            let _ = app.emit_to("main", "shortcut:status", s);
        }
    }
    fn apply(&self, app: &tauri::AppHandle, config: Config) -> Result<(), Error> {
        let _registration = self
            .registration
            .lock()
            .map_err(|_| Error::new("unavailable"))?;
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::new("stopping"));
        }
        let binding = Binding::parse(&config.binding)?;
        let old = self.status()?.active;
        let new = if binding.single_modifier() {
            None
        } else {
            Some(
                config
                    .binding
                    .parse::<Shortcut>()
                    .map_err(|_| Error::new("invalidShortcut"))?,
            )
        };
        let old_shortcut = old
            .as_ref()
            .filter(|c| !Binding::parse(&c.binding).is_ok_and(|b| b.single_modifier()))
            .and_then(|c| c.binding.parse::<Shortcut>().ok());
        if old.as_ref().is_none_or(|c| c.binding != config.binding) {
            if let Some(new) = new {
                app.global_shortcut()
                    .register(new)
                    .map_err(|_| Error::new("shortcutConflict"))?;
            }
            if let Some(old) = old_shortcut
                && app.global_shortcut().unregister(old).is_err()
            {
                if let Some(new) = new {
                    let _ = app.global_shortcut().unregister(new);
                }
                return Err(Error::new("shortcutUnavailable"));
            }
        }
        self.backend
            .lock()
            .map_err(|_| Error::new("unavailable"))?
            .as_ref()
            .ok_or_else(|| Error::new("shortcutUnavailable"))?
            .configure(config.clone(), false);
        let mut state = self.state.lock().map_err(|_| Error::new("unavailable"))?;
        state.active = Some(config);
        state.pending = false;
        state.error = None;
        Ok(())
    }
    pub async fn initialize(&self, app: &tauri::AppHandle) -> Result<(), Error> {
        let _gate = self.gate.lock().await;
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::new("stopping"));
        }
        let config = match self.db(DbOperation::Load).await? {
            DbOutput::Config(c) => c,
            _ => return Err(Error::new("unavailable")),
        };
        self.db(DbOperation::Recover).await?;
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::new("stopping"));
        }
        let (tx, mut rx) = tokio::sync::mpsc::channel(64);
        *self.backend.lock().map_err(|_| Error::new("unavailable"))? =
            Some(backend::Backend::start(tx)?);
        self.state
            .lock()
            .map_err(|_| Error::new("unavailable"))?
            .saved = config.clone();
        if self.stopping.load(Ordering::Acquire) {
            if let Ok(b) = self.backend.lock()
                && let Some(b) = b.as_ref()
            {
                b.stop();
            }
            return Err(Error::new("stopping"));
        }
        let result = self.apply(app, config);
        if let Err(e) = &result {
            self.state.lock().unwrap().error = Some(e.code.clone());
        }
        self.publish(app);
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(event) = rx.recv().await {
                let service = app.state::<Self>();
                if service.stopping.load(Ordering::Acquire) {
                    break;
                }
                match event {
                    backend::Event::Captured(binding, generation) => {
                        if !service.current_event(generation) {
                            continue;
                        }
                        if let Ok(mut state) = service.state.lock() {
                            state.captured_binding = Some(binding);
                        }
                        service.capture(&app, false).ok();
                    }
                    backend::Event::Cancel(task, generation) => {
                        if !service.current_event(generation) {
                            continue;
                        }
                        if service.status().is_ok_and(|s| s.capturing) {
                            service.capture(&app, false).ok();
                        } else {
                            app.state::<crate::dictation::DictationCoordinator>()
                                .cancel_bound(task.as_deref())
                                .ok();
                        }
                    }
                    backend::Event::Interrupt(task) => {
                        if service.status().is_ok_and(|s| s.capturing) {
                            service.capture(&app, false).ok();
                        }
                        app.state::<crate::dictation::DictationCoordinator>()
                            .interrupt_bound(task.as_deref());
                    }
                    backend::Event::Key {
                        epoch,
                        generation,
                        revision,
                        pressed,
                        anchor,
                        task,
                    } => {
                        if !service.current_event(generation) {
                            continue;
                        }
                        let s = service.status();
                        if let Ok(s) = s
                            && let Some(config) = s.active
                            && config.revision == revision
                        {
                            app.state::<crate::dictation::DictationCoordinator>().key(
                                &app,
                                epoch,
                                pressed,
                                config.mode,
                                anchor,
                                task,
                            );
                        }
                    }
                }
                service.publish(&app);
            }
        });
        result
    }
    pub async fn save(&self, app: &tauri::AppHandle, config: Config) -> Result<Status, Error> {
        let _gate = self.gate.try_lock().map_err(|_| Error::new("busy"))?;
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::new("stopping"));
        }
        let config = config.validate()?;
        let old = self.status()?.saved;
        if old.revision != config.revision {
            return Err(Error::new("conflict"));
        }
        let pending = app
            .state::<crate::dictation::DictationCoordinator>()
            .pause_shortcuts(self)?;
        struct Resume<'a>(&'a ShortcutService);
        impl Drop for Resume<'_> {
            fn drop(&mut self) {
                self.0.block_triggers(false);
            }
        }
        let _resume = Resume(self);
        if !pending {
            self.apply(app, config.clone())?;
        }
        let saved = match self.db(DbOperation::Save(config)).await {
            Ok(DbOutput::Config(c)) => c,
            Ok(_) => return Err(Error::new("unavailable")),
            Err(e) => {
                if !pending
                    && !self.stopping.load(Ordering::Acquire)
                    && let Err(rollback) = self.apply(app, old)
                {
                    if let Ok(mut status) = self.state.lock() {
                        status.active = None;
                        status.error = Some(rollback.code);
                    }
                    self.publish(app);
                }
                return Err(e);
            }
        };
        {
            let mut s = self.state.lock().map_err(|_| Error::new("unavailable"))?;
            s.saved = saved.clone();
            s.captured_binding = None;
            s.pending = pending;
        }
        if self.stopping.load(Ordering::Acquire) {
            let mut s = self.state.lock().map_err(|_| Error::new("unavailable"))?;
            s.active = None;
            s.pending = false;
            return Err(Error::new("stopping"));
        }
        if !pending {
            self.apply(app, saved)?;
        }
        self.publish(app);
        self.status()
    }
    pub fn capture(&self, app: &tauri::AppHandle, begin: bool) -> Result<Status, Error> {
        self.capture_owned(app, begin, None)
    }
    pub fn capture_owned(
        &self,
        app: &tauri::AppHandle,
        begin: bool,
        token: Option<String>,
    ) -> Result<Status, Error> {
        if let Some(token) = &token {
            uuid::Uuid::parse_str(token).map_err(|_| Error::new("invalidShortcut"))?;
        }
        if begin && self.stopping.load(Ordering::Acquire) {
            return Err(Error::new("stopping"));
        }
        if begin && self.changing.load(Ordering::Acquire) {
            return Err(Error::new("busy"));
        }
        app.state::<crate::dictation::DictationCoordinator>()
            .configure_capture(begin, || {
                if begin
                    && (self.changing.load(Ordering::Acquire)
                        || self.stopping.load(Ordering::Acquire))
                {
                    return Err(Error::new("busy"));
                }
                let mut s = self.state.lock().map_err(|_| Error::new("unavailable"))?;
                if s.capturing == begin {
                    if begin && token.is_some() && s.capture_token != token {
                        return Err(Error::new("busy"));
                    }
                    return Ok(s.clone());
                }
                s.capturing = begin;
                self.capturing.store(begin, Ordering::Release);
                self.capture_generation.fetch_add(1, Ordering::AcqRel);
                if begin {
                    s.capture_token =
                        Some(token.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()));
                    s.captured_binding = None;
                }
                if let Some(b) = self.backend.lock().ok().and_then(|b| b.clone()) {
                    b.configure(s.active.clone().unwrap_or_else(|| s.saved.clone()), begin);
                }
                drop(s);
                self.publish(app);
                self.status()
            })
    }
    pub fn capture_generation(&self) -> u64 {
        self.capture_generation.load(Ordering::Acquire)
    }
    pub fn active_task(&self, value: Option<String>) {
        if let Ok(b) = self.backend.lock()
            && let Some(b) = b.as_ref()
        {
            b.task_active(value);
        }
    }
    pub async fn settle(&self, app: &tauri::AppHandle) {
        let _gate = self.gate.lock().await;
        if self.stopping.load(Ordering::Acquire) {
            return;
        }
        if let Ok(s) = self.status()
            && s.pending
        {
            if let Err(e) = self.apply(app, s.saved)
                && let Ok(mut s) = self.state.lock()
            {
                s.error = Some(e.code);
            }
            self.publish(app);
        }
    }
    pub fn plugin_event(&self, event: ShortcutState) {
        if event == ShortcutState::Pressed
            && let Ok(s) = self.status()
            && let Some(active) = s.active
            && let Ok(b) = self.backend.lock()
            && let Some(b) = b.as_ref()
        {
            b.combo_pressed(&active.binding);
        }
    }
    pub fn shutdown(&self, _app: &tauri::AppHandle) {
        self.stopping.store(true, Ordering::Release);
        self.block_triggers(true);
        if let Ok(b) = self.backend.lock()
            && let Some(b) = b.as_ref()
        {
            b.stop();
        }
    }
    pub async fn drain(&self, app: &tauri::AppHandle) {
        let _gate = self.gate.lock().await;
        // Plugin registration dispatches to the UI thread; never wait for this lock in a UI callback.
        let _registration = self.registration.lock().ok();
        let _ = app.global_shortcut().unregister_all();
    }
}
