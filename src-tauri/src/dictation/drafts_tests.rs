//! Drive the production actor transitions with controlled native replies and a
//! real local HTTPS model. Native rendering is covered by the input fixture.
use super::*;
#[tokio::test]
async fn pending_initial_input_keeps_audio_and_all_asr_until_the_first_real_write() {
    let (base, mut old, native) = production_actor(None);
    let draft = old.draft.take().unwrap();
    let actor = SessionDraft::new_initial(
        base.task.clone(),
        base.anchor.clone(),
        InitialInput {
            connection: draft.connection(),
            draft: None,
        },
        base.control.clone(),
        base.cancel.clone(),
        None,
        None,
    );
    let (mut state, _) = actor.initial.lock().unwrap().take().unwrap();
    actor.control.resume(1, state.activity);
    actor.control.fixture_ready(1, state.activity);
    actor.activate();
    let deadline = state.initial_preparation.unwrap();
    actor.observe(&mut state, snapshot(1, vec![segment(1, "周三", false)]));
    let activity = state.activity;
    actor.native_done(
        &mut state,
        &NativeKind::InitialBegin { activity },
        NativeValue::Begun(Err(Error::new("inputComposition"))),
    );
    actor.observe(&mut state, snapshot(2, vec![segment(1, "周三开会", true)]));
    assert_eq!(state.latest.text, "周三开会");
    assert_eq!(state.initial_preparation, Some(deadline));
    assert_eq!(actor.control.audio.current(), 1);
    assert_eq!(state.turn, 1);
    assert!(native.try_recv().is_err());
    actor.native_done(
        &mut state,
        &NativeKind::InitialBegin { activity },
        NativeValue::Begun(Ok(draft)),
    );
    assert!(state.initial_preparation.is_none());
    assert_eq!(actor.control.audio.current(), 1);
    actor.schedule(&mut state).unwrap();
    let Command::ReplaceTracked(_, text, raw, id, _, reply) = command(&native).await else {
        panic!("the existing tracked write must receive the complete first source")
    };
    assert_eq!(text, "周三开会");
    assert_eq!(raw.as_deref(), Some("周三开会"));
    reply.send(Ok(tracked_snapshot(&id, &text))).unwrap();
    acknowledge(&actor, &mut state).await;
    assert_eq!(state.written_raw, "周三开会");
    actor.observe(
        &mut state,
        snapshot(3, vec![segment(1, "周三开会地点上海", true)]),
    );
    actor.schedule(&mut state).unwrap();
    assert_eq!(actor.control.audio.current(), 1);
}

#[tokio::test]
async fn end_wins_over_a_pending_initial_begin_without_writing_or_new_audio_turn() {
    let (actor, mut state, native) = production_actor(None);
    let draft = state.draft.take().unwrap();
    state.initial_preparation = Some(Instant::now() + Duration::from_secs(15));
    actor.observe(
        &mut state,
        snapshot(1, vec![segment(1, "保留这次口述", true)]),
    );
    actor.control.request_finish();
    let activity = state.activity;
    actor.native_done(
        &mut state,
        &NativeKind::InitialBegin { activity },
        NativeValue::Begun(Ok(draft)),
    );
    assert!(state.report.result_only);
    assert!(state.initial_preparation.is_none());
    actor.schedule(&mut state).unwrap();
    let Command::Finish(_, text, mode, _, _, _) = command(&native).await else {
        panic!("an end-raced empty range may only be handed off")
    };
    assert!(text.is_empty());
    assert!(matches!(mode, Finish::Handoff(_)));
    assert_eq!(state.latest.text, "保留这次口述");
    assert_eq!(state.turn, 1);
}

#[tokio::test]
async fn pending_initial_deadline_uses_result_delivery_without_dropping_recognition() {
    let (actor, mut state, _native) = production_actor(None);
    state.draft = None;
    state.initial_preparation = Some(Instant::now());
    actor.control.resume(1, state.activity);
    actor.control.fixture_ready(1, state.activity);
    actor.observe(
        &mut state,
        snapshot(1, vec![segment(1, "等待期间说的话", true)]),
    );
    actor.schedule(&mut state).unwrap();
    assert!(state.report.result_only);
    assert_eq!(actor.control.audio.current(), 1);
    assert!(state.stop_error.is_none());
    actor.native_done(
        &mut state,
        &NativeKind::RetireClose,
        NativeValue::Closed(Ok(())),
    );
    actor.observe(
        &mut state,
        snapshot(2, vec![segment(1, "等待期间说的话还在继续", true)]),
    );
    assert_eq!(state.latest.text, "等待期间说的话还在继续");
    assert_eq!(state.turn, 1);
    assert_eq!(actor.control.audio.current(), 1);
}

