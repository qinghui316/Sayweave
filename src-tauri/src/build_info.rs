use serde::Serialize;
pub(crate) mod local_runtime;
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
/// Optimizing a developer build must not select the stable data or vault.
pub(crate) fn development_profile() -> bool {
    profile_for(env!("SAYWEAVE_BUILD_CHANNEL"), cfg!(debug_assertions))
}
fn profile_for(channel: &str, debug: bool) -> bool {
    debug || channel == "development"
}
#[cfg(test)]
mod profile_tests {
    use super::*;
    #[test]
    fn optimized_development_keeps_a_separate_profile_and_stable_does_not_move() {
        assert!(profile_for("development", false));
        assert!(profile_for("development", true));
        assert!(!profile_for("stable", false));
        assert!(!profile_for("test", false));
        assert!(profile_for("stable", true));
    }
}
