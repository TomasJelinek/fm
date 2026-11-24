use super::*;
use std::cell::RefCell;

fn fixture() -> (Presenter, Arc<FindFeedSizeRequest>) {
    let root = std::env::temp_dir();
    let mut presenter = Presenter::new(root.clone(), root.clone());
    presenter.preferences.values.size_display = SizeDisplay::Bytes;
    let id = presenter.create_find(PaneId::Left).unwrap();
    let session = presenter.finds.sessions.get_mut(&id).unwrap();
    session.results = [
        ("found/folder", EntryKind::Directory),
        ("elsewhere/file", EntryKind::File),
        ("another/folder", EntryKind::Directory),
    ]
    .into_iter()
    .enumerate()
    .map(|(row, (path, kind))| {
        Arc::new(FindResult {
            id: FindResultId(row as u64 + 1),
            entry: FindEntry {
                path: root.join(path),
                kind,
                metadata: EntryMetadata::default(),
                identity: TransferIdentity {
                    device: 1,
                    object: [row as u8; 16],
                },
                revision: FindRevision([0; 6]),
            },
            duplicate_group: None,
            duplicate_group_size: 0,
            archive: None,
        })
    })
    .collect();
    presenter.feed_find_results(id).unwrap();
    let request = RefCell::new(None);
    presenter.folder_sizes(PaneId::Left, &|command| {
        let Command::ComputeFindFeedSizes(input) = command else {
            panic!()
        };
        *request.borrow_mut() = Some(input);
        true
    });
    (presenter, request.into_inner().unwrap())
}

fn updated(request: &FindFeedSizeRequest, bytes: u64) -> FindFeedSizeEvent {
    FindFeedSizeEvent {
        pane: request.pane,
        id: request.id,
        rows_serial: request.rows_serial,
        update: FindFeedSizeUpdate::Updated {
            entry: EntryId(1),
            size: FolderSize {
                bytes,
                ..Default::default()
            },
        },
    }
}

#[test]
fn fed_folder_sizes_reject_foreign_tags_and_keep_partial_totals_after_completion() {
    let (mut p, request) = fixture();
    let mut stale = updated(&request, 999);
    stale.rows_serial += 1;
    p.apply_find_feed_size(stale);
    assert!(p.feed(PaneId::Left).unwrap().sizes.values.is_empty());
    let mut stale = updated(&request, 999);
    stale.id.0 += 1;
    p.apply_find_feed_size(stale);
    assert!(p.feed(PaneId::Left).unwrap().sizes.values.is_empty());
    p.apply_find_feed_size(updated(&request, 9));
    p.apply_find_feed_size(FindFeedSizeEvent {
        pane: request.pane,
        id: request.id,
        rows_serial: request.rows_serial,
        update: FindFeedSizeUpdate::Finished {
            cancelled: true,
            error: None,
        },
    });
    p.apply_find_feed_size(updated(&request, 999));
    let feed = p.feed(PaneId::Left).unwrap();
    assert_eq!(
        feed.sizes.folder_size_text(&feed.entries[0], &p).as_deref(),
        Some("≈ 9")
    );
    assert!(!feed.sizes.running);
}

#[test]
fn file_only_marks_are_frozen_without_falling_back_to_unmarked_or_hidden_folders() {
    let (mut p, _) = fixture();
    p.folder_sizes(PaneId::Left, &|_| panic!("cancel uses the captured token"));
    p.feed_click(PaneId::Left, 1, true, false);
    let captured = RefCell::new(None);
    p.folder_sizes(PaneId::Left, &|command| {
        let Command::ComputeFindFeedSizes(request) = command else {
            panic!()
        };
        *captured.borrow_mut() = Some(request);
        true
    });
    let request = captured.into_inner().unwrap();
    p.feed_click(PaneId::Left, 1, true, false);
    assert_eq!(&*request.selected, &[false, true, false]);
    assert!(request.roots().unwrap().is_empty());
    assert_eq!(
        request.entries[0].path,
        std::env::temp_dir().join("found/folder")
    );
}

#[test]
fn leaving_fed_results_cancels_their_owned_scan_before_retirement() {
    let (mut p, request) = fixture();
    assert!(!request.cancel.is_cancelled());
    p.feed_close(PaneId::Left);
    assert!(request.cancel.is_cancelled());
    p.apply_find_feed_size(updated(&request, 999));
    assert!(p.feed(PaneId::Left).is_none());
}

#[test]
fn space_measures_only_the_focused_folder_and_advances_while_insert_and_files_only_toggle() {
    use crate::presenter::find::feed::FeedAction;
    let (mut p, _) = fixture();
    p.folder_sizes(PaneId::Left, &|_| panic!("cancel is token-only"));
    p.feed_click(PaneId::Left, 2, true, false);
    p.feed_click(PaneId::Left, 0, false, false);
    let requests = RefCell::new(Vec::new());
    let send = |command| {
        let Command::ComputeFindFeedSizes(request) = command else {
            panic!("no hidden-folder action")
        };
        requests.borrow_mut().push(request);
        true
    };
    p.feed_action(PaneId::Left, FeedAction::Toggle { advance: false }, &send);
    assert_eq!(
        requests.borrow().len(),
        1,
        "Space calculates the focused folder"
    );
    let request = requests.borrow()[0].clone();
    let roots = request.roots().unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].path, std::env::temp_dir().join("found/folder"));
    let feed = p.feed(PaneId::Left).unwrap();
    assert_eq!(feed.cursor, 1);
    assert_eq!(&**feed.selected, &[true, false, true]);
    // The focused row is now a file: Space marks and advances without sizing.
    p.feed_action(PaneId::Left, FeedAction::Toggle { advance: false }, &send);
    assert_eq!(requests.borrow().len(), 1);
    assert_eq!(p.feed(PaneId::Left).unwrap().cursor, 2);
    // Insert on the other folder only changes its mark.
    p.feed_action(PaneId::Left, FeedAction::Toggle { advance: true }, &send);
    assert_eq!(requests.borrow().len(), 1);
    assert_eq!(
        &**p.feed(PaneId::Left).unwrap().selected,
        &[true, true, false]
    );
}
