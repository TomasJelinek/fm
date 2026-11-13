//! Startup boundaries use real catalog pins; native acceptance requires an explicit installation.
use super::*;
use crate::{ImageSession, Source, Viewport};
use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, AtomicUsize},
    },
    time::{SystemTime, UNIX_EPOCH},
};

struct Fixture {
    _root: FixtureDirectory,
    provider: Provider,
}

struct FixtureDirectory(PathBuf);

impl FixtureDirectory {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let base = std::env::var_os("HOST_TEST_INSTALL_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../_build/image-startup-tests")
            });
        fs::create_dir_all(&base).unwrap();
        let base = base.canonicalize().unwrap();
        let root = base.join(format!(
            "fm-image-startup-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        Self(root)
    }

    fn cleanup(&self) -> io::Result<()> {
        // Check the exact absolute target before recursively deleting only this owned fixture.
        if self.0.canonicalize()? != self.0 {
            return Err(io::Error::other(
                "Image startup fixture cleanup target changed",
            ));
        }
        fs::remove_dir_all(&self.0)
    }
}

impl Drop for FixtureDirectory {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            if std::thread::panicking() {
                let _ = writeln!(
                    io::stderr().lock(),
                    "Image startup fixture cleanup failed at {}: {error}",
                    self.0.display(),
                );
            } else {
                panic!(
                    "Image startup fixture cleanup failed at {}: {error}",
                    self.0.display(),
                );
            }
        }
    }
}

impl Fixture {
    fn new() -> Self {
        Self::with_before_discovery(|_| {})
    }

