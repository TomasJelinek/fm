//! Observe render submission without changing the renderer's flush path.
use midnight_viewer_ui::ViewerWindow;
use slint::{
    ComponentHandle, GraphicsAPI, RenderingState,
    winit_030::{CustomApplicationHandler, EventResult, WinitWindowAccessor, winit},
};
use std::{
    cell::RefCell,
    io::Write,
    rc::Rc,
    time::{Duration, Instant},
};

pub struct Callbacks {
    pub window: slint::Weak<ViewerWindow>,
    pub before: Box<dyn FnMut()>,
    pub after: Box<dyn FnMut(winit::window::WindowId, Duration)>,
}
pub type Callback = Rc<RefCell<Option<Callbacks>>>;

const MAX_INPUTS: usize = 64;
const MAX_FRAMES: usize = 4096;
const MOVES: usize = 60;
const THUMB_TOLERANCE: f32 = 0.0001;

#[derive(Clone)]
pub(super) struct NativeFrame {
    pub serial: u64,
    pub sequence: usize,
    pub input_at: Option<Instant>,
    pub expected_position: f32,
    pub released: bool,
    pub captured_at: Instant,
    pub rendered_at: Instant,
    pub context: slint::SharedString,
    pub geometry: [f32; 4],
    pub thumb_position: f32,
    pub displayed_position: f32,
    pub position: f32,
    pub extent: f32,
    pub pending: bool,
    pub source_mode: bool,
    pub content_ready: bool,
    pub retained_raster: bool,
    pub image_mode: bool,
    pub raster_offsets: [f32; 2],
    pub tile_uploads: i32,
    pub raster_uploads: i32,
}

struct NativeInput {
    kind: &'static str,
    sequence: usize,
    position: [f32; 2],
    target: f32,
    at: Instant,
}

struct StagedFrame {
    frame: NativeFrame,
    before: bool,
}

struct NativeState {
    origin: Instant,
    error: Option<&'static str>,
    events: Vec<NativeInput>,
    frames: Vec<NativeFrame>,
    staged: Option<StagedFrame>,
    completed: Option<u64>,
    serial: u64,
    delivered: u64,
    skipped_redraws: usize,
    setup: usize,
    teardown: usize,
    sequence: usize,
    input_at: Option<Instant>,
    expected_position: f32,
    preparing: bool,
    started: bool,
    released: bool,
    focused: Option<bool>,
    positive_size: bool,
    target: Option<winit::window::WindowId>,
    redraw_attempts: usize,
}

impl Default for NativeState {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            error: None,
            events: Vec::with_capacity(MAX_INPUTS),
            frames: Vec::with_capacity(MAX_FRAMES),
            staged: None,
            completed: None,
            serial: 0,
            delivered: 0,
            skipped_redraws: 0,
            setup: 0,
            teardown: 0,
            sequence: 0,
            input_at: None,
            expected_position: 0.0,
            preparing: false,
            started: false,
            released: false,
            focused: None,
            positive_size: false,
            target: None,
            redraw_attempts: 0,
        }
    }
}

impl NativeState {
    fn fail(&mut self, error: &'static str) {
        self.error.get_or_insert(error);
    }
}

#[derive(Clone, Default)]
pub(super) struct NativeProof(Rc<RefCell<NativeState>>);

impl NativeProof {
    pub fn error(&self) -> Option<String> {
        self.0.borrow().error.map(str::to_owned)
    }

    pub fn latest_frame(&self) -> Option<NativeFrame> {
        self.0.borrow().frames.last().cloned()
    }

