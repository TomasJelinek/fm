use super::*;
use std::{
    sync::{
        Barrier, Condvar,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug)]
struct Source {
    calls: Arc<Mutex<Vec<&'static str>>>,
    path: std::path::PathBuf,
    drop_gate: Option<Gate>,
}
#[derive(Debug)]
struct Gate {
    entered: mpsc::Sender<thread::ThreadId>,
    release: Mutex<mpsc::Receiver<()>>,
    panic: bool,
}
impl Drop for Gate {
    fn drop(&mut self) {
        let _ = self.entered.send(thread::current().id());
        let _ = self
            .release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
        assert!(!self.panic, "injected archive native Drop panic");
    }
}
impl Source {
    fn new() -> Self {
        Self {
            calls: Arc::default(),
            path: std::env::temp_dir().join("archive-owned.zip"),
            drop_gate: None,
        }
    }
    fn called(&self, name: &'static str) {
        self.calls.lock().unwrap().push(name);
    }
}
impl ArchiveMount for Source {
    fn id(&self) -> u64 {
        self.called("id");
        71
    }
    fn list(&self, directory: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        self.called("list");
        assert_eq!(directory, 13);
        Ok(Vec::new())
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        _: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        self.called("read");
        assert_eq!((entry, offset, length), (8, 3, 2));
        Ok(vec![1, 2])
    }
    fn read_cooperative(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, FsError> {
        self.called("cooperative");
        assert_eq!((entry, offset, length), (9, 4, 1));
        assert!(cancelled());
        Ok(vec![3])
    }
    fn validate(&self) -> Result<(), FsError> {
        self.called("validate");
        Ok(())
    }
    fn native_container_locator(&self) -> Option<&Path> {
        self.called("locator");
        Some(&self.path)
    }
    fn persistent_member_identity(
        &self,
        entry: u64,
        expected: &[u8],
    ) -> Result<Option<Vec<u8>>, FsError> {
        self.called("identity");
        assert_eq!((entry, expected), (11, &[4, 5][..]));
        Ok(Some(vec![7]))
    }
}
fn fixture() -> (RetirementDomain, Mounts) {
    let domain = RetirementDomain::try_start(RETIREMENT_CAPACITY, 1).unwrap();
    let mounts = Mounts::new(domain.clone());
    (domain, mounts)
}
#[test]
fn canonical_alias_reuse_preserves_identity_and_does_not_spend_another_mount_slot() {
    let (_domain, mounts) = fixture();
    let raw: Arc<dyn ArchiveMount> = Arc::new(Source::new());
    let first = mounts.adopt(raw.clone()).unwrap();
    let repeated = mounts.adopt(raw).unwrap();
    assert!(Arc::ptr_eq(&first, &repeated));
    let exposed: Arc<dyn ArchiveMount> = first.clone();
    let round_trip = mounts.adopt(exposed).unwrap();
    assert!(Arc::ptr_eq(&first, &round_trip));
    let second = mounts.adopt(Arc::new(Source::new())).unwrap();
    assert_eq!(first.id(), second.id());
    assert!(
        !Arc::ptr_eq(&first, &second),
        "numeric IDs do not establish source identity"
    );
    let retained: Vec<_> = (2..RETAINED_MOUNTS)
        .map(|_| mounts.adopt(Arc::new(Source::new())).unwrap())
        .collect();
    assert!(
        mounts.reserve().is_err(),
        "the derived aggregate retention cap is enforced"
    );
    drop(second);
    let deadline = Instant::now() + Duration::from_secs(5);
    let replacement = loop {
        if let Ok(reserved) = mounts.reserve() {
            break reserved;
        }
        assert!(
            Instant::now() < deadline,
            "native retention did not release"
        );
        thread::sleep(Duration::from_millis(1));
    };
    let third = mounts.adopt_reserved(Arc::new(Source::new()), replacement);
    assert!(!Arc::ptr_eq(&first, &third));
    assert!(
        Arc::ptr_eq(&first, &round_trip),
        "admission never evicts existing output"
    );
    drop(retained);
}
#[test]
fn guarded_archive_forwards_optional_read_and_persistent_authority_methods_exactly() {
    let (_domain, mounts) = fixture();
    let source = Source::new();
    let calls = source.calls.clone();
    let path = source.path.clone();
    let mount = mounts.adopt(Arc::new(source)).unwrap();
    assert_eq!(mount.id(), 71);
    mount.validate().unwrap();
    assert!(mount.list(13, &Cancellation::default()).unwrap().is_empty());
    assert_eq!(
        mount.read(8, 3, 2, &Cancellation::default()).unwrap(),
        [1, 2]
    );
    assert_eq!(mount.read_cooperative(9, 4, 1, &|| true).unwrap(), [3]);
    assert_eq!(mount.native_container_locator(), Some(path.as_path()));
    assert_eq!(
        mount.persistent_member_identity(11, &[4, 5]).unwrap(),
        Some(vec![7])
    );
    assert_eq!(
        *calls.lock().unwrap(),
        [
            "id",
            "validate",
            "list",
            "read",
            "cooperative",
            "locator",
            "identity"
        ]
    );
}
#[test]
fn native_cleanup_keeps_retained_mount_and_cleanup_slots_through_blocked_or_panicking_drop() {
    for panic in [false, true] {
        let (domain, mounts) = fixture();
        let (entered, observe) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let mut source = Source::new();
        source.drop_gate = Some(Gate {
            entered,
            release: Mutex::new(wait),
            panic,
        });
        let mount = mounts.adopt(Arc::new(source)).unwrap();
        let others: Vec<_> = (1..RETIREMENT_CAPACITY)
            .map(|_| domain.try_reserve().unwrap())
            .collect();
        drop(mount);
        let on = observe.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_ne!(on, thread::current().id());
        assert_eq!(mounts.retained.available_permits(), RETAINED_MOUNTS - 1);
        assert!(
            domain.try_reserve().is_err(),
            "native destructor still owns its cleanup slot"
        );
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if mounts.retained.available_permits() == RETAINED_MOUNTS
                && let Ok(lease) = domain.try_reserve()
            {
                drop(lease);
                break;
            }
            assert!(
                Instant::now() < deadline,
                "native cleanup did not release its slots"
            );
            thread::sleep(Duration::from_millis(1));
        }
        drop(others);
    }
}

