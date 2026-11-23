//! Include subfolders and Compare contents: walks same-named folder pairs depth
//! first, compares what they hold by Salamander's rules, and hands file pairs that
//! must be read to a bounded pool of content readers (`contents.rs`) that run
//! meanwhile. Memory stays bounded: one folder pair's listings at a time, a pending
//! stack charged to a budget, one compact node per folder pair and the outcome
//! marks, which have their own budget.
use super::{
    compute::{compare_error, interrupted},
    contents::{Candidate, Expected, Origin, Outcome, Pool},
    rules::{Criteria, Name},
};
use midnight_core::*;
use std::{
    collections::HashMap,
    ffi::{OsStr, OsString},
    mem::size_of,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

/// Folder levels below a displayed folder pair.
pub(super) const MAX_DEPTH: usize = 256;
/// One folder pair's listings, both sides together.
pub(super) const MAX_LISTING_BYTES: usize = 32 * 1024 * 1024;
/// Folder pairs waiting on the walk's stack.
pub(super) const MAX_PENDING_BYTES: usize = 32 * 1024 * 1024;
/// Folder pair nodes and their keys.
pub(super) const MAX_NODE_BYTES: usize = 64 * 1024 * 1024;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(200);
const NONE: u32 = u32::MAX;

/// One child as its folder's listing describes it.
pub(super) enum Child {
    /// A regular file or an ordinary folder.
    Entry {
        name: OsString,
        directory: bool,
        metadata: EntryMetadata,
    },
    /// A link or a mount boundary: matched by name, never followed.
    Link(OsString),
    /// A device, pipe or socket: matched by name, never read.
    Special(OsString),
    /// The child could not be described; its folder cannot be proven equal.
    Failed(FsError),
}

/// Lists one native folder without following links.
pub(super) trait Lister: Sync {
    /// `visit` returns false to stop the listing early.
    fn list(
        &self,
        path: &Path,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(Child) -> bool,
    ) -> Result<(), FsError>;
}

/// A displayed folder pair to compare with everything inside.
pub(super) struct Top {
    pub(super) paths: [PathBuf; 2],
    pub(super) key: OsString,
}

/// How a compared item turned out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Status {
    Same,
    Differs,
    Unverified,
}

pub(super) struct Outcomes {
    /// One per `Top`, in order.
    pub(super) tops: Vec<Status>,
    /// Displayed file pairs read byte by byte: their candidate index and outcome.
    pub(super) rows: Vec<([usize; 2], Option<OsString>, Status)>,
    pub(super) counts: Counts,
}

#[derive(Default)]
pub(super) struct Counts {
    pub(super) unknown: usize,
    pub(super) ambiguous: usize,
    pub(super) skipped: u64,
    pub(super) errors: u64,
    pub(super) limited: bool,
    pub(super) dst_shifts: u64,
    pub(super) content_differences: usize,
    pub(super) content_files: u64,
    pub(super) content_bytes: u64,
    pub(super) folders: u64,
    pub(super) files: u64,
    pub(super) diagnostics: Vec<FsError>,
}

