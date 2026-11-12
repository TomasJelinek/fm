use crate::{Mailbox, navigation, page::Page};
use midnight_core::*;
use midnight_plugin_host::{
    EventKind, Runtime, Session,
    api::{messages, viewer},
};
use midnight_viewer_render::{TextOptions, TextPage, TextRenderer, Viewport};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::Receiver,
    },
    time::Duration,
};

#[path = "worker_image.rs"]
mod image;
#[path = "worker_position.rs"]
mod position;
#[path = "worker_raster.rs"]
mod raster;
#[path = "worker_scroll_map.rs"]
mod scroll_map;
#[path = "scroll_trace.rs"]
mod scroll_trace;
#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
#[path = "worker_text.rs"]
mod text;
#[path = "worker_web.rs"]
mod web;

struct Worker {
    wake: Arc<dyn Fn() -> bool + Send + Sync>,
    preference_state: Arc<crate::preferences::Preferences>,
    preference_revision: u64,
    preferences: Arc<ViewerPreferences>,
    runtime: Arc<Runtime>,
    images: Arc<[midnight_image_viewer_host::Provider]>,
    web_providers: Arc<[midnight_web_viewer_host::Provider]>,
    web: Option<web::WebView>,
    closing_web: Option<midnight_web_viewer_host::Session>,
    image_ids: Arc<std::sync::atomic::AtomicU64>,
    image: Option<crate::image_view::ImageView>,
    closing_image: Option<crate::image_worker::ImageWorker>,
    files: Vec<(PathBuf, Option<crate::capture::Captured>)>,
    current: usize,
    basic: bool,
    associations: Arc<ViewerAssociations>,
    disabled_plugins: Arc<[String]>,
    selected_mode: Option<u16>,
    modes: Vec<u16>,
    mode: u16,
    generation: u64,
    positions: crate::position_ack::Positions,
    position_page: Option<(u64, Page)>,
    session: Option<Session>,
    closing: Option<Session>,
    pending_open: bool,
    viewport_request: Option<u64>,
    search_request: Option<u64>,
    search_cursor: crate::search_cursor::SearchCursor,
    found: Option<viewer::SourceRange>,
    renderer: TextRenderer,
    page: Option<Page>,
    shaped: Option<TextPage>,
    scroll_map: Option<ViewerScrollMap>,
    text: text::Presentation,
    raster: raster::Retained,
    raster_budget: Arc<crate::raster_budget::Budget>,
    raster_owner: u64,
    appearance: ViewerAppearance,
    geometry: ViewerGeometry,
    image_size: [u32; 2],
    origin: [f32; 2],
    selection: Option<(usize, usize)>,
    length: u64,
    anchor: u64,
    binary: bool,
    wrap: bool,
    land_at_bottom: bool,
    continuation: Option<crate::viewport::Continuation>,
    queued_scroll: [f32; 2],
    caption: String,
    status: String,
    dirty: bool,
    frames: crate::interaction::Frames,
}

pub(crate) struct Services {
    pub raster_budget: Arc<crate::raster_budget::Budget>,
    pub runtime: Arc<Runtime>,
    pub images: Arc<[midnight_image_viewer_host::Provider]>,
    pub web: Arc<[midnight_web_viewer_host::Provider]>,
    pub capture: crate::NativeCapture,
    pub wake: Arc<dyn Fn() -> bool + Send + Sync>,
    pub preferences: Arc<crate::preferences::Preferences>,
}

pub(crate) use crate::input_queue::Message;