    pub fn input(
        &self,
        kind: &'static str,
        sequence: usize,
        position: [f32; 2],
        target: f32,
        at: Instant,
    ) {
        let mut state = self.0.borrow_mut();
        if state.events.len() == MAX_INPUTS {
            state.fail("native proof input event capacity exceeded");
            return;
        }
        let previous_at = state.events.last().map(|event| event.at);
        state.events.push(NativeInput {
            kind,
            sequence,
            position,
            target,
            at,
        });
        if !position.iter().all(|value| value.is_finite())
            || !target.is_finite()
            || !(0.0..=1.0).contains(&target)
            || previous_at.is_some_and(|previous| at < previous)
        {
            state.fail("native proof input has invalid coordinates, target or time");
            return;
        }
        match kind {
            "moved" if sequence == 0 && !state.preparing && !state.started => {
                state.preparing = true;
                state.input_at = Some(at);
                state.expected_position = target;
            }
            "pressed" if sequence == 0 && state.preparing && !state.started => {
                if state.focused != Some(false) || !state.positive_size || state.frames.is_empty() {
                    state.fail("native proof press requires a rendered inactive nonzero target");
                    return;
                }
                state.started = true;
                state.input_at = Some(at);
                state.expected_position = target;
            }
            "moved"
                if state.started
                    && !state.released
                    && sequence == state.sequence + 1
                    && sequence <= MOVES =>
            {
                state.sequence = sequence;
                state.input_at = Some(at);
                state.expected_position = target;
            }
            "released"
                if state.started
                    && !state.released
                    && sequence == MOVES
                    && state.sequence == MOVES =>
            {
                if (target - state.expected_position).abs() > THUMB_TOLERANCE {
                    state.fail("native proof release changed the final move target");
                }
                // Release is a raw event, but its final-content latency still
                // starts at the last move, not at this later release dispatch.
                state.released = true;
            }
            _ => state.fail("native proof input kind, order or sequence is invalid"),
        }
    }

    /// The renderer callback only pairs/stamps already captured metadata.
    /// No window/property access, formatting, I/O or snapshots belong here.
    pub fn notify(&self, rendering: RenderingState, api: &GraphicsAPI<'_>) {
        let mut state = self.0.borrow_mut();
        if !matches!(api, GraphicsAPI::WGPU30 { .. }) {
            state.fail("native proof requires the actual WGPU30 rendering API");
            return;
        }
        if state.error.is_some() {
            return;
        }
        match rendering {
            RenderingState::RenderingSetup => {
                if state.staged.as_ref().is_some_and(|staged| staged.before)
                    || (state.setup != 0 && state.started)
                {
                    state.fail("native proof renderer setup interrupted a frame or gesture");
                } else {
                    state.setup = state.setup.saturating_add(1);
                }
            }
            RenderingState::BeforeRendering => match state.staged.as_mut() {
                Some(staged) if !staged.before => staged.before = true,
                _ => state.fail("native proof BeforeRendering has no unique staged redraw"),
            },
            RenderingState::AfterRendering => {
                let Some(mut staged) = state.staged.take() else {
                    state.fail("native proof AfterRendering has no staged redraw");
                    return;
                };
                if !staged.before || state.completed.is_some() {
                    state.fail("native proof AfterRendering has no unique paired BeforeRendering");
                    return;
                }
                if state.frames.len() == MAX_FRAMES {
                    state.fail("native proof rendered frame capacity exceeded");
                    return;
                }
                let Some(serial) = state.serial.checked_add(1) else {
                    state.fail("native proof rendered frame serial overflow");
                    return;
                };
                staged.frame.serial = serial;
                staged.frame.rendered_at = Instant::now();
                state.frames.push(staged.frame);
                state.serial = serial;
                state.completed = Some(serial);
            }
            RenderingState::RenderingTeardown => {
                if state.staged.as_ref().is_some_and(|staged| staged.before) {
                    state.fail("native proof renderer teardown interrupted a paired frame");
                }
                state.teardown = state.teardown.saturating_add(1);
            }
            _ => state.fail("native proof received an unknown renderer callback"),
        }
    }

