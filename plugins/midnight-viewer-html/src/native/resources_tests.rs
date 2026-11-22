use super::*;

fn document() -> (Arc<Document>, crate::output::Receiver) {
    let (output, events) = crate::output::channel();
    let viewport = api::Viewport {
        revision: 1,
        width: 1,
        height: 1,
        scale: 1.0,
    };
    let doc = Document::new(
        api::Header {
            session: 1,
            document: 1,
            sequence: 1,
        },
        api::Resource {
            id: 1,
            length: 0,
            mime: "text/html".into(),
        },
        crate::policy::DocumentPolicy::new(1, false, false).unwrap(),
        viewport,
        output,
        false,
        false,
    )
    .unwrap();
    (Arc::new(doc), events)
}

fn resolves(events: &crate::output::Receiver) -> Vec<(u64, String)> {
    let mut found = Vec::new();
    while let Ok(event) = events.controls.try_recv() {
        match event.kind {
            api::EventKind::ResourceResolve { request, url, .. } => found.push((request, url)),
            other => panic!("unexpected {other:?}"),
        }
    }
    found
}

#[test]
fn canceled_resolution_releases_late_resource_without_releasing_main_authority() {
    let (doc, events) = document();
    let stream = Arc::new(Mutex::new(Stream::new(&doc, None)));
    doc.requests
        .lock()
        .unwrap()
        .active
        .insert(7, stream.clone());
    let handler = CapturedResource::new(
        doc.clone(),
        stream,
        format!("{}a.css", doc.policy.main_url()),
    );
    handler.cancel();
    assert!(doc.requests.lock().unwrap().active.is_empty());
    reply(
        &doc,
        api::ResourceReply::Resolved {
            request: 7,
            resource: api::Resource {
                id: 42,
                length: 20,
                mime: "text/css".into(),
            },
        },
    );
    let (output, mut receiver) = (doc.output.clone(), events);
    output.finish();
    assert_eq!(
        receiver.next(&output).unwrap().kind,
        api::EventKind::ResourceRelease { resource: 42 }
    );
    reply(
        &doc,
        api::ResourceReply::Resolved {
            request: 8,
            resource: doc.source.clone(),
        },
    );
    reply(
        &doc,
        api::ResourceReply::Data {
            request: 9,
            resource: 42,
            offset: 0,
            bytes: vec![1],
        },
    );
    reply(
        &doc,
        api::ResourceReply::Denied {
            request: 10,
            reason: "canceled".into(),
        },
    );
    assert!(receiver.next(&output).is_none());
}

#[test]
fn excess_captured_requests_wait_for_a_slot_instead_of_being_cancelled() {
    let (doc, events) = document();
    // An offline gallery with 200 images: requests beyond the window used to be
    // cancelled, and 128 in flight overflowed the 64-entry control queue.
    let streams: Vec<_> = (0..200)
        .map(|_| Arc::new(Mutex::new(Stream::new(&doc, None))))
        .collect();
    for (index, stream) in streams.iter().enumerate() {
        let url = format!("img/{index}.png");
        assert!(doc.request(stream.clone(), HostRequest::Resolve { url }));
    }
    assert!(!doc.output.failed.load(Ordering::Acquire));
    let first = resolves(&events);
    assert_eq!(first.len(), MAX_IN_FLIGHT);
    assert_eq!(first[0], (1, "img/0.png".into()));
    // Each reply frees one slot for the oldest waiting request.
    let mut admitted = Vec::new();
    for (request, _) in first {
        reply(
            &doc,
            api::ResourceReply::Denied {
                request,
                reason: "missing".into(),
            },
        );
        admitted.extend(resolves(&events));
    }
    assert_eq!(admitted.len(), MAX_IN_FLIGHT);
    assert_eq!(admitted[0].1, format!("img/{MAX_IN_FLIGHT}.png"));
    // A cancelled waiting request is forgotten and never sent.
    let waiting = streams[2 * MAX_IN_FLIGHT].clone();
    let handler = CapturedResource::new(
        doc.clone(),
        waiting,
        format!("{}img/x.png", doc.policy.main_url()),
    );
    handler.cancel();
    let next = admitted[0].0;
    reply(
        &doc,
        api::ResourceReply::Denied {
            request: next,
            reason: "missing".into(),
        },
    );
    assert_eq!(
        resolves(&events)
            .into_iter()
            .map(|(_, url)| url)
            .collect::<Vec<_>>(),
        [format!("img/{}.png", 2 * MAX_IN_FLIGHT + 1)]
    );
    assert!(!doc.output.failed.load(Ordering::Acquire));
}

