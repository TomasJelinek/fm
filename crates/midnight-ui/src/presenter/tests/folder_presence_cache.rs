use super::*;

#[test]
fn malformed_presence_receipts_leave_the_matching_request_available() {
    let mut p = ready();
    let request = request(&mut p);
    for values in [
        vec![(EntryId(1), FolderPresence::Empty); FolderPresenceRequest::MAX_VISIBLE + 1],
        vec![(EntryId(1), FolderPresence::Empty); request.visible.len()],
        vec![
            (EntryId(1), FolderPresence::Empty),
            (EntryId(999), FolderPresence::HasFolders),
            (EntryId(3), FolderPresence::HasFolders),
        ],
    ] {
        p.apply(Event::FolderPresence(Box::new(FolderPresenceSnapshot {
            root: request.root.clone(),
            revision: request.revision,
            values: values.into(),
        })));
        assert_eq!(
            p.panes[0].root_presence_disclosure(&p.panes[0].entries[0]),
            1
        );
        assert!(!request.cancel.is_cancelled());
    }
    p.apply(Event::FolderPresence(result(&request)));
    assert_eq!(
        p.panes[0].root_presence_disclosure(&p.panes[0].entries[0]),
        5
    );
}

#[test]
fn presence_cache_eviction_and_new_root_keep_old_results_from_reappearing() {
    let mut p = ready();
    let path = p.panes[0].path.clone();
    let navigation = begin(&mut p, PaneId::Left, path.to_str().unwrap());
    finish(
        &mut p,
        PaneId::Left,
        navigation,
        (0..1300)
            .map(|id| {
                Entry::new(
                    EntryId(id),
                    path.join(format!("folder{id}")),
                    EntryKind::Directory,
                )
            })
            .collect(),
    );
    let offset = p.panes[0].parent_offset();
    let mut last = None;
    for first in (0..1300).step_by(FolderPresenceRequest::MAX_VISIBLE) {
        p.panes[0].set_tree_viewport((
            offset + first,
            offset + (first + FolderPresenceRequest::MAX_VISIBLE).min(1300),
        ));
        let request = request(&mut p);
        p.apply(Event::FolderPresence(result(&request)));
        last = Some(request);
    }
    assert_eq!(
        p.panes[0].root_presence_disclosure(&p.panes[0].entries[0]),
        1,
        "old entries leave the bounded display cache"
    );
    assert_eq!(
        p.panes[0].root_presence_disclosure(&p.panes[0].entries[1299]),
        3
    );
    let old = last.unwrap();
    let entries = p.panes[0].entries.to_vec();
    let navigation = begin(&mut p, PaneId::Left, path.to_str().unwrap());
    finish(&mut p, PaneId::Left, navigation, entries);
    p.panes[0].set_tree_viewport((offset + 1280, offset + 1300));
    let newer = request(&mut p);
    p.apply(Event::FolderPresence(result(&old)));
    assert_eq!(
        p.panes[0].root_presence_disclosure(&p.panes[0].entries[1299]),
        1
    );
    p.apply(Event::FolderPresence(result(&newer)));
    assert_eq!(
        p.panes[0].root_presence_disclosure(&p.panes[0].entries[1299]),
        3
    );
    assert!(p.panes[0].tree_view().is_none());
}
