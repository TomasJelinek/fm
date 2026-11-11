//! Thin UI adapters. Reading, parsing, search, layout and selection run on the backend.
#![deny(unsafe_code)]

mod image_camera;
mod image_motion;
#[cfg(test)]
mod image_presentation_tests;
mod image_tiles;
mod image_zoom;
#[cfg(test)]
mod opening_retirement_tests;
#[cfg(test)]
mod opening_tests;
mod pixel_delivery;
mod pixel_preparation;
mod preferences;
mod raster_camera;
mod raster_motion;
mod raster_presentation;
#[cfg(test)]
mod raster_presentation_tests;
mod raster_rebase;
#[cfg(test)]
mod raster_rebase_tests;
#[cfg(test)]
mod raster_render_tests;
mod scrolling;
mod text_position;
#[cfg(test)]
mod text_presentation_tests;
mod web_input;
#[cfg(test)]
mod web_input_tests;
#[cfg(test)]
mod web_popup_tests;

use midnight_core::*;
use midnight_viewer_engine::{Service, WindowSession};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Timer, TimerMode};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

#[allow(unsafe_code)]
mod generated {
    slint::include_modules!();
}
pub use generated::{ViewerRasterTile, ViewerWindow};
pub use midnight_ui_input::WindowPlacement;

pub type KeyResolver = Rc<dyn Fn(&ShortcutChord, bool) -> Option<String>>;
/// Called once after initial backend content is ready, fails, or closes before readiness.
pub type OpenReady = Box<dyn FnOnce(Result<(), String>)>;
type PreferencesListener = Rc<dyn Fn(ViewerPreferenceChange)>;
type PreferencesRelay = Rc<RefCell<PreferencesListener>>;
type InputSender = Rc<dyn Fn(ViewerInput) -> bool>;

pub struct ViewerWindows {
    service: Service,
    windows: RefCell<Vec<Rc<OpenWindow>>>,
    preferences_listener: PreferencesRelay,
}

struct OpenWindow {
    window: ViewerWindow,
    session: Rc<WindowSession>,
    input: RefCell<InputSender>,
    timer: Timer,
    closed: Cell<bool>,
    ready: RefCell<Option<OpenReady>>,
    opening_succeeded: Cell<bool>,
    preparation: RefCell<Option<pixel_preparation::Preparation>>,
    prepared_pixels: RefCell<Option<pixel_preparation::PreparedPixels>>,
    last_geometry: Cell<(u32, u32, u32)>,
    frame: Cell<u64>,
    frame_generation: Cell<u64>,
    raster_size: Cell<(u32, u32)>,
    modes: RefCell<Vec<u16>>,
    preferences_listener: PreferencesRelay,
    image_info: RefCell<Option<ViewerImageInfo>>,
    font_catalog_ready: Cell<bool>,
    image_zoom: RefCell<image_zoom::Intent>,
    image_camera: RefCell<image_camera::Camera>,
    image_textures: RefCell<image_tiles::Textures>,
    image_redraw: Timer,
    raster_redraw: Timer,
    raster_motion: RefCell<raster_motion::Motion>,
    retained_raster: RefCell<raster_presentation::State>,
    raster_rebase: RefCell<raster_rebase::State>,
    image_native: RefCell<image_motion::Native>,
    text_position: RefCell<text_position::TextPosition>,
    web_native: RefCell<web_input::Native>,
    key_resolver: RefCell<KeyResolver>,
}

impl OpenWindow {
    fn opened(&self, result: Result<(), String>) {
        let ready = self.ready.borrow_mut().take();
        if let Some(ready) = ready {
            ready(result);
        }
    }

    fn acknowledge_opening(&self, result: Option<Result<(), String>>) {
        match result {
            Some(Ok(())) => self.opening_succeeded.set(true),
            Some(Err(error)) => self.opened(Err(error)),
            None => {}
        }
        if self.frame.get() != 0 && self.opening_succeeded.replace(false) {
            self.opened(Ok(()));
        }
    }

    fn enqueue(&self, input: ViewerInput) -> bool {
        let sender = self.input.borrow().clone();
        let interrupts = !matches!(
            input,
            ViewerInput::Scroll { .. }
                | ViewerInput::RasterRebase { .. }
                | ViewerInput::Raster {
                    input: ViewerRasterInput::PanTo { .. },
                    ..
                }
        );
        let changes_context = matches!(
            input,
            ViewerInput::Mode(_)
                | ViewerInput::NextFile
                | ViewerInput::Resize(_)
                | ViewerInput::Search { .. }
                | ViewerInput::Seek(_)
                | ViewerInput::Pan(_)
                | ViewerInput::Position { .. }
                | ViewerInput::Wrap(_)
        );
        let accepted = sender(input);
        if accepted && interrupts {
            raster_motion::cancel(self);
        }
        if accepted && changes_context {
            raster_rebase::cancel_context(self);
            self.retained_raster.borrow_mut().camera.suspend();
        }
        accepted
    }
}

impl ViewerWindows {
    pub fn new(service: Service) -> Self {
        Self {
            service,
            windows: RefCell::new(Vec::new()),
            preferences_listener: Rc::new(RefCell::new(Rc::new(|_| {}))),
        }
    }

    pub fn set_preferences_listener(&self, listener: Rc<dyn Fn(ViewerPreferenceChange)>) {
        *self.preferences_listener.borrow_mut() = listener;
    }

