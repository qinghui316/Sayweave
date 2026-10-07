//! History projects immutable generated versions, not the user's external edits.
use super::{
    quality,
    units::{SourceIndex, Unit, slice},
};
use crate::contracts::processing::{SourceTurn, UnitRecord};
pub fn text(turns: &[SourceTurn], records: &[UnitRecord]) -> String {
    let mut rendered = Vec::new();
    for source in turns {
        let indexed = SourceIndex::new(&source.segments);
        let source_length = source.text.chars().count();
        let mut text = String::new();
        let mut consumed = 0;
        let mut units = records
            .iter()
            .filter(|r| {
                r.turn == source.turn
                    && r.generation_state == "success"
                    && r.candidate_text.is_some()
            })
            .collect::<Vec<_>>();
        units.sort_by_key(|r| {
            (
                r.source_start,
                if ["applied", "handedOff"].contains(&r.application_state.as_str()) {
                    0
                } else {
                    1
                },
                r.id.as_str(),
            )
        });
        for record in units {
            if record.source_start < consumed || record.source_end > source_length {
                continue;
            }
            let Ok(refs) = serde_json::from_str(&record.source_refs) else {
                continue;
            };
            let unit = Unit {
                id: record.id.clone(),
                turn: record.turn,
                chars: record.source_start..record.source_end,
                source: refs,
                text: record.source_text.clone(),
                preserve_raw: false,
                final_source: true,
            };
            let candidate = record.candidate_text.as_deref().unwrap();
            if !unit.matches_in(source.turn, &indexed)
                || quality::validate(&record.source_text, candidate).is_err()
            {
                continue;
            }
            text.push_str(&slice(&source.text, consumed..record.source_start));
            text.push_str(&quality::retain_boundaries(&record.source_text, candidate));
            consumed = record.source_end;
        }
        text.push_str(&slice(&source.text, consumed..source_length));
        if !text.is_empty() {
            rendered.push(text);
        }
    }
    rendered.join("\n")
}
