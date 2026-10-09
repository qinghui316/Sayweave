use super::recognition::Segment;
use super::transport::Socket;
use crate::history::types::Error;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
#[derive(Default)]
pub struct Transcript {
    segments: BTreeMap<u64, Segment>,
    sequence: u64,
}
pub enum Event {
    Started,
    Segment,
    Finished,
    Heartbeat,
}
impl Transcript {
    pub fn event(&mut self, v: &Value, task: &str) -> Result<Event, Error> {
        if v.pointer("/header/task_id").and_then(Value::as_str) != Some(task) {
            return Err(Error::new("protocol"));
        }
        match v.pointer("/header/event").and_then(Value::as_str) {
            Some("task-started") => Ok(Event::Started),
            Some("task-finished") => Ok(Event::Finished),
            Some("task-failed") => Err(Error::new(
                match v
                    .pointer("/header/error_code")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                {
                    "AuthenticationFailed" | "InvalidApiKey" => "auth",
                    "Throttling" | "TooManyRequests" => "rateLimit",
                    _ => "service",
                },
            )),
            Some("result-generated") => {
                if v.pointer("/payload/output/sentence/heartbeat")
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    return Ok(Event::Heartbeat);
                }
                let s = v
                    .pointer("/payload/output/sentence")
                    .ok_or(Error::new("protocol"))?;
                let id = s
                    .get("sentence_id")
                    .and_then(Value::as_u64)
                    .ok_or(Error::new("protocol"))?;
                if id == 0 {
                    return Err(Error::new("protocol"));
                }
                let text = s
                    .get("text")
                    .and_then(Value::as_str)
                    .ok_or(Error::new("protocol"))?;
                if text.len() > 64 * 1024
                    || (self.segments.len() >= 10000 && !self.segments.contains_key(&id))
                {
                    return Err(Error::new("responseTooLarge"));
                }
                if self.segments.get(&id).is_some_and(|p| p.confirmed) {
                    return Ok(Event::Heartbeat);
                }
                self.sequence += 1;
                let seg = Segment {
                    id,
                    sequence: self.sequence,
                    text: text.into(),
                    confirmed: s.get("sentence_end").and_then(Value::as_bool) == Some(true),
                    begin_ms: s.get("begin_time").and_then(Value::as_u64),
                    end_ms: s.get("end_time").and_then(Value::as_u64),
                };
                self.segments.insert(id, seg.clone());
                if self.text(false).len() > 1024 * 1024 {
                    return Err(Error::new("responseTooLarge"));
                }
                Ok(Event::Segment)
            }
            _ => Err(Error::new("protocol")),
        }
    }
    pub fn text(&self, confirmed: bool) -> String {
        self.segments
            .values()
            .filter(|s| !confirmed || s.confirmed)
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("")
    }
    pub fn segments(&self) -> Vec<Segment> {
        self.segments.values().cloned().collect()
    }
}
pub fn endpoint(base: &str) -> Result<String, Error> {
    let mut u = reqwest::Url::parse(base).map_err(|_| Error::new("invalid"))?;
    if u.scheme() != "https"
        || u.path() != "/"
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err(Error::new("invalid"));
    }
    u.set_scheme("wss").map_err(|_| Error::new("invalid"))?;
    u.set_path("/api-ws/v1/inference");
    Ok(u.to_string())
}
pub async fn connect(
    base: &str,
    key: &str,
    model: &str,
    task: &str,
    hotwords: &[String],
) -> Result<Socket, Error> {
    let mut request = endpoint(base)?
        .into_client_request()
        .map_err(|_| Error::new("invalid"))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {key}")
            .parse()
            .map_err(|_| Error::new("invalid"))?,
    );
    let mut socket = super::transport::connect(base, request).await?;
    socket
        .send(Message::Text(
            start_request(model, task, hotwords).to_string().into(),
        ))
        .await
        .map_err(|_| Error::new("network"))?;
    Ok(socket)
}
fn start_request(model: &str, task: &str, hotwords: &[String]) -> Value {
    let mut value = json!({"header":{"action":"run-task","task_id":task,"streaming":"duplex"},"payload":{"task_group":"audio","task":"asr","function":"recognition","model":model,"parameters":{"format":"pcm","sample_rate":16000,"heartbeat":true,"semantic_punctuation_enabled":false,"max_sentence_silence":800},"input":{}}});
    let words = super::hotwords::select(hotwords, 100);
    if !words.is_empty() {
        value["payload"]["parameters"]["vocabulary"] =
            Value::Object(words.into_iter().map(|word| (word, json!(4))).collect());
    }
    value
}

pub async fn receive(socket: &mut Socket) -> Result<Value, Error> {
    loop {
        match socket.next().await {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(&text).map_err(|_| Error::new("protocol"));
            }
            Some(Ok(Message::Ping(p))) => socket
                .send(Message::Pong(p))
                .await
                .map_err(|_| Error::new("network"))?,
            Some(Ok(Message::Pong(_))) => {}
            _ => return Err(Error::new("network")),
        }
    }
}

#[cfg(test)]
mod boundary_tests {
    use super::*;
    #[test]
    fn hotwords_use_vocabulary_weights_and_keep_recording_parameters() {
        let words = (0..120).map(|i| format!("词条{i}")).collect::<Vec<_>>();
        let request = start_request("qwen-audio-3.1-asr-flash-streaming", "task", &words);
        let vocabulary = request["payload"]["parameters"]["vocabulary"]
            .as_object()
            .unwrap();
        assert_eq!(vocabulary.len(), 100);
        assert!(vocabulary.values().all(|v| *v == json!(4)));
        assert_eq!(
            request["payload"]["parameters"]["max_sentence_silence"],
            800
        );
        assert_eq!(request["header"]["action"], "run-task");
        assert!(
            start_request("model", "task", &[])["payload"]["parameters"]
                .get("vocabulary")
                .is_none()
        );
    }
    #[test]
    fn both_models_use_sentence_vad_without_ending_the_connection() {
        for model in [
            "qwen-audio-3.0-asr-flash-streaming",
            "qwen-audio-3.1-asr-flash-streaming",
        ] {
            let request = start_request(model, "task", &[]);
            assert_eq!(request["header"]["action"], "run-task");
            assert_eq!(
                request["payload"]["parameters"]["max_sentence_silence"],
                800
            );
            assert_eq!(
                request["payload"]["parameters"]["semantic_punctuation_enabled"],
                false
            );
            let mut transcript = Transcript::default();
            for (id, text, confirmed) in [
                (1, "第一段。", false),
                (1, "第一段。", true),
                (2, "尾部", false),
                (2, "尾部。", true),
            ] {
                assert!(matches!(transcript.event(&json!({"header":{"task_id":"task","event":"result-generated"},"payload":{"output":{"sentence":{"sentence_id":id,"text":text,"sentence_end":confirmed}}}}),"task").unwrap(), Event::Segment));
            }
            assert_eq!(transcript.text(true), "第一段。尾部。");
            assert!(matches!(
                transcript
                    .event(
                        &json!({"header":{"task_id":"task","event":"task-finished"}}),
                        "task"
                    )
                    .unwrap(),
                Event::Finished
            ));
        }
    }
}
