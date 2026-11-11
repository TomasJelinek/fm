//! Immediate retained-image presentation; all decoder/source work stays in the engine.
use super::*;
use i_slint_core::window::{WindowInner, WindowKind};
use slint::winit_030::winit::event::{
    ElementState, MouseScrollDelta, Touch, TouchPhase, WindowEvent,
};

#[derive(Default)]
pub(super) struct Native {
    pub(super) active: bool,
    cursor: Option<[f64; 2]>,
    control: bool,
    presses: midnight_ui_input::PressState,
}

pub(super) fn now(open: &OpenWindow) -> Duration {
    WindowInner::from_pub(open.window.window())
        .context()
        .platform()
        .duration_since_start()
}

pub(super) fn install(open: &Rc<OpenWindow>) {
    let weak = Rc::downgrade(open);
    open.image_redraw.start(
        TimerMode::Repeated,
        midnight_ui_input::WHEEL_FRAME,
        move || {
            let Some(open) = weak.upgrade() else {
                return;
            };
            if open.closed.get() || !open.image_camera.borrow().active() {
                open.image_redraw.stop();
                return;
            }
            if open.image_native.borrow().active {
                open.window.window().request_redraw();
            } else {
                step(&open);
            }
        },
    );
    open.image_redraw.stop();
}

fn wake(open: &OpenWindow) {
    if open.image_camera.borrow().active() && !open.image_redraw.running() {
        open.image_redraw.restart();
    }
    open.window.window().request_redraw();
}

pub(super) fn push(open: &OpenWindow, x: f32, y: f32, direct: bool) -> bool {
    let accepted = open.image_camera.borrow_mut().push(x, y, now(open), direct);
    if accepted {
        if direct {
            open.image_camera.borrow_mut().tick(now(open));
        }
        present(open);
        wake(open);
    }
    accepted
}

pub(super) fn seek(open: &OpenWindow, axis: usize, position: f64) {
    if open.image_camera.borrow_mut().seek(axis, position) {
        present(open);
        wake(open);
    }
}

pub(super) fn present(open: &OpenWindow) {
    let Some(info) = open.image_info.borrow().clone() else {
        return;
    };
    let (origin, vertical, horizontal, displayed) = {
        let camera = open.image_camera.borrow();
        (
            camera.origin,
            camera.position(1),
            camera.position(0),
            camera.displayed_position(1),
        )
    };
    let base = open.image_textures.borrow().base().unwrap_or(info.origin);
    open.window
        .set_tile_offset_x(((origin[0] - base[0]) / info.visible_source[0]) as f32);
    open.window
        .set_tile_offset_y(((origin[1] - base[1]) / info.visible_source[1]) as f32);
    open.window.set_position(vertical as f32);
    open.window.set_position_pending(false);
    open.window.set_displayed_position(displayed as f32);
    open.window.set_horizontal_position(horizontal as f32);
    request(open);
}

fn request(open: &OpenWindow) {
    let request = open.image_camera.borrow_mut().request();
    if let Some((surface, sequence, origin)) = request
        && !open.enqueue(ViewerInput::Image {
            frame: open.frame.get(),
            input: ViewerImageInput::PanTo {
                surface,
                sequence,
                origin,
            },
        })
    {
        open.image_camera.borrow_mut().retry();
    }
}

fn step(open: &OpenWindow) {
    if open.closed.get() || !open.window.get_tiled() {
        return;
    }
    if open.image_camera.borrow_mut().tick(now(open)) {
        present(open);
    } else {
        request(open);
    }
    if !open.image_camera.borrow().active() {
        open.image_redraw.stop();
    }
}

pub(super) fn fallback(open: &OpenWindow) {
    if !open.image_native.borrow().active {
        step(open);
    } else if open.window.get_tiled() {
        request(open);
    }
}

pub(super) fn bookkeeping(open: &OpenWindow, event: &WindowEvent) {
    {
        let mut native = open.image_native.borrow_mut();
        native.active = true;
        native.presses.observe(event);
        if matches!(
            event,
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                ..
            } | WindowEvent::MouseWheel {
                phase: TouchPhase::Started,
                ..
            } | WindowEvent::Touch(Touch {
                phase: TouchPhase::Started,
                ..
            })
        ) {
            open.image_camera.borrow_mut().cancel();
            raster_motion::cancel(open);
        }
        match event {
            WindowEvent::CursorMoved { position, .. } => {
                native.cursor = Some([position.x, position.y])
            }
            WindowEvent::CursorLeft { .. } => native.cursor = None,
            WindowEvent::ModifiersChanged(modifiers) => {
                native.control = modifiers.state().control_key()
            }

            WindowEvent::Focused(false) => {
                native.control = false;
                open.image_camera.borrow_mut().cancel();
                raster_motion::cancel(open);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
pub(super) fn observe(open: &OpenWindow, event: &WindowEvent) -> bool {
    bookkeeping(open, event);
    route(open, event)
}

pub(super) fn route(open: &OpenWindow, event: &WindowEvent) -> bool {
    if matches!(event, WindowEvent::RedrawRequested) {
        step(open);
        raster_motion::step(open);
        return false;
    }
    if matches!(
        event,
        WindowEvent::MouseWheel {
            delta: MouseScrollDelta::PixelDelta(_),
            ..
        }
    ) {
        raster_motion::cancel_legacy(open);
    }
    if matches!(event, WindowEvent::MouseWheel { .. }) && !content_target(open) {
        open.image_camera.borrow_mut().cancel();
        raster_motion::cancel(open);
        return false;
    }
    let WindowEvent::MouseWheel {
        delta: MouseScrollDelta::LineDelta(x, y),
        phase: TouchPhase::Moved,
        ..
    } = event
    else {
        return false;
    };
    if open.image_native.borrow().presses.active() {
        return false;
    }
    if open.window.get_tiled() {
        push(open, -x * 60.0, -y * 60.0, false)
    } else {
        raster_motion::push(open, -x * 60.0, -y * 60.0)
    }
}

fn content_target(open: &OpenWindow) -> bool {
    if open.window.get_web_mode() {
        return false;
    }
    let target = {
        let native = open.image_native.borrow();
        if native.control {
            return false;
        }
        native.cursor
    };
    let Some([x_pointer, y_pointer]) = target else {
        return false;
    };
    let scale = f64::from(open.window.window().scale_factor());
    let [x_pointer, y_pointer] = [x_pointer / scale, y_pointer / scale];
    let [left, top] = [
        f64::from(open.window.get_content_left()),
        f64::from(open.window.get_content_top()),
    ];
    if x_pointer < left
        || y_pointer < top
        || x_pointer >= left + f64::from(open.window.get_content_width())
        || y_pointer >= top + f64::from(open.window.get_content_height())
    {
        return false;
    }
    if WindowInner::from_pub(open.window.window())
        .active_popups()
        .iter()
        .any(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
    {
        return false;
    }
    true
}
