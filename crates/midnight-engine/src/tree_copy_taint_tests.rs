//! A tainted archive session held by a tree Copy capture, its prepared roots, a pane
//! navigation from the same tree, or the targets of any tree capture. However its
//! owners release or give up their references, the last one reaches the host's
//! tainted release, which stops the worker and returns its process permit.
use super::*;
use crate::{
    archives::Retirement,
    copy_nested_retirement as nested,
    providers::{ProviderRegistry, captured_archive_root},
    tree_actor::WorkerBudget,
    tree_projection::ProjectionOptions,
};

/// `TaintedHost::id`.
const HOST: u64 = 72;

fn member(entry: u64, name: &str) -> ArchiveNode {
    ArchiveNode {
        id: VirtualEntryId { mount: HOST, entry },
        parent: 1,
        raw_name: Arc::from(name.as_bytes()),
        display_name: name.into(),
        kind: EntryKind::Directory,
        size: None,
        readable: false,
        modified: None,
        access: Default::default(),
    }
}

/// A tree over the tab's session, seeded with the directories `a` and `b`.
fn tree_over(tab: &Arc<dyn ArchiveMount>) -> Session {
    let location = ArchiveLocation {
        mount: Some(tab.clone()),
        address: ArchiveAddress {
            nested: Vec::new(),
            provider: "taint-fixture".into(),
            format: None,
            container: std::env::temp_dir().join("tainted.zip"),
            source_version: vec![1],
            components: Vec::new(),
        },
        directory: 1,
        trail: vec![(1, "tainted.zip".into())],
    };
    let entries: Arc<[Entry]> = Arc::from([
        Entry::from_archive(member(2, "a")),
        Entry::from_archive(member(3, "b")),
    ]);
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    };
    let opened = captured_archive_root(&location, &root.entries, &Cancellation::default()).unwrap();
    Session::new(
        root,
        opened,
        Arc::new(ProviderRegistry::default()),
        WorkerBudget::new(Arc::new(Semaphore::new(2))),
        ProjectionOptions::default(),
        &Cancellation::default(),
        crate::ReportArena::testing(),
    )
    .unwrap()
}

fn select(session: &mut Session, action: TreeSelectionAction) -> TreePublication {
    let current = session.publication().unwrap();
    assert!(session.commit(&current.view));
    session
        .select(&TreeSelectionRequest {
            view: current.view,
            revision: SelectionRevision(current.selection_revision.0 + 1),
            seed: None,
            seed_revision: SelectionRevision(0),
            action,
        })
        .unwrap();
    session.publication().unwrap()
}

