//! Native presentation of one bounded retained body and an independently pinned gutter.
use super::*;

#[derive(Default)]
pub(super) struct State {
    pub camera: raster_camera::Camera,
    pixels: Option<(ViewerSurfaceId, u64)>,
    positions: [f64; 2],
    uploads: u64,
}

pub(super) fn receive(open: &OpenWindow, frame: &ViewerFrame) -> bool {
    let Some(info) = &frame.raster else {
        return false;
    };
    let body_len = pixel_bytes(frame.width, frame.height);
    let gutter_len = info
        .gutter
        .as_ref()
        .map_or(Some(0), |g| pixel_bytes(g.width, g.height));
    let (Some(body_len), Some(gutter_len)) = (body_len, gutter_len) else {
        return false;
    };
    if body_len == 0
        || body_len != frame.pixels.len() as u64
        || body_len
            .checked_add(gutter_len)
            .is_none_or(|bytes| bytes > 6 * 1024 * 1024 * 4)
        || info.gutter.as_ref().is_some_and(|g| {
            g.width >= info.viewport[0]
                || g.height != frame.height
                || gutter_len != g.pixels.len() as u64
        })
    {
        open.window
            .set_status("Invalid retained viewer surface.".into());
        return false;
    }
    let mut state = open.retained_raster.borrow_mut();
    let key = (info.surface, info.pixel_id);
    let prepared = if state.pixels != Some(key) {
        match pixel_delivery::pixels(open, frame) {
            Ok(pixels) => Some(pixels),
            Err(error) => {
                open.window.set_status(error.into());
                return false;
            }
        }
    } else {
        None
    };
    let same_surface = state
        .pixels
        .is_some_and(|(surface, _)| surface == info.surface);
    let inherited_viewport = !open.window.get_retained_raster()
        && !open.window.get_image_mode()
        && frame.generation == open.frame_generation.get()
        && open.raster_size.get() == (info.viewport[0], info.viewport[1]);
    let viewport = if same_surface || inherited_viewport {
        [
            open.window.get_raster_viewport_width(),
            open.window.get_raster_viewport_height(),
        ]
    } else {
        [
            open.window.get_content_width(),
            open.window.get_content_height(),
        ]
    }
    .map(|extent| extent.max(1.0));
    let scale =
        std::array::from_fn(|axis| f64::from(info.viewport[axis]) / f64::from(viewport[axis]));
    if !state.camera.receive(info, scale) {
        return false;
    }
    // Source/Markdown controls appear with the first frame. Their smaller canvas
    // clips these pixels; it must not squeeze glyphs or change hit testing.
    // Preserve effective raster scale too when backend pixel budgets reduce it.
    open.window.set_raster_viewport_width(viewport[0]);
    open.window.set_raster_viewport_height(viewport[1]);
    state.positions = [frame.horizontal_position, frame.position];
    if let Some(pixels) = prepared {
        open.window
            .set_surface(slint::Image::from_rgba8_premultiplied(pixels.body));
        open.window.set_raster_gutter(pixels.gutter.map_or_else(
            slint::Image::default,
            slint::Image::from_rgba8_premultiplied,
        ));
        state.pixels = Some(key);
        state.uploads = state.uploads.saturating_add(1);
        open.window
            .set_raster_upload_count(state.uploads.min(i32::MAX as u64) as i32);
    }
    open.window
        .set_raster_width(frame.width as f32 / info.viewport[0] as f32);
    open.window
        .set_raster_height(frame.height as f32 / info.viewport[1] as f32);
    open.window.set_raster_gutter_width(
        info.gutter.as_ref().map_or(0, |g| g.width) as f32 / info.viewport[0] as f32,
    );
    open.window.set_retained_raster(true);
    true
}

fn pixel_bytes(width: u32, height: u32) -> Option<u64> {
    u64::from(width)
        .checked_mul(u64::from(height))?
        .checked_mul(4)
}

pub(super) fn present(open: &OpenWindow) {
    {
        let state = open.retained_raster.borrow();
        let Some(info) = state.camera.info() else {
            return;
        };
        let origin = state.camera.origin;
        open.window.set_raster_offset_x(
            ((info.raster_origin[0] - origin[0]) / f64::from(info.viewport[0])) as f32,
        );
        open.window.set_raster_offset_y(
            ((info.raster_origin[1] - origin[1]) / f64::from(info.viewport[1])) as f32,
        );
        let mut positions: [f64; 2] = std::array::from_fn(|axis| {
            (state.positions[axis]
                + (origin[axis] - info.origin[axis]) * info.scroll_per_pixel[axis])
                .clamp(0.0, 1.0)
        });
        if let Some((position, extent)) = info
            .scroll_map
            .as_ref()
            .and_then(|map| map.metrics(origin[1], f64::from(info.viewport[1])))
        {
            positions[1] = position;
            open.window.set_vertical_extent(extent as f32);
        }
        open.window.set_displayed_position(positions[1] as f32);
        if !open.text_position.borrow().has_pending() {
            let pending = state.camera.pending_seek();
            if let Some(target) = pending {
                positions = std::array::from_fn(|axis| {
                    (state.positions[axis]
                        + (target[axis] - info.origin[axis]) * info.scroll_per_pixel[axis])
                        .clamp(0.0, 1.0)
                });
                if let Some((position, _)) = info
                    .scroll_map
                    .as_ref()
                    .and_then(|map| map.metrics(target[1], f64::from(info.viewport[1])))
                {
                    positions[1] = position;
                }
            }
            open.window.set_position(positions[1] as f32);
            open.window.set_horizontal_position(positions[0] as f32);
            open.window.set_position_pending(pending.is_some());
        }
    }
    request(open);
}