pub(crate) fn run(
    services: Services,
    launch: ViewerLaunch,
    appearance: ViewerAppearance,
    geometry: ViewerGeometry,
    receiver: Receiver<Message>,
    output: &Mailbox,
    closed: &Arc<AtomicBool>,
) -> Result<(), String> {
    let Services {
        raster_budget,
        runtime,
        images,
        web,
        capture,
        wake,
        preferences: preference_state,
    } = services;
    output.put(ViewerOutput::Status("Preparing file identities…".into()));
    let prepared = crate::capture::prepare_with(&launch, closed, capture)?;
    let (preference_revision, preferences) = preference_state.snapshot();
    let mut disabled_plugins = launch.disabled_plugins.to_vec();
    // Ambiguous identities are disabled on both transports before any code loads.
    let mut identities = std::collections::HashMap::new();
    for id in runtime
        .catalog()
        .iter()
        .map(|provider| &provider.id)
        .chain(images.iter().map(|provider| &provider.id))
        .chain(web.iter().map(|provider| &provider.id))
    {
        *identities.entry(id.clone()).or_insert(0usize) += 1;
    }
    disabled_plugins.extend(
        identities
            .into_iter()
            .filter_map(|(id, count)| (count > 1).then_some(id)),
    );
    let mut worker = Worker {
        wake,
        preference_state,
        preference_revision,
        preferences: preferences.clone(),
        runtime,
        images,
        web_providers: web,
        web: None,
        closing_web: None,
        image_ids: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        image: None,
        closing_image: None,
        files: prepared.files,
        current: prepared.current,
        basic: launch.basic,
        associations: launch.associations.clone(),
        disabled_plugins: disabled_plugins.into(),
        selected_mode: None,
        modes: Vec::new(),
        mode: viewer::MODE_TEXT,
        generation: 0,
        positions: Default::default(),
        position_page: None,
        session: None,
        closing: None,
        pending_open: false,
        viewport_request: None,
        search_request: None,
        search_cursor: Default::default(),
        found: None,
        renderer: TextRenderer::default(),
        page: None,
        shaped: None,
        scroll_map: None,
        text: Default::default(),
        raster: Default::default(),
        raster_owner: raster_budget.owner(),
        raster_budget,
        appearance,
        geometry: bounded_geometry(geometry),
        image_size: [geometry.width.max(1), geometry.height.max(1)],
        origin: [0.0; 2],
        selection: None,
        length: 0,
        anchor: 0,
        binary: false,
        wrap: preferences.wrap,
        land_at_bottom: false,
        continuation: None,
        queued_scroll: [0.0; 2],
        caption: String::new(),
        status: String::new(),
        dirty: false,
        frames: Default::default(),
    };
    drop(launch);
    worker.open(output);
    let mut capture_finished = false;
    while !closed.load(Ordering::Acquire) {
        match receiver.recv_timeout(Duration::from_millis(12)) {
            Ok(message) => match message.into_input() {
                Some(ViewerInput::Close) => break,
                Some(input) => worker.input(input, output),
                None => {}
            },
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => {}
        }
        // A bounded batch combines wheel/resize events into a single raster frame.
        for input in receiver.try_iter().take(63) {
            if let Some(input) = input.into_input() {
                worker.input(input, output);
            }
        }
        worker.sync_preferences(output);
        for _ in 0..16 {
            if capture_finished {
                break;
            }
            match prepared.ready.try_recv() {
                Ok((index, captured)) => worker.files[index].1 = Some(captured),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    capture_finished = true;
                    for (_, file) in &mut worker.files {
                        if file.is_none() {
                            *file = Some(Err("File identity preparation stopped.".into()));
                        }
                    }
                    break;
                }
            }
        }
        if worker.pending_open
            && worker.files[worker.current].1.is_some()
            && worker.closing.as_ref().is_none_or(Session::is_closed)
            && worker
                .closing_web
                .as_ref()
                .is_none_or(midnight_web_viewer_host::Session::finished)
            && worker
                .closing_image
                .as_ref()
                .is_none_or(crate::image_worker::ImageWorker::finished)
        {
            worker.closing = None;
            worker.open(output);
        }
        if let Some(session) = &worker.session {
            let events = session.drain(32);
            for event in events {
                if event.generation == worker.generation {
                    worker.event(event.request_id, event.kind, output);
                }
            }
        }
        worker.poll_image(output);
        worker.poll_web(output);
        if worker.dirty {
            worker.dirty = false;
            if let Err(error) = worker.paint(output) {
                worker.error(error, output);
            }
        }
        // Optional indexing starts only after content has been published. It
        // never participates in a viewport or scrollbar acknowledgement.
        worker.locate_source();
    }
    if let Some(session) = worker.session.take() {
        session.close();
    }
    if let Some(web) = worker.web.take() {
        web.session.close();
    }
    output.put(ViewerOutput::Closed);
    let raster_budget = worker.raster_budget.clone();
    drop(worker);
    raster_budget.reap();
    Ok(())
}

fn bounded_geometry(mut geometry: ViewerGeometry) -> ViewerGeometry {
    if !geometry.scale.is_finite() {
        geometry.scale = 1.0;
    }
    geometry.scale = geometry.scale.clamp(0.25, 8.0);
    geometry.width = geometry.width.clamp(1, 4096);
    geometry.height = geometry.height.clamp(1, 4096);
    // Cap the surface independently from the decoded-page/cache budget.
    let pixels = f64::from(geometry.width) * f64::from(geometry.height);
    if pixels > 4_194_304.0 {
        let reduction = (4_194_304.0 / pixels).sqrt();
        geometry.width = (f64::from(geometry.width) * reduction) as u32;
        geometry.height = (f64::from(geometry.height) * reduction) as u32;
        geometry.scale *= reduction as f32;
    }
    geometry
}

impl Worker {
    fn position_context(&mut self, output: &Mailbox) {
        self.reset_raster_context();
        self.position_page = None;
        if self.positions.has_pending()
            && let Some(id) = self.viewport_request.take()
            && let Some(session) = &self.session
        {
            session.cancel(id);
        }
        if let Err(error) = self.positions.reset() {
            self.error(error, output);
        }
        self.dirty = true; // Publish this context even when a search finds no match.
    }

    fn apply_font(&mut self) {
        let kind = match self.mode {
            viewer::MODE_SOURCE => ViewerFontKind::Source,
            viewer::MODE_DOCUMENT => ViewerFontKind::Markdown,
            _ => ViewerFontKind::Text,
        };
        let font = self.preferences.font(kind);
        self.appearance.family.clone_from(&font.family);
        self.appearance.font_size = f32::from(font.size).min(256.0 / self.geometry.scale);
    }

