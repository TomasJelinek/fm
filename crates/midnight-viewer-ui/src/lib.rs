//! Thin UI adapters. Reading, parsing, search, layout and selection run on the backend.
#![deny(unsafe_code)]

mod appearance;
#[cfg(test)]
mod control_sync_tests;
#[cfg(test)]
mod document_layout_tests;
#[cfg(test)]
mod find_action_tests;
mod image_camera;
#[cfg(test)]
mod image_key_tests;
mod image_motion;
#[cfg(test)]
mod image_presentation_tests;
mod image_tiles;
mod image_zoom;
#[cfg(test)]
mod kinetic_tests;
#[cfg(test)]
mod layout_handshake_tests;
mod media_controls;
#[cfg(test)]
mod opening_retirement_tests;
#[cfg(test)]
mod opening_tests;
mod pixel_delivery;
mod pixel_preparation;
mod preferences;
mod raster_camera;
#[cfg(test)]
mod raster_frontier_tests;
mod raster_motion;
mod raster_presentation;
#[cfg(test)]
mod raster_presentation_tests;
mod raster_rebase;
#[cfg(test)]
mod raster_rebase_tests;
#[cfg(test)]
mod raster_render_tests;
mod sandbox_notice;
#[cfg(test)]
mod sandbox_notice_tests;
#[cfg(test)]
mod scrollbar_drag_tests;
mod scrolling;
mod text_position;
#[cfg(test)]
mod text_presentation_tests;
mod web_input;
#[cfg(test)]
mod web_input_tests;
#[cfg(test)]
mod web_popup_tests;
#[cfg(test)]
mod web_wheel_tests;

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
pub use generated::{MediaControlsState, ViewerRasterTile, ViewerWindow};
pub use midnight_ui_input::WindowPlacement;

pub type KeyResolver = Rc<dyn Fn(&ShortcutChord, bool) -> Option<String>>;
/// Called once after initial backend content is ready, fails, or closes before readiness.
///
/// Closing a window that has not displayed content reports `Err` synchronously
/// from inside the close call (`ViewerWindows::close_all`, Escape or the window's
/// close button). The callback may re-enter `ViewerWindows`; no manager state is
/// borrowed while it runs.
pub type OpenReady = Box<dyn FnOnce(Result<(), String>)>;
type PreferencesListener = Rc<dyn Fn(ViewerPreferenceChange)>;
type PreferencesRelay = Rc<RefCell<PreferencesListener>>;
type InputSender = Rc<dyn Fn(ViewerInput) -> bool>;

#[derive(Default)]
struct LayoutState {
    token: u64,
    mode: u16,
    generation_floor: u64,
    pending_ack: bool,
    awaiting_frame: bool,
}

