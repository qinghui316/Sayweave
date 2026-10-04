use super::{
    recognition::{Event, Segment, Transcript as Text},
    transport,
};
use crate::history::types::Error;
use flate2::{Compression, read::GzDecoder, write::GzEncoder};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::io::{Read, Write};
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};

const LIMIT: usize = 1024 * 1024;
const RESOURCES: [&str; 2] = ["volc.seedasr.sauc.duration", "volc.seedasr.sauc.concurrent"];

pub fn endpoint(base: &str) -> Result<String, Error> {
    let mut url = reqwest::Url::parse(base).map_err(|_| Error::new("invalid"))?;
    if url.scheme() != "https"
        || url.path() != "/"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::new("invalid"));
    }
    url.set_scheme("wss").map_err(|_| Error::new("invalid"))?;
    url.set_path("/api/v3/sauc/bigmodel_async");
    Ok(url.to_string())
}

fn encode(kind: u8, flags: u8, json: bool, payload: &[u8]) -> Result<Vec<u8>, Error> {
    if payload.len() > LIMIT {
        return Err(Error::new("responseTooLarge"));
    }
    let mut gzip = GzEncoder::new(Vec::new(), Compression::fast());
    gzip.write_all(payload)
        .map_err(|_| Error::new("protocol"))?;
    let compressed = gzip.finish().map_err(|_| Error::new("protocol"))?;
    if compressed.len() > LIMIT {
        return Err(Error::new("responseTooLarge"));
    }
    let mut frame = vec![0x11, kind | flags, if json { 0x11 } else { 0x01 }, 0];
    frame.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
    frame.extend(compressed);
    Ok(frame)
}
pub fn audio_frame(bytes: &[u8], last: bool) -> Result<Vec<u8>, Error> {
    encode(0x20, if last { 2 } else { 0 }, false, bytes)
}
fn initial_frame() -> Result<Vec<u8>, Error> {
    encode(0x10, 0, true, json!({"user":{"uid":"sayweave"},"audio":{"format":"pcm","codec":"raw","rate":16000,"bits":16,"channel":1},"request":{"model_name":"bigmodel","enable_itn":true,"enable_punc":true,"show_utterances":true,"result_type":"full"}}).to_string().as_bytes())
}
struct Response {
    value: Value,
    last: bool,
    sequence: Option<i32>,
}
fn service_error(code: u64) -> Error {
    log::warn!(target:"sayweave","SAYWEAVE_ASR protocol=doubao stage=service code={code}");
    Error::new(match code {
        45000030 => "resourceUnavailable",
        45000002 => "emptyText",
        45000081 => "timeout",
        55000031 => "rateLimit",
        45000001 | 45000151 => "protocol",
        _ => "service",
    })
}
fn decode(bytes: &[u8]) -> Result<Response, Error> {
    if bytes.len() > LIMIT {
        return Err(Error::new("responseTooLarge"));
    }
    if bytes.len() < 4 || bytes[0] >> 4 != 1 || bytes[0] & 15 == 0 || bytes[3] != 0 {
        return Err(Error::new("protocol"));
    }
    let header = ((bytes[0] & 15) as usize) * 4;
    let kind = bytes[1] >> 4;
    let flags = bytes[1] & 15;
    if ![9, 15].contains(&kind) || flags > 3 || header > bytes.len() {
        return Err(Error::new("protocol"));
    }
    let read_u32 = |at: usize| -> Result<u32, Error> {
        let data = bytes.get(at..at + 4).ok_or(Error::new("protocol"))?;
        Ok(u32::from_be_bytes(
            data.try_into().map_err(|_| Error::new("protocol"))?,
        ))
    };
    let sequence = if kind == 9 && flags & 1 != 0 {
        Some(read_u32(header)? as i32)
    } else {
        None
    };
    let offset = header
        + if kind == 15 || sequence.is_some() {
            4
        } else {
            0
        };
    let size = read_u32(offset)? as usize;
    if size > LIMIT {
        return Err(Error::new("responseTooLarge"));
    }
    if bytes.len() != offset + 4 + size {
        return Err(Error::new("protocol"));
    }
    let payload = &bytes[offset + 4..];
    let decoded = match bytes[2] & 15 {
        0 => payload.to_vec(),
        1 => {
            let mut out = Vec::new();
            GzDecoder::new(payload)
                .take((LIMIT + 1) as u64)
                .read_to_end(&mut out)
                .map_err(|_| Error::new("protocol"))?;
            if out.len() > LIMIT {
                return Err(Error::new("responseTooLarge"));
            }
            out
        }
        _ => return Err(Error::new("protocol")),
    };
    if kind == 15 {
        return Err(service_error(read_u32(header)? as u64));
    }
    if bytes[2] >> 4 != 1 {
        return Err(Error::new("protocol"));
    }
    let value: Value = serde_json::from_slice(&decoded).map_err(|_| Error::new("protocol"))?;
    if !value.is_object() {
        return Err(Error::new("protocol"));
    }
    if let Some(code) = value.get("code") {
        let code = code.as_u64().ok_or(Error::new("protocol"))?;
        if code != 0 && code != 20000000 {
            return Err(service_error(code));
        }
    }
    Ok(Response {
        value,
        // The last-package flag is authoritative; a negative sequence also marks completion.
        // A final positive sequence or a terminal sentinel is not an ordinary update ordinal.
        last: flags & 2 != 0 || sequence.is_some_and(|s| s < 0),
        sequence,
    })
}

