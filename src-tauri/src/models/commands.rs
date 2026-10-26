use super::{ModelService, provider::TestResult, types::*};
use tauri::{State, WebviewWindow};
use tauri_plugin_opener::OpenerExt;

fn help_url(caller: &str, help: &str) -> Result<&'static str, ModelError> {
    if caller != "main" {
        return Err(ModelError::new("forbidden"));
    }
    match help {
        "doubao-api-key" => Ok("https://console.volcengine.com/speech/app"),
        "qwen-api-key" => Ok("https://help.aliyun.com/zh/model-studio/get-api-key"),
        _ => Err(ModelError::field("help")),
    }
}

#[tauri::command]
pub fn model_help(
    app: tauri::AppHandle,
    window: WebviewWindow,
    help: String,
) -> Result<(), ModelError> {
    let url = help_url(window.label(), &help)?;
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|_| ModelError::new("unavailable"))
}

#[cfg(test)]
mod help_tests {
    use super::*;
    #[test]
    fn fixed_help_is_main_only() {
        assert!(
            help_url("main", "qwen-api-key")
                .unwrap()
                .starts_with("https://help.aliyun.com/")
        );
        assert_eq!(
            help_url("voice-overlay", "qwen-api-key").unwrap_err().code,
            "forbidden"
        );
        assert_eq!(
            help_url("main", "https://example.com").unwrap_err().code,
            "invalid"
        );
    }
}
async fn observed<T>(
    operation: &'static str,
    request: Option<&str>,
    task: impl std::future::Future<Output = Result<T, ModelError>>,
) -> Result<T, ModelError> {
    let started = std::time::Instant::now();
    let result = task.await;
    let request = request
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
        .unwrap_or("none");
    let code = result
        .as_ref()
        .err()
        .map(|e| e.code.as_str())
        .unwrap_or("ok");
    log::info!(target: "sayweave", "operation={operation} request={request} stage=complete elapsed_ms={} code={code}", started.elapsed().as_millis());
    result
}
#[tauri::command]
pub async fn model_status(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    retry: bool,
) -> Result<ModelStatus, ModelError> {
    observed("model_status", None, state.status(window.label(), retry)).await
}
#[tauri::command]
pub async fn model_overview(
    window: WebviewWindow,
    state: State<'_, ModelService>,
) -> Result<Overview, ModelError> {
    observed("model_overview", None, state.overview(window.label())).await
}
#[tauri::command]
pub async fn model_save(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    request: SaveRequest,
) -> Result<Overview, ModelError> {
    let _admission = crate::lifecycle::mutation(&window).map_err(ModelError::from)?;
    observed("model_save", None, state.save(window.label(), request)).await
}
#[tauri::command]
pub async fn model_local_save(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    request: LocalSaveRequest,
) -> Result<Overview, ModelError> {
    let _admission = crate::lifecycle::mutation(&window).map_err(ModelError::from)?;
    observed(
        "model_local_save",
        None,
        state.save_local(window.label(), request),
    )
    .await
}
#[tauri::command]
pub async fn model_asset_status(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    asset_id: String,
) -> Result<crate::model_assets::AssetStatus, ModelError> {
    authorize(window.label())?;
    state
        .local_assets()?
        .status(&asset_id)
        .await
        .map_err(ModelError::new)
}
#[tauri::command]
pub async fn model_asset_download(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    asset_id: String,
) -> Result<crate::model_assets::AssetStatus, ModelError> {
    let _admission = crate::lifecycle::mutation(&window).map_err(ModelError::from)?;
    authorize(window.label())?;
    state
        .local_assets()?
        .download(&asset_id)
        .await
        .map_err(ModelError::new)
}
#[tauri::command]
pub fn model_asset_cancel(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    asset_id: String,
) -> Result<(), ModelError> {
    authorize(window.label())?;
    state
        .local_assets()?
        .cancel(&asset_id)
        .map_err(ModelError::new)
}
#[tauri::command]
pub async fn model_asset_delete(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    asset_id: String,
) -> Result<(), ModelError> {
    let _admission = crate::lifecycle::mutation(&window).map_err(ModelError::from)?;
    state.check(window.label())?;
    let _guard = state.gate.try_lock().map_err(|_| ModelError::new("busy"))?;
    let view = state.stored().await?;
    if view
        .local_configs
        .iter()
        .any(|c| c.input.asset_id == asset_id)
    {
        return Err(ModelError::new("localConfigurationPresent"));
    }
    if let Some(local) = state.local.get() {
        local.runtime.unload_idle().await.map_err(ModelError::new)?;
    }
    state
        .local_assets()?
        .delete(&asset_id)
        .await
        .map_err(ModelError::new)
}
#[tauri::command]
pub async fn model_remove(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    request: RemoveRequest,
) -> Result<Overview, ModelError> {
    let _admission = crate::lifecycle::mutation(&window).map_err(ModelError::from)?;
    observed("model_remove", None, state.remove(window.label(), request)).await
}
#[tauri::command]
pub async fn model_default(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    request: DefaultRequest,
) -> Result<Overview, ModelError> {
    let _admission = crate::lifecycle::mutation(&window).map_err(ModelError::from)?;
    observed(
        "model_default",
        None,
        state.set_default(window.label(), request),
    )
    .await
}
#[tauri::command]
pub async fn model_fetch(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    draft: NetworkDraft,
    request_id: String,
) -> Result<Vec<String>, ModelError> {
    let _admission = crate::lifecycle::mutation(&window).map_err(ModelError::from)?;
    observed(
        "model_fetch",
        Some(&request_id),
        state.fetch(window.label(), draft, request_id.clone()),
    )
    .await
}
#[tauri::command]
pub async fn model_test(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    draft: NetworkDraft,
    request_id: String,
) -> Result<TestResult, ModelError> {
    let _admission = crate::lifecycle::mutation(&window).map_err(ModelError::from)?;
    observed(
        "model_test",
        Some(&request_id),
        state.test(window.label(), draft, request_id.clone()),
    )
    .await
}
#[tauri::command]
pub fn model_cancel(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    request_id: String,
) -> Result<(), ModelError> {
    authorize(window.label())?;
    state.requests.cancel(&request_id).map_err(ModelError::from)
}
