//! Real native wheel gestures, measured only after the window submits its redraw.
use midnight_viewer_ui::ViewerWindow;
use slint::ComponentHandle;
use std::time::{Duration, Instant};
#[path = "wheel_pixels.rs"]
mod pixels;

#[cfg(target_os = "windows")]
#[path = "wheel_windows.rs"]
mod native;
#[cfg(target_os = "windows")]
use native::send;
#[cfg(not(target_os = "windows"))]
fn send(_: &ViewerWindow, _: i16) -> Result<(), String> {
    Err("native wheel injection is implemented for Windows only".into())
}

#[derive(Default)]
pub(super) struct Probe {
    prepared: Option<Instant>,
    stable: Option<(String, [f32; 2], Instant)>,
    actual_context: Option<String>,
    gestures: Vec<Gesture>,
}
struct Gesture {
    sent: Instant,
    initial: f32,
    frames: Vec<(f64, f32, f64)>,
    redraws: usize,
    pixels: Vec<Option<pixels::Stripe>>,
}

impl Probe {
    pub fn rendered(&mut self, window: &ViewerWindow, redraw: Duration) {
        let Some(gesture) = self.gestures.last_mut() else {
            return;
        };
        gesture.redraws += 1;
        let position = window.get_displayed_position();
        let previous = gesture
            .frames
            .last()
            .map_or(gesture.initial, |sample| sample.1);
        if position != previous {
            let completed = gesture.sent.elapsed().as_secs_f64() * 1000.0;
            gesture.pixels.push(pixels::Stripe::capture(window));
            gesture
                .frames
                .push((completed, position, redraw.as_secs_f64() * 1000.0));
        }
    }

    pub fn trace(&self, stage: usize) -> Vec<String> {
        self.gestures.iter().enumerate().flat_map(|(index, gesture)| {
            std::iter::once(format!(
                "wheel_trace stage={stage} notch={index} redraws={} initial={:.9} positions={}",
                gesture.redraws, gesture.initial, gesture.frames.len()
            )).chain(gesture.frames.iter().map(move |&(time, position, redraw)| format!(
                "wheel_frame stage={stage} notch={index} after_input_ms={time:.3} displayed_position={position:.9} redraw_ms={redraw:.3}"
            )))
        }).collect()
    }

    pub fn tick(
        &mut self,
        window: &ViewerWindow,
        image: bool,
        stage: usize,
    ) -> Result<Option<Vec<String>>, String> {
        if self.prepared.get_or_insert_with(Instant::now).elapsed() > Duration::from_secs(12) {
            return Err(format!(
                "native wheel timeout: stage={stage} status={}",
                window.get_status()
            ));
        }
        if self.gestures.is_empty() {
            let context = window.get_position_context().to_string();
            let size = [window.get_content_width(), window.get_content_height()];
            if self
                .stable
                .as_ref()
                .is_none_or(|old| old.0 != context || old.1 != size)
            {
                self.stable = Some((context, size, Instant::now()));
                return Ok(None);
            }
            if self.stable.as_ref().unwrap().2.elapsed() < Duration::from_millis(250) {
                return Ok(None);
            }
            let context = window.get_position_context().to_string();
            if image && (window.get_zoom_percent() - 100.0).abs() > 0.01 {
                if context.starts_with("image:") && self.actual_context.as_ref() != Some(&context) {
                    self.actual_context = Some(context);
                    window.invoke_image_command(1);
                }
                return Ok(None);
            }
        } else if self.gestures.last().unwrap().sent.elapsed() < Duration::from_millis(700) {
            return Ok(None);
        }
        if self.gestures.len() == 3 {
            let mut rows = Vec::new();
            for (index, gesture) in self.gestures.iter().enumerate() {
                let count = gesture.frames.len();
                if count < 4 {
                    return Err(format!(
                        "native wheel stage={stage} notch={index} rendered only {count} content positions"
                    ));
                }
                let total = (gesture.frames.last().unwrap().1 - gesture.initial).abs();
                if total == 0.0 {
                    return Err(format!(
                        "native wheel stage={stage} notch={index} did not move"
                    ));
                }
                let mut gaps = Vec::new();
                let mut redraws = Vec::new();
                let mut previous = (0.0, gesture.initial);
                let mut maximum_fraction = 0.0_f32;
                for &(time, position, redraw) in &gesture.frames {
                    if previous.0 > 0.0 {
                        gaps.push(time - previous.0);
                    }
                    maximum_fraction = maximum_fraction.max((position - previous.1).abs() / total);
                    redraws.push(redraw);
                    previous = (time, position);
                }
                gaps.sort_by(f64::total_cmp);
                redraws.sort_by(f64::total_cmp);
                rows.push(format!(
                    "wheel stage={stage} notch={index} rendered_positions={count} first_content_ms={:.3} motion_gap_p50_ms={:.3} motion_gap_p95_ms={:.3} motion_gap_max_ms={:.3} redraw_p95_ms={:.3} maximum_notch_fraction={maximum_fraction:.4}",
                    gesture.frames[0].0, gaps[gaps.len()/2], gaps[(gaps.len()*95/100).min(gaps.len()-1)],
                    gaps[gaps.len()-1], redraws[(redraws.len()*95/100).min(redraws.len()-1)],
                ));
                for &(time, position, redraw) in &gesture.frames {
                    rows.push(format!(
                        "wheel_frame stage={stage} notch={index} after_input_ms={time:.3} displayed_position={position:.9} redraw_ms={redraw:.3}"
                    ));
                }
                for (frame, pair) in gesture.pixels.windows(2).enumerate() {
                    if let [Some(before), Some(after)] = pair
                        && let Some((shift, error_ratio)) = before.displacement(
                            after,
                            (60.0 * window.window().scale_factor()).ceil() as usize + 2,
                        )
                    {
                        rows.push(format!("wheel_pixels stage={stage} notch={index} frame={frame} estimated_physical_shift={shift:.3} match_error_vs_unmoved={error_ratio:.5}"));
                    }
                }
                if maximum_fraction >= 0.65 {
                    return Err(format!(
                        "native wheel stage={stage} notch={index} still jumps {maximum_fraction:.3} of a notch in one displayed frame"
                    ));
                }
            }
            return Ok(Some(rows));
        }
        let direction = if self.gestures.len() == 2 { 120 } else { -120 };
        self.gestures.push(Gesture {
            sent: Instant::now(),
            initial: window.get_displayed_position(),
            frames: Vec::new(),
            redraws: 0,
            pixels: vec![pixels::Stripe::capture(window)],
        });
        send(window, direction)?;
        Ok(None)
    }
}
