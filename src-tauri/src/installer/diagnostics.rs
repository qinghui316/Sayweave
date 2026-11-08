use super::{metadata, record};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use windows::Win32::UI::Shell::FOLDERID_LocalAppData;

const MAX_BYTES: u64 = 8192;
const RETAIN: usize = 10;
pub const ATTEMPT_ENV: &str = "SAYWEAVE_UPDATE_ATTEMPT";
pub const ORIGIN_ENV: &str = "SAYWEAVE_INSTALL_UPDATE";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Attempt {
    pub id: String,
    pub started_ms: u128,
    pub package: String,
    pub scope: Option<String>,
    pub installer_pid: Option<u32>,
    pub installer_created: Option<u64>,
    pub from: Option<String>,
    pub to: String,
    pub stage: String,
    pub failure_stage: Option<String>,
    pub error: Option<String>,
    pub rollback: Option<String>,
    pub installed: bool,
    pub restart: Option<String>,
}

fn root() -> record::Result<PathBuf> {
    let path = metadata::folder(&FOLDERID_LocalAppData)?
        .join("SayweaveInstallTransactions")
        .join("diagnostics");
    record::reject_links(&path)?;
    Ok(path)
}

fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok_and(|value| value.to_string() == id)
        || id.strip_prefix("legacy-").is_some_and(|pair| {
            pair.split_once('-').is_some_and(|(pid, created)| {
                pid.parse::<u32>().is_ok() && created.parse::<u64>().is_ok()
            })
        })
}

fn token(value: &str) -> bool {
    !value.is_empty() && value.len() <= 64 && value.bytes().all(|v| v.is_ascii_alphanumeric())
}

fn version(value: &str) -> bool {
    value.len() <= 32
        && value.split('.').count() == 3
        && value.split('.').all(|part| {
            part.bytes().all(|byte| byte.is_ascii_digit()) && part.parse::<u32>().is_ok()
        })
}

impl Attempt {
    pub fn new(id: String, package: String, from: Option<String>, to: String) -> Self {
        Self {
            id,
            started_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            package,
            scope: None,
            installer_pid: None,
            installer_created: None,
            from,
            to,
            stage: "prepare".into(),
            failure_stage: None,
            error: None,
            rollback: None,
            installed: false,
            restart: None,
        }
    }

    pub fn for_installer(process: &record::Process, to: &str) -> Option<Self> {
        if std::env::var(ORIGIN_ENV).as_deref() != Ok("1") {
            return None;
        }
        let id = std::env::var(ATTEMPT_ENV)
            .ok()
            .filter(|value| uuid::Uuid::parse_str(value).is_ok_and(|id| id.to_string() == *value))
            .unwrap_or_else(|| format!("legacy-{}-{}", process.pid, process.created));
        let mut attempt = load(&id, &process.image_hash)
            .filter(|attempt| {
                attempt.to == to
                    && (attempt.installer_pid.is_none() || attempt.matches_installer(process))
            })
            .unwrap_or_else(|| Self::new(id, process.image_hash.clone(), None, to.into()));
        attempt.installer_pid = Some(process.pid);
        attempt.installer_created = Some(process.created);
        Some(attempt)
    }

    pub fn matches_installer(&self, process: &record::Process) -> bool {
        self.installer_pid == Some(process.pid)
            && self.installer_created == Some(process.created)
            && self.package == process.image_hash
    }

    fn valid(&self) -> bool {
        valid_id(&self.id)
            && (self.package.is_empty() || record::hex(&self.package, 64))
            && self
                .scope
                .as_deref()
                .is_none_or(|scope| record::hex(scope, 64))
            && self.from.as_deref().is_none_or(version)
            && version(&self.to)
            && token(&self.stage)
            && [
                &self.failure_stage,
                &self.error,
                &self.rollback,
                &self.restart,
            ]
            .into_iter()
            .all(|field| field.as_deref().is_none_or(token))
    }

    pub fn fail(&mut self, stage: &str, code: &str) {
        if self.error.is_none() {
            self.failure_stage = Some(stage.into());
            let code = code
                .split(':')
                .next()
                .filter(|code| token(code))
                .unwrap_or("unknown");
            self.error = Some(code.into());
        }
    }

    pub fn save(&self) {
        if let Err(error) = root().and_then(|root| self.save_at(&root)) {
            log::warn!(target: "sayweave", "operation=update_diagnostic_write error={error}");
        }
    }

    fn save_at(&self, root: &std::path::Path) -> record::Result<()> {
        if !self.valid() {
            return Err("diagnosticInvalid".into());
        }
        let bytes = serde_json::to_vec(self).map_err(|_| "diagnosticInvalid")?;
        record::write(&root.join(format!("{}.json", self.id)), &bytes)?;
        let mut entries = inventory(root);
        entries.sort_by(|a, b| {
            b.0.started_ms
                .cmp(&a.0.started_ms)
                .then_with(|| b.0.id.cmp(&a.0.id))
        });
        for (_, path) in entries.into_iter().skip(RETAIN) {
            let _ = fs::remove_file(path);
        }
        Ok(())
    }