    /// Restore without reporting a user edit or writing application settings.
    pub fn restore_preferences(&self, preferences: std::sync::Arc<ViewerPreferences>) {
        for open in self
            .windows
            .borrow()
            .iter()
            .filter(|open| !open.closed.get())
        {
            let previous = open.session.preferences();
            let raster_zoom_changed = previous.image_zoom != preferences.image_zoom;
            let layout_changed = previous.wrap != preferences.wrap
                || previous.line_numbers != preferences.line_numbers
                || previous.text_font != preferences.text_font
                || previous.source_font != preferences.source_font
                || previous.markdown_font != preferences.markdown_font
                || previous.markdown_preset != preferences.markdown_preset;
            if let Err(message) = open.session.restore_preferences(preferences.clone()) {
                open.window.set_status(message.into());
            } else {
                if layout_changed {
                    raster_rebase::cancel_context(open);
                    raster_motion::cancel(open);
                    open.retained_raster.borrow_mut().camera.suspend();
                }
                if raster_zoom_changed
                    && open
                        .image_info
                        .borrow()
                        .as_ref()
                        .is_some_and(|info| info.page == 0)
                {
                    open.image_zoom.borrow_mut().clear();
                }
                preferences::present(open);
            }
        }
    }

    pub fn open(
        &self,
        launch: ViewerLaunch,
        appearance: ViewerAppearance,
        placement: impl Into<WindowPlacement>,
        keys: KeyResolver,
        copy: Rc<dyn Fn(String)>,
    ) -> Result<slint::Weak<ViewerWindow>, String> {
        self.open_with_ready(launch, appearance, placement, keys, copy, Box::new(|_| {}))
    }

    /// Window construction is synchronous; `ready` separately acknowledges initial content.
    /// Synchronous errors are returned without invoking `ready`.
    pub fn open_with_ready(
        &self,
        launch: ViewerLaunch,
        appearance: ViewerAppearance,
        placement: impl Into<WindowPlacement>,
        keys: KeyResolver,
        copy: Rc<dyn Fn(String)>,
        ready: OpenReady,
    ) -> Result<slint::Weak<ViewerWindow>, String> {
        self.windows
            .borrow_mut()
            .retain(|window| !window.closed.get());
        if self.windows.borrow().len() >= 4 {
            self.service.retire(launch);
            return Err("Close a viewer before opening another (maximum four windows).".into());
        }
        // Installation discovery can overlap native/GPU construction. Content still opens only
        // after show() below supplies the exact physical canvas and monitor scale.
        self.service.prepare_catalogs();
        let window = match ViewerWindow::new() {
            Ok(window) => window,
            Err(error) => {
                self.service.retire(launch);
                return Err(error.to_string());
            }
        };
        window.set_case_sensitive(launch.preferences.case_sensitive);
        window.set_wrap_enabled(launch.preferences.wrap);
        let placement = placement.into();
        let size = placement.logical_size();
        window.set_compact(size.width < 500.0);
        window.set_narrow_typography(size.width < 680.0);
        placement.apply(window.window());
        window.set_foreground(color(appearance.foreground));
        window.set_background_color(color(appearance.background));
        window.set_accent(color(appearance.accent));
        window.set_ui_font_size(appearance.font_size);
        window.set_ui_font_family(appearance.family.as_str().into());
        window.invoke_apply_palette(
            u16::from(appearance.background[0])
                + u16::from(appearance.background[1])
                + u16::from(appearance.background[2])
                > 384,
        );
        // Showing synchronizes the Slint layout with the native client size and
        // monitor scale. The first backend viewport uses the same physical
        // canvas measurement as subsequent resize requests.
        if let Err(error) = window.show() {
            let _ = window.hide();
            self.service.retire(launch);
            return Err(error.to_string());
        }
        let geometry = window_geometry(&window);
        let session = match self.service.open(launch, appearance, geometry) {
            Ok(session) => Rc::new(session),
            Err(error) => {
                let _ = window.hide();
                return Err(error);
            }
        };
        if !session.send(ViewerInput::RetainedRaster(true)) {
            session.close();
            let _ = window.hide();
            return Err("Viewer input is unavailable.".into());
        }
        let input = Rc::downgrade(&session);
        let open = Rc::new(OpenWindow {
            window,
            session,
            input: RefCell::new(Rc::new(move |value| {
                input.upgrade().is_some_and(|session| session.send(value))
            })),
            timer: Timer::default(),
            closed: Cell::new(false),
            ready: RefCell::new(Some(ready)),
            opening_succeeded: Cell::new(false),
            preparation: RefCell::new(None),
            prepared_pixels: RefCell::new(None),
            last_geometry: Cell::new((geometry.width, geometry.height, geometry.scale.to_bits())),
            frame: Cell::new(0),
            frame_generation: Cell::new(0),
            raster_size: Cell::new((1, 1)),
            modes: RefCell::new(Vec::new()),
            preferences_listener: self.preferences_listener.clone(),
            image_info: RefCell::new(None),
            font_catalog_ready: Cell::new(false),
            image_zoom: RefCell::new(Default::default()),
            image_camera: RefCell::new(Default::default()),
            image_textures: RefCell::new(Default::default()),
            image_redraw: Timer::default(),
            raster_redraw: Timer::default(),
            raster_motion: RefCell::new(Default::default()),
            retained_raster: RefCell::new(Default::default()),
            raster_rebase: RefCell::new(Default::default()),
            image_native: RefCell::new(Default::default()),
            text_position: RefCell::new(Default::default()),
            web_native: RefCell::new(Default::default()),
            key_resolver: RefCell::new(keys.clone()),
        });
        bind(&open);
        preferences::present(&open);
        image_motion::install(&open);
        raster_motion::install(&open);
        if let Err(error) = pixel_delivery::bind(&open, self.service.clone(), copy.clone()) {
            open.ready.borrow_mut().take();
            close(&open);
            return Err(error);
        }
        let weak = Rc::downgrade(&open);
        let service = self.service.clone();
        open.timer
            .start(TimerMode::Repeated, Duration::from_millis(16), move || {
                let Some(open) = weak.upgrade() else {
                    return;
                };
                if open.closed.get() {
                    return;
                }
                let window = &open.window;
                preferences::present_font_catalog(&open);
                image_motion::refresh_scale(&open);
                let ViewerGeometry {
                    width,
                    height,
                    scale,
                } = window_geometry(window);
                let geometry = (width, height, scale.to_bits());
                if geometry != open.last_geometry.get()
                    && open.session.send(ViewerInput::Resize(ViewerGeometry {
                        width,
                        height,
                        scale,
                    }))
                {
                    raster_rebase::cancel_context(&open);
                    raster_motion::cancel(&open);
                    open.retained_raster.borrow_mut().camera.suspend();
                    if window.get_image_mode() {
                        open.image_zoom.borrow_mut().clear();
                        open.text_position.borrow_mut().cancel();
                    } else {
                        open.text_position.borrow_mut().changing_content();
                    }
                    window.set_position_context(
                        format!("resize:{width}:{height}:{}", geometry.2).into(),
                    );
                    open.last_geometry.set(geometry);
                }
                present_output(&open, &service, copy.as_ref());
                image_motion::fallback(&open);
                flush_position(&open);
            });
        open.window.invoke_focus_content();
        let weak_window = open.window.as_weak();
        self.windows.borrow_mut().push(open);
        Ok(weak_window)
    }