impl Counts {
    fn error(&mut self, error: FsError) {
        self.errors += 1;
        if self.diagnostics.len() < MAX_COMPARE_DIAGNOSTICS {
            self.diagnostics.push(error);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Same,
    Differs,
    Unverified,
}

/// One folder pair inside (or at) a displayed folder pair.
struct Node {
    parent: u32,
    state: State,
    /// Its key in the marks; None once the node budget is spent.
    key: Option<Box<OsStr>>,
}

struct Pending {
    node: u32,
    paths: [PathBuf; 2],
    depth: usize,
}
impl Pending {
    fn cost(&self) -> usize {
        size_of::<Self>()
            + self
                .paths
                .iter()
                .map(|p| p.as_os_str().len())
                .sum::<usize>()
    }
}

/// The comparison's deep part: the walk on this thread, contents on the pool.
pub(super) struct Walk<'a> {
    criteria: &'a Criteria,
    lister: &'a dyn Lister,
    cancel: &'a Cancellation,
    cancelled: &'a dyn Fn() -> bool,
    report: &'a mut dyn FnMut(&CompareProgress),
    marks: Option<&'a mut CompareMarks>,
    nodes: Vec<Node>,
    node_bytes: usize,
    pending_bytes: usize,
    counts: Counts,
    progress: CompareProgress,
    last_report: Instant,
    /// Content candidates sent and not yet answered.
    outstanding: u64,
    rows: Vec<([usize; 2], Option<OsString>, Status)>,
}

impl<'a> Walk<'a> {
    pub(super) fn new(
        criteria: &'a Criteria,
        lister: &'a dyn Lister,
        cancel: &'a Cancellation,
        cancelled: &'a dyn Fn() -> bool,
        report: &'a mut dyn FnMut(&CompareProgress),
        marks: Option<&'a mut CompareMarks>,
    ) -> Self {
        Self {
            criteria,
            lister,
            cancel,
            cancelled,
            report,
            marks,
            nodes: Vec::new(),
            node_bytes: 0,
            pending_bytes: 0,
            counts: Counts::default(),
            progress: CompareProgress::default(),
            last_report: Instant::now(),
            outstanding: 0,
            rows: Vec::new(),
        }
    }

    fn stopped(&self, path: &Path) -> Result<(), FsError> {
        if self.cancel.is_cancelled() || (self.cancelled)() {
            Err(interrupted(path))
        } else {
            Ok(())
        }
    }

    fn tick(&mut self, pool: &Pool) {
        if self.last_report.elapsed() >= PROGRESS_INTERVAL {
            self.report(pool);
        }
    }

    fn report(&mut self, pool: &Pool) {
        let shared = pool.progress();
        self.progress.content_bytes = shared.bytes;
        if self.progress.phase == ComparePhase::Contents {
            self.progress.current = shared.current.or(self.progress.current.take());
        }
        (self.report)(&self.progress);
        self.last_report = Instant::now();
    }

    /// Compares every displayed folder pair in `tops` and reads every displayed file
    /// pair in `files`, both by the pool.
    pub(super) fn run(
        mut self,
        tops: Vec<Top>,
        files: Vec<Candidate>,
        pool: &Pool,
    ) -> Result<Outcomes, FsError> {
        self.progress.top_total = tops.len() as u64;
        self.report(pool);
        for candidate in files {
            self.send(candidate, pool)?;
        }
        // Nodes 0..tops are the displayed folder pairs, in order; the stack pops
        // the first one first.
        for top in &tops {
            self.node(NONE, Some(&top.key));
        }
        let mut stack = Vec::new();
        for (index, top) in tops.iter().enumerate().rev() {
            let pending = Pending {
                node: index as u32,
                paths: top.paths.clone(),
                depth: 1,
            };
            self.pending_bytes += pending.cost();
            stack.push(pending);
        }
        while let Some(pair) = stack.pop() {
            self.pending_bytes -= pair.cost();
            self.stopped(&pair.paths[0])?;
            self.drain(pool, false)?;
            if self.top(pair.node) as u64 > self.progress.top_done {
                self.progress.top_done = self.top(pair.node) as u64;
            }
            self.progress.current = Some(Arc::new(pair.paths.clone()));
            if pair.depth > MAX_DEPTH {
                self.counts.limited = true;
                self.unverified(pair.node);
                continue;
            }
            self.folder_pair(&pair, &mut stack, pool)?;
            self.tick(pool);
        }
        self.progress.top_done = self.progress.top_total;
        self.progress.phase = ComparePhase::Contents;
        self.report(pool);
        pool.finish();
        while self.outstanding > 0 {
            self.stopped(&PathBuf::new())?;
            self.drain(pool, true)?;
            if self.outstanding > 0 && pool.closed() {
                return Err(compare_error(
                    &PathBuf::new(),
                    "The content readers ended before every file pair was read.",
                ));
            }
            self.tick(pool);
        }
        self.finish(tops.len(), pool)
    }