    fn observe(&self, event: &winit::event::WindowEvent) {
        use winit::event::WindowEvent;
        let mut state = self.0.borrow_mut();
        match event {
            WindowEvent::Focused(focused) => state.focused = Some(*focused),
            WindowEvent::Resized(size) => state.positive_size = size.width > 0 && size.height > 0,
            _ => {}
        }
        let invalid_target = match event {
            WindowEvent::Focused(true) => Some("native proof target gained physical focus"),
            WindowEvent::Occluded(true) => Some("native proof target became occluded"),
            WindowEvent::Resized(size) if size.width == 0 || size.height == 0 => {
                Some("native proof target acquired a zero native size")
            }
            _ => None,
        };
        if let Some(error) = invalid_target {
            state.fail(error);
        }
        if !state.started {
            return;
        }
        let error = match event {
            WindowEvent::Moved(_)
            | WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. } => {
                Some("native proof target geometry changed during the gesture")
            }
            WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::Touch(_)
            | WindowEvent::TouchpadPressure { .. }
            | WindowEvent::PinchGesture { .. }
            | WindowEvent::PanGesture { .. }
            | WindowEvent::RotationGesture { .. }
            | WindowEvent::DoubleTapGesture { .. } => {
                Some("native proof received physical pointer, wheel or touch input")
            }
            WindowEvent::KeyboardInput { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::Ime(_) => Some("native proof received physical keyboard input"),
            WindowEvent::Destroyed | WindowEvent::CloseRequested => {
                Some("native proof target closed during the gesture")
            }
            _ => None,
        };
        if let Some(error) = error {
            state.fail(error);
        }
    }

    fn stage(&self, window: &ViewerWindow, native: Option<&winit::window::Window>) {
        let captured_at = Instant::now();
        let surface = window.get_surface().size();
        let viewport = [window.get_content_width(), window.get_content_height()];
        let content_ready = surface.width > 0
            && surface.height > 0
            && !window.get_accessible_text().is_empty()
            && viewport
                .iter()
                .all(|value| value.is_finite() && *value > 0.0);
        let mut frame = NativeFrame {
            serial: 0,
            sequence: 0,
            input_at: None,
            expected_position: 0.0,
            released: false,
            captured_at,
            rendered_at: captured_at,
            context: window.get_position_context(),
            geometry: [
                window.get_vertical_bar_left(),
                window.get_vertical_bar_top(),
                window.get_vertical_bar_width(),
                window.get_vertical_bar_height(),
            ],
            thumb_position: window.get_vertical_thumb_position(),
            displayed_position: window.get_displayed_position(),
            position: window.get_position(),
            extent: window.get_vertical_extent(),
            pending: window.get_position_pending(),
            source_mode: window.get_source_mode(),
            content_ready,
            retained_raster: window.get_retained_raster(),
            image_mode: window.get_image_mode(),
            raster_offsets: [window.get_raster_offset_x(), window.get_raster_offset_y()],
            tile_uploads: window.get_tile_upload_count(),
            raster_uploads: window.get_raster_upload_count(),
        };
        let mut state = self.0.borrow_mut();
        state.redraw_attempts = state.redraw_attempts.saturating_add(1);
        if let Some(native) = native {
            let id = native.id();
            if state.target.is_some_and(|target| target != id) {
                state.fail("native proof target WindowId changed during observation");
            } else {
                state.target = Some(id);
            }
            let size = native.inner_size();
            state.focused = Some(native.has_focus());
            state.positive_size = size.width > 0 && size.height > 0;
        } else {
            state.fail("native proof redraw lost its exact native target");
        }
        if state.focused != Some(false) || !state.positive_size {
            state.fail("native proof redraw target is focused or has zero native size");
        }
        if state.staged.is_some() || state.completed.is_some() {
            state.fail("native proof redraw overtook an unfinished renderer pair");
            return;
        }
        frame.sequence = state.sequence;
        frame.input_at = state.input_at;
        frame.expected_position = state.expected_position;
        frame.released = state.released;
        state.staged = Some(StagedFrame {
            frame,
            before: false,
        });
    }

