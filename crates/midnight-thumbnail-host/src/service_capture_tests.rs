use super::*;
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::sync::atomic::AtomicUsize;
use std::time::Instant;

#[test]
fn stopped_service_rejects_new_work() {
    let service = Service::start(PathBuf::new(), PathBuf::new());
    service.stop();
    assert!(!service.set_view(View {
        generation: 1,
        targets: Vec::new()
    }));
}

#[derive(Debug, Default)]
struct Content(AtomicBool);
impl ContentSource for Content {
    fn length(&self) -> u64 {
        4
    }
    fn display_name(&self) -> String {
        "provider/文档".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![3, 2, 1]
    }
    fn validate(&self) -> Result<(), Error> {
        if self.0.load(Ordering::Acquire) {
            Err(Error("source changed".into()))
        } else {
            Ok(())
        }
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.validate()?;
        Ok(b"data"[offset as usize..offset as usize + length as usize].to_vec())
    }
}

#[test]
fn captured_provider_source_preserves_revision_bytes_and_storage_hint() {
    for ssd in [false, true] {
        let content = Arc::new(Content::default());
        let file = CapturedFile::from_source(content.clone()).unwrap();
        let version = file.stable_version();
        let source = captured_source(file, ssd);
        assert_eq!(source.length(), 4);
        assert_eq!(source.version(), version);
        assert_eq!(source.confirmed_local_ssd(), ssd);
        assert_eq!(source.read_at(1, 2).unwrap(), b"at");
        content.0.store(true, Ordering::Release);
        assert!(source.validate().is_err());
        assert!(source.read_at(0, 1).is_err());
    }
}

#[test]
fn cancelled_capture_is_not_pinned_and_cached_capture_honors_later_cancellation() {
    let count = Arc::new(AtomicUsize::new(0));
    let captured = count.clone();
    let capture = pin_capture(Arc::new(move |cancel| {
        if captured.fetch_add(1, Ordering::AcqRel) == 0 {
            cancel.store(true, Ordering::Release);
        }
        let file = CapturedFile::from_source(Arc::new(Content::default())).unwrap();
        Ok(captured_source(file, false))
    }));
    let cancel = AtomicBool::new(false);
    assert!(matches!(capture(&cancel), Err(e) if e.kind() == io::ErrorKind::Interrupted));
    cancel.store(false, Ordering::Release);
    assert_eq!(capture(&cancel).unwrap().read_at(0, 4).unwrap(), b"data");
    assert_eq!(count.load(Ordering::Acquire), 2);
    cancel.store(true, Ordering::Release);
    assert!(matches!(capture(&cancel), Err(e) if e.kind() == io::ErrorKind::Interrupted));
    assert_eq!(count.load(Ordering::Acquire), 2);
}

#[test]
fn admitted_capture_observes_the_live_job_cancellation() {
    let (started, entered) = mpsc::sync_channel(1);
    let capture = pin_capture(Arc::new(move |cancel| {
        started.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !cancel.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        check_cancel(cancel)?;
        Err(io::ErrorKind::TimedOut.into())
    }));
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let worker = thread::spawn(move || capture(&worker_cancel));
    entered.recv_timeout(Duration::from_secs(3)).unwrap();
    cancel.store(true, Ordering::Release);
    assert!(matches!(worker.join().unwrap(), Err(e) if e.kind() == io::ErrorKind::Interrupted));
}

struct CooperativeSource {
    cancelled: Arc<AtomicBool>,
}
impl Source for CooperativeSource {
    fn length(&self) -> u64 {
        1
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> io::Result<()> {
        Ok(())
    }
    fn read_at(&self, _: u64, _: u32) -> io::Result<Vec<u8>> {
        Ok(vec![5])
    }
    fn read_at_cooperative(
        &self,
        _: u64,
        _: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<Vec<u8>> {
        self.cancelled.store(true, Ordering::Release);
        if cancelled() {
            Err(io::ErrorKind::Interrupted.into())
        } else {
            Ok(vec![5])
        }
    }
}
#[test]
fn pinned_capture_preserves_live_cooperative_read_cancellation() {
    let flag = Arc::new(AtomicBool::new(false));
    let captured = flag.clone();
    let capture = pin_capture(Arc::new(move |_| {
        Ok(Arc::new(CooperativeSource {
            cancelled: captured.clone(),
        }))
    }));
    let source = capture(&AtomicBool::new(false)).unwrap();
    let result = source.read_at_cooperative(0, 1, &|| flag.load(Ordering::Acquire));
    assert!(
        matches!(result, Err(error) if error.kind() == io::ErrorKind::Interrupted),
        "frozen capture lost cooperative source behavior"
    );
}
