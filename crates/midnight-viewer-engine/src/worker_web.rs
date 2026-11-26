//! Interactive browser routing. All source, policy and process work stays on workers.
use super::*;
use midnight_core::viewer_web as api;
#[cfg(test)]
#[path = "worker_web_tests.rs"]
mod tests;

const WEB_BATCH: usize = 32;
/// Shown once per browser session when it stopped accepting input.
const BROWSER_CLOSED: &str = "Browser closed; reopen the document.";
/// The longest page title the status line shows, in characters.
const MAX_TITLE_CHARS: usize = 160;
/// A web address a script of an offline page tried to open without the user following a
/// link to it.
const SCRIPT_ADDRESS_REFUSED: &str = "A script of this offline page tried to open a web address; only links you follow open in your browser.";

/// The window's HTML choices that outlive one document: whether scripts run in the next
/// page it opens (JavaScript is on until the user turns it off in this window), and the
/// policy a Reload keeps for the file it reloads, and for no other.
pub(super) struct Choices {
    pub scripts: bool,
    pub reload: Option<(PathBuf, api::Policy)>,
}

impl Default for Choices {
    fn default() -> Self {
        Self {
            scripts: true,
            reload: None,
        }
    }
}

/// The browser session operations the worker uses: `midnight_web_viewer_host::Session`,
/// or a scripted session in tests. The worker owns the session and calls these on its own
/// thread; none waits for the browser. Dropping a host session joins its actor thread, so
/// while the window stays open the worker drops a closing one only after `finished` (see
/// `Worker::open`).
pub(super) trait Browser: Send {
    /// Queue one command. Apart from the host-only Open, ResourceReply and FrameAck
    /// commands, which the worker never sends, the host refuses a command only when the
    /// session stopped: it was closed, its actor finished, or the command could not be
    /// queued and the host closed the session rather than lose ordered input. A refusal
    /// is therefore never transient.
    fn send(&self, command: api::CommandKind) -> bool;
    /// Take queued output and the newest frame in the browser's order (document, then
    /// sequence), so an error the browser sent after a frame stays the newest output. The
    /// session's end is its last output, which the host publishes, and wakes the worker
    /// for, before `send` refuses commands. A failure that stops the session replaces
    /// pending output with one terminal error. A browser that closes itself, after the
    /// error of a document it could not open or without one, ends its output with the
    /// protocol's Closed event, after the output it sent before. A session the worker
    /// closes publishes no Closed event.
    fn drain(&self, maximum: usize) -> Vec<api::Event>;
    fn close(&self);
    /// The session's actor thread ended its work. The host wakes the worker once this is
    /// true, so a pending opening needs no retry.
    fn finished(&self) -> bool;
    fn policy_change_allowed(&self) -> bool;
    fn release_on_worker(self: Box<Self>) -> Result<(), String>;
    /// The system refused the browser its sandbox when it started: the typed reason of the
    /// terminal error the session published (and the worker drained). `None` for every
    /// other end and while the session runs.
    fn sandbox_refusal(&self) -> Option<midnight_web_viewer_host::SandboxRefusal> {
        None
    }
}

impl Browser for midnight_web_viewer_host::Session {
    fn send(&self, command: api::CommandKind) -> bool {
        midnight_web_viewer_host::Session::send(self, command)
    }
    fn drain(&self, maximum: usize) -> Vec<api::Event> {
        midnight_web_viewer_host::Session::drain(self, maximum)
    }
    fn close(&self) {
        midnight_web_viewer_host::Session::close(self)
    }
    fn finished(&self) -> bool {
        midnight_web_viewer_host::Session::finished(self)
    }
    fn policy_change_allowed(&self) -> bool {
        midnight_web_viewer_host::Session::policy_change_allowed(self)
    }
    fn release_on_worker(self: Box<Self>) -> Result<(), String> {
        midnight_web_viewer_host::Session::release_on_worker(*self)
    }
    fn sandbox_refusal(&self) -> Option<midnight_web_viewer_host::SandboxRefusal> {
        midnight_web_viewer_host::Session::sandbox_refusal(self)
    }
}

const SOURCE_CHANGE_NOTICE: &str = "Source changed; showing previous contents. Reopen to refresh.";
const MAX_SOURCE_ORIGIN_BYTES: usize = 1024;
const ACTION_REFUSED_NOTICE: &str =
    "Navigation could not be completed; previous contents remain available.";
