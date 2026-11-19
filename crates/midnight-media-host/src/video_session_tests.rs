use super::*;
use crate::video_transport::{SeekEpoch, SessionGeneration};
use crate::{AudioPause, DeviceClock, PCM_CHANNELS, PCM_RATE};
use std::{cell::Cell, num::NonZeroU64, rc::Rc};

fn identity(session: u64, epoch: u64) -> VideoIdentity {
    VideoIdentity {
        session: SessionGeneration(NonZeroU64::new(session).unwrap()),
        epoch: SeekEpoch(NonZeroU64::new(epoch).unwrap()),
    }
}

fn session() -> VideoSession {
    VideoSession::new(
        identity(2, 1),
        VideoMetadata {
            duration: Some(Duration::from_secs(4)),
            width: 2,
            height: 1,
            audio: AudioTrack::Absent,
        },
        || panic!("silent media does not open a native device"),
    )
}

fn frame_at(seconds: u64) -> VideoFrame {
    VideoFrame {
        presentation: Duration::from_secs(seconds),
        width: 2,
        height: 1,
        rgba: vec![0xff; 8].into_boxed_slice(),
    }
}

fn pull(identity: VideoIdentity, frame: VideoFrame) -> DecoderReply {
    DecoderReply::Pulled(PullResult {
        identity,
        frames: vec![frame],
        pcm: None,
        end_of_media: false,
    })
}

#[test]
fn later_seek_replaces_the_queued_target_and_discards_in_flight_pixels() {
    let now = Instant::now();
    let stopped = AtomicBool::new(false);
    let mut session = session();
    assert!(matches!(
        session.next_request(),
        Some(DecoderRequest::Pull(_))
    ));
    session.seek(Duration::from_secs(1)).unwrap();
    session.seek(Duration::from_secs(2)).unwrap();
    assert!(session.next_request().is_none());
    session
        .accept(pull(identity(2, 1), frame_at(0)), now)
        .unwrap();
    assert!(session.tick(now, &stopped).unwrap().frame.is_none());
    assert!(
        matches!(session.next_request(), Some(DecoderRequest::Seek(value))
        if value == Duration::from_secs(2))
    );
    session
        .accept(DecoderReply::Sought(identity(2, 2)), now)
        .unwrap();
    session.next_request().unwrap();
    session
        .accept(pull(identity(2, 2), frame_at(2)), now)
        .unwrap();
    let tick = session.tick(now, &stopped).unwrap();
    assert_eq!(tick.identity, identity(2, 2));
    assert_eq!(tick.frame.unwrap().presentation, Duration::from_secs(2));
}

#[test]
fn a_reused_seek_epoch_from_a_closed_session_cannot_supply_a_frame() {
    let now = Instant::now();
    let stopped = AtomicBool::new(false);
    let mut session = session();
    session.next_request().unwrap();
    session
        .accept(pull(identity(1, 1), frame_at(0)), now)
        .unwrap();
    assert!(session.tick(now, &stopped).unwrap().frame.is_none());
    session
        .accept(pull(identity(2, 1), frame_at(0)), now)
        .unwrap();
    assert!(session.tick(now, &stopped).unwrap().frame.is_some());
}

#[test]
fn exhausted_audio_advances_the_clock_before_the_video_queue_reaches_eof() {
    use crate::{AudioPause, DeviceClock};

    struct EmptySink;
    impl AudioSink for EmptySink {
        fn start(&mut self) -> io::Result<()> {
            Ok(())
        }
        fn pause(&mut self) -> io::Result<AudioPause> {
            Ok(AudioPause::Retained(DeviceClock::default()))
        }
        fn reset(&mut self) -> io::Result<()> {
            Ok(())
        }
        fn write(&mut self, _: &[i16]) -> io::Result<usize> {
            Ok(0)
        }
        fn sample_clock(&mut self) -> io::Result<DeviceClock> {
            Ok(DeviceClock::default())
        }
    }

    let start = Instant::now();
    let stopped = AtomicBool::new(false);
    let mut session = VideoSession::new(
        identity(2, 1),
        VideoMetadata {
            duration: None,
            width: 2,
            height: 1,
            audio: AudioTrack::Qualified,
        },
        || Ok(Box::new(EmptySink)),
    );
    session.play(start).unwrap();
    assert!(matches!(
        session.next_request(),
        Some(DecoderRequest::Pull(PullCredit {
            audio: AudioDemand::Frames(_),
            ..
        }))
    ));
    session
        .accept(
            DecoderReply::Pulled(PullResult {
                identity: identity(2, 1),
                frames: vec![frame_at(0), frame_at(2), frame_at(3), frame_at(4)],
                pcm: None,
                end_of_media: false,
            }),
            start,
        )
        .unwrap();
    assert_eq!(
        session
            .tick(start, &stopped)
            .unwrap()
            .frame
            .unwrap()
            .presentation,
        Duration::ZERO
    );
    assert!(matches!(
        session.next_request(),
        Some(DecoderRequest::Pull(PullCredit {
            audio: AudioDemand::Discard,
            ..
        }))
    ));
    let later = session
        .tick(start + Duration::from_secs(2), &stopped)
        .unwrap();
    assert_eq!(later.frame.unwrap().presentation, Duration::from_secs(2));
    assert!(!later.finished);
}