#[tokio::test(start_paused = true)]
async fn initial_begin_returning_after_the_original_deadline_only_retires_its_empty_range() {
    for check_inflight_deadline in [false, true] {
        let (actor, mut state, native) = production_actor(None);
        let draft = state.draft.take().unwrap();
        state.initial_preparation = Some(Instant::now() + Duration::from_secs(15));
        actor.control.resume(1, state.activity);
        actor.control.fixture_ready(1, state.activity);
        actor.observe(
            &mut state,
            snapshot(1, vec![segment(1, "等待中的完整口述", true)]),
        );
        tokio::time::advance(Duration::from_millis(14_900)).await;
        let (reply, returned) = oneshot::channel();
        state.native = Some(NativeJob {
            kind: NativeKind::InitialBegin {
                activity: state.activity,
            },
            job: tokio::spawn(async move {
                returned.await.unwrap();
                NativeValue::Begun(Ok(draft))
            }),
        });
        tokio::time::advance(Duration::from_millis(200)).await;
        if check_inflight_deadline {
            actor.schedule(&mut state).unwrap();
            assert!(state.report.result_only);
            assert!(matches!(
                state.native.as_ref().unwrap().kind,
                NativeKind::InitialBegin { .. }
            ));
            assert!(
                native.try_recv().is_err(),
                "the in-flight Begin must settle before cleanup"
            );
        }
        reply.send(()).unwrap();
        acknowledge(&actor, &mut state).await;
        assert!(
            state.report.result_only,
            "a late Begin cannot grant write permission"
        );
        actor.schedule(&mut state).unwrap();
        let Command::Finish(_, text, mode, _, _, _) = command(&native).await else {
            panic!("the late empty range must be handed off without writing ASR")
        };
        assert!(text.is_empty());
        assert!(matches!(mode, Finish::Handoff(_)));
        assert_eq!(state.latest.text, "等待中的完整口述");
        assert_eq!(state.turn, 1);
        assert_eq!(actor.control.audio.current(), 1);
    }
}
#[cfg(windows)]
#[path = "drafts_owned_test.rs"]
mod owned;
#[cfg(windows)]
pub(super) fn readonly_handoff_fixture_barrier() {
    owned::readonly_handoff_barrier();
}
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

#[tokio::test]
async fn ending_a_resumed_empty_range_releases_it_without_an_uncertain_commit() {
    let (actor, mut state, native) = actor(None);
    let (reply, _) = oneshot::channel();
    state.ending = Some(Ending {
        mode: Finish::Commit,
        reply,
        deadline: Instant::now() + Duration::from_secs(5),
        expired: false,
    });
    actor.schedule_final(&mut state).unwrap();
    let Command::Finish(_, text, mode, _, _, reply) = command(&native).await else {
        panic!("release empty range expected")
    };
    assert!(text.is_empty());
    assert_eq!(mode, Finish::Handoff("editing".into()));
    reply
        .send(FinishOutcome {
            delivery: Delivery::not_submitted("editing"),
            committed: false,
            handed_off: true,
            cancelled: false,
            rollback_failed: false,
        })
        .ok()
        .unwrap();
    let value = SessionDraft::native_value(&mut state.native).await.unwrap();
    let job = state.native.take().unwrap();
    let delivery = actor.native_done(&mut state, &job.kind, value).unwrap();
    assert_eq!(delivery.submission, Submission::NotSubmitted);
    assert!(!delivery.restore_error);
    assert!(state.report.raw.is_empty());
}

#[tokio::test]
async fn finish_intent_prevents_another_resume_or_late_replacement_before_end_arrives() {
    let (actor, mut state, native) = actor(None);
    state.draft = None;
    state.paused_at = Some(Instant::now() - Duration::from_secs(1));
    actor.control.request_finish();
    actor.schedule(&mut state).unwrap();
    assert!(state.native.is_none());
    assert!(native.try_recv().is_err());
    // Even a late transcript cannot arrange another write in the old turn.
    actor.observe(
        &mut state,
        snapshot(99, vec![segment(1, "旧识别全文", false)]),
    );
    actor.schedule(&mut state).unwrap();
    assert!(state.native.is_none());
    assert!(native.try_recv().is_err());
}

#[test]
fn retired_resume_rejection_cannot_replace_the_user_end_but_cleanup_failure_survives() {
    for ending_received in [false, true] {
        for code in [
            "targetReadonlyContext",
            "targetChanged",
            "editing",
            "staleSnapshot",
        ] {
            let (actor, mut state, _native) = actor(None);
            actor.control.request_finish();
            state.stopping = ending_received;
            let activity = state.activity;
            actor.native_done(
                &mut state,
                &NativeKind::Begin { turn: 2, activity },
                NativeValue::Begun(Err(Error::new(code))),
            );
            assert!(
                state.stop_error.is_none(),
                "{code}, end_received={ending_received}"
            );
            assert!(actor.control.stopping());
        }
    }
    let (actor, mut state, _native) = actor(None);
    actor.control.request_finish();
    let activity = state.activity;
    actor.native_done(
        &mut state,
        &NativeKind::Begin { turn: 2, activity },
        NativeValue::Begun(Err(Error::new("compositionCleanup"))),
    );
    assert_eq!(state.stop_error.as_deref(), Some("compositionCleanup"));
}

