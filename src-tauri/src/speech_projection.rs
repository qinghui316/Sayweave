//! Application-owned projection of native facts; no UI state belongs to SpeechService.
use crate::{
    Runtime,
    coordinator::Phase,
    dictation::DictationCoordinator,
    speech::{NativeEvent, SpeechService},
};
use tauri::{Emitter, Manager};

#[derive(Default)]
struct RuntimeCursor {
    task: Option<String>,
    owner: Option<u64>,
}
impl RuntimeCursor {
    fn apply(
        &mut self,
        runtime: &mut crate::coordinator::Snapshot,
        task: &str,
        phase: Phase,
    ) -> bool {
        if self.task.as_deref() != Some(task) {
            self.task = Some(task.into());
            self.owner = runtime.begin_native_projection(task, phase);
            return self.owner.is_some();
        }
        self.owner
            .is_some_and(|id| runtime.project_native(task, id, phase))
    }
}

pub fn attach(app: &tauri::AppHandle) -> Result<(), crate::history::types::Error> {
    let output_app = app.clone();
    app.state::<SpeechService>()
        .set_output(std::sync::Arc::new(move |event| match event {
            NativeEvent::ArchiveSettled(result) => {
                let _ = output_app.emit_to("main", "speech:archived", &result);
                let _ = output_app.emit_to("main", "processing:archived", &result.task_id);
            }
            NativeEvent::Snapshot(snapshot) => {
                // Every business transition reaches the renderer, even when the
                // current-state watch coalesces a Trial terminal with an External start.
                let _ = output_app.emit_to("main", "speech:snapshot", &snapshot);
                let _ = output_app.emit_to("voice-overlay", "speech:snapshot", &snapshot);
            }
            NativeEvent::Meter(meter) => {
                if output_app
                    .state::<SpeechService>()
                    .snapshot()
                    .is_ok_and(|s| {
                        s.task_id.as_deref() == Some(&meter.task_id) && s.phase == Phase::Recording
                    })
                {
                    let _ = output_app.emit_to("main", "speech:meter", &meter);
                    let _ = output_app.emit_to("voice-overlay", "speech:meter", &meter);
                }
            }
        }))?;
    let handle = app.clone();
    let mut states = handle.state::<SpeechService>().subscribe();
    tauri::async_runtime::spawn(async move {
        let mut current = (None, 0);
        let mut sequence = 0;
        let mut cursor = RuntimeCursor::default();
        while states.changed().await.is_ok() {
            let service = handle.state::<SpeechService>();
            if service.is_stopping() {
                break;
            }
            let state = states.borrow_and_update().clone();
            if state.sequence <= sequence
                || service
                    .snapshot()
                    .is_ok_and(|s| s.sequence != state.sequence)
            {
                continue;
            }
            sequence = state.sequence;
            if current == (state.task_id.clone(), state.lifecycle) {
                continue;
            }
            current = (state.task_id.clone(), state.lifecycle);
            handle
                .state::<DictationCoordinator>()
                .observe_speech(&handle, &state);
            let Some(task) = state.task_id.clone() else {
                continue;
            };
            let projected = if let Ok(mut runtime) = handle.state::<Runtime>().0.lock() {
                let changed = cursor.apply(&mut runtime, &task, state.phase);
                let result = cursor.owner.filter(|id| runtime.owns_native(&task, *id));
                if changed {
                    let copy = runtime.clone();
                    drop(runtime);
                    crate::publish(&handle, copy);
                }
                result
            } else {
                None
            };
            if matches!(state.phase, Phase::Completed | Phase::Cancelled)
                && let Some(runtime_id) = projected
            {
                let timer_app = handle.clone();
                let expected_lifecycle = state.lifecycle;
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
                    if crate::read(&timer_app).is_ok_and(|r| r.owns_native(&task, runtime_id)) {
                        timer_app
                            .state::<SpeechService>()
                            .acknowledge_terminal(&task, expected_lifecycle);
                    }
                });
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lost_native_ownership_cannot_rebind_after_demo_has_already_completed() {
        let mut cursor = RuntimeCursor::default();
        let mut runtime = crate::coordinator::Snapshot::default();
        assert!(cursor.apply(&mut runtime, "A", Phase::Completed));
        let old = cursor.owner.unwrap();
        assert!(runtime.owns_native("A", old)); // Timer checks here, before the race.
        runtime.demo("start", old).unwrap();
        let demo = runtime.session.id;
        runtime.settle(demo, Phase::Connecting, Phase::Recording);
        runtime.demo("finish", demo).unwrap();
        runtime.settle(demo, Phase::Processing, Phase::Completed);
        let before = runtime.clone();
        assert!(!cursor.apply(&mut runtime, "A", Phase::Idle));
        assert_eq!(runtime, before);
        assert!(cursor.apply(&mut runtime, "B", Phase::Connecting));
        assert!(runtime.owns_native("B", cursor.owner.unwrap()));
    }
}
