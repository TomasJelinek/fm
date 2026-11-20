//! Include subfolders and Compare contents: bounded native walks of same-named
//! folder pairs and byte-by-byte reads of equal-size file pairs. Every call into
//! the native search service runs on the comparison's blocking worker.
use super::*;
use std::{
    collections::HashMap,
    mem::size_of,
    time::{Duration, Instant},
};

/// Entries examined inside compared folders, both sides together.
pub(super) const MAX_NODES: u64 = 1_000_000;
/// Folder levels below a displayed folder pair.
pub(super) const MAX_DEPTH: usize = 128;
/// Listings, pending folder pairs and content candidates retained at once.
pub(super) const MAX_RETAINED_BYTES: usize = 64 * 1024 * 1024;
/// File pairs waiting to be read byte by byte.
pub(super) const MAX_CANDIDATES: usize = 100_000;
const READ_BYTES: usize = 128 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(250);

pub(super) struct Candidate {
    /// The displayed rows a difference selects: the files themselves, or the
    /// displayed folder pair that contains them.
    pub(super) rows: [usize; 2],
    pub(super) paths: [PathBuf; 2],
    /// Facts captured while walking. Displayed rows are checked against their
    /// displayed size when opened instead.
    pub(super) captured: Option<Box<[FindEntry; 2]>>,
    pub(super) size: u64,
}
impl Candidate {
    fn cost(&self) -> usize {
        size_of::<Self>()
            + self
                .paths
                .iter()
                .map(|path| path.as_os_str().len())
                .sum::<usize>()
            + self.captured.as_ref().map_or(0, |captured| {
                captured
                    .iter()
                    .map(|entry| size_of::<FindEntry>() + entry.path.as_os_str().len())
                    .sum()
            })
    }
}

enum Child {
    Entry(FindEntry),
    /// A link, special file or mount boundary: matched by name, never followed.
    Skipped(PathBuf),
}
impl Child {
    fn path(&self) -> &Path {
        match self {
            Self::Entry(entry) => &entry.path,
            Self::Skipped(path) => path,
        }
    }
    fn directory(&self) -> bool {
        matches!(self, Self::Entry(entry) if entry.kind == EntryKind::Directory)
    }
    fn cost(&self) -> usize {
        size_of::<Self>() + self.path().as_os_str().len()
    }
}

#[derive(Default)]
struct Listing {
    children: Vec<Child>,
    /// An entry could not be read; the folder pair cannot be proven equal.
    incomplete: bool,
    retained: usize,
}

pub(super) struct Walk<'a, 'b> {
    deep: &'a mut Deep<'b>,
    options: &'a CompareOptions,
    candidates: Vec<Candidate>,
    progress: CompareProgress,
    last_report: Instant,
    retained: usize,
    unknown: usize,
    ambiguous: usize,
    skipped: u64,
    errors: u64,
    limited: bool,
    content_differences: usize,
    read_files: u64,
    diagnostics: Vec<FsError>,
    buffers: [Vec<u8>; 2],
}

impl<'a, 'b> Walk<'a, 'b> {
    pub(super) fn new(deep: &'a mut Deep<'b>, options: &'a CompareOptions) -> Self {
        Self {
            deep,
            options,
            candidates: Vec::new(),
            progress: CompareProgress::default(),
            last_report: Instant::now(),
            retained: 0,
            unknown: 0,
            ambiguous: 0,
            skipped: 0,
            errors: 0,
            limited: false,
            content_differences: 0,
            read_files: 0,
            diagnostics: Vec::new(),
            buffers: [Vec::new(), Vec::new()],
        }
    }

    /// Queues one file pair for reading. False when the candidate budget is
    /// spent: the caller selects the pair as not verified.
    pub(super) fn push_candidate(&mut self, candidate: Candidate) -> bool {
        if self.candidates.len() >= MAX_CANDIDATES || !self.reserve(candidate.cost()) {
            self.limited = true;
            self.unknown += 1;
            return false;
        }
        self.candidates.push(candidate);
        true
    }

