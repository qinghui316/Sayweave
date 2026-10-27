//! Adapt native source facts to existing sentence-oriented consumers.
//! Neither normalization nor a pause grants native input permission.
use crate::speech::recognition::{Segment, Transcript};
use crate::{history::types::Error, processing::quality::protected_ranges};
use sayweave_local_asr_protocol::{Event, RunId};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Default)]
pub(super) struct Projection {
    run: Option<RunId>,
    native_confirmed: String,
    native_full: String,
    revision: i32,
    confirmed: String,
    segments: Vec<Segment>,
    next_id: u64,
    tail_revision: u64,
    tail_text: String,
    exposed_end: usize,
    pause_sequence: u64,
    pause_sample: u64,
    pub(super) final_seen: bool,
}

/// Raw UTF-8 boundaries and their rendered counterparts come from one pass.
struct Mapped {
    text: String,
    boundaries: Vec<(usize, usize)>,
}
fn han(c: char) -> bool {
    matches!(c as u32, 0x3400..=0x4dbf | 0x4e00..=0x9fff | 0xf900..=0xfaff | 0x20000..=0x323af)
}
fn normalize(raw: &str, final_result: bool) -> Mapped {
    let protected = protected_ranges(raw);
    let mut text = String::with_capacity(raw.len());
    let mut boundaries = vec![(0, 0)];
    let chars = raw.char_indices().collect::<Vec<_>>();
    let mut i = 0;
    while i < chars.len() {
        let (start, c) = chars[i];
        if c == ' ' {
            let mut end = i + 1;
            while end < chars.len() && chars[end].1 == ' ' {
                end += 1;
            }
            let right = chars.get(end).map(|c| c.1);
            let internal = i > 0 && han(chars[i - 1].1) && right.is_some_and(han);
            let guarded = protected.iter().any(|r| r.start <= start && start < r.end);
            let hold = right.is_none() && !final_result;
            for &(at, _) in &chars[i..end] {
                if !hold && (!internal || guarded) {
                    text.push(' ');
                }
                boundaries.push((at + 1, text.len()));
            }
            i = end;
        } else {
            text.push(c);
            boundaries.push((start + c.len_utf8(), text.len()));
            i += 1;
        }
    }
    Mapped { text, boundaries }
}

impl Projection {
    pub(super) fn update(&mut self, event: Event) -> Result<Transcript, Error> {
        let Event::Update {
            id,
            revision,
            full,
            confirmed,
            pause,
            final_result,
            audio_received_ms,
            ..
        } = event
        else {
            return Err(Error::new("localProtocol"));
        };
        if self.final_seen
            || revision < self.revision
            || self.run.as_ref().is_some_and(|r| r != &id)
            || !confirmed.starts_with(&self.native_confirmed)
            || !full.starts_with(&confirmed)
            || !full.is_char_boundary(confirmed.len())
            || (self.run.is_some() && revision == self.revision && full != self.native_full)
            || (final_result && full != confirmed)
        {
            return Err(Error::new("localSourceChanged"));
        }
        let mapped = normalize(&full, final_result);
        let proof_end = mapped
            .boundaries
            .iter()
            .rev()
            .find(|(raw, _)| *raw <= confirmed.len())
            .map_or(0, |(_, at)| *at);
        if !mapped.text.starts_with(&self.confirmed) {
            return Err(Error::new("localSourceChanged"));
        }
        let mut paused = false;
        if let Some(p) = pause {
            if p.sequence == 0
                || p.native_revision != revision
                || p.sample_end == 0
                || p.sample_end
                    > u64::try_from(audio_received_ms)
                        .unwrap_or(0)
                        .saturating_mul(16)
            {
                return Err(Error::new("localProtocol"));
            }
            if p.sequence > self.pause_sequence {
                if p.sample_end <= self.pause_sample {
                    return Err(Error::new("localProtocol"));
                }
                self.pause_sequence = p.sequence;
                self.pause_sample = p.sample_end;
                paused = true;
            } else if p.sequence != self.pause_sequence || p.sample_end != self.pause_sample {
                return Err(Error::new("localStaleOperation"));
            }
        }
        self.run = Some(id);
        self.revision = revision;
        self.native_confirmed = confirmed;
        self.native_full = full;
        if self.next_id == 0 {
            self.next_id = 1;
        }
        let mut minimum = self.exposed_end.max(self.confirmed.len());
        while let Some(end) = cut(
            &mapped.text,
            self.confirmed.len(),
            proof_end,
            minimum,
            paused,
            final_result,
        ) {
            let text = mapped.text[self.confirmed.len()..end].to_string();
            self.tail_revision += 1;
            self.segments.push(Segment {
                id: self.next_id,
                sequence: self.tail_revision,
                text,
                confirmed: true,
                begin_ms: None,
                end_ms: None,
            });
            self.confirmed = mapped.text[..end].to_string();
            self.next_id += 1;
            self.tail_revision = 0;
            self.tail_text.clear();
            minimum = end;
        }
        let tail = &mapped.text[self.confirmed.len()..];
        let mut segments = self.segments.clone();
        if !tail.is_empty() {
            if tail != self.tail_text || self.tail_revision == 0 {
                self.tail_revision += 1;
                self.tail_text = tail.into();
            }
            segments.push(Segment {
                id: self.next_id,
                sequence: self.tail_revision,
                text: tail.into(),
                confirmed: false,
                begin_ms: None,
                end_ms: None,
            });
        }
        self.exposed_end = mapped.text.len();
        self.final_seen = final_result;
        if final_result && mapped.text != self.confirmed {
            return Err(Error::new("localSourceChanged"));
        }
        Ok(Transcript {
            text: mapped.text,
            confirmed_text: self.confirmed.clone(),
            segments,
        })
    }
}