    pub fn summary(&self) -> &'static str {
        if self.installed {
            return match self.restart.as_deref() {
                Some("confirmed") => "更新已安装，已自动打开新版。",
                Some("requested" | "failed") => "更新已安装。若新版未打开，请从开始菜单打开。",
                _ => "更新已安装。",
            };
        }
        if self.rollback.as_deref() == Some("failed") {
            return "更新恢复未完成，请重新运行完整安装包。";
        }
        match self.error.as_deref() {
            Some("installerElevationCancelled") => "已取消管理员确认，可以重试。",
            Some(
                "installerElevationDenied"
                | "installerElevationFailed"
                | "installerMaintenanceFailed",
            ) => "无法启动安装维护，请确认 Windows 管理员授权后重试。",
            Some("installerAppRunning") => "Sayweave 仍在运行，请从托盘退出后重试。",
            Some("installerWorkerRunning") => "本地识别仍在退出，请稍后重试。",
            Some("installerFileInUse") => "安装文件正在使用，请关闭占用程序后重试。",
            Some("installerOriginalPathRequired") => "请在原安装目录中更新。",
            Some(_) => "上次更新未完成，可以重试。",
            None => "上次更新结果尚未确认。",
        }
    }

    pub fn shareable(&self) -> String {
        format!(
            "Sayweave update\nattempt: {}\ntime (Unix ms): {}\nversion: {} -> {}\npackage SHA-256: {}\nstage: {}\nfailure stage: {}\ncode: {}\nrollback: {}\ninstalled: {}\nrestart: {}",
            self.id,
            self.started_ms,
            self.from.as_deref().unwrap_or("unknown"),
            self.to,
            if self.package.is_empty() {
                "unverified"
            } else {
                &self.package
            },
            self.stage,
            self.failure_stage.as_deref().unwrap_or("none"),
            self.error.as_deref().unwrap_or("none"),
            self.rollback.as_deref().unwrap_or("none"),
            self.installed,
            self.restart.as_deref().unwrap_or("none"),
        )
    }
}

fn read(path: &std::path::Path) -> Option<Attempt> {
    let attempt: Attempt = serde_json::from_slice(&record::read(path, MAX_BYTES).ok()?).ok()?;
    (attempt.valid() && path.file_stem()?.to_str()? == attempt.id).then_some(attempt)
}

fn inventory(root: &std::path::Path) -> Vec<(Attempt, PathBuf)> {
    fs::read_dir(root)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()?.to_str()? != "json" {
                return None;
            }
            Some((read(&path)?, path))
        })
        .collect()
}

pub fn load(id: &str, package: &str) -> Option<Attempt> {
    if !valid_id(id) {
        return None;
    }
    read(&root().ok()?.join(format!("{id}.json"))).filter(|attempt| attempt.package == package)
}

pub fn latest(package: &str, scope: &str, installer: Option<&record::Process>) -> Option<Attempt> {
    inventory(&root().ok()?)
        .into_iter()
        .map(|(attempt, _)| attempt)
        .filter(|attempt| {
            attempt.package == package
                && attempt.scope.as_deref() == Some(scope)
                && installer.is_none_or(|process| attempt.matches_installer(process))
        })
        .max_by_key(|attempt| attempt.started_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retains_distinct_attempts_and_preserves_the_first_failure() {
        let directory = tempfile::tempdir().unwrap();
        let package = "a".repeat(64);
        for index in 0..12 {
            let mut attempt = Attempt::new(
                uuid::Uuid::new_v4().to_string(),
                package.clone(),
                Some("0.1.23".into()),
                "0.1.24".into(),
            );
            attempt.started_ms = index;
            attempt.fail("begin", "installerElevationCancelled:C:\\private");
            attempt.rollback = Some("restored".into());
            attempt.restart = Some("requested".into());
            attempt.fail("restart", "installerRestartFailed");
            attempt.save_at(directory.path()).unwrap();
        }
        let mut attempts = inventory(directory.path());
        attempts.sort_by_key(|(attempt, _)| attempt.started_ms);
        assert_eq!(attempts.len(), RETAIN);
        assert_eq!(attempts[0].0.started_ms, 2);
        let attempt = &attempts[9].0;
        assert_eq!(
            attempt.error.as_deref(),
            Some("installerElevationCancelled")
        );
        assert_eq!(attempt.failure_stage.as_deref(), Some("begin"));
        assert!(!attempt.shareable().contains("private"));
        assert_ne!(attempt.id, attempts[8].0.id);
    }

    #[test]
    fn installation_and_window_confirmation_are_separate() {
        let mut attempt = Attempt::new(
            uuid::Uuid::new_v4().to_string(),
            "b".repeat(64),
            None,
            "0.1.24".into(),
        );
        attempt.installed = true;
        attempt.restart = Some("requested".into());
        assert!(attempt.summary().contains("从开始菜单打开"));
        attempt.restart = Some("confirmed".into());
        assert!(attempt.summary().contains("已自动打开新版"));
    }
}