    fn reserve(&mut self, bytes: usize) -> bool {
        match self.retained.checked_add(bytes) {
            Some(total) if total <= MAX_RETAINED_BYTES => {
                self.retained = total;
                true
            }
            _ => {
                self.limited = true;
                false
            }
        }
    }

    fn error(&mut self, error: FsError) {
        self.errors += 1;
        if self.diagnostics.len() < MAX_COMPARE_DIAGNOSTICS {
            self.diagnostics.push(error);
        }
    }

    fn cancelled(&self, path: &Path) -> Result<(), FsError> {
        if self.deep.cancel.is_cancelled() {
            Err(interrupted(path))
        } else {
            Ok(())
        }
    }

    fn tick(&mut self) {
        if self.last_report.elapsed() >= PROGRESS_INTERVAL {
            self.report();
        }
    }

    fn report(&mut self) {
        (self.deep.report)(&self.progress);
        self.last_report = Instant::now();
    }

    /// Walks one displayed folder pair. Returns the sides that contain a
    /// difference; `mark` receives each differing entry (or unverified folder).
    pub(super) fn folder_pair(
        &mut self,
        rows: [usize; 2],
        paths: [&Path; 2],
        mark: &mut dyn FnMut(usize, &Path),
    ) -> Result<[bool; 2], FsError> {
        let mut contains = [false; 2];
        let mut differ = |side: usize, path: &Path, contains: &mut [bool; 2]| {
            contains[side] = true;
            mark(side, path);
        };
        let root = [paths[0].to_owned(), paths[1].to_owned()];
        let mut stack = Vec::new();
        if self.reserve(pair_cost(&root)) {
            stack.push((root, 1usize));
        } else {
            for (side, path) in root.iter().enumerate() {
                differ(side, path, &mut contains);
            }
        }
        while let Some((pair, depth)) = stack.pop() {
            self.retained -= pair_cost(&pair);
            self.cancelled(&pair[0])?;
            if self.limited || depth > MAX_DEPTH {
                // Unfinished folders are never reported as equal.
                self.limited = true;
                for (side, path) in pair.iter().enumerate() {
                    differ(side, path, &mut contains);
                }
                continue;
            }
            self.progress.folders += 1;
            let listed = [self.list(&pair[0])?, self.list(&pair[1])?];
            let [Some(left), Some(right)] = listed else {
                for (side, path) in pair.iter().enumerate() {
                    differ(side, path, &mut contains);
                }
                for listing in listed.into_iter().flatten() {
                    self.retained -= listing.retained;
                }
                continue;
            };
            let incomplete = left.incomplete || right.incomplete;
            let lists = [left, right];
            let mut groups = HashMap::<(bool, Name), [Vec<usize>; 2]>::new();
            for (side, listing) in lists.iter().enumerate() {
                for (index, child) in listing.children.iter().enumerate() {
                    let name = child.path().file_name().unwrap_or_default();
                    groups
                        .entry((
                            child.directory(),
                            Name::new(name, self.options.case_sensitive),
                        ))
                        .or_default()[side]
                        .push(index);
                }
            }
            for group in groups.into_values() {
                self.cancelled(&pair[0])?;
                if group.iter().any(|indices| indices.len() > 1) {
                    self.ambiguous += 1;
                    for (side, indices) in group.iter().enumerate() {
                        for index in indices {
                            differ(side, lists[side].children[*index].path(), &mut contains);
                        }
                    }
                    continue;
                }
                let (left, right) = match (group[0].first(), group[1].first()) {
                    (Some(left), Some(right)) => {
                        (&lists[0].children[*left], &lists[1].children[*right])
                    }
                    _ => {
                        // Present on one side only, folders and links included.
                        for (side, indices) in group.iter().enumerate() {
                            for index in indices {
                                differ(side, lists[side].children[*index].path(), &mut contains);
                            }
                        }
                        continue;
                    }
                };
                match (left, right) {
                    (Child::Entry(a), Child::Entry(b))
                        if a.kind == EntryKind::Directory && b.kind == EntryKind::Directory =>
                    {
                        let next = [a.path.clone(), b.path.clone()];
                        if self.reserve(pair_cost(&next)) {
                            stack.push((next, depth + 1));
                        } else {
                            for (side, path) in next.iter().enumerate() {
                                differ(side, path, &mut contains);
                            }
                        }
                    }
                    (Child::Entry(a), Child::Entry(b))
                        if a.kind == EntryKind::File && b.kind == EntryKind::File =>
                    {
                        let verdict = judge(self.options, [&a.metadata, &b.metadata], true);
                        if verdict.unknown {
                            self.unknown += 1;
                            differ(0, &a.path, &mut contains);
                            differ(1, &b.path, &mut contains);
                            continue;
                        }
                        if verdict.select[0] {
                            differ(0, &a.path, &mut contains);
                        }
                        if verdict.select[1] {
                            differ(1, &b.path, &mut contains);
                        }
                        if verdict.read_contents
                            && !self.push_candidate(Candidate {
                                rows,
                                paths: [a.path.clone(), b.path.clone()],
                                captured: Some(Box::new([a.clone(), b.clone()])),
                                size: a.metadata.size.unwrap_or(0),
                            })
                        {
                            differ(0, &a.path, &mut contains);
                            differ(1, &b.path, &mut contains);
                        }
                    }
                    (Child::Skipped(_), Child::Skipped(_)) => self.skipped += 1,
                    // A link opposite a file, or another kind mismatch.
                    _ => {
                        differ(0, left.path(), &mut contains);
                        differ(1, right.path(), &mut contains);
                    }
                }
            }
            if incomplete {
                for (side, path) in pair.iter().enumerate() {
                    differ(side, path, &mut contains);
                }
            }
            for listing in lists {
                self.retained -= listing.retained;
            }
            self.tick();
        }
        Ok(contains)
    }

