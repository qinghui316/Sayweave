use super::{
    ShortcutService,
    types::{Config, Status},
};
use crate::history::types::Error;
use tauri::{State, WebviewWindow};
#[tauri::command]
pub fn shortcut_status(
    window: WebviewWindow,
    service: State<'_, ShortcutService>,
) -> Result<Status, Error> {
    crate::data::model::authorize(window.label())?;
    service.status()
}
#[tauri::command]
pub async fn shortcut_save(
    app: tauri::AppHandle,
    window: WebviewWindow,
    service: State<'_, ShortcutService>,
    config: Config,
) -> Result<Status, Error> {
    let _admission = crate::lifecycle::mutation(&window)?;
    crate::data::model::authorize(window.label())?;
    service.save(&app, config).await
}
#[tauri::command]
pub async fn shortcut_capture(
    app: tauri::AppHandle,
    window: WebviewWindow,
    service: State<'_, ShortcutService>,
    begin: bool,
    token: Option<String>,
) -> Result<Status, Error> {
    let _admission = crate::lifecycle::mutation(&window)?;
    crate::data::model::authorize(window.label())?;
    let status = service.capture_owned(&app, begin, token).await?;
    let capture_token = status.capture_token.clone();
    if begin {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            app.state::<ShortcutService>()
                .finish_capture(&app, capture_token, None, None)
                .await
                .ok();
        });
    }
    Ok(status)
}
use tauri::Manager;

#[tauri::command]
pub async fn shortcut_capture_candidate(
    app: tauri::AppHandle,
    window: WebviewWindow,
    service: State<'_, ShortcutService>,
    token: String,
    binding: String,
) -> Result<Status, Error> {
    let _admission = crate::lifecycle::mutation(&window)?;
    crate::data::model::authorize(window.label())?;
    uuid::Uuid::parse_str(&token).map_err(|_| Error::new("invalidShortcut"))?;
    if !window.is_focused().unwrap_or(false) {
        return Err(Error::new("unavailable"));
    }
    service
        .finish_capture(&app, Some(token), None, Some(binding))
        .await
}
