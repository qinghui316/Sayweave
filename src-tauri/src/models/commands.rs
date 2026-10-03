use super::{ModelService, provider::TestResult, types::*};
use tauri::{State, WebviewWindow};
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
    observed("model_save", None, state.save(window.label(), request)).await
}
#[tauri::command]
pub async fn model_remove(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    request: RemoveRequest,
) -> Result<Overview, ModelError> {
    observed("model_remove", None, state.remove(window.label(), request)).await
}
#[tauri::command]
pub async fn model_default(
    window: WebviewWindow,
    state: State<'_, ModelService>,
    request: DefaultRequest,
) -> Result<Overview, ModelError> {
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
    state.provider.cancel(&request_id)
}
