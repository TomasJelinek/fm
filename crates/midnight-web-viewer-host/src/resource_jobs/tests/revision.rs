use super::*;
use crate::api;

#[derive(Debug)]
struct RevisionSource {
    valid: AtomicBool,
    pause_validation: AtomicBool,
    pause_read: AtomicBool,
    fail_read: AtomicBool,
    gate: Arc<Gate>,
    validations: AtomicUsize,
    reads: AtomicUsize,
    releases: AtomicUsize,
}
impl ContentSource for RevisionSource {
    fn length(&self) -> u64 {
        5
    }
    fn display_name(&self) -> String {
        "window.html".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        self.validations.fetch_add(1, Ordering::SeqCst);
        if self.pause_validation.swap(false, Ordering::AcqRel) {
            self.gate.enter();
        }
        if self.valid.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(Error("parent revision changed".into()))
        }
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.pause_read.swap(false, Ordering::AcqRel) {
            self.gate.enter();
        }
        if self.fail_read.load(Ordering::Acquire) {
            return Err(Error("asset I/O refusal".into()));
        }
        Ok(b"hello"[offset as usize..(offset + u64::from(length)).min(5) as usize].to_vec())
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        self.releases.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Debug)]
struct RevisionScope {
    source: Arc<RevisionSource>,
    captures: AtomicUsize,
    policy: crate::ResourcePathPolicy,
}
impl Resources for RevisionScope {
    fn path_policy(&self) -> crate::ResourcePathPolicy {
        self.policy
    }
    fn capture(&self, _: &[String]) -> Result<CapturedFile, String> {
        self.captures.fetch_add(1, Ordering::SeqCst);
        CapturedFile::from_source(self.source.clone()).map_err(|error| error.to_string())
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), String> {
        Ok(())
    }
}

fn fixture(policy: crate::ResourcePathPolicy) -> (ResourceJobs, Arc<RevisionScope>, Resource) {
    let source = Arc::new(RevisionSource {
        valid: AtomicBool::new(true),
        pause_validation: AtomicBool::new(false),
        pause_read: AtomicBool::new(false),
        fail_read: AtomicBool::new(false),
        gate: Arc::default(),
        validations: AtomicUsize::new(0),
        reads: AtomicUsize::new(0),
        releases: AtomicUsize::new(0),
    });
    let scope = Arc::new(RevisionScope {
        source,
        captures: AtomicUsize::new(0),
        policy,
    });
    let main = CapturedFile::from_source(Arc::new(Bytes {
        name: "index.html".into(),
        releases: Arc::default(),
        read_gate: None,
    }))
    .unwrap();
    let mut jobs = ResourceJobs::new(
        SourceOwner::new(main, Some(scope.clone()), Arc::default()),
        Arc::default(),
    )
    .unwrap();
    let main = jobs.reset(1, false).unwrap().main;
    (jobs, scope, main)
}

fn resolve(jobs: &mut ResourceJobs, main: &Resource) -> u64 {
    assert!(jobs.resolve(200, main.id, "window.html").is_none());
    let replies = receive(jobs, 1);
    let [ResourceReply::Resolved { resource, .. }] = replies.as_slice() else {
        panic!("{replies:?}")
    };
    resource.id
}

#[test]
fn generated_read_parent_validation_failure_revokes_without_browser_drain() {
    for before in [true, false] {
        let (mut jobs, scope, main) = fixture(crate::ResourcePathPolicy::CanonicalRoot);
        let id = resolve(&mut jobs, &main);
        let _open = OpenOnDrop(scope.source.gate.clone());
        scope
            .source
            .pause_validation
            .store(before, Ordering::Release);
        scope.source.pause_read.store(!before, Ordering::Release);
        assert!(jobs.read(201, id, 0, 1).is_none());
        let entered = scope.source.gate.entered();
        assert!(jobs.resolve(202, main.id, "later.html").is_none());
        scope.source.valid.store(false, Ordering::Release);
        scope.source.gate.open();
        let deadline = Instant::now() + Duration::from_secs(3);
        while jobs.generation.token(1).is_ok() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        let revoked_without_drain = jobs.generation.token(1).is_err();
        assert!(!jobs.generation.closed.load(Ordering::Acquire));
        let replies = jobs.drain().unwrap();
        jobs.finish().unwrap();
        assert!(entered && revoked_without_drain, "before={before}");
        assert_eq!(
            scope.captures.load(Ordering::SeqCst),
            1,
            "stale target was captured"
        );
        assert!(
            replies
                .iter()
                .all(|reply| matches!(reply, ResourceReply::Denied { .. }))
        );
        for request in [201, 202] {
            assert_eq!(replies.iter().filter(|reply| matches!(reply, ResourceReply::Denied { request: actual, .. } if *actual == request)).count(), 1);
        }
    }
}

