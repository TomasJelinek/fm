use super::*;

fn key(pressed: bool) -> api::CommandKind {
    api::CommandKind::Key(api::Key {
        key: "Shift".into(),
        code: "ShiftLeft".into(),
        pressed,
        repeat: false,
        modifiers: Default::default(),
    })
}

fn pointer(kind: api::PointerKind) -> api::CommandKind {
    api::CommandKind::Pointer(api::Pointer {
        kind,
        button: api::PointerButton::Left,
        x: 10.0,
        y: 20.0,
        modifiers: Default::default(),
        click_count: 1,
    })
}

#[test]
fn resize_keeps_keyboard_and_button_release_but_rejects_stale_coordinate_input() {
    let mut frames = crate::interaction::Frames::default();
    let first = frames.painted().unwrap();
    frames.invalidate();
    assert!(accepts_input(&frames, first, first, &key(false)));
    assert!(accepts_input(&frames, first, first, &key(true)));
    assert!(accepts_input(
        &frames,
        first,
        first,
        &pointer(api::PointerKind::Up)
    ));
    assert!(accepts_input(
        &frames,
        first,
        first,
        &api::CommandKind::Zoom(1.25)
    ));
    assert!(!accepts_input(
        &frames,
        first,
        first,
        &pointer(api::PointerKind::Down)
    ));
    assert!(!accepts_input(
        &frames,
        first,
        first,
        &pointer(api::PointerKind::Move)
    ));
    let fresh = frames.painted().unwrap();
    assert!(accepts_input(
        &frames,
        first,
        fresh,
        &pointer(api::PointerKind::Down)
    ));
}

#[test]
fn replaced_document_rejects_old_keys_releases_and_permission_increases() {
    let mut frames = crate::interaction::Frames::default();
    let old = frames.painted().unwrap();
    frames.invalidate();
    let first = frames.painted().unwrap();
    assert!(!accepts_input(&frames, first, old, &key(false)));
    assert!(!accepts_input(
        &frames,
        first,
        old,
        &pointer(api::PointerKind::Up)
    ));
    assert!(!accepts_input(
        &frames,
        first,
        old,
        &api::CommandKind::Policy(api::Policy {
            network: true,
            scripts: false
        })
    ));
    assert!(accepts_input(&frames, first, first, &key(true)));
    assert!(accepts_input(
        &frames,
        first,
        old,
        &api::CommandKind::Policy(api::Policy::default())
    ));
    assert!(!accepts_input(&frames, first, first + 1, &key(true)));
}

fn event(kind: api::EventKind) -> api::Event {
    api::Event {
        header: api::Header {
            session: 1,
            document: 1,
            sequence: 1,
        },
        kind,
    }
}

fn error(message: &str) -> api::Event {
    event(api::EventKind::Error {
        code: api::ErrorCode::Runtime,
        message: message.into(),
    })
}