/// The tree's Copy capture of both directories (holding the one capture permit of
/// `capacity`) and, when asked, a pane navigation into `a` that the pane's archive
/// guard adopted. The tree itself is closed with ordinary drops.
fn capture_and_navigate(
    tab: &Arc<dyn ArchiveMount>,
    capacity: &Arc<Semaphore>,
    navigate: bool,
) -> (
    Arc<dyn CapturedTransferSources>,
    Option<Arc<dyn ArchiveMount>>,
) {
    let mut session = tree_over(tab);
    let rows = session
        .projection()
        .rows
        .iter()
        .map(|row| row.id)
        .collect::<Vec<_>>();
    assert_eq!(rows.len(), 2);
    select(&mut session, TreeSelectionAction::Toggle(rows[0]));
    let selected = select(&mut session, TreeSelectionAction::Toggle(rows[1]));
    let capture = session
        .capture_transfer(
            &TreeTransferRequest {
                intent: TransferIntent::Copy,
                view: selected.view.clone(),
                request: TreeRequestId(1),
                selection_revision: selected.selection_revision,
                selection: selected.selection.clone(),
                fallback: None,
                cancel: Cancellation::default(),
            },
            capacity,
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(capture.root_count, 2);
    let pane = navigate.then(|| {
        let TreeNavigationTarget::Archive(mut location) = session
            .navigate(
                &selected.view,
                TreeRequestId(2),
                rows[0],
                NavigationId(2),
                &Cancellation::default(),
            )
            .unwrap()
        else {
            panic!("archive navigation expected")
        };
        // As the pane's archive navigation adopts it.
        let (pane, _, _) = Retirement::copy_mount_for_test(location.mount.take().unwrap());
        pane
    });
    drop(session);
    (capture.sources, pane)
}

/// How a release step runs: in a fresh Copy context of its own, or in one that an
/// earlier failure already quarantined.
#[derive(Clone, Copy, Debug)]
enum Context {
    Fresh,
    Quarantined,
}
impl Context {
    fn context(self) -> nested::Context {
        let context = nested::Context::default();
        if matches!(self, Self::Quarantined) {
            context.quarantine();
        }
        context
    }
}

#[test]
fn a_tainted_session_shared_by_a_tree_copy_capture_and_a_navigation_ends_its_worker_in_either_order()
 {
    use Context::{Fresh, Quarantined};
    let active = Arc::new(Semaphore::new(2));
    let contexts = [
        (Fresh, Fresh),
        (Fresh, Quarantined),
        (Quarantined, Fresh),
        (Quarantined, Quarantined),
    ];
    for (copy_context, pane_context) in contexts {
        for copy_first in [true, false] {
            let case =
                format!("copy {copy_context:?}, pane {pane_context:?}, copy first: {copy_first}");
            let (host, probe) = Retirement::tainted_host_for_test(&active);
            let (tab, _, _) = Retirement::copy_mount_for_test(host);
            let capacity = Arc::new(Semaphore::new(1));
            let (copy, pane) = capture_and_navigate(&tab, &capacity, true);
            let pane = pane.unwrap();
            probe.taint();
            let host = || (probe.releases(), probe.stopped(), probe.dropped());
            // The tab gives up only its count.
            assert!(
                Context::Fresh
                    .context()
                    .with(|| nested::release_archive(tab))
                    .is_err(),
                "{case}"
            );
            assert_eq!(host(), (0, false, false), "{case}");
            let release_copy = move || {
                copy_context
                    .context()
                    .with(|| nested::release_transfer_sources(copy))
                    .is_err()
            };
            let release_pane = move || {
                pane_context
                    .context()
                    .with(|| nested::release_archive(pane))
                    .is_err()
            };
            if copy_first {
                release_in_order(release_copy, release_pane, host, &active, &case);
            } else {
                release_in_order(release_pane, release_copy, host, &active, &case);
            }
            // Given up with its unconfirmed targets, the capture keeps its permit.
            assert_eq!(capacity.available_permits(), 0, "{case}");
        }
    }
}

/// Each release fails, as every release of a tainted session does. The first only
/// gives up counts; the last one reaches the host's tainted release, which stops the
/// worker and returns its process permit without destroying the tainted source.
fn release_in_order(
    first: impl FnOnce() -> bool,
    last: impl FnOnce() -> bool,
    host: impl Fn() -> (usize, bool, bool),
    active: &Semaphore,
    case: &str,
) {
    assert!(first(), "{case}: a tainted release succeeded");
    assert_eq!(
        host(),
        (0, false, false),
        "{case}: a shared release ended it"
    );
    assert_eq!(active.available_permits(), 1, "{case}");
    assert!(last(), "{case}: a tainted release succeeded");
    assert_eq!(
        host(),
        (1, true, false),
        "{case}: the last owner missed the host"
    );
    assert_eq!(active.available_permits(), 2, "{case}: process permit kept");
}

/// The tree's capture of both directories over a tainted session whose tab already
/// gave its count up, so the capture's two targets hold the session's last references.
/// Also returns the host's (releases, stopped, dropped).
fn tainted_capture(
    active: &Arc<Semaphore>,
) -> (CapturedSelection, impl Fn() -> (usize, bool, bool)) {
    let (host, probe) = Retirement::tainted_host_for_test(active);
    let (tab, _, _) = Retirement::copy_mount_for_test(host);
    let mut session = tree_over(&tab);
    let rows = session
        .projection()
        .rows
        .iter()
        .map(|row| row.id)
        .collect::<Vec<_>>();
    select(&mut session, TreeSelectionAction::Toggle(rows[0]));
    let selected = select(&mut session, TreeSelectionAction::Toggle(rows[1]));
    let capture = session
        .capture_selection(
            &selected.view,
            selected.selection_revision,
            &selected.selection,
            None,
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(capture.targets.len(), 2);
    drop(session);
    probe.taint();
    assert!(
        Context::Fresh
            .context()
            .with(|| nested::release_archive(tab))
            .is_err()
    );
    let host = move || (probe.releases(), probe.stopped(), probe.dropped());
    assert_eq!(host(), (0, false, false));
    (capture, host)
}

/// A tree capture gives up what its release cannot reach instead of forgetting it:
/// the targets after a failed one, and a selection or target whose own release step
/// finds the context quarantined, as when another holder quarantines it between the
/// caller's check and that step. The session's last reference then still reaches the
/// host. Clipboard, drop-directory and tree Copy captures share these paths.
#[test]
fn a_tree_capture_gives_up_what_its_release_cannot_reach() {
    /// Releases the capture one way; true when that release failed.
    type Release = fn(CapturedSelection) -> bool;
    let active = Arc::new(Semaphore::new(2));
    let cases: [(&str, Release); 3] = [
        ("the second target after the first failed", |capture| {
            Context::Fresh
                .context()
                .with(|| capture.release_on_worker())
                .is_err()
        }),
        ("the selection in a quarantined context", |capture| {
            Context::Quarantined
                .context()
                .with(|| capture.release_on_worker())
                .is_err()
        }),
        ("each target in a quarantined context", |capture| {
            capture.targets.into_iter().all(|target| {
                Context::Quarantined
                    .context()
                    .with(|| target.captured.release_on_worker())
                    .is_err()
            })
        }),
    ];
    for (case, release) in cases {
        let (capture, host) = tainted_capture(&active);
        assert!(release(capture), "{case}: a tainted release succeeded");
        assert_eq!(
            host(),
            (1, true, false),
            "{case}: the last owner missed the host"
        );
        assert_eq!(active.available_permits(), 2, "{case}: process permit kept");
    }
}

#[test]
fn prepared_tree_copy_archive_roots_dropped_in_a_copy_give_up_the_tainted_session() {
    let active = Arc::new(Semaphore::new(2));
    for context in [Context::Fresh, Context::Quarantined] {
        for capture_last in [false, true] {
            let case = format!("{context:?}, capture released last: {capture_last}");
            let (host, probe) = Retirement::tainted_host_for_test(&active);
            let (tab, _, _) = Retirement::copy_mount_for_test(host);
            // Preparing a root lists its parent directory before the taint.
            probe.answer_lists(vec![member(2, "a"), member(3, "b")]);
            let capacity = Arc::new(Semaphore::new(1));
            let (copy, _) = capture_and_navigate(&tab, &capacity, false);
            let service = midnight_platform::LocalFileSystem::new(None);
            let PreparedCapturedTransfer::Copy(roots) = copy
                .prepare_with_links(&service, CopyLinkPolicy::Reject, &Cancellation::default())
                .unwrap()
            else {
                panic!("tree Copy roots expected")
            };
            assert_eq!(roots.len(), 2);
            let mut copy = Some(copy);
            if !capture_last {
                // Preparation handed the capture over to its roots.
                Context::Fresh
                    .context()
                    .with(|| nested::release_transfer_sources(copy.take().unwrap()))
                    .unwrap();
            }
            probe.taint();
            assert!(
                Context::Fresh
                    .context()
                    .with(|| nested::release_archive(tab))
                    .is_err()
            );
            assert_eq!(probe.releases(), 0, "{case}");
            // The Copy job's payload drop: the first archive root fails on the tainted
            // session and quarantines the context, unless it already was. A capture
            // released after the roots is given up in that quarantined context.
            let copy_context = context.context();
            copy_context.with(|| {
                drop(roots);
                if let Some(copy) = copy.take() {
                    assert!(nested::release_transfer_sources(copy).is_err(), "{case}");
                }
            });
            assert!(copy_context.is_quarantined(), "{case}");
            assert_eq!(probe.releases(), 1, "{case}: the worker was kept");
            assert!(probe.stopped() && !probe.dropped(), "{case}");
            assert_eq!(active.available_permits(), 2, "{case}");
            assert_eq!(capacity.available_permits(), 0, "{case}");
        }
    }
}
