use super::*;

fn source_header() -> api::Header {
    api::Header {
        session: 11,
        document: 23,
        sequence: 47,
    }
}

fn source_resolve(url: &str) -> api::Event {
    api::Event {
        header: source_header(),
        kind: api::EventKind::ResourceResolve {
            request: 71,
            base: 42,
            url: url.into(),
        },
    }
}

fn source_read(offset: u64, length: u32) -> api::Event {
    api::Event {
        header: source_header(),
        kind: api::EventKind::ResourceRead {
            request: 71,
            resource: 42,
            offset,
            length,
        },
    }
}

fn source_diagnostic() -> api::Event {
    api::Event {
        header: source_header(),
        kind: api::EventKind::Error {
            code: api::ErrorCode::ResourceChanged,
            message: SOURCE_CHANGE_NOTICE.into(),
        },
    }
}

fn source_frame() -> api::Event {
    api::Event {
        header: api::Header {
            sequence: 49,
            ..source_header()
        },
        kind: api::EventKind::Frame(api::Frame {
            viewport_revision: 3,
            acknowledged_input: 0,
            width: 1,
            height: 1,
            rgba: vec![255; 4],
        }),
    }
}

#[test]
fn office_source_notice_accepts_adjacent_resolve_or_read_pair_once() {
    for origin in [
        source_resolve("office/v1/sheet/0/row/129/col/1.html"),
        source_resolve("/office/v1/document/0/block/0.html"),
        source_read(65_536, 256 * 1024),
    ] {
        let mut source = SourceChange::default();
        assert!(!source.observe(&origin, true));
        assert!(source.observe(&source_diagnostic(), true));
        assert!(source.reported);
        assert!(
            !source.observe(&source_diagnostic(), true),
            "duplicate diagnostic remains generic error"
        );
        assert!(!source.observe(&origin, true));
        assert!(
            !source.observe(&source_diagnostic(), true),
            "duplicate transition remains generic error"
        );
    }
}

#[test]
fn office_source_notice_origin_may_cross_drain_boundary_but_not_intervening_event() {
    let origin = source_resolve("office/v1/slide/0/block/0.html");
    let mut source = SourceChange::default();
    // Keeping the same bounded state models the next drain call; a poll boundary
    // is not an event and does not consume the stored genuine origin header.
    assert!(!source.observe(&origin, true));
    assert!(source.pending.is_some());
    assert!(source.observe(&source_diagnostic(), true));

    for intervening in [
        source_frame(),
        api::Event {
            header: source_header(),
            kind: api::EventKind::FindResult(api::FindResult {
                request_sequence: 1,
                matches: 1,
                active: 1,
                complete: true,
            }),
        },
    ] {
        let mut source = SourceChange::default();
        assert!(!source.observe(&origin, true));
        assert!(!source.observe(&intervening, true));
        assert!(!source.observe(&source_diagnostic(), true));
        assert!(!source.reported);
    }
}

#[test]
fn office_source_notice_rejects_unpaired_wrong_header_message_and_other_error_codes() {
    let origin = source_resolve("office/v1/catalog.html");
    let mut source = SourceChange::default();
    assert!(!source.observe(&source_diagnostic(), true));
    for header in [
        api::Header {
            session: 12,
            ..source_header()
        },
        api::Header {
            document: 24,
            ..source_header()
        },
        api::Header {
            sequence: 48,
            ..source_header()
        },
    ] {
        let mut source = SourceChange::default();
        assert!(!source.observe(&origin, true));
        let mut error = source_diagnostic();
        error.header = header;
        assert!(!source.observe(&error, true));
        assert!(!source.reported);
    }
    for kind in [
        api::EventKind::Error {
            code: api::ErrorCode::Runtime,
            message: SOURCE_CHANGE_NOTICE.into(),
        },
        api::EventKind::Error {
            code: api::ErrorCode::Denied,
            message: SOURCE_CHANGE_NOTICE.into(),
        },
        api::EventKind::Error {
            code: api::ErrorCode::ResourceChanged,
            message: "untrusted source text".into(),
        },
    ] {
        let mut source = SourceChange::default();
        assert!(!source.observe(&origin, true));
        assert!(!source.observe(
            &api::Event {
                header: source_header(),
                kind
            },
            true
        ));
        assert!(!source.reported);
    }
}

