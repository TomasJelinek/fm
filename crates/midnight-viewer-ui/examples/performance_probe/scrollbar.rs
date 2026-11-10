//! Native pointer gestures; keep thumb responsiveness separate from content completion.
use midnight_viewer_ui::ViewerWindow;
use slint::{
    ComponentHandle,
    platform::{PointerEventButton, WindowEvent},
};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(super) struct Probe {
    actual_context: Option<String>,
    preparing: Option<Instant>,
    started: Option<Instant>,
    next_input: Option<Instant>,
    last_input: Option<Instant>,
    released: bool,
    sent: usize,
    x: f32,
    top: f32,
    thumb: f32,
    travel: f32,
    expected_position: f32,
    sampled_input: usize,
    before_final_frame: u64,
    settled: Option<f64>,
    latencies: Vec<f64>,
    uploads_before: i32,
    content_position: f32,
    content_updates_during_drag: usize,
    stable_context: String,
    stable_geometry: [f32; 4],
    stable_since: Option<Instant>,
    previous_frame: u64,
    previous_uploads: i32,
    redraw_retained: Vec<f64>,
    redraw_content: Vec<f64>,
    redraw_uploaded: Vec<f64>,
    dispatch: Vec<f64>,
}

impl Probe {
    pub fn rendered(&mut self, window: &ViewerWindow, frames: u64, redraw: Duration) {
        let Some(sent) = self.last_input else {
            return;
        };
        let elapsed = redraw.as_secs_f64() * 1000.0;
        if frames != self.previous_frame {
            self.redraw_content.push(elapsed);
        } else {
            self.redraw_retained.push(elapsed);
        }
        self.previous_frame = frames;
        let uploads = window.get_tile_upload_count();
        if uploads != self.previous_uploads {
            self.redraw_uploaded.push(elapsed);
        }
        self.previous_uploads = uploads;
        let position = window.get_vertical_thumb_position();
        let content = window.get_displayed_position();
        if (content - self.content_position).abs() > 0.00001 {
            if !self.released {
                self.content_updates_during_drag += 1;
            }
            self.content_position = content;
        }
        // A delayed response to an older input is not a fast response to this one.
        // This is the PositionBar's rendered geometry, including its local drag.
        if self.sampled_input < self.sent && (position - self.expected_position).abs() < 0.0001 {
            self.latencies.push(sent.elapsed().as_secs_f64() * 1000.0);
            self.sampled_input = self.sent;
        }
        let final_content = if window.get_image_mode() {
            (window.get_displayed_position() - 0.7).abs() < 0.0001
        } else {
            frames > self.before_final_frame
                && !window.get_position_pending()
                && (window.get_displayed_position() - 0.7).abs() < 0.025
        };
        if self.released
            && final_content
            && window.get_position_context().as_str() == self.stable_context
        {
            self.settled
                .get_or_insert(sent.elapsed().as_secs_f64() * 1000.0);
        }
    }

