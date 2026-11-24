use super::*;
use midnight_platform::LocalFileSystem;

fn request(root: &std::path::Path, id: u64) -> Arc<FindFeedSizeRequest> {
    Arc::new(FindFeedSizeRequest {
        pane: PaneId::Left,
        id: FolderSizeScanId(id),
        rows_serial: 7,
        entries: Arc::from([Entry::new(
            EntryId(3),
            root.to_owned(),
            EntryKind::Directory,
        )]),
        selected: Arc::new(vec![false]),
        cancel: Cancellation::default(),
    })
}

#[test]
fn captured_roots_use_the_existing_scanner_and_preserve_tags_and_completion() {
    let temporary = tempfile::tempdir().unwrap();
    std::fs::write(temporary.path().join("data"), b"seven77").unwrap();
    let request = request(temporary.path(), 4);
    let mut events = Vec::new();
    scan(
        &request,
        &LocalFileSystem::new(None),
        &Arc::new(folder_sizes::IoBudget::new()),
        &ReportArena::testing(),
        &mut |event| {
            events.push(event);
            true
        },
    );
    assert!(events.iter().all(|event| event.pane == PaneId::Left
        && event.id == FolderSizeScanId(4)
        && event.rows_serial == 7));
    assert!(events.iter().any(|event| matches!(&event.update,
        FindFeedSizeUpdate::Updated {entry: EntryId(3), size} if size.finished && size.bytes == 7 && size.files == 1)));
    assert!(matches!(
        &events.last().unwrap().update,
        FindFeedSizeUpdate::Finished {
            cancelled: false,
            error: None
        }
    ));
}

#[tokio::test]
async fn replacements_coalesce_and_cancel_without_spawning_a_task_per_root() {
    let (sender, _receiver) = tokio::sync::mpsc::channel(8);
    let budget = Arc::new(Semaphore::new(0));
    let mut controller = Controller::new(
        Arc::new(LocalFileSystem::new(None)),
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        budget.clone(),
        Cancellation::default(),
        Arc::new(folder_sizes::IoBudget::new()),
        TaskLifetime::testing(()),
        ReportArena::testing(),
    );
    let mut tasks = JoinSet::new();
    let root = std::env::temp_dir();
    let first = request(&root, 1);
    let second = request(&root, 2);
    let latest = request(&root, 3);
    controller.compute(first.clone(), &mut tasks);
    controller.compute(second.clone(), &mut tasks);
    controller.compute(latest.clone(), &mut tasks);
    assert!(first.cancel.is_cancelled());
    assert!(second.cancel.is_cancelled());
    assert!(!latest.cancel.is_cancelled());
    assert_eq!(tasks.len(), 1);
    assert!(controller.busy());
    controller.cancel_all();
    assert!(latest.cancel.is_cancelled());
    budget.close();
    while tasks.join_next().await.is_some() {}
    controller.tick(&mut tasks);
    assert!(!controller.busy());
}

#[test]
fn cancelled_capture_publishes_no_values() {
    let request = request(&std::env::temp_dir(), 1);
    request.cancel.cancel();
    let mut events = Vec::new();
    scan(
        &request,
        &LocalFileSystem::new(None),
        &Arc::new(folder_sizes::IoBudget::new()),
        &ReportArena::testing(),
        &mut |event| {
            events.push(event);
            true
        },
    );
    assert_eq!(events.len(), 1);
    assert!(matches!(
        &events[0].update,
        FindFeedSizeUpdate::Finished {
            cancelled: true,
            error: None
        }
    ));
}

