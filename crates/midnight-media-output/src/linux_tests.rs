use super::*;
use midnight_media_host::{PcmAdmission, Playback, PlaybackState};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

const RING: u64 = 4_800;
const QUANTUM: u64 = 1_024;

/// PipeWire's ALSA plugin as recorded on a PipeWire 1.6 host with the sink's settings:
/// a 4800-frame ring and a 1024-frame graph quantum. Each graph cycle takes one quantum
/// from the ring, and those frames play during the next cycle. A cycle that finds less
/// than a quantum is an XRUN: the frames left in the ring never play, and until prepare
/// the status repeats the last delay (graph latency plus that cycle's transfer) and the
/// same availability. At cycle boundaries this model reports the recorded values.
struct PipeWire {
    state: c_int,
    ring: u64,
    /// Media frames accepted since the last drop, numbered as the sink submits them.
    written: u64,
    /// Graph latency, reported once a cycle has run.
    latency: u64,
    /// Frames the last cycle took; the plugin keeps them in its delay.
    transferred: u64,
    /// First frame the previous cycle took: it plays now.
    playing: Option<u64>,
    /// End of all frames the graph has taken.
    taken: u64,
    /// Every frame before this one has played or never will.
    heard: u64,
    prepares: usize,
}

impl PipeWire {
    fn new() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            state: STATE_PREPARED,
            ring: 0,
            written: 0,
            latency: 0,
            transferred: 0,
            playing: None,
            taken: 0,
            heard: 0,
            prepares: 0,
        }))
    }

    fn cycle(&mut self) {
        self.heard = self.playing.take().unwrap_or(self.taken);
        if self.state != STATE_RUNNING {
            return;
        }
        self.latency = QUANTUM;
        if self.ring < QUANTUM {
            self.state = STATE_XRUN;
            return;
        }
        let first = self.written - self.ring;
        self.ring -= QUANTUM;
        self.transferred = QUANTUM;
        self.playing = Some(first);
        self.taken = first + QUANTUM;
    }
}

impl Pcm for Rc<RefCell<PipeWire>> {
    fn status(&mut self) -> io::Result<PcmStatus> {
        let device = self.borrow();
        let pending = if device.state == STATE_RUNNING {
            device.ring
        } else {
            0
        };
        Ok(PcmStatus {
            state: device.state,
            delay: (pending + device.latency + device.transferred) as c_long,
            available: (RING - device.ring) as c_ulong,
        })
    }

    fn prepare(&mut self) -> c_int {
        let mut device = self.borrow_mut();
        device.state = STATE_PREPARED;
        device.ring = 0;
        device.prepares += 1;
        0
    }

    fn drop_stream(&mut self) -> c_int {
        let mut device = self.borrow_mut();
        device.state = STATE_SETUP;
        device.ring = 0;
        device.written = 0;
        device.playing = None;
        device.taken = 0;
        device.heard = 0;
        0
    }

    fn pause(&mut self, _: bool) -> c_int {
        -38
    }

    fn write(&mut self, interleaved: &[i16]) -> c_long {
        let mut device = self.borrow_mut();
        match device.state {
            STATE_XRUN => return -32,
            STATE_PREPARED | STATE_RUNNING => {}
            _ => return -77,
        }
        let accepted = (interleaved.len() as u64 / PCM_CHANNELS as u64).min(RING - device.ring);
        if accepted == 0 {
            return -11;
        }
        device.ring += accepted;
        device.written += accepted;
        // The sink's start threshold is one frame.
        device.state = STATE_RUNNING;
        accepted as c_long
    }
}

/// Run one graph cycle, then sample the sink as the playback worker does.
fn cycle_and_sample(
    device: &Rc<RefCell<PipeWire>>,
    sink: &mut AlsaSink<Rc<RefCell<PipeWire>>>,
    previous: DeviceClock,
) -> DeviceClock {
    device.borrow_mut().cycle();
    let clock = sink.sample_clock().unwrap();
    assert!(clock.played_frames >= previous.played_frames);
    assert!(
        clock.played_frames <= device.borrow().heard,
        "{clock:?} counts frames that have not played; heard {}",
        device.borrow().heard
    );
    clock
}