const MAX_ACTION_NOTICE_BYTES: usize = 512;

#[derive(Default)]
struct ActionNotice(Option<String>);

impl ActionNotice {
    fn observe(&mut self, kind: &api::EventKind) -> bool {
        let api::EventKind::Error {
            code: api::ErrorCode::Denied,
            message,
        } = kind
        else {
            return false;
        };
        self.0 = Some(
            if message.is_empty()
                || message.len() > MAX_ACTION_NOTICE_BYTES
                || message.chars().any(char::is_control)
            {
                ACTION_REFUSED_NOTICE.into()
            } else {
                message.clone()
            },
        );
        true
    }

    fn text(&self) -> Option<&str> {
        self.0.as_deref()
    }

    fn user_action(&mut self, command: &api::CommandKind) -> bool {
        user_action(command) && self.0.take().is_some()
    }
}

/// Whether `command` is something the user did: a press, typed text, a scroll or a
/// command; not a pointer move, leave or release, a key-up or a focus change.
pub(super) fn user_action(command: &api::CommandKind) -> bool {
    match command {
        api::CommandKind::Pointer(pointer) => pointer.kind == api::PointerKind::Down,
        api::CommandKind::Key(key) => key.pressed,
        api::CommandKind::Text(text) => !text.is_empty(),
        api::CommandKind::Wheel(wheel) => wheel.delta_x != 0.0 || wheel.delta_y != 0.0,
        api::CommandKind::Ime(_)
        | api::CommandKind::Copy
        | api::CommandKind::Find(_)
        | api::CommandKind::Zoom(_)
        | api::CommandKind::Policy(_) => true,
        _ => false,
    }
}

#[derive(Default)]
struct SourceChange {
    pending: Option<api::Header>,
    last_origin: Option<api::Header>,
    origin_before: bool,
    reported: bool,
}

impl SourceChange {
    fn observe(&mut self, event: &api::Event, fixed_office: bool) -> bool {
        let pending = self.pending.take();
        let origin_before = std::mem::take(&mut self.origin_before);
        if !fixed_office || self.reported {
            return false;
        }
        if pending == Some(event.header) && source_change_diagnostic(&event.kind) {
            self.reported = true;
            return true;
        }
        if valid_source_origin(event) {
            self.origin_before = true;
            if self.last_origin.is_none_or(|previous| {
                previous.session == event.header.session
                    && previous.document == event.header.document
                    && event.header.sequence > previous.sequence
            }) {
                self.last_origin = Some(event.header);
                if !origin_before {
                    self.pending = Some(event.header);
                }
            }
        }
        false
    }
}

fn source_change_diagnostic(kind: &api::EventKind) -> bool {
    matches!(kind, api::EventKind::Error {
        code: api::ErrorCode::ResourceChanged, message,
    } if message == SOURCE_CHANGE_NOTICE)
}

fn valid_source_origin(event: &api::Event) -> bool {
    if event.header.validate().is_err() {
        return false;
    }
    match &event.kind {
        api::EventKind::ResourceResolve { request, base, url } => {
            *request != 0 && *base != 0 && bounded_office_origin(url)
        }
        api::EventKind::ResourceRead {
            request,
            resource,
            offset,
            length,
        } => *request != 0 && *resource != 0 && api::validate_read(*offset, *length).is_ok(),
        _ => false,
    }
}

fn bounded_office_origin(url: &str) -> bool {
    // Correlation metadata only: the Host owns canonical target validation and
    // sent-Denied publication. Never use this path to resolve/read or acknowledge
    // input/paint, and never duplicate the Office parser's family/range policy.
    let path = url.strip_prefix('/').unwrap_or(url);
    url.len() <= MAX_SOURCE_ORIGIN_BYTES
        && path.starts_with("office/v1/")
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        })
}

