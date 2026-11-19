use super::*;
use std::{cell::RefCell, rc::Rc, sync::atomic::AtomicBool};

#[derive(Default)]
struct Device {
    starts: usize,
    resets: usize,
    submitted: u64,
    writes: usize,
    played: u64,
    partial: Option<usize>,
    pause_discards: bool,
    reset_fails: bool,
    cancel_on_observe: Option<Rc<AtomicBool>>,
}

impl Device {
    fn snapshot(&self) -> DeviceClock {
        DeviceClock {
            played_frames: self.played,
            queued_frames: self.submitted.saturating_sub(self.played) as usize,
        }
    }
}

struct Sink(Rc<RefCell<Device>>);

impl AudioSink for Sink {
    fn start(&mut self) -> io::Result<()> {
        self.0.borrow_mut().starts += 1;
        Ok(())
    }

    fn pause(&mut self) -> io::Result<AudioPause> {
        let mut device = self.0.borrow_mut();
        let clock = device.snapshot();
        if device.pause_discards {
            device.submitted = 0;
            device.played = 0;
            Ok(AudioPause::Discarded(clock))
        } else {
            Ok(AudioPause::Retained(clock))
        }
    }

    fn reset(&mut self) -> io::Result<()> {
        let mut device = self.0.borrow_mut();
        if device.reset_fails {
            return Err(io::Error::other("audio device failed to reset"));
        }
        device.resets += 1;
        device.submitted = 0;
        device.played = 0;
        Ok(())
    }

    fn write(&mut self, interleaved: &[i16]) -> io::Result<usize> {
        let mut device = self.0.borrow_mut();
        let frames = interleaved.len() / PCM_CHANNELS;
        let accepted = device.partial.map_or(frames, |limit| frames.min(limit));
        device.submitted += accepted as u64;
        device.writes += 1;
        Ok(accepted)
    }

    fn sample_clock(&mut self) -> io::Result<DeviceClock> {
        let device = self.0.borrow();
        if let Some(cancel) = &device.cancel_on_observe {
            cancel.store(true, Ordering::Release);
        }
        Ok(device.snapshot())
    }
}

fn audio(position: Duration) -> (Playback, Rc<RefCell<Device>>) {
    let device = Rc::new(RefCell::new(Device::default()));
    let playback = Playback::with_audio(position, Box::new(Sink(device.clone()))).unwrap();
    (playback, device)
}

fn pcm(frames: usize) -> Box<[i16]> {
    vec![1; frames * PCM_CHANNELS].into_boxed_slice()
}

#[test]
fn device_frames_drive_media_position_and_underrun_does_not_free_run() {
    let start = Duration::from_secs(2);
    let now = Instant::now();
    let (mut player, device) = audio(start);
    let epoch = player.snapshot().epoch;
    player.play(now).unwrap();
    assert!(matches!(
        player.enqueue_pcm(epoch, 0, pcm(480)).unwrap(),
        PcmAdmission::Queued
    ));
    let live = AtomicBool::new(false);
    assert_eq!(
        player
            .pump(now + Duration::from_secs(4), &live)
            .unwrap()
            .position,
        start
    );
    device.borrow_mut().played = 240;
    let seen = player.pump(now + Duration::from_secs(8), &live).unwrap();
    assert_eq!(seen.position, start + Duration::from_millis(5));
    assert_eq!(seen.audio.unwrap().queued_frames, 240);
    device.borrow_mut().played = 480;
    assert_eq!(
        player
            .pump(now + Duration::from_secs(60), &live)
            .unwrap()
            .position,
        start + Duration::from_millis(10),
    );
    assert_eq!(
        player
            .pump(now + Duration::from_secs(120), &live)
            .unwrap()
            .position,
        start + Duration::from_millis(10),
    );
}

