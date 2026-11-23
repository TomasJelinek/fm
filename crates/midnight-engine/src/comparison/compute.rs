//! The displayed rows of both panels, compared by Open Salamander's Compare
//! Directories rules (docs/developer/COMPARE_DIRECTORIES.md): files and folders are
//! matched separately by name; a file on one side only, the larger or newer file, or
//! both files for different attributes or contents are selected; folders are
//! compared only with Include subfolders (or selected when on one side only), and a
//! folder pair that differs anywhere inside is selected on both sides.
use super::{
    contents::{Candidate, Expected, Origin, Pool, READERS},
    failure,
    rules::{Criteria, Name},
    walk::{FindLister, Lister, Status, Top, TransferLister, Walk},
    *,
};
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    path::Path,
};

#[derive(Default)]
struct Group {
    rows: [Vec<usize>; 2],
}

/// Native access for Compare contents and Include subfolders. Metadata-only
/// comparisons of the displayed rows need none.
pub(super) struct Deep<'a> {
    pub(super) find: &'a dyn FindService,
    /// Lists folders from their records without opening every child; the search
    /// service lists them otherwise.
    pub(super) transfer: Option<&'a dyn TransferService>,
    pub(super) cancel: &'a Cancellation,
    pub(super) report: &'a mut dyn FnMut(&CompareProgress),
}

fn checkpoint(request: &CompareRequest, cancelled: &dyn Fn() -> bool) -> Result<(), FsError> {
    if cancelled() {
        Err(interrupted(&request.sides[0].source.path))
    } else {
        Ok(())
    }
}

/// Both panels show the same folder (Salamander refuses this too).
fn same_folder(a: &Path, b: &Path) -> bool {
    if cfg!(windows) {
        a.as_os_str().to_string_lossy().to_lowercase()
            == b.as_os_str().to_string_lossy().to_lowercase()
    } else {
        a == b
    }
}

