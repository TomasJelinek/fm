use crate::{
    Mailbox,
    input_queue::ScrollStep,
    navigation,
    page::Page,
    video_actor::{VideoUpdate, VideoView},
};
use midnight_core::*;
use midnight_media_host::video_installation::{VIDEO_PROVIDER_ID, VideoInstallation};
use midnight_plugin_host::{
    EventKind, OPEN_REQUEST_ID, Runtime, Session,
    api::{messages, viewer},
};
use midnight_viewer_render::{TextOptions, TextPage, TextRenderer, Viewport};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, RecvTimeoutError},
    },
    time::Duration,
};

/// Bounded retry for work without a wake signal: paints waiting for shared raster budget.
/// Closing plugin sessions and browser and image actors wake the worker when they end.
const RETRY: Duration = Duration::from_millis(12);
/// Events handled per drain of the active plugin session.
const DRAIN: usize = 32;
/// In-progress status of the one active search; cleared whenever that search ends.
const SEARCHING: &str = "Searching…";
/// Status while an opening waits only for its file identity.
const PREPARING: &str = "Preparing file identity…";

/// Text rows intersecting the viewport. Ctrl+A selects what the window shows, never rows
/// of the loaded page that lie outside it (including retained guard rows).
fn visible_rows(page: &TextPage, origin: f32, geometry: ViewerGeometry) -> (usize, usize) {
    let first = page.line_at_y(origin).map_or(0, |(range, _)| range.start);
    let last = page
        .line_at_y(origin + geometry.height as f32 - 0.01)
        .map_or(page.text().len(), |(range, _)| range.end);
    (first, last)
}

#[path = "worker_chain.rs"]
mod chain;
#[path = "worker_loop.rs"]
mod event_loop;
#[path = "worker_find.rs"]
mod find;
#[path = "worker_image.rs"]
mod image;
#[path = "worker_layout.rs"]
mod layout;
#[path = "worker_palette.rs"]
mod palette;
#[path = "worker_position.rs"]
mod position;
#[path = "prepare_counters.rs"]
mod prepare_counters;
#[path = "worker_raster.rs"]
mod raster;
#[path = "worker_sandbox.rs"]
mod sandbox;
#[path = "worker_scroll_map.rs"]
mod scroll_map;
#[path = "scroll_trace.rs"]
mod scroll_trace;
#[path = "worker_syntax.rs"]
mod syntax;
#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
/// Viewport requests that a test worker without a plugin session would have sent.
#[cfg(test)]
mod unsent {
    use std::cell::{Cell, RefCell};
    thread_local! {
        static REQUESTS: RefCell<Vec<super::messages::Viewport>> = const { RefCell::new(Vec::new()) };
        /// The ID a simulated provider gives the next request, while one admits them.
        static ADMIT: Cell<Option<u64>> = const { Cell::new(None) };
        static ADMITTED: RefCell<Vec<(u64, super::messages::Viewport)>> = const { RefCell::new(Vec::new()) };
        static WHOLE: Cell<bool> = const { Cell::new(false) };
    }
    /// Measurements only: read every request whole on this thread, as before
    /// `Worker::overlap` read only the rows past the shown page.
    pub(super) fn read_whole(whole: bool) {
        WHOLE.set(whole);
    }
    pub(super) fn reads_whole() -> bool {
        WHOLE.get()
    }
    pub(super) fn record(viewport: &super::messages::Viewport) {
        REQUESTS.with_borrow_mut(|requests| requests.push(viewport.clone()));
    }
    /// The requests recorded on this thread since the previous call, oldest first.
    pub(super) fn take() -> Vec<super::messages::Viewport> {
        REQUESTS.take()
    }
    /// Admit later requests on this thread as a plugin session does, numbered from
    /// `first`; `None` stops.
    pub(super) fn admit_from(first: Option<u64>) {
        ADMIT.set(first);
    }
    /// A new request ID, while a simulated provider admits requests.
    pub(super) fn next_id() -> Option<u64> {
        let id = ADMIT.get()?;
        ADMIT.set(id.checked_add(1));
        Some(id)
    }
    /// The ID of `viewport`, just recorded, when a simulated provider admits it.
    pub(super) fn admitted(viewport: &super::messages::Viewport) -> Option<u64> {
        let id = next_id()?;
        ADMITTED.with_borrow_mut(|admitted| admitted.push((id, viewport.clone())));
        Some(id)
    }
    /// The viewport requests admitted since the previous call, oldest first.
    pub(super) fn take_admitted() -> Vec<(u64, super::messages::Viewport)> {
        ADMITTED.take()
    }
}
#[path = "worker_text.rs"]
mod text;
#[path = "worker_web.rs"]
mod web;

#[derive(Clone, Copy, PartialEq, Eq)]
enum MotionOwner {
    Viewport(u64),
    Rebase(u64),
}

/// Owns relative input during a bounded read or displayed-camera handoff.
/// A candidate page retains its request ID when the motion belongs to a rebase.
#[derive(Default)]
struct RelativeMotion {
    owner: Option<MotionOwner>,
    steps: VecDeque<ScrollStep>,
    page: Option<(u64, Page)>,
}

impl RelativeMotion {
    fn adopt(&mut self, owner: MotionOwner) {
        if self.owner != Some(owner) {
            *self = Self {
                owner: Some(owner),
                ..Self::default()
            };
        }
    }

    fn defer(&mut self, owner: MotionOwner, step: ScrollStep) {
        self.adopt(owner);
        self.steps.push_back(step);
    }

    fn stage_page(&mut self, id: u64, page: Page) {
        if !matches!(self.owner, Some(MotionOwner::Rebase(_))) {
            self.adopt(MotionOwner::Viewport(id));
        }
        self.page = Some((id, page));
    }

    fn begin_rebase(&mut self, request: Option<u64>, transition: u64) {
        if request.map(MotionOwner::Viewport) != self.owner {
            *self = Self::default();
        }
        self.owner = Some(MotionOwner::Rebase(transition));
    }

    fn take_page(&mut self, id: u64) -> Option<Page> {
        if self
            .page
            .as_ref()
            .is_some_and(|(request, _)| *request == id)
        {
            self.page.take().map(|(_, page)| page)
        } else {
            None
        }
    }