impl LayoutState {
    fn accepts_frame(&self, frame: &ViewerFrame) -> bool {
        frame.generation >= self.generation_floor
            && (self.token == 0 || (!self.pending_ack && frame.mode == self.mode))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct DocumentLayoutIntent {
    token: u64,
    layout: DocumentLayout,
    requested_frame: u64,
    submitted_frame: Option<u64>,
}

pub struct ViewerWindows {
    service: Service,
    windows: RefCell<Vec<Rc<OpenWindow>>>,
    preferences_listener: PreferencesRelay,
    appearance: RefCell<Option<appearance::AppAppearance>>,
}

struct OpenWindow {
    window: ViewerWindow,
    media_controls: media_controls::MediaControls,
    session: Rc<WindowSession>,
    input: RefCell<InputSender>,
    timer: Timer,
    closed: Cell<bool>,
    ready: RefCell<Option<OpenReady>>,
    opening_succeeded: Cell<bool>,
    preparation: RefCell<Option<pixel_preparation::Preparation>>,
    prepared_pixels: RefCell<Option<pixel_preparation::PreparedPixels>>,
    palette: appearance::PaletteGate,
    last_geometry: Cell<(u32, u32, u32)>,
    layout: RefCell<LayoutState>,
    frame: Cell<u64>,
    frame_generation: Cell<u64>,
    raster_size: Cell<(u32, u32)>,
    modes: RefCell<Vec<u16>>,
    /// Chooser index of the mode presented by the last frame or requested since.
    /// A refused request restores this authoritative value.
    mode_index: Cell<i32>,
    preferences_listener: PreferencesRelay,
    image_info: RefCell<Option<ViewerImageInfo>>,
    document_layout: RefCell<Option<DocumentLayoutIntent>>,
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
        if self.closed.get() {
            return false;
        }
        if !self.window.get_web_policy_change_allowed()
            && matches!(
                &input,
                ViewerInput::Web {
                    input: viewer_web::CommandKind::Policy(_),
                    ..
                }
            )
        {
            return false;
        }
        if let ViewerInput::DocumentLayout { frame, .. } = &input
            && (*frame == 0
                || *frame != self.frame.get()
                || !self.text_position.borrow().context_available()
                || self
                    .image_info
                    .borrow()
                    .as_ref()
                    .and_then(|info| info.document_layout)
                    .is_none())
        {
            return false;
        }
        // A refused browser never paints, so its notice's Install… passes too.
        if self.layout.borrow().awaiting_frame
            && !matches!(
                input,
                ViewerInput::LayoutReady { .. }
                    | ViewerInput::Resize(_)
                    | ViewerInput::Mode(_)
                    | ViewerInput::NextFile
                    | ViewerInput::PreviousFile
                    | ViewerInput::RetainedRaster(_)
                    | ViewerInput::InstallSandboxPermission
                    | ViewerInput::Close
            )
        {
            return false;
        }
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
                | ViewerInput::PreviousFile
                | ViewerInput::Resize(_)
                | ViewerInput::Search { .. }
                | ViewerInput::Seek(_)
                | ViewerInput::Pan(_)
                | ViewerInput::Position { .. }
        );
        let retires_document = matches!(
            input,
            ViewerInput::Mode(_)
                | ViewerInput::NextFile
                | ViewerInput::PreviousFile
                | ViewerInput::Close
        );
        let replaces_motion =
            retires_document || (changes_context && !matches!(input, ViewerInput::Resize(_)));
        let accepted = sender(input);
        if accepted && replaces_motion {
            raster_motion::retire_raw(self);
        }
        if accepted && retires_document {
            self.document_layout.borrow_mut().take();
        }
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
            appearance: RefCell::new(None),
        }
    }

    pub fn set_preferences_listener(&self, listener: Rc<dyn Fn(ViewerPreferenceChange)>) {
        *self.preferences_listener.borrow_mut() = listener;
    }

    /// Apply the displayed app theme to chrome and four document palette colors.
    /// Document fonts, zoom, layout and preference listeners are unaffected.
    pub fn apply_appearance(
        &self,
        appearance: &ViewerAppearance,
        theme: PreferenceTheme,
        density: UiDensity,
        desktop_font_size: u16,
        touch_font_size: u16,
    ) {
        let next = appearance::AppAppearance::new(
            appearance,
            theme,
            density,
            desktop_font_size,
            touch_font_size,
        );
        let changed = self.appearance.borrow().as_ref() != Some(&next);
        if changed {
            *self.appearance.borrow_mut() = Some(next.clone());
        }
        for open in self
            .windows
            .borrow()
            .iter()
            .filter(|open| !open.closed.get())
        {
            if changed {
                next.present(&open.window);
            }
            // Equal app tuples do not resubmit accepted colors. A refused
            // admission remains retryable without rewriting widget properties.
            next.restore_palette(open);
        }
    }

    /// Restore without reporting a user edit or writing application settings.
    pub fn restore_preferences(&self, preferences: std::sync::Arc<ViewerPreferences>) {
        // Release closed windows, and do not hold the list while presenting.
        let windows = {
            let mut windows = self.windows.borrow_mut();
            windows.retain(|window| !window.closed.get());
            windows.clone()
        };
        for open in &windows {
            let previous = open.session.preferences();
            let raster_zoom_changed = previous.image_zoom != preferences.image_zoom;
            let layout_changed = previous.wrap != preferences.wrap
                || previous.line_numbers != preferences.line_numbers
                || previous.text_font != preferences.text_font
                || previous.source_font != preferences.source_font
                || previous.markdown_font != preferences.markdown_font
                || previous.markdown_preset != preferences.markdown_preset;
            if let Err(message) = open.session.restore_preferences(preferences.clone()) {
                open.window
                    .set_status(midnight_core::visible_name(&message).as_ref().into());
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
        mut appearance: ViewerAppearance,
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
        let opening_trace = pixel_preparation::OpeningTrace::new();
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
        apply_window_size(&window, placement.logical_size());
        placement.apply(window.window());
        let app_appearance = self.appearance.borrow().clone();
        if let Some(app) = &app_appearance {
            app.merge_palette(&mut appearance);
        }
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
        if let Some(app) = &app_appearance {
            app.present(&window);
        }
        // Showing supplies native size and scale for provisional geometry. The
        // selected backend route receives its measured canvas through LayoutReady.
        if let Err(error) = window.show() {
            let _ = window.hide();
            self.service.retire(launch);
            return Err(error.to_string());
        }
        let geometry = window_geometry(&window);
        let opening_palette = appearance.clone();
        let session = match self.service.open_with_layout(launch, appearance, geometry) {
            Ok(session) => Rc::new(session),
            Err(error) => {
                let _ = window.hide();
                return Err(error);
            }
        };
        let palette = match prepare_session(&session, &opening_palette) {
            Ok(palette) => palette,
            Err(error) => {
                session.close();
                let _ = window.hide();
                return Err(error);
            }
        };
        let input = Rc::downgrade(&session);
        let open = Rc::new_cyclic(|weak: &std::rc::Weak<OpenWindow>| OpenWindow {
            media_controls: media_controls::MediaControls::install(
                &window,
                Rc::new({
                    let weak = weak.clone();
                    move |control| {
                        weak.upgrade()
                            .is_some_and(|open| open.enqueue(ViewerInput::MediaControl(control)))
                    }
                }),
            ),
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
            palette,
            last_geometry: Cell::new((geometry.width, geometry.height, geometry.scale.to_bits())),
            layout: RefCell::new(LayoutState::default()),
            frame: Cell::new(0),
            frame_generation: Cell::new(0),
            raster_size: Cell::new((1, 1)),
            modes: RefCell::new(Vec::new()),
            mode_index: Cell::new(0),
            preferences_listener: self.preferences_listener.clone(),
            image_info: RefCell::new(None),
            document_layout: RefCell::new(None),
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
        if let Err(error) =
            pixel_delivery::bind(&open, self.service.clone(), copy.clone(), opening_trace)
        {
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
                preferences::present_font_catalog(&open);
                image_motion::refresh_scale(&open);
                pump_geometry(&open);
                present_output(&open, &service, copy.as_ref());
                flush_document_layout(&open);
                image_motion::fallback(&open);
                flush_position(&open);
            });
        open.window.invoke_focus_content();
        let weak_window = open.window.as_weak();
        self.windows.borrow_mut().push(open);
        Ok(weak_window)
    }

    pub fn close_all(&self) {
        // `close` reports cancellation to `OpenReady` callbacks, which may
        // re-enter this manager. Never run them while the list is borrowed.
        let windows = std::mem::take(&mut *self.windows.borrow_mut());
        for open in windows {
            close(&open);
        }
    }
}

/// A canvas need not be a whole number of physical pixels (for example 22.5 px
/// of chrome at 125%). Round to the nearest raster size: presentation maps it 1:1
/// to physical pixels and clips at most half a pixel instead of stretching.
fn physical_extent(logical: f32, scale: f32) -> u32 {
    (logical * scale).round().max(1.0) as u32
}

fn prepare_session(
    session: &WindowSession,
    opening_palette: &ViewerAppearance,
) -> Result<appearance::PaletteGate, String> {
    let palette = appearance::PaletteGate::opening(session, opening_palette);
    if !session.send(ViewerInput::RetainedRaster(true)) {
        return Err("Viewer input is unavailable.".into());
    }
    Ok(palette)
}

fn window_geometry(window: &ViewerWindow) -> ViewerGeometry {
    let scale = geometry_properties(window);
    ViewerGeometry {
        width: physical_extent(window.get_content_width(), scale),
        height: physical_extent(window.get_content_height(), scale),
        scale,
    }
}

fn geometry_properties(window: &ViewerWindow) -> f32 {
    let scale = window.window().scale_factor();
    apply_window_size(window, window.window().size().to_logical(scale));
    window.set_image_pixel_scale(1.0 / scale);
    scale
}

/// The layout choices that follow the window's logical size: compact and narrow
/// controls, and the size that plans the rows around an open Find bar.
fn apply_window_size(window: &ViewerWindow, size: slint::LogicalSize) {
    window.set_compact(size.width < 500.0);
    window.set_narrow_typography(size.width < 680.0);
    window.set_available_width(size.width);
    window.set_available_height(size.height);
}

fn layout_geometry(window: &ViewerWindow) -> Option<ViewerGeometry> {
    let scale = geometry_properties(window);
    i_slint_core::window::WindowInner::from_pub(window.window()).ensure_tree_instantiated();
    let width = window.get_content_width();
    let height = window.get_content_height();
    if !scale.is_finite()
        || scale <= 0.0
        || !width.is_finite()
        || !height.is_finite()
        || width <= 0.0
        || height <= 0.0
    {
        return None;
    }
    Some(ViewerGeometry {
        width: physical_extent(width, scale),
        height: physical_extent(height, scale),
        scale,
    })
}

fn apply_mode(open: &OpenWindow, mode: u16, image_mode: bool, reset_policy: bool) {
    let window = &open.window;
    if image_mode != window.get_image_mode() {
        window.set_search_visible(false);
        window.invoke_focus_content();
    }
    window.set_image_mode(image_mode);
    let web_mode = mode == VIEWER_MODE_HTML;
    let web_changed = web_mode != window.get_web_mode();
    if web_changed || reset_policy {
        window.set_web_policy_change_allowed(false);
        web_input::present_policy(open, Default::default());
    }
    if web_changed {
        window.invoke_focus_content();
    }
    window.set_web_mode(web_mode);
    window.set_whole_words_available(!image_mode && !web_mode && mode != 3);
    window.set_source_mode(mode == 2 && !image_mode);
    window.set_markdown_mode(mode == 4 && !image_mode);
    window.set_font_available(mode != 3 && mode != VIEWER_MODE_VIDEO && !image_mode && !web_mode);
    preferences::present(open);
}

fn select_layout(open: &OpenWindow, token: u64, mode: u16, generation: u64) {
    if open.closed.get() {
        return;
    }
    let mut layout = open.layout.borrow_mut();
    if token == 0
        || token <= layout.token
        || generation < layout.generation_floor
        || generation < open.frame_generation.get()
    {
        return;
    }
    layout.token = token;
    layout.mode = mode;
    layout.generation_floor = generation;
    layout.pending_ack = true;
    layout.awaiting_frame = true;
    drop(layout);
    if mode == VIEWER_MODE_VIDEO {
        open.media_controls.begin(MediaSessionId(generation));
    } else {
        open.media_controls.retire_active();
    }
    raster_motion::retire_raw(open);
    raster_rebase::cancel_context(open);
    raster_motion::cancel(open);
    open.retained_raster.borrow_mut().camera.suspend();
    open.raster_redraw.stop();
    open.image_camera.borrow_mut().suspend();
    open.image_zoom.borrow_mut().clear();
    open.document_layout.borrow_mut().take();
    open.image_redraw.stop();
    open.text_position.borrow_mut().cancel();
    open.window
        .set_position_context(format!("layout:{token}:{generation}").into());
    apply_mode(open, mode, mode == VIEWER_MODE_IMAGE, true);
}

/// Returns true while this pump is owned by a layout acknowledgement, including
/// a refused enqueue. A matching Resize must not overtake or repeat that input.
fn flush_layout_ack(open: &OpenWindow) -> bool {
    if open.closed.get() {
        return false;
    }
    let token = {
        let layout = open.layout.borrow();
        if !layout.pending_ack {
            return false;
        }
        layout.token
    };
    let Some(geometry) = layout_geometry(&open.window) else {
        return true;
    };
    let sender = open.input.borrow().clone();
    if sender(ViewerInput::LayoutReady { token, geometry }) && !open.closed.get() {
        let mut layout = open.layout.borrow_mut();
        if layout.token == token && layout.pending_ack {
            layout.pending_ack = false;
            open.last_geometry
                .set((geometry.width, geometry.height, geometry.scale.to_bits()));
        }
    }
    true
}

fn pump_geometry(open: &OpenWindow) {
    if open.closed.get() || flush_layout_ack(open) {
        return;
    }
    let window = &open.window;
    let geometry = window_geometry(window);
    let key = (geometry.width, geometry.height, geometry.scale.to_bits());
    let sender = open.input.borrow().clone();
    if key != open.last_geometry.get() && sender(ViewerInput::Resize(geometry)) {
        raster_rebase::cancel_context(open);
        raster_motion::cancel(open);
        open.retained_raster.borrow_mut().camera.suspend();
        if window.get_image_mode() {
            open.image_zoom.borrow_mut().clear();
            open.text_position.borrow_mut().cancel();
        } else if open.layout.borrow().mode == VIEWER_MODE_VIDEO {
            // Video pixels scale like images; resizing preserves the media session generation.
            open.text_position.borrow_mut().cancel();
        } else {
            open.text_position.borrow_mut().changing_content();
        }
        window.set_position_context(format!("resize:{}:{}:{}", key.0, key.1, key.2).into());
        open.last_geometry.set(key);
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
            open.window
                .set_status(midnight_core::visible_name(&message).as_ref().into());
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

fn change_document_layout(open: &OpenWindow, layout: DocumentLayout) {
    if open.closed.get()
        || open.layout.borrow().awaiting_frame
        || !open.text_position.borrow().context_available()
    {
        return;
    }
    let Some(current) = open
        .image_info
        .borrow()
        .as_ref()
        .and_then(|info| info.document_layout)
    else {
        return;
    };
    let pending = *open.document_layout.borrow();
    if pending.is_none() && current == layout {
        return;
    }
    if !pending.is_some_and(|pending| pending.layout == layout) {
        *open.document_layout.borrow_mut() = Some(DocumentLayoutIntent {
            token: open.layout.borrow().token,
            layout,
            requested_frame: open.frame.get(),
            submitted_frame: None,
        });
    }
    flush_document_layout(open);
}

fn flush_document_layout(open: &OpenWindow) {
    if open.closed.get()
        || open.layout.borrow().awaiting_frame
        || !open.text_position.borrow().context_available()
    {
        return;
    }
    let Some(mut pending) = *open.document_layout.borrow() else {
        return;
    };
    if pending.token != open.layout.borrow().token
        || open
            .image_info
            .borrow()
            .as_ref()
            .and_then(|info| info.document_layout)
            .is_none()
    {
        open.document_layout.borrow_mut().take();
        return;
    }
    let frame = open.frame.get();
    if pending.submitted_frame == Some(frame) {
        return;
    }
    if open.enqueue(ViewerInput::DocumentLayout {
        frame,
        layout: pending.layout,
    }) {
        let mut slot = open.document_layout.borrow_mut();
        if *slot == Some(pending) {
            pending.submitted_frame = Some(frame);
            *slot = Some(pending);
        }
        open.image_zoom.borrow_mut().clear();
    }
}

fn present_document_layout(open: &OpenWindow, frame: &ViewerFrame) {
    let layout = frame.image.as_ref().and_then(|image| image.document_layout);
    open.window.set_document_layout(match layout {
        None => -1,
        Some(DocumentLayout::Paged) => 0,
        Some(DocumentLayout::Continuous) => 1,
    });
    let mut slot = open.document_layout.borrow_mut();
    // LayoutSelected retires source/route authority. Frame generations may
    // advance within the same document, so they are not pending-intent identity.
    if slot.is_some_and(|pending| {
        layout.is_none()
            || pending.token != open.layout.borrow().token
            || (layout == Some(pending.layout) && frame.revision > pending.requested_frame)
    }) {
        slot.take();
    }
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

#[cfg(test)]
fn present_outputs(
    open: &OpenWindow,
    service: &Service,
    copy: &dyn Fn(String),
    outputs: Vec<ViewerOutput>,
) {
    apply_output_controls(open, &outputs);
    present_controlled_outputs(open, service, copy, outputs);
}

fn apply_output_controls(open: &OpenWindow, outputs: &[ViewerOutput]) {
    // A newly selected route fences every frame in this batch, including an old
    // frame prepared before its layout control reached the UI. Settle failed
    // seeks here as well, before the prepared frame's position fence is checked.
    // Closed batches cannot start another opening or acknowledge its canvas.
    if !open.closed.get()
        && !outputs
            .iter()
            .any(|output| matches!(output, ViewerOutput::Closed))
    {
        open.palette.observe(outputs);
        for output in outputs {
            match output {
                ViewerOutput::LayoutSelected {
                    token,
                    mode,
                    content_generation,
                } => select_layout(open, *token, *mode, *content_generation),
                ViewerOutput::MediaRetired { session } => {
                    if open.media_controls.retire(*session) {
                        // A failed replacement may never announce a layout.
                        // Hide the retired actor's picture and accessibility
                        // content before a prepared old frame can be delivered.
                        open.window.set_surface(slint::Image::default());
                        open.window.set_accessible_text(Default::default());
                    }
                }
                ViewerOutput::VerticalPositionUnavailable {
                    generation,
                    sequence,
                } => {
                    // Retained raster motion can advance the displayed camera
                    // independently of the last worker frame.
                    let displayed = open.window.get_displayed_position();
                    let settled = open.text_position.borrow_mut().vertical_unavailable(
                        *generation,
                        *sequence,
                        f64::from(displayed),
                    );
                    if settled {
                        let pending = open.text_position.borrow().has_pending();
                        open.window.set_position(displayed);
                        open.window.set_position_pending(pending);
                    }
                }
                _ => {}
            }
        }
    }
}

fn accepts_frame(open: &OpenWindow, frame: &ViewerFrame) -> bool {
    !open.closed.get()
        && frame.revision > open.frame.get()
        && frame.generation >= open.frame_generation.get()
        && open.layout.borrow().accepts_frame(frame)
        && (frame.mode != VIEWER_MODE_VIDEO
            || open
                .media_controls
                .accepts_frame(MediaSessionId(frame.generation), frame.media_control))
        && open.palette.allows(frame)
        && raster_rebase::allows(open, frame.raster_rebase)
        && open.text_position.borrow().accepts_frame(
            frame.generation,
            frame.position_sequence,
            [frame.position, frame.horizontal_position],
        )
}

/// Controls have already been applied once, including before pixel-error handling.
fn present_controlled_outputs(
    open: &OpenWindow,
    service: &Service,
    copy: &dyn Fn(String),
    outputs: Vec<ViewerOutput>,
) {
    let window = &open.window;
    for output in outputs {
        if open.closed.get() {
            service.retire(output);
            continue;
        }
        match &output {
            ViewerOutput::LayoutSelected { .. }
            | ViewerOutput::PaletteApplied { .. }
            | ViewerOutput::MediaRetired { .. }
            | ViewerOutput::VerticalPositionUnavailable { .. } => {}
            ViewerOutput::MediaSnapshot(snapshot) => {
                open.media_controls.apply(snapshot.clone());
            }
            ViewerOutput::Frame(frame) => {
                if !accepts_frame(open, frame) {
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
                        let viewport = raster_presentation::logical_viewport(
                            open,
                            [frame.width, frame.height],
                        );
                        window.set_raster_viewport_width(viewport[0]);
                        window.set_raster_viewport_height(viewport[1]);
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
                    pixel_delivery::trace_frame(open, 40, frame);
                    let model = open
                        .image_textures
                        .borrow_mut()
                        .receive(tiles, info, displayed, replace);
                    pixel_delivery::trace_frame(open, 41, frame);
                    let Ok(model) = model else {
                        if let Err(message) = model {
                            window
                                .set_status(midnight_core::visible_name(&message).as_ref().into());
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
                let mode_index = frame
                    .modes
                    .iter()
                    .position(|mode| *mode == frame.mode)
                    .unwrap_or(0) as i32;
                open.mode_index.set(mode_index);
                window.set_mode_index(mode_index);
                open.frame.set(frame.revision);
                open.frame_generation.set(frame.generation);
                let image_mode = frame.image.is_some();
                apply_mode(open, frame.mode, image_mode, false);
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
                            window.set_status(midnight_core::visible_name(&error).as_ref().into());
                            service.retire(output);
                            continue;
                        }
                    }
                }
                {
                    let mut layout = open.layout.borrow_mut();
                    layout.generation_floor = layout.generation_floor.max(frame.generation);
                    layout.awaiting_frame = false;
                }
                present_document_layout(open, frame);
                window.set_accessible_text(frame.accessible_text.as_ref().into());
                window.set_position(frame.position as f32);
                window.set_displayed_position(frame.position as f32);
                window.set_vertical_extent(frame.vertical_extent as f32);
                window.set_horizontal_position(frame.horizontal_position as f32);
                window.set_horizontal_extent(frame.horizontal_extent as f32);
                // A file name cannot reorder the window title: explicit bidirectional
                // controls show as escapes, as in the Basic viewer's text.
                window.set_caption(midnight_core::visible_name(&frame.caption).as_ref().into());
                window.set_status(midnight_core::visible_name(&frame.status).as_ref().into());
                if image_mode {
                    if let Some(info) = &frame.image {
                        window.set_position_context(
                            image_camera::position_context(
                                frame.generation,
                                info,
                                [frame.width, frame.height],
                            )
                            .into(),
                        );
                    }
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
                raster_motion::receive(open, frame);
                raster_rebase::presented(open, frame);
                raster_motion::resume_raw(open);
                let opening_report = pixel_delivery::trace_published(open, frame);
                window.invoke_frame_presented();
                if let Some(report) = opening_report {
                    service.retire(report);
                }
            }
            ViewerOutput::RasterRebase {
                surface,
                transition,
            } => {
                if !open.layout.borrow().awaiting_frame {
                    raster_rebase::receive(open, *surface, *transition);
                }
            }
            ViewerOutput::RasterUnavailable { surface, message } => {
                if !open.layout.borrow().awaiting_frame
                    && (raster_rebase::unavailable(open, *surface)
                        || open
                            .retained_raster
                            .borrow()
                            .camera
                            .info()
                            .is_some_and(|info| info.surface == *surface))
                {
                    // Keep the displayed pixels and settle at the actual camera.
                    // The failed frontier must not replay indefinitely.
                    raster_motion::retire_raw(open);
                    raster_motion::cancel(open);
                    window.set_status(midnight_core::visible_name(message).as_ref().into());
                }
            }
            ViewerOutput::Status(message) => {
                window.set_status(midnight_core::visible_name(message).as_ref().into())
            }
            ViewerOutput::SandboxNotice(notice) => sandbox_notice::present(open, notice.as_ref()),
            ViewerOutput::ImageUnavailable {
                generation,
                surface,
                message,
            } => {
                if *generation >= open.frame_generation.get()
                    && *generation >= open.layout.borrow().generation_floor
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
                    open.document_layout.borrow_mut().take();
                    open.image_camera.borrow_mut().suspend();
                    open.image_redraw.stop();
                    raster_motion::retire_raw(open);
                    raster_motion::cancel(open);
                    window.set_status(midnight_core::visible_name(message).as_ref().into());
                }
            }
            ViewerOutput::CopyText(text) => copy(text.clone()),
            ViewerOutput::WebShortcut(shortcut) => web_input::received(open, *shortcut),
            ViewerOutput::WebState {
                generation,
                policy,
                policy_change_allowed,
                zoom,
            } => {
                if *generation >= open.frame_generation.get()
                    && *generation >= open.layout.borrow().generation_floor
                    && !open.layout.borrow().awaiting_frame
                    && open.text_position.borrow().context_available()
                {
                    window.set_web_policy_change_allowed(*policy_change_allowed);
                    web_input::present_policy(open, *policy);
                    window.set_zoom_percent((zoom * 100.0) as f32);
                }
            }
            ViewerOutput::Closed => close(open),
        }
        service.retire(output);
    }
    if !open.closed.get() {
        flush_layout_ack(open);
        flush_document_layout(open);
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
    if media_controls::is_active(&open.window) {
        ancillary.cancel();
        image_motion::bookkeeping(open, event);
        return false;
    }
    // Every native transition must update auxiliary press/gesture bookkeeping,
    // including a release that the browser or retained image consumes below. It
    // runs first: the ancillary observer delivers touchpad reports to the content
    // itself, and a new gesture must cancel older motion before its first report.
    image_motion::bookkeeping(open, event);
    let ancillary_consumed = ancillary.observe(open.window.window(), event, None);
    // Touchpad lines (X11) reach the content's pixel path without wheel easing.
    let touchpad = ancillary.source() == midnight_ui_input::WheelSource::Touchpad;
    if web_input::observe_native(open, event, shortcuts, touchpad) {
        ancillary.cancel_wheel();
        return true;
    }
    if image_motion::route(open, event, touchpad) {
        ancillary.cancel();
        return true;
    }
    ancillary_consumed
}

/// Touchpad momentum continues only on the viewed content: a browser surface gets it
/// natively and every other mode through the content's scroll handler, like the
/// finger motion before it. Toolbar controls and popups get none.
fn bind_kinetic(open: &Rc<OpenWindow>, scroll: &mut midnight_ui_input::NativeScroll) {
    let weak = Rc::downgrade(open);
    scroll.set_kinetic_sink(Box::new(move |event| {
        weak.upgrade().is_some_and(|open| {
            open.window.get_web_mode() && web_input::observe_native(&open, event, true, true)
        })
    }));
    let contains = Rc::downgrade(open);
    let position = Rc::downgrade(open);
    scroll.set_kinetic_surface(midnight_ui_input::KineticSurface {
        contains: Box::new(move |point| {
            contains.upgrade().is_some_and(|open| {
                let window = &open.window;
                let (left, top) = (window.get_content_left(), window.get_content_top());
                point.x >= left
                    && point.y >= top
                    && point.x < left + window.get_content_width()
                    && point.y < top + window.get_content_height()
                    && !media_controls::is_active(window)
                    && !midnight_ui_input::popup_open(window.window())
            })
        }),
        position: Box::new(move || {
            position.upgrade().map_or([None; 2], |open| {
                if open.window.get_web_mode() {
                    web_input::content_position(&open)
                } else {
                    scrolling::momentum_position(
                        open.window.get_horizontal_position(),
                        open.window.get_position(),
                    )
                }
            })
        }),
    });
}

fn bind(open: &Rc<OpenWindow>) {
    bind_image(open);
    web_input::bind(open);
    sandbox_notice::bind(open);
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
        let Some(open) = weak.upgrade() else {
            return;
        };
        // Present accepted choices explicitly; a refused request restores the mode in force.
        let current = open.mode_index.get();
        if index == current {
            return;
        }
        let mode = usize::try_from(index)
            .ok()
            .and_then(|index| open.modes.borrow().get(index).copied());
        if let Some(mode) = mode
            && open.enqueue(ViewerInput::Mode(mode))
        {
            open.mode_index.set(index);
            open.window.set_mode_index(index);
            open.text_position.borrow_mut().changing_content();
            open.window
                .set_position_context(format!("mode:{index}").into());
        } else {
            open.window.set_mode_index(current);
        }
    });
    let native = Rc::new(RefCell::new(midnight_ui_input::ShortcutState::default()));
    let weak = Rc::downgrade(open);
    let native_state = native.clone();
    let mut ancillary_scroll = midnight_ui_input::NativeScroll::new(&open.window);
    bind_kinetic(open, &mut ancillary_scroll);
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
            action.is_some_and(|action| dispatch_keyboard_shortcut(&open, &action))
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
        action.is_some_and(|action| dispatch_keyboard_shortcut(&open, &action))
    });
    bind_pointer(open);
    let weak = Rc::downgrade(open);
    open.window.on_find(move || {
        if let Some(open) = weak.upgrade() {
            dispatch_key(&open, "viewer.find");
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_search(move |backwards| {
        if let Some(open) = weak.upgrade()
            && open.enqueue(ViewerInput::Search {
                query: open.window.get_query().to_string(),
                backwards,
                case_sensitive: open.window.get_case_sensitive(),
                whole_words: open.window.get_whole_words()
                    && open.window.get_whole_words_available(),
            })
        {
            open.text_position.borrow_mut().cancel();
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_clear_search(move || {
        if let Some(open) = weak.upgrade() {
            open.window.set_query("".into());
            if open.enqueue(ViewerInput::Search {
                query: String::new(),
                backwards: false,
                case_sensitive: open.window.get_case_sensitive(),
                whole_words: open.window.get_whole_words()
                    && open.window.get_whole_words_available(),
            }) {
                open.text_position.borrow_mut().cancel();
            }
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

fn dispatch_keyboard_shortcut(open: &OpenWindow, action: &str) -> bool {
    if open.window.get_web_mode() && matches!(action, "viewer.next-file" | "viewer.previous-file") {
        // Only Chromium-arbitrated physical keys may cycle files from browser content.
        // Other custom chords stay page input until generic live-focus arbitration exists.
        return false;
    }
    dispatch_key(open, action)
}

fn dispatch_key(open: &OpenWindow, action: &str) -> bool {
    if media_controls::is_active(&open.window)
        && !matches!(
            action,
            "viewer.close" | "viewer.next-file" | "viewer.previous-file"
        )
    {
        return false;
    }
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
        "viewer.previous-file" => ViewerInput::PreviousFile,
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
                whole_words: open.window.get_whole_words()
                    && open.window.get_whole_words_available(),
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
    if let ViewerInput::Scroll { x, y } = input {
        let scale = open.window.window().scale_factor();
        if open.window.get_tiled() {
            return image_motion::push(open, x / scale, y / scale, false);
        }
        if !raster_motion::owns_raw(open)
            && open.window.get_retained_raster()
            && raster_presentation::push(open, x / scale, y / scale, true)
        {
            return true;
        }
        if !open.window.get_image_mode() && !open.window.get_web_mode() {
            return raster_motion::admit_raw(open, x, y);
        }
    }
    if let ViewerInput::Seek(position) = input
        && open.window.get_tiled()
    {
        return image_motion::seek_end(open, position);
    }
    let changing = matches!(
        input,
        ViewerInput::NextFile | ViewerInput::PreviousFile | ViewerInput::DocumentPage { .. }
    );
    let new_file = matches!(input, ViewerInput::NextFile | ViewerInput::PreviousFile);
    let navigation = matches!(
        input,
        ViewerInput::Scroll { .. }
            | ViewerInput::Seek(_)
            | ViewerInput::Pan(_)
            | ViewerInput::Search { .. }
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
        let Some(open) = weak.upgrade() else {
            return false;
        };
        if media_controls::is_active(&open.window) {
            return false;
        }
        // Logical/pixel input already includes the platform's own momentum.
        raster_motion::cancel_legacy(&open);
        if open.window.get_tiled() {
            image_motion::push(&open, x, y, true);
            return true;
        }
        if !raster_motion::owns_raw(&open)
            && ((open.window.get_retained_raster() && raster_presentation::push(&open, x, y, true))
                || raster_motion::continue_direct(&open, x, y))
        {
            return true;
        }
        let scale = open.window.window().scale_factor();
        let (x, y) = (x * scale, y * scale);
        if !open.window.get_image_mode() && !open.window.get_web_mode() {
            return raster_motion::admit_raw(&open, x, y);
        }
        if open.enqueue(ViewerInput::Scroll { x, y }) {
            open.text_position.borrow_mut().cancel();
        }
        true
    });
    fn position(open: &OpenWindow, horizontal: bool, value: f32) {
        if !raster_motion::owns_raw(open) {
            if raster_presentation::seek(open, horizontal, f64::from(value)) {
                return;
            }
            raster_rebase::cancel_context(open);
            raster_motion::cancel(open);
        }
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
    open.window.on_drag_seek(move |horizontal, value| {
        if let Some(open) = weak.upgrade() {
            position(&open, horizontal, value);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_position_drag_began(move |_| {
        if let Some(open) = weak.upgrade()
            && !raster_motion::owns_raw(&open)
        {
            raster_presentation::begin_seek(&open);
        }
    });
    let weak = Rc::downgrade(open);
    open.window.on_position_drag_ended(move |_| {
        if let Some(open) = weak.upgrade()
            && !raster_motion::owns_raw(&open)
        {
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
    open.window.on_change_document_layout(move |layout| {
        if let Some(open) = weak.upgrade() {
            let layout = match layout {
                0 => DocumentLayout::Paged,
                1 => DocumentLayout::Continuous,
                _ => return,
            };
            change_document_layout(&open, layout);
        }
    });
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
    raster_motion::retire_raw(open);
    raster_rebase::cancel_context(open);
    raster_motion::cancel(open);
    if !open.closed.replace(true) {
        open.media_controls.retire_active();
        {
            let mut layout = open.layout.borrow_mut();
            layout.pending_ack = false;
            layout.awaiting_frame = true;
        }
        if let Some(preparation) = open.preparation.borrow().as_ref() {
            preparation.stop();
        }
        open.prepared_pixels.borrow_mut().take();
        open.session.close();
        open.timer.stop();
        open.image_zoom.borrow_mut().clear();
        open.document_layout.borrow_mut().take();
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
        VIEWER_MODE_VIDEO => "Video",
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
