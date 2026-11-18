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
