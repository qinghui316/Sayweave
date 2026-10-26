//! Private, bounded IPC for compute only; it carries no credentials or input permission.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::io::{self, Read, Write};

pub const VERSION: u32 = 2;
pub const MAX_JSON: usize = 1024 * 1024;
pub const MAX_PCM_SAMPLES: usize = 32_000;
pub fn runtime_file_names() -> &'static [String] {
    static FILES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    FILES.get_or_init(|| {
        serde_json::from_str(include_str!("../runtime-files.json"))
            .expect("bundled runtime inventory")
    })
}
pub fn failure_code(code: &str) -> &'static str {
    match code {
        "cancelled" => "cancelled",
        "localBackendUnavailable" => "localBackendUnavailable",
        "localMemoryUnavailable" => "localMemoryUnavailable",
        "localModelMissing" => "localModelMissing",
        "localModelUnreadable" => "localModelUnreadable",
        "localModelCorrupt" => "localModelCorrupt",
        "localStreamingUnsupported" => "localStreamingUnsupported",
        "localConfirmationUnsupported" => "localConfirmationUnsupported",
        "localRuntimeMismatch" => "localRuntimeMismatch",
        "localLanguageUnsupported" => "localLanguageUnsupported",
        "localBufferOverflow" => "localBufferOverflow",
        "localAudioSequence" => "localAudioSequence",
        "localStaleOperation" => "localStaleOperation",
        "localBusy" => "localBusy",
        "localProtocol" => "localProtocol",
        "localSourceDiverged" | "localSourceChanged" => "localSourceChanged",
        _ => "localInferenceFailed",
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunId {
    pub task: String,
    pub turn: u64,
    pub request: String,
}
impl RunId {
    pub fn is_valid(&self) -> bool {
        self.turn > 0
            && self.turn <= 256
            && [&self.task, &self.request]
                .iter()
                .all(|s| !s.is_empty() && s.len() <= 128 && !s.chars().any(char::is_control))
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum DevicePolicy {
    Auto,
    Cpu,
}

/// A pause in accepted PCM, never a source confirmation or end-of-stream.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PauseBoundary {
    pub sequence: u64,
    pub sample_end: u64,
    pub native_revision: i32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Command {
    Hello {
        version: u32,
        instance: String,
    },
    Prepare {
        id: RunId,
        path: String,
        sha256: String,
        device: DevicePolicy,
    },
    Start {
        id: RunId,
        language: String,
        right_context: i32,
    },
    Finish {
        id: RunId,
    },
    Cancel {
        id: RunId,
    },
    Unload,
    Shutdown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Event {
    Hello {
        version: u32,
        instance: String,
        engine: String,
    },
    Prepared {
        id: RunId,
        backend: String,
        device: Option<String>,
        device_id: Option<String>,
        device_memory_total: Option<u64>,
        device_memory_free: Option<u64>,
        warmup_ms: u64,
        streaming: bool,
    },
    Ready {
        id: RunId,
    },
    Update {
        id: RunId,
        revision: i32,
        full: String,
        confirmed: String,
        committed: String,
        tentative: String,
        native_token_count: usize,
        audio_received_ms: i64,
        audio_committed_ms: i64,
        pause: Option<PauseBoundary>,
        final_result: bool,
    },
    Finished {
        id: RunId,
    },
    Cancelled {
        id: RunId,
    },
    Unloaded,
    Error {
        id: Option<RunId>,
        code: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioHeader {
    id: RunId,
    sequence: u64,
}

pub enum Frame {
    Json(Vec<u8>),
    Audio {
        id: RunId,
        sequence: u64,
        pcm: Vec<i16>,
    },
}

fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid local ASR frame")
}

pub fn write_json(writer: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|_| invalid())?;
    if bytes.len() > MAX_JSON {
        return Err(invalid());
    }
    write_frame(writer, 0, &bytes)
}

pub fn decode_json<T: DeserializeOwned>(frame: Frame) -> io::Result<T> {
    let Frame::Json(bytes) = frame else {
        return Err(invalid());
    };
    serde_json::from_slice(&bytes).map_err(|_| invalid())
}

pub fn write_audio(
    writer: &mut impl Write,
    id: &RunId,
    sequence: u64,
    pcm: &[i16],
) -> io::Result<()> {
    if pcm.len() > MAX_PCM_SAMPLES {
        return Err(invalid());
    }
    let header = serde_json::to_vec(&AudioHeader {
        id: id.clone(),
        sequence,
    })
    .map_err(|_| invalid())?;
    if header.len() > 4096 {
        return Err(invalid());
    }
    let mut bytes = Vec::with_capacity(2 + header.len() + pcm.len() * 2);
    bytes.extend_from_slice(&(header.len() as u16).to_le_bytes());
    bytes.extend_from_slice(&header);
    for sample in pcm {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    write_frame(writer, 1, &bytes)
}

fn write_frame(writer: &mut impl Write, tag: u8, bytes: &[u8]) -> io::Result<()> {
    writer.write_all(&((bytes.len() + 1) as u32).to_le_bytes())?;
    writer.write_all(&[tag])?;
    writer.write_all(bytes)?;
    writer.flush()
}

pub fn read_frame(reader: &mut impl Read) -> io::Result<Option<Frame>> {
    let mut size = [0; 4];
    // EOF is normal only before the next frame, never halfway through one.
    if reader.read(&mut size[..1])? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut size[1..])?;
    let size = u32::from_le_bytes(size) as usize;
    if size == 0 || size > MAX_JSON + 1 {
        return Err(invalid());
    }
    let mut bytes = vec![0; size];
    reader.read_exact(&mut bytes)?;
    match bytes[0] {
        0 => Ok(Some(Frame::Json(bytes[1..].to_vec()))),
        1 => {
            if bytes.len() < 3 {
                return Err(invalid());
            }
            let header_size = u16::from_le_bytes([bytes[1], bytes[2]]) as usize;
            let start = 3_usize.checked_add(header_size).ok_or_else(invalid)?;
            if header_size > 4096 || start > bytes.len() {
                return Err(invalid());
            }
            let pcm = &bytes[start..];
            if pcm.len() % 2 != 0 || pcm.len() / 2 > MAX_PCM_SAMPLES {
                return Err(invalid());
            }
            let header: AudioHeader =
                serde_json::from_slice(&bytes[3..start]).map_err(|_| invalid())?;
            Ok(Some(Frame::Audio {
                id: header.id,
                sequence: header.sequence,
                pcm: pcm
                    .chunks_exact(2)
                    .map(|s| i16::from_le_bytes([s[0], s[1]]))
                    .collect(),
            }))
        }
        _ => Err(invalid()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    fn id() -> RunId {
        RunId {
            task: "task".into(),
            turn: 3,
            request: "request".into(),
        }
    }
    #[test]
    fn audio_roundtrip_preserves_identity_sequence_and_signed_samples() {
        let mut bytes = Vec::new();
        write_audio(&mut bytes, &id(), 9, &[i16::MIN, -1, 0, i16::MAX]).unwrap();
        let Some(Frame::Audio {
            id: got,
            sequence,
            pcm,
        }) = read_frame(&mut Cursor::new(bytes)).unwrap()
        else {
            panic!("audio");
        };
        assert_eq!(got, id());
        assert_eq!(sequence, 9);
        assert_eq!(pcm, [i16::MIN, -1, 0, i16::MAX]);
    }
    #[test]
    fn json_and_audio_are_distinct_and_eof_is_not_truncation() {
        let mut bytes = Vec::new();
        write_json(&mut bytes, &Command::Finish { id: id() }).unwrap();
        let mut reader = Cursor::new(bytes.clone());
        assert!(
            matches!(decode_json::<Command>(read_frame(&mut reader).unwrap().unwrap()).unwrap(), Command::Finish {id: got} if got == id())
        );
        assert!(read_frame(&mut reader).unwrap().is_none());
        bytes.pop();
        assert!(read_frame(&mut Cursor::new(bytes)).is_err());
    }
    #[test]
    fn allocation_and_audio_limits_are_checked_before_consumption() {
        assert!(read_frame(&mut Cursor::new(((MAX_JSON + 2) as u32).to_le_bytes())).is_err());
        assert!(write_audio(&mut Vec::new(), &id(), 1, &vec![0; MAX_PCM_SAMPLES + 1]).is_err());
        assert!(read_frame(&mut Cursor::new([1, 0, 0, 0, 9])).is_err());
    }
}