fn accepts_input(
    frames: &crate::interaction::Frames,
    first: u64,
    frame: u64,
    command: &api::CommandKind,
    policy_change_allowed: bool,
) -> bool {
    if !policy_change_allowed && matches!(command, api::CommandKind::Policy(_)) {
        return false;
    }
    if matches!(
        command,
        api::CommandKind::Policy(api::Policy { network: false, .. })
            | api::CommandKind::Focus(false)
    ) {
        return true;
    }
    if frame == 0 || frame < first || frame > frames.latest_revision() {
        return false;
    }
    match command {
        api::CommandKind::Pointer(pointer)
            if matches!(
                pointer.kind,
                api::PointerKind::Down | api::PointerKind::Move
            ) =>
        {
            frames.accepts(frame)
        }
        api::CommandKind::Wheel(_) => frames.accepts(frame),
        _ => true,
    }
}

pub(super) struct WebView {
    pub session: Box<dyn Browser>,
    viewport: api::Viewport,
    policy: api::Policy,
    zoom: f64,
    find_result: Option<String>,
    source_change: SourceChange,
    action_notice: ActionNotice,
    file_floor: u64,
    context_floor: u64,
    /// The session stopped accepting input, and `Worker::browser_closed` handled it.
    ended: bool,
    /// The newest browser output drained is an error, which can be the reason the session
    /// stopped (see `Browser::drain`). Any later output shows the session outlived it. An
    /// accepted command does not: input sent before the host marks a failed session
    /// stopped is accepted and never handled.
    reason: bool,
    /// The page's title (its browser's report), or the file's name before it has one.
    title: String,
    name: String,
    loading: bool,
    /// The link target the pointer or keyboard focus is on, as the browser reported it.
    hover: String,
}

impl WebView {
    #[cfg(test)]
    pub(super) fn new(session: Box<dyn Browser>, viewport: api::Viewport, floor: u64) -> Self {
        Self::with_policy(session, viewport, floor, api::Policy::default())
    }

    pub(super) fn with_policy(
        session: Box<dyn Browser>,
        viewport: api::Viewport,
        floor: u64,
        policy: api::Policy,
    ) -> Self {
        Self {
            session,
            viewport,
            policy,
            zoom: 1.0,
            find_result: None,
            source_change: SourceChange::default(),
            action_notice: ActionNotice::default(),
            file_floor: floor,
            context_floor: floor,
            ended: false,
            reason: false,
            title: String::new(),
            name: String::new(),
            loading: false,
            hover: String::new(),
        }
    }

    /// The policy in force.
    pub(super) fn policy(&self) -> api::Policy {
        self.policy
    }

    /// The status line: the page's title (or file name), or that it loads, then the
    /// policy, a notice and the Find result. An Office preview shows no title.
    fn status(&self) -> String {
        let status = web_presentation_status(
            self.policy,
            self.session.policy_change_allowed(),
            self.find_result.as_deref(),
            self.source_change.reported,
            self.action_notice.text(),
        );
        // An Office preview's generated pages keep their fixed status.
        if !self.session.policy_change_allowed() {
            return status;
        }
        let page = if self.title.is_empty() {
            &self.name
        } else {
            &self.title
        };
        match (self.loading, page.is_empty()) {
            (true, _) => format!("Loading… · {status}"),
            (false, true) => status,
            (false, false) => format!("{page} · {status}"),
        }
    }

    fn state(&self, generation: u64) -> ViewerOutput {
        ViewerOutput::WebState {
            generation,
            policy: self.policy,
            policy_change_allowed: self.session.policy_change_allowed(),
            zoom: self.zoom,
            loading: self.loading,
        }
    }
}

/// A title for the status line: its visible characters, at most `MAX_TITLE_CHARS`.
fn shown_title(title: &str) -> String {
    let mut shown: String = title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_TITLE_CHARS + 1)
        .collect();
    if shown.chars().count() > MAX_TITLE_CHARS {
        shown = shown.chars().take(MAX_TITLE_CHARS - 1).collect();
        shown.push('…');
    }
    shown
}

impl Worker {
    pub(super) fn open_web(
        &mut self,
        provider: midnight_web_viewer_host::Provider,
        file: midnight_plugin_host::CapturedFile,
    ) -> Result<(), String> {
        // Its links go to the host's link policy (`worker_links.rs`).
        self.start_web(
            provider,
            file,
            None,
            midnight_web_viewer_host::PolicyCeiling::UserChoice,
            midnight_web_viewer_host::LinkRouting::Host,
        )
    }

