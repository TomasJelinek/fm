use super::*;
use crate::{CommittedSnapshot, SnapshotState};
use midnight_platform::LocalFileSystem;
use std::{
    fs,
    time::{Duration, UNIX_EPOCH},
};

struct Harness {
    directory: tempfile::TempDir,
    controller: Controller,
    events: mpsc::Receiver<Event>,
    tasks: JoinSet<()>,
    request: PrepareOperation,
}

impl Harness {
    fn new(names: &[(&str, bool)]) -> Self {
        Self::with_locale(names, None)
    }
    fn with_locale(names: &[(&str, bool)], locale: Option<Arc<dyn LocaleService>>) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let mut entries = Vec::new();
        for (index, (name, folder)) in names.iter().enumerate() {
            let path = directory.path().join(name);
            if *folder {
                fs::create_dir(&path).unwrap();
            } else {
                fs::write(&path, name.as_bytes()).unwrap();
            }
            entries.push(Entry::new(
                EntryId(index as u64),
                path,
                if *folder {
                    EntryKind::Directory
                } else {
                    EntryKind::File
                },
            ));
        }
        let entries: Arc<[Entry]> = entries.into();
        let view = Arc::new(DirectoryIndex::from_entries(&entries));
        let source = VisibleDirectory {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            path: directory.path().to_owned(),
            entries: entries.clone(),
        };
        let snapshots = crate::test_snapshots([
            SnapshotState {
                archive: None,
                network: None,
                network_navigation: None,
                archive_navigation: None,
                visible: Some(CommittedSnapshot {
                    navigation: source.navigation,
                    path: source.path.clone(),
                    entries,
                    sizes: Default::default(),
                }),
                ..Default::default()
            },
            SnapshotState::default(),
        ]);
        let (sender, events) = mpsc::channel(64);
        let mut tasks = JoinSet::new();
        let controller = Controller::new(
            crate::worker_retirement::ActorRetirement::testing(),
            Services {
                mutation: Some(Arc::new(LocalFileSystem::new(None))),
                locale,
            },
            snapshots,
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            Arc::new(Semaphore::new(1)),
            CopyFence::default(),
            &mut tasks,
        );
        let mut bits = vec![u64::MAX; names.len().div_ceil(64)];
        if !names.len().is_multiple_of(64) {
            *bits.last_mut().unwrap() = (1u64 << (names.len() % 64)) - 1;
        }
        let selection = Arc::new(SelectionSnapshot::from_bits(view.clone(), bits).unwrap());
        let request = PrepareOperation {
            id: OperationId(1),
            kind: MutationKind::EditMetadata,
            source,
            view,
            selection,
            focused: None,
        };
        Self {
            directory,
            controller,
            events,
            tasks,
            request,
        }
    }
    async fn preview(&mut self, revision: u64) -> Arc<MetadataPreview> {
        loop {
            match tokio::time::timeout(Duration::from_secs(4), self.events.recv())
                .await
                .unwrap()
                .unwrap()
            {
                Event::Operation(OperationEvent::MetadataPreview(preview))
                    if preview.revision == revision =>
                {
                    return preview;
                }
                Event::Operation(OperationEvent::Progress(progress))
                    if progress.state.is_terminal() =>
                {
                    panic!("unexpected failure: {progress:?}")
                }
                _ => {}
            }
        }
    }
    async fn result(&mut self) -> (Arc<MetadataResult>, Arc<OperationProgress>) {
        let mut result = None;
        loop {
            match tokio::time::timeout(Duration::from_secs(4), self.events.recv())
                .await
                .unwrap()
                .unwrap()
            {
                Event::Operation(OperationEvent::MetadataResult(value)) => result = Some(value),
                Event::Operation(OperationEvent::Progress(progress))
                    if progress.state.is_terminal() =>
                {
                    return (result.unwrap(), progress);
                }
                _ => {}
            }
        }
    }
    async fn conflict(&mut self) -> Arc<MetadataConflict> {
        loop {
            match tokio::time::timeout(Duration::from_secs(4), self.events.recv())
                .await
                .unwrap()
                .unwrap()
            {
                Event::Operation(OperationEvent::MetadataConflict(conflict)) => return conflict,
                Event::Operation(OperationEvent::Progress(progress))
                    if progress.state.is_terminal() =>
                {
                    panic!("unexpected terminal state: {progress:?}")
                }
                _ => {}
            }
        }
    }
}
impl Drop for Harness {
    fn drop(&mut self) {
        self.controller.shutdown();
        self.tasks.abort_all();
    }
}