fn one_second_preview(now: Instant) -> VideoSession {
    let mut session = session();
    session.metadata.duration = Some(Duration::from_secs(1));
    session.next_request().unwrap();
    session
        .accept(pull(identity(2, 1), frame_at(0)), now)
        .unwrap();
    assert!(
        session
            .tick(now, &AtomicBool::new(false))
            .unwrap()
            .frame
            .is_some()
    );
    session
}

fn eof_reply() -> DecoderReply {
    DecoderReply::Pulled(PullResult {
        identity: identity(2, 1),
        frames: Vec::new(),
        pcm: None,
        end_of_media: true,
    })
}

#[test]
fn eof_keeps_the_last_frame_playing_until_the_known_duration() {
    let start = Instant::now();
    let cancelled = AtomicBool::new(false);
    let mut session = one_second_preview(start);
    session.play(start).unwrap();
    session.next_request().unwrap();
    let early = start + Duration::from_millis(20);
    session.accept(eof_reply(), early).unwrap();
    let tick = session.tick(early, &cancelled).unwrap();
    assert!(
        !tick.finished,
        "decoder EOF must not truncate the displayed final frame"
    );
    assert_eq!(tick.playback.state, PlaybackState::Playing);
    assert!(
        !session
            .tick(start + Duration::from_millis(999), &cancelled)
            .unwrap()
            .finished
    );
    let end = session
        .tick(start + Duration::from_secs(1), &cancelled)
        .unwrap();
    assert!(end.finished);
    assert_eq!(end.playback.position, Duration::from_secs(1));
    assert_eq!(end.playback.state, PlaybackState::Paused);
}

#[test]
fn eof_while_paused_keeps_the_preview_and_resumes_the_remaining_interval() {
    let start = Instant::now();
    let cancelled = AtomicBool::new(false);
    let mut session = one_second_preview(start);
    session.play(start).unwrap();
    session.next_request().unwrap();
    let paused_at = start + Duration::from_millis(20);
    session.pause(paused_at).unwrap();
    session.accept(eof_reply(), paused_at).unwrap();
    let resume_at = start + Duration::from_secs(5);
    let paused = session.tick(resume_at, &cancelled).unwrap();
    assert!(
        !paused.finished,
        "a late EOF reply must not end a paused preview"
    );
    assert_eq!(paused.playback.state, PlaybackState::Paused);
    assert_eq!(paused.playback.position, Duration::from_millis(20));
    session.play(resume_at).unwrap();
    let end = session
        .tick(resume_at + Duration::from_millis(980), &cancelled)
        .unwrap();
    assert!(end.finished);
    assert_eq!(end.playback.position, Duration::from_secs(1));
}

#[test]
fn eof_without_a_duration_finishes_only_after_playback_resumes() {
    let start = Instant::now();
    let cancelled = AtomicBool::new(false);
    let mut session = one_second_preview(start);
    session.metadata.duration = None;
    session.play(start).unwrap();
    session.next_request().unwrap();
    session.pause(start).unwrap();
    session.accept(eof_reply(), start).unwrap();
    assert!(!session.tick(start, &cancelled).unwrap().finished);
    session.play(start).unwrap();
    assert!(session.tick(start, &cancelled).unwrap().finished);
}

