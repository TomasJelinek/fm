use super::*;
use crate::session::Channel;
use midnight_core::{ArchiveMount, Cancellation};
use midnight_filesystem_api::{Format, Message, Protocol, StagePurpose};
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{
    io::{Seek, SeekFrom},
    sync::{Barrier, mpsc},
    thread::JoinHandle,
};

#[derive(Default)]
struct Gate {
    state: Mutex<(bool, bool)>,
    changed: Condvar,
}
impl Gate {
    fn opened() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new((false, true)),
            changed: Condvar::new(),
        })
    }
    fn hold(&self) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        state.0 = true;
        self.changed.notify_all();
        while !state.1 {
            state = self
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
    fn release(&self) {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .1 = true;
        self.changed.notify_all();
    }
    fn wait_entered(&self) {
        let state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let (state, _) = self
            .changed
            .wait_timeout_while(state, Duration::from_secs(10), |state| !state.0)
            .unwrap_or_else(|error| error.into_inner());
        let entered = state.0;
        drop(state);
        assert!(entered, "fixture did not reach the held native boundary");
    }
}
struct Gates {
    wait: Arc<Gate>,
    staging: Arc<Gate>,
    process: Arc<Gate>,
    source: Arc<Gate>,
}
impl Gates {
    fn new(held: bool) -> Arc<Self> {
        let gate = || {
            if held {
                Arc::new(Gate::default())
            } else {
                Gate::opened()
            }
        };
        Arc::new(Self {
            wait: gate(),
            staging: gate(),
            process: gate(),
            source: gate(),
        })
    }
    fn release_all(&self) {
        for gate in [&self.wait, &self.staging, &self.process, &self.source] {
            gate.release();
        }
    }
}
#[derive(Default)]
struct Observed {
    waits: AtomicUsize,
    process_dropped: AtomicBool,
    source_dropped: AtomicBool,
    source_releases: AtomicUsize,
    source_reads: AtomicUsize,
    source_validations: AtomicUsize,
    status_queries: AtomicUsize,
    status_panic: AtomicBool,
    source_tainted: AtomicBool,
    taint_on_validate: AtomicBool,
    validation_error: AtomicBool,
    allow_read: AtomicBool,
    taint_on_read: AtomicBool,
    read_error: AtomicBool,
    payload_drops: Arc<AtomicUsize>,
}
struct Bomb(Arc<AtomicUsize>);
impl Drop for Bomb {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
        panic!("opaque panic payload must not be dropped");
    }
}
#[derive(Clone, Copy)]
enum Failure {
    None,
    Wait,
    WaitPanic,
    StagingPanic,
    ProcessDropPanic,
    SourceDropPanic,
    SourceRelease,
}
struct FakeProcess {
    gates: Arc<Gates>,
    observed: Arc<Observed>,
    failure: Failure,
}
impl SupervisorProcess for FakeProcess {
    fn kill(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn wait_terminated(&mut self) -> io::Result<()> {
        self.observed.waits.fetch_add(1, Ordering::SeqCst);
        self.gates.wait.hold();
        if matches!(self.failure, Failure::WaitPanic) {
            std::panic::panic_any(Bomb(self.observed.payload_drops.clone()));
        }
        if matches!(self.failure, Failure::Wait) {
            return Err(io::Error::other("native wait did not confirm termination"));
        }
        Ok(())
    }
}
impl Drop for FakeProcess {
    fn drop(&mut self) {
        self.gates.process.hold();
        if matches!(self.failure, Failure::ProcessDropPanic) {
            std::panic::panic_any(Bomb(self.observed.payload_drops.clone()));
        }
        self.observed.process_dropped.store(true, Ordering::Release);
    }
}
struct Source {
    gates: Arc<Gates>,
    observed: Arc<Observed>,
    failure: Failure,
}
impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HeldArchiveSource")
    }
}
impl ContentSource for Source {
    fn cleanup_unconfirmed(&self) -> bool {
        self.observed.status_queries.fetch_add(1, Ordering::SeqCst);
        if self.observed.status_panic.load(Ordering::Acquire) {
            std::panic::panic_any(Bomb(self.observed.payload_drops.clone()));
        }
        self.observed.source_tainted.load(Ordering::Acquire)
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        self.observed.source_releases.fetch_add(1, Ordering::SeqCst);
        if self.cleanup_unconfirmed() || matches!(self.failure, Failure::SourceRelease) {
            self.observed.source_tainted.store(true, Ordering::Release);
            std::mem::forget(self);
            return Err(Error("source release unconfirmed".into()));
        }
        if let Some(source) = Arc::into_inner(self) {
            drop(source);
        }
        Ok(())
    }
    fn length(&self) -> u64 {
        4
    }
    fn display_name(&self) -> String {
        "captured archive source".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        self.observed
            .source_validations
            .fetch_add(1, Ordering::SeqCst);
        if self
            .observed
            .taint_on_validate
            .swap(false, Ordering::AcqRel)
        {
            self.observed.source_tainted.store(true, Ordering::Release);
        }
        if self.observed.validation_error.load(Ordering::Acquire) {
            Err(Error("ordinary source validation failure".into()))
        } else {
            Ok(())
        }
    }
    fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
        assert!(
            self.observed.allow_read.load(Ordering::Acquire),
            "close must not read"
        );
        self.observed.source_reads.fetch_add(1, Ordering::SeqCst);
        if self.observed.taint_on_read.swap(false, Ordering::AcqRel) {
            self.observed.source_tainted.store(true, Ordering::Release);
        }
        if self.observed.read_error.load(Ordering::Acquire) {
            Err(Error("ordinary source read failure".into()))
        } else {
            Ok(vec![b'x'])
        }
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        self.gates.source.hold();
        if matches!(self.failure, Failure::SourceDropPanic) {
            self.observed.source_tainted.store(true, Ordering::Release);
            std::panic::panic_any(Bomb(self.observed.payload_drops.clone()));
        }
        self.observed.source_dropped.store(true, Ordering::Release);
    }
}

