use super::{
    AudioPause, AudioSink, DeviceClock, MAX_DEVICE_FRAMES, PCM_CHANNELS, Timeline, open_default,
    stream_latency_ticks,
};
use std::{
    io, thread,
    time::{Duration, Instant},
};

fn frames(played_frames: u64, queued_frames: usize) -> DeviceClock {
    DeviceClock {
        played_frames,
        queued_frames,
    }
}

#[test]
fn stream_latency_accepts_zero_and_rounds_positive_clock_ticks_up() {
    assert_eq!(stream_latency_ticks(0, 192_000, 2_400).unwrap(), 0);
    assert_eq!(stream_latency_ticks(1, 192_000, 2_400).unwrap(), 1);
    assert_eq!(stream_latency_ticks(500_000, 192_000, 2_400).unwrap(), 9_600);

    let mut clock = Timeline::new(stream_latency_ticks(0, 192_000, 2_400).unwrap());
    assert_eq!(clock.observe(0, 4, 4).unwrap(), frames(0, 4));
    assert_eq!(clock.observe(10, 4, 0).unwrap(), frames(4, 0));
    assert_eq!(clock.observe(500, 4, 0).unwrap(), frames(4, 0));
}

#[test]
fn stream_latency_rejects_negative_duration_and_zero_frequency() {
    for (latency, frequency) in [(-1, 192_000), (0, 0), (1, 0)] {
        assert_eq!(
            stream_latency_ticks(latency, frequency, 2_400)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}

#[test]
fn stream_latency_rejects_native_clock_conversion_overflow() {
    assert_eq!(
        stream_latency_ticks(i64::MAX, u64::MAX, 2_400)
            .unwrap_err()
            .kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn clock_counts_media_after_downstream_latency_and_ignores_starvation() {
    let mut clock = Timeline::new(20);
    assert_eq!(clock.observe(0, 4, 4).unwrap(), frames(0, 4));
    assert_eq!(clock.observe(10, 4, 0).unwrap(), frames(0, 4));
    assert_eq!(clock.observe(29, 4, 0).unwrap(), frames(0, 4));
    assert_eq!(clock.observe(30, 4, 0).unwrap(), frames(4, 0));

    assert_eq!(clock.observe(500, 4, 0).unwrap(), frames(4, 0));
    assert_eq!(clock.observe(500, 6, 2).unwrap(), frames(4, 2));
    assert_eq!(clock.observe(510, 6, 0).unwrap(), frames(4, 2));
    assert_eq!(clock.observe(529, 6, 0).unwrap(), frames(4, 2));
    assert_eq!(clock.observe(530, 6, 0).unwrap(), frames(6, 0));
}

#[test]
fn clock_rejects_inconsistent_native_samples_and_clears_pending_frames_on_reset() {
    let mut clock = Timeline::new(4);
    assert_eq!(clock.observe(8, 6, 3).unwrap(), frames(0, 6));
    for (position, padding) in [(7, 3), (9, 4), (9, 7)] {
        assert_eq!(
            clock.observe(position, 6, padding).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
    assert_eq!(clock.observe(12, 6, 3).unwrap(), frames(3, 3));
    assert_eq!(clock.observe(13, 6, 0).unwrap(), frames(3, 3));
    clock.reset();
    assert_eq!(clock.observe(0, 0, 0).unwrap(), DeviceClock::default());
    assert_eq!(clock.observe(20, 0, 0).unwrap(), DeviceClock::default());
}

#[test]
#[ignore = "requires a usable default Windows render endpoint; submits silence only"]
fn default_endpoint_bounds_queue_and_preserves_pause_and_reset_clock() {
    let mut sink = open_default().expect("open the default shared render endpoint");
    assert_eq!(sink.sample_clock().unwrap(), DeviceClock::default());
    assert_eq!(
        sink.write(&[0]).unwrap_err().kind(),
        io::ErrorKind::InvalidInput
    );

    let silence = vec![0; (MAX_DEVICE_FRAMES + 1) * PCM_CHANNELS];
    let accepted = sink.write(&silence).unwrap();
    assert!((1..=MAX_DEVICE_FRAMES).contains(&accepted));
    assert_eq!(sink.write(&silence).unwrap(), 0);
    let primed = frames(0, accepted);
    assert_eq!(sink.pause().unwrap(), AudioPause::Retained(primed));
    assert_eq!(sink.sample_clock().unwrap(), primed);

    sink.start().unwrap();
    let played = frames(accepted as u64, 0);
    wait_until_played(&mut *sink, played);
    thread::sleep(Duration::from_millis(50));
    assert_eq!(sink.sample_clock().unwrap(), played);
    assert_eq!(sink.pause().unwrap(), AudioPause::Retained(played));

    let queued = sink.write(&silence).unwrap();
    assert!(queued > 0);
    let paused = frames(accepted as u64, queued);
    thread::sleep(Duration::from_millis(30));
    assert_eq!(sink.sample_clock().unwrap(), paused);
    sink.reset().unwrap();
    assert_eq!(sink.sample_clock().unwrap(), DeviceClock::default());

    sink.start().unwrap();
    assert_eq!(sink.sample_clock().unwrap(), DeviceClock::default());
    let tail = sink.write(&silence[..PCM_CHANNELS * 1024]).unwrap();
    assert!(tail > 0);
    wait_until_played(&mut *sink, frames(tail as u64, 0));
    // Release an active, primed stream on the same thread.
    assert!(sink.write(&silence).unwrap() > 0);
}

fn wait_until_played(sink: &mut dyn AudioSink, expected: DeviceClock) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let sample = sink.sample_clock().unwrap();
        if sample == expected {
            return;
        }
        assert!(sample.played_frames <= expected.played_frames);
        assert_eq!(
            sample.played_frames + sample.queued_frames as u64,
            expected.played_frames
        );
        assert!(Instant::now() < deadline, "last device sample: {sample:?}");
        thread::sleep(Duration::from_millis(5));
    }
}
