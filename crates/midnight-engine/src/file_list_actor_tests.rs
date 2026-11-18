use super::*;
use std::time::Duration;
use tokio::sync::mpsc;

pub(super) fn request(id: u64, generation: u64) -> FileListRequest {
    let entries: Arc<[Entry]> =
        vec![Entry::new(EntryId(1), "文档.txt".into(), EntryKind::File)].into();
    FileListRequest {
        id: DesktopRequestId(id),
        generation,
        template: "$(FileName)$(LF)".into(),
        destination: FileListDestination::Preview,
        capture: Arc::new(FileListCapture {
            directory: String::new(),
            source: FileListSource::Native {
                view: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
                selection: Arc::new(SelectionSnapshot::default()),
                fallback: Some(EntryId(1)),
            },
        }),
    }
}

pub(super) fn controller() -> (Controller, mpsc::Receiver<Event>, JoinSet<()>) {
    let (sender, events) = mpsc::channel(16);
    let outbox = Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    };
    (
        Controller::testing(
            None,
            crate::transfer_leases::namespace_gate(),
            outbox,
            crate::test_snapshots(Default::default()),
        ),
        events,
        JoinSet::new(),
    )
}

#[tokio::test]
async fn completed_folder_sizes_survive_template_edits_and_later_navigation() {
    let (mut controller, mut events, mut tasks) = controller();
    let root = std::env::temp_dir().join("file-list-captured-size");
    let entry = Entry::new(EntryId(1), root.join("folder"), EntryKind::Directory);
    let entries: Arc<[Entry]> = vec![entry.clone()].into();
    let sizes: crate::SharedSizes = Arc::new(Mutex::new(HashMap::from([(
        entry.path.clone(),
        FolderSize {
            bytes: 10,
            finished: true,
            ..Default::default()
        },
    )])));
    controller.snapshots.lock().unwrap()[0].visible = Some(crate::CommittedSnapshot {
        navigation: NavigationId(1),
        path: root,
        entries: entries.clone(),
        sizes: sizes.clone(),
    });
    let mut request = request(1, 1);
    request.capture = Arc::new(FileListCapture {
        directory: String::new(),
        source: FileListSource::Native {
            view: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
            selection: Arc::default(),
            fallback: Some(EntryId(1)),
        },
    });
    request.template = "$(FileSize)$(LF)".into();
    controller.start(request.clone(), &mut tasks).unwrap();
    let Event::FileList(event) = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!()
    };
    assert!(matches!(&event.result, Ok(FileListUpdate::Preview { text, .. }) if text == "10\n"));
    sizes.lock().unwrap().get_mut(&entry.path).unwrap().bytes = 99;
    controller.snapshots.lock().unwrap()[0] = Default::default();
    request.generation = 2;
    request.template = "$(FileName)|$(FileSize)$(LF)".into();
    controller.start(request, &mut tasks).unwrap();
    let Event::FileList(event) = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!()
    };
    assert!(
        matches!(&event.result, Ok(FileListUpdate::Preview { text, .. }) if text == "folder|10\n")
    );
    controller.cancel(DesktopRequestId(1), 2);
    while tasks.join_next().await.is_some() {}
}

