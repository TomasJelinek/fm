use super::*;
use fm_core::{SpaceInfo, StorageService, Volume, VolumeService};
use std::{
    collections::{HashMap, HashSet},
    sync::{Condvar, Mutex},
};

#[derive(Default)]
struct Counter {
    active: AtomicUsize,
    maximum: AtomicUsize,
}
struct Active<'a>(&'a Counter);
impl Counter {
    fn enter(&self) -> Active<'_> {
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.maximum.fetch_max(active, Ordering::SeqCst);
        Active(self)
    }
}
impl Drop for Active<'_> {
    fn drop(&mut self) {
        self.0.active.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct Blocker {
    state: Mutex<(usize, bool)>,
    changed: Condvar,
}
impl Blocker {
    fn block(&self, cancel: &Cancellation) {
        let mut state = self.state.lock().unwrap();
        state.0 += 1;
        self.changed.notify_all();
        while !state.1 && !cancel.is_cancelled() {
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(5))
                .unwrap()
                .0;
        }
    }
    fn wait(&self, count: usize) {
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut state = self.state.lock().unwrap();
        while state.0 < count {
            assert!(
                Instant::now() < deadline,
                "Expected concurrent reads did not enter"
            );
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(5))
                .unwrap()
                .0;
        }
    }
    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.changed.notify_all();
    }
}
struct ReleaseOnDrop(Arc<Blocker>);
impl Drop for ReleaseOnDrop {
    fn drop(&mut self) {
        self.0.release();
    }
}

struct FairnessProbe {
    producer: PathBuf,
    consumer: PathBuf,
    total_files: usize,
    completed_metadata: AtomicUsize,
}

#[derive(Default)]
struct Files {
    directories: HashMap<PathBuf, Vec<Entry>>,
    lengths: HashMap<PathBuf, u64>,
    kind: StorageKind,
    overrides: HashMap<PathBuf, Option<StorageKind>>,
    queries: Mutex<Vec<PathBuf>>,
    opens: HashMap<PathBuf, Arc<Blocker>>,
    missing: HashSet<PathBuf>,
    io: Counter,
    classifications: Counter,
    conservative: Counter,
    metadata_delay: Duration,
    fairness_probe: Option<Arc<FairnessProbe>>,
}
impl Files {
    fn files(&mut self, path: &str, count: usize) {
        let path = Path::new(path);
        let entries = (0..count)
            .map(|index| {
                let file = path.join(format!("žluťoučký-{index}.txt"));
                self.lengths.insert(file.clone(), 1);
                Entry::new(EntryId(index as u64 + 100), file, EntryKind::File)
            })
            .collect();
        self.directories.insert(path.into(), entries);
    }
    fn kind_for(&self, path: &Path) -> Option<StorageKind> {
        self.overrides.get(path).copied().unwrap_or(Some(self.kind))
    }
}
impl DirectoryService for Files {
    fn read_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        if let Some(probe) = &self.fairness_probe
            && path == probe.consumer
        {
            assert!(
                probe.completed_metadata.load(Ordering::SeqCst) < probe.total_files,
                "small root must enter before all large-root work has finished"
            );
        }
        {
            let _io = self.io.enter();
            let _conservative = (self.kind_for(path) != Some(StorageKind::LocalSsd))
                .then(|| self.conservative.enter());
            if let Some(block) = self.opens.get(path) {
                block.block(cancel);
            }
            thread::sleep(Duration::from_micros(100));
        }
        let entries = self.directories.get(path).ok_or_else(|| {
            FsError::from_io("test directory", path, io::ErrorKind::NotFound.into())
        })?;
        for entry in entries {
            if cancel.is_cancelled() {
                break;
            }
            {
                let _io = self.io.enter();
                let _conservative = (self.kind_for(path) != Some(StorageKind::LocalSsd))
                    .then(|| self.conservative.enter());
                thread::sleep(Duration::from_micros(100));
            }
            if !emit(entry.clone()) {
                break;
            }
        }
        Ok(())
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        let _io = self.io.enter();
        let _conservative = (self.kind_for(path.parent().unwrap()) != Some(StorageKind::LocalSsd))
            .then(|| self.conservative.enter());
        thread::sleep(self.metadata_delay);
        if let Some(probe) = &self.fairness_probe
            && path.parent() == Some(probe.producer.as_path())
        {
            probe.completed_metadata.fetch_add(1, Ordering::SeqCst);
        }
        if self.missing.contains(path) {
            return Err(FsError::from_io(
                "test metadata",
                path,
                io::ErrorKind::NotFound.into(),
            ));
        }
        Ok(EntryMetadata {
            size: self.lengths.get(path).copied(),
            modified: None,
            ..EntryMetadata::default()
        })
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        unreachable!()
    }
}
impl StorageService for Files {
    fn storage_kind(&self, path: &Path) -> Result<StorageKind, FsError> {
        let _query = self.classifications.enter();
        self.queries.lock().unwrap().push(path.into());
        thread::sleep(Duration::from_millis(1));
        self.kind_for(path).ok_or_else(|| {
            FsError::from_io("test storage hint", path, io::ErrorKind::Unsupported.into())
        })
    }
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        unreachable!()
    }
}