struct Fixture {
    mount: Option<Arc<Mount>>,
    supervision: Arc<Supervision>,
    receipt: Arc<CloseReceipt>,
    staging: Arc<Mutex<Option<crate::staging::Staging>>>,
    accounting: Arc<AtomicUsize>,
    gates: Arc<Gates>,
    observed: Arc<Observed>,
    supervisor: Option<JoinHandle<()>>,
    _temporary: tempfile::TempDir,
}
impl Fixture {
    fn new(failure: Failure, held: bool) -> Self {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let temporary = tempfile::tempdir_in(root).unwrap();
        let gates = Gates::new(held);
        let observed = Arc::new(Observed::default());
        let source = CapturedFile::from_source(Arc::new(Source {
            gates: gates.clone(),
            observed: observed.clone(),
            failure,
        }))
        .unwrap();
        let mut staging = crate::staging::Staging::open(&temporary.path().join("spool")).unwrap();
        let operation = Message::Read {
            request: 1,
            entry: 2,
            offset: 0,
            length: 4,
        };
        staging
            .nested(
                &operation,
                &Message::StageBegin {
                    request: 1,
                    purpose: StagePurpose::Member { entry: 2 },
                    expected_length: Some(4),
                    max_length: 4,
                },
            )
            .unwrap();
        staging
            .nested(
                &operation,
                &Message::StageAppend {
                    request: 1,
                    stage: 1,
                    offset: 0,
                    bytes: b"data".to_vec(),
                },
            )
            .unwrap();
        let staging = Arc::new(Mutex::new(Some(staging)));
        let supervision = Arc::new(Supervision::new(None));
        let receipt = Arc::new(CloseReceipt::new());
        let accounting = Arc::new(AtomicUsize::new(0));
        let owned = Supervisor::new(
            FakeProcess {
                gates: gates.clone(),
                observed: observed.clone(),
                failure,
            },
            staging.clone(),
            Accounting::for_test(accounting.clone()),
        );
        let worker_state = supervision.clone();
        let worker_receipt = receipt.clone();
        let stage_gate = gates.staging.clone();
        let worker_observed = observed.clone();
        let supervisor = std::thread::spawn(move || {
            supervise_with(owned, worker_state, worker_receipt, || {
                stage_gate.hold();
                if matches!(failure, Failure::StagingPanic) {
                    std::panic::panic_any(Bomb(worker_observed.payload_drops.clone()));
                }
            })
        });
        let mount = Arc::new(Mount {
            id: 21,
            provider: "close.fixture".into(),
            provider_version: "1".into(),
            stable_entry_ids: true,
            format: Format::Zip,
            container: temporary.path().join("archive.zip"),
            source: Some(source),
            channel: Mutex::new(Channel {
                input: tempfile::tempfile_in(temporary.path()).unwrap(),
                output: tempfile::tempfile_in(temporary.path()).unwrap(),
                request: 0,
            }),
            supervision: supervision.clone(),
            protocol: Protocol::V2,
            staging: staging.clone(),
            close: receipt.clone(),
        });
        Self {
            mount: Some(mount),
            supervision,
            receipt,
            staging,
            accounting,
            gates,
            observed,
            supervisor: Some(supervisor),
            _temporary: temporary,
        }
    }
    fn take_mount(&mut self) -> Arc<Mount> {
        self.mount.take().unwrap()
    }
    fn finish_supervisor(&mut self) {
        self.supervisor.take().unwrap().join().unwrap();
    }
    fn pending(&self) -> bool {
        matches!(*self.receipt.state.lock().unwrap(), CloseState::Pending)
    }
    fn closed(&self) -> bool {
        matches!(*self.receipt.state.lock().unwrap(), CloseState::Closed)
    }
    fn release(&mut self) -> (mpsc::Receiver<Result<(), FsError>>, JoinHandle<()>) {
        let mount: Arc<dyn ArchiveMount> = self.take_mount();
        let (send, receive) = mpsc::sync_channel(1);
        let worker = std::thread::Builder::new()
            .name("copy-close-without-runtime".into())
            .spawn(move || {
                let _ = send.send(mount.release_on_worker());
            })
            .unwrap();
        (receive, worker)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.gates.release_all();
        self.supervision.request_stop(Stop::Closed);
        drop(self.mount.take());
        if let Some(worker) = self.supervisor.take()
            && let Err(payload) = worker.join()
        {
            std::mem::forget(payload);
        }
    }
}
fn unfinished(receive: &mpsc::Receiver<Result<(), FsError>>) {
    assert!(matches!(receive.try_recv(), Err(mpsc::TryRecvError::Empty)));
}