    fn top(&self, mut node: u32) -> u32 {
        while self.nodes[node as usize].parent != NONE {
            node = self.nodes[node as usize].parent;
        }
        node
    }

    fn node(&mut self, parent: u32, key: Option<&OsStr>) -> u32 {
        let cost = size_of::<Node>() + key.map_or(0, OsStr::len);
        let key = if self.node_bytes + cost <= MAX_NODE_BYTES {
            self.node_bytes += cost;
            key.map(Box::from)
        } else {
            None
        };
        self.nodes.push(Node {
            parent,
            state: State::Same,
            key,
        });
        (self.nodes.len() - 1) as u32
    }

    /// `node` contains a difference: it and every folder above it differ.
    fn differs(&mut self, mut node: u32) {
        self.progress.differences += 1;
        while node != NONE {
            let item = &mut self.nodes[node as usize];
            if item.state != State::Same {
                return;
            }
            item.state = State::Differs;
            node = item.parent;
        }
    }

    /// `node` could not be compared completely; the folders above it differ.
    fn unverified(&mut self, node: u32) {
        let parent = self.nodes[node as usize].parent;
        self.nodes[node as usize].state = State::Unverified;
        if parent != NONE {
            self.differs(parent);
        } else {
            self.progress.differences += 1;
        }
    }

    fn mark(&mut self, node: u32, name: &OsStr, side: Option<usize>, mark: CompareMark) {
        let Some(marks) = self.marks.as_deref_mut() else {
            return;
        };
        let Some(parent) = self.nodes[node as usize].key.as_deref() else {
            return;
        };
        let key = marks.child_key(parent, name);
        match side {
            Some(side) => {
                marks.set(&key, side, mark);
            }
            None => {
                marks.set(&key, 0, mark);
                marks.set(&key, 1, mark);
            }
        }
    }

    fn child_key(&self, node: u32, name: &OsStr) -> Option<OsString> {
        let marks = self.marks.as_deref()?;
        let parent = self.nodes[node as usize].key.as_deref()?;
        Some(marks.child_key(parent, name))
    }

    /// Lists both folders of `pair` and compares what they hold.
    fn folder_pair(
        &mut self,
        pair: &Pending,
        stack: &mut Vec<Pending>,
        pool: &Pool,
    ) -> Result<(), FsError> {
        self.progress.folders += 1;
        self.counts.folders += 1;
        let listed = [self.list(&pair.paths[0])?, self.list(&pair.paths[1])?];
        let [Some(left), Some(right)] = listed else {
            self.unverified(pair.node);
            return Ok(());
        };
        if left.incomplete || right.incomplete {
            // Everything listed is still compared; the folder itself stays unproven.
            self.unverified(pair.node);
        }
        let lists = [left.children, right.children];
        let mut groups = HashMap::<(bool, Name), [Vec<usize>; 2]>::new();
        for (side, children) in lists.iter().enumerate() {
            for (index, child) in children.iter().enumerate() {
                let directory = matches!(
                    child,
                    Child::Entry {
                        directory: true,
                        ..
                    }
                );
                if self.criteria.ignored(child_name(child), directory) {
                    continue;
                }
                groups
                    .entry((directory, self.criteria.name(child_name(child))))
                    .or_default()[side]
                    .push(index);
            }
        }
        for group in groups.into_values() {
            self.stopped(&pair.paths[0])?;
            self.compare_group(pair, &lists, group, stack, pool)?;
        }
        Ok(())
    }