/// Like the filesystem host (`session_close::release`): releasing a tainted session
/// retains the incoming reference and fails. Once every live reference is retained
/// that way, the session stops its worker and returns its process permit (the host's
/// limit of two active archive workers), while the tainted source is never destroyed.
/// An ordinary drop of the last reference stops the worker as well, like `Mount::drop`.
#[derive(Debug)]
pub(super) struct TaintedHost {
    probe: HostProbe,
    permit: Mutex<Option<OwnedSemaphorePermit>>,
}
/// Test-side view of one `TaintedHost` session.
#[derive(Clone, Debug)]
pub(crate) struct HostProbe(Arc<HostState>);
#[derive(Debug, Default)]
struct HostState {
    tainted: AtomicBool,
    retained: Mutex<usize>,
    releases: AtomicUsize,
    dropped: AtomicBool,
    stopped: Mutex<bool>,
    stop: Condvar,
}
impl TaintedHost {
    /// Holds one of `active`'s process permits until its worker stops.
    pub(super) fn start(active: &Arc<Semaphore>) -> (Arc<dyn ArchiveMount>, HostProbe) {
        let probe = HostProbe(Arc::default());
        let permit = active
            .clone()
            .try_acquire_owned()
            .expect("an earlier session leaked its archive process permit");
        let host = Arc::new(Self {
            probe: probe.clone(),
            permit: Mutex::new(Some(permit)),
        });
        (host, probe)
    }
    fn stop_worker(&self) {
        drop(self.permit.lock().unwrap().take());
        *self.probe.0.stopped.lock().unwrap() = true;
        self.probe.0.stop.notify_all();
    }
}
impl Drop for TaintedHost {
    fn drop(&mut self) {
        self.probe.0.dropped.store(true, Ordering::SeqCst);
        self.stop_worker();
    }
}
impl ArchiveMount for TaintedHost {
    fn cleanup_unconfirmed(&self) -> bool {
        self.probe.0.tainted.load(Ordering::SeqCst)
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        self.probe.0.releases.fetch_add(1, Ordering::SeqCst);
        if !self.cleanup_unconfirmed() {
            drop(Arc::into_inner(self));
            return Ok(());
        }
        let mut retained = self.probe.0.retained.lock().unwrap();
        *retained += 1;
        if Arc::strong_count(&self) == *retained {
            self.stop_worker();
        }
        drop(retained);
        std::mem::forget(self);
        Err(nested::unconfirmed())
    }
    fn id(&self) -> u64 {
        72
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        panic!("release must not list")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("release must not read")
    }
    fn validate(&self) -> Result<(), FsError> {
        Ok(())
    }
}
impl HostProbe {
    pub(crate) fn taint(&self) {
        self.0.tainted.store(true, Ordering::SeqCst);
    }
    /// Host releases reached, whether or not they stopped the worker.
    pub(crate) fn releases(&self) -> usize {
        self.0.releases.load(Ordering::SeqCst)
    }
    pub(crate) fn stopped(&self) -> bool {
        *self.0.stopped.lock().unwrap()
    }
    /// Bounded wait for a stop that happens on another thread; true once stopped.
    pub(crate) fn wait_stopped(&self) -> bool {
        let stopped = self.0.stopped.lock().unwrap();
        let (stopped, _) = self
            .0
            .stop
            .wait_timeout_while(stopped, Duration::from_secs(5), |stopped| !*stopped)
            .unwrap();
        *stopped
    }
    /// Whether the session itself was destroyed rather than kept quarantined.
    pub(crate) fn dropped(&self) -> bool {
        self.0.dropped.load(Ordering::SeqCst)
    }
}

