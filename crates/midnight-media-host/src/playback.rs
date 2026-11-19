use std::{
    collections::VecDeque,
    io,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

pub const PCM_RATE: u32 = 48_000;
pub const PCM_CHANNELS: usize = 2;
pub const MAX_PCM_BLOCK_FRAMES: usize = 4_800;
pub const MAX_HOST_FRAMES: usize = 12_000;
pub const MAX_DEVICE_FRAMES: usize = 9_600;

/// Frames played and frames still queued on the device since its last reset.
/// A frame is one sample per channel. A hardware counter that continues during
/// underrun must report only played media frames.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DeviceClock {
    pub played_frames: u64,
    pub queued_frames: usize,
}

/// The last confirmed clock when playback stopped. Discarded means the adapter
/// stopped and reset its queue during pause. The decoder must seek to that time.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AudioPause {
    Retained(DeviceClock),
    Discarded(DeviceClock),
}

/// Native output owned on its backend thread for its entire lifetime. Formats
/// other than interleaved stereo signed 16-bit PCM at PCM_RATE require conversion
/// in the qualified decoder. No callback receives a Playback or a media source.
pub trait AudioSink {
    /// Start or arm output. An empty queue is valid and plays no media frames.
    fn start(&mut self) -> io::Result<()>;

    /// Stop producing sound and report whether unplayed data remains queued.
    fn pause(&mut self) -> io::Result<AudioPause>;

    /// Stop output, discard its data and reset its clock without draining.
    /// The adapter's Drop must safely retire any device-owned buffers even if
    /// reset fails. The caller keeps the sink until an explicit close or Drop.
    fn reset(&mut self) -> io::Result<()>;

    /// Try to submit complete frames without waiting for free device space.
    /// Return the number accepted, or zero for backpressure. Any retained
    /// backing memory belongs to the sink and must not borrow this input.
    fn write(&mut self, interleaved: &[i16]) -> io::Result<usize>;

    /// Read the current output clock, including queued frames used for latency
    /// and backpressure. Fail on device loss or an unqualified clock.
    fn sample_clock(&mut self) -> io::Result<DeviceClock>;
}

