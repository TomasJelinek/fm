//! Reduce an immutable adopted tree to existing regular root files on the viewer worker.
use midnight_core::{DirectoryIndex, Entry, EntryKind, ViewerLaunch};
use std::{
    collections::HashSet,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub(crate) fn normalize(
    launch: &ViewerLaunch,
    closed: &AtomicBool,
) -> Result<ViewerLaunch, String> {
    let tree = launch.tree.as_ref().ok_or("Missing captured tree")?;
    let view = &tree.view;
    if launch.generated.is_some()
        || !Arc::ptr_eq(&launch.entries, &view.root_actions.entries)
        || !Arc::ptr_eq(&launch.view, &view.root_actions.index)
        || view.rows.len() > midnight_core::TreeProjection::MAX_ROWS
    {
        return Err("The viewer tree does not match its captured root.".into());
    }
    let focus = view
        .index
        .row_positions
        .get(&tree.focused)
        .and_then(|position| view.rows.get(*position))
        .filter(|row| row.id == tree.focused)
        .ok_or("The focused tree occurrence is unavailable.")?;
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut occurrences = HashSet::new();
    let mut bytes = 0usize;
    let mut focused = false;
    for (position, row) in view.rows.iter().enumerate() {
        if !occurrences.insert(row.id) || view.index.row_positions.get(&row.id) != Some(&position) {
            return Err("The tree occurrence index is inconsistent.".into());
        }
        if closed.load(Ordering::Acquire) {
            return Err("Viewer closed".into());
        }
        if row.entry.kind != EntryKind::File {
            continue;
        }
        // Descendant files are not an authority supplied by the current folder-only tree.
        if row.depth != 0 || row.parent.is_some() || row.original_root_entry.is_none() {
            return Err("The tree contains an unsupported file occurrence.".into());
        }
        if !row.selectable || row.context_only || row.error.is_some() {
            continue;
        }
        let id = row.original_root_entry.ok_or("Missing root alias")?;
        let entry = view
            .root_actions
            .index
            .by_id
            .get(&id)
            .and_then(|position| view.root_actions.entries.get(*position))
            .filter(|entry| entry.id == id && entry.kind == EntryKind::File)
            .ok_or("The tree file is absent from its captured root.")?;
        if !seen.insert(id) || !same_source(entry, &row.entry) {
            return Err("The tree file does not identify its captured root entry.".into());
        }
        match (&launch.archive, &entry.virtual_node) {
            (Some(archive), Some(node))
                if node.id.mount == archive.mount.id() && node.kind == EntryKind::File =>
            {
                if !node.readable {
                    continue;
                }
                if row.id == focus.id {
                    if archive.focused != node.id {
                        return Err("Archive tree focus changed".into());
                    }
                    focused = true;
                }
            }
            (None, None) if entry.native_path().is_some_and(|path| path.is_absolute()) => {
                if row.id == focus.id {
                    if launch.focused != entry.path {
                        return Err("Native tree focus changed".into());
                    }
                    focused = true;
                }
            }
            _ => return Err("The tree file belongs to another content provider.".into()),
        }
        bytes = bytes
            .saturating_add(
                entry
                    .path
                    .as_os_str()
                    .as_encoded_bytes()
                    .len()
                    .saturating_mul(8),
            )
            .saturating_add(entry.virtual_node.as_ref().map_or(0, |node| {
                node.raw_name
                    .len()
                    .saturating_add(node.display_name.len())
                    .saturating_mul(8)
            }))
            .saturating_add(512);
        if bytes > 8 * 1024 * 1024 {
            return Err("The captured file order exceeds the 8 MiB viewer budget.".into());
        }
        entries.push(entry.clone());
    }
    if !focused {
        return Err("Focus a selectable regular root file before viewing.".into());
    }
    let mut normalized = launch.clone();
    normalized.tree = None;
    normalized.view = Arc::new(DirectoryIndex::from_entries(&entries));
    normalized.entries = entries.into();
    Ok(normalized)
}

fn same_source(left: &Entry, right: &Entry) -> bool {
    left.kind == right.kind
        && left.network_node.is_none()
        && right.network_node.is_none()
        && match (&left.virtual_node, &right.virtual_node) {
            (None, None) => left.path == right.path,
            (Some(left_node), Some(right_node)) => {
                left.path.as_os_str().is_empty()
                    && right.path.as_os_str().is_empty()
                    && left_node.id == right_node.id
                    && left_node.parent == right_node.parent
                    && left_node.raw_name == right_node.raw_name
                    && left_node.kind == right_node.kind
                    && left_node.readable == right_node.readable
            }
            _ => false,
        }
}
