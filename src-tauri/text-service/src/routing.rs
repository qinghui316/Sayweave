use serde::{Deserialize, Serialize};

pub const MAX_FRAME: usize = 128 * 1024;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EngineBinding {
    pub proxy_instance: String,
    pub engine_generation: u64,
}
impl EngineBinding {
    pub fn valid(&self) -> bool {
        self.engine_generation > 0
            && uuid::Uuid::parse_str(&self.proxy_instance).is_ok_and(|id| !id.is_nil())
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ModuleIdentity {
    pub base: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Runtime {
    pub engine: EngineBinding,
    pub core: Option<ModuleIdentity>,
    pub proxy: Option<ModuleIdentity>,
    pub protocol: u32,
    pub process: u32,
    pub thread: u32,
}