#[test]
fn pcm_backpressure_bounds_both_host_and_device_queues() {
    let now = Instant::now();
    let (mut player, device) = audio(Duration::ZERO);
    let epoch = player.snapshot().epoch;
    player.play(now).unwrap();
    let mut offset = 0;
    while offset < MAX_HOST_FRAMES {
        let frames = (MAX_HOST_FRAMES - offset).min(MAX_PCM_BLOCK_FRAMES);
        assert!(matches!(
            player
                .enqueue_pcm(epoch, offset as u64, pcm(frames))
                .unwrap(),
            PcmAdmission::Queued
        ));
        offset += frames;
    }
    let PcmAdmission::Backpressure(returned) =
        player.enqueue_pcm(epoch, offset as u64, pcm(1)).unwrap()
    else {
        panic!("a full PCM queue must return the unaccepted chunk");
    };
    assert_eq!(returned, pcm(1));
    let live = AtomicBool::new(false);
    for _ in 0..10 {
        player.pump(now, &live).unwrap();
    }
    assert_eq!(device.borrow().submitted, MAX_DEVICE_FRAMES as u64);
    assert_eq!(
        player.snapshot().host_queued_frames,
        MAX_HOST_FRAMES - MAX_DEVICE_FRAMES
    );
    device.borrow_mut().played = 16;
    device.borrow_mut().partial = Some(8);
    player.pump(now, &live).unwrap();
    assert_eq!(device.borrow().submitted, (MAX_DEVICE_FRAMES + 8) as u64);
    assert_eq!(
        player.snapshot().host_queued_frames,
        MAX_HOST_FRAMES - MAX_DEVICE_FRAMES - 8,
    );
}

#[test]
fn seek_invalidates_old_pcm_and_waits_for_matching_decoder_acknowledgement() {
    let now = Instant::now();
    let (mut player, device) = audio(Duration::ZERO);
    let old_epoch = player.snapshot().epoch;
    player.play(now).unwrap();
    player.enqueue_pcm(old_epoch, 0, pcm(480)).unwrap();
    let live = AtomicBool::new(false);
    player.pump(now, &live).unwrap();

    let seek = player.seek(Duration::from_secs(12)).unwrap();
    assert_ne!(seek.epoch, old_epoch);
    assert_eq!(player.snapshot().state, PlaybackState::Seeking);
    assert_eq!(player.snapshot().host_queued_frames, 0);
    assert_eq!(device.borrow().submitted, 0);
    assert!(matches!(
        player.enqueue_pcm(old_epoch, 480, pcm(2)).unwrap(),
        PcmAdmission::Stale
    ));
    assert!(matches!(
        player.enqueue_pcm(seek.epoch, 0, pcm(2)).unwrap(),
        PcmAdmission::Backpressure(_)
    ));
    assert!(!player.acknowledge_seek(old_epoch, now).unwrap());
    assert_eq!(device.borrow().starts, 1);
    assert!(player.acknowledge_seek(seek.epoch, now).unwrap());
    assert_eq!(player.snapshot().state, PlaybackState::Playing);
    assert_eq!(player.snapshot().position, seek.position);
    assert_eq!(device.borrow().starts, 2);
}

#[test]
fn paused_device_retains_pending_pcm_and_freezes_media_clock() {
    let now = Instant::now();
    let (mut player, device) = audio(Duration::ZERO);
    let epoch = player.snapshot().epoch;
    player.play(now).unwrap();
    player.enqueue_pcm(epoch, 0, pcm(480)).unwrap();
    let live = AtomicBool::new(false);
    player.pump(now, &live).unwrap();
    device.borrow_mut().played = 240;
    assert!(player.pause(now).unwrap().is_none());
    assert_eq!(player.snapshot().position, Duration::from_millis(5));
    assert_eq!(
        player
            .pump(now + Duration::from_secs(80), &live)
            .unwrap()
            .position,
        Duration::from_millis(5)
    );
    assert_eq!(device.borrow().submitted, 480);
    player.play(now).unwrap();
    assert_eq!(player.snapshot().epoch, epoch);
}

#[test]
fn pause_that_discards_device_data_produces_a_new_seek_at_the_audible_sample() {
    let now = Instant::now();
    let (mut player, device) = audio(Duration::from_secs(1));
    let epoch = player.snapshot().epoch;
    player.play(now).unwrap();
    player.enqueue_pcm(epoch, 0, pcm(480)).unwrap();
    player.pump(now, &AtomicBool::new(false)).unwrap();
    {
        let mut device = device.borrow_mut();
        device.played = 240;
        device.pause_discards = true;
    }
    let seek = player
        .pause(now)
        .unwrap()
        .expect("flushed audio must be decoded again");
    assert_ne!(seek.epoch, epoch);
    assert_eq!(seek.position, Duration::from_millis(1_005));
    assert!(player.acknowledge_seek(seek.epoch, now).unwrap());
    assert_eq!(player.snapshot().state, PlaybackState::Paused);
    assert_eq!(device.borrow().starts, 1);
}

