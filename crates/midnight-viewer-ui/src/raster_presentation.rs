//! Native presentation of one bounded retained body and an independently pinned gutter.
use super::*;

#[derive(Default)]
pub(super) struct State {
    pub camera: raster_camera::Camera,
    pixels: Option<(ViewerSurfaceId, u64)>,
    positions: [f64; 2],
    uploads: u64,
    strips: Strips,
}

/// Rows count `y` from a base within this many screens of every strip they show, so an
/// `f32` row stays precise to a small fraction of a pixel; further travel rewrites them.
const STRIP_BASE_SCREENS: f64 = 32.0;

/// What strip rows' images and sizes belong to: generation, surface, viewport, strip width
/// and gutter width.
type StripOwner = (u64, ViewerSurfaceId, [u32; 2], u32, u32);

/// The shown prepared text strips. A strip keeps its image and its Slint row while it stays
/// shown, so its texture is uploaded once however far the camera moves over it: the
/// renderer uploads a row's texture again whenever that row changes, and every row's when
/// the model is replaced. Rows are therefore only appended and removed, with `y` from a
/// base that moves rarely; the window's `raster-bands-origin` places that base.
#[derive(Default)]
struct Strips {
    /// Strip IDs in the row order of the body and the gutter models.
    body_rows: Vec<u64>,
    gutter_rows: Vec<u64>,
    /// Images of the shown strips by ID (body, gutter).
    images: std::collections::HashMap<u64, (slint::Image, Option<slint::Image>)>,
    body: Rc<slint::VecModel<ViewerRasterTile>>,
    gutter: Rc<slint::VecModel<ViewerRasterTile>>,
    /// The world y that rows count from, and what their images and sizes belong to.
    base: Option<(f64, StripOwner)>,
    /// The window shows these models.
    installed: bool,
}

impl Strips {
    fn is_empty(&self) -> bool {
        self.images.is_empty() && self.body_rows.is_empty() && self.gutter_rows.is_empty()
    }

    /// Show no strips; the window keeps the (now empty) models.
    fn clear(&mut self) {
        self.body_rows.clear();
        self.gutter_rows.clear();
        self.images.clear();
        self.base = None;
        self.body.set_vec(Vec::new());
        self.gutter.set_vec(Vec::new());
    }

    /// Show `bands`, whose converted pixels are `prepared`: strips that stay keep their
    /// rows, strips that left lose theirs and new strips are appended. Returns how many
    /// strips became new images.
    fn show(
        &mut self,
        open: &OpenWindow,
        frame: &ViewerFrame,
        info: &ViewerRasterInfo,
        bands: &ViewerRasterBands,
        prepared: &[pixel_preparation::PreparedStrip],
    ) -> u64 {
        if !self.installed {
            open.window
                .set_raster_bands(slint::ModelRc::from(self.body.clone()));
            open.window
                .set_raster_gutter_bands(slint::ModelRc::from(self.gutter.clone()));
            self.installed = true;
        }
        let owner = (
            frame.generation,
            info.surface,
            info.viewport,
            bands.width,
            bands.gutter_width,
        );
        let reach = STRIP_BASE_SCREENS * f64::from(info.viewport[1]);
        let rebase = self.base.is_none_or(|(base, old)| {
            old != owner
                || bands
                    .list
                    .iter()
                    .any(|band| (band.top - base).abs() > reach)
        });
        if rebase {
            // Every row's `y` changes: rows are written again. Other content may number its
            // strips again, so images are kept only for the same content and sizes.
            if self.base.is_some_and(|(_, old)| old != owner) {
                self.images.clear();
            }
            self.body_rows.clear();
            self.gutter_rows.clear();
            self.body.set_vec(Vec::new());
            self.gutter.set_vec(Vec::new());
            let base = bands
                .list
                .first()
                .map_or(info.raster_origin[1], |band| band.top);
            self.base = Some((base, owner));
        }
        let base = self.base.map_or(0.0, |(base, _)| base);
        let shown: std::collections::HashSet<u64> = bands.list.iter().map(|band| band.id).collect();
        for (rows, model) in [
            (&mut self.body_rows, &self.body),
            (&mut self.gutter_rows, &self.gutter),
        ] {
            for index in (0..rows.len()).rev() {
                if !shown.contains(&rows[index]) {
                    rows.remove(index);
                    model.remove(index);
                }
            }
        }
        let mut images = std::mem::take(&mut self.images);
        let mut created = 0;
        let viewport = info.viewport.map(|extent| extent as f32);
        for (band, pixels) in bands.list.iter().zip(prepared) {
            let (image, gutter) = images.remove(&band.id).unwrap_or_else(|| {
                created += 1;
                (
                    slint::Image::from_rgba8_premultiplied(pixels.body.clone()),
                    pixels
                        .gutter
                        .clone()
                        .map(slint::Image::from_rgba8_premultiplied),
                )
            });
            let y = ((band.top - base) / f64::from(info.viewport[1])) as f32;
            let height = band.height as f32 / viewport[1];
            if !self.body_rows.contains(&band.id) {
                self.body_rows.push(band.id);
                self.body.push(ViewerRasterTile {
                    image: image.clone(),
                    x: 0.0,
                    y,
                    width: bands.width as f32 / viewport[0],
                    height,
                });
            }
            if let Some(gutter) = &gutter
                && !self.gutter_rows.contains(&band.id)
            {
                self.gutter_rows.push(band.id);
                self.gutter.push(ViewerRasterTile {
                    image: gutter.clone(),
                    x: 0.0,
                    y,
                    width: bands.gutter_width as f32 / viewport[0],
                    height,
                });
            }
            self.images.insert(band.id, (image, gutter));
        }
        created
    }
}