#[test]
fn last_mount_release_waits_for_process_staging_accounting_and_remaining_native_fields() {
    let mut fixture = Fixture::new(Failure::None, true);
    let (done, worker) = fixture.release();
    fixture.gates.wait.wait_entered();
    assert!(fixture.pending());
    assert_eq!(fixture.accounting.load(Ordering::SeqCst), 1);
    assert!(fixture.staging.lock().unwrap().is_some());
    unfinished(&done);

    fixture.gates.wait.release();
    fixture.gates.staging.wait_entered();
    // The native staging is retained outside the mutex until actual destruction.
    assert!(fixture.staging.try_lock().unwrap().is_none());
    assert!(fixture.pending());
    assert!(!fixture.observed.process_dropped.load(Ordering::Acquire));
    assert_eq!(fixture.accounting.load(Ordering::SeqCst), 1);
    unfinished(&done);

    fixture.gates.staging.release();
    fixture.gates.process.wait_entered();
    assert!(fixture.pending());
    assert_eq!(fixture.accounting.load(Ordering::SeqCst), 1);
    unfinished(&done);

    fixture.gates.process.release();
    fixture.gates.source.wait_entered();
    assert!(fixture.closed());
    assert_eq!(fixture.accounting.load(Ordering::SeqCst), 0);
    assert!(fixture.observed.process_dropped.load(Ordering::Acquire));
    assert!(!fixture.observed.source_dropped.load(Ordering::Acquire));
    unfinished(&done);
    fixture.gates.source.release();
    done.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
    worker.join().unwrap();
    fixture.finish_supervisor();
    assert!(fixture.observed.source_dropped.load(Ordering::Acquire));
}

#[test]
fn shared_reference_release_leaves_the_other_tabs_session_running() {
    let mut fixture = Fixture::new(Failure::None, false);
    let other_tab = fixture.mount.as_ref().unwrap().clone();
    let (done, worker) = fixture.release();
    done.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
    worker.join().unwrap();
    assert!(!fixture.supervision.stop.load(Ordering::Acquire));
    assert!(fixture.pending());
    assert_eq!(fixture.observed.waits.load(Ordering::SeqCst), 0);
    assert!(!other_tab.cleanup_unconfirmed());
    assert!(!fixture.receipt.cleanup_unconfirmed());
    assert_eq!(fixture.observed.source_releases.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.accounting.load(Ordering::SeqCst), 1);
    other_tab.release_on_worker().unwrap();
    fixture.finish_supervisor();
    assert_eq!(fixture.accounting.load(Ordering::SeqCst), 0);
}