#[tokio::test]
async fn metadata_preview_lists_every_bounded_root_and_refreshes_native_times() {
    let names = (0..MAX_METADATA_TARGETS)
        .map(|index| format!("ŽLUŤOUČKÝ-{index:03}.TXT"))
        .collect::<Vec<_>>();
    let entries = names
        .iter()
        .map(|name| (name.as_str(), false))
        .collect::<Vec<_>>();
    let mut h = Harness::new(&entries);
    h.controller.prepare(h.request.clone()).await;
    let initial = h.preview(0).await;
    assert_eq!(initial.total, MAX_METADATA_TARGETS as u64);
    assert_eq!(
        initial.sample.len(),
        MAX_METADATA_TARGETS,
        "the list must contain all captured roots"
    );
    let time = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    fs::OpenOptions::new()
        .write(true)
        .open(h.directory.path().join(&names[0]))
        .unwrap()
        .set_modified(time)
        .unwrap();
    h.controller
        .preview_metadata(
            h.request.id,
            1,
            MetadataPatch {
                case: Some(CaseChange::default()),
                windows_mask: initial.capabilities.windows_attributes & EDIT_READ_ONLY,
                windows_value: initial.capabilities.windows_attributes & EDIT_READ_ONLY,
                unix_mask: initial.capabilities.unix_permissions & 0o222,
                modified: TimeEdit::Set(time + Duration::from_secs(3600)),
                ..Default::default()
            },
        )
        .await;
    let refreshed = h.preview(1).await;
    assert_eq!(
        refreshed.sample[0].current.modified,
        Some(time),
        "refresh reads pinned native metadata again"
    );
    assert_eq!(
        refreshed.sample[0].proposed_name.as_deref(),
        Some(std::ffi::OsStr::new("žluťoučký-000.txt"))
    );
    assert_eq!(refreshed.sample.len(), MAX_METADATA_TARGETS);
    assert_eq!(
        refreshed.sample[0].proposed.modified,
        Some(time + Duration::from_secs(3600))
    );
    if let Some(attributes) = refreshed.sample[0].proposed.windows_attributes {
        assert_eq!(attributes & EDIT_READ_ONLY, EDIT_READ_ONLY);
    }
    if let Some(mode) = refreshed.sample[0].proposed.unix_mode {
        assert_eq!(mode & 0o222, 0);
    }
    assert_eq!(
        fs::metadata(h.directory.path().join(&names[0]))
            .unwrap()
            .modified()
            .unwrap(),
        time,
        "proposed metadata is a preview and must not mutate the file"
    );
}

#[tokio::test]
async fn metadata_preview_refreshes_without_requesting_a_mutation() {
    let mut h = Harness::new(&[("FILE", false)]);
    h.controller.prepare(h.request.clone()).await;
    h.preview(0).await;
    let time = UNIX_EPOCH + Duration::from_secs(1_500_000_000);
    fs::OpenOptions::new()
        .write(true)
        .open(h.directory.path().join("FILE"))
        .unwrap()
        .set_modified(time)
        .unwrap();
    h.controller
        .preview_metadata(h.request.id, 1, MetadataPatch::default())
        .await;
    let refreshed = h.preview(1).await;
    assert_eq!(refreshed.sample[0].current.modified, Some(time));
    assert!(refreshed.patch.is_empty());
}

#[tokio::test]
async fn metadata_refresh_rejects_replacement_of_the_captured_native_target() {
    let mut h = Harness::new(&[("SOURCE", false)]);
    h.controller.prepare(h.request.clone()).await;
    h.preview(0).await;
    fs::rename(
        h.directory.path().join("SOURCE"),
        h.directory.path().join("HELD"),
    )
    .unwrap();
    fs::write(h.directory.path().join("SOURCE"), b"replacement").unwrap();
    h.controller
        .preview_metadata(h.request.id, 1, MetadataPatch::default())
        .await;
    let refreshed = h.preview(1).await;
    assert_eq!(refreshed.unsupported, 1);
    assert!(refreshed.sample[0].issue.is_some());
    assert!(
        refreshed.sample[0].current.modified.is_none(),
        "failed inspection must not show cached values as current"
    );
    assert_eq!(
        fs::read(h.directory.path().join("SOURCE")).unwrap(),
        b"replacement"
    );
}

