//! Interactive browser routing. All source, policy and process work stays on workers.
use super::*;
use midnight_core::viewer_web as api;
#[cfg(test)]
#[path = "worker_web_tests.rs"]
mod tests;

const WEB_BATCH: usize = 32;
/// Shown once per browser session when it stopped accepting input.
const BROWSER_CLOSED: &str = "Browser closed; reopen the document.";

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
}

fn accepts_input(
    frames: &crate::interaction::Frames,
    first: u64,
    frame: u64,
    command: &api::CommandKind,
) -> bool {
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
    find: String,
    find_result: Option<String>,
    file_floor: u64,
    context_floor: u64,
    /// The session stopped accepting input, and `Worker::browser_closed` handled it.
    ended: bool,
    /// The newest browser output drained is an error, which can be the reason the session
    /// stopped (see `Browser::drain`). Any later output shows the session outlived it. An
    /// accepted command does not: input sent before the host marks a failed session
    /// stopped is accepted and never handled.
    reason: bool,
}

impl WebView {
    fn new(session: Box<dyn Browser>, viewport: api::Viewport, floor: u64) -> Self {
        Self {
            session,
            viewport,
            policy: api::Policy::default(),
            zoom: 1.0,
            find: String::new(),
            find_result: None,
            file_floor: floor,
            context_floor: floor,
            ended: false,
            reason: false,
        }
    }
}

impl Worker {
    pub(super) fn open_web(
        &mut self,
        provider: midnight_web_viewer_host::Provider,
        file: midnight_plugin_host::CapturedFile,
    ) -> Result<(), String> {
        let viewport = api::Viewport {
            revision: 1,
            width: self.geometry.width,
            height: self.geometry.height,
            scale: f64::from(self.geometry.scale),
        };
        let session = midnight_web_viewer_host::Session::open(
            provider,
            file,
            None,
            viewport,
            self.wake.clone(),
        )?;
        self.web = Some(WebView::new(
            Box::new(session),
            viewport,
            self.frames.latest_revision().saturating_add(1),
        ));
        self.mode = VIEWER_MODE_HTML;
        self.modes = vec![VIEWER_MODE_HTML];
        self.binary = false;
        Ok(())
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
            if !accepts_input(&self.frames, first, *frame, input) {
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
        // A refusal means the host stopped the session, so retrying cannot help. A host
        // that stops the session because of this very command can publish its reason as
        // well; `poll_web` drains it after this input and shows it instead.
        if !web.session.send(command) {
            self.browser_closed(output);
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
            // Closed ends the session and must still see whether an error preceded it.
            if let Some(web) = self.web.as_mut()
                && !matches!(event.kind, api::EventKind::Closed)
            {
                web.reason = matches!(event.kind, api::EventKind::Error { .. });
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
                        accessible_text: Arc::from("Interactive HTML document"),
                        position: 0.0,
                        vertical_extent: 1.0,
                        horizontal_position: 0.0,
                        horizontal_extent: 1.0,
                        caption: self.caption.clone(),
                        status: web.find_result.as_ref().map_or_else(
                            || web_status(web.policy).into(),
                            |result| format!("{} · {result}", web_status(web.policy)),
                        ),
                    })));
                    output.put(ViewerOutput::WebState {
                        generation: self.positions.generation,
                        policy: web.policy,
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
                    if let Some(web) = self.web.as_mut() {
                        web.find_result = Some(message.clone());
                    }
                    self.status(message, output);
                }
                api::EventKind::HostShortcut(shortcut) => {
                    output.put(ViewerOutput::WebShortcut(shortcut))
                }
                // Not every browser error ends the session (an oversized Copy does not).
                api::EventKind::Error { message, .. } => self.error(message, output),
                // The host's last output when the browser closed itself (see `Browser::drain`).
                api::EventKind::Closed => self.browser_closed(output),
                _ => {}
            }
        }
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

fn web_status(policy: api::Policy) -> &'static str {
    if policy.scripts {
        "Online · scripts can read and transmit this document · local assets unavailable"
    } else if policy.network {
        "Online · document only · JavaScript off · local assets unavailable"
    } else {
        "Offline · JavaScript off"
    }
}