#[test]
fn office_source_notice_rejects_duplicate_origins_and_ordinary_html_pairs() {
    let origin = source_resolve("office/v1/sheet/0/first.html");
    let mut source = SourceChange::default();
    assert!(!source.observe(&origin, true));
    assert!(!source.observe(&origin, true));
    assert!(!source.observe(&source_diagnostic(), true));
    assert!(!source.reported);

    let mut newer = origin.clone();
    newer.header.sequence += 1;
    let mut newest = newer.clone();
    newest.header.sequence += 1;
    for chain in [
        vec![origin.clone(), newer.clone()],
        vec![origin.clone(), newer, newest],
    ] {
        let mut source = SourceChange::default();
        let mut diagnostic = source_diagnostic();
        for origin in chain {
            diagnostic.header = origin.header;
            assert!(!source.observe(&origin, true));
        }
        assert!(
            !source.observe(&diagnostic, true),
            "adjacent higher-sequence origins are not a pair"
        );
        assert!(!source.reported);
    }

    let mut html = SourceChange::default();
    assert!(!html.observe(&origin, false));
    assert!(!html.observe(&source_diagnostic(), false));
    assert!(!html.reported);
}

#[test]
fn office_source_origin_accepts_bounded_office_metadata_without_revalidating_targets() {
    for path in [
        "office/v1/catalog.html",
        "office/v1/document/0/block/0.html",
        "office/v1/document/0/block/1048575.html",
        "office/v1/slide/4095/block/1048575.html",
        "office/v1/sheet/1023/first.html",
        "office/v1/sheet/0/row/1048576/col/16384.html",
        "office/v1/sheet/65535/first.html",
        "office/v1/document/1/block/1048576.html",
        "office/v1/future-content/12/view.html",
        "office/v1/catalog.html/extra",
    ] {
        assert!(valid_source_origin(&source_resolve(path)), "{path}");
        assert!(
            valid_source_origin(&source_resolve(&format!("/{path}"))),
            "{path}"
        );
    }
}

#[test]
fn office_source_origin_rejects_unbounded_or_foreign_metadata_and_zero_ids() {
    for path in [
        "",
        "//office/v1/catalog.html",
        "office/v1/catalog.html?x=1",
        "office/v1/catalog.html#x",
        "https://host/office/v1/catalog.html",
        "file:/x",
        "../office/v1/catalog.html",
        "office\\v1\\catalog.html",
        "office/v1/%63atalog.html",
        "office/v1/../catalog.html",
        "office/v1/./catalog.html",
        "office/v1//catalog.html",
        "office/v1/catalog.html/",
        "office/v1/catalog.html\u{1b}",
        "office/v1/článek.html",
        "office/v1/catalog.html\0",
    ] {
        let origin = source_resolve(path);
        assert!(!valid_source_origin(&origin), "{path:?}");
        let mut source = SourceChange::default();
        assert!(!source.observe(&origin, true));
        assert!(!source.observe(&source_diagnostic(), true));
    }
    assert!(!valid_source_origin(&source_resolve(&"x".repeat(1025))));
    for header in [
        api::Header {
            session: 0,
            ..source_header()
        },
        api::Header {
            document: 0,
            ..source_header()
        },
        api::Header {
            sequence: 0,
            ..source_header()
        },
    ] {
        for mut origin in [source_resolve("office/v1/catalog.html"), source_read(0, 1)] {
            origin.header = header;
            assert!(!valid_source_origin(&origin));
        }
    }
    let mut origin = source_resolve("office/v1/catalog.html");
    origin.kind = api::EventKind::ResourceResolve {
        request: 0,
        base: 42,
        url: "office/v1/catalog.html".into(),
    };
    assert!(!valid_source_origin(&origin));
    origin.kind = api::EventKind::ResourceResolve {
        request: 71,
        base: 0,
        url: "office/v1/catalog.html".into(),
    };
    assert!(!valid_source_origin(&origin));
}

