//! Installer orchestration owns application files; component registration stays in its owner.
//! None of these entries starts Tauri, opens SQLite, or reads Windows credentials.
#[cfg(windows)]
mod ipc;
#[cfg(windows)]
mod machine;
#[cfg(windows)]
mod metadata;
#[cfg(windows)]
mod native;
mod record;

pub fn package_hash(bytes: &[u8]) -> String {
    record::hash(bytes)
}

#[cfg(windows)]
pub(crate) fn legacy_maintenance_guard(payload: &[u8]) -> Result<impl Drop, String> {
    let lock = native::MachineLock::acquire()?;
    let _ = payload;
    if machine::load()
        .map_err(|_| "componentDeploymentPending")?
        .is_some_and(|r| !r.phase.terminal())
    {
        return Err("componentDeploymentPending".into());
    }
    Ok(lock)
}

pub fn helper_entry() -> Option<i32> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if !args
        .first()
        .is_some_and(|v| v.starts_with("--sayweave-installer="))
    {
        return None;
    }
    #[cfg(windows)]
    {
        Some(match native::dispatch(&args) {
            Ok(()) => 0,
            Err(error) => {
                log::error!(target:"sayweave", "installer result=failed code={error}");
                1
            }
        })
    }
    #[cfg(not(windows))]
    {
        Some(2)
    }
}
pub fn deployment_state(payload: &[u8]) -> &'static str {
    #[cfg(windows)]
    {
        machine::deployment_state(payload)
    }
    #[cfg(not(windows))]
    {
        let _ = payload;
        "required"
    }
}
pub fn acknowledge_running(installer_hash: &str) -> bool {
    #[cfg(windows)]
    {
        native::acknowledge_running(installer_hash).unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        let _ = installer_hash;
        false
    }
}
