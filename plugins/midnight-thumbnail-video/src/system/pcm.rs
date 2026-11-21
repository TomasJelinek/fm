//! Turns timestamped decoded PCM into the contiguous blocks that `PlaybackDecoder::pcm` returns:
//! 48 kHz stereo from the last seek target, preroll dropped, gaps and the tail up to the
//! container duration filled with silence. Any platform decoder can use it.
use crate::media::{Block, MAX_PCM_FRAMES};
use std::io;

pub const RATE: u32 = 48_000;
pub const CHANNELS: usize = 2;

/// Interleaved 48 kHz stereo samples that start `start_nanos` after the container start.
pub struct Clip {
    pub start_nanos: i64,
    pub samples: Vec<i16>,
}

pub struct Timeline {
    duration_micros: Option<u64>,
    target_micros: u64,
    submitted: u64,
    /// The current clip and its first frame, counted from the seek target.
    held: Option<(i128, Vec<i16>)>,
    ended: bool,
}

impl Timeline {
    pub fn new(duration_micros: Option<u64>) -> Self {
        Self {
            duration_micros,
            target_micros: 0,
            submitted: 0,
            held: None,
            ended: false,
        }
    }

    /// Start a new epoch at `target_micros`; the decoder repositions itself.
    pub fn seek(&mut self, target_micros: u64) {
        self.target_micros = target_micros;
        self.submitted = 0;
        self.held = None;
        self.ended = false;
    }

    /// Up to `credit` frames. `decode` returns the next clip, or `None` at the end of the audio.
    pub fn next(
        &mut self,
        credit: u16,
        mut decode: impl FnMut() -> io::Result<Option<Clip>>,
    ) -> io::Result<Option<Block>> {
        if credit == 0 || credit > MAX_PCM_FRAMES {
            return Err(invalid("audio credit exceeds the PCM block limit"));
        }
        let first_frame = self.submitted;
        let mut samples = Vec::with_capacity(usize::from(credit) * CHANNELS);
        for _ in 0..4096 {
            let remaining = usize::from(credit) - samples.len() / CHANNELS;
            if remaining == 0 {
                break;
            }
            if self.held.is_none() && !self.ended {
                match decode()? {
                    Some(clip) => self.held = Some(self.place(clip)?),
                    None => self.ended = true,
                }
            }
            let position = i128::from(self.submitted);
            let frames = if let Some((start, clip)) = &self.held {
                if *start > position {
                    let gap = (*start - position).min(remaining as i128) as usize;
                    samples.resize(samples.len() + gap * CHANNELS, 0);
                    gap
                } else {
                    let total = clip.len() / CHANNELS;
                    let offset = (position - *start).min(total as i128) as usize;
                    let frames = remaining.min(total - offset);
                    samples
                        .extend_from_slice(&clip[offset * CHANNELS..(offset + frames) * CHANNELS]);
                    if offset + frames == total {
                        self.held = None;
                    }
                    frames
                }
            } else {
                // The media timeline can extend beyond the audio stream. Explicit silence
                // keeps video aligned through that tail.
                let end = self.duration_micros.map_or(position, |duration| {
                    micros_frames(duration.saturating_sub(self.target_micros))
                });
                let frames = (end - position).clamp(0, remaining as i128) as usize;
                samples.resize(samples.len() + frames * CHANNELS, 0);
                if frames == 0 {
                    break;
                }
                frames
            };
            self.submitted = self
                .submitted
                .checked_add(frames as u64)
                .ok_or_else(|| invalid("audio sequence exhausted"))?;
        }
        if samples.is_empty() && !self.ended {
            return Err(invalid("audio preroll exceeded its decode-step budget"));
        }
        Ok((!samples.is_empty()).then_some(Block {
            first_frame,
            samples,
        }))
    }

    fn place(&self, clip: Clip) -> io::Result<(i128, Vec<i16>)> {
        if clip.samples.is_empty() || !clip.samples.len().is_multiple_of(CHANNELS) {
            return Err(invalid("decoded audio is not whole stereo frames"));
        }
        let start = (i128::from(clip.start_nanos) * i128::from(RATE)).div_euclid(1_000_000_000)
            - micros_frames(self.target_micros);
        Ok((start, clip.samples))
    }
}

fn micros_frames(micros: u64) -> i128 {
    (i128::from(micros) * i128::from(RATE)).div_euclid(1_000_000)
}

fn invalid(detail: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, detail)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clip(start_millis: i64, frames: usize, value: i16) -> Clip {
        Clip {
            start_nanos: start_millis * 1_000_000,
            samples: vec![value; frames * CHANNELS],
        }
    }

    #[test]
    fn preroll_is_dropped_gaps_are_silent_and_the_tail_reaches_the_duration() {
        let mut timeline = Timeline::new(Some(100_000));
        timeline.seek(10_000);
        // 5 ms of preroll before the 10 ms target, then a clip 1 ms after the target.
        let mut clips = vec![clip(11, 48, 7), clip(5, 240, 3)];
        let block = timeline.next(4_800, || Ok(clips.pop())).unwrap().unwrap();
        assert_eq!(block.first_frame, 0);
        // 48 silent frames (the 1 ms gap), 48 frames of 7, then silence to 100 ms.
        let frames = block.samples.len() / CHANNELS;
        assert_eq!(frames, 90 * 48);
        assert!(
            block.samples[..48 * CHANNELS]
                .iter()
                .all(|sample| *sample == 0)
        );
        assert!(
            block.samples[48 * CHANNELS..96 * CHANNELS]
                .iter()
                .all(|sample| *sample == 7)
        );
        assert!(
            block.samples[96 * CHANNELS..]
                .iter()
                .all(|sample| *sample == 0)
        );
        assert!(timeline.next(4_800, || Ok(None)).unwrap().is_none());
    }

    #[test]
    fn blocks_are_contiguous_within_their_credit() {
        let mut timeline = Timeline::new(None);
        let mut clips = vec![clip(0, 100, 1)];
        let first = timeline.next(60, || Ok(clips.pop())).unwrap().unwrap();
        let second = timeline.next(60, || Ok(clips.pop())).unwrap().unwrap();
        assert_eq!((first.first_frame, first.samples.len()), (0, 120));
        assert_eq!((second.first_frame, second.samples.len()), (60, 80));
        assert!(timeline.next(60, || Ok(None)).unwrap().is_none());
    }

    #[test]
    fn invalid_credit_and_partial_frames_are_rejected() {
        let mut timeline = Timeline::new(None);
        assert!(timeline.next(0, || Ok(None)).is_err());
        assert!(timeline.next(MAX_PCM_FRAMES + 1, || Ok(None)).is_err());
        let odd = Clip {
            start_nanos: 0,
            samples: vec![1; 3],
        };
        let mut clips = vec![odd];
        assert!(timeline.next(10, || Ok(clips.pop())).is_err());
    }
}