#[test]
fn cancellation_and_drop_stop_audio_and_discard_queued_samples() {
    let now = Instant::now();
    let (mut player, device) = audio(Duration::ZERO);
    player.play(now).unwrap();
    player
        .enqueue_pcm(player.snapshot().epoch, 0, pcm(480))
        .unwrap();
    assert_eq!(
        player.pump(now, &AtomicBool::new(true)).unwrap_err().kind(),
        io::ErrorKind::Interrupted,
    );
    assert_eq!(player.snapshot().state, PlaybackState::Closed);
    assert_eq!(player.snapshot().host_queued_frames, 0);
    assert_eq!(device.borrow().resets, 2);
    drop(player);
    assert_eq!(device.borrow().resets, 2);
    let (player, device) = audio(Duration::ZERO);
    drop(player);
    assert_eq!(device.borrow().resets, 2);
}

#[test]
fn cancellation_during_clock_observation_precedes_any_further_submission() {
    let now = Instant::now();
    let (mut player, device) = audio(Duration::ZERO);
    player.play(now).unwrap();
    player
        .enqueue_pcm(player.snapshot().epoch, 0, pcm(480))
        .unwrap();
    let cancel = Rc::new(AtomicBool::new(false));
    device.borrow_mut().cancel_on_observe = Some(cancel.clone());
    assert_eq!(
        player.pump(now, &cancel).unwrap_err().kind(),
        io::ErrorKind::Interrupted
    );
    assert_eq!(device.borrow().writes, 0);
    assert_eq!(player.snapshot().state, PlaybackState::Closed);
}

#[test]
fn video_only_uses_monotonic_time_and_excludes_paused_intervals() {
    let now = Instant::now();
    let mut player = Playback::video_only(Duration::ZERO);
    let live = AtomicBool::new(false);
    player.play(now).unwrap();
    player.pause(now + Duration::from_secs(3)).unwrap();
    let later = now + Duration::from_secs(12);
    assert_eq!(
        player.pump(later, &live).unwrap().position,
        Duration::from_secs(3)
    );
    player.play(later).unwrap();
    assert_eq!(
        player
            .pump(later + Duration::from_secs(2), &live)
            .unwrap()
            .position,
        Duration::from_secs(5),
    );
    assert!(player.snapshot().audio.is_none());
}

#[test]
fn malformed_pcm_and_invalid_device_clock_fail_explicitly() {
    let now = Instant::now();
    let (mut player, device) = audio(Duration::ZERO);
    let epoch = player.snapshot().epoch;
    assert_eq!(
        player
            .enqueue_pcm(epoch, 0, vec![1].into_boxed_slice())
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData,
    );
    assert_eq!(
        player.enqueue_pcm(epoch, 1, pcm(1)).unwrap_err().kind(),
        io::ErrorKind::InvalidData,
    );
    player.play(now).unwrap();
    device.borrow_mut().played = 1;
    assert_eq!(
        player
            .pump(now, &AtomicBool::new(false))
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData,
    );
    assert_eq!(player.snapshot().state, PlaybackState::Faulted);
    assert!(player.play(now).is_err());
}

#[test]
fn a_failed_seek_reset_does_not_acknowledge_a_new_audio_generation() {
    let (mut player, device) = audio(Duration::ZERO);
    let old = player.snapshot().epoch;
    device.borrow_mut().reset_fails = true;
    assert!(player.seek(Duration::from_secs(9)).is_err());
    assert_eq!(player.snapshot().epoch, old);
    assert_eq!(player.snapshot().state, PlaybackState::Faulted);
    assert!(!player.acknowledge_seek(old, Instant::now()).unwrap());
}

#[test]
fn drained_audio_hands_a_longer_video_tail_to_the_monotonic_clock() {
    let now = Instant::now();
    let stopped = AtomicBool::new(false);
    let device = Rc::new(RefCell::new(Device::default()));
    let mut player = Playback::with_audio(Duration::ZERO, Box::new(Sink(device.clone()))).unwrap();
    player.play(now).unwrap();
    player.begin_video_tail(now).unwrap();
    let tail = now + Duration::from_millis(250);
    assert_eq!(
        player.pump(tail, &stopped).unwrap().position,
        Duration::from_millis(250)
    );
    assert_eq!(player.pause(tail).unwrap(), None);
    player.play(tail + Duration::from_secs(1)).unwrap();
    assert_eq!(device.borrow().starts, 1);
    let seek = player.seek(Duration::ZERO).unwrap();
    player
        .acknowledge_seek(seek.epoch, tail + Duration::from_secs(1))
        .unwrap();
    assert_eq!(device.borrow().starts, 2);
}
