use super::{failure, *};
use std::{
    collections::HashMap,
    ffi::OsString,
    time::{SystemTime, UNIX_EPOCH},
};
use unicase::UniCase;

#[derive(Hash, PartialEq, Eq)]
enum Name {
    Raw(OsString),
    Folded(String),
}

#[derive(Default)]
struct Group {
    rows: [Vec<usize>; 2],
}

fn checkpoint(request: &CompareRequest, cancelled: &dyn Fn() -> bool) -> Result<(), FsError> {
    if cancelled() {
        Err(FsError::from_io(
            "compare directories",
            &request.sides[0].source.path,
            io::Error::new(io::ErrorKind::Interrupted, "Comparison cancelled."),
        ))
    } else {
        Ok(())
    }
}

/// Whole seconds, rounded down also for dates before the Unix epoch.
fn seconds(time: SystemTime) -> i128 {
    match time.duration_since(UNIX_EPOCH) {
        Ok(value) => i128::from(value.as_secs()),
        Err(error) => {
            let value = error.duration();
            -i128::from(value.as_secs()) - i128::from(value.subsec_nanos() != 0)
        }
    }
}

pub(super) fn compare(
    request: Arc<CompareRequest>,
    cancelled: &dyn Fn() -> bool,
) -> Result<CompareResult, FsError> {
    checkpoint(&request, cancelled)?;
    if request.options.time_tolerance_seconds > 86_400 {
        return Err(failure(
            &request,
            "Time tolerance must be at most 86400 seconds.",
        ));
    }
    let mut groups = HashMap::<(bool, Name), Group>::new();
    let mut entries = [Vec::new(), Vec::new()];
    let mut bits = [Vec::new(), Vec::new()];
    for (side, capture) in request.sides.iter().enumerate() {
        let count = capture.view.by_id.len();
        bits[side] = vec![0; count.div_ceil(64)];
        // Reject malformed or ambiguous projections before indexing any row.
        SelectionSnapshot::from_bits(capture.view.clone(), bits[side].clone())
            .map_err(|error| failure(&request, error))?;
        entries[side].resize(count, None);
        let source_count = capture
            .tree
            .as_ref()
            .map_or(capture.source.entries.len(), |tree| tree.rows.len());
        for source_row in 0..source_count {
            let entry = source_entry(capture, source_row);
            if entry.virtual_node.is_some() {
                return Err(failure(
                    &request,
                    "Directory comparison requires native entries.",
                ));
            }
            if source_row.is_multiple_of(256) {
                checkpoint(&request, cancelled)?;
            }
            let Some(id) = capture.view.by_path.get(&entry.path) else {
                if capture.view.by_id.contains_key(&entry.id) {
                    return Err(failure(
                        &request,
                        "A displayed identity refers to multiple source entries.",
                    ));
                }
                continue;
            };
            if *id != entry.id {
                return Err(failure(
                    &request,
                    "A displayed path has an ambiguous identity.",
                ));
            }
            let row = capture.view.by_id[id];
            if entries[side][row].replace(source_row).is_some() {
                return Err(failure(
                    &request,
                    "A displayed path refers to multiple source entries.",
                ));
            }
            if capture.tree.as_ref().is_some_and(|tree| {
                let row = &tree.rows[source_row];
                row.depth != 0 || !row.selectable || row.context_only
            }) {
                continue;
            }
            let directory = matches!(entry.kind, EntryKind::Directory | EntryKind::DirectoryLink);
            if directory && !request.options.unmatched_directories {
                continue;
            }
            let name = if request.options.case_sensitive {
                Name::Raw(entry.name.clone())
            } else if let Some(name) = entry.name.to_str() {
                Name::Folded(UniCase::unicode(name).to_folded_case())
            } else {
                Name::Raw(entry.name.clone())
            };
            groups.entry((directory, name)).or_default().rows[side].push(row);
        }
        if entries[side].iter().any(Option::is_none) {
            return Err(failure(
                &request,
                "The displayed projection is not part of its captured directory.",
            ));
        }
    }
    let mut unknown = 0;
    let mut ambiguous = 0;
    for (number, ((directory, _), group)) in groups.into_iter().enumerate() {
        if number.is_multiple_of(256) {
            checkpoint(&request, cancelled)?;
        }
        let mut select = |side: usize, row: usize| {
            bits[side][row / 64] |= 1 << (row % 64);
        };
        if group.rows.iter().any(|rows| rows.len() > 1) {
            ambiguous += 1;
            for (side, rows) in group.rows.iter().enumerate() {
                for row in rows {
                    select(side, *row);
                }
            }
            continue;
        }
        let (Some(left), Some(right)) = (group.rows[0].first(), group.rows[1].first()) else {
            for (side, rows) in group.rows.iter().enumerate() {
                for row in rows {
                    select(side, *row);
                }
            }
            continue;
        };
        if directory {
            continue;
        }
        let a = source_entry(&request.sides[0], entries[0][*left].unwrap());
        let b = source_entry(&request.sides[1], entries[1][*right].unwrap());
        let mut missing = a.kind != EntryKind::File || b.kind != EntryKind::File;
        if request.options.by_size {
            match (a.metadata.size, b.metadata.size) {
                (Some(a), Some(b)) if a > b => select(0, *left),
                (Some(a), Some(b)) if b > a => select(1, *right),
                (Some(_), Some(_)) => (),
                _ => missing = true,
            }
        }
        if request.options.by_modified {
            match (a.metadata.modified, b.metadata.modified) {
                (Some(a), Some(b)) => {
                    let delta = seconds(a) - seconds(b);
                    if delta.unsigned_abs() > u128::from(request.options.time_tolerance_seconds) {
                        if delta > 0 {
                            select(0, *left);
                        } else {
                            select(1, *right);
                        }
                    }
                }
                _ => missing = true,
            }
        }
        if missing {
            unknown += 1;
            select(0, *left);
            select(1, *right);
        }
    }
    checkpoint(&request, cancelled)?;
    let [left, right] = bits;
    let selections = [left, right]
        .into_iter()
        .enumerate()
        .map(|(side, bits)| {
            SelectionSnapshot::from_bits(request.sides[side].view.clone(), bits)
                .map(Arc::new)
                .map_err(|error| failure(&request, error))
        })
        .collect::<Result<Vec<_>, _>>()?;
    checkpoint(&request, cancelled)?;
    let [left, right]: [Arc<SelectionSnapshot>; 2] = selections.try_into().unwrap();
    let selections = [left, right];
    let mut tree_replacements = [None, None];
    for (slot, side) in request.sides.iter().enumerate() {
        if let Some(tree) = &side.tree {
            let selection =
                TreeSelectionSnapshot::from_bits(tree.clone(), selections[slot].bits().to_vec())
                    .map_err(|error| failure(&request, error))?;
            tree_replacements[slot] = Some(Arc::new(
                TreeSelectionReplacement::from_snapshot(&selection)
                    .map_err(|error| failure(&request, error))?,
            ));
        }
    }
    Ok(CompareResult {
        request,
        selections,
        tree_replacements,
        unknown,
        ambiguous,
    })
}

fn source_entry(side: &CompareSide, position: usize) -> &Entry {
    side.tree.as_ref().map_or_else(
        || &side.source.entries[position],
        |tree| &tree.rows[position].entry,
    )
}
