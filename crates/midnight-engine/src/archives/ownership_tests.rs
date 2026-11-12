use super::*;
use std::{
    sync::mpsc,
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
fn fixture() -> (RetirementDomain, Mounts, TaskLifetime) {
    let domain = RetirementDomain::try_start(RETIREMENT_CAPACITY, 1).unwrap();
    let mounts = Mounts::new(domain.clone());
    (domain, mounts, TaskLifetime::testing(()))
}
#[test]
fn canonical_alias_reuse_preserves_identity_and_does_not_spend_another_mount_slot() {
    let (_domain, mounts, scope) = fixture();
    let raw: Arc<dyn ArchiveMount> = Arc::new(Source::new());
    let first = mounts.adopt(raw.clone(), scope.clone()).unwrap();
    let repeated = mounts.adopt(raw, scope.clone()).unwrap();
    assert!(Arc::ptr_eq(&first, &repeated));
    let exposed: Arc<dyn ArchiveMount> = first.clone();
    let round_trip = mounts.adopt(exposed, scope.clone()).unwrap();
    assert!(Arc::ptr_eq(&first, &round_trip));
    let second = mounts
        .adopt(Arc::new(Source::new()), scope.clone())
        .unwrap();
    assert_eq!(first.id(), second.id());
    assert!(
        !Arc::ptr_eq(&first, &second),
        "numeric IDs do not establish source identity"
    );
    let retained: Vec<_> = (2..RETAINED_MOUNTS)
        .map(|_| {
            mounts
                .adopt(Arc::new(Source::new()), scope.clone())
                .unwrap()
        })
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
    let third = mounts.adopt_reserved(Arc::new(Source::new()), replacement, scope);
    assert!(!Arc::ptr_eq(&first, &third));
    assert!(
        Arc::ptr_eq(&first, &round_trip),
        "admission never evicts existing output"
    );
    drop(retained);
}
#[test]
fn guarded_archive_forwards_optional_read_and_persistent_authority_methods_exactly() {
    let (_domain, mounts, scope) = fixture();
    let source = Source::new();
    let calls = source.calls.clone();
    let path = source.path.clone();
    let mount = mounts.adopt(Arc::new(source), scope).unwrap();
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
        let (domain, mounts, scope) = fixture();
        let (entered, observe) = mpsc::channel();
        let (release, wait) = mpsc::channel();
        let mut source = Source::new();
        source.drop_gate = Some(Gate {
            entered,
            release: Mutex::new(wait),
            panic,
        });
        let mount = mounts.adopt(Arc::new(source), scope).unwrap();
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