#[test]
fn requests_that_cannot_be_encoded_are_refused_before_reaching_the_host() {
    let (doc, events) = document();
    let stream = Arc::new(Mutex::new(Stream::new(&doc, None)));
    let read = HostRequest::Read {
        resource: 3,
        offset: 0,
        length: 0,
    };
    assert!(!doc.request(stream.clone(), read));
    let url = "a".repeat(70_000);
    assert!(!doc.request(stream, HostRequest::Resolve { url }));
    assert!(resolves(&events).is_empty());
    assert!(doc.requests.lock().unwrap().active.is_empty());
}

#[test]
fn navigation_shares_the_in_flight_bound_and_its_reply_admits_waiting_assets() {
    let (doc, events) = document();
    let now = Instant::now();
    {
        let mut navigation = doc.navigation.lock().unwrap();
        assert!(navigation.begin(
            admission::Intent {
                binding: admission::Binding {
                    session: 1,
                    document: 1,
                    browser: 1,
                    frame: "main".into(),
                    url: format!("{}page.html", doc.policy.main_url()),
                },
                root: false,
            },
            now
        ));
        assert!(matches!(
            navigation.next_effect(now, 900, true),
            Some(admission::Effect::Resolve { request: 900, .. })
        ));
    }
    let streams: Vec<_> = (0..MAX_IN_FLIGHT)
        .map(|_| Arc::new(Mutex::new(Stream::new(&doc, None))))
        .collect();
    for (index, stream) in streams.iter().enumerate() {
        assert!(doc.request(
            stream.clone(),
            HostRequest::Resolve {
                url: format!("img/{index}.png"),
            }
        ));
    }
    assert_eq!(resolves(&events).len(), MAX_IN_FLIGHT - 1);
    assert_eq!(doc.requests.lock().unwrap().waiting.len(), 1);
    reply(
        &doc,
        api::ResourceReply::Denied {
            request: 900,
            reason: "missing".into(),
        },
    );
    assert_eq!(resolves(&events).len(), 1);
    let requests = doc.requests.lock().unwrap();
    assert_eq!(requests.active.len(), MAX_IN_FLIGHT);
    assert!(requests.waiting.is_empty());
    assert!(!doc.output.failed.load(Ordering::Acquire));
}

#[test]
fn captured_paths_stay_below_the_document_root_and_inside_the_url_budget() {
    let policy = crate::policy::DocumentPolicy::new(3, false, false).unwrap();
    let main = policy.main_url();
    assert_eq!(
        policy
            .captured_path(&format!("{main}assets/site.css?v=1"))
            .as_deref(),
        Some("assets/site.css?v=1")
    );
    let longest = "a".repeat(crate::policy::MAX_RESOURCE_URL_BYTES);
    assert_eq!(
        policy.captured_path(&format!("{main}{longest}")),
        Some(longest.clone())
    );
    // A 70,000-character relative URL used to terminate the whole provider.
    assert_eq!(policy.captured_path(&format!("{main}{longest}a")), None);
    assert_eq!(policy.captured_path(&main), None);
    // "../css/site.css" from the document resolves above its root: never substituted.
    assert_eq!(
        policy.captured_path(&format!("{}/css/site.css", policy.origin())),
        None
    );
}