    pub(super) fn open_office(
        &mut self,
        provider: midnight_web_viewer_host::Provider,
        file: midnight_plugin_host::CapturedFile,
        output: &Mailbox,
    ) -> Result<(), String> {
        let prepared = self.prepare_office(file, output)?;
        // The preview's own paging links stay in the browser; it shows no document links.
        self.start_web(
            provider,
            prepared.main,
            Some(prepared.resources),
            midnight_web_viewer_host::PolicyCeiling::OfflineOnly,
            midnight_web_viewer_host::LinkRouting::Browser,
        )
    }

    /// Generates the document's first page before a browser starts. The caller reports a
    /// failure on the status line (`Worker::error`); one with diagnostic details also
    /// takes the page's place with Copy details until another document opens.
    pub(super) fn prepare_office(
        &mut self,
        file: midnight_plugin_host::CapturedFile,
        output: &Mailbox,
    ) -> Result<crate::office_preview::Prepared, String> {
        let path = self
            .files
            .get(self.current)
            .ok_or("Original Office file is unavailable")?;
        let format = crate::office_preview::format(path);
        self.length = file.length();
        match crate::office_preview::prepare(
            file,
            format,
            Cancellation::observing(self.window_closed.clone()),
        ) {
            Ok(prepared) => Ok(prepared),
            Err(error) => {
                if let Some(details) = error.details {
                    self.show_failure_notice(error.message.clone(), details, output);
                }
                Err(error.message)
            }
        }
    }

    fn start_web(
        &mut self,
        provider: midnight_web_viewer_host::Provider,
        file: midnight_plugin_host::CapturedFile,
        resources: Option<Arc<dyn midnight_web_viewer_host::Resources>>,
        ceiling: midnight_web_viewer_host::PolicyCeiling,
        links: midnight_web_viewer_host::LinkRouting,
    ) -> Result<(), String> {
        let viewport = api::Viewport {
            revision: 1,
            width: self.geometry.width,
            height: self.geometry.height,
            scale: f64::from(self.geometry.scale),
        };
        // A Reload keeps the page's choices. Otherwise each page opens offline, and with
        // JavaScript unless the user turned it off in this window. Previews stay offline
        // without scripts.
        let current = self.files.get(self.current).cloned();
        let reload = (self.web_choices.reload.take())
            .filter(|(path, _)| current.as_ref() == Some(path))
            .map(|(_, policy)| policy);
        let policy = match ceiling {
            midnight_web_viewer_host::PolicyCeiling::OfflineOnly => api::Policy::default(),
            midnight_web_viewer_host::PolicyCeiling::UserChoice => reload.unwrap_or(api::Policy {
                network: false,
                scripts: self.web_choices.scripts,
            }),
        };
        let name = midnight_core::visible_name(&file.display_name()).into_owned();
        let session = midnight_web_viewer_host::Session::open_with_options(
            provider,
            file,
            resources,
            viewport,
            self.wake.clone(),
            midnight_web_viewer_host::OpenOptions {
                ceiling,
                links,
                policy,
            },
        )?;
        let mut web = WebView::with_policy(
            Box::new(session),
            viewport,
            self.frames.latest_revision().saturating_add(1),
            policy,
        );
        web.name = name;
        web.loading = true;
        self.web = Some(web);
        self.mode = VIEWER_MODE_HTML;
        self.modes = vec![VIEWER_MODE_HTML];
        self.binary = false;
        Ok(())
    }

    pub(super) fn retire_web(&mut self) -> Result<bool, String> {
        if let Some(error) = &self.web_retirement_error {
            return Err(error.clone());
        }
        if self.web.is_some() && self.closing_web.is_some() {
            record_web_retirement(
                &mut self.web_retirement_error,
                Err("Browser retirement ownership overlaps; cleanup is unconfirmed".into()),
            )?;
        }
        if let Some(web) = self.web.take() {
            web.session.close();
            self.closing_web = Some(web.session);
        }
        if self
            .closing_web
            .as_ref()
            .is_some_and(|session| !session.finished())
        {
            return Ok(false);
        }
        if let Some(session) = self.closing_web.take() {
            record_web_retirement(&mut self.web_retirement_error, release_web(session))?;
        }
        Ok(true)
    }