    pub fn close_all(&self) {
        for open in self.windows.borrow_mut().drain(..) {
            close(&open);
        }
    }
}

fn window_geometry(window: &ViewerWindow) -> ViewerGeometry {
    let scale = window.window().scale_factor();
    let width = window.window().size().to_logical(scale).width;
    window.set_compact(width < 500.0);
    window.set_narrow_typography(width < 680.0);
    window.set_image_pixel_scale(1.0 / scale);
    ViewerGeometry {
        width: (window.get_content_width() * scale).max(1.0) as u32,
        height: (window.get_content_height() * scale).max(1.0) as u32,
        scale,
    }
}

fn accept_preference(open: &OpenWindow, change: ViewerPreferenceChange) -> bool {
    match open.session.change_preferences(&change) {
        Ok(changed) => {
            if changed {
                raster_motion::cancel(open);
                if !matches!(
                    change,
                    ViewerPreferenceChange::CaseSensitive(_)
                        | ViewerPreferenceChange::ImageZoom(_)
                        | ViewerPreferenceChange::DocumentZoom(_)
                ) {
                    open.text_position.borrow_mut().cancel();
                    raster_rebase::cancel_context(open);
                    open.retained_raster.borrow_mut().camera.suspend();
                }
                let listener = open.preferences_listener.borrow().clone();
                listener(change);
            }
            true
        }
        Err(message) => {
            open.window.set_status(message.into());
            false
        }
    }
}

fn image_preference(open: &OpenWindow, input: &ViewerImageInput) -> Option<ViewerPreferenceChange> {
    let info = open.image_info.borrow().clone()?;
    let zoom = match input {
        ViewerImageInput::ViewZoom { zoom, .. } => *zoom,
        ViewerImageInput::Fit => ViewerZoomPreference::Fit,
        ViewerImageInput::ActualPixels => ViewerZoomPreference::ActualPixels,
        ViewerImageInput::Zoom { factor, .. } => {
            let scale = open.image_zoom.borrow().scale(
                open.frame_generation.get(),
                &info,
                image_viewport(open),
            );
            ViewerZoomPreference::from_scale((scale * factor).clamp(1.0 / 32768.0, 64.0)).ok()?
        }
        ViewerImageInput::Drag { .. } | ViewerImageInput::PanTo { .. } => return None,
    };
    Some(if info.page > 0 {
        ViewerPreferenceChange::DocumentZoom(zoom)
    } else {
        ViewerPreferenceChange::ImageZoom(zoom)
    })
}

fn image_viewport(open: &OpenWindow) -> [f32; 2] {
    let scale = open.window.window().scale_factor();
    [
        open.window.get_content_width() * scale,
        open.window.get_content_height() * scale,
    ]
}

