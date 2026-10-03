pub mod commands;
mod credentials;
pub mod provider;
mod repository;
#[cfg(test)]
mod tests;
pub mod types;
use crate::data::{DataWorker, Operation, Output};
use credentials::{Vault, WindowsCredentials};
use provider::{Provider, TestResult};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::Mutex;
use types::*;
pub struct ModelService {
    data: DataWorker,
    vault: Vault,
    pub provider: Arc<Provider>,
    gate: Mutex<()>,
    ready: AtomicBool,
    cleanup_pending: AtomicBool,
    stopping: AtomicBool,
}
impl ModelService {
    pub fn new(
        data: DataWorker,
        namespace: String,
        root: &std::path::Path,
    ) -> Result<Self, ModelError> {
        let _ = root;
        let provider = Provider::new()?;
        #[cfg(feature = "desktop-acceptance")]
        let provider = if std::env::var("SAYWEAVE_DATA_ACCEPTANCE_ID")
            .is_ok_and(|id| uuid::Uuid::parse_str(&id).is_ok())
        {
            match std::fs::read(root.join("fixture-ca.der")) {
                Ok(cert) => Provider::with_client(
                    Provider::builder()
                        .no_proxy()
                        .tls_certs_only([reqwest::Certificate::from_der(&cert)
                            .map_err(|_| ModelError::new("unavailable"))?])
                        .build()
                        .map_err(|_| ModelError::new("unavailable"))?,
                )?,
                Err(_) => provider,
            }
        } else {
            provider
        };
        Self::with_vault(
            data,
            Vault::new(WindowsCredentials::new(namespace)),
            Arc::new(provider),
        )
    }
    fn with_vault(
        data: DataWorker,
        vault: Vault,
        provider: Arc<Provider>,
    ) -> Result<Self, ModelError> {
        Ok(Self {
            data,
            vault,
            provider,
            gate: Mutex::new(()),
            ready: AtomicBool::new(false),
            cleanup_pending: AtomicBool::new(false),
            stopping: AtomicBool::new(false),
        })
    }
    fn check(&self, caller: &str) -> Result<(), ModelError> {
        authorize(caller)?;
        if self.stopping.load(Ordering::Acquire) {
            return Err(ModelError::new("stopping"));
        }
        if !self.ready.load(Ordering::Acquire) {
            return Err(ModelError::new("preparing"));
        }
        Ok(())
    }
    async fn db(&self, op: DbOperation) -> Result<DbOutput, ModelError> {
        match self
            .data
            .request_admitted(Operation::Models(op))
            .await
            .map_err(ModelError::from)?
        {
            Output::Models(result) => result,
            _ => Err(ModelError::new("unavailable")),
        }
    }
    async fn stored(&self) -> Result<StoredOverview, ModelError> {
        match self.db(DbOperation::Overview).await? {
            DbOutput::Overview(v) => Ok(v),
            _ => Err(ModelError::new("unavailable")),
        }
    }
    // All callers hold gate; DB must be available before any orphan deletion.
    async fn recover(&self) -> Result<bool, ModelError> {
        let refs = match self.db(DbOperation::Pending).await? {
            DbOutput::Pending(v) => v,
            _ => return Err(ModelError::new("unavailable")),
        };
        let mut pending = false;
        for reference in refs {
            if uuid::Uuid::parse_str(&reference).is_err() {
                pending = true;
                continue;
            }
            match self.db(DbOperation::Referenced(reference.clone())).await? {
                DbOutput::Referenced(true) => {
                    self.db(DbOperation::Clear(reference)).await?;
                }
                DbOutput::Referenced(false) => {
                    if self.vault.delete(reference.clone()).await.is_ok() {
                        self.db(DbOperation::Clear(reference)).await?;
                    } else {
                        pending = true;
                    }
                }
                _ => return Err(ModelError::new("unavailable")),
            }
        }
        self.cleanup_pending.store(pending, Ordering::Release);
        Ok(pending)
    }
    pub async fn status(&self, caller: &str, retry: bool) -> Result<ModelStatus, ModelError> {
        authorize(caller)?;
        if self.stopping.load(Ordering::Acquire) {
            return Ok(ModelStatus {
                state: "stopping".into(),
                schema_version: self.data.status()?.schema_version,
                cleanup_pending: self.cleanup_pending.load(Ordering::Acquire),
            });
        }
        let _guard = self.gate.try_lock().map_err(|_| ModelError::new("busy"))?;
        if retry || !self.ready.load(Ordering::Acquire) {
            if retry {
                self.data.request("main", Operation::Retry).await?;
            }
            if self.data.status()?.state == "ready" {
                self.recover().await?;
                self.ready.store(true, Ordering::Release);
            }
        }
        let data = self.data.status()?;
        Ok(ModelStatus {
            state: if self.ready.load(Ordering::Acquire) && data.state == "ready" {
                "ready".into()
            } else {
                data.state
            },
            schema_version: data.schema_version,
            cleanup_pending: self.cleanup_pending.load(Ordering::Acquire),
        })
    }
    async fn public(&self, view: StoredOverview) -> Overview {
        let mut configs = vec![];
        for c in view.configs {
            let credential_status = match self.vault.get(c.credential_ref).await {
                Ok(_) => "present",
                Err(e) if e.code == "credentialMissing" => "missing",
                Err(_) => "unavailable",
            };
            configs.push(ConfigRecord {
                id: c.id,
                input: c.input,
                credential_status: credential_status.into(),
                revision: c.revision,
                created_at: c.created_at,
                updated_at: c.updated_at,
            });
        }
        Overview {
            configs,
            defaults: view.defaults,
            cleanup_pending: self.cleanup_pending.load(Ordering::Acquire),
        }
    }
    pub async fn overview(&self, caller: &str) -> Result<Overview, ModelError> {
        self.check(caller)?;
        let _guard = self.gate.try_lock().map_err(|_| ModelError::new("busy"))?;
        Ok(self.public(self.stored().await?).await)
    }
    fn existing<'a>(
        view: &'a StoredOverview,
        id: Option<&str>,
        rev: Option<i64>,
    ) -> Result<Option<&'a StoredConfig>, ModelError> {
        match (id, rev) {
            (Some(id), Some(rev)) => {
                let c = view
                    .configs
                    .iter()
                    .find(|c| c.id == id)
                    .ok_or(ModelError::new("notFound"))?;
                if c.revision != rev {
                    return Err(ModelError::new("conflict"));
                }
                Ok(Some(c))
            }
            (None, None) => Ok(None),
            _ => Err(ModelError::new("invalid")),
        }
    }
    async fn resolve(
        &self,
        input: &ConfigInput,
        key: KeyChange,
        old: Option<&StoredConfig>,
    ) -> Result<String, ModelError> {
        match key {
            KeyChange::Replace { value } => secret(&value),
            KeyChange::Keep => {
                let old = old.ok_or(ModelError::new("credentialMissing"))?;
                if !same_origin(&old.input.base_url, &input.base_url) {
                    return Err(ModelError::new("originChanged"));
                }
                self.vault
                    .get(old.credential_ref.clone())
                    .await
                    .and_then(|v| secret(&v))
            }
        }
    }
    pub async fn save(&self, caller: &str, req: SaveRequest) -> Result<Overview, ModelError> {
        self.check(caller)?;
        let _guard = self.gate.try_lock().map_err(|_| ModelError::new("busy"))?;
        self.check(caller)?;
        let input = req.input.validate()?;
        let view = self.stored().await?;
        let old = Self::existing(&view, req.id.as_deref(), req.revision)?;
        let credential_ref = match req.key {
            KeyChange::Keep => {
                self.resolve(&input, KeyChange::Keep, old).await?;
                old.unwrap().credential_ref.clone()
            }
            KeyChange::Replace { value } => {
                let value = secret(&value)?;
                let reference = uuid::Uuid::new_v4().to_string();
                self.db(DbOperation::Prepare(reference.clone())).await?;
                if let Err(e) = self.vault.put(reference.clone(), value).await {
                    let _ = self.recover().await;
                    return Err(e);
                }
                reference
            }
        };
        let result = self
            .db(DbOperation::Commit {
                input,
                id: req.id,
                revision: req.revision,
                expectation: req.expectation,
                credential_ref,
            })
            .await;
        // Commit success stays success even if cleanup needs retry.
        if self.recover().await.is_err() {
            self.cleanup_pending.store(true, Ordering::Release);
        }
        match result? {
            DbOutput::Overview(view) => Ok(self.public(view).await),
            _ => Err(ModelError::new("unavailable")),
        }
    }
    pub async fn remove(&self, caller: &str, req: RemoveRequest) -> Result<Overview, ModelError> {
        self.check(caller)?;
        let _guard = self.gate.try_lock().map_err(|_| ModelError::new("busy"))?;
        self.check(caller)?;
        let view = match self.db(DbOperation::Remove(req)).await? {
            DbOutput::Overview(v) => v,
            _ => return Err(ModelError::new("unavailable")),
        };
        if self.recover().await.is_err() {
            self.cleanup_pending.store(true, Ordering::Release);
        }
        Ok(self.public(view).await)
    }
    pub async fn set_default(
        &self,
        caller: &str,
        req: DefaultRequest,
    ) -> Result<Overview, ModelError> {
        self.check(caller)?;
        let _guard = self.gate.try_lock().map_err(|_| ModelError::new("busy"))?;
        self.check(caller)?;
        match self.db(DbOperation::SetDefault(req)).await? {
            DbOutput::Overview(v) => Ok(self.public(v).await),
            _ => Err(ModelError::new("unavailable")),
        }
    }
    async fn draft(
        &self,
        caller: &str,
        req: NetworkDraft,
    ) -> Result<(ConfigInput, String), ModelError> {
        self.check(caller)?;
        let _guard = self.gate.try_lock().map_err(|_| ModelError::new("busy"))?;
        self.check(caller)?;
        let mut input = req.input;
        if input.protocol != "openai-chat" {
            return Err(ModelError::field("protocol"));
        }
        input.base_url = base_url(&input.base_url)?;
        let view = self.stored().await?;
        let old = Self::existing(&view, req.id.as_deref(), req.revision)?;
        let key = self.resolve(&input, req.key, old).await?;
        Ok((input, key))
    }
    pub async fn fetch(
        &self,
        caller: &str,
        req: NetworkDraft,
        id: String,
    ) -> Result<Vec<String>, ModelError> {
        self.check(caller)?;
        let mut guard = self.provider.register(&id)?;
        guard
            .run(15, async {
                let (input, key) = self.draft(caller, req).await?;
                self.provider.models(&input, &key).await
            })
            .await
    }
    pub async fn test(
        &self,
        caller: &str,
        req: NetworkDraft,
        id: String,
    ) -> Result<TestResult, ModelError> {
        self.check(caller)?;
        let mut guard = self.provider.register(&id)?;
        guard
            .run(30, async {
                let (mut input, key) = self.draft(caller, req).await?;
                input.model_id = text(&input.model_id, 256, "modelId")?;
                self.provider.text(&input, &key).await
            })
            .await
    }
    pub fn begin_shutdown(&self) {
        self.data.begin_shutdown();
        self.stopping.store(true, Ordering::Release);
        self.provider.shutdown();
    }
    pub async fn finish_shutdown(&self) {
        let _guard = self.gate.lock().await;
        self.data.shutdown().await;
    }
}