    fn retire_viewport(&mut self) {
        self.page = None;
        if matches!(self.owner, Some(MotionOwner::Viewport(_))) {
            *self = Self::default();
        }
    }

    fn retire(&mut self, owner: MotionOwner) {
        if self.owner == Some(owner) {
            *self = Self::default();
        }
    }

    fn take_steps(&mut self, owner: MotionOwner) -> VecDeque<ScrollStep> {
        if self.owner == Some(owner) {
            std::mem::take(self).steps
        } else {
            VecDeque::new()
        }
    }

    /// Whether the steps deferred for `owner` move, in all, against a page request in
    /// direction `backwards` (towards the start when true, towards the end when false).
    fn reverses(&self, owner: MotionOwner, backwards: bool) -> bool {
        if self.owner != Some(owner) {
            return false;
        }
        let vertical: f32 = self.steps.iter().map(|step| step.delta[1]).sum();
        if backwards {
            vertical > 0.0
        } else {
            vertical < 0.0
        }
    }

    fn is_idle(&self) -> bool {
        self.owner.is_none()
    }
}

struct Worker {
    wake: Arc<dyn Fn() -> bool + Send + Sync>,
    window_closed: Arc<AtomicBool>,
    preference_state: Arc<crate::preferences::Preferences>,
    preference_revision: u64,
    palette_state: Arc<crate::palette::State>,
    palette_revision: u64,
    palette_ack: Option<(u64, u64, bool)>,
    preferences: Arc<ViewerPreferences>,
    runtime: Arc<Runtime>,
    images: Arc<[midnight_image_viewer_host::Provider]>,
    image_startup: Option<crate::image_startup::Startup>,
    web_providers: Arc<[midnight_web_viewer_host::Provider]>,
    web: Option<web::WebView>,
    closing_web: Option<Box<dyn web::Browser>>,
    web_retirement_error: Option<String>,
    /// The permission Chromium's sandbox lacks where the system refused it.
    sandbox: sandbox::Sandbox,
    image_ids: Arc<std::sync::atomic::AtomicU64>,
    image: Option<crate::image_view::ImageView>,
    closing_image: Option<crate::image_worker::ImageWorker>,
    video_installation: Option<VideoInstallation>,
    video: Option<VideoView>,
    closing_video: Option<VideoView>,
    video_retirement_error: Option<String>,
    /// Native paths or provider labels of the bounded navigation order.
    files: Arc<[PathBuf]>,
    current: usize,
    /// Identity of `files[current]`, retained for mode changes of the same file.
    captured: Option<(usize, crate::capture::Captured)>,
    /// Captures other files on demand; absent for single-source launches.
    prefetch: Option<crate::capture_prefetch::Prefetch>,
    capture_request: Option<(u64, usize)>,
    /// Browser events beyond one drained batch remain queued without another wake.
    web_backlog: bool,
    basic: bool,
    associations: Arc<ViewerAssociations>,
    disabled_plugins: Arc<[String]>,
    selected_mode: Option<u16>,
    modes: Vec<u16>,
    mode: u16,
    generation: u64,
    positions: crate::position_ack::Positions,
    position_page: Option<(u64, Page)>,
    /// The plugin session of the shown file. Its ready signal wakes this worker.
    session: Option<Session>,
    /// A previous session finishing its teardown; never set while `session` is. Its
    /// final ready signal wakes this worker when its dispatcher ends.
    closing: Option<Session>,
    pending_open: bool,
    layout: crate::layout_handshake::Handshake,
    pending_layout: Option<layout::PreparedOpening>,
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
    /// The page edge to continue at once the current reply has completed, because that
    /// reply could not pass the edge the motion wanted to cross (`Continuation::seam`).
    seam: Option<(u64, bool)>,
    /// Text replies still being joined to their request's line budget (`chain`).
    chains: chain::Chains,
    motion: RelativeMotion,
    caption: String,
    status: String,
    dirty: bool,
    frames: crate::interaction::Frames,
}

