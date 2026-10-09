//! Main-window configuration port. Processing policies remain UI-independent.
use crate::{
    contracts::{
        dictionary::DataError,
        processing::{DbOperation, DbOutput, Settings},
    },
    data::{DataWorker, Operation, Output},
    models::{
        ModelService,
        types::{Usage, executable},
    },
};
use serde::Serialize;
use tauri::{State, WebviewWindow};
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsView {
    pub settings: Settings,
    pub available: bool,
    pub model: Option<String>,
    pub error: Option<String>,
    pub personalization: Option<crate::contracts::language::Settings>,
}
fn authorize(label: &str) -> Result<(), DataError> {
    crate::contracts::dictionary::authorize(label)
}
async fn config(data: &DataWorker, operation: DbOperation) -> Result<Settings, DataError> {
    match data
        .request_admitted(Operation::Processing(operation))
        .await?
    {
        Output::Processing(DbOutput::Settings(value)) => Ok(value),
        _ => Err(DataError::new("unavailable")),
    }
}
async fn view(data: &DataWorker, models: &ModelService) -> Result<SettingsView, DataError> {
    let settings = config(data, DbOperation::Settings).await?;
    let personalization = match crate::language_learning::operation(
        data,
        crate::contracts::language::DbOperation::Settings,
    )
    .await
    {
        Ok(crate::contracts::language::DbOutput::Settings(value)) => Some(value),
        _ => None,
    };
    let result = async {
        models.status("main", false).await?;
        models.overview("main").await
    };
    match result.await {
        Ok(overview) => {
            let selected = overview
                .defaults
                .iter()
                .find(|d| d.usage == Usage::Text)
                .and_then(|d| d.config_id.as_deref());
            let model = overview
                .configs
                .iter()
                .find(|c| Some(c.id.as_str()) == selected && executable(&c.input, &Usage::Text));
            Ok(SettingsView {
                personalization,
                settings,
                available: model.is_some_and(|c| c.credential_status == "present"),
                model: model.map(|c| c.input.name.clone()),
                error: model.and_then(|c| {
                    (c.credential_status != "present").then(|| "credentialMissing".into())
                }),
            })
        }
        Err(error) => Ok(SettingsView {
            personalization,
            settings,
            available: false,
            model: None,
            error: Some(error.code),
        }),
    }
}
#[tauri::command]
pub async fn processing_status(
    data: State<'_, DataWorker>,
    models: State<'_, ModelService>,
    window: WebviewWindow,
) -> Result<SettingsView, DataError> {
    authorize(window.label())?;
    view(&data, &models).await
}
#[tauri::command]
pub async fn processing_save(
    data: State<'_, DataWorker>,
    models: State<'_, ModelService>,
    window: WebviewWindow,
    settings: Settings,
) -> Result<SettingsView, DataError> {
    authorize(window.label())?;
    config(&data, DbOperation::SaveSettings(settings)).await?;
    view(&data, &models).await
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn settings_port_is_main_only() {
        assert!(authorize("main").is_ok());
        assert_eq!(authorize("voice-overlay").unwrap_err().code, "forbidden");
    }
}
