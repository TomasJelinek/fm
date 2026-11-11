//! Frame-paced wheel input for backend-rasterized text and document viewers.
use super::*;
use midnight_ui_input::WheelMotion;

#[derive(Default)]
pub(super) struct Motion {
    wheel: WheelMotion,
    unsent: Option<(f32, f32)>,
    context: Option<(u16, u64)>,
}

impl Motion {
    fn active(&self) -> bool {
        self.wheel.active() || self.unsent.is_some()
    }

    fn push(&mut self, x: f32, y: f32, now: Duration) -> bool {
        if !self.wheel.push(x, y, now) {
            return false;
        }
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
    }
}

pub(super) fn install(open: &Rc<OpenWindow>) {
    let weak = Rc::downgrade(open);
    open.raster_redraw.start(
        TimerMode::Repeated,
        midnight_ui_input::WHEEL_FRAME,
        move || {
            let Some(open) = weak.upgrade() else { return };
            if open.closed.get() || !open.raster_motion.borrow().active() {
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
    if open.closed.get() || open.window.get_tiled() || open.window.get_web_mode() {
        cancel(open);
        return;
    }
    let delta = open
        .raster_motion
        .borrow_mut()
        .next(image_motion::now(open));
    if let Some((x, y)) = delta {
        let scale = open.window.window().scale_factor();
        if open.enqueue(ViewerInput::Scroll {
            x: x * scale,
            y: y * scale,
        }) {
            open.raster_motion.borrow_mut().unsent = None;
        }
    }
    if !open.raster_motion.borrow().active() {
        open.raster_redraw.stop();
    }
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

pub(super) fn cancel(open: &OpenWindow) {
    open.raster_motion.borrow_mut().cancel();
    open.raster_redraw.stop();
}
