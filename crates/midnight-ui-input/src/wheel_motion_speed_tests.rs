//! Glide never scrolls slower than glide Off: simulated-clock runs of single notches,
//! wheel bursts and held scroll keys at every Scrolling glide stop, compared with Off
//! (docs/verification/2026-11-24-scroll-glide-speed.md).
use super::*;

/// Presented frames at 60 Hz, in milliseconds.
const FRAME_MS: f64 = 1000.0 / 60.0;
/// The Scrolling glide stops' scales: Off, Very short, Short, Default, Long, Very long.
const SCALES: [f32; 6] = [0.0, 0.25, 0.5, 1.0, 2.0, 4.0];

fn at(millis: f64) -> Duration {
    Duration::from_secs_f64(millis / 1000.0)
}

/// One simulated run: the position after every presented frame.
struct Run {
    /// (frame time ms, position px after the frame).
    frames: Vec<(f64, f64)>,
    inputs: Vec<(f64, f32)>,
}

impl Run {
    fn total(&self) -> f64 {
        self.frames.last().map_or(0.0, |frame| frame.1)
    }

    /// The position at `millis` (after the last frame at or before it).
    fn position(&self, millis: f64) -> f64 {
        self.frames
            .iter()
            .take_while(|frame| frame.0 <= millis + 1e-9)
            .last()
            .map_or(0.0, |frame| frame.1)
    }

    /// The longest time from an input until the run has delivered `share` of its
    /// distance on top of all earlier inputs' (inputs in one direction; Off: under a
    /// frame). With `share` 1, how late the run reaches an input's target.
    fn longest_lag(&self, share: f64) -> f64 {
        let mut lag: f64 = 0.0;
        let mut before = 0.0;
        for &(input, pixels) in &self.inputs {
            let pixels = f64::from(pixels);
            let goal = before + pixels * share;
            before += pixels;
            let reached = self
                .frames
                .iter()
                .find(|frame| {
                    frame.0 >= input - 1e-9 && (frame.1 - goal) * pixels.signum() >= -1e-3
                })
                .map_or(f64::INFINITY, |frame| frame.0);
            lag = lag.max(reached - input);
        }
        lag
    }

    /// The time of the frame that reached the final position.
    fn settled(&self) -> f64 {
        let total = self.total();
        self.frames
            .iter()
            .find(|frame| (frame.1 - total).abs() < 1e-3)
            .map_or(f64::INFINITY, |frame| frame.0)
    }

    /// Each frame's movement.
    fn steps(&self) -> Vec<f64> {
        let mut previous = 0.0;
        self.frames
            .iter()
            .map(|frame| {
                let step = frame.1 - previous;
                previous = frame.1;
                step
            })
            .collect()
    }
}

/// Feed `inputs` (time ms, vertical px, sorted) to a [`WheelMotion`] at `scale` and
/// present frames every `frame_ms` until `until` ms. Input at a frame's time arrives
/// before that frame, as a native event precedes the redraw it requests. Glide Off is
/// scale 0: each input moves whole on the next frame, as the hosts' direct path does.
fn simulate(inputs: &[(f64, f32)], scale: f32, frame_ms: f64, until: f64) -> Run {
    let mut motion = WheelMotion::default();
    let mut next = 0;
    let mut position = 0.0;
    let mut frames = Vec::new();
    let mut frame = 0u32;
    loop {
        let now = f64::from(frame) * frame_ms;
        if now > until {
            break;
        }
        while next < inputs.len() && inputs[next].0 <= now + 1e-9 {
            let (time, pixels) = inputs[next];
            motion.push_scaled(0.0, pixels, at(time), scale);
            next += 1;
        }
        if let Some((_, y)) = motion.tick(at(now)) {
            position += f64::from(y);
        }
        frames.push((now, position));
        frame += 1;
    }
    Run {
        frames,
        inputs: inputs.to_vec(),
    }
}

/// `count` inputs of `pixels` every `spacing` ms from 0.
fn burst(count: u32, spacing: f64, pixels: f32) -> Vec<(f64, f32)> {
    (0..count)
        .map(|i| (f64::from(i) * spacing, pixels))
        .collect()
}

