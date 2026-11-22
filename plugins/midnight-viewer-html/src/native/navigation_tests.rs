use super::*;

#[test]
fn marker_is_exactly_one_bounded_private_header_not_first_value_only() {
    assert_eq!(marker_from_headers(vec![]), Marker::Absent);
    assert_eq!(
        marker_from_headers(vec![(MARKER_HEADER.into(), vec!["attempt-1".into()])]),
        Marker::Value("attempt-1".into())
    );
    for headers in [
        vec![(MARKER_HEADER.into(), vec![String::new()])],
        vec![(MARKER_HEADER.into(), vec!["a".into(), "b".into()])],
        vec![
            (MARKER_HEADER.into(), vec!["a".into()]),
            (MARKER_HEADER.to_ascii_uppercase(), vec!["b".into()]),
        ],
        vec![(MARKER_HEADER.into(), vec!["x".repeat(129)])],
        vec![(MARKER_HEADER.into(), vec!["not\na marker".into()])],
    ] {
        assert_eq!(marker_from_headers(headers), Marker::Invalid);
    }
}

#[test]
fn stripping_removes_all_marker_spellings_without_empty_header_or_other_header_loss() {
    let headers = vec![
        ("Accept".into(), vec!["text/html".into()]),
        (MARKER_HEADER.into(), vec!["a".into()]),
        (MARKER_HEADER.to_ascii_uppercase(), vec!["b".into()]),
        ("Custom".into(), vec!["one".into(), "two".into()]),
    ];
    assert_eq!(
        without_marker(headers),
        vec![
            ("Accept".into(), vec!["text/html".into()]),
            ("Custom".into(), vec!["one".into(), "two".into()]),
        ]
    );
}

#[test]
fn fragment_bypass_is_only_current_document_fragment_change() {
    assert!(same_document_fragment(
        "https://mewv-1.invalid/a",
        "https://mewv-1.invalid/a#part"
    ));
    assert!(same_document_fragment(
        "https://mewv-1.invalid/a#old",
        "https://mewv-1.invalid/a#new"
    ));
    assert!(!same_document_fragment(
        "https://mewv-1.invalid/a",
        "https://mewv-1.invalid/a"
    ));
    assert!(!same_document_fragment(
        "https://mewv-1.invalid/a",
        "https://mewv-1.invalid/b#part"
    ));
    assert!(!same_document_fragment(
        "https://mewv-1.invalid/a",
        "https://mewv-2.invalid/a#part"
    ));
    assert!(!same_document_fragment(
        "https://mewv-1.invalid/a?q=1",
        "https://mewv-1.invalid/a?q=2#part"
    ));
}

#[test]
fn clean_reload_key_is_explicit_user_action_without_modifier_or_release_confusion() {
    let mut key = api::Key {
        key: "F5".into(),
        code: "F5".into(),
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    };
    assert!(reload_key(&key));
    key.pressed = false;
    assert!(
        reload_key(&key),
        "both halves are consumed; only keydown begins the intent"
    );
    key.code = "KeyR".into();
    key.key = "r".into();
    assert!(!reload_key(&key));
    key.modifiers.control = true;
    assert!(reload_key(&key));
    key.modifiers.alt = true;
    assert!(!reload_key(&key));
    key.modifiers.alt = false;
    key.modifiers.meta = true;
    assert!(!reload_key(&key));
}

#[test]
fn bootstrap_root_is_a_single_admission_exception_not_a_root_whitelist() {
    let bootstrap = AtomicBool::new(true);
    assert!(!take_bootstrap(
        &bootstrap,
        "https://mewv-1.invalid/",
        "https://mewv-1.invalid/a"
    ));
    assert!(take_bootstrap(
        &bootstrap,
        "https://mewv-1.invalid/",
        "https://mewv-1.invalid/"
    ));
    assert!(!take_bootstrap(
        &bootstrap,
        "https://mewv-1.invalid/",
        "https://mewv-1.invalid/"
    ));
}

#[test]
fn exact_per_request_completion_releases_a_claim_before_body_handler_only_once() {
    let (doc, mut events) = resources::tests::document();
    let response = resources::tests::admitted(&doc);
    let request = RequestAdmission::Bound(Arc::new(BoundRequest {
        binding: admission::Binding {
            session: 1,
            document: 1,
            browser: 1,
            frame: "main".into(),
            url: format!("{}a", resources::tests::main_url()),
        },
        marker: "private-attempt".into(),
        native_request: 77,
        stripped: AtomicBool::new(false),
        response: Mutex::new(Some(response)),
    }));
    let duplicate = request.clone();
    request.completed(Some(78));
    assert!(resources::tests::written(&doc, &mut events).is_none());
    duplicate.completed(Some(77));
    assert_eq!(
        resources::tests::written(&doc, &mut events).unwrap(),
        api::EventKind::ResourceRelease { resource: 42 }
    );
    request.completed(Some(77));
    drop(duplicate);
    drop(request);
    assert!(resources::tests::written(&doc, &mut events).is_none());
}

#[test]
fn ordinary_network_return_cancels_latest_but_preserves_allowed_before_io_claim() {
    let (doc, mut events) = resources::tests::document();
    let mut document = Arc::try_unwrap(doc).ok().unwrap();
    document.policy = crate::policy::DocumentPolicy::new(1, true, false).unwrap();
    let doc = Arc::new(document);
    let now = Instant::now();
    let main = resources::tests::main_url();
    let bound = admission::Binding {
        session: 1,
        document: 1,
        browser: 1,
        frame: "main".into(),
        url: main.clone(),
    };
    let issued = {
        let mut nav = doc.navigation.lock().unwrap();
        assert!(nav.begin(
            admission::Intent {
                binding: bound.clone(),
                root: true
            },
            now
        ));
        assert_eq!(
            nav.next_effect(now, 100, true),
            Some(admission::Effect::Read {
                request: 100,
                resource: 1,
                offset: 0,
                length: 1,
            })
        );
        nav.receive(
            api::ResourceReply::Data {
                request: 100,
                resource: 1,
                offset: 0,
                bytes: vec![],
            },
            now,
        )
        .unwrap();
        let issued = nav.next_reissue(now).unwrap();
        assert!(nav.allow(&issued.marker, &bound, now));
        assert!(nav.begin(
            admission::Intent {
                binding: bound.clone(),
                root: true
            },
            now
        ));
        issued
    };
    let facts = crate::navigation_route::BrowseFacts {
        unmarked: true,
        target: &main,
        main_frame: true,
        current_main_url: Some("http://127.0.0.1:32123/roundtrip"),
        get_no_post: true,
        bootstrap_unused: false,
        same_document_fragment: false,
        redirect: true,
        direct_load: true,
        user_gesture: false,
        links: false,
    };
    assert_eq!(
        crate::navigation_route::classify(&doc.policy, &facts),
        crate::navigation_route::BrowseRoute::Original
    );
    assert_eq!(allow_original(&doc, true), 0);
    let response = {
        let mut nav = doc.navigation.lock().unwrap();
        assert_eq!(nav.unclaimed_allowed_attempt(), Some(issued.attempt));
        nav.claim(&issued.marker, &bound).unwrap()
    };
    drop(ResponseLease::new(response, &doc));
    assert!(
        doc.navigation
            .lock()
            .unwrap()
            .next_effect(now, 101, true)
            .is_none(),
        "the superseded coalesced intent must not prepare after original return"
    );
    assert!(
        resources::tests::written(&doc, &mut events).is_none(),
        "main authority is never a derived release"
    );
}