/// A browser session the host already stopped: it refuses every command. Its queue holds
/// the output the session published before it stopped. `reason` models a host that
/// publishes why the refused command stopped it, once.
struct Stopped(std::sync::Mutex<(Option<&'static str>, Vec<api::Event>)>);

impl Stopped {
    fn new(reason: Option<&'static str>) -> Box<Self> {
        Box::new(Self(std::sync::Mutex::new((reason, Vec::new()))))
    }
    fn after(output: Vec<api::Event>) -> Box<Self> {
        Box::new(Self(std::sync::Mutex::new((None, output))))
    }
}

impl Browser for Stopped {
    fn send(&self, _: api::CommandKind) -> bool {
        let mut state = self.0.lock().unwrap();
        if let Some(message) = state.0.take() {
            state.1.push(error(message));
        }
        false
    }
    fn drain(&self, maximum: usize) -> Vec<api::Event> {
        let mut state = self.0.lock().unwrap();
        let count = maximum.min(state.1.len());
        state.1.drain(..count).collect()
    }
    fn close(&self) {}
    fn finished(&self) -> bool {
        true
    }
}

fn web_worker(session: Box<dyn Browser>) -> (Worker, Mailbox) {
    let (mut worker, output) = crate::worker::tests::worker();
    let floor = worker.frames.latest_revision().saturating_add(1);
    let viewport = api::Viewport {
        revision: 1,
        width: worker.geometry.width,
        height: worker.geometry.height,
        scale: 1.0,
    };
    worker.web = Some(WebView::new(session, viewport, floor));
    (worker, output)
}

fn statuses(output: &Mailbox) -> Vec<String> {
    output
        .take()
        .into_iter()
        .filter_map(|event| match event {
            ViewerOutput::Status(status) => Some(status),
            _ => None,
        })
        .collect()
}

fn search() -> ViewerInput {
    ViewerInput::Search {
        query: "needle".into(),
        backwards: false,
        case_sensitive: false,
    }
}

#[test]
fn input_refused_by_a_stopped_browser_reports_the_closure_once_instead_of_a_busy_queue() {
    let (mut worker, output) = web_worker(Stopped::new(None));
    worker.input(ViewerInput::Scroll { x: 0.0, y: 40.0 }, &output);
    assert_eq!(worker.status, BROWSER_CLOSED);
    assert_eq!(
        output.take_open_result(),
        Some(Err(BROWSER_CLOSED.into())),
        "a browser that stopped before its first frame fails the opening"
    );
    assert_eq!(statuses(&output), [BROWSER_CLOSED]);
    worker.poll_web(&output);
    worker.input(search(), &output);
    worker.input(ViewerInput::Scroll { x: 0.0, y: -40.0 }, &output);
    assert_eq!(worker.status, BROWSER_CLOSED);
    assert!(
        statuses(&output).is_empty(),
        "later refusals neither repeat the report nor wake the UI"
    );
}

#[test]
fn the_reason_a_host_publishes_for_a_refusal_stays_visible() {
    const REASON: &str = "Browser input overflowed while the page was busy; reopen the document.";
    let (mut worker, output) = web_worker(Stopped::new(Some(REASON)));
    output.finish_open(Ok(()));
    worker.input(search(), &output);
    assert_eq!(worker.status, BROWSER_CLOSED);
    // The worker drains the browser after the input batch of the same loop pass.
    worker.poll_web(&output);
    assert_eq!(worker.status, REASON);
    assert_eq!(statuses(&output), [REASON]);
    for input in [search(), ViewerInput::Scroll { x: 0.0, y: 40.0 }] {
        worker.input(input, &output);
        worker.poll_web(&output);
    }
    assert_eq!(worker.status, REASON, "later refusals do not replace it");
    assert!(statuses(&output).is_empty());
}

#[test]
fn a_drained_browser_error_stays_the_reason_when_later_input_is_refused() {
    const FAILURE: &str = "Chromium disconnected";
    // The host publishes an actor failure and wakes the worker before it marks the session
    // stopped. A browser that cannot open the document reports that, then closes, and the
    // host forwards its Closed event after the error.
    for published in [
        vec![error(FAILURE)],
        vec![error(FAILURE), event(api::EventKind::Closed)],
    ] {
        let (mut worker, output) = web_worker(Stopped::after(published));
        output.finish_open(Ok(()));
        worker.poll_web(&output);
        assert_eq!(worker.status, FAILURE);
        assert_eq!(statuses(&output), [FAILURE]);
        for input in [ViewerInput::Scroll { x: 0.0, y: 40.0 }, search()] {
            worker.input(input, &output);
            worker.poll_web(&output);
        }
        assert_eq!(
            worker.status, FAILURE,
            "a refusal does not replace the reason"
        );
        assert!(statuses(&output).is_empty());
    }
}

#[test]
fn a_browser_that_closes_itself_is_reported_once_without_waiting_for_input() {
    let (mut worker, output) = web_worker(Stopped::after(vec![event(api::EventKind::Closed)]));
    worker.poll_web(&output);
    assert_eq!(worker.status, BROWSER_CLOSED);
    assert_eq!(
        output.take_open_result(),
        Some(Err(BROWSER_CLOSED.into())),
        "a browser that closed before its first frame fails the opening"
    );
    assert_eq!(statuses(&output), [BROWSER_CLOSED]);
    worker.input(ViewerInput::Scroll { x: 0.0, y: 40.0 }, &output);
    worker.poll_web(&output);
    assert_eq!(worker.status, BROWSER_CLOSED);
    assert!(
        statuses(&output).is_empty(),
        "a later refusal adds no report"
    );
}

#[test]
fn the_closure_of_a_painted_page_is_reported_after_its_last_frame() {
    let (mut worker, output) = web_worker(Stopped::after(Vec::new()));
    let (width, height) = (worker.geometry.width, worker.geometry.height);
    let frame = event(api::EventKind::Frame(api::Frame {
        viewport_revision: 1,
        acknowledged_input: 0,
        width,
        height,
        rgba: vec![0; width as usize * height as usize * 4],
    }));
    worker.web.as_mut().unwrap().session = Stopped::after(vec![
        event(api::EventKind::Ready),
        frame,
        event(api::EventKind::Closed),
    ]);
    worker.poll_web(&output);
    assert_eq!(worker.status, BROWSER_CLOSED);
    let outputs = output.take();
    let frame = outputs
        .iter()
        .position(|output| matches!(output, ViewerOutput::Frame(_)));
    let report = outputs.iter().position(
        |output| matches!(output, ViewerOutput::Status(status) if status == BROWSER_CLOSED),
    );
    assert!(
        frame.is_some() && frame < report,
        "the frame's status line must not replace the report"
    );
}

#[test]
fn output_after_a_browser_error_shows_that_the_session_outlived_it() {
    const COPY: &str = "Selected text exceeds the 48 KiB copy budget; select a smaller range.";
    // An oversized Copy does not end the session. The page then stops without a reason.
    let state = event(api::EventKind::State(api::State {
        title: String::new(),
        url: String::new(),
        loading: false,
        editable: false,
        zoom: 1.0,
    }));
    let (mut worker, output) = web_worker(Stopped::after(vec![error(COPY), state]));
    output.finish_open(Ok(()));
    worker.poll_web(&output);
    assert_eq!(statuses(&output), [COPY]);
    worker.input(ViewerInput::Scroll { x: 0.0, y: 40.0 }, &output);
    assert_eq!(worker.status, BROWSER_CLOSED);
    assert_eq!(statuses(&output), [BROWSER_CLOSED]);
}

#[test]
fn policy_controls_keep_file_scope_across_fresh_browser_contexts() {
    let mut frames = crate::interaction::Frames::default();
    let file = frames.painted().unwrap();
    frames.invalidate();
    let context = frames.painted().unwrap();
    let policy = api::CommandKind::Policy(api::Policy {
        network: true,
        scripts: true,
    });
    assert!(accepts_input(&frames, file, file, &policy));
    assert!(!accepts_input(&frames, context, file, &key(true)));
    assert!(accepts_input(&frames, context, context, &key(true)));
}
