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
    /// The system's wheel setting, read when the window gains the focus.
    wheel: midnight_ui_input::WheelSetting,
    /// The system asks for less motion (`ViewerWindows::set_motion_preference`), read
    /// with the wheel setting.
    reduce_motion: bool,
}

pub(super) fn wheel_setting(open: &OpenWindow) -> midnight_ui_input::WheelSetting {
    open.image_native.borrow().wheel
}

/// Whether wheel notches and scroll keys move at once: the system asks for less motion.
pub(super) fn reduce_motion(open: &OpenWindow) -> bool {
    open.image_native.borrow().reduce_motion
}

/// Logical pixels of one wheel notch over the content (`notch_pixels_for`): the shared
/// distance, scaled by the system's wheel setting.
pub(super) fn notch_pixels(open: &OpenWindow, horizontal: bool) -> f32 {
    let page = if horizontal {
        open.window.get_content_width()
    } else {
        open.window.get_content_height()
    };
    midnight_ui_input::notch_pixels_for(wheel_setting(open), page)
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

/// Input and retained pixels share the current native DPI even before the
/// resize worker supplies a new frame or the next geometry timer runs.
pub(super) fn refresh_scale(open: &OpenWindow) {
    let scale = open.window.window().scale_factor();
    open.window.set_image_pixel_scale(1.0 / scale);
    open.image_camera.borrow_mut().set_native_scale(scale);
}

pub(super) fn push(open: &OpenWindow, x: f32, y: f32, direct: bool) -> bool {
    refresh_scale(open);
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

/// A zoom step on retained tiles: show them at the new `zoom` around `anchor` (physical
/// pixels in the view) at once; the new surface's tiles replace them when ready.
pub(super) fn preview_zoom(open: &OpenWindow, zoom: f64, anchor: [f32; 2]) {
    if !open.window.get_tiled() {
        return;
    }
    let Some(info) = open.image_info.borrow().clone() else {
        return;
    };
    let [scale, ..] = open.image_textures.borrow().preview();
    let factor = zoom / (info.zoom * scale);
    let pixel = f64::from(open.window.window().scale_factor());
    // The view is centred in the canvas; the anchor in the canvas's logical pixels.
    let left = [
        (f64::from(open.window.get_content_width())
            - f64::from(open.window.get_image_viewport_width()) / pixel)
            / 2.0,
        (f64::from(open.window.get_content_height())
            - f64::from(open.window.get_image_viewport_height()) / pixel)
            / 2.0,
    ];
    let anchor = std::array::from_fn(|axis| left[axis] + f64::from(anchor[axis]) / pixel);
    let preview = open
        .image_textures
        .borrow_mut()
        .preview_zoom(factor, anchor);
    apply_preview(open, preview);
}

/// Present the tile cache's zoom preview (`Textures::preview`).
pub(super) fn apply_preview(open: &OpenWindow, [scale, x, y]: [f64; 3]) {
    open.window.set_tile_preview_scale(scale as f32);
    open.window.set_tile_preview_x(x as f32);
    open.window.set_tile_preview_y(y as f32);
}

pub(super) fn seek(open: &OpenWindow, axis: usize, position: f64) {
    if open.image_camera.borrow_mut().seek(axis, position) {
        present(open);
        wake(open);
    }
}

/// Home and End: the image's start or end along the direction it scrolls in, as a
/// document's go to its first and last page. The retained camera owns the view, so
/// this moves it like the scrollbar; the backend's Seek would not move it.
pub(super) fn seek_end(open: &OpenWindow, position: f64) -> bool {
    let Some(axis) = open.image_camera.borrow().scroll_axis() else {
        return false;
    };
    seek(open, axis, position);
    true
}

pub(super) fn present(open: &OpenWindow) {
    refresh_scale(open);
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
    // Document tiles are rendered at the exact physical scale: draw them at whole raster
    // pixels, so glyph edges are not resampled between two screen pixels.
    let origin = if info.page > 0 {
        origin.map(|value| (value * info.raster_scale).round() / info.raster_scale)
    } else {
        origin
    };
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
    refresh_scale(open);
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
        // The system's wheel setting: read with the first native event and whenever the
        // window gains the focus (the user changes it in another window), never per notch.
        if !native.active || matches!(event, WindowEvent::Focused(true)) {
            native.wheel = crate::wheel_setting::wheel_setting();
            let host = open.motion_preference.borrow().clone();
            native.reduce_motion = crate::wheel_setting::reduce_motion(host.as_ref());
        }
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
    route(open, event, false)
}

/// `touchpad`: the shared tracker classified this line delta as finger motion;
/// it then takes the native pixel path instead of the eased wheel glide.
pub(super) fn route(open: &OpenWindow, event: &WindowEvent, touchpad: bool) -> bool {
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
    if touchpad || open.image_native.borrow().presses.active() {
        return false;
    }
    // The shared notch distance and the system's wheel setting (`notch_pixels`).
    let x = -x * notch_pixels(open, true);
    let y = -y * notch_pixels(open, false);
    // With less motion, a notch moves its whole distance at once.
    let direct = reduce_motion(open);
    if open.window.get_tiled() {
        push(open, x, y, direct)
    } else {
        raster_motion::push(open, x, y, direct)
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