#[tokio::test]
async fn metadata_revision_and_selected_roots_control_actual_writes() {
    let mut h = Harness::new(&[("FIRST", false), ("SECOND", false)]);
    h.controller.prepare(h.request.clone()).await;
    assert_eq!(h.preview(0).await.total, 2);
    let patch = MetadataPatch {
        case: Some(CaseChange::default()),
        modified: TimeEdit::Now,
        accessed: TimeEdit::Now,
        ..Default::default()
    };
    h.controller.preview_metadata(h.request.id, 1, patch).await;
    let preview = h.preview(1).await;
    assert_eq!(preview.patch.modified, preview.patch.accessed);
    assert!(matches!(preview.patch.modified, TimeEdit::Set(_)));
    h.controller
        .confirm_metadata(h.request.id, 0, MetadataConflictPolicy::Skip)
        .await;
    // A later invalid revision must not discard the captured native targets.
    h.controller
        .preview_metadata(
            h.request.id,
            2,
            MetadataPatch {
                unix_mask: 0xffff,
                ..Default::default()
            },
        )
        .await;
    assert_eq!(h.preview(2).await.unsupported, 2);
    assert!(h.directory.path().join("FIRST").exists());
    h.controller
        .preview_metadata(h.request.id, 3, preview.patch.clone())
        .await;
    h.preview(3).await;
    h.controller
        .confirm_metadata(h.request.id, 3, MetadataConflictPolicy::Skip)
        .await;
    let (result, progress) = h.result().await;
    assert_eq!(progress.state, OperationState::Completed);
    assert_eq!((result.processed, result.changed, result.failed), (2, 2, 0));
    assert_eq!(
        fs::read(h.directory.path().join("first")).unwrap(),
        b"FIRST"
    );
    assert_eq!(
        fs::read(h.directory.path().join("second")).unwrap(),
        b"SECOND"
    );
}

#[tokio::test]
async fn recursive_case_and_times_are_postorder() {
    let mut h = Harness::new(&[("PARENT", true)]);
    fs::create_dir(h.directory.path().join("PARENT/CHILD")).unwrap();
    fs::write(h.directory.path().join("PARENT/CHILD/FILE.TXT"), b"payload").unwrap();
    h.controller.prepare(h.request.clone()).await;
    h.preview(0).await;
    let time = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    h.controller
        .preview_metadata(
            h.request.id,
            1,
            MetadataPatch {
                case: Some(CaseChange::default()),
                recursive: true,
                modified: TimeEdit::Set(time),
                ..Default::default()
            },
        )
        .await;
    h.preview(1).await;
    h.controller
        .confirm_metadata(h.request.id, 1, MetadataConflictPolicy::SkipAll)
        .await;
    let (result, progress) = h.result().await;
    assert_eq!(progress.state, OperationState::Completed, "{progress:?}");
    assert_eq!((result.processed, result.changed), (3, 3));
    assert_eq!(
        fs::read(h.directory.path().join("parent/child/file.txt")).unwrap(),
        b"payload"
    );
    assert_eq!(
        fs::metadata(h.directory.path().join("parent"))
            .unwrap()
            .modified()
            .unwrap(),
        time
    );
    assert_eq!(
        fs::metadata(h.directory.path().join("parent/child"))
            .unwrap()
            .modified()
            .unwrap(),
        time
    );
}

#[tokio::test]
async fn cancelled_metadata_worker_releases_full_output_queue() {
    let h = Harness::new(&[("SOURCE", false)]);
    let cancel = Cancellation::default();
    let status = Arc::new(Mutex::new(worker::initial(&h.request)));
    let mut job = worker::prepare(
        h.request.clone(),
        Some(Arc::new(LocalFileSystem::new(None))),
        cancel.clone(),
        status,
        Arc::new(Semaphore::new(MAX_METADATA_TARGETS)),
    )
    .unwrap();
    job.metadata.as_mut().unwrap().update(
        1,
        MetadataPatch {
            modified: TimeEdit::Now,
            ..Default::default()
        },
        &job.targets,
        &cancel,
    );
    let (sender, receiver) = mpsc::channel(1);
    sender
        .try_send(Event::Operation(OperationEvent::Progress(Arc::new(
            job.progress(),
        ))))
        .unwrap();
    let mut work = tokio::task::spawn_blocking(move || {
        job.execute(Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        })
    });
    tokio::time::sleep(Duration::from_millis(40)).await;
    cancel.cancel();
    let completed = tokio::time::timeout(Duration::from_millis(300), &mut work).await;
    let released = completed.is_ok();
    // Even the RED path must not leak a blocked native worker into runtime shutdown.
    drop(receiver);
    if !released {
        work.await.unwrap();
    }
    assert!(
        released,
        "cancel must release the native worker despite output backpressure"
    );
}

