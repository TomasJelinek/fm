//! Synthesised touchpad momentum ("kinetic scrolling"). Pure policy without native
//! input or Slint types, so every scroll surface shares one tested physics model.
//!
//! Hosts feed finger motion as logical-pixel scroll deltas (the sign convention of
//! Slint's `PointerScrolled`), report the finger lift, and on every presented frame
//! ask for the next momentum delta. Momentum decays exponentially, stops at once on
//! [`KineticScroll::cancel`], and stops per axis when the host reports that content
//! no longer moves (a content edge) or calls [`KineticScroll::stop_axis`].
use std::{collections::VecDeque, time::Duration};

/// Windows precision touchpads and macOS deliver their own momentum as further wheel
/// events after the fingers lift; synthesising more there would double it. Linux
/// X11 and Wayland deliver none, so the app supplies it.
pub(crate) const PLATFORM_MOMENTUM: bool = cfg!(any(target_os = "windows", target_os = "macos"));

/// Older samples are discarded.
const HISTORY: Duration = Duration::from_millis(100);
/// Velocity is the finger's motion over this newest part of the stroke, so a stroke
/// that slows down before the lift reports its final speed, not its average.
const VELOCITY_WINDOW: Duration = Duration::from_millis(50);
const MAX_SAMPLES: usize = 32;
/// Fewer samples are a tap or a single wheel-like jump, never a flick.
const MIN_SAMPLES: usize = 3;
/// Reports dispatched together (after a UI stall) share one timestamp; their speed
/// is unknown, and an unknown speed never flings.
const MIN_SPAN: Duration = Duration::from_millis(16);
/// A finger that rested this long before lifting has stopped (as Android assumes).
const MAX_REST: Duration = Duration::from_millis(40);
/// Unphased streams (X11) have no lift event: a gap of a few report intervals
/// ends the gesture. Bounded so slow devices neither hitch long nor fling early.
const MIN_RELEASE_GAP: Duration = Duration::from_millis(30);
const MAX_RELEASE_GAP: Duration = Duration::from_millis(60);
/// Exponential decay time constant: total travel is `velocity * TIME_CONSTANT`.
/// Longer than iOS's default deceleration (0.5 s), as hands-on feedback asked: a
/// 2000 px/s flick glides about 1600 px, 71% of it within the first second, and
/// eases out over about four seconds. Every surface shares it, so the same flick
/// glides as far in a panel, viewer or comparison.
const TIME_CONSTANT: f64 = 0.8;
/// A phased lift (Wayland) flings from this speed.
const MIN_FLING_SPEED: f64 = 250.0;
/// Without a lift event a rest looks exactly like a lift, so an unphased stream
/// coasts only after a clear flick: fast, and not slowing down at its end.
const MIN_UNPHASED_FLING_SPEED: f64 = 700.0;
/// The newest reports of an unphased flick move at least this share of the average.
const SLOWING_RATIO: f64 = 0.75;
/// 0.2 px per 60 Hz frame: the crawl dropped here (`STOP_SPEED * TIME_CONSTANT`,
/// 6 px) is invisible.
const STOP_SPEED: f64 = 12.0;
const MAX_SPEED: f64 = 6000.0;
/// A late redraw (a missed vsync, a slow debug frame) advances by its real time,
/// so the glide keeps the finger's speed down to 20 frames per second. A longer
/// stall resumes from there instead of jumping by unseen pixels.
const MAX_FRAME_TIME: Duration = Duration::from_millis(50);
/// An unphased stream's first momentum frame: one nominal frame, however long the
/// gap that ended the stream took to notice.
const FIRST_FRAME: Duration = Duration::from_millis(16);
/// Content that has not moved for this long while flinging is at an edge.
const STALL_TIMEOUT: Duration = Duration::from_millis(120);

/// Touchpad momentum for one window: tracks recent finger motion, decides whether
/// the lift was a flick, and produces per-frame momentum deltas with friction.
#[derive(Default)]
pub(crate) struct KineticScroll {
    samples: VecDeque<(Duration, [f64; 2])>,
    /// The current finger stream has no lift event; a time gap ends it.
    unphased: bool,
    fling: Option<Fling>,
}