#[test]
fn concurrent_consuming_releases_assign_last_owner_cleanup_exactly_once() {
    let mut fixture = Fixture::new(Failure::None, true);
    fixture.gates.staging.release();
    fixture.gates.process.release();
    fixture.gates.source.release();
    let first = fixture.take_mount();
    let second = first.clone();
    let start = Arc::new(Barrier::new(3));
    let (send, done) = mpsc::channel();
    let workers = [first, second].map(|mount| {
        let start = start.clone();
        let send = send.clone();
        std::thread::spawn(move || {
            start.wait();
            let _ = send.send(mount.release_on_worker());
        })
    });
    drop(send);
    start.wait();
    fixture.gates.wait.wait_entered();
    done.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
    unfinished(&done);
    assert!(fixture.pending());
    fixture.gates.wait.release();
    done.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
    for worker in workers {
        worker.join().unwrap();
    }
    fixture.finish_supervisor();
    assert_eq!(fixture.observed.waits.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.accounting.load(Ordering::SeqCst), 0);
}

#[test]
fn ordinary_last_drop_stays_nonblocking_while_the_supervisor_is_waiting() {
    let mut fixture = Fixture::new(Failure::None, false);
    // Hold the real supervisor before it can take/destroy the session staging.
    let shared_staging = fixture.staging.clone();
    let staged = shared_staging.lock().unwrap();
    let mount = fixture.take_mount();
    drop(mount);
    assert!(fixture.supervision.stop.load(Ordering::Acquire));
    assert!(fixture.pending());
    assert_eq!(fixture.accounting.load(Ordering::SeqCst), 1);
    assert!(staged.is_some());
    assert!(!fixture.observed.process_dropped.load(Ordering::Acquire));
    drop(staged);
    fixture.finish_supervisor();
    assert!(fixture.closed());
}

#[test]
fn cancelled_session_does_not_shortcut_an_incomplete_supervisor_wait() {
    let mut fixture = Fixture::new(Failure::None, true);
    fixture.gates.staging.release();
    fixture.gates.process.release();
    fixture.gates.source.release();
    let cancel = Cancellation::default();
    cancel.cancel();
    fixture.supervision.begin(Instant::now(), Some(&cancel));
    fixture.gates.wait.wait_entered();
    let (done, worker) = fixture.release();
    assert!(fixture.pending());
    assert_eq!(fixture.accounting.load(Ordering::SeqCst), 1);
    unfinished(&done);
    fixture.gates.wait.release();
    done.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
    worker.join().unwrap();
    fixture.finish_supervisor();
}

#[test]
fn failed_native_wait_retains_process_staging_and_accounting_and_returns_unconfirmed() {
    let mut fixture = Fixture::new(Failure::Wait, false);
    let (done, worker) = fixture.release();
    let error = done
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap_err();
    assert!(error.message.contains("native wait"));
    worker.join().unwrap();
    fixture.finish_supervisor();
    assert!(!fixture.closed());
    assert!(fixture.receipt.cleanup_unconfirmed());
    assert!(fixture.staging.lock().unwrap().is_some());
    assert_eq!(fixture.accounting.load(Ordering::SeqCst), 1);
    assert!(!fixture.observed.process_dropped.load(Ordering::Acquire));
    assert!(!fixture.observed.source_dropped.load(Ordering::Acquire));
}

#[test]
fn supervisor_panics_publish_failed_without_dropping_opaque_payload_or_retained_owners() {
    for failure in [
        Failure::WaitPanic,
        Failure::StagingPanic,
        Failure::ProcessDropPanic,
    ] {
        let mut fixture = Fixture::new(failure, false);
        let (done, worker) = fixture.release();
        done.recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap_err();
        worker.join().unwrap();
        fixture.finish_supervisor();
        assert!(!fixture.closed());
        assert_eq!(fixture.accounting.load(Ordering::SeqCst), 1);
        assert_eq!(fixture.observed.payload_drops.load(Ordering::SeqCst), 0);
        assert!(!fixture.observed.source_dropped.load(Ordering::Acquire));
    }
}