    pub(super) fn finish_web(&mut self) -> Result<(), String> {
        let mut failures = Vec::new();
        for session in [
            self.web.take().map(|web| web.session),
            self.closing_web.take(),
        ]
        .into_iter()
        .flatten()
        {
            if let Err(error) = release_web(session) {
                failures.push(error);
            }
        }
        let result = if failures.is_empty() {
            Ok(())
        } else {
            Err(failures.join("; "))
        };
        record_web_retirement(&mut self.web_retirement_error, result)
    }

    pub(super) fn web_input(&mut self, input: &ViewerInput, output: &Mailbox) -> bool {
        let policy_change = matches!(
            input,
            ViewerInput::Web {
                input: api::CommandKind::Policy(_),
                ..
            }
        );
        let Some(web) = self.web.as_mut() else {
            return false;
        };
        if let ViewerInput::Web { frame, input } = input {
            let first = if matches!(input, api::CommandKind::Policy(_)) {
                web.file_floor
            } else {
                web.context_floor
            };
            if !accepts_input(
                &self.frames,
                first,
                *frame,
                input,
                web.session.policy_change_allowed(),
            ) {
                return true;
            }
        }
        let command = match input {
            ViewerInput::NextFile | ViewerInput::PreviousFile | ViewerInput::Close => return false,
            ViewerInput::Resize(geometry) => {
                self.frames.invalidate();
                self.geometry = bounded_geometry(*geometry);
                web.viewport = api::Viewport {
                    revision: web.viewport.revision.saturating_add(1),
                    width: self.geometry.width,
                    height: self.geometry.height,
                    scale: f64::from(self.geometry.scale),
                };
                api::CommandKind::Resize(web.viewport)
            }
            ViewerInput::Web {
                input: api::CommandKind::Policy(policy),
                ..
            } => {
                web.policy = *policy;
                // The next page of this window opens with the scripts choice made here.
                if web.session.policy_change_allowed() {
                    self.web_choices.scripts = policy.scripts;
                }
                web.find_result = None;
                web.context_floor = self.frames.latest_revision().saturating_add(1);
                // Host revokes grants and recreates the context before admitting new input.
                api::CommandKind::Policy(*policy)
            }
            ViewerInput::Web { input: command, .. } => match command {
                api::CommandKind::Pointer(_)
                | api::CommandKind::Wheel(_)
                | api::CommandKind::Key(_)
                | api::CommandKind::Text(_)
                | api::CommandKind::Ime(_)
                | api::CommandKind::Focus(_)
                | api::CommandKind::Copy => command.clone(),
                api::CommandKind::Zoom(value) => {
                    web.zoom = value.clamp(0.1, 10.0);
                    api::CommandKind::Zoom(web.zoom)
                }
                _ => return true,
            },
            ViewerInput::Search {
                query,
                backwards,
                case_sensitive,
                ..
            } => {
                web.find_result = None;
                api::CommandKind::Find(api::Find {
                    query: query.clone(),
                    forward: !backwards,
                    match_case: *case_sensitive,
                    // Chromium activates a match only for a follow-up search: a new
                    // session highlights every match but reports none active. A follow-up
                    // with other text or case starts a new session and activates the
                    // nearest match on from the active one, as the text viewer searches
                    // on from where it stands.
                    find_next: true,
                })
            }
            ViewerInput::Copy { frame }
                if accepts_input(
                    &self.frames,
                    web.context_floor,
                    *frame,
                    &api::CommandKind::Copy,
                    web.session.policy_change_allowed(),
                ) =>
            {
                api::CommandKind::Copy
            }
            ViewerInput::Scroll { x, y } => api::CommandKind::Wheel(api::Wheel {
                x: 0.0,
                y: 0.0,
                delta_x: -f64::from(*x),
                delta_y: -f64::from(*y),
                modifiers: Default::default(),
            }),
            _ => return true,
        };
        // A refusal means the host stopped the session. Its final output can explain why.
        let notice_cleared = web.action_notice.user_action(&command);
        if !web.session.send(command) {
            self.browser_closed(output);
        } else if notice_cleared {
            let status = web.status();
            self.status(status, output);
        }
        if policy_change || matches!(input, ViewerInput::Resize(_)) {
            self.frames.invalidate();
            self.position_context(output);
        }
        true
    }