fn root(id: u64, path: &str) -> Entry {
    Entry::new(EntryId(id), path.into(), EntryKind::Directory)
}

fn start(
    files: Arc<Files>,
    roots: Vec<Entry>,
    budget: Arc<IoBudget>,
) -> thread::JoinHandle<Vec<(EntryId, FolderSize)>> {
    thread::spawn(move || {
        let mut results = vec![];
        compute_batched(
            files.as_ref(),
            &roots,
            &Cancellation::default(),
            &mut |id, size| {
                if size.finished {
                    results.push((id, size));
                }
                true
            },
            &budget,
        );
        results
    })
}

fn queued(budget: &IoBudget, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while budget.io.state.lock().unwrap().queue.len() < count {
        assert!(
            Instant::now() < deadline,
            "Waiting I/O request did not enqueue"
        );
        thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn a_small_ssd_root_finishes_while_another_root_is_blocked_in_io() {
    let block = Arc::new(Blocker::default());
    let _release = ReleaseOnDrop(block.clone());
    let mut files = Files {
        kind: StorageKind::LocalSsd,
        ..Files::default()
    };
    files.files("big", 1);
    files.files("small", 1);
    files.opens.insert("big".into(), block.clone());
    let files = Arc::new(files);
    let budget = Arc::new(IoBudget::new());
    let (finished, results) = mpsc::channel();
    let worker_files = files.clone();
    let worker = thread::spawn(move || {
        compute_batched(
            worker_files.as_ref(),
            &[root(1, "big"), root(2, "small")],
            &Cancellation::default(),
            &mut |id, size| {
                if size.finished {
                    finished.send(id).unwrap();
                }
                true
            },
            &budget,
        );
    });
    block.wait(1);
    assert_eq!(
        results.recv_timeout(Duration::from_secs(3)).unwrap(),
        EntryId(2)
    );
    block.release();
    worker.join().unwrap();
    assert_eq!(results.recv().unwrap(), EntryId(1));
    assert!(files.io.maximum.load(Ordering::SeqCst) <= 4);
}

fn shared_operations(kind: StorageKind, expected: usize) {
    let block = Arc::new(Blocker::default());
    let _release = ReleaseOnDrop(block.clone());
    let mut files = Files {
        kind,
        metadata_delay: Duration::from_millis(1),
        ..Files::default()
    };
    let roots = (0..8)
        .map(|id| {
            let path = format!("root-{id}");
            files.files(&path, 3);
            files.opens.insert(path.clone().into(), block.clone());
            root(id, &path)
        })
        .collect::<Vec<_>>();
    let files = Arc::new(files);
    let budget = Arc::new(IoBudget::new());
    let first = start(files.clone(), roots[..4].to_vec(), budget.clone());
    let second = start(files.clone(), roots[4..].to_vec(), budget);
    block.wait(expected);
    thread::sleep(Duration::from_millis(10));
    assert_eq!(files.io.maximum.load(Ordering::SeqCst), expected);
    block.release();
    let results = first
        .join()
        .unwrap()
        .into_iter()
        .chain(second.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.len(), 8);
    assert!(
        results
            .iter()
            .all(|(_, size)| size.bytes == 3 && size.errors == 0)
    );
    assert_eq!(files.io.maximum.load(Ordering::SeqCst), expected);
    assert_eq!(files.classifications.maximum.load(Ordering::SeqCst), 1);
}

#[test]
fn two_panes_share_at_most_four_ssd_reads() {
    shared_operations(StorageKind::LocalSsd, 4);
}

#[test]
fn rotational_network_and_unknown_reads_are_serial_across_two_operations() {
    for kind in [
        StorageKind::Rotational,
        StorageKind::Network,
        StorageKind::Unknown,
    ] {
        shared_operations(kind, 1);
    }
}

#[test]
fn fifo_tickets_prevent_a_new_small_request_from_bypassing_a_waiting_exclusive_request() {
    let budget = Arc::new(IoBudget::new());
    let held = budget.acquire_io(4, &Cancellation::default()).unwrap();
    let (granted, order) = mpsc::channel();
    let first_budget = budget.clone();
    let first_granted = granted.clone();
    let first = thread::spawn(move || {
        let _permit = first_budget
            .acquire_io(4, &Cancellation::default())
            .unwrap();
        first_granted.send(1).unwrap();
    });
    queued(&budget, 1);
    let second_budget = budget.clone();
    let second = thread::spawn(move || {
        let _permit = second_budget
            .acquire_io(1, &Cancellation::default())
            .unwrap();
        granted.send(2).unwrap();
    });
    queued(&budget, 2);
    drop(held);
    assert_eq!(order.recv_timeout(Duration::from_secs(3)).unwrap(), 1);
    assert_eq!(order.recv_timeout(Duration::from_secs(3)).unwrap(), 2);
    first.join().unwrap();
    second.join().unwrap();
}

#[test]
fn a_network_scan_yields_its_open_iterator_so_a_small_waiting_root_finishes_first() {
    let block = Arc::new(Blocker::default());
    let _release = ReleaseOnDrop(block.clone());
    let mut files = Files {
        kind: StorageKind::Network,
        ..Files::default()
    };
    // One complete quantum plus one entry is enough to require a yield. A much
    // larger fixture only multiplied platform sleep granularity and scheduler
    // load without strengthening the fairness check.
    let big_files = QUANTUM_ENTRIES + 1;
    files.files("big", big_files);
    files.files("small", 1);
    files.fairness_probe = Some(Arc::new(FairnessProbe {
        producer: "big".into(),
        consumer: "small".into(),
        total_files: big_files,
        completed_metadata: AtomicUsize::new(0),
    }));
    files.opens.insert("big".into(), block.clone());
    let files = Arc::new(files);
    let budget = Arc::new(IoBudget::new());
    let (completed, order) = mpsc::channel();
    let big_files = files.clone();
    let big_budget = budget.clone();
    let big_completed = completed.clone();
    let big = thread::spawn(move || {
        compute_batched(
            big_files.as_ref(),
            &[root(1, "big")],
            &Cancellation::default(),
            &mut |id, size| {
                if size.finished {
                    big_completed.send(id).unwrap();
                }
                true
            },
            &big_budget,
        );
    });
    block.wait(1);
    let small_budget = budget.clone();
    let small = thread::spawn(move || {
        compute_batched(
            files.as_ref(),
            &[root(2, "small")],
            &Cancellation::default(),
            &mut |id, size| {
                if size.finished {
                    completed.send(id).unwrap();
                }
                true
            },
            &small_budget,
        );
    });
    queued(&budget, 1);
    block.release();
    assert_eq!(
        order.recv_timeout(Duration::from_secs(3)).unwrap(),
        EntryId(2)
    );
    assert_eq!(
        order.recv_timeout(Duration::from_secs(3)).unwrap(),
        EntryId(1)
    );
    big.join().unwrap();
    small.join().unwrap();
}

#[test]
fn nested_mount_hint_errors_use_conservative_io_without_losing_totals() {
    let mut files = Files {
        kind: StorageKind::LocalSsd,
        ..Files::default()
    };
    for parent in ["left", "right"] {
        let nested = format!("{parent}/mounted");
        files
            .directories
            .insert(parent.into(), vec![root(50, &nested)]);
        files.files(&nested, 3);
        files.overrides.insert(nested.into(), None);
    }
    let files = Arc::new(files);
    let results = start(
        files.clone(),
        vec![root(1, "left"), root(2, "right")],
        Arc::new(IoBudget::new()),
    )
    .join()
    .unwrap();
    assert_eq!(results.len(), 2);
    assert!(
        results
            .iter()
            .all(|(_, size)| size.bytes == 3 && size.errors == 0)
    );
    assert_eq!(files.conservative.maximum.load(Ordering::SeqCst), 1);
    let queried = files.queries.lock().unwrap();
    assert!(queried.contains(&PathBuf::from("left/mounted")));
    assert!(queried.contains(&PathBuf::from("right/mounted")));
}

#[test]
fn rejected_progress_cancels_queued_and_running_workers_without_leaking_io_slots() {
    let mut files = Files {
        kind: StorageKind::LocalSsd,
        ..Files::default()
    };
    let roots = (0..100)
        .map(|index| {
            let path = format!("root-{index}");
            files.files(&path, 0);
            root(index, &path)
        })
        .collect::<Vec<_>>();
    let files = Arc::new(files);
    let budget = Arc::new(IoBudget::new());
    compute_batched(
        files.as_ref(),
        &roots,
        &Cancellation::default(),
        &mut |_, _| {
            thread::sleep(Duration::from_millis(30));
            false
        },
        &budget,
    );
    assert_eq!(budget.io.state.lock().unwrap().available, 4);
    assert!(budget.io.state.lock().unwrap().queue.is_empty());
    assert_eq!(budget.classification.state.lock().unwrap().available, 1);
    assert!(budget.classification.state.lock().unwrap().queue.is_empty());
}

#[test]
fn batched_unicode_totals_preserve_each_root_identity_and_partial_errors() {
    let mut files = Files {
        kind: StorageKind::LocalSsd,
        ..Files::default()
    };
    let directory = "文档/👩🏽‍💻";
    let nested = "文档/👩🏽‍💻/مرحبا";
    files.files(directory, 3);
    files.files(nested, 2);
    files.files("prázdná", 0);
    files
        .missing
        .insert(Path::new(directory).join("žluťoučký-1.txt"));
    let entries = files.directories.get_mut(Path::new(directory)).unwrap();
    entries.push(root(80, nested));
    entries.push(Entry::new(
        EntryId(81),
        Path::new(directory).join("cycle"),
        EntryKind::Symlink,
    ));
    entries.push(Entry::new(
        EntryId(82),
        Path::new(directory).join("device"),
        EntryKind::Other,
    ));
    entries.push(Entry::new(
        EntryId(83),
        Path::new(directory).join("junction"),
        EntryKind::DirectoryLink,
    ));
    let results = start(
        Arc::new(files),
        vec![
            root(1, directory),
            root(2, "prázdná"),
            Entry::new(
                EntryId(3),
                "never-probe-link-root".into(),
                EntryKind::DirectoryLink,
            ),
        ],
        Arc::new(IoBudget::new()),
    )
    .join()
    .unwrap()
    .into_iter()
    .collect::<HashMap<_, _>>();
    assert_eq!(
        results[&EntryId(1)],
        FolderSize {
            bytes: 4,
            files: 4,
            skipped_links: 2,
            errors: 1,
            finished: true,
        }
    );
    assert_eq!(
        results.len(),
        2,
        "a directory-link root must not start a scan"
    );
    assert_eq!(
        results[&EntryId(2)],
        FolderSize {
            finished: true,
            ..FolderSize::default()
        }
    );
}

#[test]
fn external_cancellation_removes_waiting_root_workers_without_releasing_someone_elses_io() {
    let budget = Arc::new(IoBudget::new());
    let held = budget.acquire_io(4, &Cancellation::default()).unwrap();
    let cancel = Cancellation::default();
    let mut files = Files {
        kind: StorageKind::LocalSsd,
        ..Files::default()
    };
    files.files("left", 1);
    files.files("right", 1);
    let files = Arc::new(files);
    let worker_files = files.clone();
    let worker_budget = budget.clone();
    let worker_cancel = cancel.clone();
    let (finished, completed) = mpsc::channel();
    let worker = thread::spawn(move || {
        compute_batched(
            worker_files.as_ref(),
            &[root(1, "left"), root(2, "right")],
            &worker_cancel,
            &mut |_, _| true,
            &worker_budget,
        );
        finished.send(()).unwrap();
    });
    queued(&budget, 1);
    cancel.cancel();
    completed.recv_timeout(Duration::from_secs(3)).unwrap();
    worker.join().unwrap();
    assert_eq!(files.io.maximum.load(Ordering::SeqCst), 0);
    assert_eq!(budget.io.state.lock().unwrap().available, 0);
    assert!(budget.io.state.lock().unwrap().queue.is_empty());
    drop(held);
    assert_eq!(budget.io.state.lock().unwrap().available, 4);
}
