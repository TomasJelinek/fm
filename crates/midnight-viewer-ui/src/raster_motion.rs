//! Frame-paced wheel input for backend-rasterized text and document viewers.
use super::*;
use midnight_ui_input::WheelMotion;

#[derive(Default)]
pub(super) struct Motion {
    wheel: WheelMotion,
    unsent: Option<(f32, f32)>,
    context: Option<(u16, u64)>,
    // A bounded legacy frame can have a lower effective DPI than the native window.
    scale: Option<[f32; 2]>,
    resumed_direct: bool,
}

impl Motion {
    fn active(&self) -> bool {
        self.wheel.active() || self.unsent.is_some()
    }

    fn push(&mut self, x: f32, y: f32, now: Duration) -> bool {
        if !self.wheel.push(x, y, now) {
            return false;
        }
        self.resumed_direct = false;
        if let Some((old_x, old_y)) = &mut self.unsent {
            if x != 0.0 && old_x.signum() != x.signum() {
                *old_x = 0.0;
            }
            if y != 0.0 && old_y.signum() != y.signum() {
                *old_y = 0.0;
            }
        }
        if self.unsent == Some((0.0, 0.0)) {
            self.unsent = None;
        }
        true
    }

    fn next(&mut self, now: Duration) -> Option<(f32, f32)> {
        // A full backend queue pauses the clock instead of collecting a large
        // catch-up jump or dropping the final pixels of the wheel gesture.
        if self.unsent.is_none() {
            self.unsent = self.wheel.tick(now);
        }
        self.unsent
    }

    fn cancel(&mut self) {
        self.wheel.cancel();
        self.unsent = None;
        self.scale = None;
        self.resumed_direct = false;
    }
}

pub(super) fn install(open: &Rc<OpenWindow>) {
    let weak = Rc::downgrade(open);
    open.raster_redraw.start(
        TimerMode::Repeated,
        midnight_ui_input::WHEEL_FRAME,
        move || {
            let Some(open) = weak.upgrade() else { return };
            if open.closed.get()
                || !(raster_rebase::needs_retry(&open)
                    || open.raster_motion.borrow().active()
                    || open.retained_raster.borrow().camera.active())
            {
                open.raster_redraw.stop();
                return;
            }
            if open.image_native.borrow().active {
                open.window.window().request_redraw();
            } else {
                step(&open);
            }
        },
    );
    open.raster_redraw.stop();
}

pub(super) fn push(open: &OpenWindow, x: f32, y: f32) -> bool {
    if let Some(queued) = raster_rebase::push(open, x, y, false) {
        return queued;
    }
    if open.window.get_retained_raster() {
        return raster_presentation::push(open, x, y, false);
    }
    if open.window.get_tiled()
        || open.window.get_web_mode()
        || !open.text_position.borrow().context_available()
    {
        return false;
    }
    if !open
        .raster_motion
        .borrow_mut()
        .push(x, y, image_motion::now(open))
    {
        return false;
    }
    open.text_position.borrow_mut().cancel();
    if !open.raster_redraw.running() {
        open.raster_redraw.restart();
    }
    open.window.window().request_redraw();
    true
}

pub(super) fn step(open: &OpenWindow) {
    if raster_rebase::waiting(open) {
        raster_rebase::retry(open);
        return;
    }
    if open.window.get_retained_raster() {
        raster_presentation::step(open);
        return;
    }
    if open.closed.get() || open.window.get_tiled() || open.window.get_web_mode() {
        cancel(open);
        return;
    }
    let delta = open
        .raster_motion
        .borrow_mut()
        .next(image_motion::now(open));
    if let Some((x, y)) = delta {
        let scale = open
            .raster_motion
            .borrow()
            .scale
            .unwrap_or([open.window.window().scale_factor(); 2]);
        if open.enqueue(ViewerInput::Scroll {
            x: x * scale[0],
            y: y * scale[1],
        }) {
            open.raster_motion.borrow_mut().unsent = None;
        }
    }
    if !open.raster_motion.borrow().active() {
        open.raster_motion.borrow_mut().cancel();
        open.raster_redraw.stop();
    }
}

pub(super) fn resume(open: &OpenWindow, x: f32, y: f32, direct: bool, scale: [f32; 2]) {
    {
        let mut motion = open.raster_motion.borrow_mut();
        motion.cancel();
        motion.scale = Some(scale);
        motion.resumed_direct = direct;
        let now = image_motion::now(open);
        if direct {
            motion.wheel.push_direct(x, y, now);
        } else {
            motion.wheel.push(x, y, now);
        }
    }
    if !open.raster_redraw.running() {
        open.raster_redraw.restart();
    }
    if direct {
        step(open);
    }
    open.window.window().request_redraw();
}
/// Only a resumed precise tail owns this path. Preserve its unsent pixels when
/// the next native pixel delta arrives before the backend queue has room.
pub(super) fn continue_direct(open: &OpenWindow, x: f32, y: f32) -> bool {
    {
        let mut motion = open.raster_motion.borrow_mut();
        if !motion.resumed_direct || !motion.active() {
            return false;
        }
        if !x.is_finite() || !y.is_finite() {
            return true;
        }
        let remaining = motion.wheel.pending();
        let unsent = motion.unsent.take().unwrap_or((0.0, 0.0));
        let now = image_motion::now(open);
        motion.wheel.cancel();
        motion.wheel.push_direct(
            (remaining[0] + f64::from(unsent.0)) as f32,
            (remaining[1] + f64::from(unsent.1)) as f32,
            now,
        );
        // WheelMotion preserves same-direction distance and drops an opposite
        // residual per axis, matching native retained-camera reversal semantics.
        motion.wheel.push_direct(x, y, now);
    }
    step(open);
    true
}
pub(super) fn receive(open: &OpenWindow, frame: &ViewerFrame) {
    let context = (
        frame.mode,
        frame.image.as_ref().map_or(0, |info| info.surface.0),
    );
    let mut motion = open.raster_motion.borrow_mut();
    // Input callbacks already stop movement before file/mode/resize changes.
    // A wheel cancelling an outstanding scrollbar request can itself advance
    // the worker generation; its acknowledgement must not end the new gesture.
    if motion.context != Some(context) {
        motion.cancel();
        open.raster_redraw.stop();
    }
    motion.context = Some(context);
}

/// A native pixel delta already contains momentum. Preserve its pending retained
/// pixels while cancelling only the separate legacy wheel animation.
pub(super) fn cancel_legacy(open: &OpenWindow) {
    {
        let mut motion = open.raster_motion.borrow_mut();
        if !motion.resumed_direct {
            motion.cancel();
        }
    }
    wake_or_stop(open);
}

pub(super) fn cancel(open: &OpenWindow) {
    raster_rebase::cancel_motion(open);
    open.raster_motion.borrow_mut().cancel();
    open.retained_raster.borrow_mut().camera.cancel();
    // Settle on the next event-loop frame, outside any input callback borrows.
    wake_or_stop(open);
}

fn wake_or_stop(open: &OpenWindow) {
    if !open.closed.get()
        && (raster_rebase::needs_retry(open)
            || open.raster_motion.borrow().active()
            || open.retained_raster.borrow().camera.active())
    {
        if !open.raster_redraw.running() {
            open.raster_redraw.restart();
        }
        open.window.window().request_redraw();
    } else {
        open.raster_redraw.stop();
    }
}