    fn compare_group(
        &mut self,
        pair: &Pending,
        lists: &[Vec<Child>; 2],
        group: [Vec<usize>; 2],
        stack: &mut Vec<Pending>,
        pool: &Pool,
    ) -> Result<(), FsError> {
        let node = pair.node;
        if group.iter().any(|indices| indices.len() > 1) {
            self.counts.ambiguous += 1;
            self.differs(node);
            for (side, indices) in group.iter().enumerate() {
                for index in indices {
                    self.mark(
                        node,
                        child_name(&lists[side][*index]),
                        Some(side),
                        CompareMark::Differs,
                    );
                }
            }
            return Ok(());
        }
        let (left, right) = match (group[0].first(), group[1].first()) {
            (Some(left), Some(right)) => (&lists[0][*left], &lists[1][*right]),
            _ => {
                // Present on one side only, folders and links included.
                self.differs(node);
                for (side, indices) in group.iter().enumerate() {
                    for index in indices {
                        self.mark(
                            node,
                            child_name(&lists[side][*index]),
                            Some(side),
                            CompareMark::OnlyHere,
                        );
                    }
                }
                return Ok(());
            }
        };
        let name = child_name(left);
        match (left, right) {
            (
                Child::Entry {
                    directory: true,
                    metadata: a,
                    ..
                },
                Child::Entry {
                    directory: true,
                    metadata: b,
                    name: right_name,
                },
            ) => {
                if self.criteria.folders_differ([a, b]) {
                    self.differs(node);
                    self.mark(node, name, None, CompareMark::Differs);
                    return Ok(());
                }
                let key = self.child_key(node, name);
                let child = self.node(node, key.as_deref());
                let next = Pending {
                    node: child,
                    paths: [pair.paths[0].join(name), pair.paths[1].join(right_name)],
                    depth: pair.depth + 1,
                };
                let cost = next.cost();
                if self.pending_bytes + cost > MAX_PENDING_BYTES {
                    self.counts.limited = true;
                    self.unverified(child);
                } else {
                    self.pending_bytes += cost;
                    stack.push(next);
                }
            }
            (
                Child::Entry {
                    directory: false,
                    metadata: a,
                    ..
                },
                Child::Entry {
                    directory: false,
                    metadata: b,
                    name: right_name,
                },
            ) => {
                self.counts.files += 1;
                self.progress.files += 1;
                let verdict = self.criteria.files([a, b]);
                if verdict.dst_shift {
                    self.counts.dst_shifts += 1;
                }
                if verdict.unknown {
                    self.counts.unknown += 1;
                    self.differs(node);
                    self.mark(node, name, None, CompareMark::Unverified);
                } else if verdict.differs() {
                    self.differs(node);
                    self.mark(node, name, None, CompareMark::Differs);
                } else if verdict.read_contents {
                    let candidate = Candidate {
                        origin: Origin::Node(node),
                        key: self.child_key(node, name),
                        paths: [pair.paths[0].join(name), pair.paths[1].join(right_name)],
                        expected: [a, b].map(|metadata| Expected {
                            size: metadata.size.unwrap_or(0),
                            modified: metadata.modified,
                        }),
                    };
                    self.send(candidate, pool)?;
                }
            }
            (Child::Link(_), Child::Link(_)) | (Child::Special(_), Child::Special(_)) => {
                self.counts.skipped += 1;
            }
            (Child::Failed(_), _) | (_, Child::Failed(_)) => {
                // Reported when listed; the pair cannot be proven equal.
                self.mark(node, name, None, CompareMark::Unverified);
            }
            // A link opposite a file, or another kind mismatch.
            _ => {
                self.differs(node);
                self.mark(node, name, None, CompareMark::Differs);
            }
        }
        Ok(())
    }