fn send_input(open: &OpenWindow, input: ViewerInput) -> bool {
    let preference = match &input {
        ViewerInput::Image { input, .. } => image_preference(open, input),
        _ => None,
    };
    let input = if let ViewerInput::Image {
        frame,
        input: gesture,
    } = &input
        && matches!(
            gesture,
            ViewerImageInput::Fit | ViewerImageInput::ActualPixels | ViewerImageInput::Zoom { .. }
        ) {
        if !open.text_position.borrow().context_available() {
            return false;
        }
        let Some(info) = open.image_info.borrow().clone() else {
            return false;
        };
        let zoom = match preference.as_ref() {
            Some(
                ViewerPreferenceChange::ImageZoom(zoom)
                | ViewerPreferenceChange::DocumentZoom(zoom),
            ) => *zoom,
            _ => return false,
        };
        let viewport = image_viewport(open);
        if !info.zoom.is_finite()
            || info.zoom <= 0.0
            || viewport
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return false;
        }
        if zoom != ViewerZoomPreference::Fit
            && (0..2).any(|axis| {
                (info.visible_source[axis] * info.zoom - f64::from(viewport[axis])).abs() > 1.0
            })
        {
            return false;
        }
        let anchor = match gesture {
            ViewerImageInput::Zoom {
                anchor: Some(anchor),
                ..
            } => *anchor,
            _ => viewport.map(|value| value / 2.0),
        };
        if anchor.iter().any(|value| !value.is_finite()) {
            return false;
        }
        let anchor = std::array::from_fn(|axis| anchor[axis].clamp(0.0, viewport[axis]));
        let origin = if open.window.get_tiled() {
            open.image_camera.borrow().origin
        } else {
            info.origin
        };
        let source_anchor =
            std::array::from_fn(|axis| origin[axis] + f64::from(anchor[axis]) / info.zoom);
        if source_anchor.iter().any(|value| !value.is_finite()) {
            return false;
        }
        ViewerInput::Image {
            frame: *frame,
            input: ViewerImageInput::ViewZoom {
                zoom,
                source_anchor,
                viewport_anchor: anchor,
            },
        }
    } else {
        input
    };
    let is_zoom = matches!(
        input,
        ViewerInput::Image {
            input: ViewerImageInput::ViewZoom { .. },
            ..
        }
    );
    let accepted = open.enqueue(input);
    if accepted && is_zoom {
        open.image_camera.borrow_mut().cancel();
        open.image_redraw.stop();
    }
    if accepted && let Some(preference) = preference {
        if let Some(info) = open.image_info.borrow().as_ref()
            && let ViewerPreferenceChange::ImageZoom(zoom)
            | ViewerPreferenceChange::DocumentZoom(zoom) = preference
        {
            open.image_zoom.borrow_mut().accepted(
                open.frame_generation.get(),
                info,
                image_viewport(open),
                zoom,
            );
        }
        accept_preference(open, preference);
    }
    accepted
}

fn present_output(open: &OpenWindow, service: &Service, copy: &dyn Fn(String)) {
    pixel_delivery::receive(open, service, copy);
}