#[derive(Default)]
pub struct Transcript {
    text: Text,
    sequence: u64,
    server_sequence: u32,
    pub finished: bool,
}
impl Transcript {
    pub fn event(&mut self, message: Message) -> Result<Event, Error> {
        let Message::Binary(bytes) = message else {
            return Err(Error::new("protocol"));
        };
        let frame_type = bytes.get(1).map(|b| b >> 4);
        let flags = bytes.get(1).map(|b| b & 15);
        let response = decode(&bytes).inspect_err(|e| {
            log::warn!(target:"sayweave","SAYWEAVE_ASR protocol=doubao stage=decode code={} frame_type={frame_type:?} flags={flags:?} bytes={}",e.code,bytes.len());
        })?;
        self.apply(response).inspect_err(|e| {
            log::warn!(target:"sayweave","SAYWEAVE_ASR protocol=doubao stage=transcript code={} frame_type={frame_type:?} flags={flags:?}",e.code);
        })
    }
    fn apply(&mut self, response: Response) -> Result<Event, Error> {
        if self.finished {
            return Ok(Event::Heartbeat);
        }
        if let Some(seq) = response.sequence.filter(|_| !response.last) {
            let seq = seq.unsigned_abs();
            if seq <= self.server_sequence {
                return Ok(Event::Heartbeat);
            }
            self.server_sequence = seq;
        }
        let Some(result) = response.value.get("result") else {
            return if response.last {
                Ok(self.finish_existing())
            } else {
                Ok(Event::Heartbeat)
            };
        };
        if !result.is_object() {
            return Err(Error::new("protocol"));
        }
        let text = result
            .get("text")
            .map(|v| v.as_str().ok_or(Error::new("protocol")))
            .transpose()?;
        let utterances = result
            .get("utterances")
            .map(|v| v.as_array().ok_or(Error::new("protocol")))
            .transpose()?;
        if let Some(utterances) = utterances {
            if utterances.len() > 10000 {
                return Err(Error::new("responseTooLarge"));
            }
            // Validate all declared items, including positions whose confirmed text we retain.
            for utterance in utterances {
                let text = utterance
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or(Error::new("protocol"))?;
                if text.len() > 64 * 1024 {
                    return Err(Error::new("responseTooLarge"));
                }
                if utterance.get("definite").is_some_and(|v| !v.is_boolean()) {
                    return Err(Error::new("protocol"));
                }
            }
        }
        self.sequence += 1;
        if let Some(utterances) = utterances.filter(|u| !u.is_empty()) {
            // Full results replace the tentative suffix; confirmed positions are immutable.
            let confirmed = self
                .text
                .segments
                .iter()
                .take_while(|s| s.confirmed)
                .count();
            if utterances.len() < confirmed {
                return Ok(if response.last {
                    self.finish_existing()
                } else {
                    Event::Heartbeat
                });
            }
            let mut segments = Vec::with_capacity(utterances.len());
            for (index, utterance) in utterances.iter().enumerate() {
                if let Some(old) = self.text.segments.get(index).filter(|s| s.confirmed) {
                    segments.push(old.clone());
                    continue;
                }
                let text = utterance
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or(Error::new("protocol"))?;
                if text.len() > 64 * 1024 {
                    return Err(Error::new("responseTooLarge"));
                }
                segments.push(Segment {
                    id: index as u64 + 1,
                    sequence: self.sequence,
                    text: text.to_owned(),
                    confirmed: response.last
                        || utterance.get("definite").and_then(Value::as_bool) == Some(true),
                    begin_ms: utterance.get("start_time").and_then(Value::as_u64),
                    end_ms: utterance.get("end_time").and_then(Value::as_u64),
                });
            }
            self.text.segments = segments;
        } else if let Some(text) = text {
            if text.is_empty() && response.last {
                return Ok(self.finish_existing());
            }
            let confirmed = self.text.confirmed_text.clone();
            if !text.starts_with(&confirmed) {
                return if response.last {
                    Err(Error::new("protocol"))
                } else {
                    Ok(Event::Heartbeat)
                };
            }
            self.text.segments.retain(|s| s.confirmed);
            let rest = &text[confirmed.len()..];
            if !rest.is_empty() {
                self.text.segments.push(Segment {
                    id: self.text.segments.len() as u64 + 1,
                    sequence: self.sequence,
                    text: rest.to_owned(),
                    confirmed: response.last,
                    begin_ms: None,
                    end_ms: None,
                });
            }
        } else {
            return if response.last {
                Ok(self.finish_existing())
            } else {
                Ok(Event::Heartbeat)
            };
        }
        self.text.text = self.text.segments.iter().map(|s| s.text.as_str()).collect();
        self.text.confirmed_text = self
            .text
            .segments
            .iter()
            .filter(|s| s.confirmed)
            .map(|s| s.text.as_str())
            .collect();
        if self.text.text.len() > LIMIT {
            return Err(Error::new("responseTooLarge"));
        }
        self.finished = response.last;
        Ok(Event::Update(self.text.clone()))
    }