    fn finished(&self, started: Instant) -> Option<Duration> {
        let mut state = self.0.borrow_mut();
        if let Some(staged) = state.staged.take() {
            if staged.before {
                state.fail("native proof redraw ended before paired AfterRendering");
            }
            state.skipped_redraws = state.skipped_redraws.saturating_add(1);
            return None;
        }
        let Some(serial) = state.completed.take() else {
            state.skipped_redraws = state.skipped_redraws.saturating_add(1);
            return None;
        };
        if serial <= state.delivered {
            state.fail("native proof paired AfterRendering serial did not advance");
        }
        if state.error.is_some() {
            state.skipped_redraws = state.skipped_redraws.saturating_add(1);
            return None;
        }
        let elapsed = state.frames.last().and_then(|frame| {
            (frame.serial == serial)
                .then(|| frame.rendered_at.checked_duration_since(started))
                .flatten()
        });
        if elapsed.is_none() {
            state.fail("native proof completed redraw has an invalid serial or timestamp");
            state.skipped_redraws = state.skipped_redraws.saturating_add(1);
            return None;
        }
        state.delivered = serial;
        elapsed
    }

    pub fn rows(&self) -> Vec<String> {
        let state = self.0.borrow();
        let millis =
            |at: Instant| at.saturating_duration_since(state.origin).as_secs_f64() * 1000.0;
        let mut rows = Vec::with_capacity(state.events.len() + state.frames.len() + MOVES + 2);
        rows.push(format!(
            "inactive_native_proof required_api=WGPU30 target={:?} redraw_attempts={} events={} event_limit={MAX_INPUTS} after_frames={} frame_limit={MAX_FRAMES} skipped_redraws={} setup={} teardown={} error={:?} display_presentation_verified=false occlusion_verified=false",
            state.target, state.redraw_attempts, state.events.len(), state.frames.len(), state.skipped_redraws, state.setup, state.teardown, state.error
        ));
        for event in &state.events {
            rows.push(format!(
                "inactive_native_input kind={} sequence={} at_ms={:.6} pointer={:?} target={:.9}",
                event.kind,
                event.sequence,
                millis(event.at),
                event.position,
                event.target
            ));
        }
        for frame in &state.frames {
            rows.push(format!(
                "inactive_native_frame serial={} sequence={} input_at_ms={:?} captured_ms={:.6} after_ms={:.6} target={:.9} released={} context={:?} geometry={:?} thumb={:.9} displayed={:.9} position={:.9} extent={:.9} pending={} source={} content_ready={} retained={} image={} raster_offsets={:?} tile_uploads={} raster_uploads={}",
                frame.serial, frame.sequence, frame.input_at.map(millis), millis(frame.captured_at), millis(frame.rendered_at), frame.expected_position, frame.released, frame.context, frame.geometry, frame.thumb_position, frame.displayed_position, frame.position, frame.extent, frame.pending, frame.source_mode, frame.content_ready, frame.retained_raster, frame.image_mode, frame.raster_offsets, frame.tile_uploads, frame.raster_uploads
            ));
        }
        let mut counts = [0; 4];
        for sequence in 1..=MOVES {
            let input = state
                .events
                .iter()
                .find(|event| event.kind == "moved" && event.sequence == sequence);
            let matched = input.and_then(|input| {
                state.frames.iter().find(|frame| {
                    frame.sequence == sequence
                        && frame.input_at == Some(input.at)
                        && (frame.thumb_position - input.target).abs() <= THUMB_TOLERANCE
                        && frame.captured_at >= input.at
                        && frame.rendered_at >= frame.captured_at
                })
            });
            let (class, reason, index) = if input.is_none() {
                ("skipped", "input-not-sent", 3)
            } else if matched.is_some() {
                ("matched", "paired-after-and-thumb", 0)
            } else if !state.frames.iter().any(|frame| frame.sequence == sequence)
                && state.frames.iter().any(|frame| frame.sequence > sequence)
            {
                (
                    "coalesced",
                    "higher-sequence-rendered-without-this-sequence",
                    1,
                )
            } else {
                ("unmatched", "wrong-thumb-or-no-successor-frame", 2)
            };
            counts[index] += 1;
            let latency = matched.zip(input).map(|(frame, input)| {
                frame
                    .rendered_at
                    .saturating_duration_since(input.at)
                    .as_secs_f64()
                    * 1000.0
            });
            rows.push(format!(
                "inactive_native_move sequence={sequence} class={class} reason={reason} after_serial={:?} latency_ms={latency:?}",
                matched.map(|frame| frame.serial)
            ));
        }
        rows.push(format!(
            "inactive_native_moves total={MOVES} matched={} coalesced={} unmatched={} skipped={} released={}",
            counts[0], counts[1], counts[2], counts[3], state.released
        ));
        rows
    }
}

