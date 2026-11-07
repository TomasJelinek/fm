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
#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;

struct Worker {
    runtime: Arc<Runtime>,
    images: Arc<[midnight_image_viewer_host::Provider]>,
    image: Option<crate::image_view::ImageView>,
    closing_image: Option<crate::image_worker::ImageWorker>,
    files: Vec<(PathBuf, Option<crate::capture::Captured>)>,
    current: usize,
    basic: bool,
    disabled_plugins: Arc<[String]>,
    selected_mode: Option<u16>,
    modes: Vec<u16>,
    mode: u16,
    generation: u64,
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
    caption: String,
    status: String,
    dirty: bool,
    frames: crate::interaction::Frames,
}

pub(crate) fn run(
    plugins: (Arc<Runtime>, Arc<[midnight_image_viewer_host::Provider]>),
    launch: ViewerLaunch,
    appearance: ViewerAppearance,
    geometry: ViewerGeometry,
    receiver: Receiver<ViewerInput>,
    output: &Mailbox,
    closed: &Arc<AtomicBool>,
) -> Result<(), String> {
    let (runtime, images) = plugins;
    output.put(ViewerOutput::Status("Preparing file identities…".into()));
    let prepared = crate::capture::prepare(&launch, closed)?;
    let mut disabled_plugins = launch.disabled_plugins.to_vec();
    // Ambiguous identities are disabled on both transports before any code loads.
    disabled_plugins.extend(
        runtime
            .catalog()
            .iter()
            .filter(|native| images.iter().any(|image| image.id == native.id))
            .map(|native| native.id.clone()),
    );
    let mut worker = Worker {
        runtime,
        images,
        image: None,
        closing_image: None,
        files: prepared.files,
        current: prepared.current,
        basic: launch.basic,
        disabled_plugins: disabled_plugins.into(),
        selected_mode: None,
        modes: Vec::new(),
        mode: viewer::MODE_TEXT,
        generation: 0,
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
        appearance,
        geometry: bounded_geometry(geometry),
        image_size: [geometry.width.max(1), geometry.height.max(1)],
        origin: [0.0; 2],
        selection: None,
        length: 0,
        anchor: 0,
        binary: false,
        wrap: false,
        land_at_bottom: false,
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
            Ok(ViewerInput::Close) => break,
            Ok(input) => worker.input(input, output),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Err(_) => {}
        }
        // A bounded batch combines wheel/resize events into a single raster frame.
        for input in receiver.try_iter().take(63) {
            worker.input(input, output);
        }
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
        if worker.dirty {
            worker.dirty = false;
            if let Err(error) = worker.paint(output) {
                worker.error(error, output);
            }
        }
    }
    if let Some(session) = worker.session.take() {
        session.close();
    }
    output.put(ViewerOutput::Closed);
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
    fn error(&mut self, error: String, output: &Mailbox) {
        self.status = error.clone();
        output.put(ViewerOutput::Status(error));
    }

    fn open(&mut self, output: &Mailbox) {
        self.frames.invalidate();
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
            self.error("Closing previous image…".into(), output);
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
            self.error("Closing previous file…".into(), output);
            return;
        }
        self.closing = None;
        if self.files[self.current].1.is_none() {
            self.pending_open = true;
            self.error("Preparing next file identity…".into(), output);
            return;
        }
        self.pending_open = false;
        self.generation = self.generation.saturating_add(1);
        self.page = None;
        self.frames.invalidate();
        self.shaped = None;
        self.selection = None;
        self.origin = [0.0; 2];
        self.anchor = 0;
        self.viewport_request = None;
        self.search_request = None;
        self.search_cursor = Default::default();
        self.found = None;
        let (path, captured) = &self.files[self.current];
        self.caption = format!("{} — Midnight Explorer", path.display());
        let result = (|| {
            let file = captured
                .as_ref()
                .ok_or("File identity is not ready")?
                .as_ref()
                .map_err(Clone::clone)?
                .clone();
            if !self.basic
                && let Some(provider) = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .and_then(|extension| {
                        self.images.iter().find(|provider| {
                            provider.supports_extension(extension)
                                && !self.disabled_plugins.contains(&provider.id)
                        })
                    })
            {
                self.image = Some(crate::image_view::ImageView::new(
                    crate::image_worker::ImageWorker::open(provider.clone(), Arc::new(file))?,
                ));
                self.mode = VIEWER_MODE_IMAGE;
                self.modes = vec![VIEWER_MODE_IMAGE];
                self.binary = false;
                return Ok(None);
            }
            let (plugin, mut mode) = self
                .runtime
                .choose_enabled(path, self.basic, &self.disabled_plugins)
                .ok_or(
                    "No compatible viewer plugin is installed beside the executable in viewers/."
                        .to_owned(),
                )?;
            if !self.basic && plugin.modes.contains(&viewer::MODE_BINARY) {
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
                self.session = session;
                self.error("Opening…".into(), output);
            }
            Err(error) => self.error(error, output),
        }
    }

    fn request_page(&mut self, anchor: u64, backwards: bool, output: &Mailbox) {
        let Some(session) = &self.session else {
            return;
        };
        if let Some(previous) = self.viewport_request.take() {
            session.cancel(previous);
        }
        match session.viewport(messages::Viewport {
            source_anchor: anchor,
            backwards,
            max_bytes: if self.binary { 4096 } else { 12 * 1024 },
            max_lines: 512,
        }) {
            Ok(id) => {
                self.viewport_request = Some(id);
                self.land_at_bottom = backwards;
                self.anchor = anchor;
            }
            Err(error) => self.error(error.to_string(), output),
        }
    }

    fn input(&mut self, input: ViewerInput, output: &Mailbox) {
        if self.image.is_some() && self.image_input(&input) {
            return;
        }
        match input {
            ViewerInput::Resize(geometry) => {
                self.frames.invalidate();
                self.image_size = [geometry.width.max(1), geometry.height.max(1)];
                let geometry = bounded_geometry(geometry);
                let scale_changed = geometry.scale != self.geometry.scale;
                let width_changed = geometry.width != self.geometry.width;
                self.geometry = geometry;
                if scale_changed || (width_changed && self.wrap && self.wrap_available()) {
                    self.reflow(output);
                }
                self.clamp_origin();
                self.dirty = true;
            }
            ViewerInput::Scroll { x, y } if x.is_finite() && y.is_finite() => {
                if let Some(page) = &self.page {
                    let height = self.shaped.as_ref().map_or(0.0, |p| p.height());
                    let max_y = (height - self.geometry.height as f32).max(0.0);
                    let next = self.origin[1] + y;
                    let crossing = navigation::page_direction(self.origin[1], y, max_y);
                    if crossing > 0 && page.after && self.viewport_request.is_none() {
                        self.request_page(page.end, false, output);
                    } else if crossing < 0 && page.before && self.viewport_request.is_none() {
                        self.request_page(page.start, true, output);
                    } else {
                        self.frames.invalidate();
                        self.origin[1] = next.clamp(0.0, max_y);
                        let max_x = self
                            .shaped
                            .as_ref()
                            .map_or(0.0, |p| (p.width() - self.geometry.width as f32).max(0.0));
                        self.origin[0] = (self.origin[0] + x).clamp(0.0, max_x);
                        self.dirty = true;
                    }
                }
            }
            ViewerInput::Seek(position) => {
                let (anchor, backwards) = navigation::viewport(position, self.length);
                self.request_page(anchor, backwards, output)
            }
            ViewerInput::Pan(position) if position.is_finite() => {
                self.frames.invalidate();
                let maximum = self.shaped.as_ref().map_or(0.0, |page| {
                    (page.width() - self.geometry.width as f32).max(0.0)
                });
                self.origin[0] = position.clamp(0.0, 1.0) as f32 * maximum;
                self.dirty = true;
            }
            ViewerInput::NextFile => {
                self.selected_mode = None;
                self.current = (self.current + 1) % self.files.len();
                self.open(output);
            }
            ViewerInput::Mode(mode) if self.modes.contains(&mode) && mode != self.mode => {
                self.selected_mode = Some(mode);
                self.open(output);
            }
            ViewerInput::Wrap(wrap) if self.wrap_available() && self.wrap != wrap => {
                self.wrap = wrap;
                self.reflow(output);
            }
            ViewerInput::Search {
                query,
                backwards,
                case_sensitive,
            } => {
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
                        self.error("Searching…".into(), output);
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
                    && let Ok(index) = page.hit_test(x + self.origin[0], y + self.origin[1])
                {
                    let start = if extend {
                        self.selection.map_or(index, |selection| selection.0)
                    } else {
                        index
                    };
                    self.selection = Some((start, index));
                    self.dirty = true;
                }
            }
            ViewerInput::SelectPage { frame } if self.frames.accepts(frame) => {
                if let Some(page) = &self.shaped {
                    self.selection = Some((0, page.text().len()));
                    self.dirty = true;
                }
            }
            ViewerInput::Copy { frame } if self.frames.accepts(frame) => {
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
            EventKind::Opened(_) => self.request_page(0, false, output),
            EventKind::TextPage(page) if self.viewport_request == Some(id) => {
                self.set_page(Page::text(page, &self.appearance), output)
            }
            EventKind::BinaryPage { page, bytes } if self.viewport_request == Some(id) => {
                self.set_page(Page::binary(page, &bytes), output)
            }
            EventKind::DocumentPage(page) if self.viewport_request == Some(id) => {
                self.set_page(Page::document(page, &self.appearance), output)
            }
            EventKind::SearchMatch(found) if self.search_request == Some(id) => {
                self.anchor = found.source.offset;
                self.search_cursor.found(found.source.offset);
                self.found = Some(found.source.clone());
                self.error(format!("Match at byte {}", found.source.offset), output);
                self.request_page(found.source.offset, false, output);
            }
            EventKind::Failed(failure)
                if id == 1
                    || self.search_request == Some(id)
                    || self.viewport_request == Some(id) =>
            {
                if self.viewport_request == Some(id) {
                    self.viewport_request = None;
                }
                if self.search_request == Some(id) {
                    self.search_request = None;
                }
                self.error(failure.message, output);
            }
            EventKind::Completed | EventKind::Cancelled => {
                if self.viewport_request == Some(id) {
                    self.viewport_request = None;
                }
                if self.search_request == Some(id) {
                    self.search_request = None;
                    if self.status == "Searching…" {
                        self.error("No further match.".into(), output);
                    }
                }
            }
            _ => {}
        }
    }

    fn wrap_available(&self) -> bool {
        matches!(self.mode, viewer::MODE_TEXT | viewer::MODE_SOURCE)
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
            wrap_width: (self.wrap && self.wrap_available()).then_some(self.geometry.width as f32),
            ..TextOptions::default()
        };
        self.renderer
            .layout_with_styles(&page.text, &options, &page.spans, &page.styles)
    }

    fn reflow(&mut self, output: &Mailbox) {
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
                if self.wrap && self.wrap_available() {
                    self.origin[0] = 0.0;
                }
                self.shaped = Some(shaped);
                self.frames.invalidate();
                self.clamp_origin();
                self.dirty = true;
            }
            Err(error) => self.error(format!("Cannot lay out this page: {error:?}"), output),
        }
        self.page = Some(page);
    }

    fn set_page(&mut self, page: Page, output: &Mailbox) {
        self.frames.invalidate();
        self.anchor = page.start;
        match self.layout_page(&page) {
            Ok(shaped) => {
                self.origin = [
                    0.0,
                    if self.land_at_bottom {
                        (shaped.height() - self.geometry.height as f32).max(0.0)
                    } else {
                        0.0
                    },
                ];
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
                self.shaped = Some(shaped);
                self.page = Some(page);
                self.dirty = true;
                self.status = String::new();
            }
            Err(error) => self.error(format!("Cannot lay out this page: {error:?}"), output),
        }
    }

    fn paint(&mut self, output: &Mailbox) -> Result<(), String> {
        if let Some(image) = &mut self.image {
            image.request();
            return Ok(());
        }
        let (Some(page), Some(shaped)) = (&self.page, &self.shaped) else {
            return Ok(());
        };
        let raster = self
            .renderer
            .rasterize(
                shaped,
                Viewport {
                    width: self.geometry.width,
                    height: self.geometry.height,
                    origin: self.origin,
                    background: self.appearance.background,
                    selection_color: self.appearance.selection,
                },
                self.selection.map(|(a, b)| a.min(b)..a.max(b)),
            )
            .map_err(|e| format!("Cannot render page: {e:?}"))?;
        let revision = self.frames.painted()?;
        // BOM bytes and hidden Markdown delimiters do not create a scrollable
        // remainder when the plugin reports that the entire file is present.
        let (start, end) = if !page.before && !page.after {
            (0, self.length)
        } else {
            (page.start, page.end)
        };
        let (position, vertical_extent) = navigation::scroll_metrics(
            start,
            end,
            self.length,
            shaped.height(),
            self.geometry.height as f32,
            self.origin[1],
        );
        output.put(ViewerOutput::Frame(ViewerFrame {
            revision,
            mode: self.mode,
            modes: self.modes.clone(),
            wrap_enabled: self.wrap && self.wrap_available(),
            wrap_available: self.wrap_available(),
            image: None,
            width: raster.width,
            height: raster.height,
            pixels: raster.rgba.into(),
            accessible_text: page.text.clone().into(),
            position,
            vertical_extent,
            horizontal_position: f64::from(
                self.origin[0] / (shaped.width() - self.geometry.width as f32).max(1.0),
            ),
            horizontal_extent: f64::from(
                (self.geometry.width as f32 / shaped.width().max(1.0)).min(1.0),
            ),
            caption: self.caption.clone(),
            status: format!(
                "Bytes {}–{} / {}  {}",
                page.start, page.end, self.length, self.status
            ),
        }));
        Ok(())
    }

    fn clamp_origin(&mut self) {
        if let Some(page) = &self.shaped {
            self.origin[0] =
                self.origin[0].clamp(0.0, (page.width() - self.geometry.width as f32).max(0.0));
            self.origin[1] =
                self.origin[1].clamp(0.0, (page.height() - self.geometry.height as f32).max(0.0));
        }
    }
}