/// How an owner gives up its guard reference. Tree captures, targets, navigations and
/// member content that hold the guard are covered in `providers::archive_taint_tests`.
#[derive(Clone, Copy, Debug)]
enum Release {
    /// The guard's own `ArchiveMount::release_on_worker`, as a sharing owner outside
    /// Copy calls it.
    Worker,
    /// `release_archive` inside an already quarantined Copy context.
    Quarantined,
}
impl Release {
    fn run(self, mount: Arc<dyn ArchiveMount>) -> Result<(), FsError> {
        match self {
            Self::Worker => mount.release_on_worker(),
            Self::Quarantined => {
                let context = nested::Context::default();
                context.quarantine();
                context.with(|| nested::release_archive(mount))
            }
        }
    }
}

#[test]
fn shared_tainted_release_leaves_the_last_owner_the_host_release_and_keeps_engine_slots() {
    use Release::{Quarantined, Worker};
    let (domain, mounts) = fixture();
    let active = Arc::new(Semaphore::new(2));
    let sessions = [
        (Worker, Worker),
        (Worker, Quarantined),
        (Quarantined, Worker),
        (Quarantined, Quarantined),
    ];
    for (first, last) in sessions {
        let (host, probe) = TaintedHost::start(&active);
        let pane: Arc<dyn ArchiveMount> = mounts.adopt(host).unwrap();
        let tree = pane.clone();
        probe.taint();
        assert!(first.run(tree).is_err(), "{first:?}");
        assert_eq!(
            probe.releases(),
            0,
            "{first:?}: a shared release must not end the session the pane still uses"
        );
        assert!(!probe.stopped());
        assert_eq!(active.available_permits(), 1);
        assert!(pane.cleanup_unconfirmed());
        assert!(last.run(pane).is_err(), "{first:?} then {last:?}");
        assert_eq!(
            probe.releases(),
            1,
            "{first:?} then {last:?}: the last owner hands its host reference over"
        );
        assert!(probe.stopped(), "{first:?} then {last:?}: worker kept");
        assert_eq!(active.available_permits(), 2, "process permit kept");
        assert!(!probe.dropped(), "the tainted source stays quarantined");
    }
    // The engine's retained-mount slots and cleanup leases stay quarantined.
    assert_eq!(
        mounts.retained.available_permits(),
        RETAINED_MOUNTS - sessions.len()
    );
    let others: Vec<_> = (sessions.len()..RETIREMENT_CAPACITY)
        .map(|_| domain.try_reserve().unwrap())
        .collect();
    assert!(domain.try_reserve().is_err());
    drop(others);
}

#[test]
fn shared_tainted_release_does_not_pin_the_session_past_the_last_ordinary_drop() {
    let (_domain, mounts) = fixture();
    let active = Arc::new(Semaphore::new(2));
    for first in [Release::Worker, Release::Quarantined] {
        let (host, probe) = TaintedHost::start(&active);
        let pane: Arc<dyn ArchiveMount> = mounts.adopt(host).unwrap();
        probe.taint();
        assert!(first.run(pane.clone()).is_err());
        // Closing the tab drops the last reference outside any Copy context.
        drop(pane);
        assert!(
            probe.wait_stopped(),
            "{first:?}: the closed tab kept its worker"
        );
        assert_eq!(active.available_permits(), 2);
    }
}

