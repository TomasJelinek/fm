use super::*;
use midnight_core::{
    DirectoryIndex, Entry, EntryId, EntryKind, FilenameColorRule, FilenameColorTarget,
    FilenameStyle, NavigationId, PaneId, RgbColor, TreeDisclosure, TreeDisplayRow,
    TreeLayoutRevision, TreeProjection, TreeRevision, TreeRootSnapshot, TreeRowId, TreeSessionId,
    TreeViewStamp,
};

fn request() -> FilenameColorsRequest {
    FilenameColorsRequest {
        id: 1,
        pane: PaneId::Left,
        tree: None,
        source: vec![
            Entry::new(EntryId(1), "doc.rs".into(), EntryKind::File),
            Entry::new(EntryId(2), "doc.txt".into(), EntryKind::File),
        ]
        .into(),
        rules: vec![FilenameColorRule {
            expression: "*.rs".into(),
            style: FilenameStyle {
                foreground: Some(RgbColor(10, 20, 30)),
                background: None,
            },
            ..Default::default()
        }]
        .into(),
    }
}

fn row(
    id: u64,
    mut entry: Entry,
    parent: Option<u64>,
    depth: usize,
    expanded: bool,
) -> TreeDisplayRow {
    let original_root_entry = parent.is_none().then_some(entry.id);
    entry.id = EntryId(id + 1_000);
    entry.display_name = "display label without a matching suffix".into();
    TreeDisplayRow {
        id: TreeRowId(id),
        entry,
        parent: parent.map(TreeRowId),
        depth,
        disclosure: if expanded {
            TreeDisclosure::Expanded
        } else {
            TreeDisclosure::None
        },
        expanded,
        selectable: true,
        navigable: expanded,
        context_only: false,
        error: None,
        error_details: None,
        original_root_entry,
        prefix_end: 0,
        search_match: true,
    }
}

fn tree_request() -> FilenameColorsRequest {
    let base = std::env::current_dir().unwrap();
    let root_pdf = Entry::new(EntryId(1), base.join("root.pdf"), EntryKind::File);
    let directory = Entry::new(EntryId(2), base.join("folder"), EntryKind::Directory);
    let ordinary = Entry::new(EntryId(3), base.join("ordinary.bin"), EntryKind::File);
    let source: Arc<[Entry]> = vec![root_pdf.clone(), directory.clone(), ordinary.clone()].into();
    let root = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(7),
        index: Arc::new(DirectoryIndex::from_entries(&source)),
        entries: source.clone(),
    };
    let mut rows = vec![
        row(1, root_pdf, None, 0, false),
        row(2, directory, None, 0, true),
        row(
            3,
            Entry::new(EntryId(31), base.join("folder/child.PDF"), EntryKind::File),
            Some(2),
            1,
            false,
        ),
        row(
            4,
            Entry::new(
                EntryId(32),
                base.join("folder/nested"),
                EntryKind::DirectoryLink,
            ),
            Some(2),
            1,
            true,
        ),
        row(
            5,
            Entry::new(
                EntryId(33),
                base.join("folder/nested/grandchild.pdf"),
                EntryKind::File,
            ),
            Some(4),
            2,
            false,
        ),
        row(6, ordinary, None, 0, false),
    ];
    rows[5].entry.display_name = "misleading-display.pdf".into();
    let tree = TreeProjection::new(
        root,
        TreeViewStamp {
            pane: PaneId::Left,
            navigation: NavigationId(7),
            session: TreeSessionId(9),
            revision: TreeRevision(1),
        },
        TreeLayoutRevision(1),
        rows,
    )
    .unwrap();
    FilenameColorsRequest {
        id: 1,
        pane: PaneId::Left,
        source,
        tree: Some(Arc::new(tree)),
        rules: midnight_core::default_filename_colors().into(),
    }
}

fn matched(request: FilenameColorsRequest) -> Arc<FilenameColorsResult> {
    let Some(Event::FilenameColorsReady {
        result: Ok(result), ..
    }) = compute(&Work::Match(request), &|| false)
    else {
        panic!("expected successful filename matching");
    };
    result
}

