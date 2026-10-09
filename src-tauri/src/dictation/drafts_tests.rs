//! Drive the production actor transitions with controlled native replies and a
//! real local HTTPS model. Native rendering is covered by the input fixture.
use super::*;
use crate::{
    contracts::processing::SourceSegment,
    models::provider::tests::{Fixture, Reply},
    windows_input::composition::Command,
};

fn response(text: &str) -> Reply {
    Reply::json(
        200,
        &serde_json::json!({"choices":[{"index":0,"message":{"role":"assistant","content":text},"finish_reason":"stop"}]})
            .to_string(),
    )
}
fn snapshot(sequence: u64, segments: Vec<SourceSegment>) -> NativeTranscript {
    NativeTranscript {
        task_id: "actor-test".into(),
        turn: 1,
        sequence,
        text: segments.iter().map(|s| s.text.as_str()).collect(),
        segments,
    }
}
fn segment(id: u64, text: &str, confirmed: bool) -> SourceSegment {
    SourceSegment {
        id,
        revision: 1,
        text: text.into(),
        confirmed,
    }
}
fn actor(
    context: Option<Context>,
) -> (Arc<SessionDraft>, State, std::sync::mpsc::Receiver<Command>) {
    let (draft, native) = Draft::fixture();
    let activity = input_state::activity();
    let anchor = Anchor {
        capture_id: 1,
        window: 1,
        focus: 1,
        process: 1,
        lifetime: 1,
        generation: 1,
        captured_tick: 0,
        activity,
    };
    let control = Control::new(input_state::activity_evidence());
    let (cancel, _) = watch::channel(0);
    let actor = SessionDraft::new(
        "actor-test".into(),
        anchor,
        draft,
        control,
        cancel,
        context,
        None,
    );
    let (state, _) = actor.initial.lock().unwrap().take().unwrap();
    (actor, state, native)
}
async fn generated(state: &mut State) {
    let result = tokio::time::timeout(
        Duration::from_secs(3),
        SessionDraft::generation_value(&mut state.unit),
    )
    .await
    .unwrap();
    let Some(UnitSlot::Generating(g)) = state.unit.take() else {
        panic!("generation expected")
    };
    assert!(
        result.is_ok(),
        "generation error: {:?}",
        result.as_ref().err().map(|e| &e.code)
    );
    state.unit = Some(UnitSlot::Ready(g.unit.clone(), result));
}
async fn command(native: &std::sync::mpsc::Receiver<Command>) -> Command {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Ok(command) = native.try_recv() {
                return command;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}
async fn acknowledge(actor: &SessionDraft, state: &mut State) {
    let value = SessionDraft::native_value(&mut state.native).await.unwrap();
    let job = state.native.take().unwrap();
    assert!(actor.native_done(state, &job.kind, value).is_none());
}

#[tokio::test]
async fn completed_date_correction_reaches_native_application_with_latest_tail() {
    let raw = "呃，今天10月8号，哦，不对，今天10月7号。";
    let cleaned = "今天10月7号。";
    let fixture = Fixture::start(vec![response(cleaned)]);
    let root = tempfile::tempdir().unwrap();
    let data = crate::data::DataWorker::start(root.path().to_path_buf()).unwrap();
    let context = Context::fixture(fixture.provider.clone(), fixture.input(), data.clone());
    let (actor, mut state, native) = actor(Some(context));
    actor.observe(
        &mut state,
        snapshot(1, vec![segment(1, raw, true), segment(2, "尾部", false)]),
    );
    state.merge_at = Some(Instant::now() - Duration::from_millis(151));
    actor.start_generation(&mut state).unwrap();
    generated(&mut state).await;
    actor.observe(
        &mut state,
        snapshot(
            2,
            vec![segment(1, raw, true), segment(2, "最新尾部🙂", false)],
        ),
    );
    actor.schedule(&mut state).unwrap();
    let Command::Settle(_, prefix, tail, _, _, reply) = command(&native).await else {
        panic!("corrected prefix expected")
    };
    assert_eq!(prefix, cleaned);
    assert_eq!(tail, "最新尾部🙂");
    reply
        .send(PrefixOutcome {
            prefix: Mutation::Changed,
            tail_owned: true,
            error: None,
        })
        .ok()
        .unwrap();
    acknowledge(&actor, &mut state).await;
    assert_eq!(state.submitted, cleaned);
    assert_eq!(state.consumed, raw.chars().count());
    let record = &state.report.units[0];
    assert_eq!(record.generation, "success");
    assert_eq!(record.application, "applied");
    assert_eq!(record.candidate.as_deref(), Some(cleaned));
    assert!(record.error.is_none());
    drop(state);
    drop(actor);
    data.shutdown().await;
}

#[tokio::test]
async fn next_generation_overlaps_unacknowledged_prefix_and_uses_latest_tail() {
    let a = "甲。".repeat(256);
    let b = "第二段。";
    let fixture = Fixture::start(vec![response(&a), response(b)]);
    let root = tempfile::tempdir().unwrap();
    let data = crate::data::DataWorker::start(root.path().to_path_buf()).unwrap();
    let context = Context::fixture(fixture.provider.clone(), fixture.input(), data.clone());
    let (actor, mut state, native) = actor(Some(context));
    actor.observe(
        &mut state,
        snapshot(
            1,
            vec![
                segment(1, &a, true),
                segment(2, b, true),
                segment(3, "尾一", false),
            ],
        ),
    );
    state.merge_at = Some(Instant::now() - Duration::from_millis(151));
    actor.start_generation(&mut state).unwrap();
    generated(&mut state).await;
    actor.schedule(&mut state).unwrap();
    let Command::Settle(_, prefix, tail, _, _, reply) = command(&native).await else {
        panic!("prefix expected")
    };
    assert_eq!(prefix, a);
    assert_eq!(tail, "第二段。尾一");
    // No fresh ASR event: B must already dispatch while A has no native ACK.
    actor.start_generation(&mut state).unwrap();
    generated(&mut state).await;
    assert_eq!(fixture.seen.lock().unwrap().len(), 2);
    assert_eq!(state.consumed, 0);
    actor.observe(
        &mut state,
        snapshot(
            2,
            vec![
                segment(1, &a, true),
                segment(2, b, true),
                segment(3, "最新尾部🙂", false),
            ],
        ),
    );
    assert!(matches!(state.unit, Some(UnitSlot::Ready(..))));
    reply
        .send(PrefixOutcome {
            prefix: Mutation::Changed,
            tail_owned: true,
            error: None,
        })
        .ok()
        .unwrap();
    acknowledge(&actor, &mut state).await;
    assert!(state.latest.text.ends_with("最新尾部🙂"));
    actor.schedule(&mut state).unwrap();
    let Command::Settle(_, prefix, tail, _, _, reply) = command(&native).await else {
        panic!("second prefix expected")
    };
    assert_eq!(prefix, b);
    assert_eq!(tail, "最新尾部🙂");
    reply
        .send(PrefixOutcome {
            prefix: Mutation::Changed,
            tail_owned: true,
            error: None,
        })
        .ok()
        .unwrap();
    acknowledge(&actor, &mut state).await;
    assert_eq!(state.submitted, format!("{a}{b}"));
    assert_eq!(state.consumed, 516);
    drop(state);
    drop(actor);
    data.shutdown().await;
}
#[tokio::test]
async fn learning_receipts_keep_delayed_newlines_with_the_exact_prefix_and_final_tail() {
    let a = format!("{}甲\n", "甲。".repeat(255));
    let b = "第二段。\n";
    let fixture = Fixture::start(vec![response(&a), response(b)]);
    let root = tempfile::tempdir().unwrap();
    let data = crate::data::DataWorker::start(root.path().to_path_buf()).unwrap();
    let context = Context::fixture(fixture.provider.clone(), fixture.input(), data.clone());
    let (actor, mut state, native) = actor(Some(context));
    actor.observe(
        &mut state,
        snapshot(
            1,
            vec![
                segment(1, &a, true),
                segment(2, b, true),
                segment(3, "尾部🙂", false),
            ],
        ),
    );
    state.merge_at = Some(Instant::now() - Duration::from_millis(151));
    actor.start_generation(&mut state).unwrap();
    generated(&mut state).await;
    actor.schedule(&mut state).unwrap();
    let Command::Settle(_, prefix, tail, source, _, reply) = command(&native).await else {
        panic!()
    };
    assert_eq!(prefix, a.trim_end_matches('\n'));
    assert_eq!(tail, format!("\n{b}尾部🙂"));
    assert_eq!(
        source,
        Some((a.trim_end_matches('\n').into(), format!("\n{b}尾部🙂")))
    );
    reply
        .send(PrefixOutcome {
            prefix: Mutation::Changed,
            tail_owned: true,
            error: None,
        })
        .ok()
        .unwrap();
    acknowledge(&actor, &mut state).await;
    state.merge_at = Some(Instant::now() - Duration::from_millis(151));
    actor.start_generation(&mut state).unwrap();
    generated(&mut state).await;
    actor.schedule(&mut state).unwrap();
    let Command::Settle(_, prefix, tail, source, _, reply) = command(&native).await else {
        panic!()
    };
    assert_eq!(prefix, "\n第二段。");
    assert_eq!(tail, "\n尾部🙂");
    assert_eq!(source, Some(("\n第二段。".into(), "\n尾部🙂".into())));
    reply
        .send(PrefixOutcome {
            prefix: Mutation::Changed,
            tail_owned: true,
            error: None,
        })
        .ok()
        .unwrap();
    acknowledge(&actor, &mut state).await;
    let (finish, _) = oneshot::channel();
    state.ending = Some(Ending {
        mode: Finish::Commit,
        reply: finish,
        deadline: tokio::time::Instant::now() + Duration::from_secs(15),
        expired: false,
    });
    actor.schedule_final(&mut state).unwrap();
    let Command::Finish(_, tail, mode, source, _, _) = command(&native).await else {
        panic!()
    };
    assert_eq!(mode, Finish::Commit);
    assert_eq!(tail, "\n尾部🙂");
    assert_eq!(source, Some("\n尾部🙂".into()));
    drop(state);
    drop(actor);
    data.shutdown().await;
}

#[tokio::test]
async fn in_flight_source_revision_revokes_prefix_and_lookahead_without_replaying() {
    let a = "甲。".repeat(256);
    let fixture = Fixture::start(vec![response(&a), response("第二段。")]);
    let root = tempfile::tempdir().unwrap();
    let data = crate::data::DataWorker::start(root.path().to_path_buf()).unwrap();
    let context = Context::fixture(fixture.provider.clone(), fixture.input(), data.clone());
    let (actor, mut state, native) = actor(Some(context));
    actor.observe(
        &mut state,
        snapshot(1, vec![segment(1, &a, true), segment(2, "第二段。", true)]),
    );
    state.merge_at = Some(Instant::now() - Duration::from_millis(151));
    actor.start_generation(&mut state).unwrap();
    generated(&mut state).await;
    actor.schedule(&mut state).unwrap();
    let Command::Settle(_, _, _, _, permit, reply) = command(&native).await else {
        panic!("prefix expected")
    };
    actor.start_generation(&mut state).unwrap();
    generated(&mut state).await;
    let mut changed = segment(1, &"乙。".repeat(256), true);
    changed.revision = 2;
    actor.observe(
        &mut state,
        snapshot(2, vec![changed, segment(2, "第二段。", true)]),
    );
    assert!(!permit.load(Ordering::Acquire));
    assert!(state.unit.is_none());
    // Native may already have executed: uncertain means stop, never replay.
    reply
        .send(PrefixOutcome {
            prefix: Mutation::Uncertain,
            tail_owned: false,
            error: Some("compositionPartial".into()),
        })
        .ok()
        .unwrap();
    acknowledge(&actor, &mut state).await;
    actor.start_generation(&mut state).unwrap();
    assert!(state.unit.is_none());
    assert_eq!(state.consumed, 0);
    assert_eq!(state.stop_error.as_deref(), Some("compositionPartial"));
    drop(state);
    drop(actor);
    data.shutdown().await;
}

#[tokio::test]
async fn same_body_confirmation_keeps_source_revision_and_avoids_duplicate_native_body() {
    let fixture = Fixture::start(vec![response("测试文字。")]);
    let root = tempfile::tempdir().unwrap();
    let data = crate::data::DataWorker::start(root.path().to_path_buf()).unwrap();
    let context = Context::fixture(fixture.provider.clone(), fixture.input(), data.clone());
    let (actor, mut state, native) = actor(Some(context));
    actor.observe(
        &mut state,
        snapshot(1, vec![segment(1, "测试文字。", false)]),
    );
    actor.schedule(&mut state).unwrap();
    let Command::Replace(_, text, _, reply) = command(&native).await else {
        panic!("ASR expected")
    };
    assert_eq!(text, "测试文字。");
    reply.send(Ok(())).unwrap();
    acknowledge(&actor, &mut state).await;
    actor.observe(
        &mut state,
        snapshot(2, vec![segment(1, "测试文字。", true)]),
    );
    actor.schedule(&mut state).unwrap();
    assert!(state.native.is_none());
    state.merge_at = Some(Instant::now() - Duration::from_millis(151));
    actor.start_generation(&mut state).unwrap();
    generated(&mut state).await;
    assert_eq!(fixture.seen.lock().unwrap().len(), 1);
    drop(state);
    drop(actor);
    data.shutdown().await;
}

#[test]
fn winning_cancel_survives_cleanup_failure_and_trims_archive() {
    let (actor, mut state, _native) = actor(None);
    state.written_raw = "前段尾部".into();
    state.submitted = "前段".into();
    state.committed_source = 2;
    let mut outcome = FinishOutcome {
        delivery: Delivery::not_submitted("cancelled"),
        committed: false,
        handed_off: false,
        cancelled: true,
        rollback_failed: false,
    };
    outcome.record_cleanup(true);
    let delivery = actor
        .native_done(
            &mut state,
            &NativeKind::Final {
                mode: Finish::Commit,
                raw: "前段尾部".into(),
                text: "前段尾部".into(),
            },
            NativeValue::Finalized(outcome),
        )
        .unwrap();
    assert_eq!(delivery.error.as_deref(), Some("compositionCleanup"));
    assert!(state.report.cancelled);
    assert_eq!(state.report.raw, "前段");
    assert_eq!(state.report.text, "前段");
}

#[test]
fn rollback_and_cleanup_failure_preserve_uncommitted_recovery_text() {
    let (actor, mut state, _native) = actor(None);
    state.written_raw = "仍可能存在的尾部".into();
    let mut outcome = FinishOutcome {
        delivery: Delivery::not_submitted("compositionRollback"),
        committed: false,
        handed_off: false,
        cancelled: true,
        rollback_failed: true,
    };
    outcome.record_cleanup(true);
    let delivery = actor
        .native_done(
            &mut state,
            &NativeKind::Final {
                mode: Finish::Abort,
                raw: "仍可能存在的尾部".into(),
                text: "仍可能存在的尾部".into(),
            },
            NativeValue::Finalized(outcome),
        )
        .unwrap();
    assert_eq!(delivery.error.as_deref(), Some("compositionCleanup"));
    assert!(state.report.cancelled);
    assert!(state.report.rollback_failed);
    assert!(state.report.raw.is_empty());
    assert!(state.report.text.is_empty());
    assert_eq!(state.report.recovery_raw, "仍可能存在的尾部");
    assert_eq!(state.report.cancellation_raw(), "仍可能存在的尾部");
}
