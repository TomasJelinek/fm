//! Selections of more items than a job prepares before it starts: the job keeps their
//! paths and identities and takes each item when it reaches it.
use super::*;
use midnight_core::{DiscoveryScope, DiscoveryState};

/// More items than `MAX_PREPARED_NATIVE_ROOTS`.
const MANY: usize = input::MAX_PREPARED_NATIVE_ROOTS * 3 + 1;

fn streamed_request(
    paths: Vec<PathBuf>,
    destination: &Path,
    moved: bool,
) -> input::StreamedRequest {
    input::StreamedRequest {
        destination: destination.to_owned(),
        verification: TransferVerification::FullSha256,
        paths: Arc::new(FileClipboard {
            paths,
            intent: if moved {
                FileClipboardIntent::Move
            } else {
                FileClipboardIntent::Copy
            },
        }),
        moved,
    }
}

fn status_for(intent: TransferIntent, destination: &Path) -> Status {
    let mut progress = initial(&CopyInput::from(request(1, &[], destination)));
    progress.intent = intent;
    Arc::new(Mutex::new(progress))
}

fn files(folder: &Path, count: usize) -> Vec<PathBuf> {
    fs::create_dir_all(folder).unwrap();
    (0..count)
        .map(|index| {
            let path = folder.join(format!("file {index:05}.txt"));
            fs::write(&path, format!("contents of {index}")).unwrap();
            path
        })
        .collect()
}

fn prepare(
    paths: Vec<PathBuf>,
    destination: &Path,
    intent: TransferIntent,
) -> (worker::Job, Status) {
    let status = status_for(intent, destination);
    let mut job = worker::Job::prepare_streamed(
        streamed_request(paths, destination, intent == TransferIntent::Move),
        Arc::new(midnight_platform::LocalFileSystem::new(None)),
        Cancellation::default(),
        status.clone(),
        CopyLinkPolicy::Reject,
    )
    .unwrap();
    job.prepare_discovery();
    (job, status)
}

/// Steps `job` to its end, answering each failure with `answer`; returns the failures.
fn run(
    job: &mut worker::Job,
    mut answer: impl FnMut(&CopyFailure) -> CopyChoice,
) -> Vec<Arc<CopyFailure>> {
    let mut failures = Vec::new();
    for _ in 0..1_000_000 {
        match job.step() {
            worker::Step::Done => return failures,
            worker::Step::Continue => {}
            worker::Step::Failure(failure) => {
                let choice = answer(&failure);
                assert!(job.accept_decision(CopyDecision {
                    id: failure.id,
                    version: failure.version,
                    choice,
                }));
                failures.push(failure);
            }
            worker::Step::Conflict(conflict) => panic!("unexpected conflict {conflict:?}"),
        }
    }
    panic!("the job did not end within bounded steps");
}

#[test]
fn a_streamed_copy_takes_each_item_and_totals_them_item_by_item() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let mut paths = files(&source, MANY);
    let folder = source.join("folder");
    files(&folder, 3);
    paths.push(folder);
    let (mut job, status) = prepare(paths.clone(), &destination, TransferIntent::Copy);
    // Nothing is read before the job takes its first item: the discovery is still
    // scanning, and no item was opened to total it.
    let started = snapshot(&status);
    assert_eq!(started.discovery.state, DiscoveryState::Scanning);
    assert_eq!(started.discovery.scope, DiscoveryScope::Recursive);
    assert_eq!(started.discovery.entries(), Some(0));
    assert_eq!(started.requested_roots, paths.len() as u64);
    assert!(run(&mut job, |failure| panic!("{failure:?}")).is_empty());
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.completed_roots, paths.len() as u64);
    assert_eq!(done.files_completed, MANY as u64 + 3);
    assert_eq!(
        done.discovery.state,
        DiscoveryState::Complete,
        "{:?}",
        done.discovery
    );
    assert_eq!(
        (done.discovery.files, done.discovery.directories),
        (MANY as u64 + 3, 1)
    );
    assert_eq!(done.discovery.bytes, Some(done.completed_bytes));
    for path in &paths[..MANY] {
        assert_eq!(
            fs::read(destination.join(path.file_name().unwrap())).unwrap(),
            fs::read(path).unwrap()
        );
    }
    assert_eq!(fs::read_dir(destination.join("folder")).unwrap().count(), 3);
}

