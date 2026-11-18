//! Adapter boundaries use real Engine admission and cleanup, without native I/O.
use super::*;
use midnight_core::{
    CompiledFileComparatorPolicy, DirectoryService, Entry, EntryMetadata, Event,
    FileComparatorProgram, FileComparatorRule, FileComparatorTarget, FilterMode,
    PreparedFileComparatorPolicy, SpaceInfo, StorageService, Volume, VolumeService,
};
use midnight_engine::CommandReceiver;
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

struct NeverFilesystem;
impl DirectoryService for NeverFilesystem {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        panic!("comparison adapter must not list a directory")
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        panic!("comparison adapter must use the supplied capture authority")
    }
}
impl VolumeService for NeverFilesystem {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        panic!("comparison adapter must not discover volumes")
    }
}
impl StorageService for NeverFilesystem {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        panic!("comparison adapter must not query storage")
    }
}

fn engine() -> (Engine, tokio::sync::mpsc::Receiver<Event>) {
    let (events, observer) = tokio::sync::mpsc::channel(4);
    (
        Engine::new(
            Engine::lifetime().unwrap(),
            Arc::new(NeverFilesystem),
            events,
        ),
        observer,
    )
}

struct Credit;
impl ExternalCompareSessionCredit for Credit {}

#[derive(Debug)]
struct Receipt {
    thread: thread::ThreadId,
    name: Option<String>,
    explicitly_released: bool,
}
#[derive(Debug)]
struct Source {
    reads: Arc<AtomicUsize>,
    released: AtomicBool,
    receipts: mpsc::Sender<Receipt>,
}
impl ContentSource for Source {
    fn length(&self) -> u64 {
        0
    }
    fn display_name(&self) -> String {
        "comparison fixture".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
        self.reads.fetch_add(1, Ordering::AcqRel);
        panic!("external admission must never read content")
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        self.released.store(true, Ordering::Release);
        drop(self);
        Ok(())
    }
}
impl Drop for Source {
    fn drop(&mut self) {
        let current = thread::current();
        let _ = self.receipts.send(Receipt {
            thread: current.id(),
            name: current.name().map(str::to_owned),
            explicitly_released: self.released.load(Ordering::Acquire),
        });
    }
}

struct CaptureProbe {
    paths: Arc<Mutex<Vec<PathBuf>>>,
    reads: Arc<AtomicUsize>,
    receipts: Receiver<Receipt>,
}
fn launcher(engine: &mut Engine) -> (Launcher, CommandReceiver, CaptureProbe) {
    let (commands, receiver) = engine.command_channel(1).unwrap();
    let (receipts, observed) = mpsc::channel();
    let paths = Arc::new(Mutex::new(Vec::new()));
    let reads = Arc::new(AtomicUsize::new(0));
    let captures = paths.clone();
    let content_reads = reads.clone();
    let capture: ExternalCompareNativeCapture = Arc::new(move |path, _| {
        captures.lock().unwrap().push(path.to_owned());
        Ok(CapturedFile::from_source(Arc::new(Source {
            reads: content_reads.clone(),
            released: AtomicBool::new(false),
            receipts: receipts.clone(),
        }))
        .unwrap())
    });
    (
        Launcher {
            front: engine.external_comparisons(),
            commands,
            capture,
        },
        receiver,
        CaptureProbe {
            paths,
            reads,
            receipts: observed,
        },
    )
}
fn paths() -> [PathBuf; 2] {
    let root = std::env::current_dir().unwrap();
    [
        root.join("left original Ž.txt"),
        root.join("right original 文.txt"),
    ]
}
fn submit(launcher: &Launcher) -> (Result<(), FsError>, Receiver<ExternalCompareOutcome>) {
    let paths = paths();
    let process = FileComparatorProgram {
        executable: std::env::current_dir().unwrap().join("never-launched.exe"),
        arguments: vec!["{left}".into(), "{right}".into()],
        working_directory: None,
    }
    .process_for(&paths)
    .unwrap();
    let (reply, observer) = mpsc::sync_channel(1);
    let result = launcher.submit(
        process,
        paths,
        Arc::new(Credit),
        Cancellation::default(),
        reply,
    );
    (result, observer)
}
fn assert_released(probe: &CaptureProbe) {
    for _ in 0..2 {
        let receipt = probe.receipts.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(receipt.explicitly_released);
        assert_ne!(receipt.thread, thread::current().id());
        assert_eq!(receipt.name.as_deref(), Some("fm-retirement"));
    }
    assert!(matches!(
        probe.receipts.try_recv(),
        Err(TryRecvError::Empty)
    ));
    assert_eq!(*probe.paths.lock().unwrap(), paths());
    assert_eq!(probe.reads.load(Ordering::Acquire), 0);
}
fn assert_disconnected(observer: Receiver<ExternalCompareOutcome>) {
    assert!(matches!(
        observer.recv_timeout(Duration::from_secs(5)),
        Err(RecvTimeoutError::Disconnected)
    ));
}