    pub fn tick(
        &mut self,
        window: &ViewerWindow,
        image: bool,
        frames: u64,
        stage: usize,
    ) -> Result<Option<Vec<String>>, String> {
        if self.started.is_none() {
            if self.preparing.get_or_insert_with(Instant::now).elapsed() > Duration::from_secs(12) {
                return Err(format!(
                    "scrollbar preparation timeout stage={stage} actual_context={:?} context={} zoom={} extent={} content={}x{} status={}",
                    self.actual_context,
                    window.get_position_context(),
                    window.get_zoom_percent(),
                    window.get_vertical_extent(),
                    window.get_content_width(),
                    window.get_content_height(),
                    window.get_status()
                ));
            }
            let context = window.get_position_context().to_string();
            let geometry = [
                window.get_vertical_bar_left(),
                window.get_vertical_bar_top(),
                window.get_vertical_bar_width(),
                window.get_vertical_bar_height(),
            ];
            if context != self.stable_context || geometry != self.stable_geometry {
                self.stable_context = context;
                self.stable_geometry = geometry;
                self.stable_since = Some(Instant::now());
                return Ok(None);
            }
            // Initial native DPI/viewport negotiation intentionally cancels an old
            // pointer capture. Begin only once the displayed context has settled.
            if self
                .stable_since
                .is_none_or(|since| since.elapsed() < Duration::from_millis(250))
            {
                return Ok(None);
            }
            if image && (window.get_zoom_percent() - 100.0).abs() > 0.01 {
                if !context.starts_with("image:") || self.actual_context.as_ref() == Some(&context)
                {
                    return Ok(None);
                }
                // A newer image surface invalidates coordinates from the initial
                // toolbar/DPI negotiation. Retry only for that distinct context,
                // never repeatedly submit against a stalled or refusing surface.
                eprintln!(
                    "scrollbar prepare ActualPixels stage={stage} context={context} prior={:?} content={}x{} zoom={}",
                    self.actual_context,
                    window.get_content_width(),
                    window.get_content_height(),
                    window.get_zoom_percent()
                );
                self.actual_context = Some(context);
                window.invoke_image_command(1);
                return Ok(None);
            }
            if window.get_vertical_extent() >= 1.0 {
                return Ok(None);
            }
            self.x = window.get_vertical_bar_left() + window.get_vertical_bar_width() / 2.0;
            self.top = window.get_vertical_bar_top();
            let height = window.get_vertical_bar_height();
            self.thumb = (height * window.get_vertical_extent())
                .max(28.0)
                .min(height);
            self.travel = height - self.thumb;
            self.uploads_before = window.get_tile_upload_count();
            self.previous_uploads = self.uploads_before;
            self.previous_frame = frames;
            self.content_position = window.get_displayed_position();
            let grab = self.at(window.get_position());
            window
                .window()
                .dispatch_event(WindowEvent::PointerMoved { position: grab });
            window.window().dispatch_event(WindowEvent::PointerPressed {
                position: grab,
                button: PointerEventButton::Left,
            });
            self.started = Some(Instant::now());
            self.next_input = self.started;
            return Ok(None);
        }
        if window.get_position_context().as_str() != self.stable_context {
            return Err(format!(
                "scrollbar context changed during gesture: {:?} -> {:?}",
                self.stable_context,
                window.get_position_context()
            ));
        }
        if self.started.unwrap().elapsed() > Duration::from_secs(12) {
            return Err(format!(
                "scrollbar timeout stage={stage} sent={} position={} thumb={} displayed={} context={:?}->{:?} geometry={:?} status={}",
                self.sent,
                window.get_position(),
                window.get_vertical_thumb_position(),
                window.get_displayed_position(),
                self.stable_context,
                window.get_position_context(),
                self.stable_geometry,
                window.get_status()
            ));
        }
        if self.released {
            if let Some(content_ms) = self.settled {
                if self.latencies.is_empty() {
                    return Err("scrollbar produced no visible motion".into());
                }
                if self.content_updates_during_drag < 2 {
                    return Err(format!(
                        "scrollbar content stalled during drag: stage={stage} updates={} final_content_ms={content_ms:.3}",
                        self.content_updates_during_drag
                    ));
                }
                self.latencies.sort_by(f64::total_cmp);
                let len = self.latencies.len();
                let mut rows = vec![format!(
                    "scrollbar stage={stage} pointer_moves={} rendered_positions={len} median_ms={:.3} p95_ms={:.3} max_ms={:.3} final_content_ms={content_ms:.3} final_position={} tile_uploads={} content_updates_during_drag={}",
                    self.sent,
                    self.latencies[len / 2],
                    self.latencies[(len * 95 / 100).min(len - 1)],
                    self.latencies[len - 1],
                    window.get_position(),
                    window.get_tile_upload_count() - self.uploads_before,
                    self.content_updates_during_drag
                )];
                for (kind, samples) in [
                    ("retained_redraw", &mut self.redraw_retained),
                    ("content_redraw", &mut self.redraw_content),
                    ("tile_upload_redraw", &mut self.redraw_uploaded),
                    ("pointer_dispatch", &mut self.dispatch),
                ] {
                    if !samples.is_empty() {
                        samples.sort_by(f64::total_cmp);
                        let count = samples.len();
                        rows.push(format!(
                            "profile stage={stage} kind={kind} n={count} median_ms={:.3} p95_ms={:.3} max_ms={:.3}",
                            samples[count / 2],
                            samples[(count * 95 / 100).min(count - 1)],
                            samples[count - 1]
                        ));
                    }
                }
                return Ok(Some(rows));
            }
            return Ok(None);
        }
        let now = Instant::now();
        if self.next_input.is_some_and(|next| now < next) {
            return Ok(None);
        }
        // Three continuous sweeps include an immediate reversal in each direction.
        let segment = self.sent / 20;
        let fraction = (self.sent % 20 + 1) as f32 / 20.0;
        let (from, to) = [(0.1, 0.8), (0.8, 0.2), (0.2, 0.7)][segment];
        self.expected_position = from + (to - from) * fraction;
        let position = self.at(self.expected_position);
        self.last_input = Some(now);
        self.next_input = Some(now + Duration::from_millis(10));
        self.before_final_frame = frames;
        let dispatched = Instant::now();
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        self.dispatch
            .push(dispatched.elapsed().as_secs_f64() * 1000.0);
        self.sent += 1;
        if self.sent == 60 {
            window
                .window()
                .dispatch_event(WindowEvent::PointerReleased {
                    position,
                    button: PointerEventButton::Left,
                });
            self.released = true;
        }
        Ok(None)
    }

    fn at(&self, position: f32) -> slint::LogicalPosition {
        slint::LogicalPosition::new(self.x, self.top + position * self.travel + self.thumb / 2.0)
    }
}
