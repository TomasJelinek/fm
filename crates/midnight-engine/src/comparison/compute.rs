use super::{failure, *};
use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
use unicase::UniCase;

#[path = "walk.rs"]
mod walk;
use walk::{Candidate, Walk};

#[derive(Hash, PartialEq, Eq)]
pub(super) enum Name {
    Raw(OsString),
    Folded(String),
}
impl Name {
    pub(super) fn new(name: &std::ffi::OsStr, case_sensitive: bool) -> Self {
        if case_sensitive {
            Self::Raw(name.to_owned())
        } else if let Some(name) = name.to_str() {
            Self::Folded(UniCase::unicode(name).to_folded_case())
        } else {
            Self::Raw(name.to_owned())
        }
    }
}

#[derive(Default)]
struct Group {
    rows: [Vec<usize>; 2],
}

/// Native access for Compare contents and Include subfolders. Metadata-only
/// comparisons of the displayed rows need none.
pub(super) struct Deep<'a> {
    pub(super) find: &'a dyn FindService,
    pub(super) cancel: &'a Cancellation,
    pub(super) report: &'a mut dyn FnMut(&CompareProgress),
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

/// How the enabled metadata criteria judge one same-named file pair.
pub(super) struct Verdict {
    /// The side each criterion selects (the larger and/or the newer file).
    pub(super) select: [bool; 2],
    /// Requested metadata is unavailable, or an entry is not a regular file.
    pub(super) unknown: bool,
    /// Compare contents: both sizes are known and equal and nothing else differs.
    pub(super) read_contents: bool,
}

pub(super) fn judge(
    options: &CompareOptions,
    entries: [&EntryMetadata; 2],
    files: bool,
) -> Verdict {
    let mut verdict = Verdict {
        select: [false; 2],
        unknown: !files,
        read_contents: false,
    };
    let [a, b] = entries;
    if options.by_size {
        match (a.size, b.size) {
            (Some(a), Some(b)) if a > b => verdict.select[0] = true,
            (Some(a), Some(b)) if b > a => verdict.select[1] = true,
            (Some(_), Some(_)) => (),
            _ => verdict.unknown = true,
        }
    }
    if options.by_modified {
        match (a.modified, b.modified) {
            (Some(a), Some(b)) => {
                let delta = seconds(a) - seconds(b);
                if delta.unsigned_abs() > u128::from(options.time_tolerance_seconds) {
                    if delta > 0 {
                        verdict.select[0] = true;
                    } else {
                        verdict.select[1] = true;
                    }
                }
            }
            _ => verdict.unknown = true,
        }
    }
    if options.by_content && !verdict.unknown && verdict.select == [false; 2] {
        match (a.size, b.size) {
            (Some(a), Some(b)) if a == b => verdict.read_contents = true,
            // Different lengths cannot hold the same bytes; size is not a criterion here.
            (Some(_), Some(_)) => verdict.select = [true; 2],
            _ => verdict.unknown = true,
        }
    }
    verdict
}