#[test]
fn office_source_origin_metadata_ceiling_includes_optional_leading_slash() {
    let prefix = "office/v1/";
    let at_limit = format!("{prefix}{}", "x".repeat(1024 - prefix.len()));
    assert_eq!(at_limit.len(), 1024);
    assert!(valid_source_origin(&source_resolve(&at_limit)));
    assert!(!valid_source_origin(&source_resolve(&format!(
        "{at_limit}x"
    ))));
    assert!(!valid_source_origin(&source_resolve(&format!(
        "/{at_limit}"
    ))));
    let rooted_at_limit = format!("/{}", &at_limit[..1023]);
    assert_eq!(rooted_at_limit.len(), 1024);
    assert!(valid_source_origin(&source_resolve(&rooted_at_limit)));
}

#[test]
fn office_source_read_origin_requires_real_ids_and_existing_bounded_read_contract() {
    assert!(valid_source_origin(&source_read(0, 1)));
    assert!(valid_source_origin(&source_read(
        u64::MAX - 262_144,
        262_144
    )));
    for origin in [
        source_read(0, 0),
        source_read(0, 262_145),
        source_read(u64::MAX, 1),
    ] {
        assert!(!valid_source_origin(&origin));
    }
    let mut origin = source_read(0, 1);
    origin.kind = api::EventKind::ResourceRead {
        request: 0,
        resource: 42,
        offset: 0,
        length: 1,
    };
    assert!(!valid_source_origin(&origin));
    origin.kind = api::EventKind::ResourceRead {
        request: 71,
        resource: 0,
        offset: 0,
        length: 1,
    };
    assert!(!valid_source_origin(&origin));
}

#[test]
fn office_source_notice_survives_frame_find_and_only_new_view_clears_it() {
    let mut source = SourceChange::default();
    let mut painted_before_diagnostic = source_frame();
    painted_before_diagnostic.header.sequence = 900;
    assert!(!source.observe(&painted_before_diagnostic, true));
    // The echoed origin's real older sequence identifies cause only. It is not
    // compared with this later paint or used as an input/frame acknowledgement.
    assert!(!source.observe(
        &source_resolve("office/v1/sheet/0/row/129/col/1.html"),
        true
    ));
    assert!(source.observe(&source_diagnostic(), true));
    let mut next_frame = source_frame();
    next_frame.header.sequence = 901;
    assert!(!source.observe(&next_frame, true));
    let status =
        web_presentation_status(api::Policy::default(), false, None, source.reported, None);
    assert_eq!(status, SOURCE_CHANGE_NOTICE);
    let find = api::Event {
        header: api::Header {
            sequence: 51,
            ..source_header()
        },
        kind: api::EventKind::FindResult(api::FindResult {
            request_sequence: 50,
            matches: 2,
            active: 1,
            complete: true,
        }),
    };
    assert!(!source.observe(&find, true));
    let status = web_presentation_status(
        api::Policy::default(),
        false,
        Some("Match 1 of 2"),
        source.reported,
        None,
    );
    assert_eq!(status, format!("{SOURCE_CHANGE_NOTICE} · Match 1 of 2"));
    let fresh = SourceChange::default();
    assert_eq!(
        web_presentation_status(api::Policy::default(), false, None, fresh.reported, None),
        "Office preview · Offline · JavaScript off"
    );
    assert_eq!(
        web_presentation_status(
            api::Policy::default(),
            true,
            Some("No matches"),
            false,
            None
        ),
        "Offline · JavaScript off · No matches"
    );
}

#[test]
fn browser_retirement_failure_stays_latched_after_later_successful_cleanup() {
    let mut failure = None;
    assert_eq!(
        record_web_retirement(&mut failure, Err("unconfirmed parent release".into())),
        Err("unconfirmed parent release".into()),
    );
    assert_eq!(
        record_web_retirement(&mut failure, Ok(())),
        Err("unconfirmed parent release".into()),
    );
    assert_eq!(failure.as_deref(), Some("unconfirmed parent release"));
}

#[test]
fn browser_retirement_close_preserves_both_prior_and_new_cleanup_failures() {
    let mut failure = Some("first resource release failed".into());
    let result = record_web_retirement(&mut failure, Err("closing actor cleanup failed".into()));
    assert_eq!(
        result,
        Err("first resource release failed; closing actor cleanup failed".into())
    );
    assert_eq!(
        record_web_retirement(&mut failure, Ok(())),
        Err("first resource release failed; closing actor cleanup failed".into()),
    );
}

fn accepts_input(
    frames: &crate::interaction::Frames,
    first: u64,
    frame: u64,
    command: &api::CommandKind,
) -> bool {
    super::accepts_input(frames, first, frame, command, true)
}

