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
