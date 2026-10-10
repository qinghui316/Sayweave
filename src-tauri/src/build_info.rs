use serde::Serialize;
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildInfo {
    pub version: &'static str,
    pub commit: &'static str,
    pub channel: &'static str,
    pub target: &'static str,
    pub updates_enabled: bool,
}
pub fn current() -> BuildInfo {
    let channel = env!("SAYWEAVE_BUILD_CHANNEL");
    BuildInfo {
        version: env!("SAYWEAVE_BUILD_VERSION"),
        commit: env!("SAYWEAVE_BUILD_COMMIT"),
        channel,
        target: env!("SAYWEAVE_BUILD_TARGET"),
        updates_enabled: channel == "stable" || channel == "test",
    }
}