#[test]
fn full_command_queue_returns_would_block_and_retires_both_captures() {
    let (mut engine, _events) = engine();
    let (launcher, _receiver, probe) = launcher(&mut engine);
    assert!(
        launcher
            .commands
            .try_send(Command::ListViewerPlugins)
            .is_ok()
    );
    let (result, observer) = submit(&launcher);
    let error = result.unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::WouldBlock);
    assert!(error.path.as_os_str().is_empty());
    assert_disconnected(observer);
    assert_released(&probe);
}

#[test]
fn closed_command_queue_returns_broken_pipe_and_retires_both_captures() {
    let (mut engine, _events) = engine();
    let (launcher, receiver, probe) = launcher(&mut engine);
    drop(receiver);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !launcher.commands.is_closed() {
        assert!(
            Instant::now() < deadline,
            "receiver cleanup did not close the channel"
        );
        thread::sleep(Duration::from_millis(1));
    }
    let (result, observer) = submit(&launcher);
    let error = result.unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::BrokenPipe);
    assert!(error.path.as_os_str().is_empty());
    assert_disconnected(observer);
    assert_released(&probe);
}

#[test]
fn successful_admission_is_not_started_and_unpolled_receiver_owns_cleanup() {
    let (mut engine, _events) = engine();
    let (launcher, receiver, probe) = launcher(&mut engine);
    let (result, observer) = submit(&launcher);
    assert!(result.is_ok());
    assert!(matches!(observer.try_recv(), Err(TryRecvError::Empty)));
    assert!(matches!(
        probe.receipts.try_recv(),
        Err(TryRecvError::Empty)
    ));
    assert_eq!(*probe.paths.lock().unwrap(), paths());
    assert_eq!(probe.reads.load(Ordering::Acquire), 0);
    drop(engine);
    assert!(matches!(observer.try_recv(), Err(TryRecvError::Empty)));
    drop(receiver);
    assert_disconnected(observer);
    assert_released(&probe);
}

struct Compiled {
    raw: Arc<FileComparatorPreferences>,
    selected: Arc<Mutex<Vec<[PathBuf; 2]>>>,
}
impl CompiledFileComparatorPolicy for Compiled {
    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        &self.raw
    }
    fn select(&self, paths: &[PathBuf; 2]) -> Option<usize> {
        self.selected.lock().unwrap().push(paths.clone());
        Some(0)
    }
    fn matcher_budget_bytes(&self) -> usize {
        1
    }
}
fn publish(engine: &Engine, selected: Arc<Mutex<Vec<[PathBuf; 2]>>>) -> FileComparatorSnapshot {
    let store = engine.comparator_policies();
    store.publish(
        PreparedFileComparatorPolicy::new(Arc::new(Compiled {
            raw: Arc::new(FileComparatorPreferences {
                rules: vec![FileComparatorRule {
                    enabled: true,
                    mode: FilterMode::Masks,
                    expression: "*".into(),
                    case_sensitive: false,
                    target: FileComparatorTarget::Automatic,
                }],
            }),
            selected,
        }))
        .with_id(store.prepare_id().unwrap()),
    )
}

#[test]
fn policy_adapter_preserves_exact_snapshot_and_delegates_matching_to_same_pin() {
    let (engine, _events) = engine();
    let selected = Arc::new(Mutex::new(Vec::new()));
    let snapshot = publish(&engine, selected.clone());
    let source = PolicySource(engine.comparator_policy_source());
    assert!(Arc::ptr_eq(
        &source.snapshot().preferences,
        &snapshot.preferences
    ));
    let policy = match source.pin(&snapshot, Arc::new(Credit)) {
        Ok(policy) => policy,
        Err(_) => panic!("current snapshot must pin"),
    };
    assert!(Arc::ptr_eq(policy.preferences(), &snapshot.preferences));
    assert_eq!(policy.snapshot().id, snapshot.id);
    assert!(Arc::ptr_eq(
        &policy.snapshot().preferences,
        &snapshot.preferences
    ));
    let replacement = publish(&engine, Arc::default());
    assert_ne!(replacement.id, snapshot.id);
    assert_eq!(policy.select(&paths()), Some(0));
    assert_eq!(*selected.lock().unwrap(), vec![paths()]);
    assert!(Arc::ptr_eq(policy.preferences(), &snapshot.preferences));
}

#[test]
fn policy_adapter_maps_stale_id_and_wrong_raw_arc_without_matching() {
    let (engine, _events) = engine();
    let selected = Arc::new(Mutex::new(Vec::new()));
    let snapshot = publish(&engine, selected.clone());
    let source = PolicySource(engine.comparator_policy_source());
    let mut wrong = snapshot.clone();
    wrong.preferences = Arc::new((*wrong.preferences).clone());
    assert!(matches!(
        source.pin(&wrong, Arc::new(Credit)),
        Err(FileComparatorPinError::PolicyChanged)
    ));
    publish(&engine, Arc::default());
    assert!(matches!(
        source.pin(&snapshot, Arc::new(Credit)),
        Err(FileComparatorPinError::PolicyChanged)
    ));
    assert!(selected.lock().unwrap().is_empty());
}