#[test]
fn ordinary_asset_io_error_remains_a_denial_without_revoking_valid_generation() {
    let (mut jobs, scope, main) = fixture(crate::ResourcePathPolicy::RelativeAssets);
    let id = resolve(&mut jobs, &main);
    scope.source.fail_read.store(true, Ordering::Release);
    assert!(jobs.read(210, id, 0, 1).is_none());
    assert!(matches!(
        receive(&mut jobs, 1).as_slice(),
        [ResourceReply::Denied { request: 210, .. }]
    ));
    assert!(jobs.generation.token(1).is_ok());
    scope.source.fail_read.store(false, Ordering::Release);
    assert!(jobs.read(211, id, 0, 1).is_none());
    assert!(
        matches!(receive(&mut jobs, 1).as_slice(), [ResourceReply::Data { request: 211, bytes, .. }] if bytes == b"h")
    );
    jobs.finish().unwrap();
}

#[derive(Debug)]
struct ParentScope {
    captures: AtomicUsize,
    releases: Arc<AtomicUsize>,
    scope_releases: AtomicUsize,
    pause: AtomicBool,
    fail: AtomicBool,
    gate: Arc<Gate>,
    invalidate_capture: Mutex<Option<Arc<RevisionSource>>>,
}

impl Resources for ParentScope {
    fn path_policy(&self) -> crate::ResourcePathPolicy {
        crate::ResourcePathPolicy::CanonicalRoot
    }

    fn capture(&self, _: &[String]) -> Result<CapturedFile, String> {
        self.captures.fetch_add(1, Ordering::SeqCst);
        if let Some(source) = self.invalidate_capture.lock().unwrap().as_ref() {
            let file = CapturedFile::from_source(source.clone()).unwrap();
            source.valid.store(false, Ordering::Release);
            return Ok(file);
        }
        // A provider can finish a capture before revocation, then return it late.
        let file = CapturedFile::from_source(Arc::new(Bytes {
            name: "derived.html".into(),
            releases: self.releases.clone(),
            read_gate: None,
        }))
        .unwrap();
        if self.pause.swap(false, Ordering::AcqRel) {
            self.gate.enter();
        }
        if self.fail.load(Ordering::Acquire) {
            file.release_on_worker().unwrap();
            Err("capture refused".into())
        } else {
            Ok(file)
        }
    }