#[test]
fn tree_root_child_and_grandchild_use_raw_names_and_synthetic_entry_ids() {
    let request = tree_request();
    let result = matched(request.clone());
    assert!(Arc::ptr_eq(&result.request.source, &request.source));
    assert!(Arc::ptr_eq(
        result.request.tree.as_ref().unwrap(),
        request.tree.as_ref().unwrap()
    ));
    assert!(Arc::ptr_eq(&result.request.rules, &request.rules));
    assert_eq!(result.styles.len(), 3);
    for id in [1_001, 1_003, 1_005] {
        assert_eq!(
            result.styles[&EntryId(id)].foreground,
            Some(RgbColor(239, 90, 90))
        );
    }
    for id in [1, 2, 3, 4, 5, 6, 1_002, 1_004, 1_006] {
        assert!(
            !result.styles.contains_key(&EntryId(id)),
            "unexpected style identity {id}"
        );
    }
}

#[test]
fn tree_targets_keep_files_folders_all_and_ordered_first_match_behavior() {
    for (target, expected) in [
        (FilenameColorTarget::Files, vec![1_001, 1_003, 1_005, 1_006]),
        (FilenameColorTarget::Folders, vec![1_002, 1_004]),
        (
            FilenameColorTarget::All,
            vec![1_001, 1_002, 1_003, 1_004, 1_005, 1_006],
        ),
    ] {
        let mut request = tree_request();
        request.rules = vec![
            FilenameColorRule {
                expression: "*".into(),
                target,
                style: FilenameStyle {
                    foreground: Some(RgbColor(1, 2, 3)),
                    background: None,
                },
                ..Default::default()
            },
            FilenameColorRule {
                expression: "*".into(),
                target,
                style: FilenameStyle {
                    foreground: Some(RgbColor(7, 8, 9)),
                    background: None,
                },
                ..Default::default()
            },
        ]
        .into();
        let result = matched(request);
        let mut actual: Vec<_> = result.styles.keys().map(|id| id.0).collect();
        actual.sort_unstable();
        assert_eq!(actual, expected);
        assert!(
            result
                .styles
                .values()
                .all(|style| style.foreground == Some(RgbColor(1, 2, 3)))
        );
    }
}

#[cfg(windows)]
fn undecodable_name() -> std::ffi::OsString {
    use std::os::windows::ffi::OsStringExt;
    std::ffi::OsString::from_wide(&[0xD800, 46, 112, 100, 102])
}
#[cfg(unix)]
fn undecodable_name() -> std::ffi::OsString {
    use std::os::unix::ffi::OsStringExt;
    std::ffi::OsString::from_vec(vec![0xff, b'.', b'p', b'd', b'f'])
}

#[cfg(any(windows, unix))]
#[test]
fn tree_undecodable_name_never_matches_its_pdf_display_label() {
    let mut request = tree_request();
    let prior = request.tree.take().unwrap();
    let mut rows = prior.rows.to_vec();
    rows[2].entry.name = undecodable_name();
    rows[2].entry.display_name = "child.pdf".into();
    request.tree = Some(Arc::new(
        TreeProjection::new(prior.root.clone(), prior.stamp, prior.layout_revision, rows).unwrap(),
    ));
    let result = matched(request);
    assert!(!result.styles.contains_key(&EntryId(1_003)));
    assert!(result.styles.contains_key(&EntryId(1_001)));
    assert!(result.styles.contains_key(&EntryId(1_005)));
    assert_eq!(
        result.request.tree.as_ref().unwrap().rows[2].entry.name,
        undecodable_name()
    );
}

#[test]
fn tree_request_fences_panes_navigation_and_root_arc_before_publication() {
    for mismatch in 0..5 {
        let mut request = tree_request();
        match mismatch {
            0 => request.pane = PaneId::Right,
            1 => Arc::make_mut(request.tree.as_mut().unwrap()).stamp.pane = PaneId::Right,
            2 => Arc::make_mut(request.tree.as_mut().unwrap()).root.pane = PaneId::Right,
            3 => {
                Arc::make_mut(request.tree.as_mut().unwrap())
                    .root
                    .navigation = NavigationId(8)
            }
            _ => request.source = request.source.to_vec().into(),
        }
        assert!(
            matches!(
                compute(&Work::Match(request), &|| false),
                Some(Event::FilenameColorsReady { result: Err(_), .. })
            ),
            "accepted mismatched fence {mismatch}"
        );
    }
}