fn request(open: &OpenWindow) {
    let next = open.retained_raster.borrow_mut().camera.request();
    if let Some((surface, sequence, origin, settled)) = next
        && !open.enqueue(ViewerInput::Raster {
            surface,
            input: ViewerRasterInput::PanTo {
                sequence,
                origin,
                settled,
            },
        })
    {
        open.retained_raster.borrow_mut().camera.retry();
    }
}

/// A partial page has its own byte/height mapping. Never guess a global row or
/// clamp an uncovered normalized target into the last cached pixel.
pub(super) fn seek(open: &OpenWindow, horizontal: bool, position: f64, dragging: bool) -> bool {
    if !open.window.get_retained_raster()
        || raster_rebase::waiting(open)
        || open.text_position.borrow().has_pending()
    {
        return false;
    }
    // The first absolute intent owns wheel motion. Subsequent coordinates must
    // retain an already pending other-axis seek until matching pixels arrive.
    if open
        .retained_raster
        .borrow()
        .camera
        .pending_seek()
        .is_none()
    {
        raster_motion::cancel(open);
    }
    let accepted = {
        let mut state = open.retained_raster.borrow_mut();
        let axis = usize::from(!horizontal);
        let baseline = state.positions[axis];
        state.camera.seek(axis, position, baseline, dragging)
    };
    if !accepted {
        return false;
    }
    present_seek(open);
    true
}

pub(super) fn begin_seek(open: &OpenWindow) {
    if !open.window.get_retained_raster()
        || raster_rebase::waiting(open)
        || open.text_position.borrow().has_pending()
    {
        return;
    }
    raster_motion::cancel(open);
    let started = open.retained_raster.borrow_mut().camera.begin_seek();
    if started {
        present_seek(open);
    }
}

pub(super) fn end_seek(open: &OpenWindow) {
    let ended = open.retained_raster.borrow_mut().camera.end_seek();
    if ended {
        present_seek(open);
    }
}

fn present_seek(open: &OpenWindow) {
    present(open);
    if open.retained_raster.borrow().camera.active() {
        if !open.raster_redraw.running() {
            open.raster_redraw.restart();
        }
    } else {
        open.raster_redraw.stop();
    }
    open.window.window().request_redraw();
}
pub(super) fn push(open: &OpenWindow, x: f32, y: f32, direct: bool) -> bool {
    if let Some(queued) = raster_rebase::push(open, x, y, direct) {
        return queued;
    }
    let now = image_motion::now(open);
    if !open
        .retained_raster
        .borrow_mut()
        .camera
        .push(x, y, now, direct)
    {
        return false;
    }
    open.text_position.borrow_mut().cancel();
    if direct {
        open.retained_raster.borrow_mut().camera.tick(now);
    }
    present(open);
    if !open.raster_redraw.running() {
        open.raster_redraw.restart();
    }
    open.window.window().request_redraw();
    true
}

pub(super) fn step(open: &OpenWindow) {
    if open.closed.get() {
        return;
    }
    open.retained_raster
        .borrow_mut()
        .camera
        .tick(image_motion::now(open));
    present(open);
    if !open.retained_raster.borrow().camera.active() {
        open.raster_redraw.stop();
    }
}

pub(super) fn select(open: &OpenWindow, x: f32, y: f32, extend: bool) -> Option<ViewerInput> {
    let state = open.retained_raster.borrow();
    let info = state.camera.info()?;
    Some(ViewerInput::Raster {
        surface: info.surface,
        input: ViewerRasterInput::Select {
            pixel_id: info.pixel_id,
            origin: state.camera.origin,
            x: x * info.viewport[0] as f32 / open.window.get_raster_viewport_width().max(1.0),
            y: y * info.viewport[1] as f32 / open.window.get_raster_viewport_height().max(1.0),
            extend,
        },
    })
}

pub(super) fn command(open: &OpenWindow, copy: bool) -> Option<ViewerInput> {
    let state = open.retained_raster.borrow();
    let info = state.camera.info()?;
    Some(ViewerInput::Raster {
        surface: info.surface,
        input: if copy {
            ViewerRasterInput::Copy
        } else {
            ViewerRasterInput::SelectPage {
                pixel_id: info.pixel_id,
                origin: state.camera.origin,
            }
        },
    })
}

pub(super) fn clear(open: &OpenWindow) {
    *open.retained_raster.borrow_mut() = State::default();
    open.window.set_retained_raster(false);
    open.window.set_raster_gutter(slint::Image::default());
}