#[test]
fn pipewire_underrun_keeps_the_clock_and_playback_continues() {
    let device = PipeWire::new();
    let mut sink = AlsaSink::new(device.clone(), RING as c_ulong);
    sink.reset().unwrap();
    sink.start().unwrap();
    let block = vec![0; RING as usize * PCM_CHANNELS];
    assert_eq!(sink.write(&block).unwrap(), 4_800);
    let mut clock = sink.sample_clock().unwrap();
    for _ in 0..4 {
        clock = cycle_and_sample(&device, &mut sink, clock);
    }
    assert_eq!(clock.played_frames, 2_048);

    // The fifth cycle finds 704 frames, less than a quantum. They never play.
    device.borrow_mut().cycle();
    let status = device.clone().status().unwrap();
    assert_eq!(
        (status.state, status.delay, status.available),
        (STATE_XRUN, 2_048, 4_096)
    );
    let underrun = sink.sample_clock().unwrap();
    // The plugin's status stays like this; playback must not fail while it does.
    thread::sleep(Duration::from_millis(250));
    assert_eq!(sink.sample_clock().unwrap(), underrun);
    // The device's queue is gone, and none of it counts as played without evidence.
    assert_eq!(
        underrun,
        DeviceClock {
            played_frames: 2_048,
            queued_frames: 0
        }
    );
    assert_eq!(device.borrow().prepares, 2);

    // Playback continues. The clock passes the frames lost in the ring only when the
    // next block plays.
    assert_eq!(sink.write(&block).unwrap(), 4_800);
    clock = cycle_and_sample(&device, &mut sink, underrun);
    assert_eq!(clock.played_frames, 3_776);
    for _ in 0..3 {
        clock = cycle_and_sample(&device, &mut sink, clock);
    }
    assert_eq!(clock.played_frames, 6_848);

    // An underrun found by a write recovers the same way.
    device.borrow_mut().cycle();
    assert_eq!(device.borrow().state, STATE_XRUN);
    assert_eq!(sink.write(&block).unwrap(), 0);
    assert_eq!(device.borrow().prepares, 3);
    assert_eq!(sink.write(&block).unwrap(), 4_800);
    clock = sink.sample_clock().unwrap();
    assert_eq!(clock.played_frames, 7_552);
    assert!(clock.played_frames <= device.borrow().heard);
}