    fn sync_preferences(&mut self, output: &Mailbox) {
        let (revision, preferences) = self.preference_state.snapshot();
        if revision == self.preference_revision {
            return;
        }
        let previous = std::mem::replace(&mut self.preferences, preferences);
        self.preference_revision = revision;
        self.wrap = self.preferences.wrap;
        self.apply_font();
        if let Some(image) = &mut self.image {
            // PDF geometry belongs to accepted view commands, not restored settings.
            // In particular, a late settings load cannot replace Fit or round zoom.
            if !image.document
                && previous.image_zoom != self.preferences.image_zoom
                && let Some(view) = &mut image.geometry
            {
                view.restore_zoom(self.preferences.image_zoom);
                image.changed();
                self.frames.invalidate();
                self.dirty = true;
            }
        } else if previous.wrap != self.preferences.wrap
            || previous.line_numbers != self.preferences.line_numbers
            || previous.text_font != self.preferences.text_font
            || previous.source_font != self.preferences.source_font
            || previous.markdown_font != self.preferences.markdown_font
            || previous.markdown_preset != self.preferences.markdown_preset
        {
            self.position_context(output);
            self.reflow(output);
        }
    }

    fn error(&mut self, error: String, output: &Mailbox) {
        output.finish_open(Err(error.clone()));
        self.status(error, output);
    }

    fn status(&mut self, status: String, output: &Mailbox) {
        self.status = status.clone();
        output.put(ViewerOutput::Status(status));
    }

    fn open(&mut self, output: &Mailbox) {
        self.cancel_locate();
        self.position_context(output);
        // A pending close/capture must not repaint old-file content under the
        // replacement file's new scrollbar generation.
        self.page = None;
        self.shaped = None;
        self.scroll_map = None;
        self.text = Default::default();
        self.selection = None;
        self.continuation = None;
        self.queued_scroll = [0.0; 2];
        self.frames.invalidate();
        if let Some(web) = self.web.take() {
            web.session.close();
            self.closing_web = Some(web.session);
        }
        if self
            .closing_web
            .as_ref()
            .is_some_and(|session| !session.finished())
        {
            self.pending_open = true;
            self.status("Closing previous browser…".into(), output);
            return;
        }
        self.closing_web = None;
        if let Some(image) = self.image.take() {
            image.worker.cancel();
            self.closing_image = Some(image.worker);
        }
        if self
            .closing_image
            .as_ref()
            .is_some_and(|image| !image.finished())
        {
            self.pending_open = true;
            self.status("Closing previous image…".into(), output);
            return;
        }
        self.closing_image = None;
        if let Some(session) = self.session.take() {
            session.close();
            self.closing = Some(session);
        }
        if self
            .closing
            .as_ref()
            .is_some_and(|session| !session.is_closed())
        {
            self.pending_open = true;
            self.status("Closing previous file…".into(), output);
            return;
        }
        self.closing = None;
        if self.files[self.current].1.is_none() {
            self.pending_open = true;
            self.status("Preparing file identity…".into(), output);
            return;
        }
        self.pending_open = false;
        self.generation = self.generation.saturating_add(1);
        self.page = None;
        self.frames.invalidate();
        self.shaped = None;
        self.scroll_map = None;
        self.selection = None;
        self.origin = [0.0; 2];
        self.anchor = 0;
        self.viewport_request = None;
        self.continuation = None;
        self.queued_scroll = [0.0; 2];
        self.search_request = None;
        self.search_cursor = Default::default();
        self.found = None;
        let (path, captured) = self.files[self.current].clone();
        self.caption = format!("{} — Midnight Explorer", path.display());
        let result = (|| {
            let file = captured
                .as_ref()
                .ok_or("File identity is not ready")?
                .as_ref()
                .map_err(Clone::clone)?
                .clone();
            let route = crate::routing::choose(
                self.runtime.catalog(),
                self.images
                    .iter()
                    .map(|provider| crate::routing::ProcessCandidate {
                        id: &provider.id,
                        extensions: &provider.extensions,
                    })
                    .chain(self.web_providers.iter().map(|provider| {
                        crate::routing::ProcessCandidate {
                            id: &provider.id,
                            extensions: &provider.extensions,
                        }
                    })),
                &path,
                self.basic,
                &self.disabled_plugins,
                &self.associations,
            )
            .ok_or_else(|| {
                "No compatible viewer plugin is installed beside the executable in viewers/."
                    .to_owned()
            })?;
            if let crate::routing::Route::Process { index } = route {
                if index >= self.images.len() {
                    self.open_web(self.web_providers[index - self.images.len()].clone(), file)?;
                    return Ok(None);
                }
                self.image = Some(crate::image_view::ImageView::new(
                    crate::image_worker::ImageWorker::open(
                        self.images[index].clone(),
                        Arc::new(file),
                        self.wake.clone(),
                    )?,
                    self.image_ids.clone(),
                ));
                self.mode = VIEWER_MODE_IMAGE;
                self.modes = vec![VIEWER_MODE_IMAGE];
                self.binary = false;
                return Ok(None);
            }
            let crate::routing::Route::Native {
                index,
                mut mode,
                detect_binary,
            } = route
            else {
                return Err("Unsupported viewer route.".into());
            };
            let plugin = &self.runtime.catalog()[index];
            if detect_binary {
                mode = file.preferred_basic_mode().map_err(|e| e.to_string())?;
            }
            if let Some(selected) = self
                .selected_mode
                .filter(|mode| plugin.modes.contains(mode))
            {
                mode = selected;
            }
            self.mode = mode;
            self.modes = plugin.modes.clone();
            self.binary = mode == viewer::MODE_BINARY;
            self.length = file.length();
            self.runtime
                .open(&plugin.id, file, self.generation, mode)
                .map(Some)
                .map_err(|e| e.to_string())
        })();
        match result {
            Ok(session) => {
                self.apply_font();
                if let Some(session) = &session {
                    session.notify_on_ready(self.wake.clone());
                }
                self.session = session;
                self.status("Opening…".into(), output);
            }
            Err(error) => self.error(error, output),
        }
    }