pub(super) fn compare(
    request: Arc<CompareRequest>,
    cancelled: &dyn Fn() -> bool,
    mut deep: Option<Deep<'_>>,
) -> Result<CompareResult, FsError> {
    checkpoint(&request, cancelled)?;
    if request.options.time_tolerance_seconds > 86_400 {
        return Err(failure(
            &request,
            "Time tolerance must be at most 86400 seconds.",
        ));
    }
    let options = &request.options;
    if (options.by_content || options.recursive) && deep.is_none() {
        return Err(failure(
            &request,
            "Comparing contents or subfolders needs the native search service, which is unavailable.",
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
            // Include subfolders pairs folders to walk them; unmatched ones always count.
            if directory && !options.unmatched_directories && !options.recursive {
                continue;
            }
            groups
                .entry((directory, Name::new(&entry.name, options.case_sensitive)))
                .or_default()
                .rows[side]
                .push(row);
        }
        if entries[side].iter().any(Option::is_none) {
            return Err(failure(
                &request,
                "The displayed projection is not part of its captured directory.",
            ));
        }
    }
    let mut marks = Marks {
        request: &request,
        entries: &entries,
        bits,
    };
    let mut unknown = 0;
    let mut ambiguous = 0;
    let mut skipped = 0;
    let mut folders = Vec::new();
    let mut walk = deep.as_mut().map(|deep| Walk::new(deep, options));
    for (number, ((directory, _), group)) in groups.into_iter().enumerate() {
        if number.is_multiple_of(256) {
            checkpoint(&request, cancelled)?;
        }
        if group.rows.iter().any(|rows| rows.len() > 1) {
            ambiguous += 1;
            for (side, rows) in group.rows.iter().enumerate() {
                for row in rows {
                    marks.row(side, *row);
                }
            }
            continue;
        }
        let (Some(&left), Some(&right)) = (group.rows[0].first(), group.rows[1].first()) else {
            for (side, rows) in group.rows.iter().enumerate() {
                for row in rows {
                    marks.row(side, *row);
                }
            }
            continue;
        };
        let a = marks.entry(0, left);
        let b = marks.entry(1, right);
        if directory {
            if options.recursive {
                if a.kind == EntryKind::Directory && b.kind == EntryKind::Directory {
                    folders.push(([left, right], [a.path.clone(), b.path.clone()]));
                } else {
                    // Folder links are matched by name and never followed.
                    skipped += 1;
                }
            }
            continue;
        }
        let files = a.kind == EntryKind::File && b.kind == EntryKind::File;
        let verdict = judge(options, [&a.metadata, &b.metadata], files);
        if verdict.unknown {
            unknown += 1;
            marks.row(0, left);
            marks.row(1, right);
            continue;
        }
        for (side, row) in [left, right].into_iter().enumerate() {
            if verdict.select[side] {
                marks.row(side, row);
            }
        }
        if verdict.read_contents {
            let queued = walk.as_mut().is_some_and(|walk| {
                walk.push_candidate(Candidate {
                    rows: [left, right],
                    paths: [a.path.clone(), b.path.clone()],
                    captured: None,
                    size: a.metadata.size.unwrap_or(0),
                })
            });
            if !queued {
                marks.row(0, left);
                marks.row(1, right);
            }
        }
    }
    checkpoint(&request, cancelled)?;
    let mut differing_folders = 0;
    if let Some(walk) = &mut walk {
        for (rows, paths) in &folders {
            let contains = walk.folder_pair(*rows, [&paths[0], &paths[1]], &mut |side, path| {
                marks.path(side, path)
            })?;
            for (side, contains) in contains.into_iter().enumerate() {
                if contains {
                    marks.row(side, rows[side]);
                }
            }
        }
        walk.contents(&mut |rows, paths| {
            for side in 0..2 {
                marks.row(side, rows[side]);
                marks.path(side, paths[side]);
            }
        })?;
        // Counted last: a content difference inside a folder also makes it differ.
        differing_folders = folders
            .iter()
            .filter(|(rows, _)| (0..2).any(|side| marks.is_marked(side, rows[side])))
            .count();
    }
    checkpoint(&request, cancelled)?;
    let Marks { bits, .. } = marks;
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
    let mut result = CompareResult {
        request: request.clone(),
        selections,
        tree_replacements,
        unknown,
        ambiguous,
        differing_folders,
        content_differences: 0,
        content_files: 0,
        content_bytes: 0,
        skipped,
        errors: 0,
        limited: false,
        diagnostics: Vec::new(),
    };
    if let Some(walk) = walk {
        walk.finish(&mut result);
    }
    Ok(result)
}

/// The selection bits of both panels: displayed rows, and in an expanded tree the
/// displayed subfolders on the way to a difference.
struct Marks<'a> {
    request: &'a CompareRequest,
    entries: &'a [Vec<Option<usize>>; 2],
    bits: [Vec<u64>; 2],
}
impl<'a> Marks<'a> {
    fn entry(&self, side: usize, row: usize) -> &'a Entry {
        let request: &'a CompareRequest = self.request;
        let source = self.entries[side][row].unwrap_or_default();
        source_entry(&request.sides[side], source)
    }
    fn row(&mut self, side: usize, row: usize) {
        self.bits[side][row / 64] |= 1 << (row % 64);
    }
    fn is_marked(&self, side: usize, row: usize) -> bool {
        self.bits[side][row / 64] & (1 << (row % 64)) != 0
    }
    /// Selects `path` and its ancestors that an expanded tree shows below the
    /// compared folder; flat panels show none of them.
    fn path(&mut self, side: usize, path: &Path) {
        let capture = &self.request.sides[side];
        let Some(tree) = &capture.tree else {
            return;
        };
        for ancestor in path.ancestors() {
            if ancestor == capture.source.path {
                break;
            }
            let Some(row) = capture
                .view
                .by_path
                .get(ancestor)
                .and_then(|id| capture.view.by_id.get(id))
                .copied()
            else {
                continue;
            };
            let Some(source) = self.entries[side].get(row).copied().flatten() else {
                continue;
            };
            let shown = &tree.rows[source];
            if shown.depth == 0 || !shown.selectable || shown.context_only {
                continue;
            }
            self.row(side, row);
        }
    }
}

fn source_entry(side: &CompareSide, position: usize) -> &Entry {
    side.tree.as_ref().map_or_else(
        || &side.source.entries[position],
        |tree| &tree.rows[position].entry,
    )
}

pub(super) fn interrupted(path: &Path) -> FsError {
    FsError::from_io(
        "compare directories",
        path,
        io::Error::new(io::ErrorKind::Interrupted, "Comparison cancelled."),
    )
}

/// The folder walk's node and depth limits.
#[cfg(test)]
pub(super) fn walk_limits() -> (u64, usize) {
    (walk::MAX_NODES, walk::MAX_DEPTH)
}

pub(super) fn compare_error(path: &Path, message: &str) -> FsError {
    FsError::from_io(
        "compare directories",
        path,
        io::Error::new(io::ErrorKind::InvalidData, message.to_owned()),
    )
}