#[derive(Default)]
struct BlockedScan {
    entered: tokio::sync::Notify,
    released: (std::sync::Mutex<bool>, std::sync::Condvar),
}
impl BlockedScan {
    fn release(&self) {
        *self.released.0.lock().unwrap() = true;
        self.released.1.notify_all();
    }
}
struct ReleaseBlocked(Arc<BlockedScan>);
impl Drop for ReleaseBlocked {
    fn drop(&mut self) {
        self.0.release();
    }
}
impl DirectoryService for BlockedScan {
    fn read_directory(
        &self,
        path: &std::path::Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        if path.ends_with("first") {
            self.entered.notify_one();
            let mut released = self.released.0.lock().unwrap();
            while !*released {
                released = self.released.1.wait(released).unwrap();
            }
        } else {
            assert!(
                path.ends_with("second"),
                "only captured roots may be scanned"
            );
            let mut entry = Entry::new(EntryId(9), path.join("data"), EntryKind::File);
            entry.metadata.size = Some(7);
            emit(entry);
        }
        Ok(())
    }
    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        true
    }
    fn entry_metadata(&self, _: &std::path::Path) -> Result<EntryMetadata, FsError> {
        Ok(EntryMetadata {
            size: Some(7),
            ..Default::default()
        })
    }
}
impl VolumeService for BlockedScan {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(Vec::new())
    }
}
impl StorageService for BlockedScan {
    fn storage_kind(&self, _: &std::path::Path) -> Result<StorageKind, FsError> {
        Ok(StorageKind::Unknown)
    }
    fn space_info(&self, _: &std::path::Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            available: 1,
            total: 1,
        })
    }
}

#[tokio::test]
async fn actual_engine_starts_a_queued_feed_scan_after_release_without_further_commands() {
    use std::time::Duration;
    let filesystem = Arc::new(BlockedScan::default());
    let _release = ReleaseBlocked(filesystem.clone());
    let (sender, mut events) = tokio::sync::mpsc::channel(32);
    let mut engine = crate::Engine::new(
        crate::Engine::lifetime().unwrap(),
        filesystem.clone(),
        sender,
    );
    let (commands, receiver) = engine.command_channel(8).unwrap();
    let engine = tokio::spawn(engine.run(receiver));
    let root = std::env::temp_dir();
    let entries: Arc<[Entry]> = Arc::from([
        Entry::new(EntryId(1), root.join("first"), EntryKind::Directory),
        Entry::new(EntryId(2), root.join("second"), EntryKind::Directory),
    ]);
    let make = |id, marks| {
        Arc::new(FindFeedSizeRequest {
            pane: PaneId::Left,
            id: FolderSizeScanId(id),
            rows_serial: 7,
            entries: entries.clone(),
            selected: Arc::new(marks),
            cancel: Cancellation::default(),
        })
    };
    let first = make(1, vec![true, false]);
    commands
        .send(Command::ComputeFindFeedSizes(first.clone()))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), filesystem.entered.notified())
        .await
        .unwrap();
    commands
        .send(Command::ComputeFindFeedSizes(make(2, vec![false, true])))
        .await
        .unwrap();
    // A synchronous rejection is an actor round-trip: both scans have been accepted.
    commands
        .send(Command::StartFind(Box::new(FindRequest::new(
            FindSessionId(999),
            FindGeneration(1),
            Vec::new(),
        ))))
        .await
        .unwrap();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(5), events.recv())
            .await
            .unwrap()
            .unwrap();
        if matches!(event, Event::FindRejected(rejection) if rejection.session == FindSessionId(999))
        {
            break;
        }
    }
    assert!(first.cancel.is_cancelled());
    // Let already-ready task completions drain before the blocking scan is released.
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    filesystem.release();
    let observed = tokio::time::timeout(Duration::from_secs(5), async {
        let mut measured = false;
        loop {
            if let Some(Event::FindFeedSize(event)) = events.recv().await
                && event.id == FolderSizeScanId(2)
            {
                match event.update {
                    FindFeedSizeUpdate::Updated {
                        entry: EntryId(2),
                        size,
                    } if size.finished => measured = size.bytes == 7,
                    FindFeedSizeUpdate::Finished {
                        cancelled: false,
                        error: None,
                    } => return measured,
                    _ => {}
                }
            }
        }
    })
    .await;
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
    assert!(observed.expect("queued scan must wake the actor after its predecessor leaves"));
}
