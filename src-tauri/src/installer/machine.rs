//! Elevated component transaction: fixed payload, fixed namespace, no user files/HKCU writes.
use super::{
    ipc,
    record::{Phase, Process, Result, Target},
};
use sayweave_text_service::{deployment, registration, transport::Pipe};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MachineRecord {
    pub schema_version: u8,
    pub transaction: String,
    pub phase: Phase,
    pub target: Target,
    pub installer: Process,
    pub server: Process,
    pub language: u16,
    pub old: Option<registration::InstallerRegistration>,
    pub old_hash: Option<String>,
    pub module: String,
    pub error: Option<String>,
}
impl MachineRecord {
    pub fn validate(&self) -> Result<()> {
        self.target.validate()?;
        if self.schema_version != 1
            || uuid::Uuid::parse_str(&self.transaction).is_err()
            || self.language == 0
            || self.installer.sid.is_empty()
            || self.module.len() > 4096
        {
            return Err("installerMachineRecord".into());
        }
        if self.old.as_ref().is_some_and(|v| v.module.len() > 4096)
            || self
                .old_hash
                .as_ref()
                .is_some_and(|h| !super::record::hex(h, 64))
        {
            return Err("installerMachineRecord".into());
        }
        Ok(())
    }
    fn save(&self) -> Result<()> {
        self.validate()?;
        deployment::write_installer_record(
            &serde_json::to_vec(self).map_err(|_| "installerMachineRecord")?,
        )
        .map_err(|_| "installerMachineWrite".into())
    }
    fn verify_module(&self) -> Result<()> {
        if registration::owned_module()
            .map_err(|_| "componentRegistration")?
            .as_deref()
            != Some(self.module.as_str())
            || deployment::installer_module_hash(&self.module).map_err(|_| "componentIdentity")?
                != self.target.payload_hash
        {
            return Err("componentIdentity".into());
        }
        Ok(())
    }
    fn rollback(&mut self) -> Result<()> {
        if self.phase == Phase::Committed {
            return Err("installerAlreadyCommitted".into());
        }
        if let (Some(before), Some(hash)) = (&self.old, &self.old_hash)
            && deployment::installer_module_hash(&before.module)
                .map_err(|_| "componentOldIdentity")?
                != *hash
        {
            return Err("componentOldIdentity".into());
        }
        registration::restore_installer_machine(self.old.as_ref(), &self.module)
            .map_err(|_| "componentRollback")?;
        self.phase = Phase::RolledBack;
        self.error = None;
        self.save()
    }
}
pub fn load() -> Result<Option<MachineRecord>> {
    let Some(bytes) = deployment::installer_record().map_err(|_| "installerMachineRead")? else {
        return Ok(None);
    };
    let value: MachineRecord =
        serde_json::from_slice(&bytes).map_err(|_| "installerMachineRecord")?;
    value.validate()?;
    Ok(Some(value))
}
pub fn deployment_state(payload: &[u8]) -> &'static str {
    match load() {
        Ok(Some(record)) if !record.phase.terminal() => {
            if record.phase == Phase::Recovery {
                "failed"
            } else {
                "pending"
            }
        }
        Err(_) => "failed",
        _ => {
            if !payload.is_empty() && matches!(deployment::matches_payload(payload), Ok(Some(true)))
            {
                "ready"
            } else {
                "required"
            }
        }
    }
}
pub fn serve(
    transaction: &str,
    installer: Process,
    language: u16,
    recover: bool,
    main_source: &std::path::Path,
) -> Result<()> {
    super::metadata::initialize_apartment()?;
    deployment::authorize_installer().map_err(|_| "installerAdministrator")?;
    ipc::allow_initiator_query(&installer.sid)?;
    if !ipc::live(&installer) || ipc::process(installer.pid)? != installer {
        return Err("installerProcessChanged".into());
    }
    let server = ipc::process(std::process::id())?;
    let _lock = super::native::MachineLock::acquire()?;
    let pipe = ipc::server(transaction, &installer.sid)?;
    let mut record = if recover {
        let mut prior = load()?.ok_or("installerRecoveryMissing")?;
        if prior.transaction != transaction
            || prior.installer.sid != installer.sid
            || prior.phase == Phase::Committed
        {
            return Err("installerRecoveryConflict".into());
        }
        if ipc::live(&prior.installer) && prior.installer != installer {
            return Err("installerBusy".into());
        }
        prior.installer = installer;
        prior.server = server;
        prior.save()?;
        prior
    } else {
        if let Some(prior) = load()?
            && !prior.phase.terminal()
        {
            return Err("installerRecoveryRequired".into());
        }
        let target = super::native::target(main_source)?;
        let old =
            registration::snapshot_installer_machine().map_err(|_| "componentRegistration")?;
        let old_hash = old
            .as_ref()
            .map(|old| {
                deployment::installer_module_hash(&old.module).map_err(|_| "componentOldIdentity")
            })
            .transpose()?;
        let module = deployment::prepare_installer(crate::text_service::payload())
            .map_err(|_| "componentPrepare")?;
        if deployment::installer_module_hash(&module).map_err(|_| "componentIdentity")?
            != target.payload_hash
        {
            return Err("componentIdentity".into());
        }
        let value = MachineRecord {
            schema_version: 1,
            transaction: transaction.into(),
            phase: Phase::Prepared,
            target,
            installer,
            server,
            language,
            old,
            old_hash,
            module,
            error: None,
        };
        value.save()?;
        value
    };
    let began = Instant::now();
    loop {
        if !ipc::live(&record.installer) || began.elapsed() > Duration::from_secs(15 * 60) {
            if let Err(error) = record.rollback() {
                record.phase = Phase::Recovery;
                record.error = Some(error.clone());
                record.save()?;
                return Err(error);
            }
            return Ok(());
        }
        if !pipe.connect() {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        let result = handle(&pipe, &mut record, recover);
        let reply = match &result {
            Ok(()) => serde_json::json!({"transaction":record.transaction,"phase":record.phase}),
            Err(error) => serde_json::json!({"error":error}),
        };
        // A disconnected caller must not abandon a still-owned machine change.
        // Keep the server/anchor until retry, installer death, or the bounded timeout.
        if pipe
            .send(&serde_json::to_vec(&reply).map_err(|_| "installerPipeProtocol")?)
            .is_err()
        {
            pipe.disconnect();
            if record.phase.terminal() {
                return result;
            }
            continue;
        }
        let acknowledgment_deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < acknowledgment_deadline && pipe.alive() {
            if pipe.receive().ok().flatten().is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        pipe.disconnect();
        if record.phase.terminal() {
            return result;
        }
    }
}
fn handle(pipe: &Pipe, record: &mut MachineRecord, recover: bool) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let bytes = loop {
        if let Some(bytes) = pipe.receive().map_err(|_| "installerPipeRead")? {
            break bytes;
        }
        if Instant::now() > deadline || !ipc::live(&record.installer) {
            return Err("installerPipeTimeout".into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let peer = ipc::process(pipe.client_pid().map_err(|_| "installerPipeIdentity")?)?;
    if peer.sid != record.installer.sid
        || peer.session != record.installer.session
        || peer.image_hash != record.server.image_hash
    {
        return Err("installerPipeIdentity".into());
    }
    let request: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "installerPipeProtocol")?;
    if request["transaction"].as_str() != Some(record.transaction.as_str())
        || request.as_object().is_none_or(|v| v.len() != 2)
    {
        return Err("installerPipeProtocol".into());
    }
    match request["operation"].as_str() {
        Some("prepared") => Ok(()),
        Some("commit") if !recover => {
            if record.phase == Phase::Prepared {
                deployment::commit_installer(
                    &record.module,
                    record.old.as_ref().map(|v| v.module.as_str()),
                    record.language,
                )
                .map_err(|_| "componentCommit")?;
                record.phase = Phase::MachineCommitted;
                record.save()?;
            }
            record.verify_module()
        }
        Some("finalize") if !recover && record.phase == Phase::MachineCommitted => {
            record.verify_module()?;
            record.phase = Phase::Committed;
            record.save()
        }
        Some("rollback") => record.rollback(),
        _ => Err("installerOperationInvalid".into()),
    }
}