#[test]
fn an_item_replaced_after_its_identity_was_read_fails_alone_and_is_never_copied() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let paths = files(&source, MANY);
    let (mut job, _status) = prepare(paths.clone(), &destination, TransferIntent::Copy);
    job.identify_streamed_for_test();
    // Another file now has the replaced item's name: a new identity.
    let replaced = &paths[MANY / 2];
    fs::remove_file(replaced).unwrap();
    fs::write(replaced, b"something else").unwrap();
    let failures = run(&mut job, |failure| {
        assert!(failure.can_skip && !failure.can_retry, "{failure:?}");
        CopyChoice::Skip
    });
    assert_eq!(failures.len(), 1, "{failures:?}");
    assert_eq!(failures[0].source.as_deref(), Some(replaced.as_path()));
    assert!(
        failures[0]
            .error
            .message
            .contains("replaced or changed after the Copy started"),
        "{}",
        failures[0].error
    );
    let done = job.finish();
    assert_eq!(
        done.state,
        CopyOperationState::CompletedWithErrors,
        "{done:?}"
    );
    assert_eq!((done.failed, done.skipped), (1, 1));
    assert_eq!(done.files_completed, MANY as u64 - 1);
    assert_eq!(done.completed_roots, MANY as u64 - 1);
    assert!(!destination.join(replaced.file_name().unwrap()).exists());
    assert_eq!(fs::read_dir(&destination).unwrap().count(), MANY - 1);
}

#[test]
fn a_vanished_item_fails_alone_and_retry_takes_it_again() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    let paths = files(&source, MANY);
    let (mut job, _status) = prepare(paths.clone(), &destination, TransferIntent::Copy);
    job.identify_streamed_for_test();
    let vanished = paths[7].clone();
    let contents = fs::read(&vanished).unwrap();
    fs::remove_file(&vanished).unwrap();
    let mut attempts = 0;
    let failures = run(&mut job, |failure| {
        attempts += 1;
        assert!(failure.can_retry && failure.can_skip, "{failure:?}");
        assert_eq!(
            failure.error.message,
            "removed from the source while copying"
        );
        if attempts == 1 {
            CopyChoice::Retry
        } else {
            CopyChoice::Skip
        }
    });
    assert_eq!(failures.len(), 2);
    let done = job.finish();
    assert_eq!(done.files_completed, MANY as u64 - 1, "{done:?}");
    assert!(!destination.join(vanished.file_name().unwrap()).exists());
    // Once the item is back, the same selection copies every item.
    fs::write(&vanished, &contents).unwrap();
    let again = temp.path().join("again");
    fs::create_dir(&again).unwrap();
    let (mut job, _status) = prepare(paths, &again, TransferIntent::Copy);
    assert!(run(&mut job, |failure| panic!("{failure:?}")).is_empty());
    assert_eq!(job.finish().files_completed, MANY as u64);
}

#[test]
fn a_target_that_is_a_selected_item_not_read_yet_is_never_offered_for_replacement() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    // The first item conflicts with the last one, which is in the destination: a
    // thousand items in between keep the pass from reading it within its slice.
    let mut paths = files(&source, 1_000);
    let selected_target = destination.join(paths[0].file_name().unwrap());
    fs::create_dir(&destination).unwrap();
    fs::write(&selected_target, b"a selected item").unwrap();
    paths.push(selected_target.clone());
    let first_conflict = |paths: Vec<PathBuf>| {
        let (mut job, _status) = prepare(paths, &destination, TransferIntent::Copy);
        let conflict = loop {
            match job.step() {
                worker::Step::Continue => {}
                worker::Step::Conflict(conflict) => break conflict,
                other => panic!(
                    "expected a conflict, got {}",
                    matches!(other, worker::Step::Done)
                ),
            }
        };
        job.finish_abandoned();
        conflict
    };
    let conflict = first_conflict(paths.clone());
    assert_eq!(conflict.destination, selected_target);
    assert!(!conflict.can_replace, "a selected item would be replaced");
    assert_eq!(fs::read(&selected_target).unwrap(), b"a selected item");
    // The same target, not selected, may be replaced.
    paths.pop();
    let conflict = first_conflict(paths);
    assert_eq!(conflict.destination, selected_target);
    assert!(conflict.can_replace, "an ordinary target is replaceable");
}