#[tokio::test(flavor = "current_thread")]
async fn refused_preview_does_not_cancel_the_last_accepted_generation() {
    let (mut controller, mut events, mut tasks) = controller();
    controller.start(request(1, 1), &mut tasks).unwrap();
    controller.start(request(1, 2), &mut tasks).unwrap();
    // Neither spawned task can acquire a CPU turn before this synchronous admission.
    assert!(controller.start(request(1, 3), &mut tasks).is_err());
    assert!(!controller.jobs[&DesktopRequestId(1)].cancel.is_cancelled());
    while tasks.join_next().await.is_some() {}
    let Event::FileList(event) = tokio::time::timeout(Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(event.generation, 2);
    assert!(
        matches!(&event.result, Ok(FileListUpdate::Preview { text, .. }) if text == "文档.txt\n")
    );
    assert!(events.try_recv().is_err());
}

#[tokio::test(flavor = "current_thread")]
async fn output_cannot_be_superseded_and_close_suppresses_late_results() {
    let (mut controller, mut events, mut tasks) = controller();
    let mut output = request(1, 1);
    output.destination = FileListDestination::Clipboard;
    controller.start(output, &mut tasks).unwrap();
    assert!(controller.start(request(1, 2), &mut tasks).is_err());
    assert!(!controller.jobs[&DesktopRequestId(1)].cancel.is_cancelled());
    controller.cancel(DesktopRequestId(1), 1);
    while tasks.join_next().await.is_some() {}
    assert!(events.try_recv().is_err());
    assert!(controller.jobs.is_empty());
    assert_eq!(controller.budget.available_permits(), 2);
}

#[tokio::test]
async fn completed_sessions_are_released_and_do_not_exhaust_window_budget() {
    let (mut controller, mut events, mut tasks) = controller();
    for id in 1..=8 {
        controller.start(request(id, 1), &mut tasks).unwrap();
        let Event::FileList(event) = tokio::time::timeout(Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(event.id, DesktopRequestId(id));
        assert!(event.result.is_ok());
        controller.cancel(DesktopRequestId(id), 1);
        while tasks.join_next().await.is_some() {}
        // The token returns on the cleanup worker after actual destruction; a leak
        // would keep it (and eventually exhaust the six-token bound) forever.
        tokio::time::timeout(Duration::from_secs(5), async {
            while controller.retained.available_permits() != MAX_RETAINED {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("a completed session must release its retention token");
    }
    assert!(controller.jobs.is_empty());
    assert_eq!(controller.budget.available_permits(), 2);
}

#[tokio::test(flavor = "current_thread")]
async fn stale_generation_and_confirmation_are_rejected() {
    let (mut controller, _events, mut tasks) = controller();
    controller.start(request(1, 2), &mut tasks).unwrap();
    assert!(controller.start(request(1, 2), &mut tasks).is_err());
    assert!(controller.start(request(1, 1), &mut tasks).is_err());
    assert!(
        controller
            .confirm(DesktopRequestId(1), 1, &mut tasks)
            .is_err()
    );
    controller.cancel(DesktopRequestId(1), 1);
    assert!(controller.jobs.contains_key(&DesktopRequestId(1)));
    controller.cancel(DesktopRequestId(1), 2);
    assert!(
        controller
            .confirm(DesktopRequestId(1), 2, &mut tasks)
            .is_err()
    );
    while tasks.join_next().await.is_some() {}
}

async fn absorbed(gate: &Semaphore) {
    // A queued acquisition absorbs the free permits while it waits for the rest;
    // a nonqueuing try_acquire loop never would.
    tokio::time::timeout(Duration::from_secs(5), async {
        while gate.available_permits() != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the save must queue at the gate");
}

fn file_controller(
    gate: Arc<Semaphore>,
) -> (
    Controller,
    mpsc::Receiver<Event>,
    JoinSet<()>,
    tempfile::TempDir,
) {
    let parent = std::env::temp_dir().join("file-list-lifetime/_build/tmp");
    std::fs::create_dir_all(&parent).unwrap();
    let root = tempfile::tempdir_in(parent.canonicalize().unwrap()).unwrap();
    let (sender, events) = mpsc::channel(16);
    let controller = Controller::testing(
        Some(Arc::new(midnight_platform::LocalFileSystem::new(None))),
        gate,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        crate::test_snapshots(Default::default()),
    );
    (controller, events, JoinSet::new(), root)
}

fn file_request(root: &tempfile::TempDir) -> FileListRequest {
    let mut request = request(1, 1);
    request.destination = FileListDestination::File {
        path: root.path().join("list.txt"),
        append: false,
    };
    request
}

#[tokio::test]
async fn waiting_file_save_keeps_its_fifo_place_at_the_mutation_gate() {
    use crate::transfer_leases::NAMESPACE_WEIGHT;
    let gate = Arc::new(Semaphore::new(NAMESPACE_WEIGHT as usize));
    let held = gate.clone().try_acquire_owned().unwrap();
    let cancel = Cancellation::default();
    let stopped = Arc::new(Notify::new());
    let save = tokio::spawn({
        let gate = gate.clone();
        let (cancel, stopped) = (cancel.clone(), stopped.clone());
        async move {
            let acquire = gate.acquire_many_owned(NAMESPACE_WEIGHT);
            wait_unless_stopped(&cancel, &stopped, acquire).await
        }
    });
    absorbed(&gate).await;
    let later = tokio::spawn(gate.clone().acquire_many_owned(NAMESPACE_WEIGHT));
    tokio::task::yield_now().await; // The later waiter queues behind the save.
    drop(held);
    let permit = tokio::time::timeout(Duration::from_secs(5), save)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(permit.num_permits(), NAMESPACE_WEIGHT as usize);
    assert!(!later.is_finished(), "a later mutation overtook the save");
    drop(permit);
    drop(
        tokio::time::timeout(Duration::from_secs(5), later)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
    );
}

#[tokio::test(start_paused = true)]
async fn stopped_file_save_leaves_the_gate_queue_without_a_polling_timer() {
    let gate = Arc::new(Semaphore::new(0));
    let cancel = Cancellation::default();
    let stopped = Arc::new(Notify::new());
    let save = tokio::spawn({
        let gate = gate.clone();
        let (cancel, stopped) = (cancel.clone(), stopped.clone());
        async move { wait_unless_stopped(&cancel, &stopped, gate.acquire_owned()).await }
    });
    tokio::task::yield_now().await; // The save is queued and waiting.
    let started = tokio::time::Instant::now();
    // As `Job::stop`: cancel, then wake the registered wait.
    cancel.cancel();
    stopped.notify_waiters();
    let error = tokio::time::timeout(Duration::from_secs(5), save)
        .await
        .unwrap()
        .unwrap()
        .unwrap_err();
    assert!(error.message.contains("cancelled"), "{error:?}");
    // Paused time auto-advances only to a pending timer; none may be needed.
    assert_eq!(
        tokio::time::Instant::now(),
        started,
        "the wait polled a timer"
    );
    gate.add_permits(1);
    assert!(
        gate.try_acquire().is_ok(),
        "no abandoned waiter keeps permits"
    );
}

#[tokio::test]
async fn queued_file_save_holds_no_worker_and_publishes_once_the_gate_opens() {
    use crate::transfer_leases::NAMESPACE_WEIGHT;
    let gate = Arc::new(Semaphore::new(NAMESPACE_WEIGHT as usize));
    // A running Copy holds one namespace share; the save queues behind it.
    let copy = gate.clone().try_acquire_owned().unwrap();
    let (mut controller, mut events, mut tasks, root) = file_controller(gate.clone());
    controller.start(file_request(&root), &mut tasks).unwrap();
    absorbed(&gate).await;
    assert_eq!(
        controller.budget.available_permits(),
        MAX_WORKERS,
        "a save waiting for the gate must not occupy a File List worker"
    );
    assert!(events.try_recv().is_err());
    assert!(!root.path().join("list.txt").exists());
    drop(copy);
    let Event::FileList(event) = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("file-list result")
    };
    assert!(
        matches!(event.result, Ok(FileListUpdate::Saved { .. })),
        "{:?}",
        event.result
    );
    assert_eq!(
        std::fs::read(root.path().join("list.txt")).unwrap(),
        "文档.txt\n".as_bytes()
    );
    while tasks.join_next().await.is_some() {}
    assert_eq!(controller.budget.available_permits(), MAX_WORKERS);
    assert_eq!(gate.available_permits(), NAMESPACE_WEIGHT as usize);
}

/// The native adapters report raw times; the save confirmation formats only the
/// one existing output it displays.
#[tokio::test]
async fn file_list_save_conflict_formats_only_the_displayed_existing_file() {
    use crate::transfer_composition::tests::CountingLocale;
    let gate = crate::transfer_leases::namespace_gate();
    let (mut controller, mut events, mut tasks, root) = file_controller(gate);
    let locale = Arc::new(CountingLocale::default());
    controller.locale = Some(locale.clone());
    std::fs::write(root.path().join("list.txt"), b"existing").unwrap();
    controller.start(file_request(&root), &mut tasks).unwrap();
    let Event::FileList(event) = tokio::time::timeout(Duration::from_secs(10), events.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("file-list result")
    };
    let Ok(FileListUpdate::Conflict { entry, append }) = &event.result else {
        panic!("expected a save confirmation: {:?}", event.result)
    };
    assert!(!append);
    assert!(entry.item.metadata.modified.is_some(), "{entry:?}");
    assert_eq!(
        entry.item.metadata.localized_modified,
        Some(CountingLocale::text())
    );
    assert_eq!(locale.calls(), 1);
    controller.cancel(DesktopRequestId(1), 1);
    while tasks.join_next().await.is_some() {}
    assert_eq!(
        std::fs::read(root.path().join("list.txt")).unwrap(),
        b"existing"
    );
}

/// Counts panics on threads named `name`, then defers to the previous hook so
/// every other test still reports its panics as usual.
fn panics_on(name: &'static str) -> Arc<std::sync::atomic::AtomicUsize> {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let observed = count.clone();
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().name() == Some(name) {
            observed.fetch_add(1, Ordering::SeqCst);
        }
        previous(info);
    }));
    count
}

#[test]
fn runtime_teardown_drops_a_save_queued_at_the_gate_without_panicking() {
    use crate::transfer_leases::NAMESPACE_WEIGHT;
    const THREADS: &str = "file-list-teardown-test";
    let panics = panics_on(THREADS);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name(THREADS)
        .enable_all()
        .build()
        .unwrap();
    let gate = Arc::new(Semaphore::new(NAMESPACE_WEIGHT as usize));
    let copy = gate.clone().try_acquire_owned().unwrap();
    let (mut controller, events, mut tasks, root) = file_controller(gate.clone());
    runtime.block_on(async {
        controller.start(file_request(&root), &mut tasks).unwrap();
        absorbed(&gate).await;
    });
    // Neither cancelled nor aborted: an embedder drops the runtime while the save
    // waits (or the engine's own shutdown races the cancellation). Tokio's timers
    // are shut down then, so a wait that polls one would panic here.
    let (dropped, teardown) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        drop(runtime);
        let _ = dropped.send(());
    });
    teardown
        .recv_timeout(Duration::from_secs(10))
        .expect("runtime teardown must not wait for a queued save");
    assert_eq!(
        panics.load(Ordering::SeqCst),
        0,
        "a File List save panicked during runtime teardown"
    );
    assert!(!root.path().join("list.txt").exists());
    drop((copy, tasks, events, controller));
}
