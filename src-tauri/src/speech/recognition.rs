use super::{doubao, qwen};
use crate::{history::types::Error, models::SpeechConfiguration};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Segment {
    pub id: u64,
    pub sequence: u64,
    pub text: String,
    pub confirmed: bool,
    pub begin_ms: Option<u64>,
    pub end_ms: Option<u64>,
}
#[derive(Default, Clone)]
pub struct Transcript {
    pub text: String,
    pub confirmed_text: String,
    pub segments: Vec<Segment>,
}
impl Transcript {
    pub fn text(&self, confirmed: bool) -> String {
        if confirmed {
            self.confirmed_text.clone()
        } else {
            self.text.clone()
        }
    }
    pub fn segments(&self) -> Vec<Segment> {
        self.segments.clone()
    }
}
pub enum Event {
    Update(Transcript),
    Finished,
    Heartbeat,
}
enum Command {
    Audio(Vec<i16>),
    Finish(oneshot::Sender<Result<(), Error>>),
}
enum Parser {
    Qwen(qwen::Transcript),
    Doubao(doubao::Transcript),
}
impl Parser {
    fn event(&mut self, message: Message, task: &str) -> Result<Event, Error> {
        match self {
            Self::Qwen(t) => {
                let Message::Text(text) = message else {
                    return Err(Error::new("protocol"));
                };
                let value = serde_json::from_str(&text).map_err(|_| Error::new("protocol"))?;
                match t.event(&value, task)? {
                    qwen::Event::Segment => Ok(Event::Update(Transcript {
                        text: t.text(false),
                        confirmed_text: t.text(true),
                        segments: t.segments(),
                    })),
                    qwen::Event::Finished => Ok(Event::Finished),
                    qwen::Event::Heartbeat => Ok(Event::Heartbeat),
                    qwen::Event::Started => Err(Error::new("protocol")),
                }
            }
            Self::Doubao(t) => t.event(message),
        }
    }
    fn finished(&self) -> bool {
        matches!(self,Self::Doubao(t) if t.finished)
    }
    fn audio(&self, pcm: &[i16]) -> Result<Message, Error> {
        let bytes = pcm.iter().flat_map(|s| s.to_le_bytes()).collect::<Vec<_>>();
        match self {
            Self::Qwen(_) => Ok(Message::Binary(bytes.into())),
            Self::Doubao(_) => Ok(Message::Binary(doubao::audio_frame(&bytes, false)?.into())),
        }
    }
    fn finish(&self, task: &str) -> Result<Message, Error> {
        match self {
            Self::Qwen(_) => Ok(Message::Text(serde_json::json!({"header":{"action":"finish-task","task_id":task,"streaming":"duplex"},"payload":{"input":{}}}).to_string().into())),
            Self::Doubao(_) => Ok(Message::Binary(doubao::audio_frame(&[],true)?.into())),
        }
    }
}

