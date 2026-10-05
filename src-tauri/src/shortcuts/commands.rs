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
    crate::data::model::authorize(window.label())?;
    service.save(&app, config).await
}
#[tauri::command]
pub fn shortcut_capture(
    app: tauri::AppHandle,
    window: WebviewWindow,
    service: State<'_, ShortcutService>,
    begin: bool,
    token: Option<String>,
) -> Result<Status, Error> {
    crate::data::model::authorize(window.label())?;
    let status = service.capture_owned(&app, begin, token)?;
    let generation = service.capture_generation();
    if begin {
        tauri::async_runtime::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            if app.state::<ShortcutService>().capture_generation() == generation {
                app.state::<ShortcutService>().capture(&app, false).ok();
            }
        });
    }
    Ok(status)
}
use tauri::Manager;