#[test]
fn a_streamed_move_moves_each_item_and_keeps_no_item_prepared_ahead() {
    let (_temp, root) = moves::protected_fixture();
    let source = root.join("source");
    let destination = root.join("destination");
    fs::create_dir(&destination).unwrap();
    let mut paths = files(&source, MANY);
    let folder = source.join("folder");
    files(&folder, 2);
    paths.push(folder.clone());
    let (mut job, status) = prepare(paths.clone(), &destination, TransferIntent::Move);
    // A Move on the source's own volume renames: its totals count the selected items.
    assert_eq!(
        snapshot(&status).discovery.scope,
        DiscoveryScope::SelectedRoots
    );
    assert!(run(&mut job, |failure| panic!("{failure:?}")).is_empty());
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.completed_roots, paths.len() as u64);
    assert_eq!(done.native_entries_moved, paths.len() as u64);
    assert_eq!(fs::read_dir(&source).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&destination).unwrap().count(), MANY + 1);
    assert_eq!(fs::read_dir(destination.join("folder")).unwrap().count(), 2);
}

/// Across volumes a streamed Move copies each item as Copy does, under the job's
/// verification policy (full or sampled), and removes its source once that copy is
/// verified; an item that fails and is skipped keeps its source, and only that one.
#[test]
fn a_streamed_move_across_volumes_removes_each_source_only_after_its_verified_copy() {
    for verification in [
        TransferVerification::FullSha256,
        TransferVerification::SizeAndSamples,
    ] {
        let (_temp, root) = moves::protected_fixture();
        let source = root.join("source");
        let destination = root.join("destination");
        fs::create_dir(&destination).unwrap();
        let mut paths = files(&source, MANY);
        let folder = source.join("folder");
        files(&folder, 2);
        paths.push(folder.clone());
        let expected: Vec<_> = paths[..MANY]
            .iter()
            .map(|path| fs::read(path).unwrap())
            .collect();
        let status = status_for(TransferIntent::Move, &destination);
        let mut request = streamed_request(paths.clone(), &destination, true);
        request.verification = verification;
        let mut job = worker::Job::prepare_streamed(
            request,
            Arc::new(moves::CrossDeviceService(
                midnight_platform::LocalFileSystem::new(None),
            )),
            Cancellation::default(),
            status.clone(),
            CopyLinkPolicy::Reject,
        )
        .unwrap();
        job.discover_cross_device_fixture();
        job.identify_streamed_for_test();
        // One item is replaced after its identity was read: it fails alone, and Skip
        // keeps the new file where it is.
        let replaced = paths[5].clone();
        fs::remove_file(&replaced).unwrap();
        fs::write(&replaced, b"another file").unwrap();
        let failures = run(&mut job, |failure| {
            assert_eq!(failure.source.as_deref(), Some(replaced.as_path()));
            CopyChoice::Skip
        });
        assert_eq!(failures.len(), 1, "{verification:?}: {failures:?}");
        let done = job.finish();
        let case = format!("{verification:?}: {done:?}");
        assert_eq!(
            done.state,
            CopyOperationState::CompletedWithErrors,
            "{case}"
        );
        assert_eq!(done.native_entries_moved, 0, "{case}");
        assert_eq!(done.source_files_removed, MANY as u64 + 1, "{case}");
        assert_eq!(done.completed_roots, MANY as u64, "{case}");
        assert!(
            done.verified_bytes > 0 && done.validation_bytes > 0,
            "{case}"
        );
        assert_eq!(fs::read(&replaced).unwrap(), b"another file");
        assert_eq!(
            support_names(&source),
            vec![replaced.file_name().unwrap().to_string_lossy().into_owned()]
        );
        for (index, path) in paths[..MANY].iter().enumerate() {
            if index == 5 {
                assert!(!destination.join(path.file_name().unwrap()).exists());
            } else {
                assert_eq!(
                    fs::read(destination.join(path.file_name().unwrap())).unwrap(),
                    expected[index],
                    "{case}"
                );
            }
        }
        assert_eq!(fs::read_dir(destination.join("folder")).unwrap().count(), 2);
    }
}

