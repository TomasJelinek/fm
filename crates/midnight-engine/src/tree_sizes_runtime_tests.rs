//! Real bounded scan lifecycle, including blocked native calls and ACK backpressure.
use super::*;
use crate::tree_sizes::State;
use midnight_core::*;
use std::{
    path::Path,
    sync::mpsc,
    time::{Duration, Instant},
};
use tokio::sync::Notify;

struct Files {
    calls: AtomicU64,
    entered: mpsc::SyncSender<()>,
    release: Mutex<mpsc::Receiver<()>>,
    blocked: std::sync::atomic::AtomicBool,
    partial_root: bool,
    fail_metadata: bool,
}
impl DirectoryService for Files {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.partial_root && path.file_name() == Some(std::ffi::OsStr::new("z-parent")) {
            emit(Entry::new(EntryId(55), path.join("leaf"), EntryKind::File));
            if self.fail_metadata {
                emit(Entry::new(
                    EntryId(56),
                    path.join("unreadable"),
                    EntryKind::File,
                ));
            }
            emit(Entry::new(
                EntryId(57),
                path.join("link"),
                EntryKind::DirectoryLink,
            ));
            // Cross the real scan progress interval before entering the blocked child.
            std::thread::sleep(Duration::from_millis(300));
            emit(Entry::new(
                EntryId(58),
                path.join("pending"),
                EntryKind::Directory,
            ));
            return Ok(());
        }
        if self.blocked.load(Ordering::SeqCst)
            && (!self.partial_root || path.file_name() == Some(std::ffi::OsStr::new("pending")))
        {
            self.entered.send(()).unwrap();
            self.release
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(3))
                .unwrap();
        }
        emit(Entry::new(EntryId(55), path.join("leaf"), EntryKind::File));
        Ok(())
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        if self.fail_metadata && path.file_name() == Some(std::ffi::OsStr::new("unreadable")) {
            return Err(FsError::from_io(
                "metadata",
                path,
                std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            ));
        }
        Ok(EntryMetadata {
            size: Some(17),
            ..Default::default()
        })
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(vec![])
    }
}
impl StorageService for Files {
    fn storage_kind(&self, _: &Path) -> Result<StorageKind, FsError> {
        Ok(StorageKind::LocalSsd)
    }
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            total: 100,
            available: 50,
        })
    }
}

fn filesystem(blocked: bool) -> (Arc<Files>, mpsc::Receiver<()>, mpsc::SyncSender<()>) {
    let (entered, enter) = mpsc::sync_channel(8);
    let (release, receiver) = mpsc::sync_channel(8);
    (
        Arc::new(Files {
            calls: AtomicU64::new(0),
            entered,
            release: Mutex::new(receiver),
            blocked: std::sync::atomic::AtomicBool::new(blocked),
            partial_root: false,
            fail_metadata: false,
        }),
        enter,
        release,
    )
}

fn warning_text(size: &FolderSize) -> String {
    let report = size
        .warnings
        .as_ref()
        .expect("actual size error retains details");
    let source = report.snapshot(&Cancellation::default()).unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let count = source.read_at(bytes.len() as u64, &mut chunk).unwrap();
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&chunk[..count]);
    }
    String::from_utf8(bytes).unwrap()
}
fn capture(session: &mut Session, request: u64, row: TreeRowId) -> CapturedSelection {
    let p = session.publication().unwrap();
    session.commit(&p.view);
    session
        .capture_sizes(
            &TreeSizeRequest {
                view: p.view,
                request: TreeRequestId(request),
                selection_revision: p.selection_revision,
                selection: p.selection,
                scope: TreeSizeScope::Single(row),
            },
            &Cancellation::default(),
        )
        .unwrap()
}
async fn until(
    state: &mut State,
    stamp: TreeViewStamp,
    mut accept: impl FnMut(&[TreeSizeEvent]) -> bool,
) -> Vec<TreeSizeEvent> {
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let events = state.tick(Some(stamp));
            if accept(&events) {
                break events;
            }
            // A consumer still acknowledges maps while waiting for another
            // event; otherwise a slow worker can leave the publication slot held.
            acknowledge(state, &events);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("scan event deadline")
}
fn acknowledge(state: &mut State, events: &[TreeSizeEvent]) {
    for event in events {
        if let TreeSizeEvent::Snapshot { sizes } = event {
            state.acknowledge(sizes.stamp, sizes.version);
        }
    }
}