struct DiscardingSink {
    submitted: u64,
    pauses: Rc<Cell<usize>>,
}
impl AudioSink for DiscardingSink {
    fn start(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn pause(&mut self) -> io::Result<AudioPause> {
        self.pauses.set(self.pauses.get() + 1);
        Ok(AudioPause::Discarded(DeviceClock {
            played_frames: self.submitted,
            queued_frames: 0,
        }))
    }
    fn reset(&mut self) -> io::Result<()> {
        self.submitted = 0;
        Ok(())
    }
    fn write(&mut self, samples: &[i16]) -> io::Result<usize> {
        let frames = samples.len() / PCM_CHANNELS;
        self.submitted += frames as u64;
        Ok(frames)
    }
    fn sample_clock(&mut self) -> io::Result<DeviceClock> {
        Ok(DeviceClock {
            played_frames: self.submitted,
            queued_frames: 0,
        })
    }
}
#[test]
fn final_pcm_with_eof_hands_the_drained_audio_clock_to_the_video_tail() {
    let start = Instant::now();
    let cancelled = AtomicBool::new(false);
    let pauses = Rc::new(Cell::new(0));
    let mut session = VideoSession::new(
        identity(2, 1),
        VideoMetadata {
            duration: Some(Duration::from_secs(1)),
            width: 2,
            height: 1,
            audio: AudioTrack::Qualified,
        },
        || {
            Ok(Box::new(DiscardingSink {
                submitted: 0,
                pauses: pauses.clone(),
            }))
        },
    );
    session.play(start).unwrap();
    session.next_request().unwrap();
    session
        .accept(
            DecoderReply::Pulled(PullResult {
                identity: identity(2, 1),
                frames: vec![frame_at(0)],
                pcm: Some(crate::video_transport::PcmBlock {
                    first_frame: 0,
                    samples: vec![0; PCM_RATE as usize / 10 * PCM_CHANNELS].into_boxed_slice(),
                }),
                end_of_media: true,
            }),
            start,
        )
        .unwrap();
    assert!(!session.tick(start, &cancelled).unwrap().finished);
    let drained = session
        .tick(start + Duration::from_millis(100), &cancelled)
        .unwrap();
    assert!(
        !drained.finished,
        "audio EOF must leave time for the final video interval"
    );
    assert_eq!(drained.playback.position, Duration::from_millis(100));
    let end = session
        .tick(start + Duration::from_secs(1), &cancelled)
        .unwrap();
    assert!(end.finished);
    assert_eq!(end.playback.position, Duration::from_secs(1));
    assert_eq!(end.playback.state, PlaybackState::Paused);
    assert_eq!(session.player.snapshot().state, PlaybackState::Paused);
    assert_eq!(
        pauses.get(),
        0,
        "completion must retire drained audio before pausing its clock"
    );
}

#[test]
fn initial_eof_without_a_video_frame_reports_a_terminal_error() {
    let start = Instant::now();
    let mut session = session();
    session.next_request().unwrap();
    session.accept(eof_reply(), start).unwrap();
    let result = session.tick(start, &AtomicBool::new(false));
    let error = match result {
        Err(error) => error,
        Ok(_) => panic!("empty video must not remain in its opening preview"),
    };
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    assert!(error.to_string().contains("no decodable frames"));
    assert!(session.next_request().is_none());
}

#[test]
fn a_paused_seek_to_the_end_finishes_without_requiring_another_frame() {
    let start = Instant::now();
    let mut session = one_second_preview(start);
    session.seek(Duration::from_secs(1)).unwrap();
    assert!(
        matches!(session.next_request(), Some(DecoderRequest::Seek(position))
        if position == Duration::from_secs(1))
    );
    session
        .accept(DecoderReply::Sought(identity(2, 2)), start)
        .unwrap();
    session.next_request().unwrap();
    session
        .accept(
            DecoderReply::Pulled(PullResult {
                identity: identity(2, 2),
                frames: Vec::new(),
                pcm: None,
                end_of_media: true,
            }),
            start,
        )
        .unwrap();
    let end = session.tick(start, &AtomicBool::new(false)).unwrap();
    assert!(
        end.finished,
        "seek-to-end must settle without waiting for nonexistent pixels"
    );
    assert_eq!(end.playback.state, PlaybackState::Paused);
    assert_eq!(end.playback.position, Duration::from_secs(1));
    assert!(end.frame.is_none());
}

#[test]
fn an_empty_seek_preview_discards_final_pcm_and_can_restart_through_a_real_seek() {
    let start = Instant::now();
    let pauses = Rc::new(Cell::new(0));
    let mut session = VideoSession::new(
        identity(2, 1),
        VideoMetadata {
            duration: Some(Duration::from_secs(1)),
            width: 2,
            height: 1,
            audio: AudioTrack::Qualified,
        },
        || {
            Ok(Box::new(DiscardingSink {
                submitted: 0,
                pauses: pauses.clone(),
            }))
        },
    );
    session.next_request().unwrap();
    session
        .accept(pull(identity(2, 1), frame_at(0)), start)
        .unwrap();
    assert!(
        session
            .tick(start, &AtomicBool::new(false))
            .unwrap()
            .frame
            .is_some()
    );
    session.seek(Duration::from_secs(1)).unwrap();
    assert!(matches!(
        session.next_request(),
        Some(DecoderRequest::Seek(_))
    ));
    session
        .accept(DecoderReply::Sought(identity(2, 2)), start)
        .unwrap();
    session.next_request().unwrap();
    session
        .accept(
            DecoderReply::Pulled(PullResult {
                identity: identity(2, 2),
                frames: Vec::new(),
                pcm: Some(crate::video_transport::PcmBlock {
                    first_frame: 0,
                    samples: vec![0; PCM_CHANNELS * 100].into_boxed_slice(),
                }),
                end_of_media: true,
            }),
            start,
        )
        .unwrap();
    let end = session.tick(start, &AtomicBool::new(false)).unwrap();
    assert!(
        end.finished,
        "a paused empty seek must not wait for PCM that cannot play"
    );
    assert_eq!(end.playback.state, PlaybackState::Paused);
    assert_eq!(end.playback.position, Duration::from_secs(1));
    assert_eq!(end.playback.host_queued_frames, 0);
    assert_eq!(end.playback.audio.unwrap().queued_frames, 0);
    assert_eq!(
        pauses.get(),
        0,
        "discard through reset must not create an unacknowledged seek"
    );
    session.play(start).unwrap();
    assert!(
        matches!(session.next_request(), Some(DecoderRequest::Seek(position)) if position.is_zero())
    );
    session
        .accept(DecoderReply::Sought(identity(2, 3)), start)
        .unwrap();
    assert!(matches!(
        session.next_request(),
        Some(DecoderRequest::Pull(PullCredit {
            audio: AudioDemand::Frames(_),
            ..
        }))
    ));
}
