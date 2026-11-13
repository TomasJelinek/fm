//! Native acceptance tests use only owned generated pixels and an explicitly staged private package.
use midnight_image_viewer_host::{ImageSession, Source, Viewport};
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

static SERIAL: Mutex<()> = Mutex::new(());

struct Bytes {
    bytes: Vec<u8>,
    reads: AtomicUsize,
    version: AtomicU64,
}
impl Bytes {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            reads: AtomicUsize::new(0),
            version: AtomicU64::new(1),
        }
    }
}
impl Source for Bytes {
    fn length(&self) -> u64 {
        self.bytes.len() as u64
    }
    fn version(&self) -> Vec<u8> {
        self.version.load(Ordering::Acquire).to_le_bytes().to_vec()
    }
    fn validate(&self) -> io::Result<()> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(self.bytes[offset as usize..offset as usize + length as usize].to_vec())
    }
}

fn provider() -> midnight_image_viewer_host::Provider {
    let application = PathBuf::from(
        std::env::var_os("MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION")
            .expect("Explicit private test installation required"),
    );
    let discovery = midnight_image_viewer_host::discover_with_diagnostics(&application).unwrap();
    assert!(
        discovery.diagnostics.is_empty(),
        "{:?}",
        discovery.diagnostics
    );
    discovery
        .providers
        .into_iter()
        .find(|provider| provider.id == "midnight.viewer.image")
        .unwrap()
}

fn bitmap() -> Vec<u8> {
    // Authored 2x1 BMP with red and green pixels; no external image fixtures.
    let mut bytes = b"BM".to_vec();
    bytes.extend_from_slice(&62u32.to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&54u32.to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&2i32.to_le_bytes());
    bytes.extend_from_slice(&1i32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&24u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    bytes.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]);
    bytes
}

fn viewport() -> Viewport {
    Viewport {
        width: 2,
        height: 1,
        scale: 1.0,
        origin_x: 0.0,
        origin_y: 0.0,
    }
}

#[test]
#[ignore = "Requires MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION in a trusted private test installation"]
fn sandboxed_image_session_retains_original_pixels_and_rejects_source_changes() {
    let _serial = SERIAL.lock().unwrap();
    let source = Arc::new(Bytes::new(bitmap()));
    let cancel = AtomicBool::new(false);
    let mut session = ImageSession::open(
        &provider(),
        source.clone(),
        &cancel,
        Duration::from_secs(10),
    )
    .unwrap();
    assert_eq!(
        (session.metadata().width, session.metadata().height),
        (2, 1)
    );
    let reads_after_open = source.reads.load(Ordering::Relaxed);
    assert!(reads_after_open > 0);
    for _ in 0..3 {
        assert_eq!(
            session
                .render(viewport(), &cancel, Duration::from_secs(5))
                .unwrap()
                .rgba,
            [255, 0, 0, 255, 0, 255, 0, 255]
        );
    }
    assert_eq!(source.reads.load(Ordering::Relaxed), reads_after_open);
    source.version.store(2, Ordering::Release);
    assert!(
        session
            .render(viewport(), &cancel, Duration::from_secs(5))
            .is_err()
    );
    source.version.store(1, Ordering::Release);
    assert!(
        session
            .render(viewport(), &cancel, Duration::from_secs(5))
            .is_err()
    );
    session.close();
}

#[test]
#[ignore = "Requires MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION in a trusted private test installation"]
fn corrupt_content_and_cancelled_sessions_never_return_a_frame() {
    let _serial = SERIAL.lock().unwrap();
    let provider = provider();
    assert!(
        ImageSession::open(
            &provider,
            Arc::new(Bytes::new(b"invalid image".to_vec())),
            &AtomicBool::new(false),
            Duration::from_secs(5)
        )
        .is_err()
    );
    let source = Arc::new(Bytes::new(bitmap()));
    let cancel = AtomicBool::new(true);
    assert!(
        ImageSession::open(&provider, source.clone(), &cancel, Duration::from_secs(5)).is_err()
    );
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
    cancel.store(false, Ordering::Release);
    let mut session =
        ImageSession::open(&provider, source, &cancel, Duration::from_secs(5)).unwrap();
    cancel.store(true, Ordering::Release);
    let error = session
        .render(viewport(), &cancel, Duration::from_secs(5))
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    cancel.store(false, Ordering::Release);
    assert!(
        session
            .render(viewport(), &cancel, Duration::from_secs(5))
            .is_err()
    );
}

#[test]
#[ignore = "Requires MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION in a trusted private test installation"]
fn deadline_kills_a_worker_waiting_on_the_shared_read_budget() {
    let _serial = SERIAL.lock().unwrap();
    let gate = midnight_work_budget::shared_io();
    let held = gate
        .acquire(midnight_work_budget::Admission::Exclusive, || false)
        .unwrap();
    let source = Arc::new(Bytes::new(bitmap()));
    let started = Instant::now();
    let error = ImageSession::open(
        &provider(),
        source.clone(),
        &AtomicBool::new(false),
        Duration::from_secs(1),
    )
    .err()
    .expect("Deadline must fail the pending open");
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
    assert!(started.elapsed() < Duration::from_secs(5));
    drop(held);
}
