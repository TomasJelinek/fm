use super::*;

#[derive(Debug)]
struct DropProbe(Arc<AtomicUsize>);
impl Drop for DropProbe {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[derive(Debug)]
struct PanicContent(Arc<AtomicUsize>);
impl ContentSource for PanicContent {
    fn length(&self) -> u64 {
        0
    }
    fn display_name(&self) -> String {
        "panic.html".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
        Ok(Vec::new())
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        std::panic::panic_any(DropProbe(self.0.clone()));
    }
}

#[derive(Debug)]
struct PanicScope {
    payload_drops: Arc<AtomicUsize>,
    capture_panics: bool,
}
impl Resources for PanicScope {
    fn capture(&self, _: &[String]) -> Result<CapturedFile, String> {
        std::panic::panic_any(DropProbe(self.payload_drops.clone()));
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), String> {
        if !self.capture_panics {
            std::panic::panic_any(DropProbe(self.payload_drops.clone()));
        }
        Ok(())
    }
}

#[test]
fn caught_content_cleanup_panic_retains_its_arbitrary_payload() {
    let drops = Arc::new(AtomicUsize::new(0));
    let main = CapturedFile::from_source(Arc::new(PanicContent(drops.clone()))).unwrap();
    let status: CleanupStatus = Arc::default();
    let mut owner = SourceOwner::new(main, None, status.clone());
    owner.finish();
    assert!(status.lock().unwrap().is_some());
    assert_eq!(
        drops.load(Ordering::SeqCst),
        0,
        "panic payload destructor ran outside containment"
    );
}

#[test]
fn caught_scope_cleanup_panic_retains_its_arbitrary_payload() {
    let drops = Arc::new(AtomicUsize::new(0));
    let main = CapturedFile::from_source(Arc::new(Bytes {
        name: "index.html".into(),
        releases: Arc::default(),
        read_gate: None,
    }))
    .unwrap();
    let status: CleanupStatus = Arc::default();
    let scope = Arc::new(PanicScope {
        payload_drops: drops.clone(),
        capture_panics: false,
    });
    let mut owner = SourceOwner::new(main, Some(scope), status.clone());
    owner.finish();
    assert!(status.lock().unwrap().is_some());
    assert_eq!(drops.load(Ordering::SeqCst), 0);
}

#[test]
fn caught_resource_job_panic_retains_payload_and_reports_unconfirmed_cleanup() {
    let drops = Arc::new(AtomicUsize::new(0));
    let main = CapturedFile::from_source(Arc::new(Bytes {
        name: "index.html".into(),
        releases: Arc::default(),
        read_gate: None,
    }))
    .unwrap();
    let scope = Arc::new(PanicScope {
        payload_drops: drops.clone(),
        capture_panics: true,
    });
    let mut jobs = ResourceJobs::new(
        SourceOwner::new(main, Some(scope), Arc::default()),
        Arc::default(),
    )
    .unwrap();
    let main = jobs.reset(1, false).unwrap().main;
    assert!(jobs.resolve(1, main.id, "asset.html").is_none());
    let deadline = Instant::now() + Duration::from_secs(3);
    while !jobs.generation.closed.load(Ordering::Acquire) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    let revoked = jobs.generation.closed.load(Ordering::Acquire);
    assert!(jobs.finish().is_err());
    assert!(revoked);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
}

#[test]
fn joined_resource_worker_panic_retains_its_arbitrary_payload() {
    let (mut jobs, _, _, _) = setup(None, false);
    jobs.stop().unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let worker_drops = drops.clone();
    jobs.worker = Some(std::thread::spawn(move || {
        std::panic::panic_any(DropProbe(worker_drops))
    }));
    assert!(jobs.finish().is_err());
    assert_eq!(drops.load(Ordering::SeqCst), 0);
}
