use super::{ModelService, provider::TestResult, types::*};
use tauri::{State, WebviewWindow};
#[tauri::command]
pub async fn model_status(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    retry: bool,
) -> Result<ModelStatus, ModelError> {
    state.status(window.label(), retry).await
}
#[tauri::command]
pub async fn model_overview(
    window: WebviewWindow,
    state: State<'_, ModelService>,
) -> Result<Overview, ModelError> {
    state.overview(window.label()).await
}
#[tauri::command]
pub async fn model_save(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    request: SaveRequest,
) -> Result<Overview, ModelError> {
    state.save(window.label(), request).await
}
#[tauri::command]
pub async fn model_remove(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    request: RemoveRequest,
) -> Result<Overview, ModelError> {
    state.remove(window.label(), request).await
}
#[tauri::command]
pub async fn model_default(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    request: DefaultRequest,
) -> Result<Overview, ModelError> {
    state.set_default(window.label(), request).await
}
#[tauri::command]
pub async fn model_fetch(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    draft: NetworkDraft,
    request_id: String,
) -> Result<Vec<String>, ModelError> {
    state.fetch(window.label(), draft, request_id).await
}
#[tauri::command]
pub async fn model_test(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    draft: NetworkDraft,
    request_id: String,
) -> Result<TestResult, ModelError> {
    state.test(window.label(), draft, request_id).await
}
#[tauri::command]
pub fn model_cancel(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    request_id: String,
) -> Result<(), ModelError> {
    authorize(window.label())?;
    state.provider.cancel(&request_id)
}
