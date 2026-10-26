use super::{MODEL_BYTES, MODEL_FILE, MODEL_ID, MODEL_SHA256};
#[derive(Clone, Copy)]
pub struct Descriptor {
    pub id: &'static str,
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    pub sources: &'static [&'static str],
}
pub const CATALOG: &[Descriptor] = &[Descriptor {
    id: MODEL_ID,
    file: MODEL_FILE,
    bytes: MODEL_BYTES,
    sha256: MODEL_SHA256,
    sources: &[
        "https://hf-mirror.com/cswk/sayit-asr-gguf/resolve/d686eb53176aa2e136fb80869fddbb41fc5ef3dc/nemotron-3.5-asr-streaming-0.6b-Q4_K_M.gguf",
        "https://huggingface.co/cswk/sayit-asr-gguf/resolve/d686eb53176aa2e136fb80869fddbb41fc5ef3dc/nemotron-3.5-asr-streaming-0.6b-Q4_K_M.gguf",
    ],
}];