#[test]
fn remaining_mount_source_panic_cannot_be_mistaken_for_the_supervisors_closed_receipt() {
    let mut fixture = Fixture::new(Failure::SourceDropPanic, false);
    let (done, worker) = fixture.release();
    done.recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap_err();
    worker.join().unwrap();
    fixture.finish_supervisor();
    assert!(fixture.closed());
    assert_eq!(fixture.accounting.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.observed.payload_drops.load(Ordering::SeqCst), 0);
    assert!(!fixture.observed.source_dropped.load(Ordering::Acquire));
}

#[test]
fn reported_content_release_failure_keeps_the_remaining_mount_armed_after_supervisor_close() {
    let mut fixture = Fixture::new(Failure::SourceRelease, false);
    let (done, worker) = fixture.release();
    let error = done
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap_err();
    assert!(error.message.contains("source"));
    worker.join().unwrap();
    fixture.finish_supervisor();
    assert!(fixture.closed());
    assert_eq!(fixture.accounting.load(Ordering::SeqCst), 0);
    assert!(!fixture.observed.source_dropped.load(Ordering::Acquire));
}

#[test]
fn cancelled_preparation_consumes_its_captured_source_before_returning_without_a_process() {
    let gates = Gates::new(false);
    let source_gate = Arc::new(Gate::default());
    let gates = Arc::new(Gates {
        source: source_gate.clone(),
        wait: gates.wait.clone(),
        staging: gates.staging.clone(),
        process: gates.process.clone(),
    });
    let observed = Arc::new(Observed::default());
    let captured = CapturedFile::from_source(Arc::new(Source {
        gates: gates.clone(),
        observed: observed.clone(),
        failure: Failure::None,
    }))
    .unwrap();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
    std::fs::create_dir_all(&root).unwrap();
    let temporary = tempfile::tempdir_in(root).unwrap();
    let package = crate::Package {
        id: "cancelled.fixture".into(),
        name: None,
        directory: temporary.path().to_owned(),
        version: "1".into(),
        stable_entry_ids: false,
        protocol: Protocol::V2,
        executable: temporary.path().join("must-not-start.exe"),
        formats: vec![Format::Zip],
    };
    let cancel = Cancellation::default();
    cancel.cancel();
    let path = temporary.path().join("archive.zip");
    let (send, done) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let _ = send.send(Mount::open_captured(
            &package,
            Format::Zip,
            &path,
            captured,
            &cancel,
        ));
    });
    struct Unblock(Arc<Gate>);
    impl Drop for Unblock {
        fn drop(&mut self) {
            self.0.release();
        }
    }
    let _unblock = Unblock(source_gate.clone());
    source_gate.wait_entered();
    assert!(matches!(done.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert!(!observed.source_dropped.load(Ordering::Acquire));
    source_gate.release();
    let error = done
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Interrupted);
    worker.join().unwrap();
    assert!(observed.source_dropped.load(Ordering::Acquire));
    assert_eq!(observed.waits.load(Ordering::SeqCst), 0);
}

#[test]
fn successful_validation_latches_source_taint_before_a_later_cancellation_and_shared_release() {
    let mut fixture = Fixture::new(Failure::None, false);
    let other_tab = fixture.mount.as_ref().unwrap().clone();
    fixture
        .observed
        .taint_on_validate
        .store(true, Ordering::Release);
    other_tab.validate().unwrap();
    assert!(fixture.receipt.cleanup_unconfirmed());
    let validations = fixture.observed.source_validations.load(Ordering::SeqCst);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        other_tab.read(2, 0, 1, &cancel).unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
    assert_eq!(
        fixture.observed.source_validations.load(Ordering::SeqCst),
        validations
    );
    assert!(other_tab.cleanup_unconfirmed());
    let (done, worker) = fixture.release();
    done.recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap_err();
    worker.join().unwrap();
    assert!(!fixture.supervision.stop.load(Ordering::Acquire));
    assert!(fixture.pending());
    assert_eq!(fixture.observed.source_releases.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.observed.waits.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.accounting.load(Ordering::SeqCst), 1);
    drop(other_tab);
}

