//! A PCM clock alongside inference. No timers, padding, replay or stream control.
use sayweave_local_asr_protocol::PauseBoundary;
use webrtc_vad::{SampleRate, Vad, VadMode};

const FRAME: usize = 320;
const SILENCE_FRAMES: u32 = 40;

#[derive(Default)]
struct Clock {
    sample_end: u64,
    sequence: u64,
    voiced: bool,
    silence: u32,
}
impl Clock {
    fn frame(&mut self, voice: bool) -> Option<(u64, u64)> {
        self.sample_end += FRAME as u64;
        if voice {
            self.voiced = true;
            self.silence = 0;
        } else if self.voiced {
            self.silence += 1;
            if self.silence == SILENCE_FRAMES {
                self.voiced = false;
                self.silence = 0;
                self.sequence += 1;
                return Some((self.sequence, self.sample_end));
            }
        }
        None
    }
}

pub struct PauseDetector {
    vad: Vad,
    frame: Vec<i16>,
    clock: Clock,
}
impl Default for PauseDetector {
    fn default() -> Self {
        Self {
            vad: Vad::new_with_rate_and_mode(SampleRate::Rate16kHz, VadMode::Quality),
            frame: Vec::with_capacity(FRAME),
            clock: Clock::default(),
        }
    }
}
impl PauseDetector {
    /// Coalesce pauses within one bounded packet to the latest PCM boundary.
    pub fn feed(
        &mut self,
        pcm: &[i16],
        revision: i32,
    ) -> Result<Option<PauseBoundary>, &'static str> {
        let mut latest = None;
        for sample in pcm {
            self.frame.push(*sample);
            if self.frame.len() == FRAME {
                let voice = self
                    .vad
                    .is_voice_segment(&self.frame)
                    .map_err(|_| "localProtocol")?;
                if let Some((sequence, sample_end)) = self.clock.frame(voice) {
                    latest = Some(PauseBoundary {
                        sequence,
                        sample_end,
                        native_revision: revision,
                    });
                }
                self.frame.clear();
            }
        }
        Ok(latest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn actual_silence_requires_voice_and_exactly_forty_frames_then_only_once() {
        let mut c = Clock::default();
        for _ in 0..100 {
            assert!(c.frame(false).is_none());
        }
        c.frame(true);
        for _ in 0..39 {
            assert!(c.frame(false).is_none());
        }
        assert_eq!(c.frame(false), Some((1, 141 * FRAME as u64)));
        for _ in 0..100 {
            assert!(c.frame(false).is_none());
        }
        c.frame(true);
        for _ in 0..39 {
            assert!(c.frame(false).is_none());
        }
        assert!(matches!(c.frame(false), Some((2, _))));
    }
    #[test]
    fn silence_resets_on_voice_and_absent_pcm_advances_nothing() {
        let mut c = Clock::default();
        c.frame(true);
        for _ in 0..39 {
            c.frame(false);
        }
        c.frame(true);
        for _ in 0..39 {
            assert!(c.frame(false).is_none());
        }
        assert_eq!(c.sequence, 0);
        assert!(c.frame(false).is_some());
        let mut detector = PauseDetector::default();
        assert!(detector.feed(&[], 1).unwrap().is_none());
        detector.feed(&[0; 319], 1).unwrap();
        assert_eq!(detector.clock.sample_end, 0);
        detector.feed(&[0; 2], 1).unwrap();
        assert_eq!(detector.clock.sample_end, 320);
        assert_eq!(detector.frame.len(), 1);
    }
}