#[test]
fn prepared_jobs_share_handle_budget_and_release_it_after_cleanup() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let _runtime = runtime.enter();
    let h = Harness::new(&[("FIRST", false), ("SECOND", false)]);
    let budget = Arc::new(Semaphore::new(2));
    let prepare = || {
        worker::prepare(
            h.request.clone(),
            Some(Arc::new(LocalFileSystem::new(None))),
            Cancellation::default(),
            Arc::new(Mutex::new(worker::initial(&h.request))),
            budget.clone(),
        )
    };
    let job = prepare().unwrap();
    assert_eq!(
        budget.available_permits(),
        0,
        "pins across different jobs consume one shared budget"
    );
    assert!(prepare().is_err());
    assert_eq!(budget.available_permits(), 0);
    drop(job);
    assert_eq!(budget.available_permits(), 2);
    assert!(prepare().is_ok());
}

#[tokio::test]
async fn recursive_budget_is_checked_before_any_child_is_renamed() {
    let h = Harness::new(&[("PARENT", true)]);
    for name in ["FIRST", "SECOND"] {
        fs::write(h.directory.path().join("PARENT").join(name), b"unchanged").unwrap();
    }
    let cancel = Cancellation::default();
    let mut job = worker::prepare(
        h.request.clone(),
        Some(Arc::new(LocalFileSystem::new(None))),
        cancel.clone(),
        Arc::new(Mutex::new(worker::initial(&h.request))),
        Arc::new(Semaphore::new(2)),
    )
    .unwrap();
    job.metadata.as_mut().unwrap().update(
        1,
        MetadataPatch {
            recursive: true,
            case: Some(CaseChange::default()),
            ..Default::default()
        },
        &job.targets,
        &cancel,
    );
    let (sender, _events) = mpsc::channel(64);
    let progress = tokio::task::spawn_blocking(move || {
        job.execute(Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        })
    })
    .await
    .unwrap();
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!(
        progress.processed_entries, 0,
        "a directory batch exceeding the shared budget must fail before writes"
    );
    assert_eq!(
        fs::read(h.directory.path().join("PARENT/FIRST")).unwrap(),
        b"unchanged"
    );
    assert_eq!(
        fs::read(h.directory.path().join("PARENT/SECOND")).unwrap(),
        b"unchanged"
    );
}

#[tokio::test]
async fn conflict_tokens_skip_one_then_skip_all_without_overwriting() {
    let mut h = Harness::new(&[("straße", false), ("maße", false), ("große", false)]);
    h.controller.prepare(h.request.clone()).await;
    h.preview(0).await;
    h.controller
        .preview_metadata(
            h.request.id,
            1,
            MetadataPatch {
                case: Some(CaseChange {
                    mode: CaseMode::Upper,
                    scope: CaseScope::Whole,
                }),
                ..Default::default()
            },
        )
        .await;
    h.preview(1).await;
    // Full Unicode expansions have distinct native names even on case-insensitive NTFS.
    for name in ["STRASSE", "MASSE", "GROSSE"] {
        fs::write(h.directory.path().join(name), b"existing").unwrap();
    }
    h.controller
        .confirm_metadata(h.request.id, 1, MetadataConflictPolicy::Skip)
        .await;
    let first = h.conflict().await;
    h.controller
        .resolve_metadata_conflict(
            h.request.id,
            first.token + 99,
            MetadataConflictPolicy::SkipAll,
        )
        .await;
    h.controller
        .resolve_metadata_conflict(h.request.id, first.token, MetadataConflictPolicy::Skip)
        .await;
    let second = h.conflict().await;
    assert_ne!(first.token, second.token);
    h.controller
        .resolve_metadata_conflict(h.request.id, first.token, MetadataConflictPolicy::Cancel)
        .await;
    h.controller
        .resolve_metadata_conflict(h.request.id, second.token, MetadataConflictPolicy::SkipAll)
        .await;
    let (result, progress) = h.result().await;
    assert_eq!(progress.state, OperationState::CompletedWithErrors);
    assert_eq!(
        (
            result.processed,
            result.changed,
            result.skipped,
            result.failed
        ),
        (3, 0, 3, 0)
    );
    for name in ["STRASSE", "MASSE", "GROSSE"] {
        assert_eq!(
            fs::read(h.directory.path().join(name)).unwrap(),
            b"existing"
        );
    }
    for name in ["straße", "maße", "große"] {
        assert_eq!(
            fs::read(h.directory.path().join(name)).unwrap(),
            name.as_bytes()
        );
    }
}