    pub(super) fn poll_web(&mut self, output: &Mailbox) {
        self.web_backlog = false;
        let Some(web) = self.web.as_ref() else { return };
        let events = web.session.drain(WEB_BATCH);
        // A full batch can leave events queued; the browser wakes only for newer output.
        self.web_backlog = events.len() >= WEB_BATCH;
        for event in events {
            crate::open_trace::mark(if matches!(event.kind, api::EventKind::Frame(_)) {
                "web_frame_event"
            } else {
                "web_event"
            });
            // Closed ends the session and must still see whether an error preceded it.
            if let Some(web) = self.web.as_mut()
                && !matches!(event.kind, api::EventKind::Closed)
            {
                web.reason = matches!(event.kind, api::EventKind::Error { .. });
            }
            let source_changed = match self.web.as_mut() {
                Some(web) => web
                    .source_change
                    .observe(&event, !web.session.policy_change_allowed()),
                None => break,
            };
            if source_changed {
                // This known Host diagnostic keeps the committed DOM, current
                // layout and frame. All unmatched errors retain Worker::error.
                self.status(SOURCE_CHANGE_NOTICE.into(), output);
                continue;
            }
            if let Some(web) = self.web.as_mut()
                && web.action_notice.observe(&event.kind)
            {
                let status = web.status();
                self.status(status, output);
                continue;
            }
            match event.kind {
                api::EventKind::Frame(frame) => {
                    let Some(web) = self.web.as_ref() else { break };
                    if frame.viewport_revision != web.viewport.revision
                        || frame.width != web.viewport.width
                        || frame.height != web.viewport.height
                    {
                        continue;
                    }
                    let revision = match self.frames.painted() {
                        Ok(revision) => revision,
                        Err(error) => {
                            self.error(error, output);
                            continue;
                        }
                    };
                    output.put(ViewerOutput::Frame(Box::new(ViewerFrame {
                        generation: self.positions.generation,
                        media_control: None,
                        position_sequence: [0; 2],
                        revision,
                        mode: VIEWER_MODE_HTML,
                        modes: self.modes.clone(),
                        wrap_enabled: false,
                        wrap_available: false,
                        image: None,
                        raster: None,
                        raster_rebase: None,
                        width: frame.width,
                        height: frame.height,
                        pixels: frame.rgba.into(),
                        tiles: None,
                        accessible_text: Arc::from(if web.session.policy_change_allowed() {
                            "Interactive HTML document"
                        } else {
                            "Office contents preview"
                        }),
                        position: 0.0,
                        vertical_extent: 1.0,
                        horizontal_position: 0.0,
                        horizontal_extent: 1.0,
                        caption: self.caption.clone(),
                        status: web.status(),
                    })));
                    output.put(web.state(self.positions.generation));
                }
                api::EventKind::Selection(text) => output.put(ViewerOutput::CopyText(text)),
                api::EventKind::State(state) => {
                    if let Some(web) = self.web.as_mut() {
                        web.zoom = state.zoom;
                        let title = shown_title(&state.title);
                        let shown = web.loading != state.loading || web.title != title;
                        web.loading = state.loading;
                        web.title = title;
                        output.put(web.state(self.positions.generation));
                        if shown {
                            let status = web.status();
                            self.status(status, output);
                        }
                    }
                }
                api::EventKind::Cursor(cursor) => output.put(ViewerOutput::WebCursor(cursor)),
                // The status a frame shows: the page's state, then the Find result. A
                // frame that came before this reply must not leave the bare count.
                api::EventKind::FindResult(result) => {
                    let message = find_status(&result);
                    let status = match self.web.as_mut() {
                        Some(web) => {
                            web.find_result = Some(message);
                            web.status()
                        }
                        None => message,
                    };
                    self.status(status, output);
                }
                // The page's links: where the pointer is, and a followed one.
                api::EventKind::LinkHover { target } => {
                    output.put(ViewerOutput::LinkHover(self.web_link_status(&target)));
                    if let Some(web) = self.web.as_mut() {
                        web.hover = target;
                    }
                }
                api::EventKind::LinkActivated { target } => {
                    if self.web.as_ref().is_some_and(|web| web.refuses(&target)) {
                        self.web_link_notice(SCRIPT_ADDRESS_REFUSED.into(), output);
                    } else {
                        self.follow_link(&target, output);
                    }
                }
                api::EventKind::HostShortcut(shortcut) => {
                    output.put(ViewerOutput::WebShortcut(shortcut))
                }
                // Not every browser error ends the session (an oversized Copy does not). A
                // refused sandbox is shown with the step that allows it instead of the page.
                api::EventKind::Error { message, .. } => {
                    match self
                        .web
                        .as_ref()
                        .and_then(|web| web.session.sandbox_refusal())
                    {
                        Some(refusal) => self.sandbox_refused(refusal, message, output),
                        None => self.error(message, output),
                    }
                }
                // The host's last output when the browser closed itself (see `Browser::drain`).
                api::EventKind::Closed => self.browser_closed(output),
                _ => {}
            }
        }
    }