struct Fling {
    velocity: [f64; 2],
    updated: Duration,
    /// Last observed content position per axis and when it last changed.
    seen: [Option<(f64, Duration)>; 2],
}

/// Finger speed at the end of a stroke and whether the stroke was slowing down.
struct Estimate {
    velocity: [f64; 2],
    slowing: bool,
}

fn magnitude(delta: &[f64; 2]) -> f64 {
    delta[0].hypot(delta[1])
}

impl KineticScroll {
    /// Record finger motion. A new touch always interrupts a running fling.
    /// `phased` streams end with [`Self::release`]; unphased ones end after a gap.
    pub(crate) fn track(&mut self, x: f32, y: f32, now: Duration, phased: bool) {
        self.fling = None;
        if !x.is_finite() || !y.is_finite() {
            return;
        }
        let delta = [f64::from(x), f64::from(y)];
        if self
            .samples
            .back()
            .is_some_and(|&(last, _)| now.saturating_sub(last) > MAX_REST)
        {
            self.samples.clear();
        }
        // A reversal starts a new movement; its velocity must not average with
        // the motion it replaces.
        let reversed = (0..2).any(|axis| {
            let previous: f64 = self.samples.iter().map(|(_, d)| d[axis]).sum();
            delta[axis] != 0.0 && previous != 0.0 && previous.signum() != delta[axis].signum()
        });
        if reversed {
            self.samples.clear();
        }
        while self
            .samples
            .front()
            .is_some_and(|&(time, _)| now.saturating_sub(time) > HISTORY)
            || self.samples.len() >= MAX_SAMPLES
        {
            self.samples.pop_front();
        }
        self.samples.push_back((now, delta));
        self.unphased = !phased;
    }

    /// The fingers lifted. Starts momentum when the recent motion was a flick.
    pub(crate) fn release(&mut self, now: Duration) -> bool {
        let estimate = self.estimate(now);
        self.samples.clear();
        self.unphased = false;
        match estimate {
            Some(estimate) => self.start(estimate.velocity, now, MIN_FLING_SPEED),
            None => {
                self.fling = None;
                false
            }
        }
    }

    /// Finger velocity at the end of the stroke, or `None` when it is unknown or the
    /// finger rested before `now`.
    fn estimate(&self, now: Duration) -> Option<Estimate> {
        let &(last, _) = self.samples.back()?;
        if now.saturating_sub(last) > MAX_REST {
            return None;
        }
        let recent: Vec<&(Duration, [f64; 2])> = self
            .samples
            .iter()
            .filter(|(time, _)| last.saturating_sub(*time) <= VELOCITY_WINDOW)
            .collect();
        let &&(first, _) = recent.first()?;
        let span = last.saturating_sub(first);
        if recent.len() < MIN_SAMPLES || span < MIN_SPAN {
            return None;
        }
        // The oldest sample's motion happened before the measured span began.
        let moved = &recent[1..];
        let mut velocity = [0.0; 2];
        for (_, delta) in moved {
            for (value, delta) in velocity.iter_mut().zip(delta) {
                *value += delta / span.as_secs_f64();
            }
        }
        let speed = magnitude(&velocity);
        if speed > MAX_SPEED {
            velocity = velocity.map(|value| value * MAX_SPEED / speed);
        }
        let average =
            moved.iter().map(|(_, delta)| magnitude(delta)).sum::<f64>() / moved.len() as f64;
        let newest = recent[recent.len() - 2..]
            .iter()
            .map(|(_, delta)| magnitude(delta))
            .sum::<f64>()
            / 2.0;
        Some(Estimate {
            velocity,
            slowing: newest < average * SLOWING_RATIO,
        })
    }

    /// Estimated finger velocity in logical pixels per second (zero when unknown).
    #[cfg(test)]
    pub(crate) fn velocity(&self, now: Duration) -> [f64; 2] {
        self.estimate(now)
            .map_or([0.0; 2], |estimate| estimate.velocity)
    }

    fn start(&mut self, velocity: [f64; 2], now: Duration, threshold: f64) -> bool {
        if magnitude(&velocity) < threshold {
            self.fling = None;
            return false;
        }
        self.fling = Some(Fling {
            velocity,
            updated: now,
            seen: [None; 2],
        });
        true
    }

