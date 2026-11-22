//! Provider identities are opaque IDs; never derive native member paths.
use midnight_core::*;
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

pub(crate) fn error(message: impl Into<String>) -> FsError {
    FsError::from_io(
        "update archive selection",
        std::path::Path::new(""),
        std::io::Error::other(message.into()),
    )
}

#[cfg(test)]
thread_local! {
    /// Entries and selected members `apply` visited on this thread, for the tests that
    /// bound its work.
    pub(crate) static VISITED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
fn visited(_count: usize) {
    #[cfg(test)]
    VISITED.with(|visited| visited.set(visited.get() + _count as u64));
}

pub(crate) fn apply(
    entries: &[Entry],
    view: &DirectoryIndex,
    selected: Arc<HashSet<EntryId>>,
    totals: ArchiveSelectionTotals,
    action: ArchiveSelectionAction,
) -> Result<ArchiveSelectionResult, FsError> {
    if let ArchiveSelectionAction::Remove(ids) = &action {
        return Ok(remove(entries, view, selected, totals, ids));
    }
    visited(selected.len());
    let mut result = (*selected).clone();
    match action {
        ArchiveSelectionAction::Remove(_) => unreachable!("removed above"),
        ArchiveSelectionAction::Toggle(id) => {
            if view.by_id.contains_key(&id) && !result.remove(&id) {
                result.insert(id);
            }
        }
        ArchiveSelectionAction::Bulk(SelectionOperation::Clear) => result.clear(),
        ArchiveSelectionAction::Bulk(operation) => {
            if operation == SelectionOperation::FilesOnly {
                result.clear();
            }
            for id in view.by_id.keys() {
                if operation.selects(view.is_directory(*id), result.contains(id)) {
                    result.insert(*id);
                } else {
                    result.remove(id);
                }
            }
        }
        ArchiveSelectionAction::Range {
            first,
            last,
            additive,
        } => {
            if !additive {
                result.clear();
            }
            for (id, row) in &view.by_id {
                if *row >= first && *row <= last {
                    result.insert(*id);
                }
            }
        }
        ArchiveSelectionAction::Ranges { ranges, mode } => {
            if ranges.source_len() != view.by_id.len() {
                return Err(error(
                    "Selection rectangle no longer matches its captured archive view",
                ));
            }
            if mode == SelectionMode::Replace {
                result.clear();
            }
            for (id, row) in &view.by_id {
                if ranges.contains(*row) {
                    if mode == SelectionMode::Toggle && !result.insert(*id) {
                        result.remove(id);
                    } else {
                        result.insert(*id);
                    }
                }
            }
        }
        ArchiveSelectionAction::Mask(mask) => {
            // Also compiled by the standalone archive selection tests.
            let matcher = midnight_platform_common::filter::CompiledFilter::new(&mask.filter())
                .map_err(error)?;
            // Displayed members only; their listed names are the provider captions.
            for entry in entries
                .iter()
                .filter(|entry| view.by_id.contains_key(&entry.id))
            {
                let directory =
                    matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink);
                if !mask.includes(directory) || !matcher.matches(&entry.name) {
                    continue;
                }
                if mask.select {
                    result.insert(entry.id);
                } else {
                    result.remove(&entry.id);
                }
            }
        }
    }
    visited(entries.len());
    let mut totals = ArchiveSelectionTotals::default();
    for entry in entries.iter().filter(|entry| result.contains(&entry.id)) {
        totals.change(entry.kind, entry.metadata.size, true);
    }
    Ok(ArchiveSelectionResult {
        selected: Arc::new(result),
        totals,
    })
}

/// A copy deselects its copied members a batch at a time (WIN-8). The edit owns
/// `selected` unless the panel or a copy still shares it: then it is copied once (copy on
/// write), otherwise changed in place. Each removed member's kind and length come from
/// the view, and its share is subtracted from `totals`, so a round costs O(batch).
fn remove(
    entries: &[Entry],
    view: &DirectoryIndex,
    mut selected: Arc<HashSet<EntryId>>,
    mut totals: ArchiveSelectionTotals,
    ids: &HashSet<EntryId>,
) -> ArchiveSelectionResult {
    if Arc::strong_count(&selected) > 1 {
        visited(selected.len());
    }
    let set = Arc::make_mut(&mut selected);
    visited(ids.len());
    // Members the view leaves out (a filter hides them), indexed once per edit.
    let mut hidden: Option<HashMap<EntryId, (EntryKind, Option<u64>)>> = None;
    for id in ids {
        if !set.remove(id) {
            continue;
        }
        let member = match view.by_id.get(id) {
            // The index keeps regular-file lengths only; any other non-folder counts
            // as a file of unknown length, as `change` counted it when it was added.
            Some(_) if view.is_directory(*id) => Some((EntryKind::Directory, None)),
            Some(row) => Some((
                EntryKind::File,
                view.file_sizes.get(*row).copied().flatten(),
            )),
            None => hidden
                .get_or_insert_with(|| {
                    visited(entries.len());
                    entries
                        .iter()
                        .map(|entry| (entry.id, (entry.kind, entry.metadata.size)))
                        .collect()
                })
                .get(id)
                .copied(),
        };
        // A member outside the listing was never counted in the totals.
        if let Some((kind, size)) = member {
            totals.change(kind, size, false);
        }
    }
    ArchiveSelectionResult { selected, totals }
}

#[cfg(test)]
mod removal_cost {
    use super::*;

