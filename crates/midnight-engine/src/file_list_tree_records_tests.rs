use super::*;
use std::sync::Arc;

fn archive_entry(id: u64, name: &str) -> Entry {
    let mut entry = Entry::from_archive(ArchiveNode {
        id: VirtualEntryId {
            mount: 99,
            entry: id,
        },
        parent: 1,
        raw_name: Arc::from(name.as_bytes()),
        display_name: name.into(),
        kind: EntryKind::Directory,
        size: None,
        readable: false,
        modified: None,
    });
    entry.id = EntryId(id);
    entry
}

fn row(entry: Entry, parent: Option<u64>, depth: usize, expanded: bool) -> TreeDisplayRow {
    TreeDisplayRow {
        id: TreeRowId(entry.id.0),
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
        navigable: true,
        context_only: false,
        error: None,
        error_details: None,
        original_root_entry: None,
        prefix_end: 0,
        search_match: true,
    }
}

fn capture(native_root: bool, fallback: u64) -> FileListCapture {
    let root = std::env::temp_dir().join("文档");
    let archive = root.join("pack.zip");
    let mut rows = Vec::new();
    if native_root {
        rows.push(row(
            Entry::new(EntryId(1), archive.clone(), EntryKind::File),
            None,
            0,
            true,
        ));
    }
    let depth = usize::from(native_root);
    for (id, name) in [(2, "first"), (4, "second")] {
        rows.push(row(
            archive_entry(id, name),
            native_root.then_some(1),
            depth,
            true,
        ));
        rows.push(row(
            archive_entry(id + 1, "same"),
            Some(id),
            depth + 1,
            false,
        ));
    }
    let entries: Arc<[Entry]> = rows
        .iter()
        .filter(|row| row.parent.is_none())
        .map(|row| row.entry.clone())
        .collect::<Vec<_>>()
        .into();
    let snapshot = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    };
    let view = Arc::new(
        TreeProjection::new(
            snapshot,
            TreeViewStamp {
                pane: PaneId::Left,
                navigation: NavigationId(1),
                session: TreeSessionId(1),
                revision: TreeRevision(1),
            },
            TreeLayoutRevision(1),
            rows,
        )
        .unwrap(),
    );
    FileListCapture {
        source: FileListSource::Tree {
            selection: Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap()),
            view,
            fallback: Some(TreeRowId(fallback)),
        },
        directory: if native_root { root } else { archive }
            .to_str()
            .unwrap()
            .into(),
    }
}

fn rendered(capture: &FileListCapture) -> String {
    let cancel = Cancellation::default();
    let entries = selected(capture, &cancel).unwrap();
    assert_eq!(entries.len(), 1);
    let record = captured_record(entries[0], capture, &BTreeMap::new(), &cancel).unwrap();
    FileListTemplate::parse("$(Drive)$(Path)$(FileName)")
        .unwrap()
        .render(&record, &[])
        .unwrap()
}

#[test]
fn virtual_tree_descendants_keep_each_duplicate_name_under_its_own_ancestors() {
    for (fallback, parent) in [(3, "first"), (5, "second")] {
        let capture = capture(false, fallback);
        let expected = std::env::temp_dir()
            .join("文档")
            .join("pack.zip")
            .join(parent)
            .join("same");
        assert_eq!(rendered(&capture), expected.to_str().unwrap());
    }
}

#[test]
fn archive_expanded_from_a_native_tree_includes_its_container_filename() {
    for (fallback, parent) in [(3, "first"), (5, "second")] {
        let capture = capture(true, fallback);
        let expected = std::env::temp_dir()
            .join("文档")
            .join("pack.zip")
            .join(parent)
            .join("same");
        assert_eq!(rendered(&capture), expected.to_str().unwrap());
    }
}
