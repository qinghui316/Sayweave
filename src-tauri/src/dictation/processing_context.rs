//! Application assembly only. Pure prompt/source policies do not access storage.
use crate::{
    contracts::processing::{
        DbOperation, DbOutput, GeneratedText, RunMetadata, Settings, UnitRecord,
    },
    data::{DataWorker, Operation, Output},
    models::{ExecutableConfiguration, ModelService, types::ModelError},
    processing::{prompt, units::Unit},
};
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, Ordering},
};
use tauri::Manager;
use tokio::sync::watch;
#[derive(Clone)]
pub struct Context {
    pub task_id: String,
    pub provider: Arc<crate::models::provider::Provider>,
    pub configuration: Arc<ExecutableConfiguration>,
    pub dictionary: Arc<prompt::DictionaryIndex>,
    pub language: Option<crate::contracts::language::Snapshot>,
    pub settings: Settings,
    pub data: DataWorker,
    run: Arc<OnceLock<Option<RunBinding>>>,
    save_error: Arc<AtomicBool>,
    archive: crate::archive::Archive,
}
struct RunBinding {
    metadata: RunMetadata,
}
pub async fn settings(
    data: &DataWorker,
    operation: DbOperation,
) -> Result<DbOutput, crate::contracts::dictionary::DataError> {
    match data
        .request_admitted(Operation::Processing(operation))
        .await?
    {
        Output::Processing(value) => Ok(value),
        _ => Err(crate::contracts::dictionary::DataError::new("unavailable")),
    }
}
pub async fn dictionary(
    app: &tauri::AppHandle,
) -> Result<Arc<prompt::DictionaryIndex>, ModelError> {
    let DbOutput::Dictionary(records) =
        settings(&app.state::<DataWorker>(), DbOperation::DictionarySnapshot).await?
    else {
        return Err(ModelError::new("unavailable"));
    };
    Ok(Arc::new(prompt::DictionaryIndex::new(records)))
}
impl Context {
    #[cfg(test)]
    pub(crate) fn fixture(
        provider: Arc<crate::models::provider::Provider>,
        input: crate::models::types::ConfigInput,
        data: DataWorker,
    ) -> Self {
        Self {
            task_id: "actor-test".into(),
            provider,
            configuration: Arc::new(ExecutableConfiguration {
                id: "fixture".into(),
                input,
                key: "fixture-key".into(),
                revision: 1,
            }),
            dictionary: Arc::new(prompt::DictionaryIndex::new(vec![])),
            language: None,
            settings: Settings {
                enabled: true,
                revision: 1,
            },
            run: Arc::new(OnceLock::new()),
            save_error: Arc::new(AtomicBool::new(false)),
            archive: crate::archive::Archive::new(data.clone(), "actor-test".into()).unwrap(),
            data,
        }
    }
    pub async fn prepare(
        app: &tauri::AppHandle,
        mut cancel: watch::Receiver<u8>,
        task_id: &str,
        dictionary: Arc<prompt::DictionaryIndex>,
        language: Option<crate::contracts::language::Snapshot>,
        deadline: tokio::time::Instant,
    ) -> Result<Option<Self>, ModelError> {
        let preparation = async {
            let data = app.state::<DataWorker>();
            let DbOutput::Settings(config) = settings(&data, DbOperation::Settings).await? else {
                return Err(ModelError::new("unavailable"));
            };
            if !config.enabled {
                return Ok(None);
            }
            let models = app.state::<ModelService>();
            let configuration = match models.text_config().await {
                Ok(value) => value,
                Err(error) if error.code == "notConfigured" => return Ok(None),
                Err(error) => return Err(error),
            };
            Ok(Some(Self {
                task_id: task_id.into(),
                provider: models.provider.clone(),
                configuration: Arc::new(configuration),
                dictionary,
                language,
                settings: config,
                data: data.inner().clone(),
                run: Arc::new(OnceLock::new()),
                save_error: Arc::new(AtomicBool::new(false)),
                archive: crate::archive::Archive::new(data.inner().clone(), task_id.into())?,
            }))
        };
        if *cancel.borrow() > 0 {
            return Err(ModelError::new("cancelled"));
        }
        tokio::select! {
            biased;
            _=cancel.changed()=>Err(ModelError::new("cancelled")),
            value=tokio::time::timeout_at(deadline,preparation)=>value.unwrap_or_else(|_|Err(ModelError::new("timeout"))),
        }
    }
    pub async fn generate(
        &self,
        unit: &Unit,
        context: &str,
        deadline: tokio::time::Instant,
    ) -> Result<GeneratedText, ModelError> {
        let prompt = prompt::build_with_context(
            &unit.text,
            context,
            &self.dictionary,
            self.language
                .as_ref()
                .filter(|s| s.settings.personalization.enabled)
                .map(|s| &s.application),
            self.language
                .as_ref()
                .filter(|s| s.settings.personalization.enabled)
                .map_or(&[], |s| s.preferences.as_slice()),
        )
        .map_err(|e| ModelError::new(e.code))?;
        log::info!(target:"sayweave","operation=processing_request task_id={} unit_id={} turn={} prompt_version={} settings_revision={} source_chars={} prompt_bytes={} context_chars={} dictionary_items={}",self.task_id,unit.id,unit.turn,prompt::VERSION,self.settings.revision,unit.text.chars().count(),prompt.serialized_bytes,prompt.context_chars,prompt.dictionary.terms.len()+prompt.dictionary.corrections.len());
        let mut record = self.unit_record(unit)?;
        self.persist(&record);
        let result = self
            .provider
            .generate_unit(
                &self.configuration.input,
                &self.configuration.key,
                &unit.id,
                &prompt.messages,
                deadline,
            )
            .await;
        match &result {
            Ok(value) => {
                record.candidate_text = Some(value.text.clone());
                record.generation_state = "success".into();
                record.prompt_tokens = value.usage.input;
                record.completion_tokens = value.usage.output;
                record.elapsed_ms = value.elapsed_ms;
            }
            Err(error) => {
                record.generation_state = if error.code == "cancelled" {
                    "interrupted"
                } else {
                    "failed"
                }
                .into();
                record.application_state = "notApplied".into();
                record.error_code = Some(error.code.clone());
            }
        }
        self.persist(&record);
        result
    }
    pub fn cancel(&self, unit: &Unit) {
        let _ = self.provider.requests.cancel(&unit.id);
    }
    pub async fn attach(
        &self,
        record: Option<&str>,
        attempt: Option<&str>,
    ) -> Result<(), crate::contracts::dictionary::DataError> {
        let value = match (record, attempt) {
            (Some(_record), Some(attempt)) => {
                let metadata = RunMetadata {
                    id: uuid::Uuid::new_v4().to_string(),
                    attempt_id: attempt.into(),
                    task_id: self.task_id.clone(),
                    config_id: self.configuration.id.clone(),
                    config_name: self.configuration.input.name.clone(),
                    model_id: self.configuration.input.model_id.clone(),
                    config_revision: self.configuration.revision,
                    prompt_version: prompt::VERSION.into(),
                    settings_revision: self.settings.revision,
                    dictionary_version: self.dictionary.version.clone(),
                };
                settings(&self.data, DbOperation::Prepare(metadata.clone())).await?;
                Some(RunBinding { metadata })
            }
            _ => None,
        };
        self.run
            .set(value)
            .map_err(|_| crate::contracts::dictionary::DataError::new("conflict"))
    }
    fn unit_record(&self, unit: &Unit) -> Result<UnitRecord, ModelError> {
        let run = self
            .run
            .get()
            .and_then(|r| r.as_ref())
            .map(|r| r.metadata.id.clone())
            .unwrap_or_default();
        Ok(UnitRecord {
            id: unit.id.clone(),
            run_id: run,
            turn: unit.turn,
            source_start: unit.chars.start,
            source_end: unit.chars.end,
            source_refs: serde_json::to_string(&unit.source)
                .map_err(|_| ModelError::new("invalid"))?,
            source_text: unit.text.clone(),
            candidate_text: None,
            generation_state: "pending".into(),
            application_state: "pending".into(),
            source_committed_end: None,
            error_code: None,
            prompt_tokens: None,
            completion_tokens: None,
            elapsed_ms: 0,
        })
    }
    fn persist(&self, record: &UnitRecord) {
        if !record.run_id.is_empty()
            && !self.archive.enqueue(
                record.id.clone(),
                Operation::Processing(DbOperation::SaveUnit(record.clone())),
            )
        {
            self.save_error.store(true, Ordering::Release);
        }
    }
    fn final_records(
        &self,
        report: &super::drafts::Report,
    ) -> Result<Vec<UnitRecord>, crate::contracts::dictionary::DataError> {
        report
            .units
            .iter()
            .filter_map(|result| {
                let value = (|| {
                    let mut value = self
                        .unit_record(&result.unit)
                        .map_err(|_| crate::contracts::dictionary::DataError::new("invalid"))?;
                    value.candidate_text = result.candidate.clone();
                    value.generation_state = result.generation.clone();
                    value.application_state = result.application.clone();
                    value.source_committed_end = result.source_committed_end;
                    value.error_code = result.error.clone();
                    value.prompt_tokens = result.input_tokens;
                    value.completion_tokens = result.output_tokens;
                    value.elapsed_ms = result.elapsed_ms;
                    if report.cancelled {
                        crate::processing::retention::cancelled(
                            value,
                            result.applied_text.as_deref(),
                        )
                        .map_err(|_| crate::contracts::dictionary::DataError::new("invalid"))
                    } else {
                        Ok(Some(value))
                    }
                })();
                match value {
                    Ok(Some(value)) => Some(Ok(value)),
                    Ok(None) => None,
                    Err(error) => Some(Err(error)),
                }
            })
            .collect()
    }
    pub async fn stage_cancel(
        &self,
        report: &super::drafts::Report,
    ) -> Result<(), crate::contracts::dictionary::DataError> {
        if report.cancelled
            && let Some(Some(run)) = self.run.get()
        {
            self.data
                .stage_cancel(&Operation::Processing(DbOperation::Cancel {
                    id: run.metadata.id.clone(),
                    text: report.text.clone(),
                    raw: report.cancellation_raw().to_owned(),
                    rollback_failed: report.rollback_failed,
                    units: self.final_records(report)?,
                }))
                .await?;
        }
        Ok(())
    }
    pub async fn finish(
        &self,
        report: &super::drafts::Report,
        sources: &[crate::contracts::processing::SourceTurn],
    ) -> Result<bool, crate::contracts::dictionary::DataError> {
        // Seal before terminal/cancel persistence: late model completions cannot
        // restore units pruned by cancellation or overwrite application receipts.
        if !self.archive.finish().await {
            self.save_error.store(true, Ordering::Release);
        }
        let Some(Some(run)) = self.run.get() else {
            return Ok(false);
        };
        let retained = self.final_records(report)?;
        if !report.cancelled {
            for value in &retained {
                settings(&self.data, DbOperation::SaveUnit(value.clone())).await?;
            }
        }
        if report.cancelled {
            settings(
                &self.data,
                DbOperation::Cancel {
                    id: run.metadata.id.clone(),
                    text: report.text.clone(),
                    raw: report.cancellation_raw().to_owned(),
                    rollback_failed: report.rollback_failed,
                    units: retained,
                },
            )
            .await?;
            return Ok(true);
        }
        // These exact immutable receipts have all been saved above. Finishing
        // this run must not depend on a history read guarded by another cancel.
        let text = crate::processing::render::text(sources, &retained);
        let state =
            if report.error.is_some() || report.units.iter().any(|u| u.generation == "failed") {
                "partial"
            } else {
                "success"
            };
        settings(
            &self.data,
            DbOperation::Finish {
                id: run.metadata.id.clone(),
                text,
                state: state.into(),
                error: report.error.clone().or_else(|| {
                    self.save_error
                        .load(Ordering::Acquire)
                        .then(|| "saveFailed".into())
                }),
            },
        )
        .await?;
        if self.save_error.load(Ordering::Acquire) {
            return Err(crate::contracts::dictionary::DataError::new("saveFailed"));
        }
        Ok(true)
    }
}
