use super::{Snapshot, SpeechService, audio::Device};
use crate::{
    data::{DataWorker, Operation},
    history::{playback::PlaybackState, types::*},
};
use tauri::{Manager, State, WebviewWindow};
fn speech_caller(window: &WebviewWindow) -> Result<(), Error> {
    if ["main", "voice-overlay"].contains(&window.label()) {
        Ok(())
    } else {
        Err(Error::new("forbidden"))
    }
}
fn main(window: &WebviewWindow) -> Result<(), Error> {
    crate::data::model::authorize(window.label())
}
#[tauri::command]
pub fn speech_snapshot(
    window: WebviewWindow,
    service: State<'_, SpeechService>,
) -> Result<Snapshot, Error> {
    speech_caller(&window)?;
    service.snapshot()
}
#[tauri::command]
pub async fn speech_devices(
    window: WebviewWindow,
    service: State<'_, SpeechService>,
) -> Result<Vec<Device>, Error> {
    main(&window)?;
    let _operation = service.admit()?;
    tokio::task::spawn_blocking(super::audio::devices)
        .await
        .map_err(|_| Error::new("deviceAccess"))?
}
#[tauri::command]
pub async fn speech_start(app: tauri::AppHandle, window: WebviewWindow) -> Result<Snapshot, Error> {
    speech_caller(&window)?;
    app.state::<crate::dictation::DictationCoordinator>()
        .start(app.clone(), "trial", 0, None, None, None)
        .await
}
#[tauri::command]
pub fn speech_control(window: WebviewWindow, task_id: String, action: String) -> Result<(), Error> {
    speech_caller(&window)?;
    let app = window.app_handle();
    if action == "cancel" {
        app.state::<crate::dictation::DictationCoordinator>()
            .cancel_bound(Some(&task_id))
    } else if action == "finish" {
        app.state::<crate::dictation::DictationCoordinator>()
            .finish_bound(app, &task_id)
    } else {
        Err(Error::new("invalid"))
    }
}
#[tauri::command]
pub async fn speech_preferences(
    window: WebviewWindow,
    service: State<'_, SpeechService>,
    preferences: Option<Preferences>,
) -> Result<Preferences, Error> {
    main(&window)?;
    let _operation = service.admit()?;
    match preferences {
        None => service.history.preferences().await,
        Some(p) => match service.history.db(DbOperation::SavePreferences(p)).await? {
            DbOutput::Preferences(v) => Ok(v),
            _ => Err(Error::new("unavailable")),
        },
    }
}
#[tauri::command]
pub async fn history_status(
    window: WebviewWindow,
    service: State<'_, SpeechService>,
    data: State<'_, DataWorker>,
    retry: bool,
) -> Result<crate::data::model::Status, Error> {
    main(&window)?;
    let _operation = service.admit()?;
    if retry {
        data.request("main", Operation::Retry).await?;
    }
    if data.status()?.state == "ready" {
        service.initialize().await?;
    }
    data.status()
}
#[tauri::command]
pub async fn history_list(
    window: WebviewWindow,
    service: State<'_, SpeechService>,
    query: Query,
) -> Result<Page, Error> {
    main(&window)?;
    let _operation = service.admit()?;
    service.initialize().await?;
    match service.history.db(DbOperation::List(query)).await? {
        DbOutput::Page(v) => Ok(v),
        _ => Err(Error::new("unavailable")),
    }
}
#[tauri::command]
pub async fn history_browse(
    window: WebviewWindow,
    service: State<'_, SpeechService>,
    query: BrowseQuery,
) -> Result<Feed, Error> {
    main(&window)?;
    let _operation = service.admit()?;
    service.initialize().await?;
    match service.history.db(DbOperation::Browse(query)).await? {
        DbOutput::Feed(v) => Ok(v),
        _ => Err(Error::new("unavailable")),
    }
}
#[tauri::command]
pub async fn history_detail(
    window: WebviewWindow,
    service: State<'_, SpeechService>,
    id: String,
) -> Result<Record, Error> {
    main(&window)?;
    let _operation = service.admit()?;
    service.history.detail(&id).await
}
#[tauri::command]
pub async fn history_delivery(
    window: WebviewWindow,
    service: State<'_, crate::shortcuts::ShortcutService>,
    speech: State<'_, SpeechService>,
    id: String,
) -> Result<Vec<crate::shortcuts::types::DeliveryReport>, Error> {
    main(&window)?;
    let _operation = speech.admit()?;
    match service
        .db(crate::shortcuts::types::DbOperation::Deliveries(id))
        .await?
    {
        crate::shortcuts::types::DbOutput::Deliveries(rows) => Ok(rows),
        _ => Err(Error::new("unavailable")),
    }
}
#[tauri::command]
pub async fn history_usage(
    window: WebviewWindow,
    service: State<'_, SpeechService>,
) -> Result<Usage, Error> {
    main(&window)?;
    let _operation = service.admit()?;
    match service.history.db(DbOperation::Usage).await? {
        DbOutput::Usage(v) => Ok(v),
        _ => Err(Error::new("unavailable")),
    }
}
#[tauri::command]
pub async fn history_remove(
    window: WebviewWindow,
    service: State<'_, SpeechService>,
    id: String,
    revision: i64,
) -> Result<(), Error> {
    main(&window)?;
    let _operation = service.admit()?;
    if window
        .app_handle()
        .state::<crate::dictation::DictationCoordinator>()
        .busy()
        && service.snapshot()?.record_id.as_ref() == Some(&id)
    {
        return Err(Error::new("busy"));
    }
    service.history.remove(&id, revision).await?;
    service.forget_audio(&window.app_handle().clone(), &id);
    Ok(())
}
#[tauri::command]
pub async fn history_retry(
    app: tauri::AppHandle,
    window: WebviewWindow,
    service: State<'_, SpeechService>,
    id: String,
    config_id: String,
    config_revision: i64,
) -> Result<Snapshot, Error> {
    main(&window)?;
    let _ = service;
    app.state::<crate::dictation::DictationCoordinator>()
        .start(
            app.clone(),
            "history",
            0,
            None,
            Some(id),
            Some((config_id, config_revision)),
        )
        .await
}
#[tauri::command]
pub async fn history_play(
    window: WebviewWindow,
    service: State<'_, SpeechService>,
    resource_id: Option<String>,
    action: String,
    position_ms: Option<u64>,
) -> Result<PlaybackState, Error> {
    main(&window)?;
    let _operation = service.admit()?;
    if action == "play" {
        if window
            .app_handle()
            .state::<crate::dictation::DictationCoordinator>()
            .busy()
        {
            return Err(Error::new("busy"));
        }
        service
            .play(resource_id.as_deref().ok_or(Error::new("invalid"))?)
            .await
    } else {
        if service.is_active() && action != "stop" && action != "status" {
            return Err(Error::new("busy"));
        }
        service.playback.control(&action, position_ms).await
    }
}
#[tauri::command]
pub async fn speech_copy(
    app: tauri::AppHandle,
    window: WebviewWindow,
    text: String,
) -> Result<(), Error> {
    main(&window)?;
    let _operation = app.state::<SpeechService>().admit()?;
    if text.len() > 1024 * 1024 {
        return Err(Error::new("invalid"));
    }
    app.state::<crate::windows_input::WindowsInputService>()
        .copy(text)
        .await
}

#[tauri::command]
pub async fn speech_retry_current(
    app: tauri::AppHandle,
    window: WebviewWindow,
    service: State<'_, SpeechService>,
    task_id: String,
) -> Result<Snapshot, Error> {
    main(&window)?;
    let current = service.snapshot()?;
    if current.task_id.as_deref() != Some(task_id.as_str()) {
        return Err(Error::new("staleSession"));
    }
    if !current.can_retry {
        return Err(Error::new("fileUnavailable"));
    }
    let id = current
        .source_record_id
        .ok_or_else(|| Error::new("fileUnavailable"))?;
    app.state::<crate::dictation::DictationCoordinator>()
        .retry_displayed(
            app.clone(),
            id,
            current.task_id.ok_or_else(|| Error::new("staleSession"))?,
        )
        .await
}