    fn request_page(&mut self, anchor: u64, backwards: bool, output: &Mailbox) {
        self.request_page_for(anchor, backwards, None, output);
    }

    fn request_page_for(
        &mut self,
        anchor: u64,
        backwards: bool,
        position: Option<u64>,
        output: &Mailbox,
    ) {
        self.cancel_locate();
        self.position_page = None;
        if position.is_none() && self.positions.has_pending() {
            self.position_context(output);
        }
        let options = TextOptions {
            family: if self.binary || self.appearance.family.is_empty() {
                "monospace"
            } else {
                &self.appearance.family
            },
            size: self.appearance.font_size,
            literal_family: !self.binary
                && !self.appearance.family.is_empty()
                && !midnight_viewer_render::is_generic_family(&self.appearance.family),
            scale: self.geometry.scale,
            line_height: self.line_height(),
            ..TextOptions::default()
        };
        let line_height = self
            .renderer
            .layout_with_options("M", &options, &[])
            .map_or(self.appearance.font_size * self.geometry.scale, |page| {
                page.height()
            });
        let mut requested_geometry = self.geometry;
        if self.raster.enabled {
            requested_geometry.height += raster::guard(self.geometry, false)[1] * 2;
        }
        let max_lines = crate::viewport::request_line_budget(
            requested_geometry,
            line_height,
            self.length,
            self.mode,
        );
        self.continuation = None;
        let Some(session) = &self.session else {
            return;
        };
        if let Some(previous) = self.viewport_request.take() {
            session.cancel(previous);
        }
        let _ = scroll_trace::begin("request", anchor);
        match session.viewport(messages::Viewport {
            source_anchor: anchor,
            backwards,
            max_bytes: if self.binary {
                4096
            } else if self.mode == viewer::MODE_DOCUMENT {
                64 * 1024
            } else {
                12 * 1024
            },
            max_lines,
        }) {
            Ok(id) => {
                self.viewport_request = Some(id);
                if let Some(sequence) = position {
                    self.positions.requested(id, sequence);
                }
                self.land_at_bottom = backwards;
                self.anchor = anchor;
            }
            Err(error) => self.error(error.to_string(), output),
        }
    }