    /// Lists one folder. None when it cannot be opened (recorded as an error).
    fn list(&mut self, path: &Path) -> Result<Option<Listing>, FsError> {
        let mut source = match self.deep.find.open_find_source(path, self.deep.cancel) {
            Ok(source) if source.entry().kind == EntryKind::Directory => source,
            Ok(_) => {
                self.error(compare_error(
                    path,
                    "A compared folder is no longer a folder.",
                ));
                return Ok(None);
            }
            Err(error) => {
                self.cancelled(path)?;
                self.error(error);
                return Ok(None);
            }
        };
        let mut listing = Listing::default();
        loop {
            self.cancelled(path)?;
            let child = match source.next_child(self.deep.cancel) {
                Ok(Some(child)) => child,
                Ok(None) => break,
                Err(error) => {
                    self.cancelled(path)?;
                    self.error(error);
                    listing.incomplete = true;
                    break;
                }
            };
            if self.progress.entries >= MAX_NODES {
                self.limited = true;
                listing.incomplete = true;
                break;
            }
            self.progress.entries += 1;
            let child = match child {
                FindChild::Entry(source) => {
                    let entry = source.entry();
                    if matches!(entry.kind, EntryKind::File | EntryKind::Directory) {
                        Child::Entry(entry.clone())
                    } else {
                        Child::Skipped(entry.path.clone())
                    }
                }
                FindChild::Skipped { path, .. } => Child::Skipped(path),
                FindChild::Failed(error) => {
                    self.cancelled(path)?;
                    self.error(error);
                    listing.incomplete = true;
                    continue;
                }
            };
            let cost = child.cost();
            if !self.reserve(cost) {
                listing.incomplete = true;
                break;
            }
            listing.retained += cost;
            listing.children.push(child);
            self.tick();
        }
        Ok(Some(listing))
    }

