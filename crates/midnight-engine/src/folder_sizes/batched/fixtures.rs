use super::*;
use midnight_core::{SpaceInfo, StorageService, Volume, VolumeService};
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
    producer_work_at_consumer_completion: AtomicUsize,
}

#[derive(Default)]
struct Files {
    directories: HashMap<PathBuf, Vec<Entry>>,
    lengths: HashMap<PathBuf, u64>,
    kind: StorageKind,
    overrides: HashMap<PathBuf, Option<StorageKind>>,
    queries: Mutex<Vec<PathBuf>>,
    root_threads: Mutex<HashSet<thread::ThreadId>>,
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
        self.root_threads
            .lock()
            .unwrap()
            .insert(thread::current().id());
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
        if let Some(probe) = &self.fairness_probe
            && path.parent() == Some(probe.consumer.as_path())
        {
            probe.producer_work_at_consumer_completion.store(
                probe.completed_metadata.load(Ordering::SeqCst),
                Ordering::SeqCst,
            );
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
            &crate::ReportArena::testing(),
        );
        results
    })
}

fn queued(budget: &IoBudget, count: usize) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while budget.io.snapshot().queued < count {
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
    let budget = Arc::new(IoBudget::isolated());
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
            &crate::ReportArena::testing(),
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
    let budget = Arc::new(IoBudget::isolated());
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
    let budget = Arc::new(IoBudget::isolated());
    let held = budget
        .acquire_io(Admission::Exclusive, &Cancellation::default())
        .unwrap();
    let (granted, order) = mpsc::channel();
    let first_budget = budget.clone();
    let first_granted = granted.clone();
    let first = thread::spawn(move || {
        let _permit = first_budget
            .acquire_io(Admission::Exclusive, &Cancellation::default())
            .unwrap();
        first_granted.send(1).unwrap();
    });
    queued(&budget, 1);
    let second_budget = budget.clone();
    let second = thread::spawn(move || {
        let _permit = second_budget
            .acquire_io(Admission::Shared, &Cancellation::default())
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
fn a_network_scan_yields_its_open_iterator_so_a_small_waiting_root_finishes_io_first() {
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
    let probe = Arc::new(FairnessProbe {
        producer: "big".into(),
        consumer: "small".into(),
        total_files: big_files,
        completed_metadata: AtomicUsize::new(0),
        producer_work_at_consumer_completion: AtomicUsize::new(usize::MAX),
    });
    files.fairness_probe = Some(probe.clone());
    files.opens.insert("big".into(), block.clone());
    let files = Arc::new(files);
    let budget = Arc::new(IoBudget::isolated());
    let (completed, order) = mpsc::channel();
    let (big_reported, wait_for_big_report) = mpsc::channel();
    let big_source = files.clone();
    let big_budget = budget.clone();
    let big_completed = completed.clone();
    let big = thread::spawn(move || {
        compute_batched(
            big_source.as_ref(),
            &[root(1, "big")],
            &Cancellation::default(),
            &mut |id, size| {
                if size.finished {
                    big_completed.send((id, size)).unwrap();
                    big_reported.send(()).unwrap();
                }
                true
            },
            &big_budget,
            &crate::ReportArena::testing(),
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
                    // These are separate operations with independent result
                    // receivers. Their callbacks can run in either order after
                    // releasing I/O. Deliberately deliver the small result last.
                    wait_for_big_report
                        .recv_timeout(Duration::from_secs(3))
                        .unwrap();
                    completed.send((id, size)).unwrap();
                }
                true
            },
            &small_budget,
            &crate::ReportArena::testing(),
        );
    });
    queued(&budget, 1);
    block.release();
    let (big_id, big_size) = order.recv_timeout(Duration::from_secs(3)).unwrap();
    let (small_id, small_size) = order.recv_timeout(Duration::from_secs(3)).unwrap();
    big.join().unwrap();
    small.join().unwrap();
    assert_eq!(
        (big_id, big_size.bytes, big_size.errors),
        (EntryId(1), big_files as u64, 0)
    );
    assert_eq!(
        (small_id, small_size.bytes, small_size.errors),
        (EntryId(2), 1, 0)
    );
    assert!(
        (1..big_files).contains(
            &probe
                .producer_work_at_consumer_completion
                .load(Ordering::SeqCst)
        ),
        "the small root must finish its I/O while the large root still has work"
    );
    assert_eq!(probe.completed_metadata.load(Ordering::SeqCst), big_files);
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
        Arc::new(IoBudget::isolated()),
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
    let budget = Arc::new(IoBudget::isolated());
    compute_batched(
        files.as_ref(),
        &roots,
        &Cancellation::default(),
        &mut |_, _| {
            thread::sleep(Duration::from_millis(30));
            false
        },
        &budget,
        &crate::ReportArena::testing(),
    );
    assert_eq!(budget.io.snapshot().available, 4);
    assert_eq!(budget.io.snapshot().queued, 0);
    assert_eq!(budget.classification.snapshot().available, 1);
    assert_eq!(budget.classification.snapshot().queued, 0);
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
                "unsupported-link-root".into(),
                EntryKind::DirectoryLink,
            ),
        ],
        Arc::new(IoBudget::isolated()),
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
            errors: 3,
            finished: true,
            warnings: results[&EntryId(1)].warnings.clone(),
        }
    );
    assert_eq!(
        results[&EntryId(1)]
            .warnings
            .as_ref()
            .unwrap()
            .summary()
            .observed,
        3
    );
    assert_eq!(
        results.len(),
        3,
        "an unsupported directory-link root reports its own partial result"
    );
    assert_eq!(
        (
            results[&EntryId(3)].errors,
            results[&EntryId(3)].skipped_links
        ),
        (1, 1)
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
    let budget = Arc::new(IoBudget::isolated());
    let held = budget
        .acquire_io(Admission::Exclusive, &Cancellation::default())
        .unwrap();
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
            &crate::ReportArena::testing(),
        );
        finished.send(()).unwrap();
    });
    queued(&budget, 1);
    cancel.cancel();
    completed.recv_timeout(Duration::from_secs(3)).unwrap();
    worker.join().unwrap();
    assert_eq!(files.io.maximum.load(Ordering::SeqCst), 0);
    assert_eq!(budget.io.snapshot().available, 0);
    assert_eq!(budget.io.snapshot().queued, 0);
    drop(held);
    assert_eq!(budget.io.snapshot().available, 4);
}

