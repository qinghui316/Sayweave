use super::audio::{Capture, publish_meter};
use crate::history::types::Error;
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicI32, Ordering},
};
use tauri::ipc::Channel;
use tokio::sync::{Mutex, oneshot, watch};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TestInput {
    id: String,
    device_id: Option<String>,
    gain_db: i32,
}

#[derive(Deserialize)]
#[serde(
    tag = "action",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum TestControl {
    Stop { id: String },
    Gain { id: String, gain_db: i32 },
}

#[derive(Clone, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TestEvent {
    Level { level: f32 },
    Stopped { error: Option<String> },
}

struct Test {
    id: String,
    gain: Arc<AtomicI32>,
    stop: watch::Sender<u8>,
    done: oneshot::Receiver<()>,
}

impl Test {
    async fn finish(&mut self) {
        self.stop.send_replace(1);
        let _ = (&mut self.done).await;
    }
}

#[derive(Default)]
pub(super) struct MicrophoneTest(Mutex<Option<Test>>);

impl MicrophoneTest {
    pub async fn start(
        &self,
        input: TestInput,
        events: Channel<TestEvent>,
        can_start: impl FnOnce() -> Result<(), Error>,
    ) -> Result<(), Error> {
        validate_gain(input.gain_db)?;
        let mut active = self.0.lock().await;
        if let Some(test) = active.as_mut() {
            test.finish().await;
            *active = None;
        }
        can_start()?;
        let gain = Arc::new(AtomicI32::new(input.gain_db));
        let (stop, signal) = watch::channel(0);
        let (done_tx, done) = oneshot::channel();
        let capture = Capture::preview(input.device_id, gain.clone(), signal.clone())?;
        let stop_on_disconnect = stop.clone();
        tokio::spawn(async move {
            let error = run(capture, signal, stop_on_disconnect, &events).await;
            let _ = events.send(TestEvent::Stopped { error });
            let _ = done_tx.send(());
        });
        *active = Some(Test {
            id: input.id,
            gain,
            stop,
            done,
        });
        Ok(())
    }

    pub async fn control(&self, control: TestControl) -> Result<(), Error> {
        match control {
            TestControl::Stop { id } => self.stop(Some(&id)).await,
            TestControl::Gain { id, gain_db } => {
                validate_gain(gain_db)?;
                let active = self.0.lock().await;
                if let Some(test) = active.as_ref().filter(|test| test.id == id) {
                    test.gain.store(gain_db, Ordering::Release);
                }
            }
        }
        Ok(())
    }

    pub async fn stop(&self, id: Option<&str>) {
        let mut active = self.0.lock().await;
        if let Some(test) = active.as_mut()
            && id.is_none_or(|id| test.id == id)
        {
            test.finish().await;
            *active = None;
        }
    }
}

fn validate_gain(gain: i32) -> Result<(), Error> {
    if matches!(gain, 0 | 3 | 6 | 9 | 12) {
        Ok(())
    } else {
        Err(Error::new("invalid"))
    }
}

async fn run(
    mut capture: Capture,
    signal: watch::Receiver<u8>,
    stop_on_disconnect: watch::Sender<u8>,
    events: &Channel<TestEvent>,
) -> Option<String> {
    let ready = capture.wait_ready().await;
    let publisher = ready.is_ok().then(|| {
        let events = events.clone();
        publish_meter(capture.meter.clone(), signal.clone(), move |sample| {
            if sample.capture_sequence > 0
                && events
                    .send(TestEvent::Level {
                        level: sample.level,
                    })
                    .is_err()
            {
                stop_on_disconnect.send_replace(1);
            }
        })
    });
    // Completion is sent only after CPAL has dropped the native stream.
    let result = (&mut capture.done).await;
    if let Some(publisher) = publisher {
        publisher.abort();
        let _ = publisher.await;
    }
    if *signal.borrow() != 0 {
        return None;
    }
    match ready {
        Err(error) => Some(error.code),
        Ok(()) => match result {
            Ok(result) => result.error,
            Err(_) => Some("deviceAccess".into()),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn handoff_waits_for_capture_release_and_old_ids_cannot_stop_it() {
        let microphone = MicrophoneTest::default();
        let (stop, mut signal) = watch::channel(0);
        let (released, done) = oneshot::channel();
        *microphone.0.lock().await = Some(Test {
            id: "current".into(),
            gain: Arc::new(AtomicI32::new(0)),
            stop,
            done,
        });
        microphone.stop(Some("old")).await;
        assert_eq!(*signal.borrow(), 0);
        {
            let mut handoff = std::pin::pin!(microphone.stop(None));
            tokio::select! {
                _ = &mut handoff => panic!("handoff completed before capture released"),
                _ = signal.changed() => assert_eq!(*signal.borrow(), 1),
            }
        }
        assert!(microphone.0.lock().await.is_some());
        released.send(()).unwrap();
        microphone.stop(None).await;
        assert!(microphone.0.lock().await.is_none());
    }
}