#[test]
fn broker_read_error_latches_typed_source_taint_before_error_conversion() {
    let mut fixture = Fixture::new(Failure::None, false);
    let mount = fixture.mount.as_ref().unwrap().clone();
    fixture.observed.allow_read.store(true, Ordering::Release);
    fixture
        .observed
        .taint_on_read
        .store(true, Ordering::Release);
    fixture.observed.read_error.store(true, Ordering::Release);
    {
        let mut channel = mount.channel.lock().unwrap();
        midnight_filesystem_api::write_message_with_protocol(
            &mut channel.output,
            Protocol::V2,
            &Message::SourceRead {
                request: 1,
                offset: 0,
                length: 1,
            },
        )
        .unwrap();
        channel.output.seek(SeekFrom::Start(0)).unwrap();
    }
    let error = mount.read(2, 0, 1, &Cancellation::default()).unwrap_err();
    assert!(error.message.contains("ordinary source read failure"));
    assert_eq!(fixture.observed.source_reads.load(Ordering::SeqCst), 1);
    assert!(fixture.receipt.cleanup_unconfirmed());
    assert!(mount.cleanup_unconfirmed());
    let (done, worker) = fixture.release();
    done.recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap_err();
    worker.join().unwrap();
    assert_eq!(fixture.observed.source_releases.load(Ordering::SeqCst), 0);
    drop(mount);
}

#[test]
fn shared_release_observes_already_tainted_backing_without_probing_or_stopping_session() {
    let mut fixture = Fixture::new(Failure::None, false);
    let other_tab = fixture.mount.as_ref().unwrap().clone();
    fixture
        .observed
        .source_tainted
        .store(true, Ordering::Release);
    fixture.observed.status_queries.store(0, Ordering::SeqCst);
    let validations = fixture.observed.source_validations.load(Ordering::SeqCst);
    assert!(other_tab.cleanup_unconfirmed());
    assert_eq!(fixture.observed.status_queries.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture.observed.source_validations.load(Ordering::SeqCst),
        validations
    );
    assert_eq!(fixture.observed.source_reads.load(Ordering::SeqCst), 0);
    let (done, worker) = fixture.release();
    done.recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap_err();
    worker.join().unwrap();
    assert!(!fixture.supervision.stop.load(Ordering::Acquire));
    assert!(fixture.pending());
    assert_eq!(fixture.observed.waits.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.observed.source_releases.load(Ordering::SeqCst), 0);
    drop(other_tab);
}

#[test]
fn last_tainted_release_lets_the_supervisor_reclaim_process_staging_and_permit() {
    let mut fixture = Fixture::new(Failure::None, false);
    fixture
        .observed
        .source_tainted
        .store(true, Ordering::Release);
    let (done, worker) = fixture.release();
    let error = done
        .recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap_err();
    assert!(error.message.contains("unconfirmed cleanup"), "{error}");
    worker.join().unwrap();
    assert_supervisor_reclaimed_only_its_owners(&mut fixture);
}

/// The idle supervisor was woken: the worker process, staging and the global permit
/// are reclaimed, while the tainted source and Mount stay quarantined.
fn assert_supervisor_reclaimed_only_its_owners(fixture: &mut Fixture) {
    assert!(
        fixture.receipt.closed_within(Duration::from_secs(10)),
        "the last live tainted release left its supervisor running"
    );
    fixture.finish_supervisor();
    assert!(fixture.supervision.stopped());
    assert_eq!(fixture.observed.waits.load(Ordering::SeqCst), 1);
    assert!(fixture.observed.process_dropped.load(Ordering::Acquire));
    assert!(fixture.staging.lock().unwrap().is_none());
    assert_eq!(fixture.accounting.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.observed.source_releases.load(Ordering::SeqCst), 0);
    assert!(!fixture.observed.source_dropped.load(Ordering::Acquire));
}

#[test]
fn releasing_every_tainted_reference_lets_the_supervisor_reclaim_the_session() {
    let mut fixture = Fixture::new(Failure::None, false);
    let other_tab = fixture.mount.as_ref().unwrap().clone();
    fixture
        .observed
        .source_tainted
        .store(true, Ordering::Release);
    let (done, worker) = fixture.release();
    done.recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap_err();
    worker.join().unwrap();
    // Another owner still holds a live reference: the quarantined session keeps running.
    assert!(!fixture.supervision.stopped());
    assert!(fixture.pending());
    // The first reference stays retained, yet this is the last live release.
    let error = other_tab.release_on_worker().unwrap_err();
    assert!(error.message.contains("unconfirmed cleanup"), "{error}");
    assert_supervisor_reclaimed_only_its_owners(&mut fixture);
}