    /// Lists one folder. None when it cannot be opened (recorded as an error).
    fn list(&mut self, path: &Path) -> Result<Option<Listing>, FsError> {
        let mut listing = Listing::default();
        let mut errors = Vec::new();
        let result = self.lister.list(path, self.cancel, &mut |child| {
            if let Child::Failed(error) = child {
                errors.push(error);
                listing.incomplete = true;
                return true;
            }
            let cost = size_of::<Child>() + child_name(&child).len();
            if listing.bytes + cost > MAX_LISTING_BYTES {
                listing.incomplete = true;
                listing.limited = true;
                return false;
            }
            listing.bytes += cost;
            listing.children.push(child);
            true
        });
        for error in errors {
            self.counts.error(error);
        }
        if listing.limited {
            self.counts.limited = true;
        }
        match result {
            Ok(()) => Ok(Some(listing)),
            Err(error) => {
                self.stopped(path)?;
                if error.kind == std::io::ErrorKind::Interrupted && self.cancel.is_cancelled() {
                    return Err(interrupted(path));
                }
                self.counts.error(error);
                Ok(None)
            }
        }
    }

    fn send(&mut self, candidate: Candidate, pool: &Pool) -> Result<(), FsError> {
        self.progress.content_files_total += 1;
        self.progress.content_bytes_total = self
            .progress
            .content_bytes_total
            .saturating_add(candidate.expected[0].size.saturating_mul(2));
        let path = candidate.paths[0].clone();
        let mut candidate = Some(candidate);
        // The pool's queue is bounded: wait for room, answering its outcomes and
        // watching for cancellation meanwhile.
        loop {
            match pool.offer(candidate.take().expect("candidate offered once")) {
                None => break,
                Some(back) => {
                    candidate = Some(back);
                    self.stopped(&path)?;
                    self.drain(pool, true)?;
                    self.tick(pool);
                }
            }
        }
        self.outstanding += 1;
        Ok(())
    }

    /// Applies the pool's outcomes; `wait` waits briefly for one.
    fn drain(&mut self, pool: &Pool, wait: bool) -> Result<(), FsError> {
        let mut wait = wait;
        while let Some(outcome) = pool.outcome(wait) {
            wait = false;
            self.outstanding -= 1;
            self.apply(outcome)?;
        }
        Ok(())
    }

    fn apply(&mut self, outcome: Outcome) -> Result<(), FsError> {
        let Outcome {
            origin,
            key,
            path,
            result,
        } = outcome;
        self.progress.content_files += 1;
        let status = match result {
            Ok(true) => {
                self.counts.content_files += 1;
                Status::Same
            }
            Ok(false) => {
                self.counts.content_files += 1;
                self.counts.content_differences += 1;
                Status::Differs
            }
            Err(error) => {
                // Only this comparison's cancellation ends it; any other failure
                // leaves the pair selected as not verified.
                self.stopped(&path)?;
                self.counts.error(error);
                self.counts.unknown += 1;
                Status::Unverified
            }
        };
        match origin {
            Origin::Row(rows) => self.rows.push((rows, key, status)),
            Origin::Node(node) => {
                if status != Status::Same {
                    self.differs(node);
                }
                if let (Some(marks), Some(key)) = (self.marks.as_deref_mut(), key) {
                    let mark = match status {
                        Status::Same => CompareMark::Same,
                        Status::Differs => CompareMark::Differs,
                        Status::Unverified => CompareMark::Unverified,
                    };
                    if status != Status::Same {
                        marks.set(&key, 0, mark);
                        marks.set(&key, 1, mark);
                    }
                }
            }
        }
        Ok(())
    }