fn present_outputs(
    open: &OpenWindow,
    service: &Service,
    copy: &dyn Fn(String),
    outputs: Vec<ViewerOutput>,
) {
    let window = &open.window;
    for output in outputs {
        match &output {
            ViewerOutput::Frame(frame) => {
                if frame.revision <= open.frame.get()
                    || frame.generation < open.frame_generation.get()
                    || !raster_rebase::allows(open, frame.raster_rebase)
                {
                    service.retire(output);
                    continue;
                }
                if !open.text_position.borrow_mut().frame(
                    frame.generation,
                    frame.position_sequence,
                    [frame.position, frame.horizontal_position],
                ) {
                    service.retire(output);
                    continue;
                }
                raster_motion::receive(open, frame);
                if frame.raster.is_some() {
                    if !raster_presentation::receive(open, frame) {
                        service.retire(output);
                        continue;
                    }
                } else {
                    if frame.image.is_none()
                        && (frame.generation != open.frame_generation.get()
                            || open.raster_size.get() != (frame.width, frame.height)
                            || window.get_image_mode())
                    {
                        window.set_raster_viewport_width(window.get_content_width().max(1.0));
                        window.set_raster_viewport_height(window.get_content_height().max(1.0));
                    }
                    if window.get_retained_raster() {
                        raster_presentation::clear(open);
                    }
                }
                if let (Some(info), Some(tiles)) = (&frame.image, &frame.tiles) {
                    let incoming = open.image_camera.borrow().incoming(info);
                    let Some((displayed, replace)) = incoming else {
                        service.retire(output);
                        continue;
                    };
                    let model = open
                        .image_textures
                        .borrow_mut()
                        .receive(tiles, info, displayed, replace);
                    let Ok(model) = model else {
                        if let Err(message) = model {
                            window.set_status(message.into());
                        }
                        service.retire(output);
                        continue;
                    };
                    open.image_camera.borrow_mut().receive(
                        info,
                        window.window().scale_factor(),
                        open.image_textures.borrow().grid(),
                    );
                    window.set_image_tiles(model);
                    window.set_tiled(true);
                    window.set_tile_upload_count(
                        open.image_textures.borrow().uploads.min(i32::MAX as u64) as i32,
                    );
                    window.set_surface(slint::Image::default());
                } else {
                    window.set_tiled(false);
                    window.set_image_tiles(Default::default());
                    open.image_camera.borrow_mut().suspend();
                    open.image_textures.borrow_mut().clear();
                }
                open.image_zoom
                    .borrow_mut()
                    .receive(frame.generation, frame.image.as_ref());
                *open.image_info.borrow_mut() = frame.image.clone();
                if *open.modes.borrow() != frame.modes {
                    *open.modes.borrow_mut() = frame.modes.clone();
                    window.set_modes(slint::ModelRc::new(slint::VecModel::from(
                        frame
                            .modes
                            .iter()
                            .map(|mode| mode_label(*mode).into())
                            .collect::<Vec<slint::SharedString>>(),
                    )));
                }
                window.set_mode_index(
                    frame
                        .modes
                        .iter()
                        .position(|mode| *mode == frame.mode)
                        .unwrap_or(0) as i32,
                );
                open.frame.set(frame.revision);
                open.frame_generation.set(frame.generation);
                let image_mode = frame.image.is_some();
                if image_mode != window.get_image_mode() {
                    window.set_search_visible(false);
                    window.invoke_focus_content();
                }
                window.set_image_mode(image_mode);
                if let Some(info) = &frame.image {
                    // A frame retains its physical viewport while native DPI or
                    // mode-dependent chrome settles. Stretching it to today's
                    // canvas would silently change its declared image zoom.
                    window.set_image_pixel_scale(1.0 / window.window().scale_factor());
                    window.set_image_native_pixels(info.page == 0 && info.raster_scale == 1.0);
                    window.set_image_viewport_width((info.visible_source[0] * info.zoom) as f32);
                    window.set_image_viewport_height((info.visible_source[1] * info.zoom) as f32);
                }
                let web_mode = frame.mode == VIEWER_MODE_HTML;
                if web_mode != window.get_web_mode() {
                    window.set_web_network(false);
                    window.set_web_scripts(false);
                    window.invoke_focus_content();
                }
                window.set_web_mode(web_mode);
                window.set_source_mode(frame.mode == 2 && !image_mode);
                window.set_markdown_mode(frame.mode == 4 && !image_mode);
                window.set_font_available(frame.mode != 3 && !image_mode && !web_mode);
                preferences::present(open);
                window.set_document_page(frame.image.as_ref().map_or(0, |image| image.page as i32));
                window.set_document_pages(
                    frame
                        .image
                        .as_ref()
                        .map_or(0, |image| image.page_count as i32),
                );
                if !web_mode {
                    window.set_zoom_percent(
                        frame
                            .image
                            .as_ref()
                            .map_or(100.0, |image| (image.zoom * 100.0) as f32),
                    );
                }
                window.set_wrap_enabled(frame.wrap_enabled);
                window.set_wrap_available(frame.wrap_available);
                let [width, height] = frame
                    .raster
                    .as_ref()
                    .map_or([frame.width, frame.height], |info| info.viewport);
                open.raster_size.set((width, height));
                if frame.tiles.is_none() && frame.raster.is_none() {
                    match pixel_delivery::pixels(open, frame) {
                        Ok(pixels) => {
                            window.set_surface(slint::Image::from_rgba8_premultiplied(pixels.body))
                        }
                        Err(error) => {
                            window.set_status(error.into());
                            service.retire(output);
                            continue;
                        }
                    }
                }
                window.set_accessible_text(frame.accessible_text.as_ref().into());
                window.set_position(frame.position as f32);
                window.set_displayed_position(frame.position as f32);
                window.set_vertical_extent(frame.vertical_extent as f32);
                window.set_horizontal_position(frame.horizontal_position as f32);
                window.set_horizontal_extent(frame.horizontal_extent as f32);
                window.set_caption(frame.caption.as_str().into());
                window.set_status(frame.status.as_str().into());
                if image_mode {
                    window.set_position_context(
                        format!(
                            "image:{}",
                            frame.image.as_ref().map_or(0, |info| info.surface.0)
                        )
                        .into(),
                    );
                    if frame.tiles.is_some() {
                        image_motion::present(open);
                    } else {
                        window.set_position_pending(false);
                    }
                } else {
                    window.set_position_context(format!("text:{}", frame.generation).into());
                    present_position(open);
                    if frame.raster.is_some() {
                        raster_presentation::present(open);
                    }
                }
                raster_rebase::presented(open, frame);
                window.invoke_frame_presented();
            }
            ViewerOutput::RasterRebase {
                surface,
                transition,
            } => {
                raster_rebase::receive(open, *surface, *transition);
            }
            ViewerOutput::RasterUnavailable { surface, message } => {
                if raster_rebase::unavailable(open, *surface)
                    || open
                        .retained_raster
                        .borrow()
                        .camera
                        .info()
                        .is_some_and(|info| info.surface == *surface)
                {
                    // Keep the displayed pixels and settle at the actual camera.
                    // The failed frontier must not replay indefinitely.
                    raster_motion::cancel(open);
                    window.set_status(message.as_str().into());
                }
            }
            ViewerOutput::Status(message) => window.set_status(message.as_str().into()),
            ViewerOutput::ImageUnavailable {
                generation,
                surface,
                message,
            } => {
                if *generation >= open.frame_generation.get()
                    && (surface.0 == 0
                        || open
                            .image_info
                            .borrow()
                            .as_ref()
                            .is_some_and(|info| info.surface == *surface))
                {
                    open.frame_generation.set(*generation);
                    open.text_position.borrow_mut().changing_content();
                    open.image_zoom.borrow_mut().clear();
                    open.image_camera.borrow_mut().suspend();
                    open.image_redraw.stop();
                    raster_motion::cancel(open);
                    window.set_status(message.as_str().into());
                }
            }
            ViewerOutput::CopyText(text) => copy(text.clone()),
            ViewerOutput::WebShortcut(shortcut) => web_input::received(open, *shortcut),
            ViewerOutput::WebState {
                generation,
                policy,
                zoom,
            } => {
                if *generation >= open.frame_generation.get()
                    && open.text_position.borrow().context_available()
                {
                    window.set_web_network(policy.network);
                    window.set_web_scripts(policy.scripts);
                    window.set_zoom_percent((zoom * 100.0) as f32);
                }
            }
            ViewerOutput::Closed => close(open),
        }
        service.retire(output);
    }
    if !open.closed.get() {
        // A control-only delivery can overtake preparation of the first frame.
        // Do not report successful opening before any pixels were admitted.
        open.acknowledge_opening(open.session.mailbox.take_open_result());
    }
}