async fn cancelled_partial_scan(fail_metadata: bool) {
    let mut f = tests::fixture();
    let publication = f.session.publication().unwrap();
    assert!(f.session.commit(&publication.view));
    let complete_row = publication.view.rows[0].id;
    let partial_row = publication.view.rows[1].id;
    let stamp = publication.view.stamp;
    let captured = f
        .session
        .capture_sizes(
            &TreeSizeRequest {
                view: publication.view.clone(),
                request: TreeRequestId(1),
                selection_revision: publication.selection_revision,
                selection: publication.selection.clone(),
                scope: TreeSizeScope::SelectionOrVisible,
            },
            &Cancellation::default(),
        )
        .unwrap();
    let (mut files, entered, release) = filesystem(true);
    let fixture = Arc::get_mut(&mut files).unwrap();
    fixture.partial_root = true;
    fixture.fail_metadata = fail_metadata;
    let mut state = State::new(files, Arc::new(Notify::new()));
    state.enqueue(TreeRequestId(1), captured, &Cancellation::default());
    let first = state.tick(Some(stamp));
    acknowledge(&mut state, &first);
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    let before = until(&mut state, stamp, |events| {
        events.iter().any(|event| {
            matches!(event, TreeSizeEvent::Snapshot { sizes }
            if sizes.get(complete_row).is_some_and(|size| size.finished)
                && sizes.get(partial_row).is_some_and(|size| size.bytes == 17))
        })
    })
    .await;
    let before_size = before
        .iter()
        .find_map(|event| match event {
            TreeSizeEvent::Snapshot { sizes } => sizes.get(partial_row).cloned(),
            _ => None,
        })
        .unwrap();
    acknowledge(&mut state, &before);
    assert!(state.cancel(stamp, TreeRequestId(1)).is_empty());
    release.send(()).unwrap();
    let finished = until(&mut state, stamp, |events| {
        events.iter().any(|event| {
            matches!(
                event,
                TreeSizeEvent::Finished {
                    request: TreeRequestId(1),
                    cancelled: true,
                    ..
                }
            )
        })
    })
    .await;
    let final_events = if finished
        .iter()
        .any(|event| matches!(event, TreeSizeEvent::Snapshot { .. }))
    {
        finished
    } else {
        until(&mut state, stamp, |events| {
            events
                .iter()
                .any(|event| matches!(event, TreeSizeEvent::Snapshot { .. }))
        })
        .await
    };
    let sizes = final_events
        .iter()
        .find_map(|event| match event {
            TreeSizeEvent::Snapshot { sizes } => Some(sizes),
            _ => None,
        })
        .unwrap();
    let partial = sizes.get(partial_row).unwrap();
    assert_eq!(
        partial.errors,
        u64::from(fail_metadata),
        "cancellation is not a read error"
    );
    assert!(
        !partial.finished,
        "a cancelled estimate is not a completed total"
    );
    assert_eq!(partial.bytes, 17);
    assert_eq!(partial.files, 1);
    assert_eq!(partial.skipped_links, 1);
    if fail_metadata {
        let text = warning_text(partial);
        assert!(text.contains("unreadable"));
        assert!(text.contains("PermissionDenied"));
        assert!(text.contains("Scan cancelled"));
        assert_eq!(text.matches("Reason:").count(), 1);
        assert!(
            warning_text(&before_size).contains("Scan running"),
            "earlier publication stays immutable"
        );
    } else {
        assert!(
            partial.warnings.is_none(),
            "cancellation alone has no warning report"
        );
    }
    assert_eq!(
        sizes.get(complete_row),
        Some(&FolderSize {
            bytes: 17,
            files: 1,
            finished: true,
            ..Default::default()
        })
    );
    assert!(!sizes.running(partial_row));
    assert!(!sizes.running(complete_row));
    f.session
        .select(&TreeSelectionRequest {
            view: publication.view,
            revision: SelectionRevision(1),
            seed: None,
            seed_revision: SelectionRevision(0),
            action: TreeSelectionAction::Toggle(partial_row),
        })
        .unwrap();
    let TreeSizeEvent::Selected { summary, .. } = state.selected(&f.session.publication().unwrap())
    else {
        unreachable!()
    };
    assert_eq!(summary.complete, 0);
    assert_eq!(summary.complete_bytes, Some(0));
    assert_eq!(summary.partial, 1);
    assert_eq!(summary.partial_bytes, Some(17));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_keeps_unfinished_estimates_without_fabricated_errors() {
    cancelled_partial_scan(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancellation_preserves_real_read_errors_without_adding_another() {
    cancelled_partial_scan(true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn final_snapshot_waits_for_ack_then_timer_without_more_input() {
    let mut f = tests::fixture();
    let row = f.session.projection().rows[0].id;
    let stamp = f.session.projection().stamp;
    let (files, _, _) = filesystem(false);
    let mut state = State::new(files, Arc::new(Notify::new()));
    state.enqueue(
        TreeRequestId(1),
        capture(&mut f.session, 1, row),
        &Cancellation::default(),
    );
    let first = state.tick(Some(stamp));
    assert!(
        first
            .iter()
            .any(|e| matches!(e, TreeSizeEvent::Snapshot { .. }))
    );
    let finished = until(&mut state, stamp, |events| {
        events
            .iter()
            .any(|e| matches!(e, TreeSizeEvent::Finished { .. }))
    })
    .await;
    assert!(
        !finished
            .iter()
            .any(|e| matches!(e, TreeSizeEvent::Snapshot { .. })),
        "unacknowledged first map retains publication slot"
    );
    acknowledge(&mut state, &first);
    let final_map=until(&mut state,stamp,|events|events.iter().any(|e|matches!(e,TreeSizeEvent::Snapshot{sizes} if sizes.get(row).is_some_and(|size|size.finished)))).await;
    let TreeSizeEvent::Snapshot { sizes } = final_map
        .iter()
        .find(|e| matches!(e, TreeSizeEvent::Snapshot { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    assert_eq!(sizes.get(row).unwrap().bytes, 17);
    assert!(!sizes.running(row));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_and_reset_keep_blocked_scan_capacity_until_real_return() {
    let mut f = tests::fixture();
    let row = f.session.projection().rows[0].id;
    let stamp = f.session.projection().stamp;
    let (files, entered, release) = filesystem(true);
    let mut state = State::new(files.clone(), Arc::new(Notify::new()));
    state.enqueue(
        TreeRequestId(1),
        capture(&mut f.session, 1, row),
        &Cancellation::default(),
    );
    state.tick(Some(stamp));
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    state.cancel(stamp, TreeRequestId(1));
    state.reset();
    state.enqueue(
        TreeRequestId(2),
        capture(&mut f.session, 2, row),
        &Cancellation::default(),
    );
    for _ in 0..8 {
        state.tick(Some(stamp));
        tokio::task::yield_now().await;
    }
    assert_eq!(
        files.calls.load(Ordering::SeqCst),
        1,
        "cancellation cannot replace a blocked OS call"
    );
    files.blocked.store(false, Ordering::SeqCst);
    release.send(()).unwrap();
    until(&mut state, stamp, |events| {
        events.iter().any(|e| {
            matches!(
                e,
                TreeSizeEvent::Finished {
                    request: TreeRequestId(2),
                    ..
                }
            )
        })
    })
    .await;
    assert_eq!(files.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn overlapping_queued_root_keeps_running_after_first_finishes() {
    let mut f = tests::fixture();
    let row = f.session.projection().rows[0].id;
    let stamp = f.session.projection().stamp;
    let (files, entered, release) = filesystem(true);
    let mut state = State::new(files, Arc::new(Notify::new()));
    state.enqueue(
        TreeRequestId(1),
        capture(&mut f.session, 1, row),
        &Cancellation::default(),
    );
    let first = state.tick(Some(stamp));
    acknowledge(&mut state, &first);
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    state.enqueue(
        TreeRequestId(2),
        capture(&mut f.session, 2, row),
        &Cancellation::default(),
    );
    release.send(()).unwrap();
    let first_finished = until(&mut state, stamp, |events| {
        events.iter().any(|e| {
            matches!(
                e,
                TreeSizeEvent::Finished {
                    request: TreeRequestId(1),
                    ..
                }
            )
        })
    })
    .await;
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    let running = if first_finished
        .iter()
        .any(|event| matches!(event, TreeSizeEvent::Snapshot { sizes } if sizes.running(row)))
    {
        first_finished
    } else {
        acknowledge(&mut state, &first_finished);
        until(&mut state, stamp, |events| {
            events
                .iter()
                .any(|e| matches!(e,TreeSizeEvent::Snapshot{sizes} if sizes.running(row)))
        })
        .await
    };
    acknowledge(&mut state, &running);
    release.send(()).unwrap();
    until(&mut state, stamp, |events| {
        events.iter().any(|e| {
            matches!(
                e,
                TreeSizeEvent::Finished {
                    request: TreeRequestId(2),
                    ..
                }
            )
        })
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn repeated_fast_scans_respect_one_map_per_quarter_second() {
    let mut f = tests::fixture();
    let row = f.session.projection().rows[0].id;
    let stamp = f.session.projection().stamp;
    let (files, _, _) = filesystem(false);
    let mut state = State::new(files, Arc::new(Notify::new()));
    for id in 1..=8 {
        state.enqueue(
            TreeRequestId(id),
            capture(&mut f.session, id, row),
            &Cancellation::default(),
        );
    }
    let start = Instant::now();
    let mut publications = Vec::new();
    let mut finished = 0;
    while finished < 8 || publications.len() < 2 {
        let events = state.tick(Some(stamp));
        for event in &events {
            match event {
                TreeSizeEvent::Snapshot { .. } => publications.push(Instant::now()),
                TreeSizeEvent::Finished { .. } => finished += 1,
                _ => {}
            }
        }
        acknowledge(&mut state, &events);
        assert!(start.elapsed() < Duration::from_secs(3));
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    for pair in publications.windows(2) {
        assert!(pair[1].duration_since(pair[0]) >= Duration::from_millis(245));
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn selected_totals_use_exact_current_arcs_and_known_completed_bytes() {
    let mut f = tests::fixture();
    let row = f.session.projection().rows[0].id;
    let stamp = f.session.projection().stamp;
    let (files, _, _) = filesystem(false);
    let mut state = State::new(files, Arc::new(Notify::new()));
    state.enqueue(
        TreeRequestId(1),
        capture(&mut f.session, 1, row),
        &Cancellation::default(),
    );
    let first = state.tick(Some(stamp));
    acknowledge(&mut state, &first);
    until(&mut state, stamp, |events| {
        events
            .iter()
            .any(|e| matches!(e, TreeSizeEvent::Finished { .. }))
    })
    .await;
    let publication = f.session.publication().unwrap();
    f.session.commit(&publication.view);
    f.session
        .select(&TreeSelectionRequest {
            view: publication.view,
            revision: SelectionRevision(1),
            seed: None,
            seed_revision: SelectionRevision(0),
            action: TreeSelectionAction::Toggle(row),
        })
        .unwrap();
    let publication = f.session.publication().unwrap();
    let TreeSizeEvent::Selected {
        view,
        selection,
        selection_revision,
        summary,
        ..
    } = state.selected(&publication)
    else {
        unreachable!()
    };
    assert!(Arc::ptr_eq(&view, &publication.view));
    assert!(Arc::ptr_eq(&selection, &publication.selection));
    assert_eq!(selection_revision, SelectionRevision(1));
    assert_eq!(summary.folders, 1);
    assert_eq!(summary.complete, 1);
    assert_eq!(summary.complete_bytes, Some(17));
    assert_eq!(summary.partial, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refresh_invalidation_preserves_ack_slot_and_increasing_snapshot_versions() {
    let mut f = tests::fixture();
    let row = f.session.projection().rows[0].id;
    let stamp = f.session.projection().stamp;
    let (files, _, _) = filesystem(false);
    let mut state = State::new(files, Arc::new(Notify::new()));
    state.enqueue(
        TreeRequestId(1),
        capture(&mut f.session, 1, row),
        &Cancellation::default(),
    );
    let first = state.tick(Some(stamp));
    let TreeSizeEvent::Snapshot { sizes: initial } = first
        .iter()
        .find(|e| matches!(e, TreeSizeEvent::Snapshot { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    until(&mut state, stamp, |events| {
        events
            .iter()
            .any(|e| matches!(e, TreeSizeEvent::Finished { .. }))
    })
    .await;
    state.invalidate();
    assert!(state.sort_sizes().is_empty());
    assert!(
        !state
            .tick(Some(stamp))
            .iter()
            .any(|e| matches!(e, TreeSizeEvent::Snapshot { .. })),
        "refresh cannot steal the outstanding ACK"
    );
    acknowledge(&mut state, &first);
    let cleared = until(&mut state, stamp, |events| {
        events
            .iter()
            .any(|e| matches!(e, TreeSizeEvent::Snapshot { .. }))
    })
    .await;
    let TreeSizeEvent::Snapshot { sizes } = cleared
        .iter()
        .find(|e| matches!(e, TreeSizeEvent::Snapshot { .. }))
        .unwrap()
    else {
        unreachable!()
    };
    assert!(sizes.version > initial.version);
    assert!(sizes.is_empty());
    assert!(!sizes.running(row));
}