// Source-only boundary tests authored before configurable team implementation.
fn blocked_roots(count: usize, kind: StorageKind) -> (Arc<Files>, Vec<Entry>, Arc<Blocker>) {
    let block = Arc::new(Blocker::default());
    let mut files = Files {
        kind,
        ..Files::default()
    };
    let roots = (0..count)
        .map(|id| {
            let path = format!("configured-root-{id}");
            files.files(&path, 1);
            files.opens.insert(path.clone().into(), block.clone());
            root(id as u64, &path)
        })
        .collect();
    (Arc::new(files), roots, block)
}

#[test]
fn a_new_team_can_use_six_ssd_lanes_and_finishes_every_root() {
    let budget = Arc::new(IoBudget::isolated());
    budget.io.set_limit(6).unwrap();
    let (files, roots, block) = blocked_roots(9, StorageKind::LocalSsd);
    let _release = ReleaseOnDrop(block.clone());
    let worker = start(files.clone(), roots, budget.clone());
    block.wait(6);
    assert_eq!(files.io.maximum.load(Ordering::SeqCst), 6);
    assert_eq!(budget.io.snapshot().active_shared, 6);
    block.release();
    let results = worker.join().unwrap();
    assert_eq!(results.len(), 9);
    assert!(
        results
            .iter()
            .all(|(_, size)| size.bytes == 1 && size.errors == 0)
    );
    assert_eq!(files.io.maximum.load(Ordering::SeqCst), 6);
    assert_eq!(budget.io.snapshot().available, 6);
}