/// A key held for `held` ms: one press, then Windows' default repeat (a 500 ms delay,
/// then about 30 a second).
fn held_key(held: f64, pixels: f32) -> Vec<(f64, f32)> {
    let mut inputs = vec![(0.0, pixels)];
    let mut time = 500.0;
    while time <= held {
        inputs.push((time, pixels));
        time += 1000.0 / 30.0;
    }
    inputs
}

/// The input sequences every stop must keep up with: a notch at Windows' default and at
/// one screen, spins at typical rates (also upwards and at Fast), and held keys (an
/// arrow, Page Down or Space over a 600 px view).
fn scenarios() -> Vec<(String, Vec<(f64, f32)>)> {
    let mut all = vec![
        ("one notch".to_owned(), vec![(0.0, 100.0)]),
        ("one screen notch".to_owned(), vec![(0.0, 525.0)]),
        ("held Down arrow".to_owned(), held_key(1500.0, 28.0)),
        ("held Page Down".to_owned(), held_key(1500.0, 600.0)),
        ("held Page Up".to_owned(), held_key(1000.0, -600.0)),
        ("fast spin up at Fast".to_owned(), burst(12, 16.0, -150.0)),
    ];
    for spacing in [16.0, 33.0, 50.0, 100.0] {
        all.push((
            format!("10 notches every {spacing} ms"),
            burst(10, spacing, 100.0),
        ));
        all.push((
            format!("40 notches every {spacing} ms"),
            burst(40, spacing, 100.0),
        ));
    }
    // Input between frames, as it comes in practice.
    let between = |inputs: Vec<(f64, f32)>, after: f64| -> Vec<(f64, f32)> {
        inputs
            .into_iter()
            .map(|(time, pixels)| (time + after, pixels))
            .collect()
    };
    all.push((
        "10 notches every 33 ms between frames".to_owned(),
        between(burst(10, 33.0, 100.0), 5.0),
    ));
    all.push((
        "held Down arrow between frames".to_owned(),
        between(held_key(1500.0, 28.0), 7.0),
    ));
    all
}

/// One glide at `scale` in ms: a notch's from rest (200 ms at Default).
fn glide_ms(scale: f32) -> f64 {
    glide_duration(0.0, scale).as_secs_f64() * 1000.0
}

/// Long enough for every scenario to finish at Very long.
const UNTIL: f64 = 6000.0;

#[test]
fn every_input_moves_exactly_as_far_as_with_glide_off() {
    for (name, inputs) in scenarios() {
        let off = simulate(&inputs, 0.0, FRAME_MS, UNTIL).total();
        let expected: f64 = inputs.iter().map(|input| f64::from(input.1)).sum();
        assert!((off - expected).abs() < 1e-3, "{name}: Off moved {off}");
        for scale in SCALES {
            let total = simulate(&inputs, scale, FRAME_MS, UNTIL).total();
            assert!(
                (total - off).abs() <= 1e-6 * off.abs().max(1.0),
                "{name} at x{scale}: {total} px, Off {off} px"
            );
        }
    }
}

#[test]
fn glide_reaches_each_target_within_one_glide_and_half_of_it_well_before() {
    // Half of each input arrives within 60 % of a glide: a notch from rest takes a quarter
    // of it, a held arrow key (whose glide trails it by the distance its stop needs) the
    // longest. Chromium's ease-in-out took half a glide for a notch from rest and 83 % for
    // a held Page Down at Very long.
    for (name, inputs) in scenarios() {
        let off = simulate(&inputs, 0.0, FRAME_MS, UNTIL);
        assert!(off.longest_lag(1.0) < FRAME_MS, "{name}: Off lags");
        let last = inputs.last().unwrap().0;
        for scale in SCALES {
            let run = simulate(&inputs, scale, FRAME_MS, UNTIL);
            let glide = glide_ms(scale);
            let full = run.longest_lag(1.0);
            let half = run.longest_lag(0.5);
            assert!(
                full <= glide + FRAME_MS,
                "{name} at x{scale}: an input's target took {full:.0} ms, over one glide \
                 ({glide:.0} ms)"
            );
            assert!(
                half <= glide * 0.6 + FRAME_MS,
                "{name} at x{scale}: half an input took {half:.0} ms, over 60 % of a glide \
                 ({glide:.0} ms)"
            );
            let settled = run.settled() - last;
            assert!(
                settled <= glide + FRAME_MS,
                "{name} at x{scale}: still moving {settled:.0} ms after the last input"
            );
        }
    }
}

