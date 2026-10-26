use super::{ExecutableConfiguration, ModelService, types::*};
use crate::{
    local_asr::{Runtime, TaskLease},
    model_assets::{Assets, ModelLease},
};
use std::sync::Arc;

pub(crate) struct LocalServices {
    pub runtime: Runtime,
    pub assets: Arc<Assets>,
    pub(super) definitions: &'static [AsrDefinition],
}
pub(crate) struct AsrDefinition {
    pub asset_id: &'static str,
    pub engine: &'static str,
    pub right_context: i32,
}
const ASR_DEFINITIONS: &[AsrDefinition] = &[AsrDefinition {
    asset_id: crate::model_assets::MODEL_ID,
    engine: "transcribe-cpp-0.1.3",
    right_context: 6,
}];
impl LocalServices {
    pub fn new(root: &std::path::Path) -> Result<Self, ModelError> {
        let directory = std::env::current_exe()
            .map_err(|_| ModelError::new("localRuntimeMissing"))?
            .parent()
            .ok_or(ModelError::new("localRuntimeMissing"))?
            .to_owned();
        Ok(Self {
            runtime: Runtime::new(
                directory.join("local-asr/sayweave-local-asr-worker.exe"),
                crate::build_info::local_runtime::embedded().map_err(ModelError::new)?,
            ),
            assets: Assets::new(root.join("models")).map_err(ModelError::new)?,
            definitions: ASR_DEFINITIONS,
        })
    }
}
pub struct LocalConfiguration {
    pub record: LocalConfigRecord,
    pub model: Arc<ModelLease>,
    pub runtime: Runtime,
    pub settings: crate::local_asr::StreamSettings,
    _task: Arc<TaskLease>,
}
#[cfg(test)]
impl LocalConfiguration {
    pub(crate) fn fixture(
        record: LocalConfigRecord,
        model: Arc<ModelLease>,
        runtime: Runtime,
    ) -> Self {
        let task = runtime.task_lease().unwrap();
        Self {
            record,
            model,
            runtime,
            _task: task,
            settings: crate::local_asr::StreamSettings::default(),
        }
    }
}
#[derive(Clone)]
pub enum SpeechConfiguration {
    Cloud(Arc<ExecutableConfiguration>),
    Local(Arc<LocalConfiguration>),
}
pub struct SpeechChoice {
    pub configuration: Option<SpeechConfiguration>,
    pub id: String,
    pub revision: i64,
}
impl From<ExecutableConfiguration> for SpeechConfiguration {
    fn from(config: ExecutableConfiguration) -> Self {
        Self::Cloud(Arc::new(config))
    }
}
impl SpeechConfiguration {
    pub fn id(&self) -> &str {
        match self {
            Self::Cloud(c) => &c.id,
            Self::Local(c) => &c.record.id,
        }
    }
    pub fn revision(&self) -> i64 {
        match self {
            Self::Cloud(c) => c.revision,
            Self::Local(c) => c.record.revision,
        }
    }
    pub fn name(&self) -> &str {
        match self {
            Self::Cloud(c) => &c.input.name,
            Self::Local(c) => &c.record.input.name,
        }
    }
    pub fn model_id(&self) -> &str {
        match self {
            Self::Cloud(c) => &c.input.model_id,
            Self::Local(c) => &c.record.input.asset_id,
        }
    }
    pub fn service_host(&self) -> String {
        match self {
            Self::Cloud(c) => reqwest::Url::parse(&c.input.base_url)
                .ok()
                .and_then(|u| u.host_str().map(String::from))
                .unwrap_or_default(),
            Self::Local(_) => String::new(),
        }
    }
    pub fn source(&self) -> crate::contracts::history::RecognitionSource {
        match self {
            Self::Cloud(_) => crate::contracts::history::RecognitionSource::Cloud,
            Self::Local(local) => crate::contracts::history::RecognitionSource::Local {
                asset_id: local.record.input.asset_id.clone(),
                engine: local.record.input.engine.clone(),
            },
        }
    }
    pub async fn prepare(&self, task: &str) -> Result<(), ModelError> {
        if let Self::Local(local) = self {
            local
                .runtime
                .prepare(
                    sayweave_local_asr_protocol::RunId {
                        task: task.into(),
                        turn: 1,
                        request: uuid::Uuid::new_v4().to_string(),
                    },
                    local.model.clone(),
                    local.record.input.device,
                )
                .await
                .map_err(ModelError::new)?;
        }
        Ok(())
    }
}
impl ModelService {
    pub async fn speech_choice(&self) -> Result<SpeechChoice, ModelError> {
        self.status("main", false).await?;
        self.check("main")?;
        let view = self.stored().await?;
        let id = view
            .defaults
            .iter()
            .find(|d| d.usage == Usage::Speech)
            .and_then(|d| d.config_id.as_ref())
            .ok_or(ModelError::new("notConfigured"))?;
        if let Some(record) = view.configs.iter().find(|c| &c.id == id) {
            // Cloud keys/connection stay in their original later preparation stage.
            return Ok(SpeechChoice {
                configuration: None,
                id: record.id.clone(),
                revision: record.revision,
            });
        }
        let record = view
            .local_configs
            .iter()
            .find(|c| &c.id == id)
            .ok_or(ModelError::new("notConfigured"))?;
        let config = self.freeze_speech().await?;
        if config.id() != record.id || config.revision() != record.revision {
            return Err(ModelError::new("conflict"));
        }
        Ok(SpeechChoice {
            configuration: Some(config),
            id: record.id.clone(),
            revision: record.revision,
        })
    }
    pub fn local_assets(&self) -> Result<&Arc<Assets>, ModelError> {
        self.local
            .get()
            .map(|s| &s.assets)
            .ok_or(ModelError::new("localRuntimeMissing"))
    }
    pub(super) fn local_selection(&self, view: &StoredOverview) {
        if let Some(local) = self.local.get() {
            let selected = view
                .defaults
                .iter()
                .find(|d| d.usage == Usage::Speech)
                .and_then(|d| d.config_id.as_ref());
            local
                .runtime
                .select(view.local_configs.iter().any(|c| Some(&c.id) == selected));
        }
    }
    pub async fn save_local(
        &self,
        caller: &str,
        req: LocalSaveRequest,
    ) -> Result<Overview, ModelError> {
        self.check(caller)?;
        let local = self
            .local
            .get()
            .ok_or(ModelError::new("localRuntimeMissing"))?;
        local
            .assets
            .descriptor(&req.input.asset_id)
            .map_err(ModelError::new)?;
        if !local
            .definitions
            .iter()
            .any(|d| d.asset_id == req.input.asset_id && d.engine == req.input.engine)
        {
            return Err(ModelError::field("assetId"));
        }
        let _guard = self.gate.try_lock().map_err(|_| ModelError::new("busy"))?;
        self.check(caller)?;
        match self.db(DbOperation::CommitLocal(req)).await? {
            DbOutput::Overview(view) => Ok(self.public(view).await),
            _ => Err(ModelError::new("unavailable")),
        }
    }
    pub(super) async fn freeze_speech(&self) -> Result<SpeechConfiguration, ModelError> {
        self.status("main", false).await?;
        self.check("main")?;
        let _guard = self.gate.try_lock().map_err(|_| ModelError::new("busy"))?;
        let view = self.stored().await?;
        let id = view
            .defaults
            .iter()
            .find(|d| d.usage == Usage::Speech)
            .and_then(|d| d.config_id.as_deref())
            .ok_or(ModelError::new("notConfigured"))?;
        if let Some(record) = view.local_configs.iter().find(|c| c.id == id) {
            record.input.clone().validate()?;
            let local = self
                .local
                .get()
                .ok_or(ModelError::new("localRuntimeMissing"))?;
            let definition = local
                .definitions
                .iter()
                .find(|d| d.asset_id == record.input.asset_id && d.engine == record.input.engine)
                .ok_or(ModelError::new("localStreamingUnsupported"))?;
            let model = local
                .assets
                .lease(&record.input.asset_id)
                .await
                .map_err(ModelError::new)?;
            local.runtime.select(true);
            return Ok(SpeechConfiguration::Local(Arc::new(LocalConfiguration {
                record: record.clone(),
                model,
                runtime: local.runtime.clone(),
                settings: crate::local_asr::StreamSettings {
                    language: "auto".into(),
                    right_context: definition.right_context,
                },
                _task: local.runtime.task_lease().map_err(ModelError::new)?,
            })));
        }
        let config = view
            .configs
            .iter()
            .find(|c| c.id == id)
            .ok_or(ModelError::new("notConfigured"))?;
        if !executable(&config.input, &Usage::Speech) {
            return Err(ModelError::new("unsupported"));
        }
        if let Some(local) = self.local.get() {
            local.runtime.select(false);
        }
        Ok(ExecutableConfiguration {
            id: config.id.clone(),
            input: config.input.clone(),
            revision: config.revision,
            key: self.vault.get(config.credential_ref.clone()).await?,
        }
        .into())
    }
}
