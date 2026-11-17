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

/// A browser session the host already stopped: it refuses every command. `reason` models
/// a host that publishes why the refused command stopped it, once.
struct Stopped(std::sync::Mutex<(Option<&'static str>, Vec<api::Event>)>);

impl Stopped {
    fn new(reason: Option<&'static str>) -> Box<Self> {
        Box::new(Self(std::sync::Mutex::new((reason, Vec::new()))))
    }
}

impl Browser for Stopped {
    fn send(&self, _: api::CommandKind) -> bool {
        let mut state = self.0.lock().unwrap();
        if let Some(message) = state.0.take() {
            state.1.push(api::Event {
                header: api::Header {
                    session: 1,
                    document: 1,
                    sequence: 1,
                },
                kind: api::EventKind::Error {
                    code: api::ErrorCode::Runtime,
                    message: message.into(),
                },
            });
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