    fn input(&mut self, input: ViewerInput, output: &Mailbox) {
        if self.web.is_some() && self.web_input(&input, output) {
            return;
        }
        if self.image.is_some() && self.image_input(&input) {
            return;
        }
        match input {
            ViewerInput::RetainedRaster(enabled) => {
                self.raster.enabled = enabled;
                self.reset_raster_context();
                self.dirty = true;
            }
            ViewerInput::Raster { surface, input } => self.raster_input(surface, input, output),
            ViewerInput::RasterRebase { transition, origin } => {
                self.raster_rebase_input(transition, origin, output)
            }
            ViewerInput::Position {
                generation,
                sequence,
                horizontal,
                position,
            } => {
                if !self
                    .positions
                    .accept(generation, sequence, horizontal, position)
                {
                    return;
                }
                if horizontal {
                    self.reset_raster_context();
                    self.positions.horizontal(sequence, position);
                    self.apply_horizontal_position();
                    self.frames.invalidate();
                    self.dirty = true;
                } else {
                    self.seek_position(sequence, position, output);
                }
            }
            ViewerInput::Resize(geometry) => {
                self.position_context(output);
                self.frames.invalidate();
                self.image_size = [geometry.width.max(1), geometry.height.max(1)];
                let geometry = bounded_geometry(geometry);
                let scale_changed = geometry.scale != self.geometry.scale;
                let width_changed = geometry.width != self.geometry.width;
                let height_changed = geometry.height != self.geometry.height;
                self.geometry = geometry;
                if scale_changed {
                    self.apply_font();
                }
                if scale_changed
                    || (width_changed
                        && ((self.wrap && self.wrap_available())
                            || self.mode == viewer::MODE_DOCUMENT))
                {
                    self.reflow(output);
                }
                self.clamp_origin();
                if (height_changed || scale_changed)
                    && self.viewport_request.is_none()
                    && let (Some(page), Some(shaped)) = (&self.page, &self.shaped)
                    && page.after
                    && crate::viewport::available_height(page, shaped, self.geometry.height as f32)
                        - self.origin[1]
                        < self.geometry.height as f32
                    && let Some(continuation) =
                        crate::viewport::Continuation::refill(page, shaped, self.origin)
                {
                    self.request_page(continuation.anchor, false, output);
                    self.continuation = Some(continuation);
                }
                self.dirty = true;
            }
            ViewerInput::Scroll { x, y } if x.is_finite() && y.is_finite() => {
                self.cancel_raster_rebase();
                if self.retained_active() {
                    self.reset_raster_context();
                }
                if self.positions.has_pending() {
                    self.position_context(output);
                }
                if x != 0.0 {
                    self.positions.horizontal = None;
                }
                // A continuation owns the requested source-relative camera until
                // its reply arrives. Even a reversal that fits the old fragment
                // belongs after that movement, rather than to the obsolete page.
                if self.viewport_request.is_some() && self.continuation.is_some() {
                    self.queued_scroll[0] = (self.queued_scroll[0] + x).clamp(-1e7, 1e7);
                    self.queued_scroll[1] = (self.queued_scroll[1] + y).clamp(-1e7, 1e7);
                    return;
                }
                let fallback = self.raster.enabled && !self.retained_active();
                let y = if fallback {
                    let pending = std::mem::take(&mut self.raster.fallback_scroll);
                    y + if y != 0.0 && pending.signum() != y.signum() {
                        0.0
                    } else {
                        pending
                    }
                } else {
                    y
                };
                if let Some(page) = &self.page {
                    let height = self.shaped.as_ref().map_or(0.0, |p| {
                        if fallback {
                            crate::viewport::available_height(page, p, self.geometry.height as f32)
                        } else {
                            p.height()
                        }
                    });
                    let max_y = (height - self.geometry.height as f32).max(0.0);
                    let next = self.origin[1] + y;
                    let continuation = self.shaped.as_ref().and_then(|shaped| {
                        crate::viewport::Continuation::capture(
                            page,
                            shaped,
                            self.geometry,
                            [self.origin[0] + x, self.origin[1]],
                            y,
                        )
                    });
                    let continuation = continuation.filter(|next| {
                        let progresses = if next.backwards {
                            next.anchor < page.end
                        } else {
                            next.anchor > page.start
                        };
                        if fallback && !progresses {
                            // Keep sub-row wheel distance without repeatedly
                            // fetching the same byte-limited fragment.
                            self.raster.fallback_scroll = (self.origin[1] + y
                                - (self.origin[1] + y).clamp(0.0, max_y))
                            .clamp(-1e7, 1e7);
                            false
                        } else {
                            true
                        }
                    });
                    if let Some(continuation) = continuation {
                        if self.viewport_request.is_some() {
                            self.queued_scroll[0] = (self.queued_scroll[0] + x).clamp(-1e7, 1e7);
                            self.queued_scroll[1] = (self.queued_scroll[1] + y).clamp(-1e7, 1e7);
                        } else {
                            self.request_page(continuation.anchor, continuation.backwards, output);
                            self.continuation = Some(continuation);
                        }
                    } else {
                        self.frames.invalidate();
                        self.origin[1] = next.clamp(0.0, max_y);
                        let max_x = self
                            .shaped
                            .as_ref()
                            .map_or(0.0, |p| (p.width() - self.text_width()).max(0.0));
                        self.origin[0] = (self.origin[0] + x).clamp(0.0, max_x);
                        self.dirty = true;
                    }
                }
            }
            ViewerInput::Seek(position) => {
                self.position_context(output);
                let (anchor, backwards) = navigation::viewport(position, self.length);
                self.request_page(anchor, backwards, output)
            }
            ViewerInput::Pan(position) if position.is_finite() => {
                self.position_context(output);
                self.frames.invalidate();
                let maximum = self
                    .shaped
                    .as_ref()
                    .map_or(0.0, |page| (page.width() - self.text_width()).max(0.0));
                self.origin[0] = position.clamp(0.0, 1.0) as f32 * maximum;
                self.dirty = true;
            }
            ViewerInput::NextFile | ViewerInput::PreviousFile => {
                let Some(current) = navigation::adjacent_file(
                    self.current,
                    self.files.len(),
                    matches!(input, ViewerInput::PreviousFile),
                ) else {
                    return;
                };
                self.selected_mode = None;
                self.current = current;
                self.open(output);
            }
            ViewerInput::Mode(mode) if self.modes.contains(&mode) && mode != self.mode => {
                self.selected_mode = Some(mode);
                self.open(output);
            }
            ViewerInput::Wrap(wrap) if self.wrap_available() && self.wrap != wrap => {
                self.position_context(output);
                self.wrap = wrap;
                self.reflow(output);
            }
            ViewerInput::Search {
                query,
                backwards,
                case_sensitive,
            } => {
                self.position_context(output);
                let Some(session) = &self.session else {
                    return;
                };
                if let Some(previous) = self.search_request.take() {
                    session.cancel(previous);
                }
                match session.search(messages::Search {
                    source_anchor: self
                        .search_cursor
                        .begin(&query, case_sensitive, backwards, self.anchor)
                        .min(self.length),
                    query,
                    backwards,
                    case_sensitive,
                }) {
                    Ok(id) => {
                        self.search_request = Some(id);
                        self.status("Searching…".into(), output);
                    }
                    Err(error) => self.error(error.to_string(), output),
                }
            }
            ViewerInput::Select {
                frame,
                x,
                y,
                extend,
            } if self.frames.accepts(frame) => {
                if let Some(page) = &self.shaped
                    && let Ok(index) = page.hit_test(
                        (x - self.text_margin()).max(0.0) + self.origin[0],
                        y + self.origin[1],
                    )
                {
                    let start = if extend {
                        self.selection.map_or(index, |selection| selection.0)
                    } else {
                        index
                    };
                    self.selection = Some((start, index));
                    self.cancel_raster_rebase();
                    if self.raster.enabled {
                        self.capture_raster_selection();
                    }
                    self.dirty = true;
                }
            }
            ViewerInput::SelectPage { frame } if self.frames.accepts(frame) => {
                if let Some(page) = &self.shaped {
                    self.selection = Some((0, page.text().len()));
                    self.cancel_raster_rebase();
                    if self.raster.enabled {
                        self.capture_raster_selection();
                    }
                    self.dirty = true;
                }
            }
            ViewerInput::Copy { frame } if self.frames.accepts(frame) => {
                self.cancel_raster_rebase();
                if self.copy_raster_selection(output) {
                    return;
                }
                if let (Some(page), Some((a, b))) = (&self.shaped, self.selection)
                    && let Ok(text) = page.selected_text(a.min(b)..a.max(b))
                {
                    output.put(ViewerOutput::CopyText(text.to_owned()));
                }
            }
            _ => {}
        }
    }

