//! A native selection of more items than a job prepares before it starts
//! (`input::MAX_PREPARED_NATIVE_ROOTS`), taken one item at a time.
//!
//! The job keeps the selected paths (shared with its input) and, per item, its identity,
//! read in one pass that opens each item and closes it again. That pass also totals the
//! discovery item by item, just ahead of the copy: the job takes item `i` only once the
//! pass has read and totalled it, and gives the pass a short slice of time beyond that
//! each time it takes an item, so the pass runs ahead without delaying the first copy.
//! Taking an item opens it again (for a Move, prepares it for Move) and checks that it is
//! still the item the pass read. For Copy from an ID-less share, the pass retains the
//! exact handle and stops reading ahead until the job takes that item. The job's
//! namespace claim covers the selected items' folders (`preparation::streamed_endpoints`).
//!
//! A conflict requiring the remaining identities can offer Rename and Skip while an
//! ID-less item holds the pass. This keeps the number of live native handles bounded.
use super::*;
use midnight_core::{DiscoveryScope, DiscoveryState, OperationDiscovery};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// How long the pass may read ahead each time the job takes an item.
const SCAN_SLICE: Duration = Duration::from_millis(2);
/// Discovery publishes its counts to the job's status at most this often.
const DISCOVERY_PUBLISH_INTERVAL: Duration = if cfg!(test) {
    Duration::ZERO
} else {
    Duration::from_millis(50)
};

/// What the pass read of one selected item.
#[derive(Clone, Copy)]
enum Fact {
    /// Its identity (for a link the job follows, the link's own) and revision.
    Read(Seen),
    /// It could not be opened; taking it reports why.
    Unreadable,
    /// For a Move, the same folder as an earlier item, which the job moves once.
    Repeated,
}

/// The items of a streamed selection and what the pass read of them. About 100 bytes
/// per item beside its path: the fact (32 bytes) and one map entry.
pub(super) struct StreamedRoots {
    service: Arc<dyn TransferService>,
    paths: Arc<FileClipboard>,
    links: CopyLinkPolicy,
    moved: bool,
    /// One per item the pass has read: `facts.len()` items are identified.
    facts: Vec<Fact>,
    /// An unreadable selection may still be an alias of a conflict target.
    unreadable: usize,
    /// Items whose discovery the pass has totalled (never more than it identified).
    discovered: usize,
    /// The next item the job takes.
    next: usize,
    /// Each item's own identity, with the first item that has it.
    identities: HashMap<TransferIdentity, u32>,
    /// The identity of each selected folder (what a followed link leads to included).
    directories: HashMap<TransferIdentity, u32>,
    /// The followed targets of selected links, apart from the links' own identities.
    projected: HashMap<TransferIdentity, u32>,
    /// An ID-less file or folder cannot be closed after discovery and then reacquired
    /// by pathname. Stop reading ahead at its pin until the job takes this exact object.
    pinned: Option<(usize, preparation::SharedSource)>,
    pub(super) native_roots: NativeRoots,
    /// The work the discovery has totalled so far, adopted by the job once complete.
    budget: work_budget::Budget,
    published: Instant,
}

/// The selected item as the pass read it: its native identity and, for a file, a digest
/// of its revision (length and modified time), since a file deleted and created again can
/// take the same identity (an inode number is reused). A folder keeps only its identity:
/// its modified time changes whenever an entry is added or removed, and what it holds is
/// checked entry by entry when it is listed (as `planned_source::same_planned` does).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Seen {
    identity: TransferIdentity,
    revision: u64,
}

impl Seen {
    fn of(source: &dyn TransferSource) -> Self {
        use std::hash::{Hash, Hasher};
        let entry = source.entry();
        let revision = if entry.item.kind == EntryKind::Directory {
            0
        } else {
            let mut digest = std::collections::hash_map::DefaultHasher::new();
            std::mem::discriminant(&entry.item.kind).hash(&mut digest);
            entry.item.metadata.size.hash(&mut digest);
            entry.item.metadata.modified.hash(&mut digest);
            digest.finish()
        };
        Self {
            identity: source.native_original_identity().unwrap_or(entry.identity),
            revision,
        }
    }
}

