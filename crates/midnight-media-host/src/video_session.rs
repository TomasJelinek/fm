//! Pace a worker-owned video stream against the native audio or monotonic clock.
use crate::{
    AudioSink, AudioVolume, MAX_HOST_FRAMES, MAX_PCM_BLOCK_FRAMES, PcmAdmission, Playback,
    PlaybackEpoch, PlaybackSnapshot, PlaybackState, SeekRequest,
    video_transport::{
        AudioDemand, AudioTrack, PullCredit, PullResult, VideoFrame, VideoIdentity, VideoMetadata,
    },
};
use std::{
    collections::VecDeque,
    io,
    num::NonZeroU16,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

const VIDEO_QUEUE: usize = 4;

pub enum DecoderRequest {
    Pull(PullCredit),
    Seek(Duration),
}

pub enum DecoderReply {
    Pulled(PullResult),
    Sought(VideoIdentity),
}

pub struct VideoTick {
    pub identity: VideoIdentity,
    pub epoch: PlaybackEpoch,
    pub playback: PlaybackSnapshot,
    pub frame: Option<VideoFrame>,
    pub buffering: bool,
    pub finished: bool,
}

enum Active {
    Pull {
        epoch: PlaybackEpoch,
        expected_pcm: bool,
    },
    Seek(SeekRequest),
}

pub struct VideoSession {
    identity: VideoIdentity,
    metadata: VideoMetadata,
    player: Playback,
    audio_notice: Option<String>,
    pending_seek: Option<SeekRequest>,
    active: Option<Active>,
    queued: VecDeque<VideoFrame>,
    preview: bool,
    ever_presented_frame: bool,
    eof: bool,
    audio_ended: bool,
    finished: bool,
}

impl VideoSession {
    /// Open audio output for a qualified track, played at the level `volume` holds.
    pub fn new(
        identity: VideoIdentity,
        metadata: VideoMetadata,
        volume: Arc<AudioVolume>,
        open_audio: impl FnOnce() -> io::Result<Box<dyn AudioSink>>,
    ) -> Self {
        let (player, audio_notice) = match metadata.audio {
            AudioTrack::Qualified => {
                match open_audio()
                    .and_then(|sink| Playback::with_audio_volume(Duration::ZERO, sink, volume))
                {
                    Ok(player) => (player, None),
                    Err(error) => (
                        Playback::video_only(Duration::ZERO),
                        Some(format!("Audio output unavailable: {error}")),
                    ),
                }
            }
            AudioTrack::Unqualified => (
                Playback::video_only(Duration::ZERO),
                Some("This video’s audio codec is unavailable.".into()),
            ),
            AudioTrack::Absent => (Playback::video_only(Duration::ZERO), None),
        };
        Self {
            identity,
            metadata,
            player,
            audio_notice,
            pending_seek: None,
            active: None,
            queued: VecDeque::with_capacity(VIDEO_QUEUE),
            preview: true,
            ever_presented_frame: false,
            eof: false,
            audio_ended: false,
            finished: false,
        }
    }

    pub fn audio_notice(&self) -> Option<&str> {
        self.audio_notice.as_deref()
    }

    /// Whether the session opened an audio output, so a volume change can be heard.
    pub fn plays_audio(&self) -> bool {
        self.player.snapshot().audio.is_some()
    }

    pub fn play(&mut self, now: Instant) -> io::Result<()> {
        if self.finished {
            self.seek(Duration::ZERO)?;
        }
        self.player.play(now)
    }

    pub fn pause(&mut self, now: Instant) -> io::Result<()> {
        if let Some(seek) = self.player.pause(now)? {
            self.queue_seek(seek);
        }
        Ok(())
    }

    pub fn seek(&mut self, position: Duration) -> io::Result<PlaybackEpoch> {
        let position = self
            .metadata
            .duration
            .map_or(position, |end| position.min(end));
        let seek = self.player.seek(position)?;
        self.queue_seek(seek);
        Ok(seek.epoch)
    }

    fn queue_seek(&mut self, seek: SeekRequest) {
        self.pending_seek = Some(seek);
        self.queued.clear();
        self.preview = true;
        self.eof = false;
        self.audio_ended = false;
        self.finished = false;
    }

    /// The transport owner services the one active request, including all source
    /// reads, before sending the next. Rapid seeks replace the queued intent.
    pub fn next_request(&mut self) -> Option<DecoderRequest> {
        if self.active.is_some() {
            return None;
        }
        if let Some(seek) = self.pending_seek.take() {
            self.active = Some(Active::Seek(seek));
            return Some(DecoderRequest::Seek(seek.position));
        }
        let state = self.player.snapshot();
        if self.eof || state.state == PlaybackState::Seeking {
            return None;
        }
        let playing = state.state == PlaybackState::Playing;
        if !(playing || self.preview && self.queued.is_empty()) {
            return None;
        }
        let video_frames = if playing {
            (VIDEO_QUEUE - self.queued.len()) as u8
        } else {
            1
        };
        let audio = if state.audio.is_some() && !self.audio_ended {
            let count = (MAX_HOST_FRAMES - state.host_queued_frames).min(MAX_PCM_BLOCK_FRAMES);
            // Zero PCM credit selects explicit discard at the worker. Wait for
            // space here so audio media is never skipped under backpressure.
            AudioDemand::Frames(NonZeroU16::new(count as u16)?)
        } else {
            AudioDemand::Discard
        };
        if video_frames == 0 && matches!(audio, AudioDemand::Discard) {
            return None;
        }
        self.active = Some(Active::Pull {
            epoch: state.epoch,
            expected_pcm: matches!(audio, AudioDemand::Frames(_)),
        });
        Some(DecoderRequest::Pull(PullCredit {
            video_frames,
            audio,
        }))
    }

    pub fn accept(&mut self, reply: DecoderReply, now: Instant) -> io::Result<()> {
        let session = match &reply {
            DecoderReply::Pulled(result) => result.identity.session,
            DecoderReply::Sought(identity) => identity.session,
        };
        if session != self.identity.session {
            return Ok(());
        }
        match (self.active.take(), reply) {
            (Some(Active::Seek(seek)), DecoderReply::Sought(identity)) => {
                if identity.epoch == self.identity.epoch {
                    return Err(invalid("decoder did not advance its seek epoch"));
                }
                self.identity = identity;
                self.player.acknowledge_seek(seek.epoch, now)?;
            }
            (
                Some(Active::Pull {
                    epoch,
                    expected_pcm,
                }),
                DecoderReply::Pulled(result),
            ) => {
                if result.identity != self.identity {
                    return Err(invalid("decoder replied to a different seek epoch"));
                }
                if self.player.snapshot().epoch != epoch {
                    return Ok(());
                }
                if self.queued.len() + result.frames.len() > VIDEO_QUEUE {
                    return Err(invalid("decoder exceeded the reserved video queue"));
                }
                if let Some(pcm) = result.pcm {
                    match self
                        .player
                        .enqueue_pcm(epoch, pcm.first_frame, pcm.samples)?
                    {
                        PcmAdmission::Queued => {}
                        _ => return Err(invalid("decoder exceeded the reserved PCM queue")),
                    }
                } else if expected_pcm {
                    // An accepted positive credit returns one complete block
                    // until this epoch's audio track reaches EOF.
                    self.audio_ended = true;
                }
                self.queued.extend(result.frames);
                self.eof = result.end_of_media;
            }
            _ => return Err(invalid("unexpected decoder reply")),
        }
        Ok(())
    }

    pub fn tick(&mut self, now: Instant, cancelled: &AtomicBool) -> io::Result<VideoTick> {
        let mut playback = self.player.pump(now, cancelled)?;
        let mut frame = None;
        if playback.state != PlaybackState::Seeking && !self.finished {
            if self.preview {
                frame = self.queued.pop_front();
                self.preview = frame.is_none();
            }
            while self
                .queued
                .front()
                .is_some_and(|front| front.presentation <= playback.position)
            {
                frame = self.queued.pop_front();
            }
            self.ever_presented_frame |= frame.is_some();
            if self.eof && self.preview {
                if !self.ever_presented_frame {
                    return Err(invalid("Video contains no decodable frames"));
                }
                // An empty seek preview has no frame interval to retain. Stop
                // and discard final PCM even while paused; replay still seeks
                // through the decoder before admitting another generation.
                self.player.finish_at_eof()?;
                playback = self.player.snapshot();
                self.finished = true;
            } else {
                let audio_drained = playback.host_queued_frames == 0
                    && playback.audio.is_none_or(|audio| audio.queued_frames == 0);
                // EOF may accompany the final PCM block. Drain it, then let the
                // monotonic clock retain the last video frame for its full interval.
                if (self.audio_ended || self.eof) && audio_drained {
                    self.player.begin_video_tail(now)?;
                }
                if self.eof
                    && self.queued.is_empty()
                    && audio_drained
                    && playback.state == PlaybackState::Playing
                    && self
                        .metadata
                        .duration
                        .is_none_or(|end| playback.position >= end)
                {
                    self.pause(now)?;
                    playback = self.player.snapshot();
                    self.finished = playback.state == PlaybackState::Paused;
                }
            }
        }
        Ok(VideoTick {
            identity: self.identity,
            epoch: playback.epoch,
            playback,
            frame,
            buffering: !self.finished && self.preview && playback.state == PlaybackState::Playing,
            finished: self.finished,
        })
    }

    pub fn close(&mut self) -> io::Result<()> {
        self.queued.clear();
        self.player.close()
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "video_session_tests.rs"]
mod tests;