    fn event(&mut self, id: u64, event: EventKind, output: &Mailbox) {
        match event {
            EventKind::TextPosition(position) => self.located(id, &position),
            EventKind::Failed(failure)
                if self
                    .text
                    .locate_request
                    .is_some_and(|(request, _)| request == id) =>
            {
                self.text.locate_request = None;
                // Older plugins reject the optional request. This is not a
                // document failure and must not overwrite the usable viewport.
                self.text.unavailable = failure.code != midnight_plugin_host::api::abi::QUEUE_FULL;
                self.dirty = true;
            }
            EventKind::Opened(_) => self.request_page(0, false, output),
            EventKind::TextPage(page) if self.viewport_request == Some(id) => {
                self.received_page(id, Page::text(page, &self.appearance), output);
            }
            EventKind::BinaryPage { page, bytes } if self.viewport_request == Some(id) => {
                self.received_page(id, Page::binary(page, &bytes), output);
            }
            EventKind::DocumentPage(page) if self.viewport_request == Some(id) => {
                self.received_page(id, Page::document(page, &self.appearance), output);
            }
            EventKind::SearchMatch(found) if self.search_request == Some(id) => {
                self.anchor = found.source.offset;
                self.search_cursor.found(found.source.offset);
                self.found = Some(found.source.clone());
                self.status(format!("Match at byte {}", found.source.offset), output);
                self.request_page(found.source.offset, false, output);
            }
            EventKind::Failed(failure)
                if id == 1
                    || self.search_request == Some(id)
                    || self.viewport_request == Some(id) =>
            {
                if self.viewport_request == Some(id) {
                    self.abort_raster_rebase(&failure.message, output);
                    self.abort_raster_refill(&failure.message, output);
                    self.position_page = None;
                    let failed = self.positions.complete(id, false)
                        == crate::position_ack::Completion::Failed;
                    self.viewport_request = None;
                    if failed {
                        self.position_context(output);
                        self.dirty = true;
                    }
                    self.queued_scroll = [0.0; 2];
                    self.continuation = None;
                }
                if self.search_request == Some(id) {
                    self.search_request = None;
                }
                self.error(failure.message, output);
            }
            EventKind::Completed | EventKind::Cancelled => {
                if self
                    .text
                    .locate_request
                    .is_some_and(|(request, _)| request == id)
                {
                    self.text.located_start =
                        self.text.locate_request.take().map(|(_, start)| start);
                    self.text.unavailable = self.text.coordinate.is_none();
                    self.dirty = true;
                }
                if self.viewport_request == Some(id) {
                    let mut completed = matches!(event, EventKind::Completed);
                    if self.positions.has_pending() {
                        completed = completed
                            && self.position_page.take().is_some_and(|(page_id, page)| {
                                page_id == id && self.set_page(page, output)
                            });
                    }
                    if completed && self.positions.has_pending() {
                        match self.refine_position() {
                            position::LoadedPosition::Ready => {}
                            position::LoadedPosition::Reload {
                                sequence,
                                anchor,
                                backwards,
                            } => {
                                self.viewport_request = None;
                                self.request_page_for(anchor, backwards, Some(sequence), output);
                                if self.viewport_request.is_some() {
                                    return;
                                }
                                completed = false;
                            }
                            position::LoadedPosition::Unavailable => completed = false,
                        }
                    }
                    let acknowledgement = self.positions.complete(id, completed);
                    let raster_failed = self
                        .abort_raster_refill("The requested text page was not completed.", output);
                    self.viewport_request = None;
                    self.finish_raster_rebase(completed, output);
                    match acknowledgement {
                        crate::position_ack::Completion::Ready => self.dirty = true,
                        crate::position_ack::Completion::Failed => {
                            self.position_context(output);
                            self.dirty = true;
                            self.error("The requested viewport was not completed.".into(), output);
                        }
                        crate::position_ack::Completion::Unrelated => {}
                    }
                    if acknowledgement == crate::position_ack::Completion::Ready
                        && let Some((sequence, position)) = self.positions.take_deferred()
                    {
                        self.seek_position(sequence, position, output);
                    }
                    let [x, y] = std::mem::replace(&mut self.queued_scroll, [0.0; 2]);
                    if matches!(event, EventKind::Completed) && (x != 0.0 || y != 0.0) {
                        self.input(ViewerInput::Scroll { x, y }, output);
                    }
                    if matches!(event, EventKind::Completed)
                        && self.retained_active()
                        && !raster_failed
                    {
                        self.resume_raster_target(output);
                    }
                }
                if self.search_request == Some(id) {
                    self.search_request = None;
                    if self.status == "Searching…" {
                        self.status("No further match.".into(), output);
                    }
                }
            }
            _ => {}
        }
    }