/// What taking the next item gave.
pub(super) enum Taken {
    Root(Box<dyn TransferSource>),
    /// The item cannot be taken; the job asks Retry, Skip or Cancel.
    Failed(FsError),
    /// The pass has not read and totalled the item yet; the job tries again next step.
    Waiting,
    /// The item repeats an earlier folder of a Move: the job leaves it out.
    Repeated,
    Done,
}

impl StreamedRoots {
    pub(super) fn new(
        service: Arc<dyn TransferService>,
        paths: Arc<FileClipboard>,
        links: CopyLinkPolicy,
        moved: bool,
    ) -> Self {
        Self {
            service,
            paths,
            links,
            moved,
            facts: Vec::new(),
            unreadable: 0,
            discovered: 0,
            next: 0,
            identities: HashMap::new(),
            directories: HashMap::new(),
            projected: HashMap::new(),
            pinned: None,
            native_roots: NativeRoots::default(),
            budget: work_budget::Budget::discovery(),
            published: Instant::now(),
        }
    }
    pub(super) fn len(&self) -> usize {
        self.paths.paths.len()
    }
    /// The items the job has not taken yet.
    pub(super) fn remaining(&self) -> usize {
        self.len() - self.next
    }
    pub(super) fn is_done(&self) -> bool {
        self.next >= self.len()
    }
    pub(super) fn identified_all(&self) -> bool {
        self.facts.len() >= self.len()
    }
    pub(super) fn protects_all(&self) -> bool {
        self.identified_all() && (self.moved || self.unreadable == 0)
    }
    fn advance(&mut self) {
        if self
            .pinned
            .as_ref()
            .is_some_and(|(index, _)| *index == self.next)
        {
            self.pinned = None;
        }
        self.next += 1;
    }
    /// Whether `identity` is a selected item, or what one of them leads to: a replaced
    /// or merged target must never be one.
    pub(super) fn protects(&self, identity: TransferIdentity) -> bool {
        self.identities.contains_key(&identity) || self.projected.contains_key(&identity)
    }
    fn path(&self, index: usize) -> &Path {
        &self.paths.paths[index]
    }
    fn open(
        &self,
        index: usize,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        let path = self.path(index);
        if let Some((pinned_index, source)) = &self.pinned
            && *pinned_index == index
        {
            source.revalidate(cancel)?;
            return Ok(Box::new(source.clone()));
        }
        let source = self.service.prepare_transfer_source_with_policy(
            path,
            self.links,
            TRANSFER_REVISIONS,
            cancel,
        )?;
        if source.entry().item.path != path {
            return Err(error(
                path,
                io::ErrorKind::InvalidInput,
                "Provider returned a different source.",
            ));
        }
        Ok(source)
    }
    /// Reads the next unread item's identity; returns it still open where the item can
    /// be totalled (not a repeat), so that the pass reads it once.
    fn identify(&mut self, cancel: &Cancellation) -> Option<Box<dyn TransferSource>> {
        let index = self.facts.len();
        let Ok(mut source) = self.open(index, cancel) else {
            self.facts.push(Fact::Unreadable);
            self.unreadable += 1;
            return None;
        };
        if !self.moved && source.requires_copy_directory_guard() {
            let retained = preparation::SharedSource::new(source);
            self.pinned = Some((index, retained.clone()));
            source = Box::new(retained);
        }
        let entry = source.entry();
        let seen = Seen::of(source.as_ref());
        // A Move takes a folder once: the same folder under another path (another case
        // of its name, a path through a link) would be gone after the first. Copy copies
        // each path, and a Move moves each name of a file with several links.
        if self.moved
            && entry.item.kind == EntryKind::Directory
            && self.identities.contains_key(&seen.identity)
        {
            self.facts.push(Fact::Repeated);
            return None;
        }
        self.protect(index, source.as_ref(), seen);
        self.facts.push(Fact::Read(seen));
        Some(source)
    }
    fn protect(&mut self, index: usize, source: &dyn TransferSource, seen: Seen) {
        self.native_roots.observe(source);
        let position = index as u32;
        self.identities.entry(seen.identity).or_insert(position);
        let projected = source.native_identity().unwrap_or(source.entry().identity);
        if projected != seen.identity {
            self.projected.entry(projected).or_insert(position);
        }
        if source.entry().item.kind == EntryKind::Directory {
            self.directories.entry(projected).or_insert(position);
        }
    }
    /// Reads identities only, until every item is identified or `until` passes.
    pub(super) fn identify_until(&mut self, until: Instant, cancel: &Cancellation) {
        while !self.identified_all()
            && self.pinned.is_none()
            && !cancel.is_cancelled()
            && Instant::now() < until
        {
            drop(self.identify(cancel));
        }
    }
    /// Reads and totals items, through item `through` at least and then until `until`
    /// passes. Each item is opened once for both where the pass reaches it first.
    fn scan(
        &mut self,
        through: usize,
        until: Instant,
        discovery: &mut OperationDiscovery,
        verification: TransferVerification,
        status: &Status,
        cancel: &Cancellation,
    ) {
        let traversal = cancel.child_interrupted_by_pause();
        while self.discovered < self.len()
            && self
                .pinned
                .as_ref()
                .is_none_or(|(index, _)| self.discovered <= *index)
            && (self.discovered <= through || Instant::now() < until)
            && !traversal.is_cancelled()
        {
            let index = self.discovered;
            let source = if index < self.facts.len() {
                // Identified ahead (a conflict needed every identity): open it again.
                match self.facts[index] {
                    Fact::Read(_) => self.open(index, cancel).ok(),
                    Fact::Unreadable | Fact::Repeated => None,
                }
            } else {
                self.identify(cancel)
            };
            self.discovered += 1;
            if discovery.state != DiscoveryState::Scanning {
                continue;
            }
            let Some(source) = source else {
                if matches!(self.facts[index], Fact::Unreadable) {
                    discovery.approximate("A selected item cannot be read and was not counted.");
                }
                continue;
            };
            let scope = discovery.scope;
            // A link met where the policy follows links but was not followed: the totals
            // would leave out what it leads to. A link kept as a link is a leaf.
            let projection = scope == DiscoveryScope::Recursive
                && self.links.materializes_directory_links()
                && !self.links.recreates_links();
            let mut unprojected_link = false;
            let budget = &mut self.budget;
            let published = &mut self.published;
            let mut visit = |item: &MutationItem| {
                unprojected_link |= projection && link_job::is_link_kind(item.kind);
                discovery.observe(item);
                budget.observe(item, verification);
                if published.elapsed() >= DISCOVERY_PUBLISH_INTERVAL {
                    let mut status = status.lock().unwrap_or_else(|error| error.into_inner());
                    status.discovery = discovery.clone();
                    status.current_path = Some(item.path.clone());
                    *published = Instant::now();
                }
            };
            // A file was read as it was opened just now: its entry is its total. Only a
            // folder (or a link the policy follows) is walked.
            let result = if scope == DiscoveryScope::SelectedRoots {
                source
                    .revalidate(&traversal)
                    .map(|()| visit(&source.entry().item))
            } else if source.entry().item.kind == EntryKind::File
                && !link_job::is_link_kind(source.entry().item.kind)
            {
                visit(&source.entry().item);
                Ok(())
            } else {
                source.discover(&traversal, &mut visit)
            };
            drop(source);
            match result {
                Err(failure)
                    if failure.kind == io::ErrorKind::Interrupted && cancel.pause_requested() =>
                {
                    discovery.approximate("Transfer discovery interrupted by pause.");
                }
                Err(failure) => discovery.approximate(&format!(
                    "Part of the source could not be counted: {}",
                    failure.message
                )),
                Ok(()) if unprojected_link => {
                    discovery.approximate("A link that the copy follows was not counted.");
                }
                Ok(()) => {}
            }
        }
        if self.discovered >= self.len() {
            discovery.finish();
        }
    }
    /// The folders among the items the job has not taken yet must not contain `merged`, a
    /// folder the job merges into: it would copy into what it copies later.
    fn reject_pending_ancestors(
        &mut self,
        merged: &dyn TransferDirectory,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        while !self.identified_all() {
            check_cancel(merged.path(), cancel)?;
            if self.pinned.is_some() {
                return Err(error(
                    merged.path(),
                    io::ErrorKind::Unsupported,
                    "a later Copy source has no file ID to check this merge against",
                ));
            }
            self.identify_until(Instant::now() + SCAN_SLICE, cancel);
        }
        let next = self.next as u32;
        let pending = |index: &u32| *index >= next;
        if self.directories.is_empty() {
            return Ok(());
        }
        // One walk of the merged folder's ancestry and a lookup per ancestor, however many
        // selected folders there are; a provider without the walk is asked per folder.
        match merged.ancestor_identities(cancel)? {
            Some(ancestors) => {
                if ancestors
                    .iter()
                    .any(|ancestor| self.directories.get(ancestor).is_some_and(pending))
                {
                    return Err(error(
                        merged.path(),
                        io::ErrorKind::InvalidData,
                        "destination is inside the source directory",
                    ));
                }
                Ok(())
            }
            None => {
                for (identity, _) in self.directories.iter().filter(|(_, index)| pending(index)) {
                    merged.reject_source_ancestor(*identity, cancel)?;
                }
                Ok(())
            }
        }
    }
}