#[test]
fn office_fixed_policy_rejects_all_policy_inputs_before_presentation_changes() {
    let mut frames = crate::interaction::Frames::default();
    let first = frames.painted().unwrap();
    for policy in [
        api::Policy::default(),
        api::Policy {
            network: true,
            scripts: false,
        },
        api::Policy {
            network: true,
            scripts: true,
        },
    ] {
        assert!(!super::accepts_input(
            &frames,
            first,
            first,
            &api::CommandKind::Policy(policy),
            false,
        ));
        assert!(!super::accepts_input(
            &frames,
            first,
            0,
            &api::CommandKind::Policy(policy),
            false,
        ));
    }
    for command in [
        api::CommandKind::Zoom(1.5),
        api::CommandKind::Copy,
        api::CommandKind::Focus(false),
    ] {
        assert!(super::accepts_input(&frames, first, first, &command, false));
    }
}

#[test]
fn office_status_identifies_fixed_preview_and_ordinary_html_keeps_policy_labels() {
    assert_eq!(
        web_status(api::Policy::default(), false),
        "Office preview · Offline · JavaScript off"
    );
    assert_eq!(
        web_status(api::Policy::default(), true),
        "Offline · JavaScript off"
    );
    assert!(
        web_status(
            api::Policy {
                network: true,
                scripts: false
            },
            true
        )
        .starts_with("Online")
    );
    assert!(
        web_status(
            api::Policy {
                network: true,
                scripts: true
            },
            true
        )
        .contains("scripts can read")
    );
}

#[test]
fn denied_action_notice_keeps_web_status_through_frame_and_late_find() {
    let mut notice = super::ActionNotice::default();
    assert!(notice.observe(&api::EventKind::Error {
        code: api::ErrorCode::Denied,
        message: "Cannot prepare this target; previous contents remain available.".into(),
    }));
    assert!(!notice.observe(&api::EventKind::State(api::State {
        title: String::new(),
        url: String::new(),
        loading: false,
        editable: false,
        zoom: 1.0,
    })));
    assert!(!notice.observe(&api::EventKind::Frame(api::Frame {
        viewport_revision: 1,
        acknowledged_input: 0,
        width: 1,
        height: 1,
        rgba: vec![255; 4],
    })));
    let status = super::web_presentation_status(
        api::Policy::default(),
        true,
        Some("Match 1 of 1"),
        false,
        notice.text(),
    );
    assert!(status.starts_with("Cannot prepare this target; previous contents remain available."));
    assert!(status.ends_with("Match 1 of 1"));
}

#[test]
fn denied_action_notice_is_bounded_and_unrelated_errors_remain_strict() {
    let mut notice = super::ActionNotice::default();
    for message in [String::new(), "x".repeat(513), "line\nline".into()] {
        assert!(notice.observe(&api::EventKind::Error {
            code: api::ErrorCode::Denied,
            message,
        }));
        assert_eq!(notice.text(), Some(super::ACTION_REFUSED_NOTICE));
    }
    for code in [
        api::ErrorCode::Runtime,
        api::ErrorCode::ResourceChanged,
        api::ErrorCode::BudgetExceeded,
        api::ErrorCode::InvalidRequest,
    ] {
        assert!(!notice.observe(&api::EventKind::Error {
            code,
            message: "not an action refusal".into(),
        }));
    }
    assert!(!notice.observe(&api::EventKind::Closed));
    assert_eq!(notice.text(), Some(super::ACTION_REFUSED_NOTICE));
}

