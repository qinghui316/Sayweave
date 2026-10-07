//! Application assembly only. Pure prompt/source policies do not access storage.
use crate::{
    contracts::processing::{
        DbOperation, DbOutput, GeneratedText, RunMetadata, Settings, UnitRecord,
    },
    data::{DataWorker, Operation, Output},
    models::{ExecutableConfiguration, ModelService, types::ModelError},
    processing::{prompt, units::Unit},
};
use std::{
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tauri::Manager;
use tokio::sync::watch;
#[derive(Clone)]
pub struct Context {
    pub task_id: String,
    pub provider: Arc<crate::models::provider::Provider>,
    pub configuration: Arc<ExecutableConfiguration>,
    pub dictionary: Arc<prompt::DictionaryIndex>,
    pub settings: Settings,
    pub data: DataWorker,
    run: Arc<OnceLock<Option<RunBinding>>>,
    save_error: Arc<AtomicBool>,
}
struct RunBinding {
    metadata: RunMetadata,
    record_id: String,
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
impl Context {
    pub async fn prepare(
        app: &tauri::AppHandle,
        mut cancel: watch::Receiver<u8>,
        task_id: &str,
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
            let DbOutput::Dictionary(dictionary) =
                settings(&data, DbOperation::DictionarySnapshot).await?
            else {
                return Err(ModelError::new("unavailable"));
            };
            Ok(Some(Self {
                task_id: task_id.into(),
                provider: models.provider.clone(),
                configuration: Arc::new(configuration),
                dictionary: Arc::new(prompt::DictionaryIndex::new(dictionary)),
                settings: config,
                data: data.inner().clone(),
                run: Arc::new(OnceLock::new()),
                save_error: Arc::new(AtomicBool::new(false)),
            }))
        };
        if *cancel.borrow() > 0 {
            return Err(ModelError::new("cancelled"));
        }
        tokio::select! {
            biased;
            _=cancel.changed()=>Err(ModelError::new("cancelled")),
            value=tokio::time::timeout(Duration::from_secs(15),preparation)=>value.unwrap_or_else(|_|Err(ModelError::new("timeout"))),
        }
    }
    pub async fn generate(
        &self,
        unit: &Unit,
        context: &str,
        deadline: tokio::time::Instant,
    ) -> Result<GeneratedText, ModelError> {
        let prompt = prompt::build(&unit.text, context, &self.dictionary)
            .map_err(|e| ModelError::new(e.code))?;
        log::info!(target:"sayweave","operation=processing_request task_id={} unit_id={} turn={} prompt_version={} settings_revision={} source_chars={} prompt_bytes={} context_chars={} dictionary_items={}",self.task_id,unit.id,unit.turn,prompt::VERSION,self.settings.revision,unit.text.chars().count(),prompt.serialized_bytes,prompt.context_chars,prompt.dictionary.terms.len()+prompt.dictionary.corrections.len());
        let mut record = self.unit_record(unit)?;
        self.persist(&record).await;
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
        self.persist(&record).await;
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
            (Some(record), Some(attempt)) => {
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
                Some(RunBinding {
                    metadata,
                    record_id: record.into(),
                })
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
    async fn persist(&self, record: &UnitRecord) {
        if record.run_id.is_empty() {
            return;
        }
        if settings(&self.data, DbOperation::SaveUnit(record.clone()))
            .await
            .is_err()
        {
            self.save_error.store(true, Ordering::Release);
        }
    }
    pub async fn finish(
        &self,
        report: &super::drafts::Report,
        sources: &[crate::contracts::processing::SourceTurn],
    ) -> Result<bool, crate::contracts::dictionary::DataError> {
        let Some(Some(run)) = self.run.get() else {
            return Ok(false);
        };
        let mut retained = Vec::new();
        for result in &report.units {
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
                if let Some(value) =
                    crate::processing::retention::cancelled(value, result.applied_text.as_deref())
                        .map_err(|_| crate::contracts::dictionary::DataError::new("invalid"))?
                {
                    retained.push(value);
                }
            } else {
                settings(&self.data, DbOperation::SaveUnit(value)).await?;
            }
        }
        if report.cancelled {
            settings(
                &self.data,
                DbOperation::Cancel {
                    id: run.metadata.id.clone(),
                    text: report.text.clone(),
                    raw: report.raw.clone(),
                    units: retained,
                },
            )
            .await?;
            return Ok(true);
        }
        // Read back the same run before terminal settlement. Both source and
        // generated records remain linked to this attempt; no restart replay.
        let DbOutput::Runs(runs) =
            settings(&self.data, DbOperation::Runs(run.record_id.clone())).await?
        else {
            return Err(crate::contracts::dictionary::DataError::new("unavailable"));
        };
        let saved = runs
            .iter()
            .find(|r| r.id == run.metadata.id)
            .ok_or(crate::contracts::dictionary::DataError::new("notFound"))?;
        let text = if report.cancelled {
            report.text.clone()
        } else {
            crate::processing::render::text(sources, &saved.units)
        };
        let state = if report.cancelled {
            "cancelled"
        } else if report.error.is_some() || report.units.iter().any(|u| u.generation == "failed") {
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
        Ok(true)
    }
}