fn check_cancel(path: &Path, cancel: &Cancellation) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        return Err(error(path, io::ErrorKind::Interrupted, "Copy cancelled."));
    }
    Ok(())
}

/// Why a selected item is not taken: it is no longer the item the job read.
fn changed(path: &Path, intent: TransferIntent) -> FsError {
    error(
        path,
        io::ErrorKind::InvalidData,
        match intent {
            TransferIntent::Copy => {
                "It was replaced or changed after the Copy started, so it is not the item \
                 that was read; it was not copied."
            }
            TransferIntent::Move => {
                "It was replaced or changed after the Move started, so it is not the item \
                 that was read; it was not moved."
            }
        },
    )
}

impl Job {
    /// A Copy job of a streamed selection. A Move takes `move_job::prepare_streamed`.
    pub(in crate::copy_operations) fn prepare_streamed(
        request: input::StreamedRequest,
        service: Arc<dyn TransferService>,
        cancel: Cancellation,
        status: Status,
        links: CopyLinkPolicy,
    ) -> Result<Self, FsError> {
        if request.moved {
            return move_job::prepare_streamed(request, service, cancel, status, links);
        }
        let destination = service.prepare_transfer_destination(&request.destination, &cancel)?;
        if destination.path() != request.destination {
            return Err(error(
                &request.destination,
                io::ErrorKind::InvalidInput,
                "Provider returned a different destination.",
            ));
        }
        if links != CopyLinkPolicy::Reject {
            link_job::validate_options(
                &request.destination,
                &status,
                TransferMetadataPolicy::ContentOnly,
                links,
            )?;
        }
        let mut job = ownership::Armed::new(Self::from_sources(
            destination,
            VecDeque::new(),
            cancel,
            status,
        ));
        job.links = links;
        job.set_verification(request.verification);
        job.set_streamed(StreamedRoots::new(service, request.paths, links, false));
        Ok(job.into_inner())
    }

