//! Source identities absorbed by an editor baseline. Never deduplicates text.
use crate::contracts::processing::SourceSegment;

#[derive(Default, Clone)]
pub struct Watermark(Vec<Consumed>);
#[derive(Clone)]
struct Consumed {
    id: u64,
    whole_confirmed: bool,
    prefix: String,
}
impl Watermark {
    pub fn capture(segments: &[SourceSegment], mut count: usize) -> Option<Self> {
        let mut result = Vec::new();
        for segment in segments {
            if count == 0 {
                break;
            }
            let size = segment.text.chars().count();
            let used = size.min(count);
            result.push(Consumed {
                id: segment.id,
                whole_confirmed: segment.confirmed && used == size,
                prefix: segment.text.chars().take(used).collect(),
            });
            count -= used;
        }
        (count == 0).then_some(Self(result))
    }
    pub fn offset(&self, segments: &[SourceSegment]) -> Option<usize> {
        let mut count = 0;
        for (index, consumed) in self.0.iter().enumerate() {
            let segment = segments.get(index)?;
            if segment.id != consumed.id {
                return None;
            }
            if consumed.whole_confirmed {
                // A revision of an absorbed confirmed segment cannot restore
                // text the user changed, even when its character count changes.
                count += segment.text.chars().count();
            } else if segment.text.starts_with(&consumed.prefix) {
                count += consumed.prefix.chars().count();
            } else {
                // A partial source whose boundary changed has no safe split.
                return None;
            }
        }
        Some(count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn source(id: u64, text: &str, confirmed: bool) -> SourceSegment {
        SourceSegment {
            id,
            revision: 1,
            text: text.into(),
            confirmed,
        }
    }
    #[test]
    fn confirmed_revision_is_absorbed_by_identity_not_old_length() {
        let old = [source(1, "周三开会", true)];
        let mark = Watermark::capture(&old, 4).unwrap();
        assert_eq!(
            mark.offset(&[source(1, "周三下午开会", true), source(2, "改成周一", true)]),
            Some(6)
        );
        assert_eq!(mark.offset(&[source(2, "周三开会", true)]), None);
    }
    #[test]
    fn partial_update_keeps_new_tail_once_and_refuses_changed_boundary() {
        let mark = Watermark::capture(&[source(7, "开会", false)], 2).unwrap();
        assert_eq!(mark.offset(&[source(7, "开会在周一", true)]), Some(2));
        assert_eq!(mark.offset(&[source(7, "会议在周一", true)]), None);
    }
}
