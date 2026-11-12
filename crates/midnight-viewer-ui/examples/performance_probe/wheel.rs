//! Native wheel gestures; camera movement is sampled after every completed redraw.
use midnight_viewer_ui::ViewerWindow;
use slint::ComponentHandle;
use std::time::{Duration, Instant};
#[cfg(target_os = "windows")]
#[path = "wheel_windows.rs"]
mod native;
#[path = "wheel_pixels.rs"]
mod pixels;
#[cfg(target_os = "windows")]
use native::send;
#[cfg(not(target_os = "windows"))]
fn send(_: &ViewerWindow, _: i16, _: &super::timeline::Trace) -> Result<(), String> {
    Err("native wheel injection is implemented for Windows only".into())
}

#[cfg(target_os = "windows")]
pub(super) fn real_paths() -> std::io::Result<Vec<std::path::PathBuf>> {
    Ok(native::real_paths())
}
#[cfg(not(target_os = "windows"))]
pub(super) fn real_paths() -> std::io::Result<Vec<std::path::PathBuf>> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Personal real-file cases are configured only for the native Windows wheel probe",
    ))
}

#[derive(Clone, Copy, Default)]
pub(super) struct Options {
    pub sustained: bool,
    pub tail: bool,
    pub require_wrapped_handoff: bool,
}
#[derive(Default)]
pub(super) struct Probe {
    prepared: Option<Instant>,
    stable: Option<(String, [f32; 2], Instant)>,
    actual_context: Option<String>,
    tail_sent: Option<Instant>,
    mode_requested: bool,
    requested_mode: Option<i32>,
    gestures: Vec<Gesture>,
    timeline: super::timeline::Trace,
    options: Options,
    spatial_proof: bool,
}
#[derive(Clone, Copy)]
struct Frame {
    time: f64,
    position: f32,
    redraw: f64,
    retained: bool,
    presentation: &'static str,
    offset_y: f32,
    extent: f32,
    thumb_top: f32,
    thumb_height: f32,
    raster_uploads: i32,
    tile_uploads: i32,
}
struct Gesture {
    sent: Instant,
    initial: f32,
    initial_uploads: i32,
    delta: i16,
    frames: Vec<Frame>,
    redraws: usize,
    pixels: Vec<Option<pixels::Stripe>>,
}
fn percentile(values: &[f64], percent: usize) -> f64 {
    if values.is_empty() {
        return f64::NAN;
    }
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    values[(values.len() * percent / 100).min(values.len() - 1)]
}
fn cpu_stripe(window: &ViewerWindow, spatial_proof: bool) -> Option<pixels::Stripe> {
    if spatial_proof && window.get_retained_raster() {
        return pixels::Stripe::capture_retained(window);
    }
    // Retained textures do not change as the camera moves. Inspecting the CPU
    // surface would falsely report stationary content; use separate GPU proof.
    (!window.get_retained_raster() && !window.get_tiled())
        .then(|| pixels::Stripe::capture(window))
        .flatten()
}
impl Frame {
    fn row(self, stage: usize, notch: usize) -> String {
        format!(
            "wheel_frame stage={stage} notch={notch} after_input_ms={:.3} displayed_position={:.9} redraw_ms={:.3} retained={} offset_y_logical={:.4} raster_uploads={} tile_uploads={} presentation={} vertical_extent={:.9} thumb_top_logical={:.5} thumb_height_logical={:.5}",
            self.time,
            self.position,
            self.redraw,
            self.retained,
            self.offset_y,
            self.raster_uploads,
            self.tile_uploads,
            self.presentation,
            self.extent,
            self.thumb_top,
            self.thumb_height
        )
    }
}
impl Probe {
    pub fn new(timeline: super::timeline::Trace, options: Options) -> Self {
        Self {
            timeline,
            options,
            spatial_proof: std::env::var_os("MIDNIGHT_VIEWER_PROBE_SPATIAL_PROOF").is_some(),
            requested_mode: std::env::var("MIDNIGHT_VIEWER_PROBE_MODE_INDEX")
                .ok()
                .and_then(|value| value.parse().ok())
                .filter(|index| *index >= 0),
            ..Default::default()
        }
    }
    pub fn rendered(&mut self, window: &ViewerWindow, redraw: Duration) {
        let Some(gesture) = self.gestures.last_mut() else {
            return;
        };
        gesture.redraws += 1;
        let position = window.get_displayed_position();
        let previous = gesture
            .frames
            .last()
            .map_or(gesture.initial, |sample| sample.position);
        if position != previous && gesture.frames.len() < 256 {
            if !self.options.sustained && gesture.pixels.len() < 64 {
                gesture.pixels.push(cpu_stripe(window, self.spatial_proof));
            }
            // Sample PositionBar's published track and position. These layout
            // coordinates use its exact thumb formula, not compositor readback.
            let track = window.get_vertical_bar_height();
            let extent = window.get_vertical_extent().clamp(0.0, 1.0);
            let thumb_height = (track * extent).max(28.0).min(track);
            gesture.frames.push(Frame {
                time: gesture.sent.elapsed().as_secs_f64() * 1000.0,
                position,
                redraw: redraw.as_secs_f64() * 1000.0,
                retained: window.get_retained_raster(),
                presentation: super::presentation(window),
                offset_y: window.get_raster_offset_y() * window.get_content_height(),
                extent,
                thumb_top: window.get_vertical_thumb_position() * (track - thumb_height),
                thumb_height,
                raster_uploads: window.get_raster_upload_count(),
                tile_uploads: window.get_tile_upload_count(),
            });
        }
    }
    pub fn trace(&self, stage: usize) -> Vec<String> {
        let mut rows = Vec::new();
        if self.spatial_proof {
            rows.push("wheel_spatial_proof timing_excluded=true source_pixel_crop=true actual_gpu_readback=false".into());
        }
        for (index, gesture) in self.gestures.iter().enumerate() {
            rows.push(format!("wheel_trace stage={stage} notch={index} delta={} redraws={} initial={:.9} positions={}", gesture.delta, gesture.redraws, gesture.initial, gesture.frames.len()));
            rows.extend(gesture.frames.iter().map(|frame| frame.row(stage, index)));
            if self.spatial_proof {
                for (frame, pair) in gesture.pixels.windows(2).enumerate() {
                    if let [Some(before), Some(after)] = pair
                        && let Some((shift, ratio)) = before.displacement(after, 140)
                    {
                        let (previous_position, previous_uploads) = frame
                            .checked_sub(1)
                            .map(|index| gesture.frames[index])
                            .map_or((gesture.initial, gesture.initial_uploads), |previous| {
                                (previous.position, previous.raster_uploads)
                            });
                        let next = gesture.frames[frame];
                        rows.push(format!("wheel_spatial stage={stage} notch={index} frame={frame} raster_shift={shift:.3} match_error_vs_unmoved={ratio:.5} uploads={previous_uploads}->{} normalized_delta={:.9}", next.raster_uploads, next.position - previous_position));
                    }
                }
            }
        }
        rows
    }
    fn report(&self, window: &ViewerWindow, stage: usize) -> Result<Vec<String>, String> {
        let mut rows = vec![format!(
            "wheel_mode stage={stage} index={} source={} markdown={}",
            window.get_mode_index(),
            window.get_source_mode(),
            window.get_markdown_mode(),
        )];
        let mut continuous = Vec::new();
        for (index, gesture) in self.gestures.iter().enumerate() {
            let count = gesture.frames.len();
            let total = gesture
                .frames
                .last()
                .map_or(0.0, |frame| (frame.position - gesture.initial).abs());
            let at_edge = (gesture.delta < 0 && gesture.initial >= 1.0)
                || (gesture.delta > 0 && gesture.initial <= 0.0);
            if count < 4 && !self.options.sustained && !at_edge {
                return Err(format!(
                    "native wheel stage={stage} notch={index} rendered only {count} content positions"
                ));
            }
            let mut gaps = Vec::new();
            let mut redraws = Vec::new();
            let mut previous = (0.0, gesture.initial);
            let mut maximum_fraction = 0.0_f32;
            for frame in &gesture.frames {
                if previous.0 > 0.0 {
                    gaps.push(frame.time - previous.0);
                }
                if total > 0.0 {
                    maximum_fraction =
                        maximum_fraction.max((frame.position - previous.1).abs() / total);
                }
                redraws.push(frame.redraw);
                previous = (frame.time, frame.position);
                continuous.push((
                    gesture
                        .sent
                        .duration_since(self.gestures[0].sent)
                        .as_secs_f64()
                        * 1000.0
                        + frame.time,
                    *frame,
                    index,
                ));
            }
            rows.push(format!("wheel stage={stage} notch={index} delta={} rendered_positions={count} clamped_at_start={at_edge} first_content_ms={:.3} motion_gap_p50_ms={:.3} motion_gap_p95_ms={:.3} motion_gap_max_ms={:.3} redraw_p95_ms={:.3} maximum_notch_fraction={maximum_fraction:.4}", gesture.delta, gesture.frames.first().map_or(f64::NAN, |frame| frame.time), percentile(&gaps,50), percentile(&gaps,95), percentile(&gaps,100), percentile(&redraws,95)));
            rows.extend(gesture.frames.iter().map(|frame| frame.row(stage, index)));
            for (frame, pair) in gesture.pixels.windows(2).enumerate() {
                if let [Some(before), Some(after)] = pair
                    && let Some((shift, ratio)) = before.displacement(
                        after,
                        (60.0 * window.window().scale_factor()).ceil() as usize + 2,
                    )
                {
                    rows.push(format!("wheel_pixels stage={stage} notch={index} frame={frame} estimated_physical_shift={shift:.3} match_error_vs_unmoved={ratio:.5}"));
                }
            }
            if maximum_fraction >= 0.65 && !self.options.sustained && !at_edge {
                return Err(format!(
                    "native wheel stage={stage} notch={index} still jumps {maximum_fraction:.3} of a notch in one displayed frame"
                ));
            }
        }
        for mode in ["retained-raster", "tiled-image", "legacy-viewport"] {
            let frames = continuous
                .iter()
                .filter(|(_, frame, _)| frame.presentation == mode)
                .count();
            rows.push(format!(
                "wheel_presentation stage={stage} mode={mode} moving_frames={frames}"
            ));
        }
        if self.options.require_wrapped_handoff {
            for mode in ["retained-raster", "legacy-viewport"] {
                if !continuous
                    .iter()
                    .any(|(_, frame, _)| frame.presentation == mode)
                {
                    return Err(format!(
                        "wrapped-tab stage={stage} did not observe moving {mode} frames"
                    ));
                }
            }
            if !self
                .gestures
                .iter()
                .any(|gesture| gesture.delta > 0 && !gesture.frames.is_empty())
            {
                return Err(format!(
                    "wrapped-tab stage={stage} did not move during reversal"
                ));
            }
        }
        let mut cached = Vec::new();
        let mut frontier = Vec::new();
        let mut clamped = Vec::new();
        for pair in continuous.windows(2) {
            let gap = pair[1].0 - pair[0].0;
            let clamped_pause = self.gestures[pair[0].2 + 1..pair[1].2 + 1]
                .iter()
                .any(|gesture| {
                    gesture.frames.is_empty()
                        && ((gesture.delta < 0 && gesture.initial >= 1.0)
                            || (gesture.delta > 0 && gesture.initial <= 0.0))
                });
            if clamped_pause {
                clamped.push(gap);
                continue;
            }
            let changed = pair[1].1.raster_uploads != pair[0].1.raster_uploads
                || pair[1].1.tile_uploads != pair[0].1.tile_uploads;
            if changed {
                frontier.push(gap);
            } else {
                cached.push(gap);
            }
        }
        // Isolated gestures deliberately have idle gaps. Only sustained runs use
        // this continuous aggregate for pacing; no frontier gaps are discarded.
        rows.push(format!("wheel_continuous stage={stage} sustained={} motion_frames={} within_upload_gaps={} within_upload_p50_ms={:.3} within_upload_p95_ms={:.3} within_upload_max_ms={:.3} upload_transition_gaps={} frontier_p50_ms={:.3} frontier_p95_ms={:.3} frontier_max_ms={:.3} clamped_idle_gaps={} clamped_idle_max_ms={:.3} min_position={:.9} max_position={:.9}",self.options.sustained,continuous.len(),cached.len(),percentile(&cached,50),percentile(&cached,95),percentile(&cached,100),frontier.len(),percentile(&frontier,50),percentile(&frontier,95),percentile(&frontier,100),clamped.len(),percentile(&clamped,100),continuous.iter().map(|(_,f,_)|f.position).fold(f32::INFINITY,f32::min),continuous.iter().map(|(_,f,_)|f.position).fold(f32::NEG_INFINITY,f32::max)));
        if self.options.sustained && continuous.len() < 12 {
            return Err(format!(
                "sustained stage={stage} produced only {} moving frames",
                continuous.len()
            ));
        }
        Ok(rows)
    }
    pub fn tick(
        &mut self,
        window: &ViewerWindow,
        image: bool,
        stage: usize,
    ) -> Result<Option<Vec<String>>, String> {
        if self.prepared.get_or_insert_with(Instant::now).elapsed() > Duration::from_secs(45) {
            return Err(format!(
                "native wheel timeout: stage={stage} status={}",
                window.get_status()
            ));
        }
        if !self.mode_requested {
            self.mode_requested = true;
            if let Some(index) = self.requested_mode {
                window.invoke_change_mode(index);
                return Ok(None);
            }
        }
        // Mode callbacks revoke old position authority immediately. A stable
        // provisional "mode:N" context is not an acknowledged content frame.
        if !mode_ready(
            self.requested_mode,
            window.get_mode_index(),
            window.get_position_context().as_str(),
        ) {
            return Ok(None);
        }
        let count = if self.options.sustained { 96 } else { 3 };
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
            let start = std::env::var("MIDNIGHT_VIEWER_PROBE_START")
                .ok()
                .and_then(|value| value.parse::<f32>().ok())
                .filter(|value| (0.0..=1.0).contains(value));
            if self.options.tail || start.is_some() {
                let Some(sent) = self.tail_sent else {
                    self.tail_sent = Some(Instant::now());
                    window.invoke_seek(start.unwrap_or(1.0));
                    self.timeline
                        .record("tail_seek", f64::from(start.unwrap_or(1.0)));
                    return Ok(None);
                };
                if sent.elapsed() < Duration::from_millis(700) || window.get_position_pending() {
                    return Ok(None);
                }
                let wanted = start.unwrap_or(1.0);
                if !seek_ready(
                    wanted,
                    window.get_displayed_position(),
                    window.get_position_pending(),
                ) {
                    return Err(format!(
                        "initial seek was not acknowledged: requested={wanted}, displayed={}, context={}, status={}",
                        window.get_displayed_position(),
                        window.get_position_context(),
                        window.get_status()
                    ));
                }
            }
        } else {
            let wait = if self.options.sustained && self.gestures.len() < count {
                std::env::var("MIDNIGHT_VIEWER_PROBE_WHEEL_MS")
                    .ok()
                    .and_then(|value| value.parse::<u64>().ok())
                    .filter(|value| (1..=500).contains(value))
                    .unwrap_or(100)
            } else {
                700
            };
            if self.gestures.last().unwrap().sent.elapsed() < Duration::from_millis(wait) {
                return Ok(None);
            }
        }
        if self.gestures.len() == count {
            return self.report(window, stage).map(Some);
        }
        let reverse = if self.options.sustained {
            self.gestures.len() >= count / 2
        } else {
            self.gestures.len() == 2
        };
        let direction = if reverse ^ self.options.tail {
            120
        } else {
            -120
        };
        self.timeline
            .record("gesture_clock", self.gestures.len() as f64);
        self.gestures.push(Gesture {
            sent: Instant::now(),
            initial: window.get_displayed_position(),
            initial_uploads: window.get_raster_upload_count(),
            delta: direction,
            frames: Vec::new(),
            redraws: 0,
            pixels: if self.options.sustained {
                Vec::new()
            } else {
                vec![cpu_stripe(window, self.spatial_proof)]
            },
        });
        self.timeline.record("initial_stripe_captured", 0.0);
        send(window, direction, &self.timeline)?;
        Ok(None)
    }
}