#[test]
fn tree_duplicate_synthetic_ids_and_disabled_invalid_rules_cannot_publish_styles() {
    let mut request = tree_request();
    let tree = Arc::make_mut(request.tree.as_mut().unwrap());
    let rows = Arc::make_mut(&mut tree.rows);
    rows[2].entry.id = rows[0].entry.id;
    assert!(matches!(
        compute(&Work::Match(request), &|| false),
        Some(Event::FilenameColorsReady { result: Err(_), .. })
    ));
    let mut request = tree_request();
    request.rules = vec![FilenameColorRule {
        enabled: false,
        mode: midnight_core::FilterMode::Regex,
        expression: "[".into(),
        ..Default::default()
    }]
    .into();
    assert!(matches!(
        compute(&Work::Match(request), &|| false),
        Some(Event::FilenameColorsReady { result: Err(_), .. })
    ));
}

#[test]
fn tree_matching_cancels_during_rows_and_releases_the_snapshot_without_an_event() {
    let mut request = tree_request();
    let prior = request.tree.take().unwrap();
    let mut rows = vec![prior.rows[1].clone()];
    for index in 0..600 {
        let entry = Entry::new(
            EntryId(2_000 + index),
            prior.root.entries[1].path.join(format!("{index}.pdf")),
            EntryKind::File,
        );
        rows.push(row(100 + index, entry, Some(2), 1, false));
    }
    request.tree = Some(Arc::new(
        TreeProjection::new(prior.root.clone(), prior.stamp, prior.layout_revision, rows).unwrap(),
    ));
    drop(prior);
    let retained = Arc::downgrade(request.tree.as_ref().unwrap());
    let work = Work::Match(request);
    let checks = std::cell::Cell::new(0);
    assert!(
        compute(&work, &|| {
            checks.set(checks.get() + 1);
            checks.get() >= 2
        })
        .is_none()
    );
    assert_eq!(
        checks.get(),
        3,
        "checks at row0, row256, then final suppression"
    );
    assert!(retained.upgrade().is_some());
    drop(work);
    assert!(retained.upgrade().is_none());
}

async fn wait_for_tree_owners(
    tree: &std::sync::Weak<TreeProjection>,
    ready: impl Fn(usize) -> bool,
) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        while !ready(tree.strong_count()) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("filename matching must reach its ownership boundary");
}

