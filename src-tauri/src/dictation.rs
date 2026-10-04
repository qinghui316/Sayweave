use crate::{
    coordinator::Phase,
    history::types::Error,
    shortcuts::{
        ShortcutService, TaskStamp,
        types::{DbOperation, Mode},
    },
    speech::{NativeStart, Snapshot as SpeechSnapshot, SpeechService},
    windows_input::{Anchor, Delivery, DeliveryGate, WindowsInputService},
};
use serde::Serialize;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use tauri::{Emitter, Manager};
use tokio::sync::{oneshot, watch};
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub sequence: u64,
    pub task_id: Option<String>,
    pub source: String,
    pub stage: String,
    pub text: String,
    pub delivery: Option<Delivery>,
    pub error: Option<String>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            sequence: 1,
            task_id: None,
            source: "trial".into(),
            stage: "idle".into(),
            text: String::new(),
            delivery: None,
            error: None,
        }
    }
}
#[derive(Clone)]
struct Active {
    id: String,
    source: String,
    epoch: u64,
    origin: Option<TaskStamp>,
    cancel: watch::Sender<u8>,
    gate: Arc<DeliveryGate>,
    original_overlay: bool,
    overlay_revision: u64,
}
struct Reservation {
    id: String,
    expected_task: Option<String>,
    cancel: watch::Sender<u8>,
    gate: Arc<DeliveryGate>,
}
pub struct DictationCoordinator {
    active: Mutex<Option<Active>>,
    snapshot: Mutex<Snapshot>,
    stopping: AtomicBool,
}
impl Default for DictationCoordinator {
    fn default() -> Self {
        Self {
            active: Mutex::new(None),
            snapshot: Mutex::new(Snapshot::default()),
            stopping: AtomicBool::new(false),
        }
    }
}
impl DictationCoordinator {
    pub async fn retry_displayed(
        &self,
        app: tauri::AppHandle,
        record: String,
        task: String,
    ) -> Result<SpeechSnapshot, Error> {
        let mut reserved = self.reserve(&app, "history", 0, None)?;
        reserved.expected_task = Some(task);
        self.start_reserved(app, "history", None, Some(record), None, reserved)
            .await
    }
    pub fn busy(&self) -> bool {
        self.active.lock().map(|s| s.is_some()).unwrap_or(true)
    }
    pub fn configure_capture<T>(
        &self,
        begin: bool,
        operation: impl FnOnce() -> Result<T, Error>,
    ) -> Result<T, Error> {
        let active = self.active.lock().map_err(|_| Error::new("unavailable"))?;
        if begin && active.is_some() {
            return Err(Error::new("busy"));
        }
        operation()
    }
    pub fn snapshot(&self) -> Result<Snapshot, Error> {
        self.snapshot
            .lock()
            .map(|s| s.clone())
            .map_err(|_| Error::new("unavailable"))
    }
    fn update(&self, app: &tauri::AppHandle, id: &str, change: impl FnOnce(&mut Snapshot)) {
        if let Ok(mut s) = self.snapshot.lock()
            && s.task_id.as_deref() == Some(id)
        {
            let old_stage = s.stage.clone();
            let old_text = s.text.is_empty();
            let old_recovery = s.delivery.as_ref().is_some_and(|d| d.restore_error);
            change(&mut s);
            let layout = old_stage != s.stage
                || old_text != s.text.is_empty()
                || old_recovery != s.delivery.as_ref().is_some_and(|d| d.restore_error);
            s.sequence += 1;
            let copy = s.clone();
            drop(s);
            if layout {
                crate::refresh_voice(app);
            }
            let _ = app.emit_to("main", "dictation:snapshot", &copy);
            let _ = app.emit_to("voice-overlay", "dictation:snapshot", &copy);
        }
    }
    pub fn key(
        &self,
        app: &tauri::AppHandle,
        epoch: u64,
        pressed: bool,
        mode: Mode,
        anchor: Option<Anchor>,
        task: TaskStamp,
    ) {
        let active = {
            let Ok(active) = self.active.lock() else {
                return;
            };
            if !key_matches_task(
                active.as_ref().map(|a| a.id.as_str()),
                active.as_ref().and_then(|a| a.origin.as_ref()),
                &task,
                app.state::<ShortcutService>().task_is_current(&task),
            ) {
                return;
            }
            active
                .as_ref()
                .map(|a| (a.id.clone(), a.epoch, a.source.clone()))
        };
        if active.is_some() {
            if let Some((id, current, source)) = active
                && source == "external"
                && (mode == Mode::Toggle && pressed
                    || mode == Mode::Hold && !pressed && epoch == current)
            {
                let phase = app
                    .state::<SpeechService>()
                    .snapshot()
                    .ok()
                    .map(|s| s.phase);
                if phase == Some(Phase::Recording) {
                    self.finish_bound(app, &id).ok();
                } else if matches!(phase, Some(Phase::Connecting))
                    || self.snapshot().is_ok_and(|s| s.stage == "preparing")
                {
                    self.cancel_bound(Some(&id)).ok();
                }
            }
            return;
        }
        if !pressed {
            return;
        }
        let Ok(reserved) = self.reserve(app, "external", epoch, Some(&task)) else {
            return;
        };
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = app
                .state::<Self>()
                .start_reserved(app.clone(), "external", anchor, None, None, reserved)
                .await;
        });
    }
    pub async fn start(
        &self,
        app: tauri::AppHandle,
        source: &str,
        epoch: u64,
        anchor: Option<Anchor>,
        retry: Option<String>,
        expected: Option<(String, i64)>,
    ) -> Result<SpeechSnapshot, Error> {
        let reserved = self.reserve(&app, source, epoch, None)?;
        self.start_reserved(app, source, anchor, retry, expected, reserved)
            .await
    }
    pub fn pause_shortcuts(&self, service: &ShortcutService) -> Result<bool, Error> {
        let active = self.active.lock().map_err(|_| Error::new("unavailable"))?;
        if service.is_capturing() {
            return Err(Error::new("busy"));
        }
        service.block_triggers(true);
        Ok(active.is_some())
    }
    fn reserve(
        &self,
        app: &tauri::AppHandle,
        source: &str,
        epoch: u64,
        event: Option<&TaskStamp>,
    ) -> Result<Reservation, Error> {
        let id = uuid::Uuid::new_v4().to_string();
        let (tx, _) = watch::channel(0u8);
        let gate = Arc::new(DeliveryGate::default());
        {
            let mut a = self.active.lock().map_err(|_| Error::new("unavailable"))?;
            if self.stopping.load(Ordering::Acquire) {
                return Err(Error::new("stopping"));
            }
            if a.is_some() || app.state::<ShortcutService>().triggers_paused() {
                return Err(Error::new("busy"));
            }
            if event.is_some_and(|event| !app.state::<ShortcutService>().task_is_current(event)) {
                return Err(Error::new("staleSession"));
            }
            let runtime = crate::read(app).map_err(|_| Error::new("unavailable"))?;
            *a = Some(Active {
                id: id.clone(),
                source: source.into(),
                epoch,
                origin: event.cloned(),
                cancel: tx.clone(),
                gate: gate.clone(),
                original_overlay: runtime.windows.overlay,
                overlay_revision: crate::OVERLAY_VISIBILITY.load(Ordering::Acquire),
            });
        }
        app.state::<ShortcutService>().active_task(Some(id.clone()));
        let sequence = self.snapshot()?.sequence + 1;
        *self
            .snapshot
            .lock()
            .map_err(|_| Error::new("unavailable"))? = Snapshot {
            task_id: Some(id.clone()),
            source: source.into(),
            sequence,
            stage: "preparing".into(),
            ..Snapshot::default()
        };
        Ok(Reservation {
            id,
            expected_task: None,
            cancel: tx,
            gate,
        })
    }
    async fn start_reserved(
        &self,
        app: tauri::AppHandle,
        source: &str,
        anchor: Option<Anchor>,
        retry: Option<String>,
        expected: Option<(String, i64)>,
        reserved: Reservation,
    ) -> Result<SpeechSnapshot, Error> {
        let Reservation {
            id,
            expected_task,
            cancel: tx,
            gate,
        } = reserved;
        app.state::<ShortcutService>().active_task(Some(id.clone()));
        self.update(&app, &id, |_| {});
        let input = app.state::<WindowsInputService>();
        let capture = if source == "external" {
            let service = input.inner().clone();
            let task = id.clone();
            Some(tauri::async_runtime::spawn(async move {
                if let Some(anchor) = anchor {
                    service.capture(task, anchor).await
                } else {
                    Err(Error::new("targetUnavailable"))
                }
            }))
        } else {
            None
        };
        if source == "external" {
            let _ = crate::window_action_internal(&app, "overlay", "show");
        }
        let (terminal_tx, terminal_rx) = oneshot::channel();
        let result = app
            .state::<SpeechService>()
            .start_native(
                app.clone(),
                retry,
                expected,
                NativeStart {
                    id: id.clone(),
                    expected_task,
                    cancel: tx,
                    terminal: terminal_tx,
                },
            )
            .await;
        if result.is_err() {
            if let Some(c) = capture {
                let _ = c.await;
            }
            input.forget(id.clone());
            self.update(&app, &id, |s| {
                s.stage = "idle".into();
                s.error = result.as_ref().err().map(|e| e.code.clone());
            });
            if !gate.cancelled()
                && let Err(error) = &result
                && error.code != "cancelled"
                && error.code != "staleSession"
            {
                app.state::<SpeechService>().start_error(&app, &id, error);
            }
            self.release(&app, &id).await;
            return result;
        }
        self.update(&app, &id, |s| s.stage = "recognizing".into());
        let source = source.to_string();
        tauri::async_runtime::spawn(async move {
            let capture_result = match capture {
                Some(c) => c
                    .await
                    .unwrap_or_else(|_| Err(Error::new("targetUnsupported"))),
                None => Err(Error::new("targetUnavailable")),
            };
            let coordinator = app.state::<Self>();
            if let Ok(terminal) = terminal_rx.await {
                coordinator.update(&app, &id, |s| {
                    s.text = terminal.text.clone();
                    s.stage = "result".into();
                    s.error = terminal.error.clone();
                });
                if source == "external" && !terminal.text.is_empty() {
                    let mut delivery = if terminal.phase == Phase::Completed
                        && !gate.cancelled()
                        && capture_result.is_ok()
                        && app.state::<ShortcutService>().target_tracking_ready()
                    {
                        let journal = if terminal.persistent {
                            coordinator
                                .journal(&app, &id, terminal.attempt_id.as_deref(), "pending", None)
                                .await
                        } else {
                            Ok(())
                        };
                        if journal.is_err() || terminal.persistent && !terminal.saved {
                            Delivery::copy("saveFailed")
                        } else {
                            coordinator.update(&app, &id, |s| s.stage = "delivering".into());
                            let preparing = if terminal.persistent {
                                coordinator
                                    .journal(
                                        &app,
                                        &id,
                                        terminal.attempt_id.as_deref(),
                                        "delivering",
                                        None,
                                    )
                                    .await
                            } else {
                                Ok(())
                            };
                            if preparing.is_err() {
                                Delivery::copy("saveFailed")
                            } else {
                                app.state::<WindowsInputService>()
                                    .deliver(id.clone(), terminal.text.clone(), gate.clone())
                                    .await
                            }
                        }
                    } else {
                        Delivery::copy(if gate.cancelled() {
                            "cancelled"
                        } else if terminal.phase != Phase::Completed {
                            "recognitionFailed"
                        } else if let Err(error) = &capture_result {
                            &error.code
                        } else {
                            "targetUnsupported"
                        })
                    };
                    if terminal.persistent {
                        let result = coordinator
                            .journal(
                                &app,
                                &id,
                                terminal.attempt_id.as_deref(),
                                &delivery.state,
                                delivery.error.clone(),
                            )
                            .await;
                        if result.is_err() {
                            delivery.error = Some("saveFailed".into());
                        }
                    }
                    coordinator.update(&app, &id, |s| {
                        log::info!(target:"sayweave", "operation=dictation_delivery task_id={} state={} elapsed_ms={} error={}", id, delivery.state, delivery.elapsed_ms, delivery.error.as_deref().unwrap_or("none"));
                        s.delivery = Some(delivery);
                        s.stage = "result".into();
                    });
                }
            } else {
                let partial = app
                    .state::<SpeechService>()
                    .snapshot()
                    .ok()
                    .filter(|s| s.task_id.as_deref() == Some(&id))
                    .map(|s| s.text)
                    .unwrap_or_default();
                coordinator.update(&app, &id, |s| {
                    s.error = Some("interrupted".into());
                    s.stage = "result".into();
                    s.text = partial;
                    if source == "external" && !s.text.is_empty() {
                        s.delivery = Some(Delivery::copy("recognitionFailed"));
                    }
                });
            }
            app.state::<WindowsInputService>().forget(id.clone());
            coordinator.release(&app, &id).await;
        });
        result
    }
    async fn journal(
        &self,
        app: &tauri::AppHandle,
        id: &str,
        attempt: Option<&str>,
        state: &str,
        error: Option<String>,
    ) -> Result<(), Error> {
        let attempt = attempt.ok_or_else(|| Error::new("saveFailed"))?;
        app.state::<ShortcutService>()
            .db(DbOperation::Delivery {
                task: id.into(),
                attempt: attempt.into(),
                state: state.into(),
                error,
            })
            .await
            .map(|_| ())
    }
    async fn release(&self, app: &tauri::AppHandle, id: &str) {
        let old = {
            let Ok(a) = self.active.lock() else {
                return;
            };
            if !a.as_ref().is_some_and(|a| a.id == id) {
                return;
            }
            a.clone()
        };
        app.state::<ShortcutService>().settle(app).await;
        {
            let Ok(mut a) = self.active.lock() else {
                return;
            };
            if !a.as_ref().is_some_and(|a| a.id == id) {
                return;
            }
            app.state::<ShortcutService>().active_task(None);
            *a = None;
        }
        if let Some(old) = old
            && old.source == "external"
            && !old.original_overlay
            && old.overlay_revision == crate::OVERLAY_VISIBILITY.load(Ordering::Acquire)
        {
            let hide = self.snapshot().is_ok_and(|s| {
                s.delivery
                    .as_ref()
                    .is_some_and(|d| d.state == "confirmed" && !d.restore_error)
                    || s.text.is_empty()
                        && (s.error.is_none() || s.error.as_deref() == Some("cancelled"))
            });
            if hide {
                let app = app.clone();
                let id = id.to_string();
                let visibility = old.overlay_revision;
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                    if app
                        .state::<Self>()
                        .snapshot()
                        .is_ok_and(|s| s.task_id.as_ref() == Some(&id))
                        && !app.state::<Self>().busy()
                        && visibility == crate::OVERLAY_VISIBILITY.load(Ordering::Acquire)
                    {
                        let _ = crate::window_action_internal(&app, "overlay", "hide");
                    }
                });
            }
        }
    }
    pub fn finish_bound(&self, app: &tauri::AppHandle, id: &str) -> Result<(), Error> {
        let active = self.active.lock().map_err(|_| Error::new("unavailable"))?;
        let a = active
            .as_ref()
            .filter(|a| a.id == id)
            .ok_or_else(|| Error::new("staleSession"))?;
        let s = app.state::<SpeechService>().snapshot()?;
        if s.task_id.as_ref() == Some(&a.id) && s.phase == Phase::Recording {
            a.cancel.send(1).map_err(|_| Error::new("staleSession"))
        } else {
            Err(Error::new("busy"))
        }
    }
    pub fn cancel_bound(&self, id: Option<&str>) -> Result<(), Error> {
        let active = self.active.lock().map_err(|_| Error::new("unavailable"))?;
        let a = active
            .as_ref()
            .filter(|a| Some(a.id.as_str()) == id)
            .ok_or_else(|| Error::new("staleSession"))?;
        if !a.gate.cancel() {
            return Err(Error::new("inputSubmitted"));
        }
        a.cancel.send_replace(2);
        Ok(())
    }
    pub fn interrupt_bound(&self, id: Option<&str>) {
        if let Ok(a) = self.active.lock()
            && let Some(a) = a.as_ref().filter(|a| Some(a.id.as_str()) == id)
        {
            a.gate.cancel();
            a.cancel.send_replace(3);
        }
    }
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::Release);
        if let Ok(a) = self.active.lock()
            && let Some(a) = a.as_ref()
        {
            a.gate.cancel();
            a.cancel.send_replace(3);
        }
    }
    pub async fn drain(&self) {
        while self.busy() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
}
fn key_matches_task(
    id: Option<&str>,
    origin: Option<&TaskStamp>,
    event: &TaskStamp,
    current: bool,
) -> bool {
    current && id == event.id.as_deref()
        || id.is_some() && event.id.is_none() && origin == Some(event)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_release_and_second_toggle_can_only_cancel_the_task_they_started() {
        let initial = TaskStamp {
            id: None,
            sequence: 0,
        };
        // Both presses/releases can be enqueued before the first press creates A.
        assert!(key_matches_task(Some("A"), Some(&initial), &initial, false));
        assert!(!key_matches_task(None, None, &initial, false));
        let next_idle = TaskStamp {
            id: None,
            sequence: 2,
        };
        assert!(!key_matches_task(
            Some("B"),
            Some(&next_idle),
            &initial,
            false
        ));
        let a = TaskStamp {
            id: Some("A".into()),
            sequence: 1,
        };
        assert!(!key_matches_task(Some("B"), Some(&next_idle), &a, false));
        assert!(key_matches_task(None, None, &next_idle, true));
    }
}
#[tauri::command]
pub fn dictation_snapshot(
    window: tauri::WebviewWindow,
    service: tauri::State<'_, DictationCoordinator>,
) -> Result<Snapshot, Error> {
    caller(window.label())?;
    service.snapshot()
}
fn caller(label: &str) -> Result<(), Error> {
    if ["main", "voice-overlay"].contains(&label) {
        Ok(())
    } else {
        Err(Error::new("forbidden"))
    }
}
#[tauri::command]
pub async fn dictation_start(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    service: tauri::State<'_, DictationCoordinator>,
) -> Result<SpeechSnapshot, Error> {
    caller(window.label())?;
    service
        .start(
            app.clone(),
            "external",
            0,
            WindowsInputService::anchor(),
            None,
            None,
        )
        .await
}
#[tauri::command]
pub fn dictation_control(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    service: tauri::State<'_, DictationCoordinator>,
    action: String,
    task_id: String,
) -> Result<(), Error> {
    caller(window.label())?;
    match action.as_str() {
        "finish" => service.finish_bound(&app, &task_id),
        "cancel" => service.cancel_bound(Some(&task_id)),
        _ => Err(Error::new("invalid")),
    }
}
#[tauri::command]
pub async fn dictation_copy(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
    service: tauri::State<'_, DictationCoordinator>,
    task_id: String,
) -> Result<(), Error> {
    caller(window.label())?;
    let snapshot = service.snapshot()?;
    if snapshot.task_id.as_deref() != Some(&task_id) {
        return Err(Error::new("staleSession"));
    }
    let text = snapshot.text;
    if text.is_empty() {
        return Err(Error::new("invalid"));
    }
    app.state::<WindowsInputService>().copy(text).await
}
#[tauri::command]
pub async fn dictation_restore(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Result<(), Error> {
    caller(window.label())?;
    app.state::<WindowsInputService>().restore().await
}