    pub(super) fn set_streamed(&mut self, streamed: StreamedRoots) {
        self.progress.requested_roots = streamed.len() as u64;
        self.streamed = Some(streamed);
    }

    /// Whether any selected item is still to be taken.
    pub(super) fn roots_left(&self) -> bool {
        !self.roots.is_empty() || self.streamed.as_ref().is_some_and(|roots| !roots.is_done())
    }

    /// The selected items the job has not taken yet.
    pub(super) fn unstarted_roots(&self) -> usize {
        self.roots.len() + self.streamed.as_ref().map_or(0, StreamedRoots::remaining)
    }

    /// Discovery of a streamed selection totals each item as the pass reaches it. A Move
    /// whose first item is on the destination's volume renames, so it counts the selected
    /// items only, as `prepare_discovery` does for prepared ones.
    pub(super) fn start_streamed_discovery(&mut self) {
        let destination = self.destination.identity().device;
        let Some(streamed) = self.streamed.as_mut() else {
            return;
        };
        let rename = streamed.moved
            && {
                if streamed.facts.is_empty() && streamed.len() > 0 {
                    drop(streamed.identify(&self.cancel));
                }
                matches!(streamed.facts.first(), Some(Fact::Read(seen)) if seen.identity.device == destination)
            };
        let scope = if rename {
            DiscoveryScope::SelectedRoots
        } else {
            DiscoveryScope::Recursive
        };
        self.progress.discovery = OperationDiscovery::scanning(scope);
        self.store_progress();
    }