pub struct Observer {
    pub callback: Callback,
    pub drawing: Option<(winit::window::WindowId, Instant)>,
    pub injected_wheel: bool,
    pub native_events: Rc<RefCell<Vec<String>>>,
    pub timeline: super::timeline::Trace,
    pub native_proof: Option<NativeProof>,
}

impl Observer {
    fn finished(&mut self) {
        if let Some((window_id, started)) = self.drawing.take() {
            let elapsed = if let Some(proof) = &self.native_proof {
                let Some(elapsed) = proof.finished(started) else {
                    return;
                };
                elapsed
            } else {
                started.elapsed()
            };
            let mut callback = self.callback.borrow_mut();
            let Some(callback) = callback.as_mut() else {
                return;
            };
            self.timeline
                .record("redraw_end", elapsed.as_secs_f64() * 1000.0);
            (callback.after)(window_id, elapsed);
        }
    }
}

impl CustomApplicationHandler for Observer {
    fn window_event(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        window_id: winit::window::WindowId,
        native: Option<&winit::window::Window>,
        _: Option<&slint::Window>,
        event: &winit::event::WindowEvent,
    ) -> EventResult {
        self.finished();
        let target = self.callback.borrow().as_ref().and_then(|callback| {
            callback
                .window
                .upgrade()
                .and_then(|window| window.window().with_winit_window(|native| native.id()))
        });
        if target != Some(window_id) {
            return EventResult::Propagate;
        }
        if let Some(proof) = &self.native_proof {
            if matches!(
                event,
                winit::event::WindowEvent::CloseRequested | winit::event::WindowEvent::Destroyed
            ) {
                let _ = writeln!(
                    std::io::stderr(),
                    "probe_terminal schema=1 context=native_window_event target={window_id:?} event={event:?}"
                );
            }
            proof.observe(event);
        }
        if matches!(event, winit::event::WindowEvent::MouseWheel { .. }) {
            self.timeline.record("native_wheel", 0.0);
        }
        if self.injected_wheel
            && matches!(
                event,
                winit::event::WindowEvent::MouseWheel { .. }
                    | winit::event::WindowEvent::CursorMoved { .. }
                    | winit::event::WindowEvent::CursorLeft { .. }
                    | winit::event::WindowEvent::Focused(_)
                    | winit::event::WindowEvent::Resized(_)
            )
            && self.native_events.borrow().len() < 4096
        {
            self.native_events
                .borrow_mut()
                .push(format!("wheel_native {event:?}"));
        }
        // Synthetic native mouse messages leave the desktop cursor in place.
        // Its unrelated leave notification must not cancel this probe gesture.
        if self.native_proof.is_none()
            && self.injected_wheel
            && matches!(event, winit::event::WindowEvent::CursorLeft { .. })
        {
            return EventResult::PreventDefault;
        }
        if matches!(event, winit::event::WindowEvent::RedrawRequested) {
            if let Some(callback) = self.callback.borrow_mut().as_mut() {
                (callback.before)();
            }
            self.timeline.record("redraw_begin", 0.0);
            let started = Instant::now();
            if let Some(proof) = &self.native_proof
                && let Some(window) = self
                    .callback
                    .borrow()
                    .as_ref()
                    .and_then(|callback| callback.window.upgrade())
            {
                proof.stage(&window, native);
            }
            self.drawing = Some((window_id, started));
        }
        EventResult::Propagate
    }

    fn about_to_wait(&mut self, _: &winit::event_loop::ActiveEventLoop) -> EventResult {
        self.finished();
        EventResult::Propagate
    }
}