#[test]
fn tainted_last_reference_dropped_inside_a_copy_still_reaches_the_host_release() {
    let (domain, mounts) = fixture();
    let active = Arc::new(Semaphore::new(2));
    for quarantined in [false, true] {
        let (host, probe) = TaintedHost::start(&active);
        let pane: Arc<dyn ArchiveMount> = mounts.adopt(host).unwrap();
        probe.taint();
        let context = nested::Context::default();
        if quarantined {
            context.quarantine();
        }
        context.with(|| drop(pane));
        assert!(context.is_quarantined());
        assert_eq!(probe.releases(), 1, "quarantined Copy: {quarantined}");
        assert!(probe.stopped(), "quarantined Copy: {quarantined}");
        assert_eq!(active.available_permits(), 2);
        assert!(!probe.dropped(), "the tainted source stays quarantined");
    }
    assert_eq!(mounts.retained.available_permits(), RETAINED_MOUNTS - 2);
    let others: Vec<_> = (2..RETIREMENT_CAPACITY)
        .map(|_| domain.try_reserve().unwrap())
        .collect();
    assert!(
        domain.try_reserve().is_err(),
        "cleanup leases stay quarantined"
    );
    drop(others);
}

#[test]
fn quarantined_release_retains_an_untainted_last_owner_and_decrements_a_shared_one() {
    let (domain, mounts) = fixture();
    let active = Arc::new(Semaphore::new(2));
    // A shared reference given up in a quarantined Copy only loses its count, so the
    // pane, now the last owner, performs the actual release and nothing stays pinned.
    let (host, probe) = TaintedHost::start(&active);
    let pane: Arc<dyn ArchiveMount> = mounts.adopt(host).unwrap();
    assert!(Release::Quarantined.run(pane.clone()).is_err());
    assert_eq!(probe.releases(), 0);
    assert!(!probe.dropped());
    pane.release_on_worker().unwrap();
    assert_eq!(probe.releases(), 1);
    assert!(probe.dropped() && probe.stopped());
    assert_eq!(active.available_permits(), 2);
    assert_eq!(mounts.retained.available_permits(), RETAINED_MOUNTS);
    // A quarantined Copy cannot certify an untainted last owner's native teardown: the
    // session, its worker, retained-mount slot and cleanup lease all stay.
    let (host, probe) = TaintedHost::start(&active);
    let pane: Arc<dyn ArchiveMount> = mounts.adopt(host).unwrap();
    assert!(Release::Quarantined.run(pane).is_err());
    assert_eq!(probe.releases(), 0);
    assert!(!probe.dropped() && !probe.stopped());
    assert_eq!(active.available_permits(), 1);
    assert_eq!(mounts.retained.available_permits(), RETAINED_MOUNTS - 1);
    let others: Vec<_> = (1..RETIREMENT_CAPACITY)
        .map(|_| domain.try_reserve().unwrap())
        .collect();
    assert!(
        domain.try_reserve().is_err(),
        "the cleanup lease stays retained"
    );
    drop(others);
}

#[test]
fn concurrent_tainted_releases_reach_the_host_release_exactly_once() {
    use Release::{Quarantined, Worker};
    let (_domain, mounts) = fixture();
    let active = Arc::new(Semaphore::new(2));
    for _ in 0..32 {
        let (host, probe) = TaintedHost::start(&active);
        let pane: Arc<dyn ArchiveMount> = mounts.adopt(host).unwrap();
        probe.taint();
        let owners = [pane.clone(), pane.clone(), pane.clone(), pane];
        let start = Arc::new(Barrier::new(owners.len()));
        let releasers: Vec<_> = owners
            .into_iter()
            .zip([Worker, Quarantined, Worker, Quarantined])
            .map(|(owner, release)| {
                let start = start.clone();
                thread::spawn(move || {
                    start.wait();
                    release.run(owner).is_err()
                })
            })
            .collect();
        for releaser in releasers {
            assert!(
                releaser.join().unwrap(),
                "a tainted release reported success"
            );
        }
        assert_eq!(probe.releases(), 1);
        assert!(probe.stopped());
        assert_eq!(active.available_permits(), 2);
        assert!(!probe.dropped());
    }
}