    /// Reads every queued pair. `mark` receives the pairs whose bytes differ or
    /// that could not be read completely.
    pub(super) fn contents(
        &mut self,
        mark: &mut dyn FnMut([usize; 2], [&Path; 2]),
    ) -> Result<(), FsError> {
        if self.candidates.is_empty() {
            return Ok(());
        }
        let candidates = std::mem::take(&mut self.candidates);
        self.progress.phase = ComparePhase::Contents;
        self.progress.content_files_total = candidates.len() as u64;
        self.progress.content_bytes_total = candidates
            .iter()
            .map(|candidate| candidate.size.saturating_mul(2))
            .fold(0, u64::saturating_add);
        self.buffers = [vec![0; READ_BYTES], vec![0; READ_BYTES]];
        self.report();
        for candidate in &candidates {
            self.cancelled(&candidate.paths[0])?;
            match self.same_content(candidate) {
                Ok(true) => self.read_files += 1,
                Ok(false) => {
                    self.read_files += 1;
                    self.content_differences += 1;
                    mark(candidate.rows, [&candidate.paths[0], &candidate.paths[1]]);
                }
                Err(error) => {
                    // Only this comparison's cancellation ends it; any other
                    // failure leaves the pair selected as not verified.
                    self.cancelled(&candidate.paths[0])?;
                    self.error(error);
                    self.unknown += 1;
                    mark(candidate.rows, [&candidate.paths[0], &candidate.paths[1]]);
                }
            }
            self.progress.content_files += 1;
            self.tick();
        }
        self.buffers = [Vec::new(), Vec::new()];
        Ok(())
    }

    fn same_content(&mut self, candidate: &Candidate) -> Result<bool, FsError> {
        let open = |side: usize| -> Result<Box<dyn FindSource>, FsError> {
            let path = &candidate.paths[side];
            let source = self.deep.find.open_find_source(path, self.deep.cancel)?;
            let entry = source.entry();
            let same = entry.kind == EntryKind::File
                && match &candidate.captured {
                    Some(captured) => entry.matches_capture(&captured[side]),
                    None => entry.metadata.size == Some(candidate.size),
                };
            if same {
                Ok(source)
            } else {
                Err(compare_error(
                    path,
                    "The file changed after it was listed; compare again.",
                ))
            }
        };
        let mut sources = [open(0)?, open(1)?];
        let cancel = self.deep.cancel;
        let mut total = 0u64;
        let equal = loop {
            let [left, right] = &mut self.buffers;
            let [first, second] = &mut sources;
            let a = fill(first.as_mut(), left, cancel)?;
            let b = fill(second.as_mut(), right, cancel)?;
            self.progress.content_bytes += (a + b) as u64;
            if a != b || left[..a] != right[..b] {
                break false;
            }
            if a == 0 {
                break true;
            }
            total += a as u64;
            if total > candidate.size {
                return Err(compare_error(
                    &candidate.paths[0],
                    "The file grew while it was read; compare again.",
                ));
            }
            self.tick();
        };
        if equal && total != candidate.size {
            return Err(compare_error(
                &candidate.paths[0],
                "The file shrank while it was read; compare again.",
            ));
        }
        // A file written during the read cannot prove either answer.
        for source in &sources {
            source.revalidate(cancel)?;
        }
        Ok(equal)
    }

    pub(super) fn finish(self, result: &mut CompareResult) {
        result.unknown += self.unknown;
        result.ambiguous += self.ambiguous;
        result.skipped += self.skipped;
        result.errors = self.errors;
        result.limited = self.limited;
        result.content_differences = self.content_differences;
        result.content_files = self.read_files;
        result.content_bytes = self.progress.content_bytes;
        result.diagnostics = self.diagnostics;
    }
}

fn pair_cost(pair: &[PathBuf; 2]) -> usize {
    pair.iter()
        .map(|path| path.as_os_str().len())
        .sum::<usize>()
        + size_of::<([PathBuf; 2], usize)>()
}

/// Fills `buffer` unless the file ends first; short provider reads are normal.
fn fill(
    source: &mut dyn FindSource,
    buffer: &mut [u8],
    cancel: &Cancellation,
) -> Result<usize, FsError> {
    let mut filled = 0;
    while filled < buffer.len() {
        if cancel.is_cancelled() {
            return Err(interrupted(&source.entry().path));
        }
        let read = source.read_chunk(&mut buffer[filled..], cancel)?;
        if read > buffer.len() - filled {
            return Err(compare_error(
                &source.entry().path,
                "The search service returned an invalid read count.",
            ));
        }
        if read == 0 {
            break;
        }
        filled += read;
    }
    Ok(filled)
}