pub(crate) struct Services {
    pub image_startup: crate::image_startup::Startup,
    pub raster_budget: Arc<crate::raster_budget::Budget>,
    pub runtime: Arc<Runtime>,
    pub images: Arc<[midnight_image_viewer_host::Provider]>,
    pub web: Arc<[midnight_web_viewer_host::Provider]>,
    pub capture: crate::NativeCapture,
    pub wakes: Arc<crate::input_queue::Wakes>,
    pub preferences: Arc<crate::preferences::Preferences>,
    pub palette: Arc<crate::palette::State>,
    pub negotiate_layout: bool,
    /// The service's installed video support, discovered when an opening may route to it.
    pub video: Box<dyn FnOnce() -> Option<VideoInstallation> + Send>,
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
        image_startup,
        raster_budget,
        runtime,
        images,
        web,
        capture,
        wakes,
        preferences: preference_state,
        palette: palette_state,
        negotiate_layout,
        video,
    } = services;
    output.put(ViewerOutput::Status(PREPARING.into()));
    let order = crate::capture::prepare_with(&launch, closed, capture)?;
    let backend = wakes.clone();
    let wake: Arc<dyn Fn() -> bool + Send + Sync> = Arc::new(move || backend.wake());
    let prefetch = order
        .capture
        .map(|capture| {
            crate::capture_prefetch::Prefetch::spawn(
                order.files.len(),
                order.current,
                order.focused.as_ref().ok().cloned(),
                capture,
                closed.clone(),
                wake.clone(),
            )
        })
        .transpose()?;
    let (preference_revision, preferences) = preference_state.snapshot();
    let mut disabled_plugins = launch.disabled_plugins.to_vec();
    let video_installation = if launch.basic
        || launch.generated.is_some()
        || disabled_plugins.iter().any(|id| id == VIDEO_PROVIDER_ID)
    {
        None
    } else {
        video()
    };
    // Ambiguous identities are disabled across transports before any code loads.
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
    if video_installation.is_some() {
        *identities.entry(VIDEO_PROVIDER_ID.to_owned()).or_insert(0) += 1;
    }
    disabled_plugins.extend(
        identities
            .into_iter()
            .filter_map(|(id, count)| (count > 1).then_some(id)),
    );
    let (palette_revision, colors) = palette_state.snapshot();
    let mut appearance = appearance;
    colors.apply(&mut appearance);
    let mut worker = Worker {
        wake,
        window_closed: closed.clone(),
        preference_state,
        preference_revision,
        palette_state,
        palette_revision,
        palette_ack: None,
        preferences: preferences.clone(),
        runtime,
        images,
        web_providers: web,
        image_startup: Some(image_startup),
        web: None,
        closing_web: None,
        web_retirement_error: None,
        sandbox: sandbox::Sandbox::new(sandbox::system_offer()),
        image_ids: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        image: None,
        closing_image: None,
        video_installation,
        video: None,
        closing_video: None,
        video_retirement_error: None,
        files: order.files,
        current: order.current,
        captured: Some((order.current, order.focused)),
        prefetch,
        capture_request: None,
        web_backlog: false,
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
        layout: crate::layout_handshake::Handshake::new(negotiate_layout),
        pending_layout: None,
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
        seam: None,
        chains: Default::default(),
        motion: RelativeMotion::default(),
        caption: String::new(),
        status: String::new(),
        dirty: false,
        frames: Default::default(),
    };
    drop(launch);
    worker.open(output);
    while !closed.load(Ordering::Acquire) {
        // Sessions, the image actor, the browser, file captures and settings changes wake
        // this worker. Only work without such a signal is retried after a bounded delay.
        let first = match worker.retry_delay() {
            Some(delay) => match receiver.recv_timeout(delay) {
                Ok(message) => Some(message),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => break,
            },
            // The worker's own wake handle keeps a sender alive. Closing the window sets
            // `closed` and wakes it through the reserved slot (`Wakes::close`).
            None => match receiver.recv() {
                Ok(message) => Some(message),
                Err(_) => break,
            },
        };
        // A bounded batch combines wheel/resize events into a single raster frame. It can
        // take every admitted input and the wake, so a full channel drains in one turn.
        let batch: Vec<_> = first
            .into_iter()
            .chain(receiver.try_iter().take(crate::input_queue::INPUTS))
            .collect();
        // Taken inputs free their admission slots; the wake slot stays reserved.
        wakes.taken(
            batch
                .iter()
                .filter(|message| !matches!(message, Message::Ready))
                .count(),
        );
        worker.input_batch(batch, closed, &wakes, output);
        if closed.load(Ordering::Acquire) || worker.layout.closed() {
            break;
        }
        worker.sync_preferences(output);
        worker.sync_palette(output);
        worker.resume_open(output);
        worker.drain_session(output);
        if worker.video.is_some() {
            worker.acknowledge_palette(output, None);
        } else {
            worker.announce_content_palette(output);
        }
        worker.poll_image(output);
        worker.poll_web(output);
        worker.poll_sandbox(output);
        worker.poll_video(output);
        if worker.dirty {
            worker.dirty = false;
            if worker.mode != VIEWER_MODE_VIDEO
                && let Err(error) = worker.paint(output)
            {
                worker.error(error, output);
            }
        }
        // Prepared text: strips beyond the window, between inputs (`worker_bands.rs`).
        worker.prepare_strips();
        // Optional indexing starts only after content has been published. It
        // never participates in a viewport or scrollbar acknowledgement.
        worker.prepare_source(output);
        worker.locate_source();
    }
    if let Some(session) = worker.session.take() {
        session.close();
    }
    let video_cleanup = worker.finish_video();
    let web_cleanup = worker.finish_web();
    output.put(ViewerOutput::Closed);
    let raster_budget = worker.raster_budget.clone();
    drop(worker);
    raster_budget.reap();
    video_cleanup.and(web_cleanup)
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
        // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN context_before
        #[cfg(test)]
        if position::observation::enabled(self) {
            position::observation::record(
                self,
                11,
                0,
                self.viewport_request,
                position::observation::Extra {
                    reset_done: Some(false),
                    ..Default::default()
                },
            );
        }
        // SOURCE_COMPLETION_OBSERVATION_V2_END context_before
        self.invalidate_source_target();
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
        // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN context_after
        #[cfg(test)]
        if position::observation::enabled(self) {
            position::observation::record(
                self,
                11,
                0,
                self.viewport_request,
                position::observation::Extra {
                    reset_done: Some(true),
                    ..Default::default()
                },
            );
        }
        // SOURCE_COMPLETION_OBSERVATION_V2_END context_after
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
        } else if self.mode != VIEWER_MODE_VIDEO
            && (previous.wrap != self.preferences.wrap
                || previous.line_numbers != self.preferences.line_numbers
                || previous.text_font != self.preferences.text_font
                || previous.source_font != self.preferences.source_font
                || previous.markdown_font != self.preferences.markdown_font
                || previous.markdown_preset != self.preferences.markdown_preset)
        {
            self.position_context(output);
            self.reflow(output);
        }
    }

    fn error(&mut self, error: String, output: &Mailbox) {
        self.invalidate_layout();
        output.finish_open(Err(error.clone()));
        self.status(error, output);
    }

    fn status(&mut self, status: String, output: &Mailbox) {
        self.status = status.clone();
        output.put(ViewerOutput::Status(status));
    }

    fn open(&mut self, output: &Mailbox) {
        self.clear_sandbox_notice(output);
        if self.video.is_some() || self.mode == VIEWER_MODE_VIDEO {
            output.discard_pending_video();
        }
        if let Some(video) = &self.video {
            output.put(ViewerOutput::MediaRetired {
                session: video.session_id(),
            });
        }
        self.invalidate_layout();
        self.close_source_target();
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
        self.motion = RelativeMotion::default();
        self.frames.invalidate();
        // Nothing the previous file or mode showed can be copied any more.
        self.frames.selection_changed();
        // Start capture while the previous session retires.
        self.request_capture();
        match self.retire_video() {
            Ok(true) => {}
            Ok(false) => {
                self.pending_open = true;
                self.status("Closing previous video…".into(), output);
                return;
            }
            Err(error) => {
                self.pending_open = false;
                self.error(error, output);
                return;
            }
        }
        match self.retire_web() {
            Ok(true) => {}
            Ok(false) => {
                self.pending_open = true;
                self.status("Closing previous browser…".into(), output);
                return;
            }
            Err(error) => {
                self.pending_open = false;
                self.error(error, output);
                return;
            }
        }
        if let Some(mut image) = self.image.take() {
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
        if !self.capture_ready() {
            self.pending_open = true;
            self.status(PREPARING.into(), output);
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
        self.seam = None;
        self.motion = RelativeMotion::default();
        self.search_request = None;
        self.search_cursor = Default::default();
        self.found = None;
        let path = self.files[self.current].clone();
        let captured = self.captured.as_ref().map(|(_, captured)| captured.clone());
        self.caption = format!("{} — Midnight Explorer", path.display());
        let result = (|| {
            let file = captured.ok_or("File identity is not ready")??;
            let opening = self.prepare_opening(&path, file)?;
            self.select_opening(opening, output)
        })();
        if let Err(error) = result {
            self.error(error, output);
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
        let viewport = self.page_viewport(anchor, backwards);
        self.request_viewport(viewport, position, output);
    }

    fn request_viewport(
        &mut self,
        viewport: messages::Viewport,
        position: Option<u64>,
        output: &Mailbox,
    ) {
        self.invalidate_source_target();
        self.cancel_locate();
        self.position_page = None;
        let source_position = self.mode == viewer::MODE_SOURCE && position.is_some();
        if source_position {
            // An absolute seek replaces the owner of any deferred relative input.
            self.motion = RelativeMotion::default();
        } else {
            self.motion.retire_viewport();
        }
        if position.is_none() && self.positions.has_pending() {
            self.position_context(output);
        }
        // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN admission_viewport
        #[cfg(test)]
        let observation_viewport = position::observation::enabled(self).then_some((
            viewport.source_anchor,
            viewport.backwards,
            viewport.max_bytes,
            viewport.max_lines,
        ));
        // SOURCE_COMPLETION_OBSERVATION_V2_END admission_viewport
        self.continuation = None;
        // A new request replaces whatever edge an earlier reply left to continue at.
        self.seam = None;
        // The rows the shown page holds are not read again (`worker_chain.rs`).
        let (held, read) = match self.overlap(&viewport, position) {
            Some((held, read)) => (Some(held), read),
            None => (None, viewport.clone()),
        };
        let Some(session) = &self.session else {
            #[cfg(test)]
            unsent::record(&viewport);
            // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN admission_no_session
            #[cfg(test)]
            if position::observation::enabled(self) {
                position::observation::record(
                    self,
                    2,
                    9,
                    None,
                    position::observation::Extra {
                        viewport: observation_viewport,
                        admitted: Some(false),
                        ..Default::default()
                    },
                );
            }
            // SOURCE_COMPLETION_OBSERVATION_V2_END admission_no_session
            // A simulated provider admits the request as a session would.
            #[cfg(test)]
            if let Some(id) = unsent::admitted(&read) {
                self.viewport_request = None;
                self.viewport_admitted(id, viewport, position, held);
            }
            #[cfg(not(test))]
            let _ = (held, read);
            return;
        };
        if let Some(previous) = self.viewport_request.take() {
            session.cancel(previous);
            // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN admission_cancel_previous
            #[cfg(test)]
            if position::observation::enabled(self) {
                position::observation::record(
                    self,
                    2,
                    1,
                    Some(previous),
                    position::observation::Extra {
                        cancelled_request: Some(previous),
                        viewport: observation_viewport,
                        ..Default::default()
                    },
                );
            }
            // SOURCE_COMPLETION_OBSERVATION_V2_END admission_cancel_previous
        }
        let _ = scroll_trace::begin("request", viewport.source_anchor);
        match session.viewport(read) {
            Ok(id) => self.viewport_admitted(id, viewport, position, held),
            Err(error) if !source_position => {
                self.search_cursor.cancel_candidate();
                self.error(error.to_string(), output);
            }
            Err(_) => {}
        }
        // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN admission_result
        #[cfg(test)]
        if position::observation::enabled(self) {
            position::observation::record(
                self,
                2,
                if self.viewport_request.is_some() {
                    0
                } else {
                    9
                },
                self.viewport_request,
                position::observation::Extra {
                    viewport: observation_viewport,
                    admitted: Some(self.viewport_request.is_some()),
                    admitted_id: self.viewport_request,
                    ..Default::default()
                },
            );
        }
        // SOURCE_COMPLETION_OBSERVATION_V2_END admission_result
    }

    /// Track viewport request `id`, which the plugin session accepted, possibly as the read
    /// of the part after the rows `held` of the shown page.
    fn viewport_admitted(
        &mut self,
        id: u64,
        viewport: messages::Viewport,
        position: Option<u64>,
        held: Option<viewer::TextPage>,
    ) {
        self.viewport_request = Some(id);
        if let Some(sequence) = position {
            self.positions.requested(id, sequence);
            if self.mode == viewer::MODE_SOURCE
                && self.positions.source.is_none()
                && self
                    .positions
                    .target
                    .is_some_and(|target| target.sequence == sequence)
            {
                self.positions.source =
                    Some(crate::position_ack::SourceRecovery::new(viewport.clone()));
            }
        }
        if self.positions.source.is_none() {
            self.land_at_bottom = viewport.backwards;
            self.anchor = viewport.source_anchor;
        }
        self.chain_requested_with(id, &viewport, held);
        self.source_requested(id, viewport);
    }

    fn page_viewport(&mut self, anchor: u64, backwards: bool) -> messages::Viewport {
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
            // The page holds the prepared screens around the window (`worker_bands.rs`).
            requested_geometry.height += self.raster.page_extension(self.geometry);
        }
        let max_lines = crate::viewport::request_line_budget(
            requested_geometry,
            line_height,
            self.length,
            self.mode,
        );
        messages::Viewport {
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
        }
    }

    fn input(&mut self, input: ViewerInput, output: &Mailbox) {
        self.apply_input(input, None, output);
    }

    fn admitted_scroll(&mut self, step: ScrollStep, output: &Mailbox) {
        let [x, y] = step.delta;
        self.apply_input(ViewerInput::Scroll { x, y }, Some(step), output);
    }

    fn apply_input(&mut self, input: ViewerInput, step: Option<ScrollStep>, output: &Mailbox) {
        if matches!(input, ViewerInput::Close) {
            self.close_source_target();
            self.layout.close();
            self.retire_pending_layout();
            self.pending_open = false;
            return;
        }
        if self.layout.closed() {
            return;
        }
        if let ViewerInput::LayoutReady { token, geometry } = input {
            if let Some(opening) = self.take_layout_ready(token, geometry)
                && let Err(error) = self.start_opening(opening, output)
            {
                self.error(error, output);
            }
            return;
        }
        if self.layout.waiting() {
            match &input {
                ViewerInput::Resize(geometry) => {
                    self.image_size = [geometry.width.max(1), geometry.height.max(1)];
                    self.geometry = bounded_geometry(*geometry);
                    return;
                }
                ViewerInput::NextFile
                | ViewerInput::PreviousFile
                | ViewerInput::Mode(_)
                | ViewerInput::RetainedRaster(_) => {}
                _ => return,
            }
        }
        if matches!(input, ViewerInput::InstallSandboxPermission) {
            self.install_sandbox_permission(output);
            return;
        }
        // A window-level choice also applies to text files shown after HTML or images.
        if let ViewerInput::RetainedRaster(enabled) = input {
            self.invalidate_source_target();
            self.raster.enabled = enabled;
            self.source_viewport_parameters_changed();
            if self.mode != VIEWER_MODE_VIDEO && self.web.is_none() && self.image.is_none() {
                self.reset_raster_context();
                self.dirty = true;
            }
            return;
        }
        if self.mode == VIEWER_MODE_VIDEO && self.video_input(&input, output) {
            return;
        }
        if self.web.is_some() && self.web_input(&input, output) {
            return;
        }
        if self.image.is_some() && self.image_input(&input) {
            return;
        }
        match input {
            ViewerInput::Raster { surface, input } => {
                if matches!(&input, ViewerRasterInput::PanTo { .. }) {
                    self.invalidate_source_target();
                }
                self.raster_input(surface, input, output);
            }
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
                self.interrupt_find();
                self.invalidate_source_target();
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
                self.interrupt_find();
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
                self.source_viewport_parameters_changed();
            }
            ViewerInput::Scroll { x, y } if x.is_finite() && y.is_finite() => {
                self.scroll(step.unwrap_or_else(|| ScrollStep::new(x, y)), output);
            }
            ViewerInput::Seek(position) => {
                self.interrupt_find();
                self.position_context(output);
                let (anchor, backwards) = navigation::viewport(position, self.length);
                self.request_page(anchor, backwards, output)
            }
            ViewerInput::Pan(position) if position.is_finite() => {
                self.interrupt_find();
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
            ViewerInput::Search {
                query,
                backwards,
                case_sensitive,
                whole_words,
            } => self.start_find(query, backwards, case_sensitive, whole_words, output),
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
                    self.set_selection(Some((start, index)));
                    self.cancel_raster_rebase();
                    if self.raster.enabled {
                        self.capture_raster_selection();
                    }
                    self.dirty = true;
                }
            }
            ViewerInput::SelectPage { frame } if self.frames.accepts(frame) => {
                if let Some(page) = &self.shaped {
                    // Same rows as the retained path: what the window shows of the page.
                    let rows = visible_rows(page, self.origin[1], self.geometry);
                    self.set_selection(Some(rows));
                    self.cancel_raster_rebase();
                    if self.raster.enabled {
                        self.capture_raster_selection();
                    }
                    self.dirty = true;
                }
            }
            ViewerInput::Copy { frame } if self.frames.accepts_copy(frame) => {
                self.cancel_raster_rebase();
                if self.copy_raster_selection(output) {
                    return;
                }
                self.copy_page_selection(output);
            }
            _ => {}
        }
        self.source_stationary_target();
    }

    fn scroll(&mut self, mut step: ScrollStep, output: &Mailbox) {
        self.interrupt_find();
        self.invalidate_source_target();
        if let Some(transition) = self.pending_raster_rebase() {
            self.motion.defer(MotionOwner::Rebase(transition), step);
            return;
        }
        // A continuation owns motion even when the new step fits the old page.
        if let Some(id) = self.viewport_request
            && (self.continuation.is_some() || self.motion.owner == Some(MotionOwner::Viewport(id)))
        {
            self.motion.defer(MotionOwner::Viewport(id), step);
            return;
        }
        if self.retained_active() {
            self.reset_raster_for_scroll();
        }
        if self.positions.has_pending() && !self.cancel_source_position_for_scroll(output) {
            self.position_context(output);
        }
        let [x, y] = step.delta;
        if x != 0.0 {
            self.positions.horizontal = None;
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
            let max_y = self.shaped.as_ref().map_or(0.0, |p| {
                // A page too short for the window keeps its camera span (`camera_span`).
                let span = self.camera_span(page, p).span;
                if fallback {
                    span
                } else {
                    (p.height() - self.geometry.height as f32).max(span)
                }
            });
            let next = self.origin[1] + y;
            let continuation = self.shaped.as_ref().and_then(|shaped| {
                crate::viewport::Continuation::capture(
                    page,
                    shaped,
                    self.geometry,
                    [self.origin[0] + x, self.origin[1]],
                    y,
                    matches!(self.mode, viewer::MODE_TEXT | viewer::MODE_SOURCE),
                )
                .map(|continuation| continuation.bounded_by(page))
            });
            let continuation = continuation.filter(|continuation| {
                let progresses = if continuation.backwards {
                    continuation.anchor < page.end
                } else {
                    continuation.anchor > page.start
                };
                if fallback && !progresses {
                    // Keep sub-row wheel distance without repeatedly fetching
                    // the same byte-limited fragment.
                    self.raster.fallback_scroll = (next - next.clamp(0.0, max_y)).clamp(-1e7, 1e7);
                    false
                } else {
                    true
                }
            });
            if let Some(continuation) = continuation {
                if let Some(id) = self.viewport_request {
                    step.delta[1] = y;
                    self.motion.defer(MotionOwner::Viewport(id), step);
                    return;
                }
                self.request_page(continuation.anchor, continuation.backwards, output);
                if self.viewport_request.is_some() || self.session.is_none() {
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

    fn resume_motion(&mut self, owner: MotionOwner, output: &Mailbox) {
        let steps = self.motion.take_steps(owner);
        if steps.is_empty() {
            return;
        }
        for step in steps {
            self.scroll(step, output);
        }
        self.source_stationary_target();
    }

    fn event(&mut self, id: u64, event: EventKind, output: &Mailbox) {
        // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN event_entry
        #[cfg(test)]
        if position::observation::enabled(self) {
            position::observation::record(
                self,
                3,
                0,
                Some(id),
                position::observation::Extra::event(&event),
            );
        }
        // SOURCE_COMPLETION_OBSERVATION_V2_END event_entry
        if self.source_event(id, &event, output) {
            // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN event_source_handled
            #[cfg(test)]
            if position::observation::enabled(self) {
                position::observation::record(
                    self,
                    4,
                    0,
                    Some(id),
                    position::observation::Extra::event(&event),
                );
            }
            // SOURCE_COMPLETION_OBSERVATION_V2_END event_source_handled
            return;
        }
        // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN event_normal_route
        #[cfg(test)]
        if position::observation::enabled(self) {
            position::observation::record(
                self,
                5,
                0,
                Some(id),
                position::observation::Extra::event(&event),
            );
        }
        // SOURCE_COMPLETION_OBSERVATION_V2_END event_normal_route
        if matches!(
            &event,
            EventKind::Completed | EventKind::Cancelled | EventKind::Failed(_)
        ) && self.retire_superseded_source_position(id, output)
        {
            return;
        }
        match event {
            EventKind::TextPosition(position) => self.located(id, &position),
            EventKind::Failed(failure)
                if self
                    .text
                    .locate_request
                    .is_some_and(|(request, _)| request == id) =>
            {
                let start = self.text.locate_request.take().map(|(_, start)| start);
                // Older plugins reject the optional request. This is not a
                // document failure and must not overwrite the usable viewport.
                // Session-fatal events are handled by session_events.
                if failure.code == midnight_plugin_host::api::abi::UNSUPPORTED {
                    self.text.locate_unsupported = true;
                    self.text.unavailable = true;
                } else if failure.code == midnight_plugin_host::api::abi::QUEUE_FULL {
                    self.retry_locate();
                } else {
                    self.text.located_start = start;
                    self.text.unavailable = true;
                    self.status(failure.message, output);
                }
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
                self.search_request = None;
                self.found_match(found.source, output);
            }
            EventKind::Failed(failure) if self.reports_failure(id) => {
                // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN native_failed
                #[cfg(test)]
                if position::observation::enabled(self) {
                    position::observation::record(
                        self,
                        6,
                        10,
                        Some(id),
                        position::observation::Extra {
                            event_kind: Some(4),
                            failure_code: Some(failure.code),
                            ..Default::default()
                        },
                    );
                }
                // SOURCE_COMPLETION_OBSERVATION_V2_END native_failed
                if self.viewport_request == Some(id) {
                    self.search_cursor.cancel_candidate();
                    if let crate::position_ack::Completion::Failed(sequence) =
                        self.retire_failed_viewport(id, &failure.message, output)
                    {
                        if self.mode == viewer::MODE_SOURCE {
                            self.source_position_unavailable(
                                sequence,
                                Some(failure.message),
                                output,
                            );
                            return;
                        }
                        self.position_context(output);
                        self.dirty = true;
                    }
                }
                if self.search_request == Some(id) {
                    self.search_request = None;
                }
                self.error(failure.message, output);
            }
            EventKind::Completed | EventKind::Cancelled => {
                if self.viewport_request == Some(id) && self.search_cursor.candidate.is_some() {
                    // A context page either advances to a new request or settles its candidate.
                    // Terminal-without-page must not leave Find permanently marked as searching.
                    self.search_cursor.cancel_candidate();
                    self.error(
                        "The requested Find context was not completed.".into(),
                        output,
                    );
                }
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
                    // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN terminal_before_page
                    #[cfg(test)]
                    if position::observation::enabled(self) {
                        position::observation::record(
                            self,
                            6,
                            if matches!(event, EventKind::Cancelled) {
                                1
                            } else {
                                0
                            },
                            Some(id),
                            position::observation::Extra::event(&event),
                        );
                    }
                    // SOURCE_COMPLETION_OBSERVATION_V2_END terminal_before_page
                    // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN terminal_page_preconditions
                    #[cfg(test)]
                    let observation_page = position::observation::enabled(self).then(|| {
                        (
                            self.positions.has_pending(),
                            self.position_page.as_ref().map(|(page_id, _)| *page_id),
                            matches!(event, EventKind::Completed),
                        )
                    });
                    // SOURCE_COMPLETION_OBSERVATION_V2_END terminal_page_preconditions
                    let mut source_reread = None;
                    let source_position =
                        self.mode == viewer::MODE_SOURCE && self.positions.has_pending();
                    let mut completed = matches!(event, EventKind::Completed);
                    if self.positions.has_pending() {
                        let page = self.position_page.take();
                        completed = completed
                            && page.is_some_and(|(page_id, page)| {
                                if page_id != id {
                                    return false;
                                }
                                if self.positions.source.is_none() {
                                    return self.set_page(page, output);
                                }
                                match self.resolve_source_position(page, output) {
                                    position::SourcePosition::Ready => true,
                                    position::SourcePosition::Reload { sequence, viewport } => {
                                        source_reread = Some((sequence, viewport));
                                        false
                                    }
                                    position::SourcePosition::Unavailable => false,
                                }
                            });
                    } else {
                        if let Some(page) = self.motion.take_page(id) {
                            completed = completed && self.set_page(page, output);
                        } else if self.continuation.is_some() {
                            completed = false;
                        }
                        self.continuation = None;
                    }
                    // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN terminal_page_decision
                    #[cfg(test)]
                    if position::observation::enabled(self)
                        && let Some((pending, staged_id, terminal_completed)) = observation_page
                    {
                        let page_match = staged_id.map(|page_id| page_id == id);
                        let reason = if !terminal_completed {
                            1
                        } else if pending && staged_id.is_none() {
                            2
                        } else if pending && page_match == Some(false) {
                            3
                        } else if pending && !completed {
                            4
                        } else {
                            0
                        };
                        position::observation::record(
                            self,
                            7,
                            reason,
                            Some(id),
                            position::observation::Extra {
                                page_match,
                                page_applied: (terminal_completed
                                    && pending
                                    && page_match == Some(true))
                                .then_some(completed),
                                ..position::observation::Extra::event(&event)
                            },
                        );
                    }
                    // SOURCE_COMPLETION_OBSERVATION_V2_END terminal_page_decision
                    if let Some((sequence, viewport)) = source_reread {
                        self.viewport_request = None;
                        self.source_foreground_finished(id, false);
                        self.request_viewport(viewport, Some(sequence), output);
                        if self.viewport_request.is_some() {
                            return;
                        }
                    }
                    if completed && self.positions.has_pending() && self.positions.source.is_none()
                    {
                        match self.refine_position() {
                            position::LoadedPosition::Ready => {}
                            position::LoadedPosition::ReloadViewport { sequence, viewport } => {
                                self.viewport_request = None;
                                self.request_viewport(viewport, Some(sequence), output);
                                if self.viewport_request.is_some() {
                                    return;
                                }
                                completed = false;
                            }
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
                                // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN correction_not_admitted
                                #[cfg(test)]
                                if position::observation::enabled(self) {
                                    position::observation::record(
                                        self,
                                        2,
                                        9,
                                        Some(id),
                                        position::observation::Extra {
                                            correction: Some((anchor, backwards)),
                                            admitted: Some(false),
                                            ..Default::default()
                                        },
                                    );
                                }
                                // SOURCE_COMPLETION_OBSERVATION_V2_END correction_not_admitted
                                completed = false;
                            }
                            position::LoadedPosition::Unavailable => completed = false,
                        }
                    }
                    // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN terminal_before_complete
                    #[cfg(test)]
                    if position::observation::enabled(self) {
                        position::observation::record(
                            self,
                            9,
                            if self.positions.has_pending() && !self.positions.uncompleted_page() {
                                11
                            } else if matches!(event, EventKind::Cancelled) {
                                1
                            } else {
                                0
                            },
                            Some(id),
                            position::observation::Extra {
                                completion_success: Some(completed),
                                ..position::observation::Extra::event(&event)
                            },
                        );
                    }
                    // SOURCE_COMPLETION_OBSERVATION_V2_END terminal_before_complete
                    let acknowledgement = self.positions.complete(id, completed);
                    // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN terminal_after_complete
                    #[cfg(test)]
                    if position::observation::enabled(self) {
                        position::observation::record(
                            self,
                            10,
                            0,
                            Some(id),
                            position::observation::Extra {
                                completion: Some(match &acknowledgement {
                                    crate::position_ack::Completion::Unrelated => 0,
                                    crate::position_ack::Completion::Ready => 1,
                                    crate::position_ack::Completion::Failed(_) => 2,
                                }),
                                ..position::observation::Extra::event(&event)
                            },
                        );
                    }
                    // SOURCE_COMPLETION_OBSERVATION_V2_END terminal_after_complete
                    let raster_failed = self
                        .abort_raster_refill("The requested text page was not completed.", output);
                    self.viewport_request = None;
                    if !completed {
                        self.search_cursor.waiting = None;
                    }
                    self.source_foreground_finished(id, completed);
                    self.finish_raster_rebase(completed, output);
                    match acknowledgement {
                        crate::position_ack::Completion::Ready => self.dirty = true,
                        crate::position_ack::Completion::Failed(sequence) => {
                            if source_position {
                                self.source_position_unavailable(sequence, None, output);
                                return;
                            }
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
                    if let Some((anchor, backwards)) = self.seam.take()
                        && completed
                        && self.viewport_request.is_none()
                        // Motion that came meanwhile and turned back wins: it is replayed
                        // on this page below, instead of a jump past the edge.
                        && !self.motion.reverses(MotionOwner::Viewport(id), backwards)
                    {
                        // Continue at the edge the reply could not pass, once: this
                        // request carries no continuation, so a reply that still cannot
                        // advance ends there. Deferred motion then waits for its page.
                        self.request_page(anchor, backwards, output);
                    }
                    let viewport_motion = MotionOwner::Viewport(id);
                    if !source_position && completed {
                        self.resume_motion(viewport_motion, output);
                    } else {
                        self.motion.retire(viewport_motion);
                    }
                    if completed && self.retained_active() && !raster_failed {
                        self.resume_raster_target(output);
                    }
                    if self.viewport_request.is_none()
                        && let Some(found) = self.search_cursor.waiting.take()
                    {
                        self.continuation = None;
                        self.motion = RelativeMotion::default();
                        self.position_context(output);
                        self.found_match(found, output);
                    }
                }
                if self.search_request == Some(id) {
                    self.search_request = None;
                    if self.status == SEARCHING {
                        self.status("No further match.".into(), output);
                    }
                }
            }
            // Includes late events of requests no longer tracked, such as cancelled ones.
            // A session-fatal failure among them never gets here (see `session_events`).
            _ => {}
        }
    }

    /// The viewport request `id` ended without a page: drop its raster handoff or refill
    /// and fail the scrollbar position it was serving.
    fn viewport_failed(&mut self, id: u64, message: &str, output: &Mailbox) {
        if matches!(
            self.retire_failed_viewport(id, message, output),
            crate::position_ack::Completion::Failed(_),
        ) {
            self.position_context(output);
            self.dirty = true;
        }
    }

    fn retire_failed_viewport(
        &mut self,
        id: u64,
        message: &str,
        output: &Mailbox,
    ) -> crate::position_ack::Completion {
        self.abort_raster_rebase(message, output);
        self.abort_raster_refill(message, output);
        self.position_page = None;
        let completion = self.positions.complete(id, false);
        self.viewport_request = None;
        self.source_foreground_finished(id, false);
        self.motion.retire_viewport();
        self.continuation = None;
        self.seam = None;
        completion
    }

    fn wrap_available(&self) -> bool {
        matches!(self.mode, viewer::MODE_TEXT | viewer::MODE_SOURCE)
    }

    fn received_page(&mut self, id: u64, page: Page, output: &Mailbox) {
        if self.search_cursor.candidate.is_some() && self.find_page(&page, output) {
            return;
        }
        if self.positions.has_pending() {
            if self.mode == viewer::MODE_SOURCE && self.positions.has_deferred() {
                return;
            }
            self.position_page = Some((id, page));
            self.positions.loaded(id);
        } else if self.continuation.is_some()
            || self.motion.owner == Some(MotionOwner::Viewport(id))
        {
            self.motion.stage_page(id, page);
        } else {
            self.set_page(page, output);
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
        prepare_counters::layout(page.text.len());
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
        self.source_viewport_parameters_changed();
    }

    fn set_page(&mut self, page: Page, output: &Mailbox) -> bool {
        match self.shape_page(&page) {
            Ok(shaped) => {
                self.seam = self.continuation.as_ref().and_then(|continuation| {
                    continuation
                        .seam(&page)
                        .map(|anchor| (anchor, continuation.backwards))
                });
                self.publish_page(page, shaped, None, output);
                true
            }
            Err(error) => {
                self.frames.invalidate();
                self.anchor = page.start;
                self.error(format!("Cannot lay out this page: {error:?}"), output);
                false
            }
        }
    }

    fn shape_page(&mut self, page: &Page) -> Result<TextPage, midnight_viewer_render::LayoutError> {
        let started = scroll_trace::begin("layout", page.start);
        let layout = self.layout_page(page);
        scroll_trace::end("layout", started);
        layout
    }

    fn publish_page(
        &mut self,
        page: Page,
        shaped: TextPage,
        resolved_position: Option<(Option<ViewerScrollMap>, f32)>,
        output: &Mailbox,
    ) {
        // The new page derives its selection from the find result or the retained
        // selection's source range. Copy keeps accepting frames of the previous page
        // only if this one selects the same source.
        let shown = self.selected();
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
        let restored = self
            .continuation
            .take()
            .and_then(|continuation| continuation.restore(&page, &shaped));
        self.origin = if let Some((_, origin)) = &resolved_position {
            [0.0, *origin]
        } else {
            restored.unwrap_or([
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
                        .and_then(|anchor| shaped.caret_geometry(anchor.text_offset as usize).ok())
                        .map_or(0.0, |caret| caret.y0 as f32)
                } else {
                    0.0
                },
            ])
        };
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
        self.scroll_map = if let Some((map, _)) = resolved_position {
            map
        } else {
            scroll_map::build(
                &page,
                &shaped,
                self.length,
                !continuing_layout,
                self.geometry.height,
            )
        };
        self.shaped = Some(shaped);
        self.apply_horizontal_position();
        if self.search_cursor.reveal {
            self.search_cursor.reveal = false;
            self.center_find();
        }
        self.clamp_origin();
        let previous = self
            .page
            .as_ref()
            .filter(|_| self.mode == viewer::MODE_SOURCE);
        self.text.page(previous, &page);
        self.page = Some(page);
        if let Some(id) = self.viewport_request {
            self.source_page_loaded(id);
        }
        self.raster_loaded_with_handoff(
            raster_continuing,
            restored,
            raster_local,
            same_raster_context,
            continuing_layout,
            output,
        );
        if !self.selected().same(shown) {
            self.frames.selection_changed();
        }
        self.dirty = true;
        if self.status != SEARCHING
            || (self.search_request.is_none() && self.search_cursor.waiting.is_none())
        {
            self.status.clear();
        }
    }

    /// Replace the selection on the shown page. A different one cannot be copied by
    /// naming a frame or retained surface that showed the previous one.
    fn set_selection(&mut self, selection: Option<(usize, usize)>) {
        if selection != self.selection {
            self.selection = selection;
            self.frames.selection_changed();
        }
    }

    /// Copy the shown page's selection, as frames without retained raster show it.
    fn copy_page_selection(&self, output: &Mailbox) {
        if let (Some(page), Some((a, b))) = (&self.shaped, self.selection)
            && let Ok(text) = page.selected_text(a.min(b)..a.max(b))
        {
            output.put(ViewerOutput::CopyText(text.to_owned()));
        }
    }

    /// The selection's source range on the shown page.
    fn selected(&self) -> crate::interaction::Selected {
        let anchors = self.page.as_ref().map_or(&[][..], |page| &page.anchors[..]);
        crate::interaction::Selected::on(anchors, self.selection)
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
            let (width, height) = (self.geometry.width, self.geometry.height);
            prepare_counters::raster(u64::from(width) * u64::from(height));
            // Painted in place and moved into the frame without another copy.
            let mut pixels = raster::shared_pixels(width, height)?;
            let rgba = raster::unique(&mut pixels)?;
            self.renderer
                .rasterize_into(
                    shaped,
                    Viewport {
                        width,
                        height,
                        origin: [self.origin[0] - margin, self.origin[1]],
                        background: self.appearance.background,
                        selection_color: self.appearance.selection,
                        selection_text: self.appearance.selection_text,
                    },
                    self.selection.map(|(a, b)| a.min(b)..a.max(b)),
                    rgba,
                )
                .map_err(|e| format!("Cannot render page: {e:?}"))?;
            if self.mode == viewer::MODE_SOURCE && self.preferences.line_numbers {
                let mut surface = midnight_viewer_render::RasterMut::new(width, height, rgba)
                    .map_err(|e| format!("Cannot render page: {e:?}"))?;
                text::paint_gutter(
                    &mut self.renderer,
                    &mut self.text,
                    &self.appearance,
                    self.geometry,
                    self.origin,
                    margin,
                    &mut surface,
                )?;
            }
            (width, height, pixels, None)
        };
        scroll_trace::end("paint", started);
        let revision = self.frames.painted()?;
        if let Some(info) = &raster_info {
            self.frames.painted_surface(info.surface.0);
        }
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
        let frame = Box::new(ViewerFrame {
            generation: self.positions.generation,
            media_control: None,
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
            // A reference-count increment: every frame of a page shares its text.
            accessible_text: page.text.clone(),
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
        });
        self.acknowledge_palette(output, Some(frame.revision));
        output.put(ViewerOutput::Frame(frame));
        Ok(())
    }

    fn clamp_origin(&mut self) {
        if let Some(shaped) = &self.shaped {
            // A page too short for the window keeps its camera span (`camera_span`).
            let span = self
                .page
                .as_ref()
                .map_or(0.0, |page| self.camera_span(page, shaped).span);
            let maximum = (shaped.height() - self.geometry.height as f32).max(span);
            self.origin[0] =
                self.origin[0].clamp(0.0, (shaped.width() - self.text_width()).max(0.0));
            self.origin[1] = self.origin[1].clamp(0.0, maximum);
        }
    }
}