    /// A message about a followed link as the browser's notice, which stays through frames
    /// until the next user action, like a refused navigation.
    pub(super) fn web_link_notice(&mut self, message: String, output: &Mailbox) {
        let Some(web) = self.web.as_mut() else {
            self.status(message, output);
            return;
        };
        web.action_notice.observe(&api::EventKind::Error {
            code: api::ErrorCode::Denied,
            message,
        });
        let status = web.status();
        self.status(status, output);
    }

    /// The active browser session stopped and accepts no more input. Handle that once,
    /// and report it only when no browser error is the newest output: such an error is the
    /// stop's reason and stays visible. Later reports would replace a reason the host
    /// publishes when it refuses a command, and wake the UI for every pointer move over
    /// the stopped page.
    fn browser_closed(&mut self, output: &Mailbox) {
        let Some(web) = self.web.as_mut() else { return };
        if !std::mem::replace(&mut web.ended, true) && !web.reason {
            self.error(BROWSER_CLOSED.into(), output);
        }
    }
}

/// A browser Find result for the status line, worded as the comparator's Find: the
/// active match of the count once Chromium activated one, else the count alone.
fn find_status(result: &api::FindResult) -> String {
    match (result.matches, result.active) {
        (0, _) => "No matches".into(),
        (1, 0) => "1 match".into(),
        (matches, 0) => format!("{matches} matches"),
        (matches, active) => format!("Match {active} of {matches}"),
    }
}

impl WebView {
    /// Whether a followed link must not open: offline, the page's scripts run without any
    /// network, so a web address (anything with a scheme, `mailto:` included) opens only
    /// when it is the link the pointer or keyboard focus is on, which the status line
    /// shows. Another one comes from a script, such as a click handler that navigates;
    /// opening it would carry the page's data to the network. Local targets still open.
    fn refuses(&self, target: &str) -> bool {
        self.policy.scripts
            && !self.policy.network
            && matches!(
                midnight_core::classify_link(target),
                midnight_core::LinkTarget::External(_)
            )
            && target != self.hover
    }
}

fn web_status(policy: api::Policy, policy_change_allowed: bool) -> &'static str {
    if !policy_change_allowed {
        "Office preview · Offline · JavaScript off"
    } else if policy.network && policy.scripts {
        "Online · scripts can read and transmit this document · local assets unavailable"
    } else if policy.network {
        "Online · document only · JavaScript off · local assets unavailable"
    } else if policy.scripts {
        "Offline · JavaScript on"
    } else {
        "Offline · JavaScript off"
    }
}

fn web_presentation_status(
    policy: api::Policy,
    policy_change_allowed: bool,
    find: Option<&str>,
    source_changed: bool,
    action_notice: Option<&str>,
) -> String {
    let mut status = if source_changed {
        SOURCE_CHANGE_NOTICE.to_owned()
    } else if let Some(notice) = action_notice {
        notice.to_owned()
    } else {
        web_status(policy, policy_change_allowed).to_owned()
    };
    if let Some(find) = find {
        status.push_str(" · ");
        status.push_str(find);
    }
    status
}

fn release_web(session: Box<dyn Browser>) -> Result<(), String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| session.release_on_worker()))
        .unwrap_or_else(|_| Err("Browser cleanup panicked; cleanup is unconfirmed".into()))
}

fn record_web_retirement(
    failure: &mut Option<String>,
    result: Result<(), String>,
) -> Result<(), String> {
    if let Err(error) = result {
        if let Some(previous) = failure {
            previous.push_str("; ");
            previous.push_str(&error);
        } else {
            *failure = Some(error);
        }
    }
    failure.as_ref().map_or(Ok(()), |error| Err(error.clone()))
}