pub(super) fn compare(
    request: Arc<CompareRequest>,
    cancelled: &dyn Fn() -> bool,
    mut deep: Option<Deep<'_>>,
) -> Result<CompareResult, FsError> {
    checkpoint(&request, cancelled)?;
    let criteria = Criteria::new(&request.options).map_err(|error| failure(&request, error))?;
    let options = &request.options;
    if same_folder(&request.sides[0].source.path, &request.sides[1].source.path) {
        return Err(failure(
            &request,
            "Both panels show the same folder. Open another folder in one of them.",
        ));
    }
    if (options.by_content || options.recursive) && deep.is_none() {
        return Err(failure(
            &request,
            "Comparing contents or subfolders needs the native search service, which is unavailable.",
        ));
    }
    let directories = options.recursive || options.unmatched_directories;
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
            // Folders are compared only with Include subfolders, or selected when
            // unmatched; otherwise they are left out, names included (as in Salamander).
            if (directory && !directories) || criteria.ignored(&entry.name, directory) {
                continue;
            }
            groups
                .entry((directory, criteria.name(&entry.name)))
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
    let mut marks = options.recursive.then(|| {
        CompareMarks::new(
            [
                request.sides[0].source.path.to_path_buf(),
                request.sides[1].source.path.to_path_buf(),
            ],
            options.case_sensitive,
        )
    });
    let mut selection = Selection {
        request: &request,
        entries: &entries,
        bits,
    };
    let mut unknown = 0;
    let mut ambiguous = 0;
    let mut skipped = 0;
    let mut dst_shifts = 0;
    let mut tops = Vec::new();
    let mut top_rows = Vec::new();
    let mut files = Vec::new();
    let mut groups: Vec<_> = groups.into_iter().collect();
    // Displayed order does not matter for the outcome; name order keeps the walk's
    // progress predictable.
    groups.sort_by(|a, b| a.0.cmp(&b.0));
    for (number, ((directory, _), group)) in groups.into_iter().enumerate() {
        if number.is_multiple_of(256) {
            checkpoint(&request, cancelled)?;
        }

        let set = |marks: &mut Option<CompareMarks>,
                   key: Option<OsString>,
                   side: Option<usize>,
                   mark: CompareMark| {
            if let (Some(marks), Some(key)) = (marks.as_mut(), key) {
                for slot in side.map_or(0..2, |side| side..side + 1) {
                    marks.set(&key, slot, mark);
                }
            }
        };
        if group.rows.iter().any(|rows| rows.len() > 1) {
            ambiguous += 1;
            for (side, rows) in group.rows.iter().enumerate() {
                for row in rows {
                    selection.row(side, *row);
                    let key = top_key(&marks, &selection.entry(side, *row).name);
                    set(&mut marks, key, Some(side), CompareMark::Differs);
                }
            }
            continue;
        }
        let (Some(&left), Some(&right)) = (group.rows[0].first(), group.rows[1].first()) else {
            // Present on one side only.
            for (side, rows) in group.rows.iter().enumerate() {
                for row in rows {
                    selection.row(side, *row);
                    let key = top_key(&marks, &selection.entry(side, *row).name);
                    set(&mut marks, key, Some(side), CompareMark::OnlyHere);
                }
            }
            continue;
        };
        let a = selection.entry(0, left);
        let b = selection.entry(1, right);
        let pair_key = top_key(&marks, &a.name);
        if directory {
            match (a.kind, b.kind) {
                (EntryKind::Directory, EntryKind::Directory) => {
                    if criteria.folders_differ([&a.metadata, &b.metadata]) {
                        selection.row(0, left);
                        selection.row(1, right);
                        set(&mut marks, pair_key, None, CompareMark::Differs);
                    } else if options.recursive {
                        tops.push(Top {
                            paths: [a.path.clone(), b.path.clone()],
                            key: pair_key.unwrap_or_default(),
                        });
                        top_rows.push([left, right]);
                    }
                }
                // Folder links are matched by name and never followed.
                (EntryKind::DirectoryLink, EntryKind::DirectoryLink) => skipped += 1,
                // A link opposite a folder cannot be compared without following it.
                _ if options.recursive => {
                    selection.row(0, left);
                    selection.row(1, right);
                    set(&mut marks, pair_key, None, CompareMark::Differs);
                }
                _ => skipped += 1,
            }
            continue;
        }
        match (a.kind, b.kind) {
            (EntryKind::File, EntryKind::File) => (),
            (EntryKind::Symlink, EntryKind::Symlink) | (EntryKind::Other, EntryKind::Other) => {
                skipped += 1;
                continue;
            }
            _ => {
                selection.row(0, left);
                selection.row(1, right);
                set(&mut marks, pair_key, None, CompareMark::Differs);
                continue;
            }
        }
        let verdict = criteria.files([&a.metadata, &b.metadata]);
        if verdict.dst_shift {
            dst_shifts += 1;
        }
        if verdict.unknown {
            unknown += 1;
            selection.row(0, left);
            selection.row(1, right);
            set(&mut marks, pair_key, None, CompareMark::Unverified);
            continue;
        }
        for (side, row) in [left, right].into_iter().enumerate() {
            if verdict.select[side] {
                selection.row(side, row);
            }
        }
        if verdict.read_contents {
            files.push(Candidate {
                origin: Origin::Row([left, right]),
                key: pair_key,
                paths: [a.path.clone(), b.path.clone()],
                expected: [a, b].map(|entry| Expected {
                    size: entry.metadata.size.unwrap_or(0),
                    modified: None,
                }),
            });
        } else if verdict.differs() {
            set(&mut marks, pair_key, None, CompareMark::Differs);
        } else {
            set(&mut marks, pair_key, None, CompareMark::Same);
        }
    }
    checkpoint(&request, cancelled)?;
    let mut counts = None;
    let compared_folders = tops.len();
    let mut differing_folders = 0;
    if let Some(deep) = deep.as_mut()
        && (!tops.is_empty() || !files.is_empty())
    {
        let find_lister = FindLister(deep.find);
        let transfer_lister = deep.transfer.map(TransferLister);
        let lister: &dyn Lister = match &transfer_lister {
            Some(lister) => lister,
            None => &find_lister,
        };
        let cancel = deep.cancel;
        let find = deep.find;
        let report = &mut *deep.report;
        let outcomes = std::thread::scope(|scope| {
            let readers = if options.by_content { READERS } else { 0 };
            let pool = Pool::start(scope, Some(find), cancel, readers);
            let walk = Walk::new(&criteria, lister, cancel, cancelled, report, marks.as_mut());
            // Dropping the pool, also on an error, stops its readers before the
            // scope waits for them.
            walk.run(tops, files, &pool)
        })?;
        for (rows, status) in top_rows.iter().zip(&outcomes.tops) {
            if *status != Status::Same {
                differing_folders += 1;
                selection.row(0, rows[0]);
                selection.row(1, rows[1]);
            }
        }
        for (rows, key, status) in &outcomes.rows {
            let mark = match status {
                Status::Same => CompareMark::Same,
                Status::Differs => CompareMark::Differs,
                Status::Unverified => CompareMark::Unverified,
            };
            if *status != Status::Same {
                selection.row(0, rows[0]);
                selection.row(1, rows[1]);
            }
            if let (Some(marks), Some(key)) = (marks.as_mut(), key) {
                marks.set(key, 0, mark);
                marks.set(key, 1, mark);
            }
        }
        counts = Some(outcomes.counts);
    }
    checkpoint(&request, cancelled)?;
    let Selection { bits, .. } = selection;
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
    let counts = counts.unwrap_or_default();
    Ok(CompareResult {
        request: request.clone(),
        selections,
        tree_replacements,
        marks: marks.map(Arc::new),
        unknown: unknown + counts.unknown,
        ambiguous: ambiguous + counts.ambiguous,
        differing_folders,
        compared_folders,
        content_differences: counts.content_differences,
        content_files: counts.content_files,
        content_bytes: counts.content_bytes,
        walked_folders: counts.folders,
        walked_files: counts.files,
        dst_shifts: dst_shifts + counts.dst_shifts,
        skipped: skipped + counts.skipped,
        errors: counts.errors,
        limited: counts.limited,
        diagnostics: counts.diagnostics,
    })
}

fn top_key(marks: &Option<CompareMarks>, name: &OsStr) -> Option<OsString> {
    marks
        .as_ref()
        .map(|marks| marks.child_key(OsStr::new(""), name))
}

/// The selection bits of both panels' displayed rows.
struct Selection<'a> {
    request: &'a CompareRequest,
    entries: &'a [Vec<Option<usize>>; 2],
    bits: [Vec<u64>; 2],
}
impl<'a> Selection<'a> {
    fn entry(&self, side: usize, row: usize) -> &'a Entry {
        let request: &'a CompareRequest = self.request;
        let source = self.entries[side][row].unwrap_or_default();
        source_entry(&request.sides[side], source)
    }
    fn row(&mut self, side: usize, row: usize) {
        self.bits[side][row / 64] |= 1 << (row % 64);
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

pub(super) fn compare_error(path: &Path, message: &str) -> FsError {
    FsError::from_io(
        "compare directories",
        path,
        io::Error::new(io::ErrorKind::InvalidData, message.to_owned()),
    )
}