#[test]
fn playback_continues_through_pipewire_underruns_and_drains_at_the_end() {
    let device = PipeWire::new();
    let sink = Box::new(AlsaSink::new(device.clone(), RING as c_ulong));
    let mut playback = Playback::with_audio(Duration::ZERO, sink).unwrap();
    let now = Instant::now();
    playback.play(now).unwrap();
    let epoch = playback.snapshot().epoch;
    let cancelled = AtomicBool::new(false);
    let blocks = 12;
    let mut delivered = 0;
    let mut played = 0;
    for cycle in 0..160 {
        // A decoder that is slow after its first block, as a first 1080p decode can
        // be, and stalls once more mid-stream.
        let stalled = (1..12).contains(&cycle) || (40..70).contains(&cycle);
        if !stalled && delivered < blocks {
            let pcm = vec![0; MAX_PCM_BLOCK_FRAMES * PCM_CHANNELS].into_boxed_slice();
            let first = delivered * MAX_PCM_BLOCK_FRAMES as u64;
            match playback.enqueue_pcm(epoch, first, pcm).unwrap() {
                PcmAdmission::Queued => delivered += 1,
                PcmAdmission::Backpressure(_) => {}
                PcmAdmission::Stale => panic!("one playback generation"),
            }
        }
        let audio = playback.pump(now, &cancelled).unwrap().audio.unwrap();
        let heard = device.borrow().heard;
        assert!(audio.played_frames >= played);
        assert!(
            audio.played_frames <= heard,
            "cycle {cycle}: {audio:?} counts frames that have not played; heard {heard}"
        );
        played = audio.played_frames;
        device.borrow_mut().cycle();
    }
    assert_eq!(delivered, blocks);
    let snapshot = playback.snapshot();
    assert_eq!(snapshot.state, PlaybackState::Playing);
    assert_eq!(snapshot.host_queued_frames, 0);
    // The last block drained: the session can finish on the monotonic clock.
    assert_eq!(snapshot.audio.unwrap().queued_frames, 0);
    assert!(played > (blocks - 1) * MAX_PCM_BLOCK_FRAMES as u64);
    assert!(device.borrow().prepares > 2);
}

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
fn unconfirmed_underrun_retires_the_queue_without_counting_it() {
    let mut playout = Playout::default();
    playout.submit(480).unwrap();
    playout.observe_delay(24).unwrap();
    playout.underrun(false);
    assert_eq!(
        playout.clock(),
        DeviceClock {
            played_frames: 456,
            queued_frames: 0
        }
    );
    // After the restart only running delay moves the clock, here past the 24 retired
    // frames once the delay shows that later media plays.
    playout.submit(480).unwrap();
    assert_eq!(
        playout.observe_delay(600).unwrap(),
        DeviceClock {
            played_frames: 456,
            queued_frames: 480
        }
    );
    assert_eq!(
        playout.observe_delay(400).unwrap(),
        DeviceClock {
            played_frames: 560,
            queued_frames: 400
        }
    );
    playout.underrun(true);
    assert_eq!(
        playout.clock(),
        DeviceClock {
            played_frames: 960,
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
            wait_for_drain(&mut *sink, first_total);
        }
        AudioPause::Discarded(clock) => {
            check_clock(clock, first_total);
            assert_eq!(clock.queued_frames, 0);
            assert_eq!(sink.sample_clock().unwrap(), DeviceClock::default());
        }
    }

    // Each block underruns once it has drained. Playback resumes after the underrun,
    // and the clock passes the first block only when the second one plays.
    sink.reset().unwrap();
    sink.start().unwrap();
    let total = submit(&mut *sink, &pcm, 0);
    let drained = wait_for_drain(&mut *sink, total);
    thread::sleep(Duration::from_millis(25));
    assert_eq!(sink.sample_clock().unwrap(), drained);
    let first = total;
    let total = submit(&mut *sink, &pcm, total);
    let resumed = wait_for_drain(&mut *sink, total);
    assert!(
        resumed.played_frames > first,
        "{resumed:?} after {first} frames"
    );

    // An underrun that a write finds first: nothing samples the device meanwhile.
    let total = submit(&mut *sink, &pcm, total);
    thread::sleep(Duration::from_millis(400));
    assert_eq!(sink.write(&pcm).unwrap(), 0);
    let total = submit(&mut *sink, &pcm, total);
    wait_for_drain(&mut *sink, total);
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

#[test]
#[ignore = "requires a qualified ALSA default playback device and emits a quiet stereo tone"]
fn default_device_playback_continues_after_a_slow_start() {
    let sink = open_default().expect("open the configured ALSA default playback device");
    let mut playback = Playback::with_audio(Duration::ZERO, sink).unwrap();
    let started = Instant::now();
    playback.play(started).unwrap();
    let epoch = playback.snapshot().epoch;
    let cancelled = AtomicBool::new(false);
    let pcm = tone();
    let blocks = 10;
    let mut delivered = 0;
    let mut position = Duration::ZERO;
    while Instant::now() < started + Duration::from_secs(5) {
        let now = Instant::now();
        // The first block comes at once; the decoder then needs 300 ms, as a slow first
        // decode can, and keeps up afterwards.
        let ready = delivered == 0 || now >= started + Duration::from_millis(300);
        if ready && delivered < blocks {
            let first = delivered * MAX_PCM_BLOCK_FRAMES as u64;
            if let PcmAdmission::Queued = playback.enqueue_pcm(epoch, first, pcm.clone()).unwrap() {
                delivered += 1;
            }
        }
        let snapshot = playback.pump(now, &cancelled).unwrap();
        let elapsed = Instant::now() - started;
        assert!(snapshot.position >= position);
        assert!(
            snapshot.position <= elapsed,
            "the clock is at {:?} after {elapsed:?}",
            snapshot.position
        );
        position = snapshot.position;
        let drained = snapshot.audio.is_some_and(|audio| audio.queued_frames == 0);
        if delivered == blocks && snapshot.host_queued_frames == 0 && drained {
            break;
        }
        thread::sleep(Duration::from_millis(12));
    }
    let snapshot = playback.snapshot();
    assert_eq!(snapshot.state, PlaybackState::Playing);
    assert_eq!((delivered, snapshot.host_queued_frames), (blocks, 0));
    assert_eq!(snapshot.audio.unwrap().queued_frames, 0);
    // The clock reached the last of the ten 100 ms blocks.
    assert!(position > Duration::from_millis(900), "{position:?}");
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

/// Wait until nothing is queued: every frame played, or retired by the underrun that
/// follows a drained block. PipeWire never plays the last partial graph quantum.
fn wait_for_drain(sink: &mut dyn AudioSink, submitted: u64) -> DeviceClock {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut played = 0;
    loop {
        let clock = sink.sample_clock().unwrap();
        check_clock(clock, submitted);
        assert!(clock.played_frames >= played);
        if clock.queued_frames == 0 {
            return clock;
        }
        assert!(Instant::now() < deadline, "ALSA did not drain the test PCM");
        played = clock.played_frames;
        thread::sleep(Duration::from_millis(5));
    }
}

fn check_clock(clock: DeviceClock, submitted: u64) {
    assert!(clock.played_frames <= submitted);
    assert!(clock.queued_frames <= MAX_DEVICE_FRAMES);
    assert!(clock.queued_frames as u64 <= submitted - clock.played_frames);
}