#[test]
fn glide_moves_one_way_and_never_jumps_further_than_off_in_a_frame_and_a_glide() {
    // A frame of the glide moves no more than Off moves in a frame and one glide: a glide
    // shorter than two frames (Very short at 30 Hz) catches up on the next frame, but
    // never further than Off would have moved meanwhile.
    for frame in [1000.0 / 30.0, FRAME_MS, 1000.0 / 144.0] {
        for (name, inputs) in scenarios() {
            let off = simulate(&inputs, 0.0, frame, UNTIL);
            for scale in SCALES {
                let window = frame + glide_ms(scale);
                let largest = off
                    .frames
                    .iter()
                    .map(|&(time, position)| (position - off.position(time - window)).abs())
                    .fold(0.0_f64, f64::max);
                let run = simulate(&inputs, scale, frame, UNTIL);
                for (index, step) in run.steps().into_iter().enumerate() {
                    assert!(
                        step * f64::from(inputs[0].1.signum()) >= 0.0,
                        "{name} at x{scale}, {frame:.1} ms frames: frame {index} moved back \
                         {step}"
                    );
                    assert!(
                        step.abs() <= largest + 1e-6,
                        "{name} at x{scale}, {frame:.1} ms frames: frame {index} jumped \
                         {step} px, Off {largest} px within {window:.0} ms"
                    );
                }
            }
        }
    }
}

#[test]
fn glide_keeps_its_timing_at_30_60_and_144_hz() {
    // The position is a function of time: a run at another frame rate is where the 60 Hz
    // run is at about the same moment. Input reaches the motion on the next frame of
    // either rate, so the comparison allows a frame of each, and a pixel or 1 % where
    // notches restart the curve between frames.
    for (name, inputs) in scenarios() {
        let sign = f64::from(inputs[0].1.signum());
        for scale in SCALES {
            let reference = simulate(&inputs, scale, FRAME_MS, UNTIL);
            let tolerance = 0.01 * reference.total().abs() + 1.0;
            for frame in [1000.0 / 30.0, 1000.0 / 144.0] {
                let run = simulate(&inputs, scale, frame, UNTIL);
                for &(time, position) in &run.frames {
                    let low = reference.position(time - FRAME_MS - frame) * sign;
                    let high = reference.position(time + FRAME_MS) * sign;
                    assert!(
                        position * sign >= low - tolerance && position * sign <= high + tolerance,
                        "{name} at x{scale}, {frame:.1} ms frames: {position:.1} px at \
                         {time:.0} ms, {low:.1} to {high:.1} px at 60 Hz"
                    );
                }
                assert!(
                    (run.total() - reference.total()).abs() < 1e-3,
                    "{name} at x{scale}"
                );
            }
        }
    }
}

#[test]
fn a_spin_or_a_held_key_trails_glide_off_by_twice_its_interval_times_the_stop() {
    // Once input repeats, the glide is where Off was `INTERVAL_RATIO` input intervals
    // (at least `MIN_GLIDE`) times the stop ago, give or take a frame: at Default a spin
    // every 33 ms trails Off by 83 ms; Chromium's glide trailed it by its whole glide.
    for (name, inputs) in scenarios() {
        if inputs.len() < 3 {
            continue;
        }
        let off = simulate(&inputs, 0.0, FRAME_MS, UNTIL);
        // The repeat interval: the last one (a held key waits 500 ms before it repeats).
        let interval = inputs[inputs.len() - 1].0 - inputs[inputs.len() - 2].0;
        // From the second repeat: the first follows a pause (a key's 500 ms delay).
        let repeating = inputs[2].0;
        for scale in &SCALES[1..] {
            let run = simulate(&inputs, *scale, FRAME_MS, UNTIL);
            let trail = (interval * INTERVAL_RATIO).max(MIN_GLIDE.as_secs_f64() * 1000.0)
                * f64::from(*scale)
                + FRAME_MS;
            let sign = f64::from(inputs[0].1.signum());
            for &(time, position) in run
                .frames
                .iter()
                .filter(|frame| frame.0 >= repeating + trail)
            {
                let behind = off.position(time - trail) * sign - position * sign;
                assert!(
                    behind <= 1e-3,
                    "{name} at x{scale}: {position:.0} px at {time:.0} ms, {behind:.0} px \
                     behind where Off was {trail:.0} ms before"
                );
            }
        }
    }
}