fn support_names(folder: &Path) -> Vec<String> {
    let mut names: Vec<_> = fs::read_dir(folder)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// On a case-insensitive volume the same folder under two spellings of its name is one
/// folder: a Move takes it once.
#[cfg(windows)]
#[test]
fn a_streamed_move_takes_a_folder_selected_twice_once() {
    let (_temp, root) = moves::protected_fixture();
    let source = root.join("source");
    let destination = root.join("destination");
    fs::create_dir(&destination).unwrap();
    let mut paths = files(&source, MANY);
    let folder = source.join("Folder");
    files(&folder, 2);
    paths.push(folder);
    paths.push(source.join("FOLDER"));
    let (mut job, _status) = prepare(paths.clone(), &destination, TransferIntent::Move);
    assert!(run(&mut job, |failure| panic!("{failure:?}")).is_empty());
    let done = job.finish();
    assert_eq!(done.state, CopyOperationState::Completed, "{done:?}");
    assert_eq!(done.completed_roots, done.requested_roots);
    assert_eq!(done.requested_roots, paths.len() as u64 - 1);
    assert_eq!(fs::read_dir(destination.join("Folder")).unwrap().count(), 2);
}

#[test]
fn a_streamed_selection_keeps_about_a_hundred_bytes_per_item_beside_its_paths() {
    let temp = fixture();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir(&destination).unwrap();
    // Far above the prepared-root limit. The retained bytes are capacities, which grow in
    // powers of two: an eighth of the earlier 20,000 items keeps every capacity's ratio to
    // the item count, so the bytes per item are the same, for an eighth of the files.
    let count = 2_500;
    let paths = files(&source, count);
    let path_bytes: usize = paths
        .iter()
        .map(|path| path.as_os_str().len())
        .sum::<usize>()
        + paths.capacity() * std::mem::size_of::<PathBuf>();
    let (mut job, _status) = prepare(paths, &destination, TransferIntent::Copy);
    job.identify_streamed_for_test();
    let (retained, identified) = job.streamed_retained_for_test();
    assert_eq!(identified, count);
    let per_item = (retained - path_bytes) / count;
    eprintln!("streamed selection: {per_item} bytes per item beside its paths");
    assert!(per_item <= 128, "{per_item} bytes per item");
}

/// The folder of the measurement fixture (`MIDNIGHT_STREAMED_DIR`, else a folder of the
/// temporary folder that the runs reuse), with `MIDNIGHT_STREAMED_ITEMS` files (default
/// 1,000,000) of 16 bytes each, created once; returns their paths in name order.
fn measured_items(name: &str) -> (PathBuf, Vec<PathBuf>) {
    let items: usize = std::env::var("MIDNIGHT_STREAMED_ITEMS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(1_000_000);
    let root = std::env::var_os("MIDNIGHT_STREAMED_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("midnight-streamed"));
    let folder = root.join(format!("{name}-{items}"));
    let paths: Vec<_> = (0..items)
        .map(|index| folder.join(format!("item {index:07}.bin")))
        .collect();
    if !paths.last().is_some_and(|path| path.exists()) {
        let started = std::time::Instant::now();
        fs::create_dir_all(&folder).unwrap();
        for (index, path) in paths.iter().enumerate() {
            fs::write(path, format!("{index:015}\n")).unwrap();
        }
        eprintln!("created {items} files in {:.1?}", started.elapsed());
    }
    (root, paths)
}

fn empty_folder(root: &Path, name: &str) -> PathBuf {
    let folder = root.join(name);
    if folder.exists() {
        fs::remove_dir_all(&folder).unwrap();
    }
    fs::create_dir_all(&folder).unwrap();
    folder
}

/// Measurement, not a check: a Job of `MIDNIGHT_STREAMED_ITEMS` items (default 1,000,000)
/// prepares, takes its first item and copies it, then reads every identity, and reports the
/// times and the bytes it keeps per item. Run in a release build:
/// `cargo test --release -p midnight-engine --lib streamed::measure_admission -- --ignored --nocapture`.
#[test]
#[ignore = "measurement of 1,000,000 items; run explicitly"]
fn measure_admission() {
    let (root, paths) = measured_items("copy");
    let destination = empty_folder(&root, "admission-destination");
    let count = paths.len();
    let path_bytes: usize = paths
        .iter()
        .map(|path| path.as_os_str().len())
        .sum::<usize>()
        + paths.capacity() * std::mem::size_of::<PathBuf>();
    let started = std::time::Instant::now();
    let (mut job, _status) = prepare(paths, &destination, TransferIntent::Copy);
    let prepared = started.elapsed();
    let first = loop {
        match job.step() {
            worker::Step::Continue if job.take_completed_root().is_some() => {
                break started.elapsed();
            }
            worker::Step::Continue => {}
            _ => panic!("the first item did not copy"),
        }
    };
    let pass = std::time::Instant::now();
    job.identify_streamed_for_test();
    let identified = pass.elapsed();
    let (retained, read) = job.streamed_retained_for_test();
    assert_eq!(read, count);
    eprintln!(
        "{count} items: prepared in {prepared:.1?}, first item copied after {first:.1?}; \
         every identity read in {identified:.1?} ({:.1} µs per item); retained {} MB, {} \
         bytes per item beside the paths ({} MB of paths)",
        identified.as_secs_f64() * 1e6 / count as f64,
        retained / 1_000_000,
        (retained - path_bytes) / count,
        path_bytes / 1_000_000,
    );
    job.finish_abandoned();
}

/// Measurement, not a check: the whole application path (the copy actor, admission, the
/// namespace claim) copies or moves `MIDNIGHT_STREAMED_ITEMS` files (default 1,000,000).
/// Reports the time to the first copied item and the total. `MIDNIGHT_STREAMED_MOVE_TO`
/// names a folder on another volume for a cross-volume Move; without it the Move stays on
/// one volume (renames). Run in a release build:
/// `cargo test --release -p midnight-engine --lib streamed::measure_transfer -- --ignored --nocapture`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measurement of 1,000,000 items; run explicitly"]
async fn measure_transfer() {
    let moved = std::env::var_os("MIDNIGHT_STREAMED_INTENT").is_some_and(|intent| intent == "move");
    let (root, paths) = measured_items(if moved { "move" } else { "copy" });
    let destination = match std::env::var_os("MIDNIGHT_STREAMED_MOVE_TO") {
        Some(other) if moved => empty_folder(Path::new(&other), "move-destination"),
        _ => empty_folder(
            &root,
            if moved {
                "move-destination"
            } else {
                "copy-destination"
            },
        ),
    };
    let mut h = Harness::native_with_events(1024);
    let started = std::time::Instant::now();
    // `MIDNIGHT_STREAMED_FOLDER`: the folder that holds the items instead, a prepared
    // selection of one item, for comparison.
    let request = if std::env::var_os("MIDNIGHT_STREAMED_FOLDER").is_some() {
        request(1, &[paths[0].parent().unwrap().to_owned()], &destination)
    } else {
        request(1, &paths, &destination)
    };
    let count = request.clipboard.paths.len();
    drop(paths);
    // `MIDNIGHT_STREAMED_SECONDS`: cancel the job after that long.
    let limit = std::env::var("MIDNIGHT_STREAMED_SECONDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .map(Duration::from_secs);
    if moved {
        h.controller.enqueue_move(request).await;
    } else {
        h.controller.enqueue(request).await;
    }
    let mut first = None;
    let mut cancelled = false;
    let mut last_report = std::time::Instant::now();
    let done = loop {
        let event = tokio::time::timeout(Duration::from_secs(600), h.events.recv())
            .await
            .expect("an event within 10 minutes")
            .expect("events");
        if !cancelled && limit.is_some_and(|limit| started.elapsed() >= limit) {
            h.controller.cancel(OperationId(1));
            cancelled = true;
        }
        match event {
            Event::Copy(CopyEvent::Progress(progress)) if progress.state.is_terminal() => {
                break progress;
            }
            Event::Copy(CopyEvent::Progress(progress)) => {
                if progress.files_completed > 0 {
                    first.get_or_insert_with(|| started.elapsed());
                }
                if last_report.elapsed() > Duration::from_secs(10) {
                    eprintln!(
                        "{:.0?}: {} files done, {} items; discovery {:?} {} files",
                        started.elapsed(),
                        progress.files_completed,
                        progress.completed_roots,
                        progress.discovery.state,
                        progress.discovery.files
                    );
                    last_report = std::time::Instant::now();
                }
            }
            Event::Copy(CopyEvent::Failure(failure)) => panic!("{failure:?}"),
            Event::Copy(CopyEvent::Conflict(conflict)) => panic!("{conflict:?}"),
            _ => {}
        }
    };
    let total = started.elapsed();
    eprintln!(
        "{} of {count} selected items: {:?}, first file done after {:.1?}; {} files in {total:.1?} ({:.0} files/s); {} files moved by rename",
        if moved { "Move" } else { "Copy" },
        done.state,
        first.unwrap_or_default(),
        done.files_completed,
        done.files_completed as f64 / total.as_secs_f64(),
        done.native_entries_moved,
    );
    if !cancelled {
        assert_eq!(
            done.state,
            CopyOperationState::Completed,
            "{:?}",
            done.errors
        );
        assert_eq!(done.completed_roots, count as u64);
    }
    h.close().await;
}