    /// Reads and totals items through `through` and within the slice; adopts the totals
    /// once the discovery is complete.
    fn scan_streamed(&mut self, through: usize) {
        let Some(streamed) = self.streamed.as_mut() else {
            return;
        };
        let was_scanning = self.progress.discovery.state == DiscoveryState::Scanning;
        streamed.scan(
            through,
            Instant::now() + SCAN_SLICE,
            &mut self.progress.discovery,
            self.verification,
            &self.status,
            &self.cancel,
        );
        if was_scanning && self.progress.discovery.state == DiscoveryState::Complete {
            self.work_budget.adopt_readback(&streamed.budget);
        }
    }

    /// Takes the next selected item: opens it (for a Move, prepares it for Move) and
    /// checks that it is still the item the pass read.
    pub(super) fn take_streamed_root(&mut self) -> Taken {
        let Some(streamed) = self.streamed.as_ref() else {
            return Taken::Done;
        };
        if streamed.is_done() {
            return Taken::Done;
        }
        let index = streamed.next;
        self.scan_streamed(index);
        let streamed = self.streamed.as_mut().expect("streamed roots");
        if streamed.discovered <= index {
            return Taken::Waiting;
        }
        let expected = match streamed.facts[index] {
            Fact::Repeated => {
                streamed.advance();
                // The folder is moved once, as its first occurrence, as a repeated path is
                // left out of a prepared selection.
                self.progress.requested_roots = self.progress.requested_roots.saturating_sub(1);
                self.report_unstarted_now();
                return Taken::Repeated;
            }
            Fact::Read(seen) => Some(seen),
            Fact::Unreadable => None,
        };
        let path = streamed.path(index).to_owned();
        let taken = if streamed.moved {
            self.take_streamed_move(&path, expected)
        } else {
            self.take_streamed_copy(index, expected)
        };
        match taken {
            Ok(source) => {
                let streamed = self.streamed.as_mut().expect("streamed roots");
                streamed.advance();
                self.report_unstarted_now();
                Taken::Root(source)
            }
            Err(error) => Taken::Failed(error),
        }
    }

    fn take_streamed_copy(
        &mut self,
        index: usize,
        expected: Option<Seen>,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        let streamed = self.streamed.as_mut().expect("streamed roots");
        let source = streamed.open(index, &self.cancel)?;
        if expected.is_some_and(|expected| expected != Seen::of(source.as_ref())) {
            return Err(changed(streamed.path(index), TransferIntent::Copy));
        }
        link_job::admit_root(source.as_ref(), self.links, &self.cancel)?;
        if expected.is_none() {
            // An unreadable item can become available on Retry. Protect this
            // newly acquired selection in all the conflicts that follow it.
            let seen = Seen::of(source.as_ref());
            streamed.protect(index, source.as_ref(), seen);
            streamed.facts[index] = Fact::Read(seen);
            streamed.unreadable -= 1;
        }
        Ok(source)
    }

