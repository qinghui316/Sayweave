use super::{AudioLease, types::Error};
use serde::Serialize;
use std::{path::PathBuf, time::Duration};
use tokio::sync::{mpsc, oneshot};
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackState {
    pub resource_id: Option<String>,
    pub playing: bool,
    pub position_ms: u64,
    pub duration_ms: u64,
}
enum Action {
    Start(PathBuf, Box<AudioLease>),
    Pause,
    Resume,
    Seek(u64),
    Stop,
    Status,
}
struct Request {
    action: Action,
    response: oneshot::Sender<Result<PlaybackState, Error>>,
}
#[derive(Clone)]
pub struct Playback {
    sender: mpsc::Sender<Request>,
}
impl Playback {
    pub fn new() -> Self {
        let (sender, mut receiver) = mpsc::channel::<Request>(16);
        std::thread::spawn(move || {
            let mut output = None;
            let mut player: Option<rodio::Player> = None;
            let mut lease: Option<AudioLease> = None;
            loop {
                if player.as_ref().is_some_and(|p| p.empty()) {
                    player = None;
                    lease = None;
                    output = None;
                }
                let req = match receiver.try_recv() {
                    Ok(req) => req,
                    Err(mpsc::error::TryRecvError::Disconnected) => break,
                    Err(mpsc::error::TryRecvError::Empty) => {
                        std::thread::sleep(Duration::from_millis(50));
                        continue;
                    }
                };
                let result = (|| -> Result<(), Error> {
                    match req.action {
                        Action::Start(path, new_lease) => {
                            if let Some(p) = player.take() {
                                p.stop();
                            }
                            lease = None;
                            let sink = rodio::DeviceSinkBuilder::open_default_sink()
                                .map_err(|_| Error::new("playbackFailed"))?;
                            let p = rodio::Player::connect_new(sink.mixer());
                            let file = std::fs::File::open(path)
                                .map_err(|_| Error::new("fileUnavailable"))?;
                            let decoder = rodio::Decoder::try_from(file)
                                .map_err(|_| Error::new("fileUnavailable"))?;
                            p.append(decoder);
                            output = Some(sink);
                            player = Some(p);
                            lease = Some(*new_lease);
                        }
                        Action::Pause => {
                            if let Some(p) = &player {
                                p.pause();
                            }
                        }
                        Action::Resume => {
                            if let Some(p) = &player {
                                p.play();
                            }
                        }
                        Action::Seek(ms) => {
                            if let Some(p) = &player {
                                p.try_seek(Duration::from_millis(ms))
                                    .map_err(|_| Error::new("playbackFailed"))?;
                            }
                        }
                        Action::Stop => {
                            if let Some(p) = player.take() {
                                p.stop();
                            }
                            lease = None;
                            output = None;
                        }
                        Action::Status => {}
                    }
                    Ok(())
                })();
                let state = PlaybackState {
                    resource_id: lease.as_ref().map(|l| l.audio.id.clone()),
                    playing: player
                        .as_ref()
                        .is_some_and(|p| !p.is_paused() && !p.empty()),
                    position_ms: player
                        .as_ref()
                        .map(|p| p.get_pos().as_millis() as u64)
                        .unwrap_or(0),
                    duration_ms: lease.as_ref().map(|l| l.audio.bytes / 32).unwrap_or(0),
                };
                if player.as_ref().is_some_and(|p| p.empty()) {
                    player = None;
                    lease = None;
                    output = None;
                }
                let _ = &output;
                let _ = req.response.send(result.map(|_| state));
            }
        });
        Self { sender }
    }
    async fn call(&self, action: Action) -> Result<PlaybackState, Error> {
        let (response, rx) = oneshot::channel();
        self.sender
            .try_send(Request { action, response })
            .map_err(|_| Error::new("busy"))?;
        rx.await.map_err(|_| Error::new("unavailable"))?
    }
    pub async fn start(&self, path: PathBuf, lease: AudioLease) -> Result<PlaybackState, Error> {
        self.call(Action::Start(path, Box::new(lease))).await
    }
    pub async fn control(&self, action: &str, ms: Option<u64>) -> Result<PlaybackState, Error> {
        self.call(match action {
            "pause" => Action::Pause,
            "resume" => Action::Resume,
            "seek" => Action::Seek(ms.ok_or(Error::new("invalid"))?),
            "stop" => Action::Stop,
            "status" => Action::Status,
            _ => return Err(Error::new("invalid")),
        })
        .await
    }
}