    fn wrap_available(&self) -> bool {
        matches!(self.mode, viewer::MODE_TEXT | viewer::MODE_SOURCE)
    }

    fn received_page(&mut self, id: u64, page: Page, output: &Mailbox) {
        if self.positions.has_pending() {
            self.position_page = Some((id, page));
            self.positions.loaded(id);
        } else {
            let continuing = self.continuation.is_some();
            if self.set_page(page, output) && continuing {
                // A TextPage can precede its terminal event. Apply the latest
                // wheel intent before publishing this first replacement frame.
                let [x, y] = std::mem::replace(&mut self.queued_scroll, [0.0; 2]);
                if x != 0.0 || y != 0.0 {
                    self.input(ViewerInput::Scroll { x, y }, output);
                }
                if self.retained_active() {
                    self.resume_raster_target(output);
                }
            }
        }
    }

    fn layout_page(
        &mut self,
        page: &Page,
    ) -> Result<TextPage, midnight_viewer_render::LayoutError> {
        let options = TextOptions {
            family: if self.binary || self.appearance.family.is_empty() {
                "monospace"
            } else {
                &self.appearance.family
            },
            size: self.appearance.font_size,
            scale: self.geometry.scale,
            foreground: self.appearance.foreground,
            literal_family: !self.binary
                && !self.appearance.family.is_empty()
                && !midnight_viewer_render::is_generic_family(&self.appearance.family),
            wrap_width: ((self.wrap && self.wrap_available())
                || self.mode == viewer::MODE_DOCUMENT)
                .then_some(self.text_width()),
            line_height: self.line_height(),
            ..TextOptions::default()
        };
        page.layout(
            &mut self.renderer,
            &options,
            self.preferences.markdown_preset == MarkdownPreset::GitHub,
            self.appearance.background,
        )
    }

    fn reflow(&mut self, output: &Mailbox) {
        self.reset_raster_context();
        self.text.gutter = None;
        let Some(page) = self.page.take() else {
            return;
        };
        let anchor = self.shaped.as_ref().and_then(|shaped| {
            let index = shaped.hit_test(self.origin[0], self.origin[1] + 0.5).ok()?;
            let caret = shaped.caret_geometry(index).ok()?;
            Some((index, (self.origin[1] - caret.y0 as f32).max(0.0)))
        });
        match self.layout_page(&page) {
            Ok(shaped) => {
                if let Some((index, offset)) = anchor
                    && let Ok(caret) = shaped.caret_geometry(index)
                {
                    self.origin[1] = caret.y0 as f32 + offset;
                }
                if (self.wrap && self.wrap_available()) || self.mode == viewer::MODE_DOCUMENT {
                    self.origin[0] = 0.0;
                }
                self.scroll_map =
                    scroll_map::build(&page, &shaped, self.length, true, self.geometry.height);
                self.shaped = Some(shaped);
                self.frames.invalidate();
                self.clamp_origin();
                self.dirty = true;
            }
            Err(error) => self.error(format!("Cannot lay out this page: {error:?}"), output),
        }
        self.page = Some(page);
    }

    fn set_page(&mut self, page: Page, output: &Mailbox) -> bool {
        let continuing_layout = self.continuation.is_some();
        let raster_continuing = self.raster.continuing.take();
        let same_raster_context = !matches!(self.mode, viewer::MODE_TEXT | viewer::MODE_SOURCE)
            || self
                .page
                .as_ref()
                .is_none_or(|previous| crate::viewport::retains_start_context(previous, &page));
        self.frames.invalidate();
        let requested_anchor = self.anchor;
        self.anchor = page.start;
        let started = scroll_trace::begin("layout", page.start);
        let layout = self.layout_page(&page);
        scroll_trace::end("layout", started);
        match layout {
            Ok(shaped) => {
                let restored = self
                    .continuation
                    .take()
                    .and_then(|continuation| continuation.restore(&page, &shaped));
                self.origin = restored.unwrap_or([
                    0.0,
                    if self.land_at_bottom {
                        (shaped.height() - self.geometry.height as f32).max(0.0)
                    } else if self.mode == viewer::MODE_DOCUMENT && requested_anchor > page.start {
                        // Semantic pages keep complete paragraphs/tables around
                        // a request. Place its source anchor, not the block start,
                        // at the viewport edge without changing line wrapping.
                        page.anchors
                            .iter()
                            .find(|anchor| anchor.source_offset >= requested_anchor)
                            .and_then(|anchor| {
                                shaped.caret_geometry(anchor.text_offset as usize).ok()
                            })
                            .map_or(0.0, |caret| caret.y0 as f32)
                    } else {
                        0.0
                    },
                ]);
                let raster_local = self.origin;
                self.selection = self.found.as_ref().and_then(|found| {
                    let first = page
                        .anchors
                        .iter()
                        .find(|a| a.source_offset == found.offset)?;
                    let last = page
                        .anchors
                        .iter()
                        .find(|a| a.source_offset == found.offset + found.length)?;
                    Some((first.text_offset as usize, last.text_offset as usize))
                });
                self.scroll_map = scroll_map::build(
                    &page,
                    &shaped,
                    self.length,
                    !continuing_layout,
                    self.geometry.height,
                );
                self.shaped = Some(shaped);
                self.apply_horizontal_position();
                self.clamp_origin();
                let previous = self
                    .page
                    .as_ref()
                    .filter(|_| self.mode == viewer::MODE_SOURCE);
                self.text.page(previous, &page);
                self.page = Some(page);
                self.raster_loaded_with_handoff(
                    raster_continuing,
                    restored,
                    raster_local,
                    same_raster_context,
                    output,
                );
                self.dirty = true;
                self.status = String::new();
                true
            }
            Err(error) => {
                self.raster.continuing = raster_continuing;
                self.error(format!("Cannot lay out this page: {error:?}"), output);
                false
            }
        }
    }