fn present_position(open: &OpenWindow) {
    let state = open.text_position.borrow();
    let [vertical, horizontal] = state.positions();
    open.window.set_position_pending(state.has_pending());
    open.window.set_position(vertical as f32);
    open.window.set_horizontal_position(horizontal as f32);
}

fn flush_position(open: &OpenWindow) {
    open.text_position.borrow_mut().flush(|target| {
        open.enqueue(ViewerInput::Position {
            generation: target.generation,
            sequence: target.sequence,
            horizontal: target.horizontal,
            position: target.position,
        })
    });
}

fn observe_native_content(
    open: &OpenWindow,
    event: &slint::winit_030::winit::event::WindowEvent,
    shortcuts: bool,
    ancillary: &mut midnight_ui_input::NativeScroll,
) -> bool {
    // Every native transition must update auxiliary press/gesture bookkeeping,
    // including a release that the browser or retained image consumes below.
    let ancillary_consumed = ancillary.observe(open.window.window(), event, None);
    image_motion::bookkeeping(open, event);
    if web_input::observe(open, event, shortcuts) {
        ancillary.cancel();
        return true;
    }
    if image_motion::route(open, event) {
        ancillary.cancel();
        return true;
    }
    ancillary_consumed
}

fn bind(open: &Rc<OpenWindow>) {
    bind_image(open);
    web_input::bind(open);
    preferences::bind(open);
    let weak = Rc::downgrade(open);
    open.window.on_change_wrap(move |wrap| {
        if let Some(open) = weak.upgrade() {
            accept_preference(&open, ViewerPreferenceChange::Wrap(wrap));
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_change_case_sensitive(move |value| {
        if let Some(open) = weak.upgrade() {
            accept_preference(&open, ViewerPreferenceChange::CaseSensitive(value));
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_change_mode(move |index| {
        if let Some(open) = weak.upgrade()
            && let Ok(index) = usize::try_from(index)
            && let Some(mode) = open.modes.borrow().get(index)
            && index != open.window.get_mode_index() as usize
            && open.enqueue(ViewerInput::Mode(*mode))
        {
            open.text_position.borrow_mut().changing_content();
            open.window
                .set_position_context(format!("mode:{index}").into());
        }
    });
    let native = Rc::new(RefCell::new(midnight_ui_input::ShortcutState::default()));
    let weak = Rc::downgrade(open);
    let native_state = native.clone();
    let mut ancillary_scroll = midnight_ui_input::NativeScroll::new(&open.window);
    open.window.window().on_winit_window_event(move |_, event| {
        let Some(open) = weak.upgrade() else {
            return EventResult::Propagate;
        };
        if midnight_ui_input::native_escape_pressed(event) {
            close(&open);
            ancillary_scroll.cancel();
            return EventResult::PreventDefault;
        }
        let consumed = native_state.borrow_mut().handle(event, |chord| {
            let action = (open.key_resolver.borrow())(&chord, !open.window.get_content_focused());
            action.is_some_and(|action| dispatch_key(&open, &action))
        });
        if consumed {
            ancillary_scroll.cancel();
            return EventResult::PreventDefault;
        }
        if observe_native_content(
            &open,
            event,
            !native_state.borrow().alt_gr(),
            &mut ancillary_scroll,
        ) {
            EventResult::PreventDefault
        } else {
            EventResult::Propagate
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_input_key(move |event, search| {
        let Some(open) = weak.upgrade() else {
            return false;
        };
        if midnight_ui_input::escape_pressed(&event) {
            close(&open);
            return true;
        }
        let Some(chord) = native.borrow().logical_chord(&event) else {
            return false;
        };
        let action = (open.key_resolver.borrow())(&chord, search);
        action.is_some_and(|action| dispatch_key(&open, &action))
    });
    bind_pointer(open);
    let weak = Rc::downgrade(open);
    open.window.on_search(move |backwards| {
        if let Some(open) = weak.upgrade()
            && open.enqueue(ViewerInput::Search {
                query: open.window.get_query().to_string(),
                backwards,
                case_sensitive: open.window.get_case_sensitive(),
            })
        {
            open.text_position.borrow_mut().cancel();
        }
    });
    let weak = Rc::downgrade(open);
    open.window.window().on_close_requested(move || {
        if let Some(open) = weak.upgrade() {
            close(&open);
        }
        slint::CloseRequestResponse::HideWindow
    });
}

fn dispatch_key(open: &OpenWindow, action: &str) -> bool {
    if raster_rebase::waiting(open) && matches!(action, "viewer.copy" | "viewer.select-page") {
        return true;
    }
    if open.window.get_web_mode()
        && let Some(result) = web_input::navigate(open, action)
    {
        return result;
    }
    let input = match action {
        "viewer.close" => {
            close(open);
            return true;
        }
        "viewer.find" => {
            if open.window.get_image_mode() {
                return false;
            }
            focus_search(open);
            return true;
        }
        "viewer.next-file" => ViewerInput::NextFile,
        "viewer.document-previous" | "viewer.document-next" => {
            if open.window.get_document_page() == 0 {
                return false;
            }
            if action == "viewer.document-next"
                && open.window.get_document_page() >= open.window.get_document_pages()
                || action == "viewer.document-previous" && open.window.get_document_page() <= 1
            {
                return false;
            }
            ViewerInput::DocumentPage {
                frame: open.frame.get(),
                delta: if action == "viewer.document-next" {
                    1
                } else {
                    -1
                },
            }
        }
        "viewer.wrap" => {
            if !open.window.get_wrap_available() {
                return false;
            }
            accept_preference(
                open,
                ViewerPreferenceChange::Wrap(!open.window.get_wrap_enabled()),
            );
            return true;
        }
        "viewer.zoom-in" | "viewer.zoom-out" | "viewer.fit" | "viewer.actual-pixels" => {
            if open.window.get_web_mode() {
                let current = f64::from(open.window.get_zoom_percent()) / 100.0;
                let zoom = match action {
                    "viewer.zoom-in" => current * 1.25,
                    "viewer.zoom-out" => current * 0.8,
                    _ => 1.0,
                }
                .clamp(0.1, 10.0);
                if web_input::send(open, viewer_web::CommandKind::Zoom(zoom)) {
                    open.window.set_zoom_percent((zoom * 100.0) as f32);
                    return true;
                }
                return false;
            }
            if !open.window.get_image_mode() {
                return false;
            }
            let input = match action {
                "viewer.fit" => ViewerImageInput::Fit,
                "viewer.actual-pixels" => ViewerImageInput::ActualPixels,
                _ => ViewerImageInput::Zoom {
                    factor: if action == "viewer.zoom-in" {
                        1.25
                    } else {
                        0.8
                    },
                    anchor: None,
                },
            };
            ViewerInput::Image {
                frame: open.frame.get(),
                input,
            }
        }
        "viewer.copy" => raster_presentation::command(open, true).unwrap_or(ViewerInput::Copy {
            frame: open.frame.get(),
        }),
        "viewer.select-page" => {
            raster_presentation::command(open, false).unwrap_or(ViewerInput::SelectPage {
                frame: open.frame.get(),
            })
        }
        "viewer.find-next" | "viewer.find-previous" => {
            if open.window.get_image_mode() {
                return false;
            }
            if open.window.get_query().is_empty() {
                focus_search(open);
                return true;
            }
            ViewerInput::Search {
                query: open.window.get_query().to_string(),
                backwards: action == "viewer.find-previous",
                case_sensitive: open.window.get_case_sensitive(),
            }
        }
        "viewer.first" => ViewerInput::Seek(0.0),
        "viewer.last" => ViewerInput::Seek(1.0),
        action => match scrolling::keyboard(
            action,
            open.window.get_content_height(),
            open.window.window().scale_factor(),
        ) {
            Some(input) => input,
            None => return false,
        },
    };
    if let ViewerInput::Scroll { x, y } = input
        && open.window.get_tiled()
    {
        return image_motion::push(
            open,
            x / open.window.window().scale_factor(),
            y / open.window.window().scale_factor(),
            false,
        );
    }
    if let ViewerInput::Scroll { x, y } = input
        && open.window.get_retained_raster()
        && raster_presentation::push(
            open,
            x / open.window.window().scale_factor(),
            y / open.window.window().scale_factor(),
            true,
        )
    {
        return true;
    }
    let changing = matches!(
        input,
        ViewerInput::NextFile | ViewerInput::DocumentPage { .. }
    );
    let new_file = matches!(input, ViewerInput::NextFile);
    let navigation = matches!(
        input,
        ViewerInput::Scroll { .. }
            | ViewerInput::Seek(_)
            | ViewerInput::Pan(_)
            | ViewerInput::Search { .. }
            | ViewerInput::Wrap(_)
    );
    let accepted = send_input(open, input);
    if changing && accepted {
        open.image_zoom.borrow_mut().clear();
        if new_file {
            open.image_camera.borrow_mut().suspend();
            open.text_position.borrow_mut().changing_content();
        } else {
            open.image_camera.borrow_mut().cancel();
        }
        open.window
            .set_position_context(format!("changing:{}", open.frame.get()).into());
    }
    if navigation && accepted {
        raster_motion::cancel(open);
        open.text_position.borrow_mut().cancel();
    }
    accepted
}

fn focus_search(open: &OpenWindow) {
    if open.window.get_web_mode() {
        web_input::send(open, viewer_web::CommandKind::Focus(false));
    }
    open.window.invoke_focus_search();
}

fn bind_pointer(open: &Rc<OpenWindow>) {
    let weak = Rc::downgrade(open);
    open.window.on_scroll(move |x, y| {
        if let Some(open) = weak.upgrade() {
            // Logical/pixel input already includes the platform's own momentum.
            raster_motion::cancel_legacy(&open);
            if open.window.get_tiled() {
                image_motion::push(&open, x, y, true);
                return;
            }
            if open.window.get_retained_raster() && raster_presentation::push(&open, x, y, true) {
                return;
            }
            if raster_motion::continue_direct(&open, x, y) {
                return;
            }
            let scale = open.window.window().scale_factor();
            if open.enqueue(ViewerInput::Scroll {
                x: x * scale,
                y: y * scale,
            }) {
                open.text_position.borrow_mut().cancel();
            }
        }
    });
    fn position(open: &OpenWindow, horizontal: bool, value: f32, dragging: bool) {
        if raster_presentation::seek(open, horizontal, f64::from(value), dragging) {
            return;
        }
        raster_rebase::cancel_context(open);
        raster_motion::cancel(open);
        if open.window.get_tiled() {
            image_motion::seek(open, usize::from(!horizontal), f64::from(value));
            return;
        }
        if open.window.get_image_mode() {
            let input = if horizontal {
                ViewerInput::Pan(f64::from(value))
            } else {
                ViewerInput::Seek(f64::from(value))
            };
            if open.enqueue(input) {
                if horizontal {
                    open.window.set_horizontal_position(value);
                } else {
                    open.window.set_position(value);
                }
            }
            return;
        }
        open.text_position
            .borrow_mut()
            .request(horizontal, f64::from(value));
        present_position(open);
        flush_position(open);
    }
    let weak = Rc::downgrade(open);
    open.window.on_seek(move |value| {
        if let Some(open) = weak.upgrade() {
            position(&open, false, value, false);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_pan(move |value| {
        if let Some(open) = weak.upgrade() {
            position(&open, true, value, false);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_drag_seek(move |horizontal, value| {
        if let Some(open) = weak.upgrade() {
            position(&open, horizontal, value, true);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_position_drag_began(move |_| {
        if let Some(open) = weak.upgrade() {
            raster_presentation::begin_seek(&open);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_position_drag_ended(move |_| {
        if let Some(open) = weak.upgrade() {
            raster_presentation::end_seek(&open);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_select_point(move |x, y, extend| {
        if let Some(open) = weak.upgrade() {
            raster_motion::cancel(&open);
            if raster_rebase::waiting(&open) {
                return;
            }
            if let Some(input) = raster_presentation::select(&open, x, y, extend) {
                open.enqueue(input);
                return;
            }
            let (width, height) = open.raster_size.get();
            open.enqueue(ViewerInput::Select {
                frame: open.frame.get(),
                x: x * width as f32 / open.window.get_raster_viewport_width().max(1.0),
                y: y * height as f32 / open.window.get_raster_viewport_height().max(1.0),
                extend,
            });
        }
    });
}

fn bind_image(open: &Rc<OpenWindow>) {
    let weak = Rc::downgrade(open);
    open.window.on_document_turn(move |delta| {
        if let Some(open) = weak.upgrade() {
            dispatch_key(
                &open,
                if delta > 0 {
                    "viewer.document-next"
                } else {
                    "viewer.document-previous"
                },
            );
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_image_command(move |command| {
        if let Some(open) = weak.upgrade() {
            let action = match command {
                0 => "viewer.fit",
                1 => "viewer.actual-pixels",
                2 => "viewer.zoom-in",
                3 => "viewer.zoom-out",
                _ => return,
            };
            dispatch_key(&open, action);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_image_zoom(move |delta, x, y| {
        if let Some(open) = weak.upgrade()
            && delta.is_finite()
            && delta != 0.0
        {
            let scale = open.window.window().scale_factor();
            send_input(
                &open,
                ViewerInput::Image {
                    frame: open.frame.get(),
                    input: ViewerImageInput::Zoom {
                        factor: if delta > 0.0 { 1.25 } else { 0.8 },
                        anchor: Some([x * scale, y * scale]),
                    },
                },
            );
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_image_drag(move |x, y| {
        if let Some(open) = weak.upgrade() {
            if open.window.get_tiled() {
                image_motion::push(&open, -x, -y, true);
                return;
            }
            let scale = open.window.window().scale_factor();
            send_input(
                &open,
                ViewerInput::Image {
                    frame: open.frame.get(),
                    input: ViewerImageInput::Drag {
                        x: x * scale,
                        y: y * scale,
                    },
                },
            );
        }
    });
}

fn close(open: &OpenWindow) {
    raster_rebase::cancel_context(open);
    raster_motion::cancel(open);
    if !open.closed.replace(true) {
        if let Some(preparation) = open.preparation.borrow().as_ref() {
            preparation.stop();
        }
        open.prepared_pixels.borrow_mut().take();
        open.session.close();
        open.timer.stop();
        open.image_zoom.borrow_mut().clear();
        open.image_redraw.stop();
        open.raster_redraw.stop();
        let _ = open.window.hide();
        open.window.set_surface(slint::Image::default());
        open.window.set_image_tiles(Default::default());
        open.image_textures.borrow_mut().clear();
        raster_presentation::clear(open);
        open.window.set_accessible_text("".into());
        open.opened(Err("Viewer closed before displaying content.".into()));
    }
}

fn mode_label(mode: u16) -> &'static str {
    match mode {
        1 => "Text",
        2 => "Source",
        3 => "Binary",
        4 => "Markdown",
        VIEWER_MODE_IMAGE => "Image",
        VIEWER_MODE_HTML => "HTML",
        _ => "Unknown",
    }
}

fn color(value: [u8; 4]) -> slint::Color {
    slint::Color::from_argb_u8(value[3], value[0], value[1], value[2])
}