fn cut(
    text: &str,
    start: usize,
    proven: usize,
    minimum: usize,
    pause: bool,
    final_result: bool,
) -> Option<usize> {
    if start >= proven {
        return None;
    }
    if final_result {
        return Some(proven);
    }
    let protected = protected_ranges(text);
    let mut count = 0;
    for (offset, g) in text[start..].grapheme_indices(true) {
        count += g.chars().count();
        let end = start + offset + g.len();
        if end > proven {
            break;
        }
        if end < minimum || protected.iter().any(|r| r.start < end && end < r.end) {
            continue;
        }
        let punctuation = g.chars().any(|c| "，。！？；,!?;\n".contains(c))
            || (g == "."
                && text[end..]
                    .chars()
                    .next()
                    .is_some_and(|c| !c.is_ascii_digit()));
        let token_continues = end == text.len() && protected.iter().any(|r| r.end == end);
        if punctuation || (count >= 512 && !token_continues) {
            return Some(end);
        }
    }
    (pause
        && proven >= minimum
        && text.is_char_boundary(proven)
        && !protected.iter().any(|r| r.start < proven && proven < r.end)
        && text
            .grapheme_indices(true)
            .any(|(at, g)| at + g.len() == proven))
    .then_some(proven)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sayweave_local_asr_protocol::PauseBoundary;
    fn event(revision: i32, text: &str, pause: bool, final_result: bool) -> Event {
        Event::Update {
            id: RunId {
                task: "task".into(),
                turn: 1,
                request: "request".into(),
            },
            revision,
            full: text.into(),
            confirmed: text.into(),
            committed: text.into(),
            tentative: String::new(),
            native_token_count: revision as usize,
            audio_received_ms: 2000,
            audio_committed_ms: 0,
            pause: pause.then_some(PauseBoundary {
                sequence: 1,
                sample_end: 16000,
                native_revision: revision,
            }),
            final_result,
        }
    }
    #[test]
    fn pause_confirms_same_identity_without_ending_and_final_tail_is_once() {
        let mut p = Projection::default();
        let a = p.update(event(1, "周三开会", false, false)).unwrap();
        assert!(a.confirmed_text.is_empty());
        let b = p.update(event(1, "周三开会", true, false)).unwrap();
        assert_eq!(a.segments[0].id, b.segments[0].id);
        assert!(b.segments[0].confirmed);
        let c = p
            .update(event(2, "周三开会地点在上海", false, false))
            .unwrap();
        assert_eq!(c.confirmed_text, "周三开会");
        assert_ne!(c.segments[0].id, c.segments[1].id);
        let d = p
            .update(event(2, "周三开会地点在上海", false, true))
            .unwrap();
        assert_eq!(d.text, d.confirmed_text);
        assert!(p.update(event(3, "迟到", false, false)).is_err());
    }
    #[test]
    fn raw_mapping_is_consistent_and_trailing_space_never_retracts_delivered_text() {
        let mut p = Projection::default();
        assert_eq!(p.update(event(1, "今 ", false, false)).unwrap().text, "今");
        assert_eq!(
            p.update(event(2, "今 天 ", false, false)).unwrap().text,
            "今天"
        );
        assert_eq!(
            p.update(event(3, "今 天 SQLite 3.5\n中 文🙂e\u{301}", false, true))
                .unwrap()
                .text,
            "今天 SQLite 3.5\n中文🙂e\u{301}"
        );
        let m = normalize("中 文", false);
        assert_eq!(m.boundaries, [(0, 0), (3, 3), (4, 3), (7, 6)]);
        assert_eq!(
            normalize("`中 文` https://例子.测试/a 中 文", true).text,
            "`中 文` https://例子.测试/a 中文"
        );
    }
    #[test]
    fn an_old_punctuation_cannot_repartition_a_published_source_id() {
        let mut p = Projection::default();
        let first = p.update(event(1, "第一句", false, false)).unwrap();
        let next = p.update(event(2, "第一句。后文", false, false)).unwrap();
        assert_eq!(next.segments[0].id, first.segments[0].id);
        assert_eq!(next.confirmed_text, "第一句。");
        // A late comma already inside an exposed mutable segment is not a new cut.
        let mut p = Projection::default();
        p.update(event(1, "网址 https://x.test/a.剩余", false, false))
            .unwrap();
        assert!(
            p.update(event(2, "网址 https://x.test/a.剩余内容", false, false))
                .unwrap()
                .confirmed_text
                .is_empty()
        );
    }
    #[test]
    fn decimals_protected_tokens_and_capacity_keep_text_and_source() {
        let mut p = Projection::default();
        assert!(
            p.update(event(1, "版本 3.", false, false))
                .unwrap()
                .confirmed_text
                .is_empty()
        );
        assert!(
            p.update(event(2, "版本 3.5 和 https://x.test/a", false, false))
                .unwrap()
                .confirmed_text
                .is_empty()
        );
        let mut p = Projection::default();
        let t = p
            .update(event(1, &"字".repeat(1100), false, false))
            .unwrap();
        assert_eq!(t.confirmed_text.chars().count(), 1024);
        assert_eq!(t.segments.len(), 3);
        assert_eq!(
            t.segments
                .iter()
                .map(|s| s.text.as_str())
                .collect::<String>(),
            t.text
        );
    }
    #[test]
    fn delivered_unconfirmed_prefix_is_not_consumed_twice_on_confirmation() {
        use crate::{contracts::processing::SourceSegment, processing::draft_source::Watermark};
        let sources = |t: &Transcript| {
            t.segments
                .iter()
                .map(|s| SourceSegment {
                    id: s.id,
                    revision: s.sequence,
                    text: s.text.clone(),
                    confirmed: s.confirmed,
                })
                .collect::<Vec<_>>()
        };
        let mut p = Projection::default();
        let first = p.update(event(1, "周三开会", false, false)).unwrap();
        let mark = Watermark::capture(&sources(&first), 4).unwrap();
        let next = p.update(event(2, "周三开会地点", true, false)).unwrap();
        assert_eq!(mark.offset(&sources(&next)), Some(4));
    }
    #[test]
    fn production_block_streams_immediately_but_generates_only_at_a_processing_boundary() {
        use crate::processing::draft::Block;
        let mut p = Projection::default();
        let mut block = Block::new("owned-block".into());
        for (revision, raw) in [(1, "今 "), (2, "今 天"), (3, "今 天 下 午")] {
            let t = p.update(event(revision, raw, false, false)).unwrap();
            block
                .observe(t.text.clone(), t.confirmed_text.chars().count())
                .unwrap();
            assert_eq!(block.visible(), t.text);
            assert!(block.snapshot(false).is_none());
        }
        let t = p.update(event(3, "今 天 下 午", true, false)).unwrap();
        block
            .observe(t.text, t.confirmed_text.chars().count())
            .unwrap();
        assert_eq!(block.snapshot(false).unwrap().text, "今天下午");
    }
    #[test]
    fn empty_or_duplicate_pause_has_no_new_source_and_unproven_text_cannot_be_confirmed() {
        let mut p = Projection::default();
        let e = event(1, "", true, false);
        assert!(p.update(e.clone()).unwrap().segments.is_empty());
        assert!(p.update(e).unwrap().segments.is_empty());
        let mut e = event(2, "可见尾部", true, false);
        if let Event::Update {
            confirmed,
            pause: Some(p),
            ..
        } = &mut e
        {
            confirmed.clear();
            p.sequence = 2;
            p.sample_end = 24000;
        }
        let t = p.update(e).unwrap();
        assert_eq!(t.text, "可见尾部");
        assert!(t.confirmed_text.is_empty());
    }
    #[test]
    fn old_run_false_pause_and_changed_native_source_are_rejected() {
        let mut p = Projection::default();
        p.update(event(1, "周三", false, false)).unwrap();
        assert!(p.update(event(2, "周五", true, false)).is_err());
        let mut e = event(1, "周三", true, false);
        if let Event::Update { pause: Some(p), .. } = &mut e {
            p.native_revision = 2;
        }
        assert!(p.update(e).is_err());
        let mut e = event(2, "周三开会", false, false);
        if let Event::Update { id, .. } = &mut e {
            id.turn = 2;
        }
        assert!(p.update(e).is_err());
    }

    #[test]
    fn trace_metadata_is_separate_from_the_strict_native_protocol() {
        let mut raw = serde_json::to_value(event(1, "周三开会", false, false)).unwrap();
        raw["elapsedMs"] = serde_json::json!(123.5);
        assert!(serde_json::from_value::<Event>(raw.clone()).is_err());
        let decoded = trace_event(&raw);
        assert!(matches!(decoded, Event::Update { revision: 1, .. }));
        assert_eq!(raw["elapsedMs"], 123.5);
    }

    fn trace_event(raw: &serde_json::Value) -> Event {
        let mut frame = raw.clone();
        frame
            .as_object_mut()
            .unwrap()
            .remove("elapsedMs")
            .expect("capture metadata must be explicit");
        serde_json::from_value(frame).unwrap()
    }

    #[test]
    #[ignore = "explicit saved CPU/Vulkan worker traces; records raw and projected text together"]
    fn saved_real_worker_updates_preserve_processing_and_display_contracts() {
        use sha2::{Digest, Sha256};
        use std::{collections::HashMap, path::Path};
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join(".local/local-asr/reports");
        let output = root.join("alignment-raw-projection-pairs.json");
        assert!(!output.exists(), "Preserve the first replay result");
        let mut reports = Vec::new();
        for device in ["cpu", "auto"] {
            for fixture in ["short-zh", "mixed", "english", "long-zh"] {
                let file = root.join(format!("alignment-raw-{device}-{fixture}.json"));
                let bytes = std::fs::read(&file).unwrap();
                let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(report["passed"], true);
                assert_eq!(report["resourcesClosed"], true);
                let mut streams = HashMap::<String, Projection>::new();
                let mut pairs = Vec::new();
                for raw in report["trace"].as_array().unwrap() {
                    if raw["kind"] != "update" {
                        continue;
                    }
                    let event = trace_event(raw);
                    let Event::Update { id, .. } = &event else {
                        unreachable!()
                    };
                    let projection = streams.entry(id.request.clone()).or_default();
                    let previous = projection.confirmed.clone();
                    let text = projection.update(event).unwrap();
                    assert!(text.confirmed_text.starts_with(&previous));
                    assert_eq!(
                        text.segments
                            .iter()
                            .map(|s| s.text.as_str())
                            .collect::<String>(),
                        text.text
                    );
                    assert_eq!(
                        text.segments
                            .iter()
                            .take_while(|s| s.confirmed)
                            .map(|s| s.text.as_str())
                            .collect::<String>(),
                        text.confirmed_text
                    );
                    pairs.push(serde_json::json!({
                        "raw": raw,
                        "full": text.text,
                        "processingConfirmed": text.confirmed_text,
                        "segments": text.segments,
                    }));
                }
                assert_eq!(streams.len(), 2);
                assert!(streams.values().all(|p| p.final_seen));
                reports.push(serde_json::json!({
                    "device": device, "fixture": fixture, "input": file,
                    "inputSha256": format!("{:x}", Sha256::digest(bytes)),
                    "workerSha256": report["workerSha256"], "pairs": pairs,
                }));
            }
        }
        std::fs::write(
            output,
            serde_json::to_vec_pretty(&serde_json::json!({
                "source": crate::build_info::current().commit,
                "purpose": "replay exact worker updates through the production Projection; no host or microphone claim",
                "reports": reports, "passed": true,
            }))
            .unwrap(),
        )
        .unwrap();
    }
}
