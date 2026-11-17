use super::*;
use crate::{
    native_output_test_fixture as fixture,
    providers::{OpenedLocation, ProviderRegistry},
    tree_actor::{BranchJob, Session, WorkerBudget},
    tree_projection::ProjectionOptions,
};
use midnight_core::*;
use std::{
    sync::mpsc,
    thread::{self, ThreadId},
    time::Duration,
};

fn branch() -> (
    tempfile::TempDir,
    BranchJob,
    mpsc::Receiver<ThreadId>,
    Arc<Semaphore>,
    Session,
) {
    let (directory, owner, report) = fixture::owner();
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(1),
        directory.path().join("child"),
        EntryKind::Directory,
    )]
    .into();
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    };
    let listing = Arc::new(Semaphore::new(1));
    let mut session = Session::new(
        root,
        OpenedLocation {
            mount: owner,
            root: fixture::token(0),
        },
        Arc::new(ProviderRegistry::default()),
        WorkerBudget::new(listing.clone()),
        ProjectionOptions::default(),
        &Cancellation::default(),
        crate::ReportArena::testing(),
    )
    .unwrap();
    let view = session.projection().clone();
    assert!(session.commit(&view));
    session
        .branch(
            &view,
            TreeRequestId(1),
            view.rows[0].id,
            TreeBranchAction::Expand,
        )
        .unwrap();
    let job = session.take_work().unwrap().unwrap();
    (directory, job, report, listing, session)
}

#[test]
fn queued_completion_does_not_retain_a_native_mount_after_model_worker_unwind() {
    let (_directory, job, dropped, listing, session) = branch();
    let inbox = Arc::new(Mailbox::default());
    let model = inbox.clone();
    let result = thread::spawn(move || {
        model.complete(job.run());
        drop(session);
        panic!("injected model worker unwind before mailbox drain");
    })
    .join();
    assert!(result.is_err());
    assert_eq!(
        listing.available_permits(),
        0,
        "queued completion retains admission"
    );
    // Completed rows contain only NodeRef/display metadata. The consumed root
    // registration no longer owns the native mount either; release already ran
    // on the model worker although the completion and admission remain queued.
    assert_ne!(dropped.try_recv().unwrap(), thread::current().id());
    inbox.close();
    drop(inbox);
    assert_eq!(listing.available_permits(), 1);
}

/// The tree workers' only filesystem; idle workers never call it.
struct Idle;
impl DirectoryService for Idle {
    fn read_directory(
        &self,
        _: &std::path::Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        Ok(())
    }
    fn entry_metadata(&self, _: &std::path::Path) -> Result<EntryMetadata, FsError> {
        Ok(EntryMetadata::default())
    }
}
impl VolumeService for Idle {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(Vec::new())
    }
}
impl StorageService for Idle {
    fn space_info(&self, _: &std::path::Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            total: 0,
            available: 0,
        })
    }
}

/// Reports the thread that releases the last copy of the controller's outbox.
struct LastOwner(mpsc::Sender<Option<String>>);
impl Drop for LastOwner {
    fn drop(&mut self) {
        let _ = self.0.send(thread::current().name().map(str::to_owned));
    }
}

#[test]
fn pane_workers_run_on_named_threads_without_blocking_pool_capacity() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    // The only blocking-pool thread stays occupied until the end of the test.
    let (entered, ready) = mpsc::channel();
    let (release, wait) = mpsc::channel::<()>();
    let blocker = runtime.spawn_blocking(move || {
        entered.send(()).unwrap();
        let _ = wait.recv();
    });
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    let (owner, last_owner) = mpsc::channel();
    let owner = LastOwner(owner);
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(1),
        std::env::current_dir()
            .unwrap()
            .join("_build/tree-worker-root/child"),
        EntryKind::Directory,
    )]
    .into();
    let retained = Arc::downgrade(&entries);
    runtime.block_on(async {
        let (sender, _events) = tokio::sync::mpsc::channel(4);
        let mut controller = Controller::new(
            Services {
                shared_native: None,
                qualified_locale: None,
                rename: crate::tree_rename::Retirement::testing(),
                filesystem: Arc::new(Idle),
                locale: None,
                reports: crate::ReportArena::testing(),
            },
            std::array::from_fn(|_| Arc::new(Semaphore::new(1))),
            Arc::new(Semaphore::new(1)),
            crate::test_snapshots(Default::default()),
            crate::Outbox {
                sender,
                wakeup: Arc::new(move || {
                    let _ = &owner;
                }),
            },
            None,
        );
        controller.invalidate(PaneId::Left, NavigationId(1), SortSpec::default());
        let root = TreeRootSnapshot {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        };
        let command = TreeCommand::Enable {
            root,
            selection_revision: SelectionRevision(0),
            selected: TreeRootSelection::Empty,
        };
        assert!(controller.submit(command).await.is_none());
        // Closed lanes stop their workers; each releases its pending root.
        for lane in &controller.lanes {
            lane.close();
        }
        for worker in &mut controller.workers {
            tokio::time::timeout(Duration::from_secs(5), worker.take().unwrap())
                .await
                .expect("a pane worker must not wait for blocking-pool capacity")
                .unwrap();
        }
    });
    assert!(retained.upgrade().is_none());
    assert_eq!(
        last_owner.try_recv().unwrap().as_deref(),
        Some("midnight-tree-worker")
    );
    release.send(()).unwrap();
    runtime.block_on(blocker).unwrap();
}

#[test]
fn queued_branch_runtime_shutdown_closes_native_owner_on_blocking_worker() {
    let (_directory, job, dropped, listing, session) = branch();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let (entered, ready) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    let blocker = runtime.spawn_blocking(move || {
        entered.send(()).unwrap();
        wait.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    let inbox = Arc::new(Mailbox::default());
    let task = {
        let _entered = runtime.enter();
        inbox.start_branch(job)
    };
    drop(session);
    inbox.close();
    drop(inbox);
    let caller = thread::current().id();
    task.abort();
    runtime.shutdown_background();
    // Always release the fixture before assertions, including on regression.
    let early = dropped.try_recv().ok();
    let permits = listing.available_permits();
    release.send(()).unwrap();
    let retired = early
        .or_else(|| dropped.recv_timeout(Duration::from_secs(5)).ok())
        .unwrap();
    drop(task);
    drop(blocker);
    assert_ne!(
        retired, caller,
        "queued native closure must not retire on shutdown caller"
    );
    assert_eq!(
        permits, 0,
        "admission must remain held while queued closure is retained"
    );
}