#[test]
fn concurrent_tainted_releases_agree_on_the_last_live_reference() {
    for _ in 0..8 {
        let mut fixture = Fixture::new(Failure::None, false);
        let first = fixture.take_mount();
        let second = first.clone();
        fixture
            .observed
            .source_tainted
            .store(true, Ordering::Release);
        let barrier = Arc::new(Barrier::new(2));
        let releases: Vec<_> = [first, second]
            .into_iter()
            .map(|mount| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    mount.release_on_worker().is_err()
                })
            })
            .collect();
        for release in releases {
            assert!(
                release.join().unwrap(),
                "a tainted release reported success"
            );
        }
        assert_supervisor_reclaimed_only_its_owners(&mut fixture);
    }
}

#[test]
fn ordinary_source_error_and_cancel_with_false_status_allow_clean_shared_release() {
    let mut fixture = Fixture::new(Failure::None, false);
    let other_tab = fixture.mount.as_ref().unwrap().clone();
    fixture
        .observed
        .validation_error
        .store(true, Ordering::Release);
    other_tab.validate().unwrap_err();
    assert!(!other_tab.cleanup_unconfirmed());
    assert!(!fixture.receipt.cleanup_unconfirmed());
    fixture
        .observed
        .validation_error
        .store(false, Ordering::Release);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert_eq!(
        other_tab.read(2, 0, 1, &cancel).unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
    assert!(!other_tab.cleanup_unconfirmed());
    let (done, worker) = fixture.release();
    done.recv_timeout(Duration::from_secs(10)).unwrap().unwrap();
    worker.join().unwrap();
    assert!(!fixture.supervision.stop.load(Ordering::Acquire));
    assert_eq!(fixture.observed.source_releases.load(Ordering::SeqCst), 0);
    other_tab.release_on_worker().unwrap();
    fixture.finish_supervisor();
}

#[test]
fn preparation_validation_taint_is_retained_before_its_error_is_propagated() {
    let gates = Gates::new(false);
    let observed = Arc::new(Observed::default());
    let captured = CapturedFile::from_source(Arc::new(Source {
        gates,
        observed: observed.clone(),
        failure: Failure::None,
    }))
    .unwrap();
    let mut input = CapturedInput::new(captured);
    observed.taint_on_validate.store(true, Ordering::Release);
    observed.validation_error.store(true, Ordering::Release);
    input.revalidate().unwrap_err();
    assert!(input.cleanup_unconfirmed());
    let error = input.finish_error(FsError::from_io(
        "open archive",
        Path::new("archive.zip"),
        io::ErrorKind::Interrupted.into(),
    ));
    assert_ne!(error.kind, io::ErrorKind::Interrupted);
    assert_eq!(observed.source_releases.load(Ordering::SeqCst), 0);
    assert!(!observed.source_dropped.load(Ordering::Acquire));
}

#[test]
fn opaque_status_getter_panic_retains_the_incoming_shared_mount_and_latches_failure() {
    let mut fixture = Fixture::new(Failure::None, false);
    let other_tab = fixture.mount.as_ref().unwrap().clone();
    fixture.observed.status_panic.store(true, Ordering::Release);
    fixture.observed.status_queries.store(0, Ordering::SeqCst);
    let (done, worker) = fixture.release();
    done.recv_timeout(Duration::from_secs(10))
        .unwrap()
        .unwrap_err();
    worker.join().unwrap();
    assert!(fixture.receipt.cleanup_unconfirmed());
    assert!(other_tab.cleanup_unconfirmed());
    assert_eq!(fixture.observed.status_queries.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.observed.payload_drops.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.observed.source_releases.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.observed.source_reads.load(Ordering::SeqCst), 0);
    assert!(!fixture.observed.source_dropped.load(Ordering::Acquire));
    assert!(!fixture.supervision.stop.load(Ordering::Acquire));
    assert_eq!(fixture.observed.waits.load(Ordering::SeqCst), 0);
    drop(other_tab);
}