async fn blocked_output() -> (Controller, JoinSet<()>, tokio::sync::mpsc::Receiver<Event>) {
    let (sender, events) = tokio::sync::mpsc::channel(1);
    sender
        .send(Event::FilenameColorsValidated {
            request: FilenameColorsValidation {
                id: 0,
                rules: vec![],
            },
            result: Ok(()),
        })
        .await
        .unwrap();
    let mut tasks = JoinSet::new();
    let controller = Controller::new(
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    (controller, tasks, events)
}

#[tokio::test]
async fn clear_cancels_a_finished_tree_result_waiting_for_output_and_releases_its_arc() {
    let (controller, mut tasks, mut events) = blocked_output().await;
    let request = tree_request();
    let retained = Arc::downgrade(request.tree.as_ref().unwrap());
    controller.matches(request);
    wait_for_tree_owners(&retained, |owners| owners >= 3).await;
    controller.clear(PaneId::Left, 2);
    wait_for_tree_owners(&retained, |owners| owners == 0).await;
    assert!(matches!(
        events.recv().await,
        Some(Event::FilenameColorsValidated { .. })
    ));
    drop(controller);
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    assert!(
        events.recv().await.is_none(),
        "Clear must suppress obsolete tree styles"
    );
}

#[tokio::test]
async fn replaced_tree_with_same_root_and_layout_revision_suppresses_old_output() {
    let (controller, mut tasks, mut events) = blocked_output().await;
    let first = tree_request();
    let retained = Arc::downgrade(first.tree.as_ref().unwrap());
    let mut latest = first.clone();
    latest.id = 2;
    let old_tree = latest.tree.take().unwrap();
    let mut rows = old_tree.rows.to_vec();
    rows[2].entry.name = "new-name.bin".into();
    latest.tree = Some(Arc::new(
        TreeProjection::new(
            old_tree.root.clone(),
            TreeViewStamp {
                revision: TreeRevision(2),
                ..old_tree.stamp
            },
            old_tree.layout_revision,
            rows,
        )
        .unwrap(),
    ));
    drop(old_tree);
    let expected = latest.tree.as_ref().unwrap().clone();
    controller.matches(first);
    wait_for_tree_owners(&retained, |owners| owners >= 3).await;
    controller.matches(latest);
    wait_for_tree_owners(&retained, |owners| owners == 0).await;
    assert!(matches!(
        events.recv().await,
        Some(Event::FilenameColorsValidated { .. })
    ));
    let event = tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    let Event::FilenameColorsReady {
        request,
        result: Ok(result),
    } = event
    else {
        panic!("latest tree must publish")
    };
    assert_eq!(request.id, 2);
    assert!(Arc::ptr_eq(request.tree.as_ref().unwrap(), &expected));
    assert!(Arc::ptr_eq(
        result.request.tree.as_ref().unwrap(),
        &expected
    ));
    assert!(!result.styles.contains_key(&EntryId(1_003)));
    drop(controller);
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    assert!(events.recv().await.is_none());
}

#[test]
fn computation_preserves_exact_source_and_matches_only_requested_names() {
    let request = request();
    let event = compute(&Work::Match(request.clone()), &|| false).unwrap();
    let Event::FilenameColorsReady {
        result: Ok(result), ..
    } = event
    else {
        panic!("expected matched result")
    };
    assert!(Arc::ptr_eq(&result.request.source, &request.source));
    assert_eq!(result.styles.len(), 1);
    assert_eq!(
        result.styles[&EntryId(1)].foreground,
        Some(RgbColor(10, 20, 30))
    );
}

#[test]
fn cancellation_and_ambiguous_ids_cannot_publish_styles() {
    let mut request = request();
    assert!(compute(&Work::Match(request.clone()), &|| true).is_none());
    request.source = vec![request.source[0].clone(), request.source[0].clone()].into();
    assert!(matches!(
        compute(&Work::Match(request), &|| false),
        Some(Event::FilenameColorsReady { result: Err(_), .. })
    ));
}

#[tokio::test]
async fn coalesced_validation_publishes_only_latest_revision() {
    let (sender, mut events) = tokio::sync::mpsc::channel(4);
    let outbox = Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    };
    let mut tasks = JoinSet::new();
    let controller = Controller::new(outbox, &mut tasks);
    controller.validate(FilenameColorsValidation {
        id: 1,
        rules: vec![],
    });
    controller.validate(FilenameColorsValidation {
        id: 2,
        rules: vec![FilenameColorRule {
            expression: "[".into(),
            mode: midnight_core::FilterMode::Regex,
            ..Default::default()
        }],
    });
    let event = tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        event,
        Event::FilenameColorsValidated {
            request: FilenameColorsValidation { id: 2, .. },
            result: Err(_)
        }
    ));
    drop(controller);
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
}

#[tokio::test]
async fn disabling_colors_releases_captured_source_and_cancels_pending_publication() {
    let (sender, mut events) = tokio::sync::mpsc::channel(4);
    let outbox = Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    };
    let mut tasks = JoinSet::new();
    let controller = Controller::new(outbox, &mut tasks);
    let request = request();
    let source = Arc::downgrade(&request.source);
    controller.matches(request);
    controller.clear(PaneId::Left, 2);
    drop(controller);
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }
    assert!(events.recv().await.is_none());
    assert!(source.upgrade().is_none());
}
