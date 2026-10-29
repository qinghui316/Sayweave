#[cfg(windows)]
mod backend;
pub use backend::TaskStamp;
// The owned actor can observe real manual edits through the same hook owner as
// the desktop. The receiver retains trigger events for the fixture's lifetime.
#[cfg(all(test, windows))]
pub(crate) struct FixturePhysicalInput {
    backend: backend::Backend,
    _events: tokio::sync::mpsc::Receiver<backend::Event>,
}
#[cfg(all(test, windows))]
impl FixturePhysicalInput {
    pub(crate) fn start(task: &str) -> Result<Self, crate::history::types::Error> {
        let (sender, receiver) = tokio::sync::mpsc::channel(16);
        let backend = backend::Backend::start(sender)?;
        backend.task_active(Some(task.into()));
        Ok(Self {
            backend,
            _events: receiver,
        })
    }
}
#[cfg(all(test, windows))]
impl Drop for FixturePhysicalInput {
    fn drop(&mut self) {
        self.backend.stop();
    }
}
pub mod commands;
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
// Register only new keys; a failed explicit change rolls back every addition.
fn register_additions(
    new: &[Shortcut],
    previous: &[Shortcut],
    optional: Option<Shortcut>,
    mut register: impl FnMut(Shortcut) -> Result<(), ()>,
    mut unregister: impl FnMut(Shortcut),
) -> Result<(Vec<Shortcut>, bool), Error> {
    let mut added = Vec::new();
    let mut translation_conflict = false;
    for key in new.iter().filter(|key| !previous.contains(key)) {
        if register(*key).is_err() {
            if Some(*key) == optional {
                translation_conflict = true;
                continue;
            }
            for key in added {
                unregister(key);
            }
            return Err(Error::new("shortcutConflict"));
        }
        added.push(*key);
    }
    Ok((added, translation_conflict))
}
pub struct ShortcutService {
    data: DataWorker,
    state: Mutex<Status>,
    gate: tokio::sync::Mutex<()>,
    registration: Mutex<()>,
    registered: Mutex<Vec<Shortcut>>,
    capture_registrations: Mutex<Option<Vec<Shortcut>>>,
    stopping: AtomicBool,
    changing: AtomicBool,
    capturing: AtomicBool,
    backend: Mutex<Option<backend::Backend>>,
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
            registered: Mutex::new(Vec::new()),
            capture_registrations: Mutex::new(None),
            stopping: AtomicBool::new(false),
            changing: AtomicBool::new(false),
            capturing: AtomicBool::new(false),
            backend: Mutex::new(None),
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
        let config = config.validate()?;
        let old = self.status()?.active;
        let registered = |config: &Config| -> Result<Vec<Shortcut>, Error> {
            [&config.binding, &config.translation_binding]
                .into_iter()
                .filter(|value| {
                    !Binding::parse(value).is_ok_and(|b| b.single_modifier() || b.modifier_pair())
                })
                .map(|value| {
                    value
                        .parse::<Shortcut>()
                        .map_err(|_| Error::new("invalidShortcut"))
                })
                .collect()
        };
        let mut new = registered(&config)?;
        let previous = self
            .registered
            .lock()
            .map_err(|_| Error::new("unavailable"))?
            .clone();
        // On initial registration a newly introduced translation shortcut may
        // conflict with another program. Keep the original dictation available.
        // Explicit setting changes remain atomic and preserve the prior config.
        let optional = old
            .is_none()
            .then(|| config.translation_binding.parse::<Shortcut>().ok())
            .flatten();
        let (added, translation_conflict) = register_additions(
            &new,
            &previous,
            optional,
            |key| app.global_shortcut().register(key).map_err(|_| ()),
            |key| {
                let _ = app.global_shortcut().unregister(key);
            },
        )?;
        if translation_conflict {
            new.retain(|key| Some(*key) != optional);
        }
        let mut removed = Vec::new();
        for shortcut in previous.iter().filter(|key| !new.contains(key)) {
            if app.global_shortcut().unregister(*shortcut).is_err() {
                for key in removed {
                    let _ = app.global_shortcut().register(key);
                }
                for key in added {
                    let _ = app.global_shortcut().unregister(key);
                }
                return Err(Error::new("shortcutUnavailable"));
            }
            removed.push(*shortcut);
        }
        self.backend
            .lock()
            .map_err(|_| Error::new("unavailable"))?
            .as_ref()
            .ok_or_else(|| Error::new("shortcutUnavailable"))?
            .configure(config.clone(), false, !translation_conflict);
        let mut state = self.state.lock().map_err(|_| Error::new("unavailable"))?;
        *self
            .registered
            .lock()
            .map_err(|_| Error::new("unavailable"))? = new;
        state.active = Some(config);
        state.pending = false;
        state.error = translation_conflict.then(|| "translationShortcutConflict".into());
        Ok(())
    }
    pub async fn initialize(&self, app: &tauri::AppHandle) -> Result<(), Error> {
        let _gate = self.gate.lock().await;
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::new("stopping"));
        }
        let DbOutput::Config(config) = self.db(DbOperation::Load).await?;
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
                    backend::Event::OutsidePointer(task) => {
                        if service.task_is_current(&task)
                            && let Some(id) = task.id
                        {
                            let _ = app
                                .state::<crate::dictation::DictationCoordinator>()
                                .translation_menu(&app, &id, false);
                        }
                    }

                    backend::Event::Captured(binding, generation) => {
                        service
                            .finish_capture(&app, None, Some(generation), Some(binding))
                            .await
                            .ok();
                    }
                    backend::Event::Cancel(task, generation) => {
                        if !service.current_event(generation) {
                            continue;
                        }
                        if service.is_capturing() {
                            service
                                .finish_capture(&app, None, Some(generation), None)
                                .await
                                .ok();
                        } else {
                            app.state::<crate::dictation::DictationCoordinator>()
                                .cancel_bound(task.as_deref())
                                .ok();
                        }
                    }
                    backend::Event::Interrupt(task, generation) => {
                        service
                            .finish_capture(&app, None, Some(generation), None)
                            .await
                            .ok();
                        app.state::<crate::dictation::DictationCoordinator>()
                            .interrupt_bound(task.as_deref());
                    }
                    backend::Event::Key {
                        epoch,
                        generation,
                        revision,
                        purpose,
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
                                crate::dictation::ShortcutInput {
                                    epoch,
                                    pressed,
                                    mode: config.mode,
                                    purpose,
                                    anchor,
                                    task,
                                },
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
    // This transaction runs off the UI thread. No status/Bridge lock crosses
    // a plugin registration call, which may dispatch synchronously to the UI.
    fn suspend_capture_registration(&self, app: &tauri::AppHandle) -> Result<(), Error> {
        let _registration = self
            .registration
            .lock()
            .map_err(|_| Error::new("unavailable"))?;
        let previous = self
            .registered
            .lock()
            .map_err(|_| Error::new("unavailable"))?
            .clone();
        *self
            .capture_registrations
            .lock()
            .map_err(|_| Error::new("unavailable"))? = Some(previous.clone());
        for key in previous {
            if app.global_shortcut().unregister(key).is_err() {
                return Err(Error::new("shortcutUnavailable"));
            }
            self.registered
                .lock()
                .map_err(|_| Error::new("unavailable"))?
                .retain(|k| *k != key);
        }
        Ok(())
    }
    fn restore_capture_registration(&self, app: &tauri::AppHandle) -> Result<(), Error> {
        let _registration = self
            .registration
            .lock()
            .map_err(|_| Error::new("unavailable"))?;
        let previous = self
            .capture_registrations
            .lock()
            .map_err(|_| Error::new("unavailable"))?
            .take();
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::new("stopping"));
        }
        let mut failed = false;
        for key in previous.unwrap_or_default() {
            if self
                .registered
                .lock()
                .map_err(|_| Error::new("unavailable"))?
                .contains(&key)
            {
                continue;
            }
            if app.global_shortcut().register(key).is_err() {
                failed = true;
            } else {
                self.registered
                    .lock()
                    .map_err(|_| Error::new("unavailable"))?
                    .push(key);
            }
        }
        if failed {
            Err(Error::new("shortcutUnavailable"))
        } else {
            Ok(())
        }
    }
    pub async fn capture_owned(
        &self,
        app: &tauri::AppHandle,
        begin: bool,
        token: Option<String>,
    ) -> Result<Status, Error> {
        if let Some(token) = &token {
            uuid::Uuid::parse_str(token).map_err(|_| Error::new("invalidShortcut"))?;
        }
        if !begin {
            return self.finish_capture(app, token, None, None).await;
        }
        let _gate = self.gate.lock().await;
        if self.stopping.load(Ordering::Acquire) {
            return Err(Error::new("stopping"));
        }
        if self.changing.load(Ordering::Acquire) {
            return Err(Error::new("busy"));
        }
        let current = self.status()?;
        if current.capturing {
            return if token.is_none() || current.capture_token == token {
                Ok(current)
            } else {
                Err(Error::new("busy"))
            };
        }
        app.state::<crate::dictation::DictationCoordinator>()
            .configure_capture(true, || {
                if self.stopping.load(Ordering::Acquire) {
                    return Err(Error::new("stopping"));
                }
                let mut s = self.state.lock().map_err(|_| Error::new("unavailable"))?;
                s.capturing = true;
                s.capture_token = Some(token.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()));
                s.captured_binding = None;
                self.capturing.store(true, Ordering::Release);
                if let Some(b) = self.backend.lock().ok().and_then(|b| b.clone()) {
                    b.configure(
                        s.active.clone().unwrap_or_else(|| s.saved.clone()),
                        true,
                        s.error.as_deref() != Some("translationShortcutConflict"),
                    );
                }
                Ok(())
            })?;
        if let Err(e) = self.suspend_capture_registration(app) {
            let restored = self.restore_capture_registration(app);
            self.complete_capture(None, restored.err().or(Some(e.clone())))?;
            self.publish(app);
            return Err(e);
        }
        self.publish(app);
        self.status()
    }
    pub async fn finish_capture(
        &self,
        app: &tauri::AppHandle,
        token: Option<String>,
        generation: Option<u64>,
        binding: Option<String>,
    ) -> Result<Status, Error> {
        let _gate = self.gate.lock().await;
        let current = self.status()?;
        if !current.capturing
            || (token.is_none() && generation.is_none())
            || token
                .as_ref()
                .is_some_and(|t| current.capture_token.as_ref() != Some(t))
            || generation.is_some_and(|g| !self.current_event(g))
        {
            return Ok(current);
        }
        // Keep rejected/reserved candidates visible in the existing save editor.
        // Recording a candidate never makes it an active binding.
        if binding
            .as_ref()
            .is_some_and(|b| b.is_empty() || b.len() > 128)
        {
            return Err(Error::new("invalidShortcut"));
        }
        let restored = self.restore_capture_registration(app);
        // Registration may pump UI messages. A focus change or native
        // invalidation during restoration cannot publish the earlier candidate.
        let still_current = generation.is_none_or(|g| self.current_event(g))
            && (binding.is_none()
                || app
                    .get_webview_window("main")
                    .is_some_and(|w| w.is_focused().unwrap_or(false)));
        self.complete_capture(if still_current { binding } else { None }, restored.err())?;
        self.publish(app);
        self.status()
    }
    fn complete_capture(&self, binding: Option<String>, error: Option<Error>) -> Result<(), Error> {
        let mut s = self.state.lock().map_err(|_| Error::new("unavailable"))?;
        let stopping = self.stopping.load(Ordering::Acquire);
        s.capturing = false;
        s.captured_binding = if error.is_none() && !stopping {
            binding
        } else {
            None
        };
        if let Some(error) = error {
            s.active = None;
            s.error = Some(error.code);
        }
        if !stopping && let Some(b) = self.backend.lock().ok().and_then(|b| b.clone()) {
            b.configure(
                s.active.clone().unwrap_or_else(|| s.saved.clone()),
                false,
                s.active.is_some() && s.error.as_deref() != Some("translationShortcutConflict"),
            );
        }
        self.capturing.store(false, Ordering::Release);
        Ok(())
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
    pub fn plugin_event(&self, shortcut: &Shortcut, event: ShortcutState) {
        if !self.triggers_paused()
            && !self.stopping.load(Ordering::Acquire)
            && event == ShortcutState::Pressed
            && let Ok(s) = self.status()
            && let Some(active) = s.active
            && let Ok(b) = self.backend.lock()
            && let Some(b) = b.as_ref()
        {
            for binding in [&active.binding, &active.translation_binding] {
                if binding
                    .parse::<Shortcut>()
                    .is_ok_and(|key| key == *shortcut)
                {
                    b.combo_pressed(binding);
                }
            }
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
    #[cfg(feature = "desktop-acceptance")]
    pub async fn acceptance_hold_drain(&self, ready: tokio::sync::oneshot::Sender<()>) {
        let _guard = self.gate.lock().await;
        let _ = ready.send(());
        tokio::time::sleep(std::time::Duration::from_secs(8)).await;
    }
    pub async fn drain(&self, app: &tauri::AppHandle) -> Result<(), Error> {
        let _gate = self.gate.lock().await;
        // Plugin registration dispatches to the UI thread; never wait for this lock in a UI callback.
        let _registration = self.registration.lock().ok();
        app.global_shortcut()
            .unregister_all()
            .map_err(|_| Error::new("shortcutRegistration"))
    }
}
