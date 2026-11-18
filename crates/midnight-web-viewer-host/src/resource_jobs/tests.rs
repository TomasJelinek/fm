//! These tests exercise broker scheduling and ownership without starting Chromium.
#[path = "tests/cleanup.rs"]
mod cleanup;
#[path = "tests/revision.rs"]
mod revision;
use super::*;
use midnight_plugin_host::{ContentSource, Error};
use std::{
    sync::{Condvar, atomic::AtomicUsize, mpsc},
    time::{Duration, Instant},
};

#[derive(Debug, Default)]
struct Gate {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}

struct OpenOnDrop(Arc<Gate>);
impl Drop for OpenOnDrop {
    fn drop(&mut self) {
        self.0.open();
    }
}
impl Gate {
    fn enter(&self) {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.changed.notify_all();
        while !state.1 {
            state = self.changed.wait(state).unwrap();
        }
    }
    fn entered(&self) -> bool {
        let state = self.state.lock().unwrap();
        self.changed
            .wait_timeout_while(state, Duration::from_secs(3), |state| !state.0)
            .unwrap()
            .0
            .0
    }
    fn open(&self) {
        self.state.lock().unwrap().1 = true;
        self.changed.notify_all();
    }
}

#[derive(Debug)]
struct Bytes {
    name: String,
    releases: Arc<AtomicUsize>,
    read_gate: Option<Arc<Gate>>,
}
impl ContentSource for Bytes {
    fn length(&self) -> u64 {
        5
    }
    fn display_name(&self) -> String {
        self.name.clone()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        if let Some(gate) = &self.read_gate {
            gate.enter();
        }
        Ok(b"hello"[offset as usize..(offset + u64::from(length)).min(5) as usize].to_vec())
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        self.releases.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

#[derive(Debug)]
struct Scope {
    gate: Option<Arc<Gate>>,
    captured: Mutex<Vec<String>>,
    releases: Arc<AtomicUsize>,
    scope_releases: Arc<AtomicUsize>,
    stop_on_cancel: bool,
    path_policy: crate::ResourcePathPolicy,
}
impl Resources for Scope {
    fn path_policy(&self) -> crate::ResourcePathPolicy {
        self.path_policy
    }
    fn capture(&self, segments: &[String]) -> Result<CapturedFile, String> {
        self.capture_with_cancellation(segments, &ResourceCancellation::default())
    }
    fn capture_with_cancellation(
        &self,
        segments: &[String],
        cancellation: &ResourceCancellation,
    ) -> Result<CapturedFile, String> {
        self.captured.lock().unwrap().push(segments.join("/"));
        if let Some(gate) = &self.gate {
            if self.stop_on_cancel {
                let mut state = gate.state.lock().unwrap();
                state.0 = true;
                gate.changed.notify_all();
                let deadline = Instant::now() + Duration::from_secs(3);
                while !cancellation.is_cancelled() {
                    if Instant::now() >= deadline {
                        return Err("Cancellation was not observed".into());
                    }
                    state = gate
                        .changed
                        .wait_timeout(state, Duration::from_millis(2))
                        .unwrap()
                        .0;
                }
            } else {
                gate.enter();
            }
        }
        CapturedFile::from_source(Arc::new(Bytes {
            name: "window.html".into(),
            releases: self.releases.clone(),
            read_gate: None,
        }))
        .map_err(|error| error.to_string())
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), String> {
        self.scope_releases.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }
}

fn setup(
    gate: Option<Arc<Gate>>,
    stop_on_cancel: bool,
) -> (ResourceJobs, Arc<Generation>, Arc<Scope>, Resource) {
    setup_with_policy(
        gate,
        stop_on_cancel,
        crate::ResourcePathPolicy::RelativeAssets,
    )
}

fn setup_with_policy(
    gate: Option<Arc<Gate>>,
    stop_on_cancel: bool,
    path_policy: crate::ResourcePathPolicy,
) -> (ResourceJobs, Arc<Generation>, Arc<Scope>, Resource) {
    let scope = Arc::new(Scope {
        gate,
        captured: Mutex::new(Vec::new()),
        releases: Arc::default(),
        scope_releases: Arc::default(),
        stop_on_cancel,
        path_policy,
    });
    let main = CapturedFile::from_source(Arc::new(Bytes {
        name: "index.html".into(),
        releases: Arc::default(),
        read_gate: None,
    }))
    .unwrap();
    let generation = Arc::new(Generation::default());
    let owner = SourceOwner::new(main, Some(scope.clone()), Arc::default());
    let mut jobs = ResourceJobs::new(owner, generation.clone()).unwrap();
    let initial = jobs.reset(1, false).unwrap();
    assert!(initial.denied.is_empty());
    (jobs, generation, scope, initial.main)
}

fn receive(jobs: &mut ResourceJobs, count: usize) -> Vec<ResourceReply> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut replies = Vec::new();
    while replies.len() < count && Instant::now() < deadline {
        replies.extend(jobs.drain().unwrap());
        std::thread::sleep(Duration::from_millis(1));
    }
    replies
}

#[test]
fn blocked_generation_keeps_admission_available_and_denies_replaced_target() {
    // A synchronous capture in resolve, or silently discarded queued work, breaks this.
    let gate = Arc::new(Gate::default());
    let (ready, returned) = mpsc::channel();
    let owned_gate = gate.clone();
    let actor = std::thread::spawn(move || {
        let (mut jobs, _, scope, main) = setup_with_policy(
            Some(owned_gate),
            false,
            crate::ResourcePathPolicy::CanonicalRoot,
        );
        assert!(jobs.resolve(10, main.id, "a.html").is_none());
        ready.send(()).unwrap();
        (jobs, scope, main)
    });
    let admitted = returned.recv_timeout(Duration::from_secs(3)).is_ok();
    if !admitted {
        gate.open();
    }
    let (mut jobs, scope, main) = actor.join().unwrap();
    let _open = OpenOnDrop(gate.clone());
    assert!(admitted, "capture blocked actor-side admission");
    let entered = gate.entered();
    assert!(jobs.resolve(11, main.id, "b.html").is_none());
    assert!(matches!(
        jobs.resolve(12, main.id, "c.html"),
        Some(ResourceReply::Denied { request: 11, .. })
    ));
    gate.open();
    assert!(entered);
    let replies = receive(&mut jobs, 2);
    assert!(matches!(
        replies.as_slice(),
        [
            ResourceReply::Resolved { request: 10, .. },
            ResourceReply::Resolved { request: 12, .. },
        ]
    ));
    assert_eq!(*scope.captured.lock().unwrap(), ["a.html", "c.html"]);
    jobs.finish().unwrap();
    assert_eq!(scope.releases.load(Ordering::Acquire), 2);
    assert_eq!(scope.scope_releases.load(Ordering::Acquire), 1);
}

#[test]
fn close_cancels_running_generation_and_releases_a_late_success() {
    // Removing direct cancellation or stale-success cleanup leaks a derived parent lease.
    let gate = Arc::new(Gate::default());
    let (mut jobs, generation, scope, main) = setup(Some(gate.clone()), true);
    assert!(jobs.resolve(20, main.id, "a.html").is_none());
    let entered = gate.entered();
    generation.close();
    jobs.finish().unwrap();
    assert!(entered);
    assert_eq!(scope.releases.load(Ordering::Acquire), 1);
    assert_eq!(scope.scope_releases.load(Ordering::Acquire), 1);
}

#[test]
fn revoked_base_cannot_publish_a_completed_capture() {
    let gate = Arc::new(Gate::default());
    let (mut jobs, _, scope, main) = setup(Some(gate.clone()), false);
    let _open = OpenOnDrop(gate.clone());
    assert!(jobs.resolve(30, main.id, "a.html").is_none());
    let entered = gate.entered();
    let immediate = jobs.release(main.id);
    gate.open();
    assert!(entered);
    let mut replies = receive(&mut jobs, if immediate.is_some() { 1 } else { 2 });
    replies.extend(immediate);
    assert!(
        replies
            .iter()
            .any(|reply| matches!(reply, ResourceReply::Denied { request: 30, .. }))
    );
    assert!(
        !replies
            .iter()
            .any(|reply| matches!(reply, ResourceReply::Resolved { .. }))
    );
    jobs.finish().unwrap();
    assert_eq!(scope.releases.load(Ordering::Acquire), 1);
}

#[test]
fn new_document_denies_every_accepted_request_and_discards_late_grants() {
    let gate = Arc::new(Gate::default());
    let (mut jobs, generation, scope, main) = setup(Some(gate.clone()), false);
    let _open = OpenOnDrop(gate.clone());
    assert!(jobs.resolve(40, main.id, "a.html").is_none());
    let entered = gate.entered();
    assert!(jobs.resolve(41, main.id, "b.html").is_none());
    generation.advance(2).unwrap();
    let reset = jobs.reset(2, false).unwrap();
    let mut denied: Vec<_> = reset
        .denied
        .iter()
        .map(|reply| match reply {
            ResourceReply::Denied { request, .. } => *request,
            other => panic!("unexpected {other:?}"),
        })
        .collect();
    denied.sort_unstable();
    assert_eq!(denied, [40, 41]);
    gate.open();
    assert!(entered);
    assert!(jobs.read(42, reset.main.id, 0, 5).is_none());
    assert_eq!(
        receive(&mut jobs, 1),
        [ResourceReply::Data {
            request: 42,
            resource: reset.main.id,
            offset: 0,
            bytes: b"hello".to_vec(),
        }]
    );
    jobs.finish().unwrap();
    assert_eq!(scope.releases.load(Ordering::Acquire), 1);
    assert_eq!(*scope.captured.lock().unwrap(), ["a.html"]);
}

#[test]
fn ordinary_html_resolve_and_chunked_read_still_use_captured_authority() {
    let (mut jobs, generation, _, main) = setup(None, false);
    assert!(
        jobs.resolve(50, main.id, "styles/a.html?q=1#anchor")
            .is_none()
    );
    let replies = receive(&mut jobs, 1);
    let [ResourceReply::Resolved { resource, .. }] = replies.as_slice() else {
        panic!("expected a resolved captured asset, got {replies:?}");
    };
    let id = resource.id;
    assert!(jobs.read(51, id, 1, 3).is_none());
    assert_eq!(
        receive(&mut jobs, 1),
        [ResourceReply::Data {
            request: 51,
            resource: id,
            offset: 1,
            bytes: b"ell".to_vec(),
        }]
    );
    generation.advance(2).unwrap();
    let reset = jobs.reset(2, true).unwrap();
    assert!(matches!(
        jobs.resolve(52, reset.main.id, "a.html"),
        Some(ResourceReply::Denied { request: 52, .. })
    ));
    assert!(matches!(
        jobs.read(53, id, 0, 1),
        Some(ResourceReply::Denied { request: 53, .. })
    ));
    jobs.finish().unwrap();
}

#[test]
fn ordinary_html_asset_fanout_does_not_replace_unrelated_queued_assets() {
    // Applying Office's latest-navigation policy to HTML would deny styles/images in this batch.
    let gate = Arc::new(Gate::default());
    let (mut jobs, _, scope, main) = setup(Some(gate.clone()), false);
    let _open = OpenOnDrop(gate.clone());
    assert!(jobs.resolve(1000, main.id, "first.css").is_none());
    let entered = gate.entered();
    let mut immediate = Vec::new();
    for request in 1001..1021 {
        immediate.extend(jobs.resolve(request, main.id, &format!("asset{request}.png")));
    }
    gate.open();
    assert!(entered);
    assert!(
        immediate.is_empty(),
        "ordinary assets were replaced: {immediate:?}"
    );
    let replies = receive(&mut jobs, 21);
    assert_eq!(replies.len(), 21);
    assert!(
        replies
            .iter()
            .all(|reply| matches!(reply, ResourceReply::Resolved { .. }))
    );
    jobs.finish().unwrap();
    assert_eq!(scope.releases.load(Ordering::Acquire), 21);
}

#[test]
fn blocked_read_has_bounded_queued_work_and_no_replacement_worker() {
    // Unbounded request retention or one thread per read makes this exceed its admission limit.
    let gate = Arc::new(Gate::default());
    let releases = Arc::new(AtomicUsize::new(0));
    let main = CapturedFile::from_source(Arc::new(Bytes {
        name: "index.html".into(),
        releases: releases.clone(),
        read_gate: Some(gate.clone()),
    }))
    .unwrap();
    let generation = Arc::new(Generation::default());
    let mut jobs = ResourceJobs::new(
        SourceOwner::new(main, None, Arc::default()),
        generation.clone(),
    )
    .unwrap();
    let _open = OpenOnDrop(gate.clone());
    let initial = jobs.reset(1, false).unwrap().main;
    assert!(jobs.read(60, initial.id, 0, 1).is_none());
    let entered = gate.entered();
    let mut accepted = 1;
    let mut denied = 0;
    for request in 61..200 {
        if matches!(
            jobs.read(request, initial.id, 0, 1),
            Some(ResourceReply::Denied { .. })
        ) {
            denied += 1;
        } else {
            accepted += 1;
        }
    }
    generation.close();
    gate.open();
    jobs.finish().unwrap();
    assert!(entered);
    assert!(accepted <= 64 && denied > 0);
    assert_eq!(releases.load(Ordering::Acquire), 1);
}

#[test]
fn root_navigation_is_explicitly_scoped_and_still_rejects_authority_changes() {
    let (mut jobs, _, scope, main) =
        setup_with_policy(None, false, crate::ResourcePathPolicy::CanonicalRoot);
    assert!(
        jobs.resolve(70, main.id, "/office/v1/catalog.html")
            .is_none()
    );
    assert!(matches!(
        receive(&mut jobs, 1).as_slice(),
        [ResourceReply::Resolved { request: 70, .. }]
    ));
    assert_eq!(*scope.captured.lock().unwrap(), ["office/v1/catalog.html"]);
    for (offset, url) in [
        "//host/x",
        "/../x",
        "/%2e%2e/x",
        "file:/x",
        "/a%5cb",
        "office/v1/catalog.html?x=1",
        "office/v1/catalog.html#x",
        "%6fffice/v1/catalog.html",
    ]
    .iter()
    .enumerate()
    {
        assert!(
            matches!(
                jobs.resolve(71 + offset as u64, main.id, url),
                Some(ResourceReply::Denied { .. })
            ),
            "{url}"
        );
    }
    jobs.finish().unwrap();
}

#[test]
fn native_html_without_scope_has_confirmed_cleanup() {
    // The conservative scope default must not strand an ordinary, scope-free native document.
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("index.html");
    std::fs::write(&path, b"hello").unwrap();
    let main = CapturedFile::capture(&path).unwrap();
    let mut jobs =
        ResourceJobs::new(SourceOwner::new(main, None, Arc::default()), Arc::default()).unwrap();
    let main = jobs.reset(1, false).unwrap().main;
    assert!(jobs.read(80, main.id, 0, 5).is_none());
    assert_eq!(
        receive(&mut jobs, 1),
        [ResourceReply::Data {
            request: 80,
            resource: main.id,
            offset: 0,
            bytes: b"hello".to_vec(),
        }]
    );
    jobs.finish().unwrap();
}

#[test]
fn unconfirmed_scope_cleanup_is_reported_in_the_consuming_receipt() {
    #[derive(Debug)]
    struct Unknown;
    impl Resources for Unknown {
        fn capture(&self, _: &[String]) -> Result<CapturedFile, String> {
            Err("No assets".into())
        }
    }
    let main = CapturedFile::from_source(Arc::new(Bytes {
        name: "index.html".into(),
        releases: Arc::default(),
        read_gate: None,
    }))
    .unwrap();
    let jobs = ResourceJobs::new(
        SourceOwner::new(main, Some(Arc::new(Unknown)), Arc::default()),
        Arc::default(),
    )
    .unwrap();
    assert!(jobs.finish().is_err());
}

#[test]
fn revoked_generation_cannot_publish_an_already_completed_capture() {
    // Reset is deliberately not called before drain: Session revocation must fence publication.
    let (mut jobs, generation, scope, main) = setup(None, false);
    assert!(jobs.resolve(90, main.id, "a.html").is_none());
    let deadline = Instant::now() + Duration::from_secs(3);
    let completed = loop {
        if !jobs.shared.state.lock().unwrap().completed.is_empty() {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        std::thread::sleep(Duration::from_millis(1));
    };
    generation.advance(2).unwrap();
    let replies = jobs.drain().unwrap();
    jobs.finish().unwrap();
    assert!(completed);
    assert!(matches!(
        replies.as_slice(),
        [ResourceReply::Denied { request: 90, .. }]
    ));
    assert_eq!(scope.releases.load(Ordering::Acquire), 1);
}

#[test]
fn generated_document_grants_are_bounded_until_old_leases_are_released() {
    let (mut jobs, _, scope, main) =
        setup_with_policy(None, false, crate::ResourcePathPolicy::CanonicalRoot);
    let mut ids = Vec::new();
    for request in 100..103 {
        assert!(
            jobs.resolve(request, main.id, &format!("window{request}.html"))
                .is_none()
        );
        let replies = receive(&mut jobs, 1);
        let [ResourceReply::Resolved { resource, .. }] = replies.as_slice() else {
            panic!("Expected bounded window, got {replies:?}");
        };
        ids.push(resource.id);
    }
    assert!(matches!(
        jobs.resolve(103, main.id, "overflow.html"),
        Some(ResourceReply::Denied { request: 103, .. })
    ));
    let release = jobs.release(ids[0]);
    assert!(jobs.resolve(104, main.id, "next.html").is_none());
    let mut replies = receive(&mut jobs, if release.is_some() { 1 } else { 2 });
    replies.extend(release);
    assert!(
        replies
            .iter()
            .any(|reply| matches!(reply, ResourceReply::Resolved { request: 104, .. }))
    );
    jobs.finish().unwrap();
    assert_eq!(scope.captured.lock().unwrap().len(), 4);
    assert_eq!(scope.releases.load(Ordering::Acquire), 4);
}

#[test]
fn newest_navigation_reuses_the_replaced_reservation_at_the_grant_limit() {
    let gate = Arc::new(Gate::default());
    gate.open();
    let (mut jobs, _, _, main) = setup_with_policy(
        Some(gate.clone()),
        false,
        crate::ResourcePathPolicy::CanonicalRoot,
    );
    let _open = OpenOnDrop(gate.clone());
    assert!(jobs.resolve(110, main.id, "previous.html").is_none());
    assert!(matches!(
        receive(&mut jobs, 1).as_slice(),
        [ResourceReply::Resolved { .. }]
    ));
    *gate.state.lock().unwrap() = (false, false);
    assert!(jobs.resolve(111, main.id, "active.html").is_none());
    let entered = gate.entered();
    assert!(jobs.resolve(112, main.id, "older.html").is_none());
    let replacement = jobs.resolve(113, main.id, "newest.html");
    gate.open();
    assert!(entered);
    assert!(matches!(
        replacement,
        Some(ResourceReply::Denied { request: 112, .. })
    ));
    let replies = receive(&mut jobs, 2);
    assert!(
        replies
            .iter()
            .any(|reply| matches!(reply, ResourceReply::Resolved { request: 113, .. }))
    );
    jobs.finish().unwrap();
}

#[test]
fn oversized_generated_resource_is_released_before_any_grant() {
    #[derive(Debug)]
    struct Large {
        released: Arc<AtomicUsize>,
    }
    impl ContentSource for Large {
        fn length(&self) -> u64 {
            1024 * 1024 + 1
        }
        fn display_name(&self) -> String {
            "large.html".into()
        }
        fn version(&self) -> Vec<u8> {
            vec![1]
        }
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
        fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
            Err(Error("Oversized generated content must not be read".into()))
        }
        fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
            self.released.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }
    }
    #[derive(Debug)]
    struct LargeScope {
        released: Arc<AtomicUsize>,
    }
    impl Resources for LargeScope {
        fn path_policy(&self) -> crate::ResourcePathPolicy {
            crate::ResourcePathPolicy::CanonicalRoot
        }
        fn capture(&self, _: &[String]) -> Result<CapturedFile, String> {
            CapturedFile::from_source(Arc::new(Large {
                released: self.released.clone(),
            }))
            .map_err(|error| error.to_string())
        }
        fn release_on_worker(self: Arc<Self>) -> Result<(), String> {
            Ok(())
        }
    }
    let released = Arc::new(AtomicUsize::new(0));
    let main = CapturedFile::from_source(Arc::new(Bytes {
        name: "index.html".into(),
        releases: Arc::default(),
        read_gate: None,
    }))
    .unwrap();
    let owner = SourceOwner::new(
        main,
        Some(Arc::new(LargeScope {
            released: released.clone(),
        })),
        Arc::default(),
    );
    let mut jobs = ResourceJobs::new(owner, Arc::default()).unwrap();
    let main = jobs.reset(1, false).unwrap().main;
    assert!(jobs.resolve(120, main.id, "large.html").is_none());
    assert!(matches!(
        receive(&mut jobs, 1).as_slice(),
        [ResourceReply::Denied { request: 120, .. }]
    ));
    jobs.finish().unwrap();
    assert_eq!(released.load(Ordering::Acquire), 1);
}
