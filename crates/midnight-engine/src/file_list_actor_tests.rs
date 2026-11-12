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
            Arc::new(Semaphore::new(1)),
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
