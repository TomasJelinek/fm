//! The sound level of video playback, set by presentation and read by the output loop.
//!
//! One [`AudioVolume`] belongs to a viewer window: every video it plays shares it, so
//! the level carries from file to file. Presentation stores a new level at any time
//! without waiting; [`Playback`](crate::Playback) reads it once per output step, before it
//! submits PCM to the device, and scales only that block. Frames already queued on the
//! device keep the level they were written with, so a change is heard within one device
//! buffer (at most `MAX_DEVICE_FRAMES`, 200 ms) and needs no seek or restart.
use std::{
    io,
    sync::atomic::{AtomicU32, Ordering},
};

/// The highest level: the decoded samples unchanged.
pub const MAX_VOLUME_PERCENT: u8 = 100;

/// Unity gain in the Q15 fixed point that [`scale_pcm`] applies.
pub(crate) const UNITY_GAIN: u16 = 1 << 15;

const MUTED: u32 = 1 << 8;

/// Level from 0 to 100 percent and a mute switch that keeps the level, packed in one
/// atomic word. Both halves change together, so a reader never sees a level from one
/// change with the mute state of another.
///
/// Ordering: `Relaxed` on both sides. The word publishes no other memory: a reader needs
/// only some recent value, which coherence delivers, and it never derives a pointer or an
/// index from it. The output loop reads it with one load and no lock, so the
/// presentation thread can never block or delay audio output.
#[derive(Debug)]
pub struct AudioVolume(AtomicU32);

impl Default for AudioVolume {
    /// Full level, not muted.
    fn default() -> Self {
        Self(AtomicU32::new(u32::from(MAX_VOLUME_PERCENT)))
    }
}

impl AudioVolume {
    /// Store a new level. A level above [`MAX_VOLUME_PERCENT`] is refused and the
    /// current one stays.
    pub fn set(&self, percent: u8, muted: bool) -> io::Result<()> {
        if percent > MAX_VOLUME_PERCENT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Volume must be from 0 to 100 percent",
            ));
        }
        let muted = if muted { MUTED } else { 0 };
        self.0.store(u32::from(percent) | muted, Ordering::Relaxed);
        Ok(())
    }

    /// The level in percent and whether it is muted.
    pub fn get(&self) -> (u8, bool) {
        let word = self.0.load(Ordering::Relaxed);
        ((word & 0xff) as u8, word & MUTED != 0)
    }

    /// The Q15 gain for the current level: zero when muted.
    pub(crate) fn gain(&self) -> u16 {
        let (percent, muted) = self.get();
        if muted { 0 } else { gain(percent) }
    }
}

/// Q15 gain of a level: `(percent / 100)³`, rounded. The cubic curve is the one
/// PulseAudio and PipeWire use for their volume sliders: equal slider steps sound like
/// roughly equal loudness steps (50 % is −18 dB), where a linear amplitude would leave
/// the upper half of the slider sounding almost the same. 100 % is exactly
/// [`UNITY_GAIN`]; 1 % is −120 dB, below 16-bit resolution, and rounds to silence.
pub(crate) fn gain(percent: u8) -> u16 {
    let percent = u64::from(percent.min(MAX_VOLUME_PERCENT));
    let cube = percent * percent * percent;
    ((cube * u64::from(UNITY_GAIN) + 500_000) / 1_000_000) as u16
}

/// Write `input` scaled by a Q15 `gain` into `output`, rounding to nearest and saturating
/// at the ends of the 16-bit range. `output` is cleared first; reserve it once and this
/// does not allocate.
pub(crate) fn scale_pcm(input: &[i16], gain: u16, output: &mut Vec<i16>) {
    output.clear();
    // The i64 product cannot overflow for any u16 gain. [`gain`] never exceeds unity,
    // but a caller's larger gain saturates instead of wrapping.
    output.extend(input.iter().map(|&sample| {
        let scaled = (i64::from(sample) * i64::from(gain) + (1 << 14)) >> 15;
        scaled.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16
    }));
}

#[cfg(test)]
#[path = "volume_tests.rs"]
mod tests;