#[tokio::test]
async fn late_resume_success_is_released_without_replaying_the_edited_body() {
    let (actor, mut state, native) = actor(None);
    let draft = state.draft.take().unwrap();
    actor.control.request_finish();
    let activity = state.activity;
    actor.native_done(
        &mut state,
        &NativeKind::Begin { turn: 2, activity },
        NativeValue::Begun(Ok(draft)),
    );
    assert!(state.handoff);
    actor.schedule(&mut state).unwrap();
    let Command::Finish(_, text, mode, _, _, reply) = command(&native).await else {
        panic!("retired lease must hand off, never replace")
    };
    assert!(text.is_empty());
    assert!(matches!(mode, Finish::Handoff(_)));
    reply
        .send(FinishOutcome {
            delivery: Delivery::not_submitted("editing"),
            committed: false,
            handed_off: true,
            cancelled: false,
            rollback_failed: false,
        })
        .ok()
        .unwrap();
    acknowledge(&actor, &mut state).await;
    actor.schedule(&mut state).unwrap();
    assert!(state.native.is_none());
    assert!(native.try_recv().is_err());
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

#[tokio::test]
async fn input_failure_retires_writing_while_the_original_source_continues_until_finish() {
    let (actor, mut state, native) = actor(None);
    bound_preview(&actor, &mut state, &native, "周三开会").await;
    actor.control.resume(state.turn, state.activity);
    actor.control.fixture_ready(state.turn, state.activity);
    let turn = state.turn;
    state.record_failure(&actor.task, "targetLost".into());
    actor.detach_failed_input(&mut state);
    assert!(!state.stopping);
    assert!(!actor.control.stopping());
    assert_eq!(*actor.cancel.borrow(), 0);
    assert_eq!(actor.control.audio.current(), turn);
    actor.schedule(&mut state).unwrap();
    let Command::Finish(_, text, mode, _, _, reply) = command(&native).await else {
        panic!("retire original composition without another body write")
    };
    assert!(text.is_empty());
    assert!(matches!(mode, Finish::Handoff(_)));
    reply
        .send(FinishOutcome {
            delivery: Delivery {
                submission: Submission::Submitted,
                state: "unconfirmed".into(),
                stage: "composition".into(),
                ..Delivery::not_submitted("inputUnconfirmed")
            },
            committed: false,
            handed_off: true,
            cancelled: false,
            rollback_failed: false,
        })
        .ok()
        .unwrap();
    acknowledge(&actor, &mut state).await;
    // Close completion is independent of recording. Drive its real actor
    // receipt here; the connection worker has separate retirement tests.
    actor.native_done(
        &mut state,
        &NativeKind::RetireClose,
        NativeValue::Closed(Ok(())),
    );
    actor.observe(
        &mut state,
        snapshot(2, vec![segment(1, "周三开会下周一上午开会", false)]),
    );
    actor.start_generation(&mut state).unwrap();
    actor.schedule(&mut state).unwrap();
    assert!(state.native.is_none());
    assert!(native.try_recv().is_err());
    assert_eq!(state.turn, turn);
    assert_eq!(actor.control.audio.current(), turn);
    assert!(
        actor.result_delivery(&mut state).is_none(),
        "input cleanup cannot finish recording"
    );
    let (reply, _) = oneshot::channel();
    state.ending = Some(Ending {
        mode: Finish::Commit,
        reply,
        deadline: Instant::now() + Duration::from_secs(15),
        expired: false,
    });
    let delivery = actor.result_delivery(&mut state).unwrap();
    assert_eq!(delivery.submission, Submission::Submitted);
    assert_eq!(state.report.text, "周三开会");
    assert_eq!(state.report.result_raw, "周三开会下周一上午开会");
    assert!(state.report.result_only);
    assert_eq!(state.stop_error.as_deref(), Some("targetLost"));
}

#[tokio::test]
async fn in_flight_write_must_settle_before_retirement_without_replay_or_new_source() {
    let (actor, mut state, native) = actor(None);
    state.revisable = Some(revisable::State::new());
    actor.observe(&mut state, snapshot(1, vec![segment(1, "首批文字", false)]));
    actor.schedule(&mut state).unwrap();
    let Command::ReplaceTracked(_, text, _, id, _, reply) = command(&native).await else {
        panic!("tracked write")
    };
    state.record_failure(&actor.task, "targetChanged".into());
    actor.detach_failed_input(&mut state);
    actor.schedule(&mut state).unwrap();
    assert!(
        native.try_recv().is_err(),
        "in-flight write must settle before cleanup"
    );
    reply.send(Ok(tracked_snapshot(&id, &text))).unwrap();
    acknowledge(&actor, &mut state).await;
    actor.observe(
        &mut state,
        snapshot(2, vec![segment(1, "首批文字后续文字", false)]),
    );
    actor.schedule(&mut state).unwrap();
    let Command::Finish(_, text, mode, _, _, _) = command(&native).await else {
        panic!("only handoff may follow")
    };
    assert!(text.is_empty());
    assert!(matches!(mode, Finish::Handoff(_)));
    assert_eq!(state.turn, 1);
    assert_eq!(state.written_raw, "首批文字");
    assert_eq!(state.latest.text, "首批文字后续文字");
}

#[test]
fn cancelled_result_keeps_uncertain_input_facts_and_has_no_copy_text() {
    let (actor, mut state, _native) = actor(None);
    state.input_delivery = delivery::InputDelivery::Result(Delivery {
        submission: Submission::Uncertain,
        ..Delivery::not_submitted("compositionPartial")
    });
    let (reply, _) = oneshot::channel();
    state.ending = Some(Ending {
        mode: Finish::Abort,
        reply,
        deadline: Instant::now() + Duration::from_secs(15),
        expired: false,
    });
    state.latest = snapshot(3, vec![segment(1, "保留识别原文", false)]);
    let delivery = actor.result_delivery(&mut state).unwrap();
    assert_eq!(delivery.submission, Submission::Uncertain);
    assert_eq!(delivery.error.as_deref(), Some("cancelled"));
    assert!(state.report.result_raw.is_empty());
    assert!(state.report.cancelled);
}
#[test]
fn input_failure_during_final_drain_keeps_result_but_cancel_retains_its_own_policy() {
    for mode in [Finish::Commit, Finish::Abort] {
        let (actor, mut state, _) = actor(None);
        state.stopping = true;
        actor.control.request_finish();
        let (reply, _) = oneshot::channel();
        state.ending = Some(Ending {
            mode: mode.clone(),
            reply,
            deadline: Instant::now() + Duration::from_secs(15),
            expired: false,
        });
        state.record_failure(&actor.task, "compositionPartial".into());
        actor.detach_failed_input(&mut state);
        assert_eq!(state.report.result_only, mode == Finish::Commit);
        assert!(actor.control.stopping());
    }
}

#[tokio::test]
async fn tracking_failure_preserves_the_completed_body_and_never_rebinds_or_replays() {
    let (actor, mut state, native) = actor(None);
    state.revisable = Some(revisable::State::new());
    actor.observe(&mut state, snapshot(1, vec![segment(1, "已经写入", false)]));
    actor.schedule(&mut state).unwrap();
    let Command::ReplaceTracked(_, text, _, _, _, reply) = command(&native).await else {
        panic!("tracked write")
    };
    reply
        .send(Err(windows_input::composition::WriteFailure {
            error: Error::new("compositionPartial"),
            mutation: Mutation::Changed,
            tracking_failed: true,
            scope_unavailable: false,
        }))
        .unwrap();
    acknowledge(&actor, &mut state).await;
    assert_eq!(text, "已经写入");
    assert_eq!(state.written_raw, "已经写入");
    assert!(!state.revisable.as_ref().unwrap().uncertain());
    actor.detach_failed_input(&mut state);
    actor.observe(
        &mut state,
        snapshot(2, vec![segment(1, "已经写入继续口述", false)]),
    );
    actor.schedule(&mut state).unwrap();
    let Command::Finish(_, text, mode, _, _, _) = command(&native).await else {
        panic!("only cleanup follows")
    };
    assert!(text.is_empty());
    assert!(matches!(mode, Finish::Handoff(_)));
    assert!(!actor.control.stopping());
    assert_eq!(state.latest.text, "已经写入继续口述");
}

#[tokio::test]
async fn revisable_preview_does_not_commit_and_next_request_rewrites_full_block() {
    let fixture = Fixture::start(vec![response("周三开会。"), response("周四开会。")]);
    let root = tempfile::tempdir().unwrap();
    let data = crate::data::DataWorker::start(root.path().to_path_buf()).unwrap();
    let context = Context::fixture(fixture.provider.clone(), fixture.input(), data.clone());
    let (actor, mut state, native) = actor(Some(context));
    state.revisable = Some(revisable::State::new());
    actor.observe(&mut state, snapshot(1, vec![segment(1, "周三开会", true)]));
    state.merge_at = Some(Instant::now() - Duration::from_millis(151));
    actor.start_generation(&mut state).unwrap();
    actor.schedule(&mut state).unwrap();
    assert!(
        matches!(state.unit, Some(UnitSlot::Generating(_))),
        "scheduling must retain the running request"
    );
    let Command::ReplaceTracked(_, text, _, id, _, reply) = command(&native).await else {
        panic!("tracked real-time preview expected")
    };
    assert_eq!(text, "周三开会");
    reply.send(Ok(tracked_snapshot(&id, &text))).unwrap();
    acknowledge(&actor, &mut state).await;
    generated(&mut state).await;
    actor.schedule(&mut state).unwrap();
    let Command::Replace(_, text, _, _, reply) = command(&native).await else {
        panic!("model preview must replace, never Settle")
    };
    assert_eq!(text, "周三开会。");
    reply.send(Ok(())).unwrap();
    acknowledge(&actor, &mut state).await;
    assert!(state.submitted.is_empty());
    assert!(state.report.text.is_empty());
    actor.observe(
        &mut state,
        snapshot(
            2,
            vec![segment(1, "周三开会", true), segment(2, "不对是周四", true)],
        ),
    );
    state.merge_at = Some(Instant::now() - Duration::from_millis(151));
    actor.start_generation(&mut state).unwrap();
    assert_eq!(
        state.unit.as_ref().unwrap().source().text,
        "周三开会不对是周四"
    );
    generated(&mut state).await;
    actor.schedule(&mut state).unwrap();
    let Command::Replace(_, text, _, _, reply) = command(&native).await else {
        panic!("whole corrected preview expected")
    };
    assert_eq!(text, "周四开会。");
    reply.send(Ok(())).unwrap();
    acknowledge(&actor, &mut state).await;
    assert_eq!(fixture.seen.lock().unwrap().len(), 2);
    assert!(
        state.report.text.is_empty(),
        "pause/preview must not finalize a block"
    );
    drop(state);
    drop(actor);
    data.shutdown().await;
}

#[tokio::test]
async fn revisable_stop_uses_handoff_and_does_not_append_unshown_tail() {
    let (actor, mut state, native) = actor(None);
    state.revisable = Some(revisable::State::new());
    actor.observe(&mut state, snapshot(1, vec![segment(1, "已经显示", false)]));
    actor.schedule(&mut state).unwrap();
    let Command::ReplaceTracked(_, text, _, id, _, reply) = command(&native).await else {
        panic!("preview expected")
    };
    assert_eq!(text, "已经显示");
    reply.send(Ok(tracked_snapshot(&id, &text))).unwrap();
    acknowledge(&actor, &mut state).await;
    actor.observe(
        &mut state,
        snapshot(2, vec![segment(1, "已经显示尚未显示", false)]),
    );
    let (reply, _) = oneshot::channel();
    state.ending = Some(Ending {
        mode: Finish::Abort,
        reply,
        deadline: Instant::now() + Duration::from_secs(15),
        expired: false,
    });
    state.stopping = true;
    actor.schedule(&mut state).unwrap();
    // Stop retains the already tracked visible text and never calls native Abort.
    let Command::Finish(_, text, mode, _, keep, reply) = command(&native).await else {
        panic!("retained handoff expected")
    };
    assert!(text.is_empty());
    assert!(matches!(mode, Finish::Handoff(_)));
    assert!(keep);
    reply
        .send(FinishOutcome {
            delivery: Delivery::not_submitted("editing"),
            committed: false,
            handed_off: true,
            cancelled: false,
            rollback_failed: false,
        })
        .ok()
        .unwrap();
    acknowledge(&actor, &mut state).await;
    actor.schedule(&mut state).unwrap();
    let Command::Block(_, _, request, _, _, reply) = command(&native).await else {
        panic!("release tracked stopped block")
    };
    assert_eq!(
        request.action,
        sayweave_text_service::wire::BlockAction::Release
    );
    reply
        .send(Ok(block_reply(&actor.task, None, true)))
        .unwrap();
    acknowledge(&actor, &mut state).await;
    actor.schedule(&mut state).unwrap();
    assert_eq!(state.report.text, "已经显示");
    assert_eq!(state.report.raw, "已经显示");
}
#[tokio::test]
async fn older_equal_preview_cannot_skip_the_final_append_snapshot() {
    let fixture = Fixture::start(vec![response("周三开会"), response("周四开会")]);
    let root = tempfile::tempdir().unwrap();
    let data = crate::data::DataWorker::start(root.path().to_path_buf()).unwrap();
    let context = Context::fixture(fixture.provider.clone(), fixture.input(), data.clone());
    let (actor, mut state, native) = actor(Some(context));
    bound_preview(&actor, &mut state, &native, "周三开会").await;
    actor.observe(&mut state, snapshot(2, vec![segment(1, "周三开会", true)]));
    state.merge_at = Some(Instant::now() - Duration::from_millis(151));
    actor.start_generation(&mut state).unwrap();
    generated(&mut state).await;
    actor.observe(
        &mut state,
        snapshot(
            3,
            vec![segment(1, "周三开会", true), segment(2, "不对周四", true)],
        ),
    );
    let (reply, _) = oneshot::channel();
    state.ending = Some(Ending {
        mode: Finish::Commit,
        reply,
        deadline: Instant::now() + Duration::from_secs(15),
        expired: false,
    });
    state.stopping = true;
    actor.schedule(&mut state).unwrap();
    assert!(matches!(state.unit, Some(UnitSlot::Generating(_))));
    assert_eq!(
        state.unit.as_ref().unwrap().source().text,
        "周三开会不对周四"
    );
    generated(&mut state).await;
    assert_eq!(fixture.seen.lock().unwrap().len(), 2);
    drop(state);
    drop(actor);
    data.shutdown().await;
}
pub(super) fn actor(
    context: Option<Context>,
) -> (Arc<SessionDraft>, State, std::sync::mpsc::Receiver<Command>) {
    let (actor, mut state, native) = production_actor(context);
    state.revisable = None; // Legacy policy remains an independently tested rollback.
    (actor, state, native)
}
pub(super) fn production_actor(
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
        editor: crate::windows_input::EditorEndpoint {
            window: 1,
            process: 1,
            thread: 1,
            lifetime: 1,
        },
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
pub(super) fn block_reply(
    task: &str,
    snapshot: Option<sayweave_text_service::wire::BlockSnapshot>,
    applied: bool,
) -> sayweave_text_service::wire::Reply {
    sayweave_text_service::wire::Reply {
        block: snapshot,
        engine: sayweave_text_service::wire::EngineBinding {
            proxy_instance: uuid::Uuid::new_v4().to_string(),
            engine_generation: 1,
        },
        task: task.into(),
        turn: 1,
        operation: 1,
        revision: 1,
        applied,
        code: "blockFixture".into(),
        mutation: Mutation::NotAttempted,
        runtime: None,
        range_epoch: 0,
        settlement: None,
        edit_observation: None,
    }
}
fn tracked_snapshot(id: &str, text: &str) -> sayweave_text_service::wire::BlockSnapshot {
    sayweave_text_service::wire::BlockSnapshot {
        id: id.into(),
        epoch: 1,
        version: 1,
        lease: 0,
        text: text.into(),
        selection_start: text.encode_utf16().count() as u32,
        selection_end: text.encode_utf16().count() as u32,
        writable: false,
    }
}
async fn bound_preview(
    actor: &SessionDraft,
    state: &mut State,
    native: &std::sync::mpsc::Receiver<Command>,
    body: &str,
) -> String {
    state.revisable = Some(revisable::State::new());
    actor.observe(state, snapshot(1, vec![segment(1, body, false)]));
    actor.schedule(state).unwrap();
    let Command::ReplaceTracked(_, text, _, id, _, reply) = command(native).await else {
        panic!("atomic tracked preview")
    };
    assert_eq!(text, body);
    reply.send(Ok(tracked_snapshot(&id, body))).unwrap();
    acknowledge(actor, state).await;
    id
}
#[tokio::test]
async fn manual_edit_rebases_same_block_and_new_speech_never_replays_the_old_turn() {
    let fixture = Fixture::start(vec![
        response("周五下午开会。"),
        response("下周一开会。"),
        response("下周一下午\n开会。"),
    ]);
    let root = tempfile::tempdir().unwrap();
    let data = crate::data::DataWorker::start(root.path().to_path_buf()).unwrap();
    let context = Context::fixture(fixture.provider.clone(), fixture.input(), data.clone());
    let (actor, mut state, native) = actor(Some(context));
    let id = bound_preview(&actor, &mut state, &native, "周三开会").await;
    state.revisable.as_mut().unwrap().handoff();
    state.handoff = true;
    state.paused_at = Some(Instant::now());
    actor.schedule(&mut state).unwrap();
    let Command::Finish(_, _, mode, _, _, reply) = command(&native).await else {
        panic!("handoff")
    };
    assert!(matches!(mode, Finish::Handoff(_)));
    reply
        .send(FinishOutcome {
            delivery: Delivery::not_submitted("editing"),
            committed: false,
            handed_off: true,
            cancelled: false,
            rollback_failed: false,
        })
        .ok()
        .unwrap();
    acknowledge(&actor, &mut state).await;
    assert_eq!(state.turn, 1);
    assert_eq!(state.latest.text, "周三开会");
    // Clicking inside the block precedes the actual text edit. A successful
    // native read of unchanged text must not discard the tracked block just
    // because that caret is not the previous ASR insertion endpoint.
    let mut positioned = tracked_snapshot(&id, "周三开会");
    positioned.selection_start = 1;
    positioned.selection_end = 1;
    let activity = input_state::activity();
    actor.native_done(
        &mut state,
        &NativeKind::Revisable(revisable::Effect::Read { activity }),
        NativeValue::block(Ok(block_reply(&actor.task, Some(positioned), true))),
    );
    assert!(
        state.native.is_none(),
        "caret-only read must not release the block"
    );
    assert_eq!(state.turn, 1);
    assert_eq!(state.latest.text, "周三开会");
    assert!(state.paused_at.is_none());
    // The selection-only read has now resumed successfully. The following
    // real keystroke opens a new handoff before its edited snapshot arrives.
    state.revisable.as_mut().unwrap().handoff();
    // No composition is held after the preceding successful handoff. Model
    // the next read after the actor has settled its no-composition handoff.
    let edited = sayweave_text_service::wire::BlockSnapshot {
        id,
        epoch: 1,
        version: 2,
        lease: 2,
        text: "周四下午开会".into(),
        selection_start: 6,
        selection_end: 6,
        writable: false,
    };
    let activity = input_state::activity();
    assert!(
        actor
            .native_done(
                &mut state,
                &NativeKind::Revisable(revisable::Effect::Read { activity }),
                NativeValue::block(Ok(block_reply(&actor.task, Some(edited), true)))
            )
            .is_none()
    );
    let next = snapshot(
        2,
        vec![
            segment(1, "周三开会", true),
            segment(42, "不对改为周五", true),
        ],
    );
    actor.observe(&mut state, next);
    state.merge_at = Some(Instant::now() - Duration::from_millis(151));
    actor.start_generation(&mut state).unwrap();
    let source = state.unit.as_ref().unwrap().source();
    assert_eq!(source.text, "周四下午开会不对改为周五");
    assert_eq!(
        source.draft.as_ref().unwrap().source.baseline_kind,
        crate::processing::draft::BaselineKind::EditorSnapshot
    );
    assert_eq!(
        source
            .draft
            .as_ref()
            .unwrap()
            .asr_sources
            .iter()
            .map(|s| s.segment)
            .collect::<Vec<_>>(),
        vec![1, 42],
        "the same logical block keeps earlier provenance without replaying it"
    );
    generated(&mut state).await;
    actor.schedule(&mut state).unwrap();
    let Command::Acquire(_, request, _, _, reply) = command(&native).await else {
        panic!("acquire")
    };
    assert_eq!(
        request.action,
        sayweave_text_service::wire::BlockAction::Acquire
    );
    let mut lease = sayweave_text_service::wire::BlockSnapshot {
        id: request.id,
        epoch: 1,
        version: 2,
        lease: 3,
        text: request.expected,
        selection_start: 6,
        selection_end: 6,
        writable: true,
    };
    reply
        .send(Ok(block_reply(&actor.task, Some(lease.clone()), true)))
        .unwrap();
    acknowledge(&actor, &mut state).await;
    assert!(state.draft.is_some());
    actor.schedule(&mut state).unwrap();
    let Command::Replace(_, text, _, _, reply) = command(&native).await else {
        panic!("shared Draft update")
    };
    assert_eq!(text, "周五下午开会。");
    reply.send(Ok(())).unwrap();
    acknowledge(&actor, &mut state).await;
    assert!(state.report.raw.is_empty());
    assert!(state.report.text.is_empty());
    let mut sources = vec![
        segment(1, "周三开会", true),
        segment(42, "不对改为周五", true),
    ];
    for (round, baseline, spoken, expected) in [
        (0, "周五开会。", "改成下周一", "下周一开会。"),
        (1, "下周一\n开会。", "改成下午", "下周一下午\n开会。"),
    ] {
        state.revisable.as_mut().unwrap().handoff();
        state.handoff = true;
        actor.schedule(&mut state).unwrap();
        let Command::Finish(_, _, mode, _, _, reply) = command(&native).await else {
            panic!("later handoff")
        };
        assert!(matches!(mode, Finish::Handoff(_)));
        reply
            .send(FinishOutcome {
                delivery: Delivery::not_submitted("editing"),
                committed: false,
                handed_off: true,
                cancelled: false,
                rollback_failed: false,
            })
            .ok()
            .unwrap();
        acknowledge(&actor, &mut state).await;
        let previous = lease.clone();
        let edited = sayweave_text_service::wire::BlockSnapshot {
            version: previous.version + 2,
            lease: previous.lease + 2,
            text: baseline.into(),
            selection_start: baseline.encode_utf16().count() as u32,
            selection_end: baseline.encode_utf16().count() as u32,
            writable: false,
            ..previous
        };
        actor.native_done(
            &mut state,
            &NativeKind::Revisable(revisable::Effect::Read { activity }),
            NativeValue::block(Ok(block_reply(&actor.task, Some(edited), true))),
        );
        assert_eq!(state.turn, 1);
        // A changed-length revision of absorbed ASR must not restore deleted text.
        sources[0] = segment(1, "周三下午开会", true);
        sources.push(segment(43 + round, spoken, true));
        actor.observe(&mut state, snapshot(3 + round, sources.clone()));
        state.merge_at = Some(Instant::now() - Duration::from_millis(151));
        actor.start_generation(&mut state).unwrap();
        assert_eq!(
            state.unit.as_ref().unwrap().source().text,
            format!("{baseline}{spoken}")
        );
        generated(&mut state).await;
        actor.schedule(&mut state).unwrap();
        let Command::Acquire(_, request, _, _, reply) = command(&native).await else {
            panic!("later acquire")
        };
        lease = sayweave_text_service::wire::BlockSnapshot {
            id: request.id,
            epoch: request.epoch,
            version: request.version,
            lease: request.lease + 1,
            text: request.expected,
            selection_start: request.selection_start,
            selection_end: request.selection_end,
            writable: true,
        };
        reply
            .send(Ok(block_reply(&actor.task, Some(lease.clone()), true)))
            .unwrap();
        acknowledge(&actor, &mut state).await;
        actor.schedule(&mut state).unwrap();
        let Command::Replace(_, text, _, _, reply) = command(&native).await else {
            panic!("later shared writer")
        };
        assert_eq!(text, expected);
        reply.send(Ok(())).unwrap();
        acknowledge(&actor, &mut state).await;
    }
    drop(state);
    drop(actor);
    data.shutdown().await;
}
#[tokio::test]
async fn capacity_delivers_one_block_and_keeps_the_same_dictation_open() {
    let (actor, mut state, native) = actor(None);
    state.revisable = Some(revisable::State::new());
    let first = "甲。".repeat(1000);
    let whole = format!("{first}后续文字");
    actor.observe(&mut state, snapshot(1, vec![segment(1, &whole, true)]));
    actor.start_generation(&mut state).unwrap();
    actor.schedule(&mut state).unwrap();
    let Command::ReplaceTracked(_, text, _, id, _, reply) = command(&native).await else {
        panic!("first tracked preview")
    };
    assert_eq!(text, first);
    reply.send(Ok(tracked_snapshot(&id, &first))).unwrap();
    acknowledge(&actor, &mut state).await;
    actor.schedule(&mut state).unwrap();
    let Command::Block(_, _, request, _, _, reply) = command(&native).await else {
        panic!("release prior block")
    };
    assert_eq!(
        request.action,
        sayweave_text_service::wire::BlockAction::Release
    );
    reply
        .send(Ok(block_reply(&actor.task, None, true)))
        .unwrap();
    acknowledge(&actor, &mut state).await;
    actor.schedule(&mut state).unwrap();
    let Command::Settle(_, prefix, tail, _, _, reply) = command(&native).await else {
        panic!("capacity settlement")
    };
    assert_eq!(prefix, first);
    assert!(tail.is_empty());
    reply
        .send(PrefixOutcome {
            prefix: Mutation::Changed,
            tail_owned: true,
            error: None,
        })
        .ok()
        .unwrap();
    acknowledge(&actor, &mut state).await;
    assert!(!state.stopping);
    assert!(state.draft.is_some());
    assert_eq!(state.report.text, first);
    actor.schedule(&mut state).unwrap();
    let Command::ReplaceTracked(_, text, _, id, _, reply) = command(&native).await else {
        panic!("continuing tracked tail")
    };
    assert_eq!(text, "后续文字");
    reply.send(Ok(tracked_snapshot(&id, &text))).unwrap();
    acknowledge(&actor, &mut state).await;
    assert!(!state.stopping);
    assert_eq!(state.report.raw, first);
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
pub(super) async fn command(native: &std::sync::mpsc::Receiver<Command>) -> Command {
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
pub(super) async fn acknowledge(actor: &SessionDraft, state: &mut State) {
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
    let Command::Replace(_, text, _, _, reply) = command(&native).await else {
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

#[test]
fn first_live_failure_survives_secondary_teardown_but_keeps_uncertain_effects() {
    let (_actor, mut state, _native) = actor(None);
    state.record_failure("actor-test", "compositionRejected".into());
    state.record_failure("actor-test", "staleSnapshot".into());
    let mut delivery = Delivery::not_submitted("staleSnapshot");
    state.terminal_failure("actor-test", &mut delivery);
    assert_eq!(state.stop_error.as_deref(), Some("compositionRejected"));
    assert_eq!(state.report.error.as_deref(), Some("compositionRejected"));
    assert_eq!(delivery.error.as_deref(), Some("compositionRejected"));
    assert_eq!(state.secondary_errors, vec!["staleSnapshot"]);

    delivery.submission = Submission::Uncertain;
    delivery.restore_error = true;
    delivery.error = Some("compositionCleanup".into());
    state.terminal_failure("actor-test", &mut delivery);
    assert_eq!(delivery.error.as_deref(), Some("compositionCleanup"));
    assert!(delivery.restore_error);
    assert_eq!(delivery.submission, Submission::Uncertain);
    assert_eq!(state.report.error.as_deref(), Some("compositionRejected"));
    assert_eq!(
        state.secondary_errors,
        vec!["staleSnapshot", "compositionCleanup"]
    );
}

#[test]
fn failure_diagnostics_are_bounded_and_cancel_keeps_its_terminal_result() {
    let (_actor, mut state, _native) = actor(None);
    state.record_failure("actor-test", "compositionRejected".into());
    for n in 0..20 {
        state.record_failure("actor-test", format!("secondary{n}"));
    }
    assert_eq!(state.secondary_errors.len(), 8);
    state.report.cancelled = true;
    let mut delivery = Delivery::not_submitted("cancelled");
    state.terminal_failure("actor-test", &mut delivery);
    assert_eq!(delivery.error.as_deref(), Some("cancelled"));
    assert_eq!(state.stop_error.as_deref(), Some("compositionRejected"));
}

#[test]
fn retained_audio_resume_publishes_settled_view_without_readiness_event() {
    let (actor, mut state, _native) = actor(None);
    actor.control.resume(state.turn, state.activity);
    actor.control.fixture_ready(state.turn, state.activity);
    let mut readiness = actor.control.readiness();
    let _ = readiness.borrow_and_update();
    let values = Mutex::new(Vec::new());
    let publish = |value| values.lock().unwrap().push(value);
    let mut published = None;
    actor.publish_projected(&state, &mut published, &publish);
    actor.control.hold_input();
    state.handoff = true;
    state.paused_at = Some(Instant::now());
    actor.publish_projected(&state, &mut published, &publish);
    actor.publish_projected(&state, &mut published, &publish);
    assert_eq!(actor.control.audio.current(), 0);
    state.handoff = false;
    state.paused_at = None;
    actor.control.resume_retained(state.turn, state.activity);
    assert!(!readiness.has_changed().unwrap());
    assert_eq!(actor.control.audio.current(), state.turn);
    actor.publish_projected(&state, &mut published, &publish);
    assert_eq!(
        *values.lock().unwrap(),
        vec![
            Presentation::Recording,
            Presentation::Editing,
            Presentation::Recording
        ]
    );
    actor.control.request_finish();
    state.handoff = true;
    actor.publish_projected(&state, &mut published, &publish);
    state.handoff = false;
    actor.publish_projected(&state, &mut published, &publish);
    assert_eq!(
        values.lock().unwrap().len(),
        3,
        "late transitions cannot reopen a finishing task"
    );
}
#[test]
fn result_route_change_is_published_even_when_recording_presentation_is_unchanged() {
    let (actor, mut state, _) = actor(None);
    actor.control.resume(state.turn, state.activity);
    actor.control.fixture_ready(state.turn, state.activity);
    let values = Mutex::new(Vec::new());
    let publish = |value| values.lock().unwrap().push((value, actor.result_only()));
    let mut published = None;
    actor.publish_projected(&state, &mut published, &publish);
    state.record_failure(&actor.task, "targetChanged".into());
    actor.detach_failed_input(&mut state);
    actor.publish_projected(&state, &mut published, &publish);
    actor.publish_projected(&state, &mut published, &publish);
    assert_eq!(
        *values.lock().unwrap(),
        vec![
            (Presentation::Recording, false),
            (Presentation::Recording, true)
        ]
    );
}