    fn finish_existing(&mut self) -> Event {
        self.finished = true;
        self.sequence += 1;
        for segment in &mut self.text.segments {
            segment.confirmed = true;
            segment.sequence = self.sequence;
        }
        self.text.confirmed_text = self.text.text.clone();
        if self.text.segments.is_empty() {
            Event::Finished
        } else {
            Event::Update(self.text.clone())
        }
    }
}

async fn connect(
    base: &str,
    key: &str,
    task: &str,
    resource: &str,
) -> Result<transport::Socket, Error> {
    let mut request = endpoint(base)?
        .into_client_request()
        .map_err(|_| Error::new("invalid"))?;
    for (name, value) in [
        ("X-Api-Key", key),
        ("X-Api-Resource-Id", resource),
        ("X-Api-Request-Id", task),
    ] {
        request
            .headers_mut()
            .insert(name, value.parse().map_err(|_| Error::new("invalid"))?);
    }
    transport::connect(base, request).await
}

pub async fn prepare(
    base: &str,
    key: &str,
    task: &str,
) -> Result<(transport::Socket, Transcript), Error> {
    for (index, resource) in RESOURCES.iter().enumerate() {
        let attempt = async {
            let mut socket = connect(base, key, task, resource).await?;
            socket
                .send(Message::Binary(initial_frame()?.into()))
                .await
                .map_err(|_| Error::new("network"))?;
            loop {
                match socket.next().await {
                    Some(Ok(Message::Ping(p))) => socket
                        .send(Message::Pong(p))
                        .await
                        .map_err(|_| Error::new("network"))?,
                    Some(Ok(Message::Pong(_))) => {}
                    Some(Ok(Message::Binary(bytes))) => {
                        let ack = decode(&bytes)?;
                        if ack.last
                            || ack.value.get("result").is_some_and(|r| {
                                r.get("text")
                                    .and_then(Value::as_str)
                                    .is_some_and(|t| !t.is_empty())
                            })
                        {
                            return Err(Error::new("protocol"));
                        }
                        return Ok((socket, Transcript::default()));
                    }
                    _ => return Err(Error::new("network")),
                }
            }
        }
        .await;
        match attempt {
            Err(e) if index == 0 && e.code == "resourceUnavailable" => continue,
            result => return result,
        }
    }
    Err(Error::new("resourceUnavailable"))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn response(value: Value, flags: u8, compressed: bool) -> Vec<u8> {
        let mut frame = if compressed {
            encode(0x90, flags, true, value.to_string().as_bytes()).unwrap()
        } else {
            let body = value.to_string().into_bytes();
            let mut f = vec![0x11, 0x90 | flags, 0x10, 0];
            f.extend_from_slice(&(body.len() as u32).to_be_bytes());
            f.extend(body);
            f
        };
        if flags & 1 != 0 {
            let sequence = if flags & 2 != 0 { -2i32 } else { 1 };
            frame.splice(4..4, sequence.to_be_bytes());
        }
        frame
    }
    fn with_sequence(value: Value, flags: u8, sequence: i32) -> Vec<u8> {
        let mut frame = response(value, flags, true);
        frame[4..8].copy_from_slice(&sequence.to_be_bytes());
        frame
    }
    #[test]
    fn terminal_sequence_is_not_a_stale_partial_update() {
        for (flags, sequence) in [(3, -1), (3, 43), (1, -43)] {
            let mut t = Transcript::default();
            for (seq, text) in [(1, "你"), (42, "你好")] {
                t.event(Message::Binary(
                    with_sequence(json!({"result":{"text":text}}), 1, seq).into(),
                ))
                .unwrap();
            }
            assert!(matches!(
                t.event(Message::Binary(
                    with_sequence(json!({"result":{"text":"你好。"}}), flags, sequence).into()
                ))
                .unwrap(),
                Event::Update(_)
            ));
            assert!(t.finished);
            assert_eq!(t.text.confirmed_text, "你好。");
        }
    }
    #[test]
    fn explicit_empty_terminal_packet_confirms_existing_text_only() {
        for value in [
            json!({}),
            json!({"result":{}}),
            json!({"result":{"text":"","utterances":[]}}),
        ] {
            let mut t = Transcript::default();
            t.event(Message::Binary(
                with_sequence(json!({"result":{"text":"有效文字"}}), 1, 42).into(),
            ))
            .unwrap();
            t.event(Message::Binary(with_sequence(value, 3, -1).into()))
                .unwrap();
            assert!(t.finished);
            assert_eq!(t.text.confirmed_text, "有效文字");
            assert!(t.text.segments.iter().all(|s| s.confirmed));
        }
        let mut empty = Transcript::default();
        empty
            .event(Message::Binary(with_sequence(json!({}), 3, -1).into()))
            .unwrap();
        assert!(empty.finished);
        assert!(empty.text.text.is_empty());
        let mut malformed = Transcript::default();
        assert!(
            malformed
                .event(Message::Binary(
                    with_sequence(json!({"result":[]}), 3, -1).into()
                ))
                .is_err()
        );
        assert!(!malformed.finished);
    }
    #[test]
    fn malformed_terminal_payload_cannot_confirm_partial_or_completed_segments() {
        for value in [
            json!([]),
            json!(null),
            json!({"code":"45000001"}),
            json!({"code":-1}),
            json!({"result":{"text":123}}),
            json!({"result":{"utterances":"bad"}}),
            json!({"result":{"utterances":[null]}}),
            json!({"result":{"utterances":[{"text":123}]}}),
            json!({"result":{"utterances":[{"text":"有效文字","definite":"true"}]}}),
        ] {
            for confirmed in [false, true] {
                let mut transcript = Transcript::default();
                transcript
                    .event(Message::Binary(
                        with_sequence(
                            json!({"result":{"utterances":[{"text":"有效文字","definite":confirmed}]}}),
                            1,
                            42,
                        )
                        .into(),
                    ))
                    .unwrap();
                assert_eq!(
                    transcript
                        .event(Message::Binary(with_sequence(value.clone(), 3, -1).into()))
                        .err()
                        .unwrap()
                        .code,
                    "protocol"
                );
                assert!(!transcript.finished);
                assert_eq!(transcript.text.text, "有效文字");
                assert_eq!(transcript.text.segments[0].confirmed, confirmed);
            }
        }
    }
    #[test]
    fn cumulative_results_replace_partial_and_preserve_confirmed_text() {
        let mut t = Transcript::default();
        for (text, end) in [("你", false), ("你好。", true), ("旧暂定", false)] {
            t.event(Message::Binary(
                response(
                    json!({"result":{"utterances":[{"text":text,"definite":end}]}}),
                    0,
                    true,
                )
                .into(),
            ))
            .unwrap();
        }
        assert_eq!(t.text.text, "你好。");
        t.event(Message::Binary(response(json!({"result":{"utterances":[{"text":"你好。","definite":true},{"text":"世界。","definite":false}]}}),3,true).into())).unwrap();
        assert!(t.finished);
        assert_eq!(t.text.confirmed_text, "你好。世界。");
        assert_eq!(t.text.segments.len(), 2);
    }
    #[test]
    fn validates_lengths_compression_sequence_errors_and_size_limits() {
        let good = response(json!({"code":20000000,"result":{"text":"hello"}}), 1, true);
        assert!(decode(&good).is_ok());
        for at in 0..good.len() {
            assert!(decode(&good[..at]).is_err());
        }
        let mut invalid = good.clone();
        invalid[0] = 0x21;
        assert!(decode(&invalid).is_err());
        let mut invalid = good.clone();
        invalid[2] = 0x12;
        assert!(decode(&invalid).is_err());
        let oversized = encode(0x90, 0, true, &vec![b'x'; LIMIT]).unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&vec![b'x'; LIMIT + 1]).unwrap();
        let compressed = encoder.finish().unwrap();
        let mut bomb = vec![0x11, 0x90, 0x11, 0];
        bomb.extend_from_slice(&(compressed.len() as u32).to_be_bytes());
        bomb.extend(compressed);
        assert_eq!(decode(&bomb).err().unwrap().code, "responseTooLarge");
        assert!(decode(&oversized).is_err());
        let mut error = vec![0x11, 0xf0, 0, 0];
        error.extend_from_slice(&45000030u32.to_be_bytes());
        error.extend_from_slice(&6u32.to_be_bytes());
        error.extend_from_slice(b"SECRET");
        let e = decode(&error).err().unwrap();
        assert_eq!(e.code, "resourceUnavailable");
        assert!(!serde_json::to_string(&e).unwrap().contains("SECRET"));
    }
    #[test]
    fn audio_and_endpoint_have_fixed_protocol_mapping() {
        assert_eq!(
            endpoint("https://openspeech.bytedance.com").unwrap(),
            "wss://openspeech.bytedance.com/api/v3/sauc/bigmodel_async"
        );
        assert!(endpoint("https://host/v1").is_err());
        assert!(endpoint("https://user:key@host").is_err());
        assert_eq!(
            &audio_frame(&[0, 1], false).unwrap()[..4],
            &[0x11, 0x20, 0x01, 0]
        );
        assert_eq!(audio_frame(&[], true).unwrap()[1], 0x22);
    }
}