/// Identity of one uninterrupted decoder and device segment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaybackEpoch(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SeekRequest {
    pub epoch: PlaybackEpoch,
    pub position: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlaybackState {
    Paused,
    Playing,
    Seeking,
    Faulted,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PlaybackSnapshot {
    pub state: PlaybackState,
    pub epoch: PlaybackEpoch,
    pub position: Duration,
    pub audio: Option<DeviceClock>,
    pub host_queued_frames: usize,
}

#[derive(Debug, Eq, PartialEq)]
pub enum PcmAdmission {
    Queued,
    Stale,
    /// Keep the same epoch, sequence and samples for a bounded retry.
    Backpressure(Box<[i16]>),
}

struct Audio {
    sink: Box<dyn AudioSink>,
    pending: VecDeque<[i16; PCM_CHANNELS]>,
    base: Duration,
    next_frame: u64,
    submitted: u64,
    device: DeviceClock,
}

impl Audio {
    fn new(sink: Box<dyn AudioSink>, base: Duration) -> Self {
        Self {
            sink,
            pending: VecDeque::with_capacity(MAX_HOST_FRAMES),
            base,
            next_frame: 0,
            submitted: 0,
            device: DeviceClock::default(),
        }
    }

    fn rebase(&mut self, position: Duration) {
        self.pending.clear();
        self.base = position;
        self.next_frame = 0;
        self.submitted = 0;
        self.device = DeviceClock::default();
    }

    fn enqueue(
        &mut self,
        first_frame: u64,
        samples: Box<[i16]>,
        frames: usize,
    ) -> io::Result<PcmAdmission> {
        if first_frame != self.next_frame {
            return Err(invalid("PCM discontinuity within a playback generation"));
        }
        if frames > MAX_HOST_FRAMES - self.pending.len() {
            return Ok(PcmAdmission::Backpressure(samples));
        }
        self.next_frame = first_frame
            .checked_add(frames as u64)
            .ok_or_else(|| invalid("PCM frame counter overflow"))?;
        self.pending
            .extend(samples.as_chunks::<PCM_CHANNELS>().0.iter().copied());
        Ok(PcmAdmission::Queued)
    }

    fn observe_clock(&mut self, sample: DeviceClock) -> io::Result<Duration> {
        if sample.played_frames < self.device.played_frames
            || sample.played_frames > self.submitted
            || sample.queued_frames > MAX_DEVICE_FRAMES
            || sample.queued_frames as u64 > self.submitted - sample.played_frames
        {
            return Err(invalid(
                "audio device returned an inconsistent sample clock",
            ));
        }
        let rate = u64::from(PCM_RATE);
        let play_time = Duration::new(
            sample.played_frames / rate,
            ((sample.played_frames % rate) * 1_000_000_000 / rate) as u32,
        );
        let position = self
            .base
            .checked_add(play_time)
            .ok_or_else(|| invalid("audio media position overflow"))?;
        self.device = sample;
        Ok(position)
    }

    fn pump(&mut self, cancelled: &AtomicBool) -> io::Result<Duration> {
        let sample = self.sink.sample_clock()?;
        let position = self.observe_clock(sample)?;
        // Even across a wrap, submit just the first contiguous span and at most
        // one bounded block. The next device-ready tick handles the other span.
        let head = self.pending.as_slices().0;
        let frames = head
            .len()
            .min(MAX_PCM_BLOCK_FRAMES)
            .min(MAX_DEVICE_FRAMES - self.device.queued_frames);
        if frames != 0 && !cancelled.load(Ordering::Acquire) {
            let accepted = self.sink.write(head[..frames].as_flattened())?;
            if accepted > frames {
                return Err(invalid("audio sink accepted more frames than supplied"));
            }
            drop(self.pending.drain(..accepted));
            self.submitted += accepted as u64;
            // Until the next hardware observation, account all accepted frames
            // as queued. This is a conservative queue and latency estimate.
            self.device.queued_frames += accepted;
        }
        Ok(position)
    }
}

/// Single-worker playback coordinator. A snapshot carries no decoder or device
/// authority and can be read by presentation on another thread.
pub struct Playback {
    state: PlaybackState,
    epoch: PlaybackEpoch,
    position: Duration,
    video_tick: Option<Instant>,
    video_tail: bool,
    resume_after_seek: bool,
    audio: Option<Audio>,
}

impl Playback {
    pub fn video_only(position: Duration) -> Self {
        Self {
            state: PlaybackState::Paused,
            epoch: PlaybackEpoch(1),
            position,
            video_tick: None,
            video_tail: false,
            resume_after_seek: false,
            audio: None,
        }
    }

    /// Establish a known stopped output. Unavailable audio is an error, so a
    /// caller must explicitly choose video-only playback if that is desired.
    pub fn with_audio(position: Duration, mut sink: Box<dyn AudioSink>) -> io::Result<Self> {
        sink.reset()?;
        let mut player = Self::video_only(position);
        player.audio = Some(Audio::new(sink, position));
        Ok(player)
    }

    pub fn snapshot(&self) -> PlaybackSnapshot {
        PlaybackSnapshot {
            state: self.state,
            epoch: self.epoch,
            position: self.position,
            audio: self.audio.as_ref().map(|audio| audio.device),
            host_queued_frames: self.audio.as_ref().map_or(0, |audio| audio.pending.len()),
        }
    }

    pub fn play(&mut self, now: Instant) -> io::Result<()> {
        self.check_open()?;
        match self.state {
            PlaybackState::Seeking => {
                self.resume_after_seek = true;
                return Ok(());
            }
            PlaybackState::Playing => return Ok(()),
            _ => {}
        }
        if !self.video_tail
            && let Some(audio) = &mut self.audio
            && let Err(error) = audio.sink.start()
        {
            return Err(self.fault(error));
        }
        self.video_tick = (self.audio.is_none() || self.video_tail).then_some(now);
        self.state = PlaybackState::Playing;
        Ok(())
    }

    /// Freeze the published clock. If the device discarded data, return an exact
    /// decoder seek request and keep the host stopped until its acknowledgement.
    pub fn pause(&mut self, now: Instant) -> io::Result<Option<SeekRequest>> {
        self.check_open()?;
        match self.state {
            PlaybackState::Seeking => {
                self.resume_after_seek = false;
                return Ok(None);
            }
            PlaybackState::Playing => {}
            _ => return Ok(None),
        }
        if !self.video_tail
            && let Some(audio) = &mut self.audio
        {
            let stopped = audio.sink.pause().and_then(|pause| {
                let (sample, discarded) = match pause {
                    AudioPause::Retained(sample) => (sample, false),
                    AudioPause::Discarded(sample) => (sample, true),
                };
                audio
                    .observe_clock(sample)
                    .map(|position| (position, discarded))
            });
            let (position, discarded) = stopped.map_err(|error| self.fault(error))?;
            self.position = position;
            if discarded {
                let epoch = self.next_epoch().map_err(|error| self.fault(error))?;
                return Ok(Some(self.begin_seek(epoch, position, false)));
            }
        } else {
            self.advance_video(now).map_err(|error| self.fault(error))?;
        }
        self.video_tick = None;
        self.state = PlaybackState::Paused;
        Ok(None)
    }

    /// Flush both queues before issuing a new decoder generation. Blocked or
    /// late decoder replies cannot put old media back onto the device.
    pub fn seek(&mut self, position: Duration) -> io::Result<SeekRequest> {
        self.check_open()?;
        let epoch = self.next_epoch()?;
        let resume = self.state == PlaybackState::Playing
            || self.state == PlaybackState::Seeking && self.resume_after_seek;
        if let Some(audio) = &mut self.audio
            && let Err(error) = audio.sink.reset()
        {
            return Err(self.fault(error));
        }
        Ok(self.begin_seek(epoch, position, resume))
    }

    /// The decoder acknowledges only after aligning the first incoming PCM
    /// block to frame zero at the requested media position.
    pub fn acknowledge_seek(&mut self, epoch: PlaybackEpoch, now: Instant) -> io::Result<bool> {
        if self.state != PlaybackState::Seeking || self.epoch != epoch {
            return Ok(false);
        }
        let resume = std::mem::take(&mut self.resume_after_seek);
        self.state = PlaybackState::Paused;
        if resume {
            self.play(now)?;
        }
        Ok(true)
    }

    pub fn enqueue_pcm(
        &mut self,
        epoch: PlaybackEpoch,
        first_frame: u64,
        samples: Box<[i16]>,
    ) -> io::Result<PcmAdmission> {
        if self.epoch != epoch {
            return Ok(PcmAdmission::Stale);
        }
        self.check_open()?;
        if self.video_tail {
            return Err(invalid("PCM after the audio track ended"));
        }
        let audio = self.audio.as_mut().ok_or_else(|| {
            io::Error::new(io::ErrorKind::Unsupported, "playback has no audio output")
        })?;
        let frames = samples.len() / PCM_CHANNELS;
        if frames == 0
            || frames > MAX_PCM_BLOCK_FRAMES
            || !samples.len().is_multiple_of(PCM_CHANNELS)
        {
            return Err(invalid("invalid size for an interleaved PCM block"));
        }
        if self.state == PlaybackState::Seeking {
            return Ok(PcmAdmission::Backpressure(samples));
        }
        audio.enqueue(first_frame, samples, frames)
    }

    /// Drive one bounded output step. The caller supplies a live cancellation
    /// flag and schedules another tick on device readiness or decoder progress.
    /// No input/output call assumes ownership of a source or a disk lease.
    pub fn pump(&mut self, now: Instant, cancelled: &AtomicBool) -> io::Result<PlaybackSnapshot> {
        self.check_cancelled(cancelled)?;
        if self.state == PlaybackState::Playing {
            if !self.video_tail
                && let Some(audio) = &mut self.audio
            {
                let result = audio.pump(cancelled);
                self.position = result.map_err(|error| self.fault(error))?;
            } else {
                self.advance_video(now).map_err(|error| self.fault(error))?;
            }
        }
        self.check_cancelled(cancelled)?;
        Ok(self.snapshot())
    }

    /// Settle an exhausted seek preview without waiting for paused PCM to play.
    /// The decoder is already at EOF; the session must seek before admitting
    /// another segment. Preserve the confirmed position and reset both queues.
    pub(crate) fn finish_at_eof(&mut self) -> io::Result<()> {
        self.check_open()?;
        if let Some(audio) = &mut self.audio {
            if let Err(error) = audio.sink.reset() {
                return Err(self.fault(error));
            }
            audio.rebase(self.position);
        }
        self.video_tick = None;
        self.video_tail = false;
        self.resume_after_seek = false;
        self.state = PlaybackState::Paused;
        Ok(())
    }

    /// Keep presentation moving if a completed audio track is shorter than
    /// the video. No PCM may be admitted until the next seek resets the epoch.
    pub(crate) fn begin_video_tail(&mut self, now: Instant) -> io::Result<()> {
        if self.video_tail || self.state != PlaybackState::Playing {
            return Ok(());
        }
        let Some(audio) = self.audio.as_mut() else {
            return Ok(());
        };
        if !audio.pending.is_empty() || audio.device.queued_frames != 0 {
            return Err(invalid("audio must drain before starting the video tail"));
        }
        audio.sink.reset().map_err(|error| self.fault(error))?;
        self.video_tail = true;
        self.video_tick = Some(now);
        Ok(())
    }

    fn check_cancelled(&mut self, cancelled: &AtomicBool) -> io::Result<()> {
        if cancelled.load(Ordering::Acquire) {
            self.close()?;
            return Err(io::ErrorKind::Interrupted.into());
        }
        Ok(())
    }

    /// Retire output explicitly so a native reset refusal remains observable.
    pub fn close(&mut self) -> io::Result<()> {
        self.video_tick = None;
        self.resume_after_seek = false;
        if let Some(audio) = &mut self.audio {
            audio.pending.clear();
            if let Err(error) = audio.sink.reset() {
                self.state = PlaybackState::Faulted;
                return Err(error);
            }
        }
        self.audio = None;
        self.state = PlaybackState::Closed;
        Ok(())
    }

    fn check_open(&self) -> io::Result<()> {
        match self.state {
            PlaybackState::Closed => {
                Err(io::Error::new(io::ErrorKind::BrokenPipe, "playback closed"))
            }
            PlaybackState::Faulted => Err(io::Error::other("playback device fault")),
            _ => Ok(()),
        }
    }

    fn next_epoch(&self) -> io::Result<PlaybackEpoch> {
        self.epoch
            .0
            .checked_add(1)
            .map(PlaybackEpoch)
            .ok_or_else(|| io::Error::other("playback generation exhausted"))
    }

    fn begin_seek(
        &mut self,
        epoch: PlaybackEpoch,
        position: Duration,
        resume: bool,
    ) -> SeekRequest {
        self.epoch = epoch;
        self.video_tail = false;
        self.position = position;
        self.video_tick = None;
        self.resume_after_seek = resume;
        self.state = PlaybackState::Seeking;
        if let Some(audio) = &mut self.audio {
            audio.rebase(position);
        }
        SeekRequest { epoch, position }
    }

    fn advance_video(&mut self, now: Instant) -> io::Result<()> {
        let previous = self.video_tick.unwrap_or(now);
        let elapsed = now
            .checked_duration_since(previous)
            .ok_or_else(|| invalid("video-only clock moved backward"))?;
        self.position = self
            .position
            .checked_add(elapsed)
            .ok_or_else(|| invalid("video-only media position overflow"))?;
        self.video_tick = Some(now);
        Ok(())
    }

    fn fault(&mut self, error: io::Error) -> io::Error {
        self.state = PlaybackState::Faulted;
        self.video_tick = None;
        self.resume_after_seek = false;
        if let Some(audio) = &mut self.audio {
            audio.pending.clear();
            if let Err(reset) = audio.sink.reset() {
                return io::Error::new(
                    error.kind(),
                    format!("{error}. Audio reset also failed: {reset}"),
                );
            }
            audio.rebase(self.position);
        }
        error
    }
}

impl Drop for Playback {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "playback_tests.rs"]
mod tests;