    /// Manual measurement (WIN-8): deselecting copied members 64 at a time from a
    /// selection of 100,001, as the panel does while a selection is copied out.
    #[test]
    #[ignore = "manual measurement"]
    fn removing_copied_members_from_a_large_archive_selection() {
        let entries: Vec<_> = (0..100_001u64)
            .map(|index| {
                Entry::from_archive(ArchiveNode {
                    modified: None,
                    id: VirtualEntryId {
                        mount: 9,
                        entry: index + 2,
                    },
                    parent: 1,
                    raw_name: format!("{index:06}.txt").into_bytes().into(),
                    display_name: format!("{index:06}.txt"),
                    kind: EntryKind::File,
                    size: Some(1),
                    readable: true,
                    access: Default::default(),
                })
            })
            .collect();
        let view = DirectoryIndex::from_entries(&entries);
        let mut selected: Arc<HashSet<EntryId>> =
            Arc::new(entries.iter().map(|entry| entry.id).collect());
        let mut totals = ArchiveSelectionTotals::default();
        for entry in &entries {
            totals.change(entry.kind, entry.metadata.size, true);
        }
        let started = std::time::Instant::now();
        let mut edits = 0;
        for chunk in entries.chunks(64).take(200) {
            let ids: HashSet<EntryId> = chunk.iter().map(|entry| entry.id).collect();
            let result = apply(
                &entries,
                &view,
                selected,
                totals,
                ArchiveSelectionAction::Remove(Arc::new(ids)),
            )
            .unwrap();
            selected = result.selected;
            totals = result.totals;
            edits += 1;
        }
        let per_edit = started.elapsed().as_secs_f64() * 1000.0 / edits as f64;
        println!(
            "PERF archive deselection: {per_edit:.2} ms per edit of 64 from about 100,000; \
             {:.0} s for all 100,001 members",
            per_edit * (100_001.0 / 64.0) / 1000.0
        );
    }
}

#[cfg(test)]
mod removal_work {
    use super::*;

    fn members(count: u64) -> Vec<Entry> {
        (0..count)
            .map(|index| {
                Entry::from_archive(ArchiveNode {
                    modified: None,
                    id: VirtualEntryId {
                        mount: 9,
                        entry: index + 2,
                    },
                    parent: 1,
                    raw_name: format!("{index:06}.txt").into_bytes().into(),
                    display_name: format!("{index:06}.txt"),
                    kind: EntryKind::File,
                    size: Some(index % 7),
                    readable: true,
                    access: Default::default(),
                })
            })
            .collect()
    }

    /// Deselecting 100,001 copied members in rounds of 64 (WIN-8) visits O(n) entries in
    /// total, not O(n) per round: each round changes the edit's own selection in place
    /// and subtracts the removed members from the totals, which stay exact.
    #[test]
    fn removing_a_large_selection_in_rounds_does_linear_work_with_exact_totals() {
        const COUNT: u64 = 100_001;
        let entries = members(COUNT);
        let view = DirectoryIndex::from_entries(&entries);
        let mut selected: Arc<HashSet<EntryId>> =
            Arc::new(entries.iter().map(|entry| entry.id).collect());
        let mut totals = ArchiveSelectionTotals::default();
        for entry in &entries {
            totals.change(entry.kind, entry.metadata.size, true);
        }
        let before = VISITED.with(std::cell::Cell::get);
        for (round, chunk) in entries.chunks(64).enumerate() {
            let ids: HashSet<EntryId> = chunk.iter().map(|entry| entry.id).collect();
            let result = apply(
                &entries,
                &view,
                selected,
                totals,
                ArchiveSelectionAction::Remove(Arc::new(ids)),
            )
            .unwrap();
            (selected, totals) = (result.selected, result.totals);
            let left = &entries[((round + 1) * 64).min(entries.len())..];
            if round % 500 == 0 {
                assert_eq!(selected.len(), left.len());
                assert_eq!(totals.files(), left.len());
                assert_eq!(
                    totals.file_bytes(),
                    Some(left.iter().map(|entry| entry.metadata.size.unwrap()).sum())
                );
            }
        }
        let work = VISITED.with(std::cell::Cell::get) - before;
        assert!(selected.is_empty() && totals == ArchiveSelectionTotals::default());
        assert!(
            work <= 2 * COUNT,
            "{work} entries visited for {COUNT} removals; at most {}",
            2 * COUNT
        );
    }

    /// A selection that the panel still shares is copied once, not changed under it.
    #[test]
    fn a_shared_selection_is_copied_and_left_unchanged() {
        let entries = members(10);
        let view = DirectoryIndex::from_entries(&entries);
        let shared: Arc<HashSet<EntryId>> =
            Arc::new(entries.iter().map(|entry| entry.id).collect());
        let mut totals = ArchiveSelectionTotals::default();
        for entry in &entries {
            totals.change(entry.kind, entry.metadata.size, true);
        }
        let result = apply(
            &entries,
            &view,
            shared.clone(),
            totals,
            ArchiveSelectionAction::Remove(Arc::new(HashSet::from([entries[3].id]))),
        )
        .unwrap();
        assert_eq!(shared.len(), 10, "the panel's selection changed");
        assert_eq!(result.selected.len(), 9);
        assert!(!result.selected.contains(&entries[3].id));
        assert_eq!(result.totals.files(), 9);
        assert_eq!(
            result.totals.file_bytes(),
            Some((0..10u64).map(|index| index % 7).sum::<u64>() - 3)
        );
    }
}