    fn release_on_worker(self: Arc<Self>) -> Result<(), String> {
        self.scope_releases.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn parent_fixture() -> (
    ResourceJobs,
    Arc<RevisionSource>,
    Arc<ParentScope>,
    Resource,
) {
    let parent = Arc::new(RevisionSource {
        valid: AtomicBool::new(true),
        pause_validation: AtomicBool::new(false),
        pause_read: AtomicBool::new(false),
        fail_read: AtomicBool::new(false),
        gate: Arc::default(),
        validations: AtomicUsize::new(0),
        reads: AtomicUsize::new(0),
        releases: AtomicUsize::new(0),
    });
    let scope = Arc::new(ParentScope {
        captures: AtomicUsize::new(0),
        releases: Arc::default(),
        scope_releases: AtomicUsize::new(0),
        pause: AtomicBool::new(false),
        fail: AtomicBool::new(false),
        gate: Arc::default(),
        invalidate_capture: Mutex::new(None),
    });
    let main = CapturedFile::from_source(parent.clone()).unwrap();
    let mut jobs = ResourceJobs::new(
        SourceOwner::new(main, Some(scope.clone()), Arc::default()),
        Arc::default(),
    )
    .unwrap();
    let main = jobs.reset(1, false).unwrap().main;
    (jobs, parent, scope, main)
}

fn header() -> api::Header {
    api::Header {
        session: 1,
        document: 1,
        sequence: 37,
    }
}

fn origin(request: u64, base: u64) -> api::Event {
    api::Event {
        header: header(),
        kind: api::EventKind::ResourceResolve {
            request,
            base,
            url: "/office/v1/catalog.html".into(),
        },
    }
}

fn wait_revoked(jobs: &ResourceJobs) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !jobs.generation.resources_revoked() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(jobs.generation.resources_revoked());
    assert!(!jobs.generation.closed.load(Ordering::Acquire));
}

fn assert_denied(replies: &[ResourceReply], expected: &[u64]) {
    let mut requests: Vec<_> = replies
        .iter()
        .map(|reply| match reply {
            ResourceReply::Denied { request, .. } => *request,
            other => panic!("unexpected authority grant: {other:?}"),
        })
        .collect();
    requests.sort_unstable();
    assert_eq!(requests, expected);
}

fn quiesce(jobs: &mut ResourceJobs) -> Vec<ResourceReply> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut replies = Vec::new();
    loop {
        replies.extend(jobs.drain().unwrap());
        if jobs.shared.state.lock().unwrap().outstanding == 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "resource accounting did not settle"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    replies.extend(jobs.drain().unwrap());
    replies
}

#[test]
fn generated_base_rejection_is_sticky_without_capture_or_browser_shutdown() {
    let (mut jobs, parent, scope, main) = parent_fixture();
    let derived = resolve(&mut jobs, &main);
    let token = jobs.generation.token(1).unwrap();
    parent.valid.store(false, Ordering::Release);
    assert!(
        jobs.resolve_with_header(header(), 301, main.id, "/office/v1/catalog.html")
            .is_none()
    );
    wait_revoked(&jobs);
    assert!(token.is_cancelled());
    assert_denied(&quiesce(&mut jobs), &[301]);
    assert_eq!(scope.captures.load(Ordering::SeqCst), 1);
    assert_eq!(
        jobs.generation.take_revocation(1, 301),
        Some(origin(301, main.id))
    );
    assert!(jobs.generation.take_revocation(1, 301).is_none());

    // A rollback cannot restore any old main or derived resource capability.
    parent.valid.store(true, Ordering::Release);
    let calls = (
        parent.validations.load(Ordering::SeqCst),
        parent.reads.load(Ordering::SeqCst),
    );
    for (request, base) in [(302, main.id), (303, derived)] {
        assert_denied(
            &[jobs
                .resolve(request, base, "/office/v1/catalog.html")
                .unwrap()],
            &[request],
        );
    }
    for (request, resource) in [(304, main.id), (305, derived)] {
        assert_denied(&[jobs.read(request, resource, 0, 1).unwrap()], &[request]);
    }
    assert!(jobs.reset(1, false).is_err());
    assert!(jobs.reset(2, false).is_err());
    assert!(jobs.generation.advance(2).is_err());
    assert!(jobs.generation.ensure_at_least(1).is_err());
    assert!(jobs.generation.ensure_at_least(2).is_err());
    assert!(quiesce(&mut jobs).is_empty());
    assert_eq!(
        calls,
        (
            parent.validations.load(Ordering::SeqCst),
            parent.reads.load(Ordering::SeqCst)
        )
    );
    assert_eq!(scope.captures.load(Ordering::SeqCst), 1);
    assert!(!jobs.generation.closed.load(Ordering::Acquire));
    jobs.finish().unwrap();
    assert_eq!(parent.releases.load(Ordering::SeqCst), 1);
    assert_eq!(scope.releases.load(Ordering::SeqCst), 1);
    assert_eq!(scope.scope_releases.load(Ordering::SeqCst), 1);
}

#[test]
fn generated_capture_rechecks_parent_after_success_and_error_and_denies_queued_work() {
    for capture_error in [false, true] {
        let (mut jobs, parent, scope, main) = parent_fixture();
        let _open = OpenOnDrop(scope.gate.clone());
        scope.pause.store(true, Ordering::Release);
        scope.fail.store(capture_error, Ordering::Release);
        assert!(
            jobs.resolve(310, main.id, "/office/v1/catalog.html")
                .is_none()
        );
        assert!(scope.gate.entered());
        assert!(
            jobs.resolve(311, main.id, "/office/v1/sheet/1/first.html")
                .is_none()
        );
        assert!(jobs.read(312, main.id, 0, 1).is_none());
        parent.valid.store(false, Ordering::Release);
        scope.gate.open();
        wait_revoked(&jobs);
        assert_denied(&quiesce(&mut jobs), &[310, 311, 312]);
        assert_eq!(scope.captures.load(Ordering::SeqCst), 1);
        assert_eq!(parent.reads.load(Ordering::SeqCst), 0);
        jobs.finish().unwrap();
        assert_eq!(parent.releases.load(Ordering::SeqCst), 1);
        assert_eq!(scope.releases.load(Ordering::SeqCst), 1);
        assert_eq!(scope.scope_releases.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn generated_capture_error_with_valid_parent_does_not_poison_future_navigation() {
    let (mut jobs, parent, scope, main) = parent_fixture();
    scope.fail.store(true, Ordering::Release);
    assert!(
        jobs.resolve(320, main.id, "/office/v1/catalog.html")
            .is_none()
    );
    assert_denied(&receive(&mut jobs, 1), &[320]);
    assert!(!jobs.generation.resources_revoked());
    assert!(jobs.generation.token(1).is_ok());
    scope.fail.store(false, Ordering::Release);
    assert!(
        jobs.resolve(321, main.id, "/office/v1/catalog.html")
            .is_none()
    );
    assert!(matches!(
        receive(&mut jobs, 1).as_slice(),
        [ResourceReply::Resolved { request: 321, .. }]
    ));
    jobs.finish().unwrap();
    assert_eq!(parent.releases.load(Ordering::SeqCst), 1);
    assert_eq!(scope.releases.load(Ordering::SeqCst), 2);
}

#[test]
fn generated_capture_that_fails_validation_is_retired_without_grant() {
    let (mut jobs, parent, scope, main) = parent_fixture();
    let child = Arc::new(RevisionSource {
        valid: AtomicBool::new(true),
        pause_validation: AtomicBool::new(false),
        pause_read: AtomicBool::new(false),
        fail_read: AtomicBool::new(false),
        gate: Arc::default(),
        validations: AtomicUsize::new(0),
        reads: AtomicUsize::new(0),
        releases: AtomicUsize::new(0),
    });
    *scope.invalidate_capture.lock().unwrap() = Some(child.clone());
    assert!(
        jobs.resolve(323, main.id, "/office/v1/catalog.html")
            .is_none()
    );
    wait_revoked(&jobs);
    assert_denied(&quiesce(&mut jobs), &[323]);
    assert_eq!(child.reads.load(Ordering::SeqCst), 0);
    jobs.finish().unwrap();
    assert_eq!(parent.releases.load(Ordering::SeqCst), 1);
    assert_eq!(child.releases.load(Ordering::SeqCst), 1);
    assert_eq!(scope.scope_releases.load(Ordering::SeqCst), 1);
}

#[test]
fn generated_main_read_checks_parent_before_and_after_even_when_read_fails() {
    for before in [false, true] {
        for read_error in [false, true] {
            let (mut jobs, parent, scope, main) = parent_fixture();
            let _open = OpenOnDrop(parent.gate.clone());
            parent.pause_validation.store(before, Ordering::Release);
            parent.pause_read.store(!before, Ordering::Release);
            parent.fail_read.store(read_error, Ordering::Release);
            assert!(
                jobs.read_with_header(header(), 325, main.id, 0, 1)
                    .is_none()
            );
            assert!(parent.gate.entered());
            parent.valid.store(false, Ordering::Release);
            parent.gate.open();
            wait_revoked(&jobs);
            assert_denied(&quiesce(&mut jobs), &[325]);
            assert_eq!(
                jobs.generation.take_revocation(1, 325),
                Some(api::Event {
                    header: header(),
                    kind: api::EventKind::ResourceRead {
                        request: 325,
                        resource: main.id,
                        offset: 0,
                        length: 1
                    },
                }),
                "read refusal must preserve its actual origin, not invent a Resolve"
            );
            assert_eq!(scope.captures.load(Ordering::SeqCst), 0);
            if before {
                assert_eq!(parent.reads.load(Ordering::SeqCst), 0);
            }
            jobs.finish().unwrap();
            assert_eq!(parent.releases.load(Ordering::SeqCst), 1);
        }
    }
}

#[test]
fn revocation_denies_full_admitted_read_backlog_once_without_executing_it() {
    let (mut jobs, parent, scope, main) = parent_fixture();
    let _open = OpenOnDrop(parent.gate.clone());
    parent.pause_read.store(true, Ordering::Release);
    assert!(jobs.read(400, main.id, 0, 1).is_none());
    assert!(parent.gate.entered());
    let mut expected = vec![400];
    let mut refused = false;
    for request in 401..(401 + MAX_PENDING as u64) {
        if let Some(reply) = jobs.read(request, main.id, 0, 1) {
            assert_denied(&[reply], &[request]);
            refused = true;
        } else {
            expected.push(request);
        }
    }
    assert!(refused, "bounded admission must refuse overflow");
    assert_eq!(expected.len(), MAX_PENDING);
    parent.valid.store(false, Ordering::Release);
    parent.gate.open();
    wait_revoked(&jobs);
    assert_denied(&quiesce(&mut jobs), &expected);
    assert_eq!(jobs.shared.state.lock().unwrap().outstanding, 0);
    assert_eq!(parent.reads.load(Ordering::SeqCst), 1);
    assert_eq!(scope.captures.load(Ordering::SeqCst), 0);
    jobs.finish().unwrap();
    assert_eq!(parent.releases.load(Ordering::SeqCst), 1);
}

fn take_completion(jobs: &ResourceJobs) -> Completion {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let mut state = jobs.shared.state.lock().unwrap();
        if let Some(completion) = state.completed.pop_front() {
            if completion.counted {
                state.outstanding -= 1;
            }
            drop(state);
            jobs.shared.changed.notify_all();
            return completion;
        }
        drop(state);
        assert!(Instant::now() < deadline, "completion not produced");
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn revocation_before_protected_acceptance_denies_already_completed_resolve_and_read() {
    for read in [false, true] {
        let (mut jobs, parent, scope, main) = parent_fixture();
        if read {
            assert!(jobs.read(330, main.id, 0, 1).is_none());
        } else {
            assert!(
                jobs.resolve(330, main.id, "/office/v1/catalog.html")
                    .is_none()
            );
        }
        // Physically remove the valid result before revocation: this reproduces the
        // former gap between a drain snapshot and final reply acceptance.
        let completion = take_completion(&jobs);
        assert!(matches!(
            &completion.done,
            Done::Resolved { .. } | Done::Reply(ResourceReply::Data { .. })
        ));
        assert!(jobs.generation.revoke_resources(origin(331, main.id)));
        assert!(jobs.accept_completion(completion).is_none());
        assert_denied(&quiesce(&mut jobs), &[330]);
        assert!(quiesce(&mut jobs).is_empty(), "duplicate terminal denial");
        assert!(!jobs.generation.closed.load(Ordering::Acquire));
        jobs.finish().unwrap();
        assert_eq!(parent.releases.load(Ordering::SeqCst), 1);
        assert_eq!(scope.releases.load(Ordering::SeqCst), usize::from(!read));
    }
}

#[test]
fn revocation_after_protected_acceptance_does_not_retroactively_duplicate_reply() {
    let (mut jobs, parent, scope, main) = parent_fixture();
    assert!(
        jobs.resolve(340, main.id, "/office/v1/catalog.html")
            .is_none()
    );
    let completion = take_completion(&jobs);
    assert!(matches!(
        jobs.accept_completion(completion),
        Some(ResourceReply::Resolved { request: 340, .. })
    ));
    assert!(jobs.generation.revoke_resources(origin(341, main.id)));
    assert!(quiesce(&mut jobs).is_empty());
    assert_denied(&[jobs.read(342, main.id, 0, 1).unwrap()], &[342]);
    jobs.finish().unwrap();
    assert_eq!(parent.releases.load(Ordering::SeqCst), 1);
    assert_eq!(scope.releases.load(Ordering::SeqCst), 1);
}

#[test]
fn queued_ordinary_policy_generations_allow_intermediate_reset_but_not_stale_reads() {
    let (mut jobs, generation, scope, old_main) = setup(None, false);
    // Session admission can advance both queued Policies before the actor handles
    // the first. FIFO reset must catch up without granting intermediate authority.
    generation.advance(2).unwrap();
    generation.advance(3).unwrap();
    generation.ensure_at_least(2).unwrap();
    let intermediate = jobs.reset(2, false).unwrap();
    assert!(intermediate.denied.is_empty());
    assert!(!generation.resources_revoked());
    assert!(!generation.closed.load(Ordering::Acquire));
    assert!(generation.token(2).is_err());
    assert_denied(&[jobs.read(350, old_main.id, 0, 5).unwrap()], &[350]);
    assert_denied(
        &[jobs.read(351, intermediate.main.id, 0, 5).unwrap()],
        &[351],
    );
    assert!(scope.captured.lock().unwrap().is_empty());
    generation.ensure_at_least(3).unwrap();
    let current = jobs.reset(3, false).unwrap();
    assert!(current.denied.is_empty());
    assert!(generation.token(3).is_ok());
    assert!(jobs.read(352, current.main.id, 0, 5).is_none());
    assert_eq!(
        receive(&mut jobs, 1),
        [ResourceReply::Data {
            request: 352,
            resource: current.main.id,
            offset: 0,
            bytes: b"hello".to_vec(),
        }]
    );
    assert!(quiesce(&mut jobs).is_empty());
    jobs.finish().unwrap();
    assert!(scope.captured.lock().unwrap().is_empty());
    assert_eq!(scope.releases.load(Ordering::Acquire), 0);
    assert_eq!(scope.scope_releases.load(Ordering::Acquire), 1);
}