pub(super) fn receive(open: &OpenWindow, frame: &ViewerFrame) -> bool {
    // Metadata can reuse an existing pixel key without calling pixels(). Fence
    // that path too, before it changes the camera or any native property.
    if !open.palette.allows(frame) {
        return false;
    }
    let Some(info) = &frame.raster else {
        return false;
    };
    if let Some(bands) = &info.bands {
        return receive_strips(open, frame, info, bands);
    }
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
            .is_none_or(|bytes| bytes > pixel_preparation::RETAINED_BYTES)
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
    if !state.strips.is_empty() {
        state.strips.clear();
    }
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
        .map(|extent| extent.max(1.0))
    } else {
        logical_viewport(open, info.viewport)
    };
    let scale =
        std::array::from_fn(|axis| f64::from(info.viewport[axis]) / f64::from(viewport[axis]));
    if !state.camera.receive(info, scale) {
        // Pixels that do not reach the shown camera leave it asking for itself again.
        if state.camera.active() && !open.raster_redraw.running() {
            open.raster_redraw.restart();
        }
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

/// A frame of prepared text strips (`ViewerRasterBands`): the camera moves over them, and
/// only strips it has not shown before become new images.
fn receive_strips(
    open: &OpenWindow,
    frame: &ViewerFrame,
    info: &ViewerRasterInfo,
    bands: &ViewerRasterBands,
) -> bool {
    if bands.gutter_width >= info.viewport[0]
        || bands.validate(pixel_preparation::PREPARED_BYTES).is_err()
    {
        open.window
            .set_status("Invalid retained viewer surface.".into());
        return false;
    }
    let mut state = open.retained_raster.borrow_mut();
    let key = (info.surface, info.pixel_id);
    let prepared = if state.pixels != Some(key) {
        match pixel_delivery::pixels(open, frame) {
            Ok(pixels) if pixels.strips.len() == bands.list.len() => Some(pixels),
            Ok(_) => {
                open.window
                    .set_status("Prepared viewer pixels do not match the current frame.".into());
                return false;
            }
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
    // As for one body raster: the opening's viewport-only frame hands its extent over.
    let inherited_viewport = !open.window.get_retained_raster()
        && !open.window.get_image_mode()
        && frame.generation == open.frame_generation.get()
        && open.raster_size.get() == (info.viewport[0], info.viewport[1]);
    let viewport = if same_surface || inherited_viewport {
        [
            open.window.get_raster_viewport_width(),
            open.window.get_raster_viewport_height(),
        ]
        .map(|extent| extent.max(1.0))
    } else {
        logical_viewport(open, info.viewport)
    };
    let scale =
        std::array::from_fn(|axis| f64::from(info.viewport[axis]) / f64::from(viewport[axis]));
    if !state.camera.receive(info, scale) {
        if state.camera.active() && !open.raster_redraw.running() {
            open.raster_redraw.restart();
        }
        return false;
    }
    open.window.set_raster_viewport_width(viewport[0]);
    open.window.set_raster_viewport_height(viewport[1]);
    state.positions = [frame.horizontal_position, frame.position];
    if let Some(pixels) = prepared {
        let created = state.strips.show(open, frame, info, bands, &pixels.strips);
        open.window.set_surface(slint::Image::default());
        open.window.set_raster_gutter(slint::Image::default());
        state.pixels = Some(key);
        state.uploads = state.uploads.saturating_add(created);
        open.window
            .set_raster_upload_count(state.uploads.min(i32::MAX as u64) as i32);
    }
    // The page origin moves with every refill; the rows stay and only their base moves.
    if let Some((base, _)) = state.strips.base {
        open.window.set_raster_bands_origin(
            ((base - info.raster_origin[1]) / f64::from(info.viewport[1])) as f32,
        );
    }
    open.window
        .set_raster_width(frame.width as f32 / info.viewport[0] as f32);
    open.window
        .set_raster_height(frame.height as f32 / info.viewport[1] as f32);
    open.window
        .set_raster_gutter_width(bands.gutter_width as f32 / info.viewport[0] as f32);
    open.window.set_retained_raster(true);
    true
}

fn clear_strips(open: &OpenWindow) {
    open.window
        .set_raster_bands(slint::ModelRc::new(slint::VecModel::default()));
    open.window
        .set_raster_gutter_bands(slint::ModelRc::new(slint::VecModel::default()));
}

/// Logical extent onto which a new raster viewport of `pixels` is mapped. A raster
/// rendered for the current canvas at native resolution maps 1:1 to physical
/// pixels: a canvas that is not a whole number of physical pixels clips the
/// sub-pixel remainder instead of resampling every texel. A raster whose
/// resolution the backend budget reduced keeps stretching over the canvas.
pub(super) fn logical_viewport(open: &OpenWindow, pixels: [u32; 2]) -> [f32; 2] {
    let scale = open.window.window().scale_factor();
    let content = [
        open.window.get_content_width(),
        open.window.get_content_height(),
    ];
    std::array::from_fn(|axis| {
        let pixels = pixels[axis] as f32;
        if scale.is_finite() && scale > 0.0 && (pixels - content[axis] * scale).abs() <= 1.0 {
            pixels / scale
        } else {
            content[axis].max(1.0)
        }
    })
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

/// A scrollbar drag step. A partial page has its own byte/height mapping. Never
/// guess a global row or clamp an uncovered normalized target into the last
/// cached pixel. The drag's end settles the camera (see `end_seek`).
pub(super) fn seek(open: &OpenWindow, horizontal: bool, position: f64) -> bool {
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
        state.camera.seek(axis, position, baseline)
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
    clear_strips(open);
}