/// Owns transport tasks only. Recording, persistence and windows stay in SpeechService.
pub struct RecognitionSession {
    audio: mpsc::Sender<Command>,
    events: mpsc::Receiver<Result<Event, Error>>,
    worker: tokio::task::JoinHandle<()>,
}
impl RecognitionSession {
    #[cfg(test)]
    pub async fn open(cfg: &SpeechConfiguration, task: &str) -> Result<Self, Error> {
        Self::open_with_hotwords(cfg, task, &[]).await
    }
    pub async fn open_with_hotwords(
        cfg: &SpeechConfiguration,
        task: &str,
        hotwords: &[String],
    ) -> Result<Self, Error> {
        if !crate::models::types::capabilities(&cfg.input.protocol).is_some_and(|c| c.live_text) {
            return Err(Error::new("unsupported"));
        }
        let began = Instant::now();
        let prepared = tokio::time::timeout(Duration::from_secs(15), async {
            match cfg.input.protocol.as_str() {
                "qwen-audio-streaming" => {
                    let mut socket = qwen::connect(
                        &cfg.input.base_url,
                        &cfg.key,
                        &cfg.input.model_id,
                        task,
                        hotwords,
                    )
                    .await?;
                    let mut transcript = qwen::Transcript::default();
                    loop {
                        let value = qwen::receive(&mut socket).await?;
                        match transcript.event(&value, task)? {
                            qwen::Event::Started => break,
                            qwen::Event::Heartbeat => {}
                            _ => return Err(Error::new("protocol")),
                        }
                    }
                    Ok((socket, Parser::Qwen(transcript)))
                }
                "doubao-asr-streaming" => {
                    let (socket, transcript) =
                        doubao::prepare(&cfg.input.base_url, &cfg.key, task, hotwords).await?;
                    Ok((socket, Parser::Doubao(transcript)))
                }
                _ => Err(Error::new("unsupported")),
            }
        })
        .await
        .unwrap_or_else(|_| Err(Error::new("connectionTimeout")));
        log::info!(target:"sayweave", "SAYWEAVE_SPEECH operation=recognition_prepare protocol={} elapsed_ms={} result={}",
            cfg.input.protocol, began.elapsed().as_millis(), prepared.as_ref().map(|_| "ok").unwrap_or_else(|e| e.code.as_str()));
        let prepared = prepared?;
        Ok(Self::spawn(prepared.0, prepared.1, task.to_owned()))
    }
    fn spawn(socket: super::transport::Socket, mut parser: Parser, task: String) -> Self {
        let (audio, mut commands) = mpsc::channel(32);
        let (events_tx, events) = mpsc::channel(32);
        let worker = tokio::spawn(async move {
            let began = Instant::now();
            let (mut writer, mut reader) = socket.split();
            // Encoding is independent of response state; use a separate encoder for the writer.
            let encoder = match &parser {
                Parser::Qwen(_) => Parser::Qwen(qwen::Transcript::default()),
                Parser::Doubao(_) => Parser::Doubao(doubao::Transcript::default()),
            };
            let (pong_tx, mut pong_rx) = mpsc::channel(8);
            let write = async {
                let mut ended = false;
                loop {
                    let command = tokio::select! {
                        Some(pong)=pong_rx.recv()=>{writer.send(Message::Pong(pong)).await.map_err(|_|Error::new("network"))?;continue;},
                        command=commands.recv()=>command.ok_or(Error::new("cancelled"))?,
                    };
                    if ended {
                        return Err(Error::new("protocol"));
                    }
                    let (message, ack) = match command {
                        Command::Audio(pcm) => (encoder.audio(&pcm)?, None),
                        Command::Finish(ack) => (encoder.finish(&task)?, Some(ack)),
                    };
                    let sent = tokio::time::timeout(Duration::from_secs(5), writer.send(message))
                        .await
                        .map_err(|_| Error::new("timeout"))?
                        .map_err(|_| Error::new("network"));
                    if let Some(ack) = ack {
                        log::info!(target:"sayweave", "operation=asr_finish_sent task_id={} elapsed_ms={} success={}",task,began.elapsed().as_millis(),sent.is_ok());
                        let result = sent.clone();
                        let _ = ack.send(sent);
                        result?;
                        ended = true;
                    } else {
                        sent?;
                    }
                }
            };
            let read = async {
                let mut boundary = None;
                while let Some(message) = reader.next().await {
                    let message = message.map_err(|_| Error::new("network"))?;
                    match message {
                        Message::Ping(p) => {
                            pong_tx
                                .try_send(p)
                                .map_err(|_| Error::new("bufferOverflow"))?;
                            continue;
                        }
                        Message::Pong(_) => continue,
                        Message::Close(_) => return Err(Error::new("network")),
                        message => {
                            let event = parser.event(message, &task)?;
                            let explicit = matches!(event, Event::Finished);
                            let finished = explicit || parser.finished();
                            if let Event::Update(transcript) = &event {
                                let counts = (
                                    transcript.segments.len(),
                                    transcript.segments.iter().filter(|s| s.confirmed).count(),
                                    finished,
                                );
                                if boundary != Some(counts) {
                                    let basis = if finished {
                                        "task_final"
                                    } else {
                                        match &parser {
                                            Parser::Qwen(_) => "sentence_end",
                                            Parser::Doubao(_) => "definite",
                                        }
                                    };
                                    log::info!(target:"sayweave", "operation=asr_boundary task_id={} basis={} segments={} confirmed={} terminal={} elapsed_ms={}", task,basis,counts.0,counts.1,finished,began.elapsed().as_millis());
                                    boundary = Some(counts);
                                }
                            }
                            events_tx
                                .send(Ok(event))
                                .await
                                .map_err(|_| Error::new("cancelled"))?;
                            if finished {
                                if !explicit {
                                    events_tx
                                        .send(Ok(Event::Finished))
                                        .await
                                        .map_err(|_| Error::new("cancelled"))?;
                                }
                                return Ok(());
                            }
                        }
                    }
                }
                Err(Error::new("network"))
            };
            tokio::pin!(write, read);
            let result = tokio::select! {
                result=&mut read=>result,
                result=&mut write=>match result {Ok(())=>read.await,Err(e)=>Err(e)},
            };
            if let Err(error) = result {
                let _ = events_tx.send(Err(error)).await;
            }
        });
        Self {
            audio,
            events,
            worker,
        }
    }
    pub async fn send_pcm(&self, pcm: &[i16]) -> Result<(), Error> {
        if pcm.len() > 32000 {
            return Err(Error::new("bufferOverflow"));
        }
        // A bounded burst after a disk checkpoint must let the transport drain its queue.
        // Sustained backpressure still fails explicitly; capture never waits on this task.
        tokio::time::timeout(
            Duration::from_millis(500),
            self.audio.send(Command::Audio(pcm.to_vec())),
        )
        .await
        .map_err(|_| Error::new("bufferOverflow"))?
        .map_err(|_| Error::new("network"))
    }
    pub async fn finish(&self) -> Result<(), Error> {
        let (tx, rx) = oneshot::channel();
        tokio::time::timeout(Duration::from_secs(5), async {
            self.audio
                .send(Command::Finish(tx))
                .await
                .map_err(|_| Error::new("network"))?;
            rx.await.map_err(|_| Error::new("network"))?
        })
        .await
        .map_err(|_| Error::new("timeout"))?
    }
    pub async fn next_event(&mut self) -> Result<Event, Error> {
        self.events.recv().await.ok_or(Error::new("network"))?
    }
}
impl Drop for RecognitionSession {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

#[cfg(test)]
pub(super) struct ControlledPeer {
    pub events: mpsc::Sender<Result<Event, Error>>,
    commands: mpsc::Receiver<Command>,
}
#[cfg(test)]
impl ControlledPeer {
    pub async fn audio(&mut self) -> Vec<i16> {
        match tokio::time::timeout(Duration::from_secs(2), self.commands.recv())
            .await
            .unwrap()
            .unwrap()
        {
            Command::Audio(pcm) => pcm,
            Command::Finish(_) => panic!("finish arrived instead of PCM"),
        }
    }
    pub async fn finished_audio(&mut self) {
        match tokio::time::timeout(Duration::from_secs(2), self.commands.recv())
            .await
            .unwrap()
            .unwrap()
        {
            Command::Finish(ack) => {
                ack.send(Ok(())).unwrap();
            }
            Command::Audio(_) => panic!("paused or wrong-turn PCM reached connection"),
        }
    }
}
#[cfg(test)]
impl RecognitionSession {
    pub(super) fn controlled() -> (Self, ControlledPeer) {
        let (audio, commands) = mpsc::channel(32);
        let (send, events) = mpsc::channel(32);
        (
            Self {
                audio,
                events,
                worker: tokio::spawn(std::future::pending()),
            },
            ControlledPeer {
                events: send,
                commands,
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn transport_streams_updates_and_answers_ping_after_audio_finish() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            assert!(matches!(
                socket.next().await.unwrap().unwrap(),
                Message::Binary(_)
            ));
            let result = serde_json::json!({"header":{"task_id":"task","event":"result-generated"},"payload":{"output":{"sentence":{"sentence_id":1,"text":"你好","sentence_end":false}}}});
            socket
                .send(Message::Text(result.to_string().into()))
                .await
                .unwrap();
            assert!(matches!(
                socket.next().await.unwrap().unwrap(),
                Message::Text(_)
            ));
            socket.send(Message::Ping(vec![1, 2].into())).await.unwrap();
            assert!(matches!(
                tokio::time::timeout(Duration::from_secs(2), socket.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap(),
                Message::Pong(_)
            ));
            // Final tail arrives after finish; the receiver and heartbeat remain alive.
            tokio::time::sleep(Duration::from_millis(180)).await;
            let mut result = result;
            result["payload"]["output"]["sentence"]["text"] = "你好，尾字。".into();
            result["payload"]["output"]["sentence"]["sentence_end"] = true.into();
            socket
                .send(Message::Text(result.to_string().into()))
                .await
                .unwrap();
            socket
                .send(Message::Text(
                    serde_json::json!({"header":{"task_id":"task","event":"task-finished"}})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        });
        let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let mut session = RecognitionSession::spawn(
            socket,
            Parser::Qwen(qwen::Transcript::default()),
            "task".into(),
        );
        session.send_pcm(&[1, 2, 3]).await.unwrap();
        assert!(
            matches!(session.next_event().await.unwrap(),Event::Update(t) if t.text=="你好" && t.confirmed_text.is_empty())
        );
        session.finish().await.unwrap();
        assert!(
            matches!(session.next_event().await.unwrap(),Event::Update(t) if t.confirmed_text=="你好，尾字。")
        );
        assert!(matches!(
            session.next_event().await.unwrap(),
            Event::Finished
        ));
        server.await.unwrap();
    }
    #[tokio::test]
    async fn bounded_burst_yields_to_transport_instead_of_reporting_false_overflow() {
        let (tx, mut rx) = mpsc::channel(1);
        let (_, events) = mpsc::channel(1);
        let worker = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            for _ in 0..40 {
                assert!(matches!(rx.recv().await, Some(Command::Audio(_))));
            }
        });
        let session = RecognitionSession {
            audio: tx,
            events,
            worker,
        };
        for _ in 0..40 {
            session.send_pcm(&[1; 320]).await.unwrap();
        }
        drop(session);
    }
    #[tokio::test]
    async fn bounded_audio_backpressure_is_an_error_and_drop_stops_transport() {
        let (tx, _rx) = mpsc::channel(1);
        let (_, events) = mpsc::channel(1);
        let worker = tokio::spawn(std::future::pending());
        let abort = worker.abort_handle();
        let session = RecognitionSession {
            audio: tx,
            events,
            worker,
        };
        session.send_pcm(&[1]).await.unwrap();
        assert_eq!(
            session.send_pcm(&[2]).await.unwrap_err().code,
            "bufferOverflow"
        );
        drop(session);
        tokio::task::yield_now().await;
        assert!(abort.is_finished());
    }
}
