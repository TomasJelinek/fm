//! Provider identities are opaque IDs; never derive native member paths.
use midnight_core::*;
use std::{collections::HashSet, sync::Arc};

pub(crate) fn error(message: impl Into<String>) -> FsError {
    FsError::from_io(
        "update archive selection",
        std::path::Path::new(""),
        std::io::Error::other(message.into()),
    )
}

pub(crate) fn apply(
    entries: &[Entry],
    view: &DirectoryIndex,
    selected: &HashSet<EntryId>,
    action: ArchiveSelectionAction,
) -> Result<ArchiveSelectionResult, FsError> {
    let mut result = selected.clone();
    match action {
        ArchiveSelectionAction::Toggle(id) => {
            if view.by_id.contains_key(&id) && !result.remove(&id) {
                result.insert(id);
            }
        }
        ArchiveSelectionAction::Bulk(SelectionOperation::Clear) => result.clear(),
        ArchiveSelectionAction::Bulk(operation) => {
            for id in view.by_id.keys() {
                if operation == SelectionOperation::Invert && result.contains(id) {
                    result.remove(id);
                } else {
                    result.insert(*id);
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
            if ranges.len() != view.by_id.len() {
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
    }
    let mut totals = ArchiveSelectionTotals::default();
    for entry in entries.iter().filter(|entry| result.contains(&entry.id)) {
        totals.change(entry.kind, entry.metadata.size, true);
    }
    Ok(ArchiveSelectionResult {
        selected: Arc::new(result),
        totals,
    })
}
