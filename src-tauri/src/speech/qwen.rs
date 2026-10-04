use crate::history::types::Error;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, client::IntoClientRequest, protocol::WebSocketConfig},
};
pub type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub id: u64,
    pub sequence: u64,
    pub text: String,
    pub confirmed: bool,
    pub begin_ms: Option<u64>,
    pub end_ms: Option<u64>,
}
#[derive(Default)]
pub struct Transcript {
    segments: BTreeMap<u64, Segment>,
    sequence: u64,
}
pub enum Event {
    Started,
    Segment(Segment),
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
                Ok(Event::Segment(seg))
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
pub async fn connect(base: &str, key: &str, model: &str, task: &str) -> Result<Socket, Error> {
    let mut request = endpoint(base)?
        .into_client_request()
        .map_err(|_| Error::new("invalid"))?;
    request.headers_mut().insert(
        "Authorization",
        format!("Bearer {key}")
            .parse()
            .map_err(|_| Error::new("invalid"))?,
    );
    let config = WebSocketConfig::default()
        .max_message_size(Some(1024 * 1024))
        .max_frame_size(Some(1024 * 1024));
    let connection = async {
        #[cfg(feature = "desktop-acceptance")]
        if let Some(connector) = acceptance_connector(base)? {
            return tokio_tungstenite::connect_async_tls_with_config(
                request,
                Some(config),
                false,
                Some(connector),
            )
            .await
            .map_err(connection_error);
        }
        tokio_tungstenite::connect_async_with_config(request, Some(config), false)
            .await
            .map_err(connection_error)
    };
    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(5), connection)
        .await
        .map_err(|_| Error::new("timeout"))??;
    socket.send(Message::Text(json!({"header":{"action":"run-task","task_id":task,"streaming":"duplex"},"payload":{"task_group":"audio","task":"asr","function":"recognition","model":model,"parameters":{"format":"pcm","sample_rate":16000,"heartbeat":true},"input":{}}}).to_string().into())).await.map_err(|_|Error::new("network"))?;
    Ok(socket)
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
pub async fn audio(socket: &mut Socket, pcm: &[i16]) -> Result<(), Error> {
    let bytes = pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>();
    tokio::time::timeout(
        Duration::from_secs(5),
        socket.send(Message::Binary(bytes.into())),
    )
    .await
    .map_err(|_| Error::new("timeout"))?
    .map_err(|_| Error::new("network"))
}
pub async fn finish(socket: &mut Socket, task: &str) -> Result<(), Error> {
    tokio::time::timeout(Duration::from_secs(5),socket.send(Message::Text(json!({"header":{"action":"finish-task","task_id":task,"streaming":"duplex"},"payload":{"input":{}}}).to_string().into()))).await.map_err(|_|Error::new("timeout"))?.map_err(|_|Error::new("network"))
}

fn connection_error(e: tokio_tungstenite::tungstenite::Error) -> Error {
    match e {
        tokio_tungstenite::tungstenite::Error::Http(r) => Error::new(match r.status().as_u16() {
            401 | 403 => "auth",
            429 => "rateLimit",
            _ => "protocol",
        }),
        _ => Error::new("network"),
    }
}
#[cfg(feature = "desktop-acceptance")]
fn acceptance_connector(base: &str) -> Result<Option<tokio_tungstenite::Connector>, Error> {
    let id = match std::env::var("SAYWEAVE_DATA_ACCEPTANCE_ID") {
        Ok(id) if uuid::Uuid::parse_str(&id).is_ok() => id,
        _ => return Ok(None),
    };
    if reqwest::Url::parse(base)
        .ok()
        .and_then(|u| u.host_str().map(String::from))
        .as_deref()
        != Some("localhost")
    {
        return Ok(None);
    }
    let file = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../.local/data-acceptance")
        .join(id)
        .join("data/fixture-ca.der");
    let certificate = std::fs::read(file).map_err(|_| Error::new("protocol"))?;
    let mut roots = rustls::RootCertStore::empty();
    roots
        .add(rustls::pki_types::CertificateDer::from(certificate))
        .map_err(|_| Error::new("protocol"))?;
    Ok(Some(tokio_tungstenite::Connector::Rustls(
        std::sync::Arc::new(
            rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth(),
        ),
    )))
}