#[test]
fn a_new_notch_never_slows_a_glide_and_a_notch_from_rest_leaves_at_once() {
    for scale in &SCALES[1..] {
        let mut motion = WheelMotion::default();
        let mut now = Duration::ZERO;
        motion.push_scaled(0.0, 100.0, now, *scale);
        // From rest the first 60 Hz frame moves a share of the notch at once.
        now += at(FRAME_MS);
        let (_, first) = motion.tick(now).unwrap();
        let share = 100.0 * FRAME_MS / glide_ms(*scale);
        assert!(
            f64::from(first) >= share.min(100.0),
            "x{scale}: the first frame moved {first} px, under the glide's average {share}"
        );
        for _ in 0..40 {
            for _ in 0..2 {
                now += at(FRAME_MS);
                motion.tick(now);
            }
            let before = motion.speed();
            motion.push_scaled(0.0, 100.0, now, *scale);
            let after = motion.speed();
            assert!(
                after >= before * (1.0 - 1e-9),
                "x{scale}: a notch slowed the glide from {before} to {after} px/s"
            );
        }
    }
}

/// Print the distance over time of `inputs` at every glide stop (`--nocapture`).
/// The inputs arrive 5 ms after a 60 Hz frame, as input comes between frames; each
/// position is the one after the last frame at or before its time.
fn table(title: &str, inputs: &[(f64, f32)], times: &[f64]) {
    let inputs: Vec<_> = inputs
        .iter()
        .map(|&(time, pixels)| (time + 5.0, pixels))
        .collect();
    let inputs = &inputs[..];
    println!("\n{title}");
    print!("{:>8}", "glide");
    for time in times {
        print!("{:>8}", format!("{time:.0}ms"));
    }
    println!("{:>9}{:>9}{:>8}{:>8}", "total", "settled", "half", "full");
    for scale in SCALES {
        let run = simulate(inputs, scale, FRAME_MS, UNTIL);
        print!("{:>8}", format!("x{scale}"));
        for time in times {
            print!("{:>8.0}", run.position(*time));
        }
        println!(
            "{:>9.1}{:>7.0}ms{:>6.0}ms{:>6.0}ms",
            run.total(),
            run.settled(),
            run.longest_lag(0.5),
            run.longest_lag(1.0)
        );
    }
}

#[test]
#[ignore = "measurement: prints distance-over-time tables"]
fn print_distance_over_time_at_every_glide_stop() {
    table(
        "10 notches of 100 px every 33 ms",
        &burst(10, 33.0, 100.0),
        &[
            50.0, 100.0, 150.0, 200.0, 250.0, 300.0, 350.0, 400.0, 500.0, 700.0, 1000.0,
        ],
    );
    table(
        "Page Down (600 px) held for 1.5 s at Windows' repeat rate",
        &held_key(1500.0, 600.0),
        &[
            100.0, 500.0, 600.0, 750.0, 1000.0, 1250.0, 1500.0, 1600.0, 1800.0, 2500.0,
        ],
    );
    table(
        "Down arrow (28 px) held for 1.5 s at Windows' repeat rate",
        &held_key(1500.0, 28.0),
        &[
            100.0, 500.0, 750.0, 1000.0, 1250.0, 1500.0, 1600.0, 1800.0, 2500.0,
        ],
    );
    for spacing in [16.0, 50.0, 100.0] {
        table(
            &format!("20 notches of 100 px every {spacing} ms"),
            &burst(20, spacing, 100.0),
            &[250.0, 500.0, 1000.0, 1500.0, 2000.0, 2500.0],
        );
    }
}