fn mode_ready(requested: Option<i32>, actual: i32, context: &str) -> bool {
    requested.is_none_or(|mode| mode == actual) && !context.starts_with("mode:")
}

fn seek_ready(requested: f32, displayed: f32, pending: bool) -> bool {
    !pending
        && requested.is_finite()
        && displayed.is_finite()
        && (requested - displayed).abs() <= 0.0001
}

#[cfg(test)]
mod readiness_tests {
    use super::*;

    #[test]
    fn mode_delivery_must_replace_the_revoked_position_context_before_seeking() {
        assert!(!mode_ready(Some(1), 0, "text:7"));
        assert!(!mode_ready(Some(1), 1, "mode:1"));
        assert!(mode_ready(Some(1), 1, "text:8"));
        assert!(mode_ready(Some(0), 0, "text:7"));
    }

    #[test]
    fn rejected_or_unacknowledged_initial_seek_cannot_become_a_successful_wheel_case() {
        assert!(!seek_ready(0.1, 0.0, false));
        assert!(!seek_ready(0.1, 0.1, true));
        assert!(!seek_ready(0.7, 0.0, false));
        assert!(seek_ready(0.1, 0.1, false));
        assert!(seek_ready(1.0, 1.0, false));
        assert!(!seek_ready(0.1, f32::NAN, false));
    }
}