    fn apply_horizontal_position(&mut self) {
        if let Some(position) = self.positions.horizontal {
            let maximum = self
                .shaped
                .as_ref()
                .map_or(0.0, |page| (page.width() - self.text_width()).max(0.0));
            self.origin[0] = position as f32 * maximum;
        }
    }

    fn paint(&mut self, output: &Mailbox) -> Result<(), String> {
        if self.raster_rebase_waiting() {
            return Ok(());
        }
        if self.image.is_some() {
            return self.paint_image(output);
        }
        // Other-axis input can dirty the view while this page is still awaiting
        // its terminal event. Never stamp that uncompleted page with an old ack.
        if self.positions.uncompleted_page() {
            return Ok(());
        }
        let started = scroll_trace::begin("paint", self.anchor);
        self.prepare_gutter()?;
        let retained = if self.retained_active() {
            let Some(snapshot) = self.retained_pixels()? else {
                return Ok(());
            };
            Some(snapshot)
        } else {
            None
        };
        let margin = self.text_margin();
        let (Some(page), Some(shaped)) = (&self.page, &self.shaped) else {
            return Ok(());
        };
        let (width, height, pixels, raster_info) = if let Some(snapshot) = retained {
            (
                snapshot.width,
                snapshot.height,
                snapshot.pixels,
                Some(snapshot.info),
            )
        } else {
            let mut raster = self
                .renderer
                .rasterize(
                    shaped,
                    Viewport {
                        width: self.geometry.width,
                        height: self.geometry.height,
                        origin: [self.origin[0] - margin, self.origin[1]],
                        background: self.appearance.background,
                        selection_color: self.appearance.selection,
                    },
                    self.selection.map(|(a, b)| a.min(b)..a.max(b)),
                )
                .map_err(|e| format!("Cannot render page: {e:?}"))?;
            if self.mode == viewer::MODE_SOURCE && self.preferences.line_numbers {
                text::paint_gutter(
                    &mut self.renderer,
                    &self.text,
                    &self.appearance,
                    self.geometry,
                    self.origin,
                    margin,
                    &mut raster,
                )?;
            }
            (raster.width, raster.height, Arc::from(raster.rgba), None)
        };
        scroll_trace::end("paint", started);
        let revision = self.frames.painted()?;
        // BOM bytes and hidden Markdown delimiters do not create a scrollable
        // remainder when the plugin reports that the entire file is present.
        let (start, end) = if !page.before && !page.after {
            (0, self.length)
        } else {
            (page.start, page.end)
        };
        let (position, vertical_extent) = self.position_metrics().unwrap_or_else(|| {
            navigation::scroll_metrics(
                start,
                end,
                self.length,
                shaped.height(),
                self.geometry.height as f32,
                self.origin[1],
            )
        });
        output.put(ViewerOutput::Frame(Box::new(ViewerFrame {
            generation: self.positions.generation,
            position_sequence: self.positions.acknowledged,
            revision,
            mode: self.mode,
            modes: self.modes.clone(),
            wrap_enabled: self.wrap && self.wrap_available(),
            wrap_available: self.wrap_available(),
            image: None,
            raster: raster_info,
            raster_rebase: self.raster_rebase_ack(),
            width,
            height,
            pixels,
            tiles: None,
            accessible_text: page.text.clone().into(),
            position,
            vertical_extent,
            horizontal_position: f64::from(
                self.origin[0] / (shaped.width() - self.text_width()).max(1.0),
            ),
            horizontal_extent: f64::from((self.text_width() / shaped.width().max(1.0)).min(1.0)),
            caption: self.caption.clone(),
            status: format!(
                "{}Bytes {}–{} / {}  {}",
                self.source_status(page, shaped),
                page.start,
                page.end,
                self.length,
                self.status
            ),
        })));
        Ok(())
    }

    fn clamp_origin(&mut self) {
        if let Some(page) = &self.shaped {
            self.origin[0] = self.origin[0].clamp(0.0, (page.width() - self.text_width()).max(0.0));
            self.origin[1] =
                self.origin[1].clamp(0.0, (page.height() - self.geometry.height as f32).max(0.0));
        }
    }
}