    fn with_before_discovery(before_discovery: impl FnOnce(&Path)) -> Self {
        let root = FixtureDirectory::new();
        let application = root.0.join("me.exe");
        fs::write(&application, b"catalog fixture, never executed").unwrap();
        let package = root.0.join("viewers/image");
        fs::create_dir_all(&package).unwrap();
        fs::write(
            package.join("image-viewer.json"),
            serde_json::to_vec(&serde_json::json!({
                "schema": 1,
                "protocol": 1,
                "id": "image.startup.fixture",
                "name": "Startup fixture",
                "version": "1",
                "target": midnight_plugin_host::host_target(),
                "executable": "worker.exe",
                "extensions": ["png", "bmp"]
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(package.join("worker.exe"), b"never executed").unwrap();
        before_discovery(&root.0);
        let mut discovery = crate::discover_with_diagnostics(&application).unwrap();
        assert!(
            discovery.diagnostics.is_empty(),
            "{:?}",
            discovery.diagnostics
        );
        assert_eq!(discovery.providers.len(), 1);
        Self {
            _root: root,
            provider: discovery.providers.remove(0),
        }
    }
}

#[test]
fn failed_fixture_discovery_reclaims_its_owned_directory() {
    let mut created = None;
    let failure = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _fixture = Fixture::with_before_discovery(|root| {
            created = Some(root.to_path_buf());
            fs::write(root.join("viewers/image/image-viewer.json"), b"{}").unwrap();
        });
    }));
    assert!(failure.is_err());
    let root = created.expect("constructor must reach the malformed discovery boundary");
    assert!(!root.try_exists().unwrap(), "{}", root.display());
}

#[test]
fn catalog_matching_rejects_changed_descriptors_and_package_mapping() {
    let fixture = Fixture::new();
    let identity = ProviderIdentity::capture(&fixture.provider).unwrap();
    assert!(identity.catalog_matches(&fixture.provider));
    let mut variants = Vec::new();
    let mut changed = fixture.provider.clone();
    changed.id.push_str(".changed");
    variants.push(changed);
    let mut changed = fixture.provider.clone();
    changed.name.push_str(" changed");
    variants.push(changed);
    let mut changed = fixture.provider.clone();
    changed.version = "2".into();
    variants.push(changed);
    let mut changed = fixture.provider.clone();
    changed.extensions.reverse();
    variants.push(changed);
    let mut changed = fixture.provider.clone();
    changed.protocol = 2;
    variants.push(changed);
    let mut changed = fixture.provider.clone();
    changed.executable = changed.package_dir().join("another.exe");
    variants.push(changed);
    let another_installation = Fixture::new();
    variants.push(another_installation.provider.clone());
    for changed in variants {
        assert!(!identity.catalog_matches(&changed), "{changed:?}");
        assert!(identity.validate_for(&changed).is_err());
    }
}

#[test]
fn stale_catalog_is_rejected_before_a_worker_can_be_started() {
    let fixture = Fixture::new();
    let path = fixture.provider.package_dir().join("image-viewer.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["version"] = "2".into();
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    assert!(ProviderIdentity::capture(&fixture.provider).is_err());
}

#[test]
fn cold_open_rejects_a_stale_catalog_before_accessing_the_source() {
    let fixture = Fixture::new();
    let path = fixture.provider.package_dir().join("image-viewer.json");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    manifest["version"] = "2".into();
    fs::write(path, serde_json::to_vec(&manifest).unwrap()).unwrap();
    let source = bitmap();
    assert!(
        ImageSession::open(
            &fixture.provider,
            source.clone(),
            &AtomicBool::new(false),
            Duration::from_secs(1),
        )
        .is_err()
    );
    assert_eq!(source.validations.load(Ordering::Relaxed), 0);
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
    assert_eq!(Arc::strong_count(&source), 1);
}

#[test]
fn changed_manifest_schema_target_and_executable_are_rejected_before_startup() {
    let fixture = Fixture::new();
    let path = fixture.provider.package_dir().join("image-viewer.json");
    let original: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for (field, value) in [
        ("schema", serde_json::json!(2)),
        ("target", serde_json::json!("foreign")),
        ("executable", serde_json::json!("../worker.exe")),
        ("protocol", serde_json::json!(2)),
        ("unknown", serde_json::json!(true)),
    ] {
        let mut changed = original.clone();
        changed[field] = value;
        fs::write(&path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(
            ProviderIdentity::capture(&fixture.provider).is_err(),
            "{field}"
        );
    }
}

#[test]
fn equal_manifest_bytes_on_a_replaced_file_do_not_match_the_prepared_identity() {
    let fixture = Fixture::new();
    let identity = ProviderIdentity::capture(&fixture.provider).unwrap();
    let path = fixture.provider.package_dir().join("image-viewer.json");
    let bytes = fs::read(&path).unwrap();
    fs::rename(&path, path.with_extension("previous")).unwrap();
    fs::write(&path, bytes).unwrap();
    assert!(identity.catalog_matches(&fixture.provider));
    assert!(identity.validate_for(&fixture.provider).is_err());
}

#[test]
fn equal_executable_bytes_on_a_replaced_file_do_not_match_the_prepared_identity() {
    let fixture = Fixture::new();
    let identity = ProviderIdentity::capture(&fixture.provider).unwrap();
    let path = &fixture.provider.executable;
    let bytes = fs::read(path).unwrap();
    fs::rename(path, path.with_extension("previous")).unwrap();
    fs::write(path, bytes).unwrap();
    assert!(identity.validate_for(&fixture.provider).is_err());
}

#[test]
fn newly_ambiguous_package_invalidates_a_prepared_identity() {
    let fixture = Fixture::new();
    let identity = ProviderIdentity::capture(&fixture.provider).unwrap();
    fs::write(
        fixture.provider.package_dir().join("web-viewer.json"),
        b"{}",
    )
    .unwrap();
    assert!(identity.validate_for(&fixture.provider).is_err());
}

#[test]
fn preparation_cancellation_precedes_native_startup() {
    let fixture = Fixture::new();
    let error = PreparedImageWorker::prepare(
        &fixture.provider,
        &AtomicBool::new(true),
        Duration::from_secs(1),
    )
    .err()
    .expect("cancelled preparation cannot start the invalid fixture executable");
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
}

#[test]
fn preparation_zero_deadline_precedes_native_startup() {
    let fixture = Fixture::new();
    let error =
        PreparedImageWorker::prepare(&fixture.provider, &AtomicBool::new(false), Duration::ZERO)
            .err()
            .expect("expired preparation cannot start the invalid fixture executable");
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
}

#[test]
fn readiness_deadline_terminates_and_joins_the_blocked_exchange() {
    let blocked = Arc::new((Mutex::new(false), Condvar::new()));
    let terminated = AtomicBool::new(false);
    let joined = AtomicBool::new(false);
    let operation_blocked = blocked.clone();
    let result = supervise(
        &AtomicBool::new(false),
        Duration::from_millis(20),
        || {
            terminated.store(true, Ordering::Release);
            let (ready, wake) = blocked.as_ref();
            *ready.lock().unwrap() = true;
            wake.notify_one();
        },
        |stopped| {
            let (ready, wake) = operation_blocked.as_ref();
            let _guard = wake
                .wait_while(ready.lock().unwrap(), |ready| !*ready)
                .unwrap();
            assert!(stopped.load(Ordering::Acquire));
            joined.store(true, Ordering::Release);
            Ok(())
        },
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
    assert!(terminated.load(Ordering::Acquire));
    assert!(joined.load(Ordering::Acquire));
}

#[test]
fn readiness_cancellation_terminates_and_joins_the_blocked_exchange() {
    let cancel = AtomicBool::new(false);
    let blocked = Arc::new((Mutex::new(false), Condvar::new()));
    let operation_blocked = blocked.clone();
    let terminated = AtomicBool::new(false);
    let joined = AtomicBool::new(false);
    let result = supervise(
        &cancel,
        Duration::from_secs(1),
        || {
            terminated.store(true, Ordering::Release);
            let (ready, wake) = blocked.as_ref();
            *ready.lock().unwrap() = true;
            wake.notify_one();
        },
        |stopped| {
            cancel.store(true, Ordering::Release);
            let (ready, wake) = operation_blocked.as_ref();
            let _guard = wake
                .wait_while(ready.lock().unwrap(), |ready| !*ready)
                .unwrap();
            assert!(stopped.load(Ordering::Acquire));
            joined.store(true, Ordering::Release);
            Ok(())
        },
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
    assert!(terminated.load(Ordering::Acquire));
    assert!(joined.load(Ordering::Acquire));
}

#[test]
fn readiness_failure_is_terminal_and_terminates_the_transport() {
    let terminated = AtomicBool::new(false);
    let result = supervise(
        &AtomicBool::new(false),
        Duration::from_secs(1),
        || terminated.store(true, Ordering::Release),
        |_| {
            Err::<(), _>(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "sandbox denied",
            ))
        },
    );
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
    assert!(terminated.load(Ordering::Acquire));
}

#[test]
fn successful_exchange_preserves_the_result_without_terminating_the_transport() {
    let terminated = AtomicBool::new(false);
    let result = supervise(
        &AtomicBool::new(false),
        Duration::from_secs(1),
        || terminated.store(true, Ordering::Release),
        |_| Ok(42),
    );
    assert_eq!(result.unwrap(), 42);
    assert!(!terminated.load(Ordering::Acquire));
}

struct Bytes {
    bytes: Vec<u8>,
    reads: AtomicUsize,
    validations: AtomicUsize,
    version: AtomicU64,
}

impl Source for Bytes {
    fn length(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn version(&self) -> Vec<u8> {
        self.version.load(Ordering::Acquire).to_le_bytes().to_vec()
    }

    fn validate(&self) -> io::Result<()> {
        self.validations.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    fn read_at(&self, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        Ok(self.bytes[offset as usize..offset as usize + length as usize].to_vec())
    }
}

fn bitmap() -> Arc<Bytes> {
    // Authored 2x1 BMP with red and green pixels, matching the ordinary cold acceptance boundary.
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
    Arc::new(Bytes {
        bytes,
        reads: AtomicUsize::new(0),
        validations: AtomicUsize::new(0),
        version: AtomicU64::new(1),
    })
}

fn native_provider() -> Provider {
    let application = PathBuf::from(
        std::env::var_os("MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION")
            .expect("explicit private test installation required"),
    );
    crate::discover(&application)
        .unwrap()
        .into_iter()
        .find(|provider| provider.id == "midnight.viewer.image")
        .unwrap()
}

#[test]
#[ignore = "Requires MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION in a trusted private test installation"]
fn paired_cold_and_prepared_worker_control() {
    let provider = native_provider();
    let cancel = AtomicBool::new(false);
    let cold_source = bitmap();
    let source = bitmap();
    assert_eq!(cold_source.bytes.len(), 62);
    assert_eq!(cold_source.bytes, source.bytes);
    let viewport = Viewport {
        width: 2,
        height: 1,
        scale: 1.0,
        origin_x: 0.0,
        origin_y: 0.0,
    };
    let mut creation_attempts = 0;
    let mut creation_successes = 0;

    // These are logical host API creation counts. Existing restricted-process traces independently
    // verify the actual creations inside each phase; this test never starts the engine or refill.
    eprintln!(
        "image_startup_control phase=cold_begin logical_creation_attempts=0 logical_creation_successes=0"
    );
    let cold_start = Instant::now();
    creation_attempts += 1;
    let mut cold = ImageSession::open(
        &provider,
        cold_source.clone(),
        &cancel,
        Duration::from_secs(10),
    )
    .unwrap();
    creation_successes += 1;
    let cold_metadata = cold_start.elapsed();
    assert_eq!((cold.metadata().width, cold.metadata().height), (2, 1));
    let cold_open_reads = cold_source.reads.load(Ordering::Relaxed);
    assert!(cold_open_reads > 0);
    assert_eq!(
        cold.render(viewport, &cancel, Duration::from_secs(5))
            .unwrap()
            .rgba,
        [255, 0, 0, 255, 0, 255, 0, 255]
    );
    let cold_first_pixels = cold_start.elapsed();
    let cold_first_reads = cold_source.reads.load(Ordering::Relaxed);
    assert_eq!(cold_first_reads, cold_open_reads);
    assert_eq!(
        cold.render(viewport, &cancel, Duration::from_secs(5))
            .unwrap()
            .rgba,
        [255, 0, 0, 255, 0, 255, 0, 255]
    );
    let cold_retained_reads = cold_source.reads.load(Ordering::Relaxed);
    assert_eq!(cold_retained_reads, cold_first_reads);
    eprintln!(
        "image_startup_control phase=cold_open_end open_to_metadata_ms={:.3} open_to_exact_first_pixels_ms={:.3} source_validations={} source_reads={} rerender_added_reads={} logical_creation_attempts={} logical_creation_successes={}",
        cold_metadata.as_secs_f64() * 1000.0,
        cold_first_pixels.as_secs_f64() * 1000.0,
        cold_source.validations.load(Ordering::Relaxed),
        cold_retained_reads,
        cold_retained_reads - cold_first_reads,
        creation_attempts,
        creation_successes,
    );
    let cleanup_start = Instant::now();
    cold.close();
    let cold_cleanup = cleanup_start.elapsed();
    assert!(cold_cleanup < Duration::from_secs(5));
    assert_eq!(Arc::strong_count(&cold_source), 1);
    eprintln!(
        "image_startup_control phase=cold_cleanup_end cleanup_kind=live cleanup_ms={:.3} logical_creation_attempts={} logical_creation_successes={}",
        cold_cleanup.as_secs_f64() * 1000.0,
        creation_attempts,
        creation_successes,
    );

    assert_eq!(source.validations.load(Ordering::Relaxed), 0);
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
    eprintln!(
        "image_startup_control phase=preparation_begin logical_creation_attempts={} logical_creation_successes={}",
        creation_attempts, creation_successes,
    );
    let preparation_start = Instant::now();
    creation_attempts += 1;
    let prepared =
        PreparedImageWorker::prepare(&provider, &cancel, Duration::from_secs(15)).unwrap();
    creation_successes += 1;
    let preparation_elapsed = preparation_start.elapsed();
    assert_eq!(source.validations.load(Ordering::Relaxed), 0);
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
    assert_eq!(Arc::strong_count(&source), 1);
    eprintln!(
        "image_startup_control phase=preparation_end preparation_ms={:.3} source_validations={} source_reads={} logical_creation_attempts={} logical_creation_successes={}",
        preparation_elapsed.as_secs_f64() * 1000.0,
        source.validations.load(Ordering::Relaxed),
        source.reads.load(Ordering::Relaxed),
        creation_attempts,
        creation_successes,
    );

    eprintln!("image_startup_control phase=claim_begin");
    // Ready open_to* includes the manager claim validation and the immediate host identity guard.
    let ready_start = Instant::now();
    let claim_start = ready_start;
    prepared.validate_for(&provider).unwrap();
    let claim_elapsed = claim_start.elapsed();
    assert_eq!(source.validations.load(Ordering::Relaxed), 0);
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
    eprintln!(
        "image_startup_control phase=claim_end claim_validation_ms={:.3} source_validations={} source_reads={}",
        claim_elapsed.as_secs_f64() * 1000.0,
        source.validations.load(Ordering::Relaxed),
        source.reads.load(Ordering::Relaxed),
    );

    let ready_creation_attempts = creation_attempts;
    let ready_creation_successes = creation_successes;
    eprintln!(
        "image_startup_control phase=ready_open_begin logical_creation_attempts={} logical_creation_successes={}",
        creation_attempts, creation_successes,
    );
    let mut session = ImageSession::open_prepared(
        &provider,
        prepared,
        source.clone(),
        &cancel,
        Duration::from_secs(5),
    )
    .unwrap();
    let ready_metadata = ready_start.elapsed();
    assert_eq!(
        (session.metadata().width, session.metadata().height),
        (2, 1)
    );
    let ready_open_reads = source.reads.load(Ordering::Relaxed);
    assert!(ready_open_reads > 0);
    assert_eq!(
        session
            .render(viewport, &cancel, Duration::from_secs(5))
            .unwrap()
            .rgba,
        [255, 0, 0, 255, 0, 255, 0, 255]
    );
    let ready_first_pixels = ready_start.elapsed();
    let ready_first_reads = source.reads.load(Ordering::Relaxed);
    assert_eq!(ready_first_reads, ready_open_reads);
    assert_eq!(
        session
            .render(viewport, &cancel, Duration::from_secs(5))
            .unwrap()
            .rgba,
        [255, 0, 0, 255, 0, 255, 0, 255]
    );
    let ready_retained_reads = source.reads.load(Ordering::Relaxed);
    assert_eq!(ready_retained_reads, ready_first_reads);
    assert_eq!((creation_attempts, creation_successes), (2, 2));
    eprintln!(
        "image_startup_control phase=ready_open_end open_to_metadata_ms={:.3} open_to_exact_first_pixels_ms={:.3} source_validations={} source_reads={} rerender_added_reads={} logical_creation_attempts={} logical_creation_successes={} interval_creation_attempts={} interval_creation_successes={}",
        ready_metadata.as_secs_f64() * 1000.0,
        ready_first_pixels.as_secs_f64() * 1000.0,
        source.validations.load(Ordering::Relaxed),
        ready_retained_reads,
        ready_retained_reads - ready_first_reads,
        creation_attempts,
        creation_successes,
        creation_attempts - ready_creation_attempts,
        creation_successes - ready_creation_successes,
    );
    source.version.store(2, Ordering::Release);
    assert!(
        session
            .render(viewport, &cancel, Duration::from_secs(5))
            .is_err()
    );
    source.version.store(1, Ordering::Release);
    assert!(
        session
            .render(viewport, &cancel, Duration::from_secs(5))
            .is_err()
    );
    let cleanup_start = Instant::now();
    session.close();
    let ready_cleanup = cleanup_start.elapsed();
    assert!(ready_cleanup < Duration::from_secs(5));
    assert_eq!(Arc::strong_count(&source), 1);
    assert_eq!(source.reads.load(Ordering::Relaxed), ready_retained_reads);
    eprintln!(
        "image_startup_control phase=ready_cleanup_end cleanup_kind=after_source_change cleanup_ms={:.3} source_validations={} source_reads={} logical_creation_attempts={} logical_creation_successes={}",
        ready_cleanup.as_secs_f64() * 1000.0,
        source.validations.load(Ordering::Relaxed),
        source.reads.load(Ordering::Relaxed),
        creation_attempts,
        creation_successes,
    );
}

#[test]
#[ignore = "Requires MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION in a trusted private test installation"]
fn cancelled_prepared_adoption_never_reads_the_source() {
    let provider = native_provider();
    let cancel = AtomicBool::new(false);
    let prepared =
        PreparedImageWorker::prepare(&provider, &cancel, Duration::from_secs(15)).unwrap();
    let source = bitmap();
    cancel.store(true, Ordering::Release);
    let error = ImageSession::open_prepared(
        &provider,
        prepared,
        source.clone(),
        &cancel,
        Duration::from_secs(5),
    )
    .err()
    .expect("cancelled prepared adoption must fail");
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
    assert_eq!(Arc::strong_count(&source), 1);
}

#[test]
#[ignore = "Requires MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION in a trusted private test installation"]
fn prepared_open_rejects_corrupt_source_without_publishing_a_session() {
    let provider = native_provider();
    let cancel = AtomicBool::new(false);
    let prepared =
        PreparedImageWorker::prepare(&provider, &cancel, Duration::from_secs(15)).unwrap();
    let source = Arc::new(Bytes {
        bytes: b"invalid image".to_vec(),
        reads: AtomicUsize::new(0),
        validations: AtomicUsize::new(0),
        version: AtomicU64::new(1),
    });
    let error = ImageSession::open_prepared(
        &provider,
        prepared,
        source.clone(),
        &cancel,
        Duration::from_secs(5),
    )
    .err()
    .expect("corrupt source must end the consumed worker");
    assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    assert!(source.reads.load(Ordering::Relaxed) > 0);
    assert_eq!(Arc::strong_count(&source), 1);
}

#[test]
#[ignore = "Requires MIDNIGHT_IMAGE_VIEWER_TEST_APPLICATION in a trusted private test installation"]
fn prepared_open_deadline_kills_a_worker_waiting_on_the_read_budget() {
    let provider = native_provider();
    let cancel = AtomicBool::new(false);
    let prepared =
        PreparedImageWorker::prepare(&provider, &cancel, Duration::from_secs(15)).unwrap();
    let budget = midnight_work_budget::shared_io();
    let held = budget
        .acquire(midnight_work_budget::Admission::Exclusive, || false)
        .unwrap();
    let source = bitmap();
    let start = Instant::now();
    let error = ImageSession::open_prepared(
        &provider,
        prepared,
        source.clone(),
        &cancel,
        Duration::from_millis(100),
    )
    .err()
    .expect("prepared Open must retain its deadline while waiting for I/O admission");
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
    assert_eq!(Arc::strong_count(&source), 1);
    assert!(start.elapsed() < Duration::from_secs(5));
    drop(held);
}