#[test]
fn action_notice_clears_for_admitted_user_actions_and_keeps_sticky_source_notice() {
    let mut notice = super::ActionNotice::default();
    let denied = api::EventKind::Error {
        code: api::ErrorCode::Denied,
        message: "Navigation was refused.".into(),
    };
    for command in [
        key(true),
        pointer(api::PointerKind::Down),
        api::CommandKind::Copy,
        api::CommandKind::Text("a".into()),
        api::CommandKind::Find(api::Find {
            query: "cell".into(),
            forward: true,
            match_case: true,
            find_next: false,
        }),
    ] {
        assert!(notice.observe(&denied));
        assert!(notice.user_action(&command));
        assert_eq!(notice.text(), None);
    }
    notice.observe(&denied);
    for command in [
        key(false),
        pointer(api::PointerKind::Move),
        api::CommandKind::Focus(true),
        api::CommandKind::Resize(api::Viewport {
            revision: 2,
            width: 900,
            height: 600,
            scale: 1.0,
        }),
    ] {
        assert!(!notice.user_action(&command));
        assert_eq!(notice.text(), Some("Navigation was refused."));
    }
    let sticky = super::web_presentation_status(
        api::Policy::default(),
        false,
        Some("No matches"),
        true,
        notice.text(),
    );
    assert_eq!(
        sticky,
        format!("{} · No matches", super::SOURCE_CHANGE_NOTICE)
    );
    assert!(notice.user_action(&api::CommandKind::Copy));
    assert_eq!(
        super::web_presentation_status(api::Policy::default(), false, None, true, notice.text(),),
        super::SOURCE_CHANGE_NOTICE
    );
}

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
    fn policy_change_allowed(&self) -> bool {
        true
    }
    fn release_on_worker(self: Box<Self>) -> Result<(), String> {
        Ok(())
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
        whole_words: false,
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

/// A running browser that records the commands it accepts and returns queued output.
#[derive(Clone, Default)]
struct Recording(std::sync::Arc<std::sync::Mutex<(Vec<api::CommandKind>, Vec<api::Event>)>>);

impl Recording {
    fn finds(&self) -> Vec<api::Find> {
        let mut state = self.0.lock().unwrap();
        state
            .0
            .drain(..)
            .filter_map(|command| match command {
                api::CommandKind::Find(find) => Some(find),
                _ => None,
            })
            .collect()
    }

    fn reply(&self, matches: u32, active: u32) {
        self.0
            .lock()
            .unwrap()
            .1
            .push(event(api::EventKind::FindResult(api::FindResult {
                request_sequence: 1,
                matches,
                active,
                complete: true,
            })));
    }
}

impl Browser for Recording {
    fn send(&self, command: api::CommandKind) -> bool {
        self.0.lock().unwrap().0.push(command);
        true
    }
    fn drain(&self, maximum: usize) -> Vec<api::Event> {
        let mut state = self.0.lock().unwrap();
        let count = maximum.min(state.1.len());
        state.1.drain(..count).collect()
    }
    fn close(&self) {}
    fn finished(&self) -> bool {
        false
    }
    fn policy_change_allowed(&self) -> bool {
        true
    }
    fn release_on_worker(self: Box<Self>) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn every_search_asks_chromium_to_activate_a_match_and_the_status_counts_in_good_english() {
    let browser = Recording::default();
    let (mut worker, output) = web_worker(Box::new(browser.clone()));
    output.finish_open(Ok(()));
    // A new session would only highlight the matches and report none active; a
    // follow-up activates the nearest one, also for a changed query.
    worker.input(search(), &output);
    worker.input(search(), &output);
    worker.input(
        ViewerInput::Search {
            query: "other".into(),
            backwards: true,
            case_sensitive: true,
            whole_words: false,
        },
        &output,
    );
    let finds = browser.finds();
    assert_eq!(finds.len(), 3);
    assert!(finds.iter().all(|find| find.find_next), "{finds:?}");
    assert!(!finds[2].forward && finds[2].match_case);
    statuses(&output);
    for (matches, active, status) in [
        (0, 0, "No matches"),
        (1, 0, "1 match"),
        (3, 0, "3 matches"),
        (1, 1, "Match 1 of 1"),
        (3, 2, "Match 2 of 3"),
    ] {
        browser.reply(matches, active);
        worker.poll_web(&output);
        // After the page's state, as each frame's status shows it.
        assert_eq!(
            statuses(&output),
            [format!("Offline · JavaScript off · {status}")],
            "{matches} matches, active {active}"
        );
    }
}

/// A running Office preview: the fixed offline policy, and the output queued for it.
#[derive(Clone, Default)]
struct OfficePreview(std::sync::Arc<std::sync::Mutex<Vec<api::Event>>>);

impl Browser for OfficePreview {
    fn send(&self, _: api::CommandKind) -> bool {
        true
    }
    fn drain(&self, maximum: usize) -> Vec<api::Event> {
        let mut events = self.0.lock().unwrap();
        let count = maximum.min(events.len());
        events.drain(..count).collect()
    }
    fn close(&self) {}
    fn finished(&self) -> bool {
        false
    }
    fn policy_change_allowed(&self) -> bool {
        false
    }
    fn release_on_worker(self: Box<Self>) -> Result<(), String> {
        Ok(())
    }
}

/// The status line after these outputs: the last status, from a status or a frame.
fn shown_status(output: &Mailbox) -> Option<String> {
    output
        .take()
        .into_iter()
        .rev()
        .find_map(|output| match output {
            ViewerOutput::Status(status) => Some(status),
            ViewerOutput::Frame(frame) => Some(frame.status),
            _ => None,
        })
}

/// Chromium's reply to Find may come before or after the page's next frame, whatever the
/// document. Either way the status line names the preview, as for each format.
#[test]
fn office_find_status_names_the_preview_for_every_format_whatever_paints_last() {
    for document in ["padded.docx", "padded.pptx", "padded.xlsx"] {
        for frame_first in [true, false] {
            let preview = OfficePreview::default();
            let (mut worker, output) = web_worker(Box::new(preview.clone()));
            output.finish_open(Ok(()));
            worker.caption = format!("{document} — Midnight Explorer");
            worker.input(search(), &output);
            let (width, height) = (worker.geometry.width, worker.geometry.height);
            let frame = event(api::EventKind::Frame(api::Frame {
                viewport_revision: 1,
                acknowledged_input: 0,
                width,
                height,
                rgba: vec![0; width as usize * height as usize * 4],
            }));
            let found = event(api::EventKind::FindResult(api::FindResult {
                request_sequence: 1,
                matches: 1,
                active: 1,
                complete: true,
            }));
            *preview.0.lock().unwrap() = if frame_first {
                vec![frame, found]
            } else {
                vec![found, frame]
            };
            worker.poll_web(&output);
            assert_eq!(
                shown_status(&output).as_deref(),
                Some("Office preview · Offline · JavaScript off · Match 1 of 1"),
                "{document}, frame first: {frame_first}"
            );
        }
    }
}

const OLDER_FORMAT: &str = "This presentation is password-protected or saved in the older \
                            binary Office format (.doc, .xls or .ppt), which the preview \
                            cannot open.";

/// The notices among `outputs`.
fn notices(output: &Mailbox) -> Vec<Option<ViewerSandboxNotice>> {
    output
        .take()
        .into_iter()
        .filter_map(|output| match output {
            ViewerOutput::SandboxNotice(notice) => Some(notice),
            _ => None,
        })
        .collect()
}

/// A worker showing the file at `path`, of these bytes.
fn showing(path: &std::path::Path, bytes: &[u8]) -> (Worker, Mailbox) {
    std::fs::write(path, bytes).unwrap();
    let (mut worker, output) = crate::worker::tests::worker();
    worker.files = Arc::from([path.to_path_buf()]);
    (worker, output)
}

/// An Office refusal with diagnostic details: the caller puts its message on the status
/// line, and the details stay available in the page's place until another document opens.
#[test]
fn an_office_refusal_offers_its_details_in_the_pages_place_until_another_document_opens() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("protected.pptx");
    let mut compound = vec![0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
    compound.resize(4096, 0);
    let (mut worker, output) = showing(&path, &compound);
    let file = midnight_plugin_host::CapturedFile::capture(&path).unwrap();
    let Err(message) = worker.prepare_office(file, &output) else {
        panic!("a compound file prepared as a presentation");
    };
    assert_eq!(message, OLDER_FORMAT);
    assert_eq!(
        notices(&output),
        [Some(ViewerSandboxNotice {
            message: OLDER_FORMAT.into(),
            installable: false,
            installing: false,
            details: Some(
                "Unsupported Office content: is password-protected or saved in the older \
                 binary Office format (.doc, .xls or .ppt), which the preview cannot open"
                    .into()
            ),
        })]
    );
    worker.open(&output);
    assert_eq!(notices(&output), [None]);
}

/// A failure without diagnostic details stays on the status line alone.
#[test]
fn an_office_failure_without_details_shows_no_notice() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("closing.pptx");
    let (mut worker, output) = showing(&path, b"PK");
    let file = midnight_plugin_host::CapturedFile::capture(&path).unwrap();
    // The window closed before the preview started.
    worker
        .window_closed
        .store(true, std::sync::atomic::Ordering::Release);
    assert!(worker.prepare_office(file, &output).is_err());
    assert!(notices(&output).is_empty());
}

/// Queue `kind` from the browser, let the worker drain it, and return what it published.
fn browser_says(
    browser: &Recording,
    worker: &mut Worker,
    output: &Mailbox,
    kind: api::EventKind,
) -> Vec<ViewerOutput> {
    browser.0.lock().unwrap().1.push(event(kind));
    worker.poll_web(output);
    output.take()
}

/// The page's links take the host's link policy: hovering names the real target, a web
/// link goes to the system's handler, and a refused one stays as the browser's notice
/// through later frames.
#[test]
fn html_links_take_the_host_link_policy_and_refusals_stay_through_frames() {
    let browser = Recording::default();
    let (mut worker, output) = web_worker(Box::new(browser.clone()));
    output.finish_open(Ok(()));
    let folder = tempfile::tempdir().unwrap();
    worker.files = Arc::from([folder.path().join("page.html")]);
    let hover = |outputs: Vec<ViewerOutput>| {
        outputs.into_iter().find_map(|output| match output {
            ViewerOutput::LinkHover(status) => Some(status),
            _ => None,
        })
    };
    let other = folder.path().join("sub").join("other.html");
    assert_eq!(
        hover(browser_says(
            &browser,
            &mut worker,
            &output,
            api::EventKind::LinkHover {
                target: "sub/other.html#part".into()
            }
        )),
        Some(Some(format!("Link to {}", other.display())))
    );
    assert_eq!(
        hover(browser_says(
            &browser,
            &mut worker,
            &output,
            api::EventKind::LinkHover {
                target: String::new()
            }
        )),
        Some(None)
    );
    let outputs = browser_says(
        &browser,
        &mut worker,
        &output,
        api::EventKind::LinkActivated {
            target: "https://example.com/x".into(),
        },
    );
    let requests: Vec<_> = outputs
        .iter()
        .filter_map(|output| match output {
            ViewerOutput::LinkRequest(ViewerLinkRequest::OpenExternal(url)) => {
                Some(url.as_str().to_owned())
            }
            _ => None,
        })
        .collect();
    assert_eq!(requests, ["https://example.com/x"]);
    assert_eq!(worker.status, "Opening in your browser…");
    let outputs = browser_says(
        &browser,
        &mut worker,
        &output,
        api::EventKind::LinkActivated {
            target: "javascript:alert(1)".into(),
        },
    );
    assert!(
        !outputs
            .iter()
            .any(|output| matches!(output, ViewerOutput::LinkRequest(_)))
    );
    const REFUSED: &str = "Links to javascript: addresses are not followed.";
    assert_eq!(worker.status, REFUSED);
    let (width, height) = (worker.geometry.width, worker.geometry.height);
    let frame = api::EventKind::Frame(api::Frame {
        viewport_revision: 1,
        acknowledged_input: 0,
        width,
        height,
        rgba: vec![0; width as usize * height as usize * 4],
    });
    let shown = browser_says(&browser, &mut worker, &output, frame)
        .into_iter()
        .find_map(|output| match output {
            ViewerOutput::Frame(frame) => Some(frame.status),
            _ => None,
        });
    assert_eq!(
        shown.as_deref(),
        Some(REFUSED),
        "the notice outlives a repaint"
    );
}

/// The window asks its next hover question only after an answer: a raster hover that
/// reaches an HTML page (sent before the page replaced the text) is answered with no link.
#[test]
fn a_raster_hover_over_an_html_page_is_answered_with_no_link() {
    let (mut worker, output) = web_worker(Box::new(Recording::default()));
    output.finish_open(Ok(()));
    output.take();
    worker.input(
        ViewerInput::Raster {
            surface: ViewerSurfaceId(1),
            input: ViewerRasterInput::Hover {
                pixel_id: 1,
                origin: [0.0, 0.0],
                x: 10.0,
                y: 10.0,
            },
        },
        &output,
    );
    let answers: Vec<_> = output
        .take()
        .into_iter()
        .filter_map(|output| match output {
            ViewerOutput::LinkHover(status) => Some(status),
            _ => None,
        })
        .collect();
    assert_eq!(answers, [None]);
}