#[tokio::test]
async fn cancellation_at_conflict_still_delivers_completed_item_outcomes() {
    let mut h = Harness::new(&[("first", false), ("straße", false)]);
    h.controller.prepare(h.request.clone()).await;
    h.preview(0).await;
    h.controller
        .preview_metadata(
            h.request.id,
            1,
            MetadataPatch {
                case: Some(CaseChange {
                    mode: CaseMode::Upper,
                    scope: CaseScope::Whole,
                }),
                ..Default::default()
            },
        )
        .await;
    h.preview(1).await;
    fs::write(h.directory.path().join("STRASSE"), b"existing").unwrap();
    h.controller
        .confirm_metadata(h.request.id, 1, MetadataConflictPolicy::Skip)
        .await;
    h.conflict().await;
    h.controller.cancel(h.request.id);
    let (result, progress) = h.result().await;
    assert_eq!(progress.state, OperationState::Cancelled);
    assert_eq!(result.changed, 1);
    assert!(
        result
            .sample
            .iter()
            .any(|item| item.current_path == h.directory.path().join("FIRST")
                && item.changes.renamed)
    );
    assert_eq!(
        fs::read(h.directory.path().join("FIRST")).unwrap(),
        b"first"
    );
    assert_eq!(
        fs::read(h.directory.path().join("STRASSE")).unwrap(),
        b"existing"
    );
}

#[tokio::test(flavor = "current_thread")]
async fn metadata_preview_captures_native_locale_on_worker_for_current_and_proposed_times() {
    struct Locale(std::thread::ThreadId);
    impl LocaleService for Locale {
        fn number_locale(&self) -> Result<NumberLocale, FsError> {
            Ok(NumberLocale::default())
        }
        fn datetime_input_format(&self) -> Option<DateTimeInputFormat> {
            assert_ne!(
                std::thread::current().id(),
                self.0,
                "locale discovery must use a worker"
            );
            Some(DateTimeInputFormat {
                pattern: "%d. %m. %Y %H:%M:%S".into(),
                am_pm: ["".into(), "".into()],
            })
        }
        fn format_datetime(&self, _: std::time::SystemTime) -> Option<LocalizedDateTime> {
            assert_ne!(
                std::thread::current().id(),
                self.0,
                "native formatting must use a worker"
            );
            Some(LocalizedDateTime {
                date: "native date".into(),
                time: "native time".into(),
            })
        }
    }
    let mut harness = Harness::with_locale(
        &[("žluťoučký.txt", false)],
        Some(Arc::new(Locale(std::thread::current().id()))),
    );
    harness.controller.prepare(harness.request.clone()).await;
    let initial = harness.preview(0).await;
    assert_eq!(
        initial.time_input_format.as_ref().unwrap().pattern,
        "%d. %m. %Y %H:%M:%S"
    );
    let current = initial.sample[0].current.modified.unwrap();
    assert_eq!(initial.localized_times[&current].date, "native date");
    let proposed = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    harness
        .controller
        .preview_metadata(
            initial.id,
            1,
            MetadataPatch {
                modified: TimeEdit::Set(proposed),
                ..Default::default()
            },
        )
        .await;
    let reviewed = harness.preview(1).await;
    assert_eq!(reviewed.localized_times[&proposed].time, "native time");
    assert_eq!(reviewed.sample[0].proposed.modified, Some(proposed));
    assert_eq!(reviewed.id, initial.id);
}

#[path = "tree_metadata_operation_tests.rs"]
mod tree_metadata;