    fn release_gap(&self) -> Duration {
        let (Some(&(first, _)), Some(&(last, _))) = (self.samples.front(), self.samples.back())
        else {
            return MAX_RELEASE_GAP;
        };
        let intervals = u32::try_from(self.samples.len().saturating_sub(1).max(1)).unwrap_or(1);
        (last.saturating_sub(first) / intervals * 3).clamp(MIN_RELEASE_GAP, MAX_RELEASE_GAP)
    }

    /// Advance to a presented frame and return this frame's momentum delta.
    pub(crate) fn tick(&mut self, now: Duration) -> Option<(f32, f32)> {
        if self.unphased
            && let Some(&(last, _)) = self.samples.back()
            && now.saturating_sub(last) >= self.release_gap()
        {
            // Continue with the last finger speed as if the fingers lifted one
            // frame ago: the gap is not caught up as a jump.
            let estimate = self.estimate(last).filter(|estimate| !estimate.slowing);
            self.samples.clear();
            self.unphased = false;
            if let Some(estimate) = estimate {
                let lift = now.saturating_sub(FIRST_FRAME).max(last);
                self.start(estimate.velocity, lift, MIN_UNPHASED_FLING_SPEED);
            }
        }
        let fling = self.fling.as_mut()?;
        let elapsed = now.saturating_sub(fling.updated).min(MAX_FRAME_TIME);
        fling.updated = now.max(fling.updated);
        let decay = (-elapsed.as_secs_f64() / TIME_CONSTANT).exp();
        let mut delta = [0.0; 2];
        for (delta, velocity) in delta.iter_mut().zip(&mut fling.velocity) {
            // Exact integral of the exponential decay over this frame, so travel
            // does not depend on the display's frame rate.
            *delta = *velocity * TIME_CONSTANT * (1.0 - decay);
            *velocity *= decay;
            if velocity.abs() < STOP_SPEED {
                *velocity = 0.0;
            }
        }
        if fling.velocity == [0.0; 2] {
            self.fling = None;
        }
        (delta != [0.0; 2]).then_some((delta[0] as f32, delta[1] as f32))
    }

    /// Report the content position each frame (any unit). An axis whose content
    /// stopped moving while momentum pushes it has reached an edge and stops.
    pub(crate) fn observe_position(&mut self, position: [Option<f64>; 2], now: Duration) {
        let Some(fling) = &mut self.fling else {
            return;
        };
        let axes = position
            .into_iter()
            .zip(&mut fling.velocity)
            .zip(&mut fling.seen);
        for ((current, velocity), seen) in axes {
            let Some(current) = current.filter(|value| value.is_finite()) else {
                continue;
            };
            if *velocity == 0.0 {
                continue;
            }
            match *seen {
                Some((old, since)) if old == current => {
                    if now.saturating_sub(since) >= STALL_TIMEOUT {
                        *velocity = 0.0;
                    }
                }
                _ => *seen = Some((current, now)),
            }
        }
        if fling.velocity == [0.0; 2] {
            self.fling = None;
        }
    }

    /// The running fling's current velocity, if any.
    pub(crate) fn fling_velocity(&self) -> Option<[f64; 2]> {
        self.fling.as_ref().map(|fling| fling.velocity)
    }

    /// A known content edge, or an axis the target cannot scroll, stops that axis
    /// without stopping the other.
    pub(crate) fn stop_axis(&mut self, axis: usize) {
        if let Some(fling) = &mut self.fling
            && let Some(velocity) = fling.velocity.get_mut(axis)
        {
            *velocity = 0.0;
            if fling.velocity == [0.0; 2] {
                self.fling = None;
            }
        }
    }

    /// Momentum is running.
    pub(crate) fn flinging(&self) -> bool {
        self.fling.is_some()
    }

    /// Momentum is running, or an unphased stream still waits for its end.
    pub(crate) fn active(&self) -> bool {
        self.fling.is_some() || (self.unphased && !self.samples.is_empty())
    }

    /// A press, key, new wheel notch or focus change stops everything at once.
    pub(crate) fn cancel(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
#[path = "kinetic_tests.rs"]
mod tests;
