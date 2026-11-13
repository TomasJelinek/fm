//! Presentation must track each tree occurrence rather than the enclosing scan.
use super::*;
use std::collections::{HashMap, HashSet};

#[test]
fn tree_partial_totals_keep_warnings_but_only_unfinished_occurrences_are_highlighted() {
    for pane in [PaneId::Left, PaneId::Right] {
        let path = std::env::current_dir()
            .unwrap()
            .join("size-presentation-tree");
        let mut p = Presenter::new(path.clone(), path.clone());
        let navigation = std::cell::Cell::new(None);
        assert!(p.navigate(pane, path.clone(), None, &|command| {
            if let Command::Navigate { navigation: id, .. }
            | Command::Refresh { navigation: id, .. } = command
            {
                navigation.set(Some(id));
            }
            true
        }));
        let navigation = navigation.get().unwrap();
        let entries: Arc<[Entry]> = vec![Entry::new(
            EntryId(42),
            path.join("folder 資料"),
            EntryKind::Directory,
        )]
        .into();
        p.apply(Event::ListingFinished {
            pane,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries: entries.clone(),
        });
        let root = TreeRootSnapshot {
            pane,
            navigation,
            entries,
            index: p.panes[pane.index()].index.clone(),
        };
        let identity = p.panes[pane.index()].row_identity(&root.entries[0]);
        p.disclose_tree(pane, &identity, &|_| true);
        let stamp = TreeViewStamp {
            pane,
            navigation: root.navigation,
            session: TreeSessionId(5),
            revision: TreeRevision(1),
        };
        let rows = (0..2)
            .map(|index| {
                let path = if index == 0 {
                    root.entries[0].path.clone()
                } else {
                    root.entries[0].path.join("unfinished child")
                };
                let entry = Entry::new(EntryId(7 + index), path, EntryKind::Directory);
                TreeDisplayRow {
                    id: TreeRowId(entry.id.0),
                    entry,
                    parent: (index == 1).then_some(TreeRowId(7)),
                    depth: index as usize,
                    disclosure: if index == 0 {
                        TreeDisclosure::Expanded
                    } else {
                        TreeDisclosure::Unknown
                    },
                    expanded: index == 0,
                    selectable: true,
                    navigable: true,
                    context_only: false,
                    error: None,
                    error_details: None,
                    original_root_entry: (index == 0).then_some(EntryId(42)),
                    prefix_end: 0,
                    search_match: false,
                }
            })
            .collect();
        let view = Arc::new(TreeProjection::new(root, stamp, TreeLayoutRevision(1), rows).unwrap());
        let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
        let publication = TreePublication::new(view, SelectionRevision(0), selection).unwrap();
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(publication))),
            &|_| true,
        );
        let sizes = Arc::new(
            TreeSizeSnapshot::new(
                stamp,
                1,
                HashMap::from([
                    (
                        TreeRowId(7),
                        FolderSize {
                            bytes: 4096,
                            errors: 1,
                            finished: true,
                            ..Default::default()
                        },
                    ),
                    (
                        TreeRowId(8),
                        FolderSize {
                            bytes: 2048,
                            errors: 1,
                            ..Default::default()
                        },
                    ),
                ]),
                HashSet::from([TreeRowId(8)]),
            )
            .unwrap(),
        );
        p.apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Sizes(TreeSizeEvent::Snapshot {
                sizes,
            }))),
            &|_| true,
        );
        let model = Rows::new(Rc::new(RefCell::new(p)), pane);
        let values: Vec<_> = (0..model.row_count())
            .filter_map(|index| model.row_data(index))
            .filter(|row| !row.parent_entry)
            .collect();
        assert_eq!(values.len(), 2, "the expanded child must be presented");
        assert_eq!(values[0].size_state, 3);
        assert!(!values[0].size_running);
        assert!(values[0].size.as_str().starts_with("≈ "));
        assert!(values[0].size_hint.contains("Completed with warnings"));
        assert_eq!(values[1].size_state, 3);
        assert!(values[1].size_running);
        assert!(values[1].size_hint.contains("Calculating"));
    }
}
