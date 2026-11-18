//! Interactive browser routing. All source, policy and process work stays on workers.
use super::*;
use midnight_core::viewer_web as api;
#[cfg(test)]
#[path = "worker_web_tests.rs"]
mod tests;

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
        let genuine = match command {
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
        };
        genuine && self.0.take().is_some()
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
    pub session: midnight_web_viewer_host::Session,
    viewport: api::Viewport,
    policy: api::Policy,
    zoom: f64,
    find: String,
    find_result: Option<String>,
    source_change: SourceChange,
    action_notice: ActionNotice,
    file_floor: u64,
    context_floor: u64,
}

impl Worker {
    pub(super) fn open_web(
        &mut self,
        provider: midnight_web_viewer_host::Provider,
        file: midnight_plugin_host::CapturedFile,
    ) -> Result<(), String> {
        self.start_web(
            provider,
            file,
            None,
            midnight_web_viewer_host::PolicyCeiling::UserChoice,
        )
    }

    pub(super) fn open_office(
        &mut self,
        provider: midnight_web_viewer_host::Provider,
        file: midnight_plugin_host::CapturedFile,
    ) -> Result<(), String> {
        let path = &self
            .files
            .get(self.current)
            .ok_or("Original Office file is unavailable")?
            .0;
        let format = crate::office_preview::format(path);
        self.length = file.length();
        let prepared = crate::office_preview::prepare(
            file,
            format,
            Cancellation::observing(self.window_closed.clone()),
        )?;
        self.start_web(
            provider,
            prepared.main,
            Some(prepared.resources),
            midnight_web_viewer_host::PolicyCeiling::OfflineOnly,
        )
    }

    fn start_web(
        &mut self,
        provider: midnight_web_viewer_host::Provider,
        file: midnight_plugin_host::CapturedFile,
        resources: Option<Arc<dyn midnight_web_viewer_host::Resources>>,
        ceiling: midnight_web_viewer_host::PolicyCeiling,
    ) -> Result<(), String> {
        let viewport = api::Viewport {
            revision: 1,
            width: self.geometry.width,
            height: self.geometry.height,
            scale: f64::from(self.geometry.scale),
        };
        self.web = Some(WebView {
            file_floor: self.frames.latest_revision().saturating_add(1),
            context_floor: self.frames.latest_revision().saturating_add(1),
            session: midnight_web_viewer_host::Session::open_with_policy_ceiling(
                provider,
                file,
                resources,
                viewport,
                self.wake.clone(),
                ceiling,
            )?,
            viewport,
            policy: api::Policy::default(),
            zoom: 1.0,
            find: String::new(),
            find_result: None,
            source_change: SourceChange::default(),
            action_notice: ActionNotice::default(),
        });
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
                if policy.scripts && !policy.network {
                    return true;
                }
                web.policy = *policy;
                web.find.clear();
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
            } => {
                let find_next = web.find == *query;
                web.find.clone_from(query);
                web.find_result = None;
                api::CommandKind::Find(api::Find {
                    query: query.clone(),
                    forward: !backwards,
                    match_case: *case_sensitive,
                    find_next,
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
        if !web.session.send(command.clone()) {
            self.status("Browser input queue is busy; try again.".into(), output);
        } else if web.action_notice.user_action(&command) {
            let status = web_presentation_status(
                web.policy,
                web.session.policy_change_allowed(),
                web.find_result.as_deref(),
                web.source_change.reported,
                web.action_notice.text(),
            );
            self.status(status, output);
        }
        if policy_change || matches!(input, ViewerInput::Resize(_)) {
            self.frames.invalidate();
            self.position_context(output);
        }
        true
    }

    pub(super) fn poll_web(&mut self, output: &Mailbox) {
        let Some(web) = self.web.as_ref() else { return };
        let events = web.session.drain(32);
        for event in events {
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
                let status = web_presentation_status(
                    web.policy,
                    web.session.policy_change_allowed(),
                    web.find_result.as_deref(),
                    web.source_change.reported,
                    web.action_notice.text(),
                );
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
                        status: web_presentation_status(
                            web.policy,
                            web.session.policy_change_allowed(),
                            web.find_result.as_deref(),
                            web.source_change.reported,
                            web.action_notice.text(),
                        ),
                    })));
                    output.put(ViewerOutput::WebState {
                        generation: self.positions.generation,
                        policy: web.policy,
                        policy_change_allowed: web.session.policy_change_allowed(),
                        zoom: web.zoom,
                    });
                }
                api::EventKind::Selection(text) => output.put(ViewerOutput::CopyText(text)),
                api::EventKind::State(state) => {
                    if let Some(web) = self.web.as_mut() {
                        web.zoom = state.zoom;
                        output.put(ViewerOutput::WebState {
                            generation: self.positions.generation,
                            policy: web.policy,
                            policy_change_allowed: web.session.policy_change_allowed(),
                            zoom: web.zoom,
                        });
                    }
                }
                api::EventKind::FindResult(result) => {
                    let message = if result.matches == 0 {
                        "No matches".into()
                    } else if result.active == 0 {
                        format!("{} matches", result.matches)
                    } else {
                        format!("Match {} of {}", result.active, result.matches)
                    };
                    let status = if let Some(web) = self.web.as_mut() {
                        web.find_result = Some(message.clone());
                        if web.source_change.reported || web.action_notice.text().is_some() {
                            web_presentation_status(
                                web.policy,
                                web.session.policy_change_allowed(),
                                web.find_result.as_deref(),
                                web.source_change.reported,
                                web.action_notice.text(),
                            )
                        } else {
                            message
                        }
                    } else {
                        message
                    };
                    self.status(status, output);
                }
                api::EventKind::HostShortcut(shortcut) => {
                    output.put(ViewerOutput::WebShortcut(shortcut))
                }
                api::EventKind::Error { message, .. } => self.error(message, output),
                api::EventKind::Closed => self.error("Browser closed.".into(), output),
                _ => {}
            }
        }
    }
}

fn web_status(policy: api::Policy, policy_change_allowed: bool) -> &'static str {
    if !policy_change_allowed {
        "Office preview · Offline · JavaScript off"
    } else if policy.scripts {
        "Online · scripts can read and transmit this document · local assets unavailable"
    } else if policy.network {
        "Online · document only · JavaScript off · local assets unavailable"
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

fn release_web(session: midnight_web_viewer_host::Session) -> Result<(), String> {
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
