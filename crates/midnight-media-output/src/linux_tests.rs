use super::*;
use midnight_media_host::{PcmAdmission, Playback, PlaybackState};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

#[test]
fn delay_counts_only_submitted_media_frames() {
    let mut playout = Playout::default();
    assert_eq!(playout.observe_delay(128).unwrap(), DeviceClock::default());
    playout.submit(480).unwrap();
    assert_eq!(
        playout.observe_delay(512).unwrap(),
        DeviceClock {
            played_frames: 0,
            queued_frames: 480
        },
    );
    assert_eq!(
        playout.observe_delay(96).unwrap(),
        DeviceClock {
            played_frames: 384,
            queued_frames: 96
        },
    );
    assert_eq!(
        playout.observe_delay(100).unwrap(),
        DeviceClock {
            played_frames: 384,
            queued_frames: 96
        },
    );
    assert_eq!(
        playout.observe_delay(-1).unwrap_err().kind(),
        io::ErrorKind::InvalidData,
    );
}

#[test]
fn underrun_with_unconfirmed_downstream_media_keeps_the_clock() {
    let mut playout = Playout::default();
    playout.submit(480).unwrap();
    let previous = playout.observe_delay(24).unwrap();
    assert_eq!(
        playout.drain_underrun(false).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    assert_eq!(playout.clock(), previous);
    playout.drain_underrun(true).unwrap();
    assert_eq!(
        playout.clock(),
        DeviceClock {
            played_frames: 480,
            queued_frames: 0
        }
    );
}

#[test]
fn stopped_zero_delay_needs_prior_ring_only_evidence() {
    let mut evidence = DelayEvidence::default();
    let empty_xrun = PcmStatus {
        state: STATE_XRUN,
        delay: 0,
        available: 9600,
    };
    assert!(!evidence.exhausted(empty_xrun, 9600));
    evidence.observe(480, 480);
    assert!(evidence.exhausted(empty_xrun, 9600));
    evidence.observe(480, 499);
    assert!(!evidence.exhausted(empty_xrun, 9600));
    assert!(evidence.exhausted(
        PcmStatus {
            delay: -4,
            ..empty_xrun
        },
        9600
    ));
    assert!(!evidence.exhausted(
        PcmStatus {
            available: 9599,
            ..empty_xrun
        },
        9600
    ));
}

#[test]
#[ignore = "requires a qualified ALSA default playback device and emits a quiet stereo tone"]
fn default_device_pause_underrun_and_retirement() {
    assert!(AlsaSink::open(c"hw:2147483647").is_err());
    let mut sink = open_default().expect("open the configured ALSA default playback device");
    sink.reset().unwrap();
    sink.start().unwrap();
    thread::sleep(Duration::from_millis(25));
    assert_eq!(sink.sample_clock().unwrap(), DeviceClock::default());

    let pcm = tone();
    let first_total = submit(&mut *sink, &pcm, 0);
    match sink.pause().unwrap() {
        AudioPause::Retained(clock) => {
            check_clock(clock, first_total);
            thread::sleep(Duration::from_millis(25));
            assert_eq!(sink.sample_clock().unwrap(), clock);
            sink.start().unwrap();
            wait_for_played(&mut *sink, first_total);
        }
        AudioPause::Discarded(clock) => {
            check_clock(clock, first_total);
            assert_eq!(clock.queued_frames, 0);
            assert_eq!(sink.sample_clock().unwrap(), DeviceClock::default());
        }
    }

    sink.reset().unwrap();
    sink.start().unwrap();
    let total = submit(&mut *sink, &pcm, 0);
    let drained = wait_for_played(&mut *sink, total);
    thread::sleep(Duration::from_millis(25));
    assert_eq!(sink.sample_clock().unwrap(), drained);
    let total = submit(&mut *sink, &pcm, total);
    wait_for_played(&mut *sink, total);
    submit(&mut *sink, &pcm, total);
    drop(sink);

    let sink = open_default().expect("reopen the ALSA device after an active sink was dropped");
    let mut playback = Playback::with_audio(Duration::ZERO, sink).unwrap();
    playback.play(Instant::now()).unwrap();
    assert!(matches!(
        playback
            .enqueue_pcm(playback.snapshot().epoch, 0, pcm)
            .unwrap(),
        PcmAdmission::Queued
    ));
    let cancelled = AtomicBool::new(false);
    playback.pump(Instant::now(), &cancelled).unwrap();
    cancelled.store(true, Ordering::Release);
    assert_eq!(
        playback
            .pump(Instant::now(), &cancelled)
            .unwrap_err()
            .kind(),
        io::ErrorKind::Interrupted,
    );
    assert_eq!(playback.snapshot().state, PlaybackState::Closed);
}

fn tone() -> Box<[i16]> {
    (0..MAX_PCM_BLOCK_FRAMES)
        .flat_map(|frame| {
            let sample = if (frame / 57) % 2 == 0 { 384 } else { -384 };
            [sample, sample]
        })
        .collect()
}

fn submit(sink: &mut dyn AudioSink, pcm: &[i16], first: u64) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut submitted = 0;
    while submitted * PCM_CHANNELS < pcm.len() {
        assert!(
            Instant::now() < deadline,
            "ALSA did not accept the test PCM block"
        );
        check_clock(sink.sample_clock().unwrap(), first + submitted as u64);
        let accepted = sink.write(&pcm[submitted * PCM_CHANNELS..]).unwrap();
        submitted += accepted;
        if accepted == 0 {
            thread::sleep(Duration::from_millis(5));
        }
    }
    first + submitted as u64
}

fn wait_for_played(sink: &mut dyn AudioSink, submitted: u64) -> DeviceClock {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut played = 0;
    loop {
        let clock = sink.sample_clock().unwrap();
        check_clock(clock, submitted);
        assert!(clock.played_frames >= played);
        if clock.played_frames == submitted {
            assert_eq!(clock.queued_frames, 0);
            return clock;
        }
        assert!(
            Instant::now() < deadline,
            "ALSA did not confirm the test PCM played"
        );
        played = clock.played_frames;
        thread::sleep(Duration::from_millis(5));
    }
}

fn check_clock(clock: DeviceClock, submitted: u64) {
    assert!(clock.played_frames <= submitted);
    assert!(clock.queued_frames <= MAX_DEVICE_FRAMES);
    assert!(clock.queued_frames as u64 <= submitted - clock.played_frames);
}
