//! A panel sorted by Size re-sorts itself when calculated folder sizes change its
//! order: once per interval while a calculation runs and once at its end, keeping
//! the cursor on its item, and never when the order stands or another column sorts.
use super::*;

fn sizes_panel(names: &[&str]) -> (Presenter, NavigationId, Vec<Entry>) {
    let root = test_root().join("panel");
    let mut rows: Vec<_> = names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            Entry::new(
                EntryId(index as u64 + 1),
                root.join(name),
                EntryKind::Directory,
            )
        })
        .collect();
    let mut file = Entry::new(EntryId(99), root.join("file.txt"), EntryKind::File);
    file.metadata.size = Some(1);
    rows.push(file);
    let mut p = Presenter::new(root.clone(), test_root());
    let navigation = begin(&mut p, PaneId::Left, root.to_str().unwrap());
    finish(&mut p, PaneId::Left, navigation, rows.clone());
    (p, navigation, rows)
}

fn sort_by_size(p: &mut Presenter, rows: &[Entry]) {
    assert!(p.sort(PaneId::Left, SortColumn::Size, &|_| true));
    let current = NavigationId(p.panes[0].generation);
    finish(p, PaneId::Left, current, rows.to_vec());
}

/// Start a full calculation; it belongs to the panel's current navigation.
fn start_scan(p: &mut Presenter) -> (NavigationId, FolderSizeScanId) {
    let navigation = NavigationId(p.panes[0].generation);
    let commands = RefCell::new(Vec::new());
    p.folder_sizes(PaneId::Left, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    match commands.into_inner().pop().unwrap() {
        Command::ComputeFolderSizes { scan, .. } => (navigation, scan),
        _ => panic!("expected a single scan request"),
    }
}

/// Apply `event` as the application does and return the re-sorts it caused.
fn apply(p: &mut Presenter, event: Event) -> Vec<SortSpec> {
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(event, &|command| {
        sent.borrow_mut().push(command);
        true
    });
    sent.into_inner()
        .into_iter()
        .filter_map(|command| match command {
            Command::Resort { sort, .. } => Some(sort),
            _ => None,
        })
        .collect()
}

fn measured(
    navigation: NavigationId,
    scan: FolderSizeScanId,
    entry: u64,
    bytes: u64,
    errors: u64,
) -> Event {
    Event::FolderSizeUpdated {
        pane: PaneId::Left,
        navigation,
        scan,
        entry: EntryId(entry),
        size: FolderSize {
            bytes,
            errors,
            finished: true,
            ..Default::default()
        },
    }
}

fn finished(navigation: NavigationId, scan: FolderSizeScanId) -> Event {
    Event::FolderSizesFinished {
        pane: PaneId::Left,
        navigation,
        scan,
        cancelled: false,
    }
}

const ASCENDING: SortSpec = SortSpec {
    column: SortColumn::Size,
    descending: false,
};

#[test]
fn arriving_sizes_resort_the_panel_and_keep_the_cursor_on_its_folder() {
    let (mut p, _, rows) = sizes_panel(&["a", "b", "c"]);
    sort_by_size(&mut p, &rows);
    p.panes[0].cursor = Some(EntryId(2));
    let (navigation, scan) = start_scan(&mut p);
    // "c" finished, partial after an unreadable entry: it now leads the folders.
    assert_eq!(
        apply(&mut p, measured(navigation, scan, 3, 50, 1)),
        [ASCENDING]
    );
    let resorted = NavigationId(p.panes[0].generation);
    finish(
        &mut p,
        PaneId::Left,
        resorted,
        vec![
            rows[2].clone(),
            rows[0].clone(),
            rows[1].clone(),
            rows[3].clone(),
        ],
    );
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    assert_eq!(p.panes[0].sort, ASCENDING);
    // Within the interval of a running calculation, nothing is re-sorted...
    assert!(apply(&mut p, measured(navigation, scan, 1, 10, 0)).is_empty());
    // ...and its end re-sorts once for every size that arrived meanwhile.
    assert_eq!(apply(&mut p, finished(navigation, scan)), [ASCENDING]);
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    let current = NavigationId(p.panes[0].generation);
    finish(
        &mut p,
        PaneId::Left,
        current,
        vec![
            rows[0].clone(),
            rows[2].clone(),
            rows[1].clone(),
            rows[3].clone(),
        ],
    );
    assert_eq!(p.panes[0].folder_sizes.len(), 2, "sizes survive re-sorts");
}

#[test]
fn sizes_that_leave_the_order_unchanged_or_another_sort_never_resort() {
    let (mut p, _, rows) = sizes_panel(&["a", "b"]);
    sort_by_size(&mut p, &rows);
    let (navigation, scan) = start_scan(&mut p);
    // "a" measured: sized folders lead, and "a" already does.
    assert!(apply(&mut p, measured(navigation, scan, 1, 7, 0)).is_empty());
    assert!(apply(&mut p, finished(navigation, scan)).is_empty());
    // A running or cancelled total has no size to sort by.
    let (navigation, scan) = start_scan(&mut p);
    let unfinished = Event::FolderSizeUpdated {
        pane: PaneId::Left,
        navigation,
        scan,
        entry: EntryId(2),
        size: FolderSize {
            bytes: 1,
            ..Default::default()
        },
    };
    assert!(apply(&mut p, unfinished).is_empty());
    assert!(apply(&mut p, finished(navigation, scan)).is_empty());

    let (mut p, _, _) = sizes_panel(&["a", "b"]);
    let (navigation, scan) = start_scan(&mut p);
    assert_eq!(p.panes[0].sort.column, SortColumn::Name);
    assert!(apply(&mut p, measured(navigation, scan, 2, 7, 0)).is_empty());
    assert!(apply(&mut p, finished(navigation, scan)).is_empty());
}

#[test]
fn a_busy_queue_retries_the_resort_with_the_next_event_without_an_error() {
    let (mut p, _, rows) = sizes_panel(&["a", "b"]);
    sort_by_size(&mut p, &rows);
    let (navigation, scan) = start_scan(&mut p);
    p.apply_with_sender(measured(navigation, scan, 2, 7, 0), &|_| false);
    assert_eq!(p.panes[0].error, "");
    assert!(!p.panes[0].loading);
    assert_eq!(apply(&mut p, finished(navigation, scan)), [ASCENDING]);
}

/// An archive folder sorted by Size follows its member-folder totals the same way.
#[test]
fn an_archive_panel_resorts_when_member_folder_sizes_change_its_order() {
    use super::archives::{archive_pane_of, node};
    let rows = vec![
        node(2, "a", EntryKind::Directory),
        node(5, "b", EntryKind::Directory),
        node(3, "file.txt", EntryKind::File),
    ];
    let mut p = archive_pane_of(rows.clone());
    let location = p.panes[0].archive.as_ref().unwrap().location.clone();
    let relist = |p: &mut Presenter, rows: Vec<Entry>| {
        let entries: Arc<[Entry]> = rows.into();
        p.apply(Event::ArchiveListed {
            pane: PaneId::Left,
            navigation: NavigationId(p.panes[0].generation),
            result: Ok(ArchiveListing {
                location: location.clone(),
                index: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
            }),
        });
    };
    assert!(p.sort(PaneId::Left, SortColumn::Size, &|_| true));
    relist(&mut p, rows.clone());
    p.panes[0].cursor = Some(EntryId(2));
    let snapshot = |version, entry| {
        Event::ArchiveSizes(Box::new(ArchiveSizeEvent::Snapshot {
            pane: PaneId::Left,
            scan: None,
            sizes: Arc::new(ArchiveSizeSnapshot::new(
                42,
                version,
                HashMap::from([(
                    VirtualEntryId { mount: 42, entry },
                    FolderSize {
                        bytes: 9,
                        finished: true,
                        ..Default::default()
                    },
                )]),
            )),
        }))
    };
    let relists = |p: &mut Presenter, event| {
        let sent = RefCell::new(Vec::new());
        p.apply_with_sender(event, &|command| {
            sent.borrow_mut().push(command);
            true
        });
        sent.into_inner()
            .into_iter()
            .filter(|command| {
                matches!(
                    command,
                    Command::ArchiveNavigate {
                        action: ArchiveAction::List(_),
                        ..
                    }
                )
            })
            .count()
    };
    // "a" measured: it already leads the folders.
    assert_eq!(relists(&mut p, snapshot(1, 2)), 0);
    // Only "b" measured: it moves before the unmeasured "a".
    assert_eq!(relists(&mut p, snapshot(2, 5)), 1);
    relist(
        &mut p,
        vec![rows[1].clone(), rows[0].clone(), rows[2].clone()],
    );
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    assert_eq!(p.panes[0].error, "");
}