    fn take_streamed_move(
        &mut self,
        path: &Path,
        expected: Option<Seen>,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        let streamed = self.streamed.as_ref().expect("streamed roots");
        let prepared = streamed.service.prepare_move_source_with_policy(
            path,
            streamed.links,
            TRANSFER_REVISIONS,
            &self.cancel,
        )?;
        let source = prepared.source();
        if source.entry().item.path != path {
            return Err(move_error(
                path,
                io::ErrorKind::Unsupported,
                "Provider returned a different Move source.",
            ));
        }
        if expected.is_some_and(|expected| expected != Seen::of(source)) {
            return Err(changed(path, TransferIntent::Move));
        }
        let move_work = self
            .move_work
            .as_mut()
            .expect("a streamed Move has Move work");
        move_work.admit_root(prepared, self.destination.as_ref(), &self.cancel)
    }

    /// Skip of an item that could not be taken: the job goes on with the next one.
    pub(super) fn skip_streamed_root(&mut self) {
        if let Some(streamed) = self.streamed.as_mut()
            && !streamed.is_done()
        {
            streamed.advance();
        }
        self.progress.skipped += 1;
        self.report_unstarted_now();
    }

    /// The path of the item the job could not take, for its failure.
    pub(super) fn failed_streamed_root(&self) -> Option<std::path::PathBuf> {
        let streamed = self.streamed.as_ref()?;
        (!streamed.is_done()).then(|| streamed.path(streamed.next).to_owned())
    }

    fn report_unstarted_now(&self) {
        if let Some(unstarted) = &self.unstarted {
            unstarted.set(self.unstarted_roots());
        }
    }

    /// Before a conflict is offered: whether every selected item's identity is read, so
    /// that a target that is one of them is never replaced or merged. Reads them for one
    /// slice where not; the job then asks again at its next step.
    pub(super) fn identities_ready(&mut self) -> bool {
        let Some(streamed) = self.streamed.as_mut() else {
            return true;
        };
        if !streamed.identified_all() {
            self.progress.phase = CopyOperationPhase::Planning;
            streamed.identify_until(Instant::now() + STEP_BUDGET, &self.cancel);
        }
        // A held ID-less root stops read-ahead. The user can still Skip or Rename
        // the collision; Merge and Replace require a complete list of protected roots.
        streamed.identified_all() || streamed.pinned.is_some() || self.cancel.is_cancelled()
    }

    /// A folder merged into must not be inside a selected folder the job takes later.
    pub(super) fn reject_streamed_ancestors(
        &mut self,
        merged: &dyn TransferDirectory,
    ) -> Result<(), FsError> {
        match self.streamed.as_mut() {
            Some(streamed) => streamed.reject_pending_ancestors(merged, &self.cancel),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
impl Job {
    /// Reads every selected item's identity now, as a conflict would.
    pub(in crate::copy_operations) fn identify_streamed_for_test(&mut self) {
        if let Some(streamed) = self.streamed.as_mut() {
            while !streamed.identified_all() && streamed.pinned.is_none() {
                streamed.identify_until(Instant::now() + SCAN_SLICE, &self.cancel);
            }
        }
    }
    /// The memory the streamed selection keeps beside its paths, and the items it has
    /// identified: the per-item cost that `MAX_NATIVE_TRANSFER_ROOTS` bounds.
    pub(in crate::copy_operations) fn streamed_retained_for_test(&self) -> (usize, usize) {
        let Some(streamed) = self.streamed.as_ref() else {
            return (0, 0);
        };
        fn map<K, V>(map: &HashMap<K, V>) -> usize {
            // hashbrown: one control byte per bucket and some empty buckets.
            map.capacity() * (std::mem::size_of::<(K, V)>() + 1) * 8 / 7
        }
        let paths = streamed.paths.paths.capacity() * std::mem::size_of::<std::path::PathBuf>()
            + streamed
                .paths
                .paths
                .iter()
                .map(|path| path.as_os_str().len())
                .sum::<usize>();
        (
            paths
                + streamed.facts.capacity() * std::mem::size_of::<Fact>()
                + map(&streamed.identities)
                + map(&streamed.directories)
                + map(&streamed.projected),
            streamed.facts.len(),
        )
    }
}