#[test]
fn an_existing_one_lane_team_stays_small_after_growth_but_the_next_team_uses_six() {
    let budget = Arc::new(IoBudget::isolated());
    budget.io.set_limit(1).unwrap();
    let (files, roots, block) = blocked_roots(9, StorageKind::LocalSsd);
    let _release = ReleaseOnDrop(block.clone());
    let worker = start(files.clone(), roots, budget.clone());
    block.wait(1);
    budget.io.set_limit(6).unwrap();
    block.release();
    assert_eq!(worker.join().unwrap().len(), 9);
    // Once the gate has six slots, extra root lanes would overlap on real callbacks.
    assert_eq!(files.io.maximum.load(Ordering::SeqCst), 1);
    assert_eq!(files.root_threads.lock().unwrap().len(), 1);
    let (next_files, next_roots, next_block) = blocked_roots(9, StorageKind::LocalSsd);
    let _next_release = ReleaseOnDrop(next_block.clone());
    let next = start(next_files.clone(), next_roots, budget.clone());
    next_block.wait(6);
    assert_eq!(next_files.io.maximum.load(Ordering::SeqCst), 6);
    next_block.release();
    assert_eq!(next.join().unwrap().len(), 9);
}

#[test]
fn an_existing_six_lane_team_drains_after_shrink_without_releasing_blocked_io() {
    let budget = Arc::new(IoBudget::isolated());
    budget.io.set_limit(6).unwrap();
    let (files, roots, block) = blocked_roots(12, StorageKind::LocalSsd);
    let _release = ReleaseOnDrop(block.clone());
    let worker = start(files.clone(), roots, budget.clone());
    block.wait(6);
    budget.io.set_limit(1).unwrap();
    assert_eq!(budget.io.snapshot().active_shared, 6);
    assert_eq!(budget.io.snapshot().available, 0);
    block.release();
    assert_eq!(worker.join().unwrap().len(), 12);
    assert_eq!(budget.io.snapshot().active_shared, 0);
    assert_eq!(budget.io.snapshot().available, 1);
    let (next_files, next_roots, next_block) = blocked_roots(8, StorageKind::LocalSsd);
    let _next_release = ReleaseOnDrop(next_block.clone());
    let next = start(next_files.clone(), next_roots, budget.clone());
    next_block.wait(1);
    next_block.release();
    assert_eq!(next.join().unwrap().len(), 8);
    assert_eq!(next_files.io.maximum.load(Ordering::SeqCst), 1);
}

#[test]
fn four_maximum_teams_share_one_32_io_limit() {
    let budget = Arc::new(IoBudget::isolated());
    budget.io.set_limit(32).unwrap();
    let (files, roots, block) = blocked_roots(160, StorageKind::LocalSsd);
    let _release = ReleaseOnDrop(block.clone());
    let workers: Vec<_> = roots
        .chunks(40)
        .map(|roots| start(files.clone(), roots.to_vec(), budget.clone()))
        .collect();
    block.wait(32);
    queued(&budget, 96);
    // Four sampled teams of 32 lanes: 32 callbacks hold admission, 96 lanes await it.
    assert_eq!(budget.io.snapshot().queued, 96);
    assert_eq!(budget.io.snapshot().active_shared, 32);
    assert_eq!(files.io.maximum.load(Ordering::SeqCst), 32);
    block.release();
    let results: Vec<_> = workers
        .into_iter()
        .flat_map(|worker| worker.join().unwrap())
        .collect();
    assert_eq!(results.len(), 160);
    assert!(
        results
            .iter()
            .all(|(_, size)| size.bytes == 1 && size.errors == 0)
    );
    assert_eq!(files.io.maximum.load(Ordering::SeqCst), 32);
    assert_eq!(files.root_threads.lock().unwrap().len(), 128);
    assert_eq!(budget.io.snapshot().queued, 0);
    assert_eq!(budget.io.snapshot().available, 32);
}

#[test]
fn manual_six_keeps_rotational_network_and_unknown_scans_exclusive() {
    for kind in [
        StorageKind::Rotational,
        StorageKind::Network,
        StorageKind::Unknown,
    ] {
        let budget = Arc::new(IoBudget::isolated());
        budget.io.set_limit(6).unwrap();
        let (files, roots, block) = blocked_roots(8, kind);
        let _release = ReleaseOnDrop(block.clone());
        let worker = start(files.clone(), roots, budget.clone());
        block.wait(1);
        queued(&budget, 5);
        assert!(budget.io.snapshot().exclusive);
        assert_eq!(files.io.maximum.load(Ordering::SeqCst), 1);
        block.release();
        assert_eq!(worker.join().unwrap().len(), 8);
        assert_eq!(files.io.maximum.load(Ordering::SeqCst), 1);
        assert_eq!(budget.io.snapshot().available, 6);
    }
}