    fn finish(mut self, tops: usize, pool: &Pool) -> Result<Outcomes, FsError> {
        self.counts.content_bytes = pool.progress().bytes;
        self.counts.folders = self.progress.folders;
        let statuses: Vec<_> = self
            .nodes
            .iter()
            .map(|node| match node.state {
                State::Same => Status::Same,
                State::Differs => Status::Differs,
                State::Unverified => Status::Unverified,
            })
            .collect();
        if let Some(marks) = self.marks.as_deref_mut() {
            for (node, status) in self.nodes.iter().zip(&statuses) {
                let Some(key) = node.key.as_deref() else {
                    continue;
                };
                let mark = match status {
                    Status::Same => CompareMark::Same,
                    Status::Differs => CompareMark::Differs,
                    Status::Unverified => CompareMark::Unverified,
                };
                marks.set(key, 0, mark);
                marks.set(key, 1, mark);
            }
            if self.nodes.iter().any(|node| node.key.is_none()) {
                // Folders without a stored key cannot answer for their children.
                marks.mark_incomplete();
            }
        }
        Ok(Outcomes {
            tops: statuses.into_iter().take(tops).collect(),
            rows: std::mem::take(&mut self.rows),
            counts: std::mem::take(&mut self.counts),
        })
    }
}

#[derive(Default)]
struct Listing {
    children: Vec<Child>,
    /// A child could not be described, or the listing budget ended it early.
    incomplete: bool,
    limited: bool,
    bytes: usize,
}

fn child_name(child: &Child) -> &OsStr {
    match child {
        Child::Entry { name, .. } | Child::Link(name) | Child::Special(name) => name,
        Child::Failed(error) => error.path.file_name().unwrap_or_else(|| OsStr::new("")),
    }
}

/// Lists through the search service: every child is opened, without following it.
pub(super) struct FindLister<'a>(pub(super) &'a dyn FindService);

impl Lister for FindLister<'_> {
    fn list(
        &self,
        path: &Path,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(Child) -> bool,
    ) -> Result<(), FsError> {
        let mut source = self.0.open_find_source(path, cancel)?;
        if source.entry().kind != EntryKind::Directory {
            return Err(compare_error(
                path,
                "A compared folder is no longer a folder.",
            ));
        }
        while let Some(child) = source.next_child(cancel)? {
            let child = match child {
                FindChild::Entry(source) => describe(
                    &source.entry().path,
                    source.entry().kind,
                    &source.entry().metadata,
                ),
                FindChild::Skipped { path, reason } => {
                    let name = path.file_name().unwrap_or_default().to_owned();
                    match reason {
                        FindSkipReason::Special => Child::Special(name),
                        FindSkipReason::Link | FindSkipReason::MountBoundary => Child::Link(name),
                    }
                }
                FindChild::Failed(error) => Child::Failed(error),
            };
            if !visit(child) {
                break;
            }
        }
        Ok(())
    }
}

/// Lists through the transfer service's folder records: no child is opened, so a
/// folder of many files lists at the speed of the native directory enumeration.
pub(super) struct TransferLister<'a>(pub(super) &'a dyn TransferService);

impl Lister for TransferLister<'_> {
    fn list(
        &self,
        path: &Path,
        cancel: &Cancellation,
        visit: &mut dyn FnMut(Child) -> bool,
    ) -> Result<(), FsError> {
        let mut source = self.0.prepare_transfer_source(path, cancel)?;
        if source.entry().item.kind != EntryKind::Directory {
            return Err(compare_error(
                path,
                "A compared folder is no longer a folder.",
            ));
        }
        let device = source.entry().identity.device;
        while let Some(child) = source.next_listed_child(&|_| false, cancel)? {
            let entry = child.entry();
            let child =
                if entry.identity.device != device && entry.item.kind == EntryKind::Directory {
                    // Another volume mounted here: not followed.
                    Child::Link(entry.item.path.file_name().unwrap_or_default().to_owned())
                } else {
                    describe(&entry.item.path, entry.item.kind, &entry.item.metadata)
                };
            if !visit(child) {
                break;
            }
        }
        Ok(())
    }
}

fn describe(path: &Path, kind: EntryKind, metadata: &EntryMetadata) -> Child {
    let name = path.file_name().unwrap_or_default().to_owned();
    match kind {
        EntryKind::File | EntryKind::Directory => Child::Entry {
            name,
            directory: kind == EntryKind::Directory,
            metadata: EntryMetadata {
                size: metadata.size,
                modified: metadata.modified,
                native_attributes: metadata.native_attributes,
                unix_mode: metadata.unix_mode,
                ..EntryMetadata::default()
            },
        },
        EntryKind::DirectoryLink | EntryKind::Symlink => Child::Link(name),
        _ => Child::Special(name),
    }
}
