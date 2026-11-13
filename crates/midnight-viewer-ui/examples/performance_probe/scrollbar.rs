//! Native pointer gestures; keep thumb responsiveness separate from content completion.
use super::render_observer::{NativeFrame, NativeProof};
use midnight_viewer_ui::ViewerWindow;
use slint::{
    ComponentHandle,
    platform::{PointerEventButton, WindowEvent},
};
use std::time::{Duration, Instant};

const NATIVE_FRAME_LIMIT: usize = 4096;
const NATIVE_MOVE_LIMIT: usize = 60;

#[derive(Default)]
pub(super) struct Probe {
    native_proof: Option<NativeProof>,
    native_frame: Option<NativeFrame>,
    native_error: Option<String>,
    native_serial: u64,
    native_frames: usize,
    native_raster_uploads: i32,
    native_extent: Option<f32>,
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
    initial_crop: Option<(u64, bool)>,
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
    content_frames: Vec<(f64, f32, i32, f64)>,
}

impl Probe {
    pub fn with_native_proof(proof: NativeProof) -> Self {
        Self {
            native_proof: Some(proof),
            preparing: Some(Instant::now()),
            ..Self::default()
        }
    }

    pub fn rendered(&mut self, window: &ViewerWindow, frames: u64, redraw: Duration) {
        if let Some(proof) = &self.native_proof {
            if let Some(frame) = proof.latest_frame() {
                self.rendered_native(frame);
            }
            return;
        }
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
            self.content_frames.push((
                self.started.unwrap().elapsed().as_secs_f64() * 1000.0,
                content,
                window.get_raster_upload_count(),
                elapsed,
            ));
            self.content_position = content;
        }
        // A delayed response to an older input is not a fast response to this one.
        // This is the PositionBar's rendered geometry, including its local drag.
        if self.sampled_input < self.sent && (position - self.expected_position).abs() < 0.0001 {
            self.latencies.push(sent.elapsed().as_secs_f64() * 1000.0);
            self.sampled_input = self.sent;
        }
        // A retained seek can move pixels without publishing a backend frame.
        // This callback observes the completed native redraw, not input dispatch.
        let tolerance = if window.get_image_mode() || window.get_retained_raster() {
            0.0001
        } else {
            0.025
        };
        let final_content = !window.get_position_pending()
            && (window.get_displayed_position() - self.expected_position).abs() < tolerance
            && window.get_raster_offset_x().is_finite()
            && window.get_raster_offset_y().is_finite();
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
        if self.native_proof.is_some() {
            return self.tick_native(window, stage);
        }
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
            self.initial_crop = (!image).then(|| visible_crop(window)).flatten();
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
                let final_crop = (!image).then(|| visible_crop(window)).flatten();
                if !image
                    && (final_crop.is_none_or(|(_, varied)| !varied)
                        || self
                            .initial_crop
                            .zip(final_crop)
                            .is_some_and(|(before, after)| before.0 == after.0))
                {
                    return Err(format!(
                        "scrollbar final visible CPU crop is blank or unchanged: stage={stage}"
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
                rows.push(format!("scrollbar_cpu_crop stage={stage} checked={} changed={} nonblank={} presentation={} raster_offset_x={} raster_offset_y={} actual_gpu_readback=false",
                    !image, self.initial_crop.zip(final_crop).is_some_and(|(before, after)| before.0 != after.0), final_crop.is_some_and(|(_, varied)| varied), super::presentation(window), window.get_raster_offset_x(), window.get_raster_offset_y()));
                let mut motion_gaps: Vec<_> = self
                    .content_frames
                    .windows(2)
                    .map(|pair| pair[1].0 - pair[0].0)
                    .collect();
                motion_gaps.sort_by(f64::total_cmp);
                if !motion_gaps.is_empty() {
                    let count = motion_gaps.len();
                    rows.push(format!("scrollbar_content_motion stage={stage} moving_frames={} gap_p50_ms={:.3} gap_p95_ms={:.3} gap_max_ms={:.3}", self.content_frames.len(), motion_gaps[count / 2], motion_gaps[(count * 95 / 100).min(count - 1)], motion_gaps[count - 1]));
                }
                rows.extend(self.content_frames.iter().map(|(time, content, uploads, redraw)| format!("scrollbar_content_frame stage={stage} elapsed_ms={time:.3} displayed_position={content:.9} raster_uploads={uploads} redraw_ms={redraw:.3}")));
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

impl Probe {
    // Everything credited to a native draw comes from the immutable metadata
    // paired with that draw's actual renderer AfterRendering marker.
    fn rendered_native(&mut self, frame: NativeFrame) {
        if self.native_error.is_some() || frame.serial == self.native_serial {
            return;
        }
        if frame.serial < self.native_serial {
            self.native_error = Some("native render serial moved backwards".into());
            return;
        }
        self.native_serial = frame.serial;
        self.native_frame = Some(frame.clone());
        let Some(started) = self.started else {
            return;
        };
        if frame.rendered_at < started {
            return;
        }
        if self.native_frames >= NATIVE_FRAME_LIMIT {
            self.native_error = Some("native scrollbar render samples exceeded 4096".into());
            return;
        }
        if let Err(error) = validate_native_frame(&frame) {
            self.native_error = Some(error);
            return;
        }
        if frame.context.as_str() != self.stable_context
            || frame.geometry != self.stable_geometry
            || Some(frame.extent) != self.native_extent
        {
            self.native_error =
                Some("native scrollbar context, geometry or extent changed during drag".into());
            return;
        }
        self.native_frames += 1;
        let scene_ms = frame
            .rendered_at
            .duration_since(frame.captured_at)
            .as_secs_f64()
            * 1000.0;
        let moved = (frame.displayed_position - self.content_position).abs() > 0.00001;
        if moved || frame.raster_uploads != self.native_raster_uploads {
            self.redraw_content.push(scene_ms);
        } else {
            self.redraw_retained.push(scene_ms);
        }
        self.native_raster_uploads = frame.raster_uploads;
        if frame.tile_uploads != self.previous_uploads {
            self.redraw_uploaded.push(scene_ms);
        }
        self.previous_uploads = frame.tile_uploads;
        if moved {
            if !frame.released {
                self.content_updates_during_drag += 1;
            }
            self.content_frames.push((
                frame.rendered_at.duration_since(started).as_secs_f64() * 1000.0,
                frame.displayed_position,
                frame.raster_uploads,
                scene_ms,
            ));
            self.content_position = frame.displayed_position;
        }
        let Some(input_at) = frame.input_at.filter(|_| frame.sequence > 0) else {
            return;
        };
        let latency_ms = frame.rendered_at.duration_since(input_at).as_secs_f64() * 1000.0;
        let thumb_matches = (frame.thumb_position - frame.expected_position).abs() < 0.0001;
        if frame.sequence > self.sampled_input && frame.sequence <= self.sent && thumb_matches {
            if self.latencies.len() >= NATIVE_MOVE_LIMIT {
                self.native_error = Some("native scrollbar thumb samples exceeded 60".into());
                return;
            }
            self.latencies.push(latency_ms);
            self.sampled_input = frame.sequence;
        }
        if frame.released && frame.sequence == NATIVE_MOVE_LIMIT {
            let tolerance = if frame.retained_raster { 0.0001 } else { 0.025 };
            let complete = !frame.pending
                && thumb_matches
                && (frame.position - frame.expected_position).abs() < 0.0001
                && (frame.displayed_position - frame.expected_position).abs() < tolerance
                && self.sent == NATIVE_MOVE_LIMIT
                && self.last_input == Some(input_at);
            self.settled = complete.then_some(latency_ms);
        }
    }

    fn tick_native(
        &mut self,
        window: &ViewerWindow,
        stage: usize,
    ) -> Result<Option<Vec<String>>, String> {
        let proof = self.native_proof.as_ref().expect("native branch").clone();
        if let Some(error) = proof.error().or_else(|| self.native_error.clone()) {
            return Err(error);
        }
        let now = Instant::now();
        if self.started.is_none() {
            if now.duration_since(*self.preparing.get_or_insert(now)) > Duration::from_secs(12) {
                return Err(format!(
                    "native scrollbar preparation timeout stage={stage}"
                ));
            }
            let Some(frame) = proof.latest_frame().filter(|frame| frame.content_ready) else {
                return Ok(None);
            };
            validate_native_frame(&frame)?;
            if !native_live_context_matches(window, &frame)
                || window.get_position() != frame.position
                || window.get_vertical_thumb_position() != frame.thumb_position
                || window.get_vertical_extent() != frame.extent
                || window.get_displayed_position() != frame.displayed_position
                || window.get_position_pending() != frame.pending
            {
                self.stable_since = None;
                return Ok(None);
            }
            if frame.context.as_str() != self.stable_context
                || frame.geometry != self.stable_geometry
            {
                self.stable_context = frame.context.to_string();
                self.stable_geometry = frame.geometry;
                self.stable_since = Some(now);
                return Ok(None);
            }
            if self
                .stable_since
                .is_none_or(|since| now.duration_since(since) < Duration::from_millis(250))
            {
                self.stable_since.get_or_insert(now);
                return Ok(None);
            }
            self.x = frame.geometry[0] + frame.geometry[2] / 2.0;
            self.top = frame.geometry[1];
            self.thumb = (frame.geometry[3] * frame.extent)
                .max(28.0)
                .min(frame.geometry[3]);
            self.travel = frame.geometry[3] - self.thumb;
            if !self.travel.is_finite() || self.travel <= 0.0 {
                return Err("native scrollbar has no positive thumb travel".into());
            }
            self.expected_position = frame.thumb_position;
            self.native_extent = Some(frame.extent);
            self.uploads_before = frame.tile_uploads;
            self.previous_uploads = frame.tile_uploads;
            self.native_raster_uploads = frame.raster_uploads;
            self.content_position = frame.displayed_position;
            self.native_frame = Some(frame);
            self.initial_crop = Some(
                visible_crop(window)
                    .filter(|(_, varied)| *varied)
                    .ok_or("native scrollbar requires a nonblank initial CPU crop")?,
            );
            let grab = self.at(self.expected_position);
            validate_native_point(window, self.stable_geometry, grab, self.expected_position)?;
            let moved_at = Instant::now();
            self.started = Some(moved_at);
            self.next_input = Some(moved_at);
            proof.input(
                "moved",
                0,
                [grab.x, grab.y],
                self.expected_position,
                moved_at,
            );
            window
                .window()
                .dispatch_event(WindowEvent::PointerMoved { position: grab });
            proof.input(
                "pressed",
                0,
                [grab.x, grab.y],
                self.expected_position,
                Instant::now(),
            );
            window.window().dispatch_event(WindowEvent::PointerPressed {
                position: grab,
                button: PointerEventButton::Left,
            });
            return Ok(None);
        }
        let started = self.started.expect("native gesture started");
        if now.duration_since(started) > Duration::from_secs(12) {
            let rendered = self.native_frame.as_ref().map(|frame| {
                (
                    frame.serial,
                    frame.sequence,
                    frame.thumb_position,
                    frame.displayed_position,
                )
            });
            return Err(format!(
                "native scrollbar timeout stage={stage} sent={} latest_render={rendered:?}",
                self.sent
            ));
        }
        let frame = proof
            .latest_frame()
            .ok_or("native scrollbar lost its render proof")?;
        validate_native_frame(&frame)?;
        if frame.context.as_str() != self.stable_context
            || frame.geometry != self.stable_geometry
            || Some(frame.extent) != self.native_extent
            || !native_live_context_matches(window, &frame)
        {
            return Err(
                "native scrollbar input context, geometry or extent changed during gesture".into(),
            );
        }
        if self.released {
            return if self.settled.is_some() {
                self.native_rows(window, stage).map(Some)
            } else {
                Ok(None)
            };
        }
        if self.next_input.is_some_and(|next| now < next) {
            return Ok(None);
        }
        if self.sent >= NATIVE_MOVE_LIMIT || self.dispatch.len() >= NATIVE_MOVE_LIMIT {
            return Err("native scrollbar pointer samples exceeded 60".into());
        }
        let segment = self.sent / 20;
        let fraction = (self.sent % 20 + 1) as f32 / 20.0;
        let (from, to) = [(0.1, 0.8), (0.8, 0.2), (0.2, 0.7)][segment];
        self.expected_position = from + (to - from) * fraction;
        let position = self.at(self.expected_position);
        validate_native_point(
            window,
            self.stable_geometry,
            position,
            self.expected_position,
        )?;
        let sequence = self.sent + 1;
        let input_at = Instant::now();
        self.last_input = Some(input_at);
        self.next_input = Some(input_at + Duration::from_millis(10));
        proof.input(
            "moved",
            sequence,
            [position.x, position.y],
            self.expected_position,
            input_at,
        );
        window
            .window()
            .dispatch_event(WindowEvent::PointerMoved { position });
        self.dispatch
            .push(input_at.elapsed().as_secs_f64() * 1000.0);
        self.sent = sequence;
        if sequence == NATIVE_MOVE_LIMIT {
            if !native_live_context_matches(window, &frame) {
                return Err(
                    "native scrollbar context, geometry or extent changed before release".into(),
                );
            }
            validate_native_point(
                window,
                self.stable_geometry,
                position,
                self.expected_position,
            )?;
            proof.input(
                "released",
                sequence,
                [position.x, position.y],
                self.expected_position,
                Instant::now(),
            );
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

    fn native_rows(&mut self, window: &ViewerWindow, stage: usize) -> Result<Vec<String>, String> {
        if self.latencies.is_empty() {
            return Err("native scrollbar produced no matched rendered thumb positions".into());
        }
        if self.content_updates_during_drag < 2 {
            return Err("native scrollbar content stalled during the rendered gesture".into());
        }
        let final_crop = visible_crop(window);
        if final_crop.is_none_or(|(_, varied)| !varied)
            || self
                .initial_crop
                .zip(final_crop)
                .is_some_and(|(before, after)| before.0 == after.0)
        {
            return Err("native scrollbar final CPU crop is blank or unchanged".into());
        }
        let frame = self
            .native_frame
            .as_ref()
            .ok_or("native final render metadata missing")?;
        self.latencies.sort_by(f64::total_cmp);
        let count = self.latencies.len();
        let mut rows = vec![format!(
            "scrollbar stage={stage} pointer_moves={} rendered_positions={count} median_ms={:.3} p95_ms={:.3} max_ms={:.3} final_content_ms={:.3} final_position={} final_displayed_position={} tile_uploads={} content_updates_during_drag={} actual_after_frames={} timing=direct_slint_input_to_native_after_rendering os_input=false gpu_completion=false",
            self.sent,
            self.latencies[count / 2],
            self.latencies[(count * 95 / 100).min(count - 1)],
            self.latencies[count - 1],
            self.settled.expect("settled native rows"),
            frame.position,
            frame.displayed_position,
            frame.tile_uploads - self.uploads_before,
            self.content_updates_during_drag,
            self.native_frames
        )];
        rows.push(format!(
            "scrollbar_cpu_crop stage={stage} checked=true changed={} nonblank={} presentation={} raster_offset_x={} raster_offset_y={} actual_gpu_readback=false cpu_only=true outside_timing=true",
            self.initial_crop.zip(final_crop).is_some_and(|(before, after)| before.0 != after.0), final_crop.is_some_and(|(_, varied)| varied), if frame.retained_raster { "retained-raster" } else { "legacy-viewport" }, frame.raster_offsets[0], frame.raster_offsets[1]
        ));
        let mut gaps: Vec<_> = self
            .content_frames
            .windows(2)
            .map(|pair| pair[1].0 - pair[0].0)
            .collect();
        gaps.sort_by(f64::total_cmp);
        if !gaps.is_empty() {
            let count = gaps.len();
            rows.push(format!("scrollbar_content_motion stage={stage} moving_frames={} gap_p50_ms={:.3} gap_p95_ms={:.3} gap_max_ms={:.3}", self.content_frames.len(), gaps[count / 2], gaps[(count * 95 / 100).min(count - 1)], gaps[count - 1]));
        }
        rows.extend(self.content_frames.iter().map(|(time, content, uploads, scene)| format!("scrollbar_content_frame stage={stage} elapsed_ms={time:.3} displayed_position={content:.9} raster_uploads={uploads} scene_render_ms={scene:.3}")));
        for (kind, samples) in [
            ("native_retained_scene", &mut self.redraw_retained),
            ("native_content_scene", &mut self.redraw_content),
            ("native_tile_upload_scene", &mut self.redraw_uploaded),
            ("pointer_record_and_dispatch", &mut self.dispatch),
        ] {
            if !samples.is_empty() {
                samples.sort_by(f64::total_cmp);
                let count = samples.len();
                rows.push(format!("profile stage={stage} kind={kind} n={count} median_ms={:.3} p95_ms={:.3} max_ms={:.3}", samples[count / 2], samples[(count * 95 / 100).min(count - 1)], samples[count - 1]));
            }
        }
        Ok(rows)
    }
}

fn validate_native_frame(frame: &NativeFrame) -> Result<(), String> {
    if !frame.source_mode || frame.image_mode || !frame.content_ready || frame.context.is_empty() {
        return Err("native scrollbar requires a ready Source frame with a fixed context".into());
    }
    if !frame.geometry.iter().all(|value| value.is_finite())
        || frame.geometry[0] < 0.0
        || frame.geometry[1] < 0.0
        || frame.geometry[2] <= 0.0
        || frame.geometry[3] <= 0.0
        || !frame.extent.is_finite()
        || frame.extent <= 0.0
        || frame.extent >= 1.0
        || ![
            frame.thumb_position,
            frame.displayed_position,
            frame.position,
            frame.expected_position,
        ]
        .into_iter()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
        || !frame.raster_offsets.iter().all(|value| value.is_finite())
    {
        return Err("native scrollbar render has invalid geometry, extent or position".into());
    }
    if frame.sequence > NATIVE_MOVE_LIMIT
        || (frame.sequence > 0 && frame.input_at.is_none())
        || frame.rendered_at < frame.captured_at
        || frame
            .input_at
            .is_some_and(|input| input > frame.captured_at)
    {
        return Err("native scrollbar render has invalid input sequence or timestamps".into());
    }
    Ok(())
}

// Live reads only admit input; they are never credited as rendered content.
fn native_live_context_matches(window: &ViewerWindow, frame: &NativeFrame) -> bool {
    window.get_source_mode()
        && !window.get_image_mode()
        && window.get_position_context() == frame.context
        && [
            window.get_vertical_bar_left(),
            window.get_vertical_bar_top(),
            window.get_vertical_bar_width(),
            window.get_vertical_bar_height(),
        ] == frame.geometry
        && window.get_vertical_extent() == frame.extent
        && window.get_vertical_extent().is_finite()
        && window.get_vertical_extent() > 0.0
        && window.get_vertical_extent() < 1.0
}

fn validate_native_point(
    window: &ViewerWindow,
    geometry: [f32; 4],
    position: slint::LogicalPosition,
    target: f32,
) -> Result<(), String> {
    let size = window.window().size();
    let scale = window.window().scale_factor();
    if !scale.is_finite()
        || scale <= 0.0
        || size.width == 0
        || size.height == 0
        || !target.is_finite()
        || !(0.0..=1.0).contains(&target)
        || !position.x.is_finite()
        || !position.y.is_finite()
        || position.x < geometry[0]
        || position.x > geometry[0] + geometry[2]
        || position.y < geometry[1]
        || position.y > geometry[1] + geometry[3]
        || position.x < 0.0
        || position.y < 0.0
        || position.x >= size.width as f32 / scale
        || position.y >= size.height as f32 / scale
    {
        return Err("native scrollbar pointer target is outside its owned logical viewport".into());
    }
    Ok(())
}

// Bounded CPU-only evidence outside gesture timing. Optional --gpu-proof runs
// separately; this crop never proves native rendering or GPU readback.
fn visible_crop(window: &ViewerWindow) -> Option<(u64, bool)> {
    let image = window.get_surface().to_rgba8_premultiplied()?;
    let [width, height] = [image.width() as usize, image.height() as usize];
    if width == 0 || height == 0 {
        return None;
    }
    let (left, top, columns, rows) = if window.get_retained_raster() {
        let [rw, rh] = [window.get_raster_width(), window.get_raster_height()];
        if rw <= 0.0 || rh <= 0.0 {
            return None;
        }
        (
            -window.get_raster_offset_x() / rw * width as f32,
            -window.get_raster_offset_y() / rh * height as f32,
            (1.0 - window.get_raster_gutter_width()) / rw * width as f32,
            height as f32 / rh,
        )
    } else {
        (0.0, 0.0, width as f32, height as f32)
    };
    if left < -1.0
        || top < -1.0
        || columns <= 0.0
        || rows <= 0.0
        || left + columns > width as f32 + 1.0
        || top + rows > height as f32 + 1.0
    {
        return None;
    }
    let mut hash = 0xcbf29ce484222325_u64;
    let mut first = None;
    let mut varied = false;
    for y in 0..64 {
        for x in 0..64 {
            let column =
                (left + (x as f32 + 0.5) * columns / 64.0).clamp(0.0, (width - 1) as f32) as usize;
            let row =
                (top + (y as f32 + 0.5) * rows / 64.0).clamp(0.0, (height - 1) as f32) as usize;
            let p = image.as_slice()[row * width + column];
            let rgba = u32::from_le_bytes([p.r, p.g, p.b, p.a]);
            varied |= first.is_some_and(|value| value != rgba);
            first.get_or_insert(rgba);
            hash = (hash ^ u64::from(rgba)).wrapping_mul(0x100000001b3);
        }
    }
    Some((hash, varied))
}
