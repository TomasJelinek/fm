//! Complete native namespace claims; provider callbacks never run under registry locks.
//!
//! Admission is an atomic check-and-claim of a job's own footprint: only its own pins are
//! probed, and granted peers are compared by their committed facts, which every change
//! announces before it mutates anything. So a grant never waits for a peer's turn or
//! depends on how much a peer has published. A selected root that a granted job already
//! claims or reserves is deferred, not refused: the job starts, and that item is claimed
//! by its first change that no earlier job refuses (changes wait for earlier jobs only,
//! so waits cannot form a cycle). A job still waits as a whole when a footprint is
//! unknown, when it would read what another job changes, or behind an older waiting job.
#[cfg(test)]
use midnight_core::TransferNamespaceParent;
use midnight_core::{
    Cancellation, FsError, TransferIdentity, TransferNamespaceAccess, TransferNamespaceExtent,
    TransferNamespacePin, TransferNamespaceProbe, TransferNamespaceScope,
};
use midnight_platform_common::transfer_namespace::{
    TransferNamespaceChange, TransferNamespaceChanges, TransferNamespaceRefreshBatch,
};
use midnight_work_budget::{Gate, Permit};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    io,
    path::Path,
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const POLL: Duration = Duration::from_millis(10);
const DIAGNOSTICS: usize = 16;
const READ: u8 = 1;
const WRITE: u8 = 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FactsState {
    Stable,
    InFlight,
    Opaque,
    Closing,
    Quarantined,
}

#[derive(Clone)]
pub(crate) struct Registry {
    inner: Arc<Inner>,
}
struct Inner {
    state: Mutex<State>,
    changed: Condvar,
    /// Wakes async waiters (refused grants, refused namespace changes) without polling.
    notify: tokio::sync::Notify,
    io: Arc<Gate>,
}
#[derive(Default)]
struct State {
    revision: u64,
    /// Bumped when a claim is granted, removed or quarantined: the only events that can
    /// end a conflict refusal (besides fact changes, which waiters revalidate slowly).
    structure: u64,
    records: BTreeMap<u64, Record>,
    turn: Option<u64>,
    /// Grants so far; each granted record keeps its position (`Record::granted_at`).
    grants: u64,
}
/// Registry state observed when a grant or a namespace change was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Seen {
    revision: u64,
    structure: u64,
}
impl State {
    fn seen(&self) -> Seen {
        Seen {
            revision: self.revision,
            structure: self.structure,
        }
    }
}
/// A grant attempt's outcome, decided at once. `Conflict`: the job cannot start beside the
/// granted jobs until a claim is granted, removed or quarantined; `Hold` says why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Grant {
    Granted,
    Conflict(Seen, Hold),
}
/// Why a job waits as a whole for other jobs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hold {
    /// Its own footprint, or that of a granted job or an older waiting one, is unknown
    /// (opaque or latched): such a job runs alone, whatever items the other one uses.
    Unknown,
    /// It would read what a granted job changes, or an older waiting job uses one of its
    /// items.
    Overlap,
}
impl Hold {
    /// What the waiting job tells the user.
    pub(crate) fn wait_reason(self) -> midnight_core::TransferWaitReason {
        match self {
            Self::Unknown => midnight_core::TransferWaitReason::UnconfirmedFolders,
            Self::Overlap => midnight_core::TransferWaitReason::SameFolders,
        }
    }
}
/// Why a granted job's namespace change was refused (see `take_peer_refusal`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refusal {
    /// Granted peers are opaque, latched or intersecting; retry once one of them changes.
    Peer(Blockers),
    /// A refusing peer is quarantined; its claim never clears, so waiting would never end.
    Quarantined,
}
/// The granted peers that refused a namespace change, each with its record revision at
/// the refusal. The refusing job's own turns never end the wait: only these records can
/// clear the refusal (the change's own facts are re-probed by every retry).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Blockers(Vec<Blocker>);
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Blocker {
    sequence: u64,
    revision: u64,
    /// A latched peer stays opaque until its claim is removed.
    latched: bool,
    /// The peer's footprint was unknown (latched or opaque) at the refusal, so it refused
    /// the change whatever items the change touches.
    unknown: bool,
}
impl Blockers {
    /// Whether a blocker can clear without being removed, so periodic revalidation of
    /// external changes can help. A peer latched opaque only clears by its removal.
    pub(crate) fn can_clear_in_place(&self) -> bool {
        self.0.is_empty() || self.0.iter().any(|blocker| !blocker.latched)
    }
    /// Why the change waits: `Unknown` only when every refusing peer's footprint was
    /// unknown; a peer that uses the same items makes it `Overlap`.
    pub(crate) fn hold(&self) -> Hold {
        if !self.0.is_empty() && self.0.iter().all(|blocker| blocker.unknown) {
            Hold::Unknown
        } else {
            Hold::Overlap
        }
    }
}
thread_local! {
    static PEER_REFUSAL: std::cell::Cell<Option<Refusal>> = const { std::cell::Cell::new(None) };
}
/// Fixture adapters record the refusal a real `reserve_private_child` would record. It
/// names no peer, so the async owner retries after the minimum spacing.
#[cfg(test)]
pub(crate) fn refuse_for_test() {
    PEER_REFUSAL.with(|refusal| refusal.set(Some(Refusal::Peer(Blockers::default()))));
}
/// Fixture adapters record a refusal by a quarantined peer.
#[cfg(test)]
pub(crate) fn refuse_quarantined_for_test() {
    PEER_REFUSAL.with(|refusal| refusal.set(Some(Refusal::Quarantined)));
}
/// Forget any refusal recorded by an earlier step on this (reused) blocking thread.
pub(crate) fn clear_peer_refusal() {
    PEER_REFUSAL.with(|refusal| refusal.set(None));
}
/// Whether the last namespace reservation of this thread's current step was refused by
/// granted peers. Every reservation first clears the record, so a refused attempt that a
/// later reservation of the same step replaced (a fresh private name) is not reported.
/// Adapters report the refusal as an ordinary error; the refused change mutated nothing.
pub(crate) fn take_peer_refusal() -> Option<Refusal> {
    PEER_REFUSAL.with(|refusal| refusal.take())
}
/// Async view of registry changes; no lock is held across an await.
#[derive(Clone)]
pub(crate) struct Watch(Arc<Inner>);
impl Watch {
    pub(crate) fn seen(&self) -> Seen {
        self.0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .seen()
    }
    /// Completes once the registry changed after `since`: any change, or with
    /// `structural` only a grant, removal or quarantine.
    pub(crate) async fn changed(&self, since: Seen, structural: bool) {
        loop {
            let notified = self.0.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let now = self.seen();
            if now.structure != since.structure || (!structural && now.revision != since.revision) {
                return;
            }
            notified.await;
        }
    }
    /// Completes once a refusing peer may have cleared: its record changed, was removed or
    /// quarantined. A latched peer clears only by removal (or quarantine, which ends the
    /// retry with an error). A refusal that names no peer completes at once.
    pub(crate) async fn blockers_changed(&self, blockers: &Blockers) {
        loop {
            let notified = self.0.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.blockers_cleared(blockers) {
                return;
            }
            notified.await;
        }
    }
    fn blockers_cleared(&self, blockers: &Blockers) -> bool {
        let state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        blockers.0.is_empty()
            || blockers
                .0
                .iter()
                .any(|blocker| match state.records.get(&blocker.sequence) {
                    None => true,
                    Some(record) => {
                        record.state == FactsState::Quarantined
                            || (!blocker.latched && record.revision != blocker.revision)
                    }
                })
    }
}
struct Record {
    revision: u64,
    state: FactsState,
    planned: bool,
    granted: bool,
    /// Grant order: a change waits for reservations of jobs granted before it only.
    granted_at: u64,
    /// Per planned root: whether the job reads it (a source) rather than only changing it
    /// through announced changes (a destination).
    reads: Vec<bool>,
    /// Selected roots that a granted job claimed or reserved at this grant. They are not
    /// claims yet: each becomes one with its first change that no earlier job refuses.
    deferred: Vec<usize>,
    reserved_pins: Vec<PinFact>,
    /// Facts of the deferred roots, which later jobs respect as if claimed.
    reserved: Option<Arc<Facts>>,
    /// Roots the job only reads whose geometry is unknown (a followed link, a provider
    /// without native geometry). They are never written, so they are left unclaimed
    /// instead of making the whole job exclusive; reads through them are unprotected.
    unprotected: Vec<usize>,
    opaque_latched: bool,
    facts: Arc<Facts>,
    native: Weak<PinStore>,
    io: Weak<Permit>,
    probes: usize,
    native_closed: bool,
    quarantined: Arc<AtomicBool>,
    diagnostics: Arc<Mutex<Vec<FsError>>>,
}
struct Context {
    registry: Arc<Inner>,
    sequence: u64,
    native: Weak<PinStore>,
    quarantined: Arc<AtomicBool>,
    diagnostics: Arc<Mutex<Vec<FsError>>>,
}

/// Pure host ownership. Dropping an uncommitted claim never releases authority.
pub(crate) struct Claim {
    context: Arc<Context>,
    completed: AtomicBool,
}
/// The only persistent strong pin store. Install under the Copy seal immediately.
pub(crate) struct NativeOwners {
    store: Option<Arc<PinStore>>,
    context: Arc<Context>,
    witness: Option<crate::transfer_composition::TransferRetirementWitness>,
}
#[derive(Default)]
struct PinStore {
    data: Mutex<PinData>,
}
#[derive(Default)]
struct PinData {
    roots: Vec<PinEntry>,
    root_identity: HashMap<usize, Vec<usize>>,
    aliases: BTreeMap<u64, PinEntry>,
    alias_identity: HashMap<usize, u64>,
    next_alias: u64,
    installed: bool,
}
#[derive(Clone)]
struct PinEntry {
    access: TransferNamespaceAccess,
    pin: Option<Arc<dyn TransferNamespacePin>>,
}
#[derive(Clone)]
struct PinFact {
    root: Option<usize>,
    alias: Option<u64>,
    access: TransferNamespaceAccess,
    probe: TransferNamespaceProbe,
}

#[derive(Default)]
struct Facts {
    initialized: bool,
    opaque: bool,
    index: Arc<Index>,
    overlay: Mutex<Overlay>,
}
#[derive(Default)]
struct Overlay {
    opaque: bool,
    index: Index,
    roots: HashMap<usize, PinFact>,
    aliases: BTreeMap<u64, PinFact>,
}
#[derive(Clone, PartialEq, Eq, Hash)]
struct Slot {
    parent: TransferIdentity,
    domain: Arc<[u8]>,
    key: Arc<[u8]>,
}
#[derive(Default)]
struct Bucket {
    counts: [usize; 2],
    roots: HashMap<usize, usize>,
}
impl Bucket {
    fn mask(&self) -> u8 {
        (u8::from(self.counts[0] != 0) * READ) | (u8::from(self.counts[1] != 0) * WRITE)
    }
    fn change(&mut self, mask: u8, root: Option<usize>, add: bool) -> Result<(), FsError> {
        for (index, bit) in [READ, WRITE].into_iter().enumerate() {
            if mask & bit != 0 {
                self.counts[index] = count(self.counts[index], add)?;
            }
        }
        if let Some(root) = root {
            let value = count(self.roots.get(&root).copied().unwrap_or(0), add)?;
            if value == 0 {
                self.roots.remove(&root);
            } else {
                self.roots.insert(root, value);
            }
        }
        Ok(())
    }
}
fn count(value: usize, add: bool) -> Result<usize, FsError> {
    (if add {
        value.checked_add(1)
    } else {
        value.checked_sub(1)
    })
    .ok_or_else(unconfirmed)
}
fn adjust<K: std::hash::Hash + Eq>(
    map: &mut HashMap<K, Bucket>,
    key: K,
    mask: u8,
    root: Option<usize>,
    add: bool,
) -> Result<(), FsError> {
    let mut entry = match map.entry(key) {
        std::collections::hash_map::Entry::Occupied(entry) => entry,
        std::collections::hash_map::Entry::Vacant(entry) if add => {
            let mut bucket = Bucket::default();
            bucket.change(mask, root, true)?;
            entry.insert(bucket);
            return Ok(());
        }
        std::collections::hash_map::Entry::Vacant(_) => return Err(unconfirmed()),
    };
    entry.get_mut().change(mask, root, add)?;
    if entry.get().mask() == 0 {
        entry.remove();
    }
    Ok(())
}
struct ParentFacts {
    domain: Arc<[u8]>,
    ancestors: Arc<[TransferIdentity]>,
    bucket: Bucket,
}
#[derive(Default)]
struct Index {
    objects: HashMap<TransferIdentity, Bucket>,
    slots: HashMap<Slot, Bucket>,
    subtrees: HashMap<TransferIdentity, Bucket>,
    ancestors: HashMap<TransferIdentity, Bucket>,
    parents: HashMap<TransferIdentity, ParentFacts>,
    ancestor_parents: HashMap<TransferIdentity, HashSet<TransferIdentity>>,
    #[cfg(test)]
    visits: usize,
}
fn access(value: TransferNamespaceAccess) -> u8 {
    if value == TransferNamespaceAccess::Write {
        WRITE
    } else {
        READ
    }
}
fn writes(a: u8, b: u8) -> bool {
    (a | b) & WRITE != 0
}
fn slot(scope: &TransferNamespaceScope) -> Slot {
    Slot {
        parent: scope.parent.identity,
        domain: scope.parent.lookup_domain.clone(),
        key: scope.lookup_key.clone(),
    }
}
fn intersects<K: std::hash::Hash + Eq>(a: &HashMap<K, Bucket>, b: &HashMap<K, Bucket>) -> bool {
    let (a, b) = if a.len() <= b.len() { (a, b) } else { (b, a) };
    a.iter().any(|(key, mask)| {
        b.get(key)
            .is_some_and(|peer| writes(mask.mask(), peer.mask()))
    })
}
impl Index {
    fn update(&mut self, pin: &PinFact, add: bool) -> Result<bool, FsError> {
        let TransferNamespaceProbe::Known(scopes) = &pin.probe else {
            return Ok(true);
        };
        if scopes.is_empty() && pin.alias.is_none() {
            return Err(failure(
                io::ErrorKind::InvalidData,
                "An ordinary native root returned an empty footprint.",
            ));
        }
        let mut opaque = false;
        for scope in scopes.iter() {
            #[cfg(test)]
            {
                self.visits += 1;
            }
            let parent = &scope.parent;
            if parent.ancestors.first() != Some(&parent.identity)
                || (scope.lookup_key.is_empty()
                    && (scope.object != Some(parent.identity)
                        || scope.extent != TransferNamespaceExtent::Subtree))
            {
                return Err(failure(
                    io::ErrorKind::InvalidData,
                    "Native namespace ancestry or root binding is invalid.",
                ));
            }
            let mask = access(pin.access);
            if let std::collections::hash_map::Entry::Vacant(entry) =
                self.parents.entry(parent.identity)
            {
                if !add {
                    return Err(unconfirmed());
                }
                let ancestors: HashSet<_> = parent.ancestors.iter().collect();
                if ancestors.len() != parent.ancestors.len() {
                    return Err(failure(
                        io::ErrorKind::InvalidData,
                        "Native namespace ancestry contains a cycle.",
                    ));
                }
                entry.insert(ParentFacts {
                    domain: parent.lookup_domain.clone(),
                    ancestors: parent.ancestors.clone(),
                    bucket: Bucket::default(),
                });
            }
            let group = self
                .parents
                .get_mut(&parent.identity)
                .expect("parent was installed");
            opaque |= group.domain != parent.lookup_domain || group.ancestors != parent.ancestors;
            let before = group.bucket.mask();
            group.bucket.change(mask, pin.root, add)?;
            let after = group.bucket.mask();
            let ancestors = group.ancestors.clone();
            for ancestor in ancestors.iter() {
                for bit in [READ, WRITE] {
                    if before & bit != after & bit {
                        adjust(&mut self.ancestors, *ancestor, bit, None, after & bit != 0)?;
                    }
                }
                if before == 0 {
                    self.ancestor_parents
                        .entry(*ancestor)
                        .or_default()
                        .insert(parent.identity);
                }
                if after == 0
                    && let Some(parents) = self.ancestor_parents.get_mut(ancestor)
                {
                    parents.remove(&parent.identity);
                    if parents.is_empty() {
                        self.ancestor_parents.remove(ancestor);
                    }
                }
            }
            if after == 0 {
                self.parents.remove(&parent.identity);
            }
            adjust(&mut self.slots, slot(scope), mask, pin.root, add)?;
            if let Some(object) = scope.object {
                adjust(&mut self.objects, object, mask, pin.root, add)?;
                if scope.extent == TransferNamespaceExtent::Subtree {
                    adjust(&mut self.subtrees, object, mask, pin.root, add)?;
                }
            }
        }
        Ok(opaque)
    }
    fn domains_conflict(&self, other: &Self) -> bool {
        let (a, b) = if self.parents.len() <= other.parents.len() {
            (&self.parents, &other.parents)
        } else {
            (&other.parents, &self.parents)
        };
        a.iter().any(|(id, parent)| {
            b.get(id).is_some_and(|peer| {
                parent.domain != peer.domain || parent.ancestors != peer.ancestors
            })
        })
    }
    fn conflicts(&self, other: &Self) -> bool {
        self.domains_conflict(other)
            || intersects(&self.objects, &other.objects)
            || intersects(&self.slots, &other.slots)
            || intersects(&self.subtrees, &other.ancestors)
            || intersects(&other.subtrees, &self.ancestors)
    }
    fn related(&self, scope: &TransferNamespaceScope, roots: &mut HashSet<usize>) {
        let mut collect = |bucket: Option<&Bucket>| {
            if let Some(bucket) = bucket {
                roots.extend(bucket.roots.keys().copied());
            }
        };
        collect(self.slots.get(&slot(scope)));
        if let Some(object) = scope.object {
            collect(self.objects.get(&object));
        }
        for ancestor in scope.parent.ancestors.iter() {
            collect(self.subtrees.get(ancestor));
        }
        if scope.extent == TransferNamespaceExtent::Subtree
            && let Some(parents) = scope
                .object
                .and_then(|object| self.ancestor_parents.get(&object))
        {
            for parent in parents {
                collect(self.parents.get(parent).map(|parent| &parent.bucket));
            }
        }
    }
}
impl Facts {
    fn build(pins: Vec<PinFact>) -> Result<Self, FsError> {
        let mut facts = Self {
            initialized: true,
            opaque: pins.is_empty(),
            ..Self::default()
        };
        for pin in pins {
            if let Some(alias) = pin.alias {
                let overlay = facts
                    .overlay
                    .get_mut()
                    .unwrap_or_else(|error| error.into_inner());
                let opaque = overlay.index.update(&pin, true)?;
                overlay.opaque |= opaque;
                overlay.aliases.insert(alias, pin);
            } else {
                facts.opaque |= Arc::get_mut(&mut facts.index)
                    .expect("new immutable base")
                    .update(&pin, true)?;
            }
        }
        let overlay = facts
            .overlay
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        overlay.opaque |= facts.index.domains_conflict(&overlay.index);
        Ok(facts)
    }
    /// Facts of part of a planned footprint. Unlike a whole plan, an empty part (every
    /// root deferred) claims nothing instead of an unknown footprint.
    fn subset(pins: Vec<PinFact>) -> Result<Self, FsError> {
        let empty = pins.is_empty();
        let mut facts = Self::build(pins)?;
        facts.opaque &= !empty;
        Ok(facts)
    }
    fn revalidated(prior: &Self, pins: Vec<PinFact>) -> Result<Self, FsError> {
        if !prior.initialized {
            return Self::build(pins);
        }
        // Admission still refreshes every native pin. Keep the initial indexed
        // authority immutable; the replacement overlay contains only current facts.
        let facts = Self {
            initialized: true,
            opaque: prior.opaque,
            index: prior.index.clone(),
            overlay: Mutex::default(),
        };
        facts.update(pins)?;
        Ok(facts)
    }
    fn is_opaque(&self) -> bool {
        self.opaque
            || self
                .overlay
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .opaque
    }
    fn conflicts(&self, other: &Self) -> bool {
        fn compare(a: &Facts, ao: &Overlay, b: &Facts, bo: &Overlay) -> bool {
            a.opaque
                || ao.opaque
                || b.opaque
                || bo.opaque
                || a.index.conflicts(&b.index)
                || a.index.conflicts(&bo.index)
                || ao.index.conflicts(&b.index)
                || ao.index.conflicts(&bo.index)
        }
        // Pure metadata locks only; a fixed order avoids cross-query deadlocks.
        if std::ptr::eq(self, other) {
            let overlay = self
                .overlay
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            return compare(self, &overlay, other, &overlay);
        }
        let (a, b) = if (self as *const Self as usize) < (other as *const Self as usize) {
            (self, other)
        } else {
            (other, self)
        };
        let ao = a.overlay.lock().unwrap_or_else(|error| error.into_inner());
        let bo = b.overlay.lock().unwrap_or_else(|error| error.into_inner());
        compare(a, &ao, b, &bo)
    }
    fn announcement(&self, candidate: &PinFact) -> Option<HashSet<usize>> {
        let TransferNamespaceProbe::Known(scopes) = &candidate.probe else {
            return None;
        };
        let overlay = self
            .overlay
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if self.opaque || overlay.opaque {
            return None;
        }
        let mut roots = HashSet::new();
        for scope in scopes.iter() {
            for index in [&self.index, &overlay.index] {
                if index
                    .parents
                    .get(&scope.parent.identity)
                    .is_some_and(|parent| {
                        parent.domain != scope.parent.lookup_domain
                            || parent.ancestors != scope.parent.ancestors
                    })
                {
                    return None;
                }
                index.related(scope, &mut roots);
            }
        }
        Some(roots)
    }
    fn update(&self, pins: Vec<PinFact>) -> Result<(), FsError> {
        let mut overlay = self
            .overlay
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for pin in pins {
            let prior = if let Some(root) = pin.root {
                overlay.roots.remove(&root)
            } else if let Some(alias) = pin.alias {
                overlay.aliases.remove(&alias)
            } else {
                return Err(unconfirmed());
            };
            if let Some(prior) = prior {
                overlay.index.update(&prior, false)?;
            }
            if matches!(&pin.probe, TransferNamespaceProbe::Known(scopes) if scopes.is_empty())
                && pin.alias.is_some()
            {
                continue;
            }
            if let TransferNamespaceProbe::Known(scopes) = &pin.probe {
                overlay.opaque |= scopes.iter().any(|scope| {
                    self.index
                        .parents
                        .get(&scope.parent.identity)
                        .is_some_and(|parent| {
                            parent.domain != scope.parent.lookup_domain
                                || parent.ancestors != scope.parent.ancestors
                        })
                });
            }
            let opaque = overlay.index.update(&pin, true)?;
            overlay.opaque |= opaque;
            if let Some(root) = pin.root {
                overlay.roots.insert(root, pin);
            } else if let Some(alias) = pin.alias {
                overlay.aliases.insert(alias, pin);
            }
        }
        Ok(())
    }
}
impl Default for Registry {
    fn default() -> Self {
        Self::new_with_io(midnight_work_budget::shared_io())
    }
}
impl Registry {
    pub(crate) fn new_with_io(io: Arc<Gate>) -> Self {
        Self {
            inner: Arc::new(Inner {
                state: Mutex::default(),
                changed: Condvar::new(),
                notify: tokio::sync::Notify::new(),
                io,
            }),
        }
    }
    pub(crate) fn new_job(&self, sequence: u64) -> Result<(Claim, NativeOwners), FsError> {
        let store = Arc::new(PinStore::default());
        let context = Arc::new(Context {
            registry: self.inner.clone(),
            sequence,
            native: Arc::downgrade(&store),
            quarantined: Arc::default(),
            diagnostics: Arc::default(),
        });
        let mut state = self
            .inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if sequence == 0 || state.records.contains_key(&sequence) {
            return Err(failure(
                io::ErrorKind::InvalidInput,
                "A namespace claim requires a unique nonzero admission sequence.",
            ));
        }
        state.revision = state
            .revision
            .checked_add(1)
            .ok_or_else(|| failure(io::ErrorKind::Other, "Namespace revision exhausted."))?;
        state.records.insert(
            sequence,
            Record {
                revision: 0,
                state: FactsState::Stable,
                planned: false,
                granted: false,
                granted_at: 0,
                reads: Vec::new(),
                deferred: Vec::new(),
                reserved_pins: Vec::new(),
                reserved: None,
                unprotected: Vec::new(),
                opaque_latched: false,
                facts: Arc::default(),
                native: Arc::downgrade(&store),
                io: Weak::new(),
                probes: 0,
                native_closed: false,
                quarantined: context.quarantined.clone(),
                diagnostics: context.diagnostics.clone(),
            },
        );
        drop(state);
        Ok((
            Claim {
                context: context.clone(),
                completed: AtomicBool::new(false),
            },
            NativeOwners {
                store: Some(store),
                context,
                witness: None,
            },
        ))
    }
    #[cfg(test)]
    fn state(&self, sequence: u64) -> FactsState {
        self.inner.state.lock().unwrap().records[&sequence].state
    }
    /// Granted claims in admission order.
    #[cfg(test)]
    pub(crate) fn granted_for_test(&self) -> Vec<u64> {
        let state = self.inner.state.lock().unwrap();
        state
            .records
            .iter()
            .filter(|(_, record)| record.granted)
            .map(|(sequence, _)| *sequence)
            .collect()
    }
    /// Latch a claim opaque as a failed refresh would, without charging it a diagnostic.
    #[cfg(test)]
    pub(crate) fn latch_opaque_for_test(&self, sequence: u64) {
        let mut state = self.inner.state.lock().unwrap();
        let record = state.records.get_mut(&sequence).unwrap();
        record.opaque_latched = true;
        if record.state == FactsState::Stable {
            record.state = FactsState::Opaque;
        }
        Inner::bump(&mut state, sequence).unwrap();
        self.inner.signal();
    }
}

fn failure(kind: io::ErrorKind, message: &str) -> FsError {
    FsError::from_io(
        "transfer namespace",
        Path::new(""),
        io::Error::new(kind, message),
    )
}
fn cancelled(cancel: &Cancellation) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(failure(
            io::ErrorKind::Interrupted,
            "Namespace admission cancelled.",
        ))
    } else {
        Ok(())
    }
}
fn unconfirmed() -> FsError {
    failure(
        io::ErrorKind::Other,
        "Native namespace teardown is unconfirmed; authority remains quarantined.",
    )
}
const QUARANTINE_BLOCK: &str = "transfer admission";
/// A quarantined claim is never removed, so waiting for it would never end. Admission
/// fails at once with an actionable, non-retryable error instead.
fn blocked_by_quarantine() -> FsError {
    FsError::from_io(
        QUARANTINE_BLOCK,
        Path::new(""),
        io::Error::other(
            "An earlier transfer's native cleanup could not be confirmed, so its files stay \
             reserved. Restart the application to run further transfers.",
        ),
    )
}
pub(crate) fn is_blocked_by_quarantine(error: &FsError) -> bool {
    error.operation == QUARANTINE_BLOCK
}
/// A running job's change refused by a quarantined peer: the same actionable,
/// non-retryable error, reported for the refused path.
pub(crate) fn refused_by_quarantine(path: &Path) -> FsError {
    FsError {
        path: path.to_owned(),
        ..blocked_by_quarantine()
    }
}
fn checked<T>(operation: impl FnOnce() -> T) -> Result<T, FsError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)) {
        Ok(value) => Ok(value),
        Err(panic) => {
            std::mem::forget(panic);
            Err(unconfirmed())
        }
    }
}
/// Native probes and changes run under this job's own held slot. Peers' slots are only
/// identity witnesses: a peer paused or awaiting a decision has released its capacity.
fn admitted<R>(registry: &Inner, io: &Permit, operation: impl FnOnce() -> R) -> Result<R, FsError> {
    if !io.is_held() {
        return Err(unconfirmed());
    }
    midnight_platform_common::transfer_io::with_transfer_io_permit_from(&registry.io, io, operation)
}
impl Inner {
    /// Wake blocking waiters (condvar) and async waiters (`Watch::changed`).
    fn signal(&self) {
        self.changed.notify_all();
        self.notify.notify_waiters();
    }
    fn restructure(state: &mut State) {
        state.structure = state.structure.wrapping_add(1);
    }
    fn bump(state: &mut State, sequence: u64) -> Result<(), FsError> {
        let record = state.records.get_mut(&sequence).ok_or_else(|| {
            failure(
                io::ErrorKind::InvalidInput,
                "Namespace claim is no longer registered.",
            )
        })?;
        let Some(revision) = state.revision.checked_add(1) else {
            record.state = FactsState::Quarantined;
            record.quarantined.store(true, Ordering::Release);
            return Err(unconfirmed());
        };
        let Some(local) = record.revision.checked_add(1) else {
            record.state = FactsState::Quarantined;
            record.quarantined.store(true, Ordering::Release);
            return Err(unconfirmed());
        };
        state.revision = revision;
        record.revision = local;
        Ok(())
    }
    fn quarantine(&self, sequence: u64) {
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        if let Some(record) = state.records.get_mut(&sequence) {
            record.state = FactsState::Quarantined;
            record.quarantined.store(true, Ordering::Release);
            let _ = Self::bump(&mut state, sequence);
            Self::restructure(&mut state);
        }
        if state.turn == Some(sequence) {
            state.turn = None;
        }
        self.signal();
    }
    fn opaque(&self, sequence: u64, error: FsError) {
        let diagnostics = {
            let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
            let Some(record) = state.records.get_mut(&sequence) else {
                return;
            };
            record.opaque_latched = true;
            if !matches!(
                record.state,
                FactsState::Closing | FactsState::Quarantined | FactsState::InFlight
            ) {
                record.state = FactsState::Opaque;
            }
            let diagnostics = record.diagnostics.clone();
            let _ = Self::bump(&mut state, sequence);
            self.signal();
            diagnostics
        };
        let mut errors = diagnostics
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        // A persistent failure seen again (a later turn, or the final revalidation before
        // close) is reported once.
        if errors.iter().any(|known| {
            known.operation == error.operation
                && known.kind == error.kind
                && known.path == error.path
                && known.message == error.message
        }) {
            return;
        }
        if errors.len() == DIAGNOSTICS {
            errors.remove(0);
        }
        errors.push(error);
    }
}

/// One selected root of a grant candidate: its facts, and the same geometry as a read,
/// which shows whether another job changes what this job would read there.
struct CandidateRoot {
    index: usize,
    facts: Facts,
    reads: Facts,
}
impl CandidateRoot {
    fn new(index: usize, pin: &PinFact) -> Result<Self, FsError> {
        let read = PinFact {
            access: TransferNamespaceAccess::Read,
            ..pin.clone()
        };
        Ok(Self {
            index,
            facts: Facts::build(vec![pin.clone()])?,
            reads: Facts::build(vec![read])?,
        })
    }
}

/// Whether a candidate starts beside the granted jobs, and which of its selected roots it
/// defers because a granted job claims or reserves an overlapping item. Otherwise it waits
/// as a whole: for an unknown footprint (its own, a granted job's or an older waiting
/// job's), for a granted job that changes what it would read, or behind an older waiting
/// job that shares an item with it.
/// Pure metadata under the registry lock; each comparison walks the candidate's side.
fn verdict(
    state: &State,
    sequence: u64,
    opaque: bool,
    facts: &Facts,
    roots: &[CandidateRoot],
    reads: &[bool],
) -> Result<Vec<usize>, Hold> {
    for (other, peer) in &state.records {
        if *other == sequence || !(peer.granted || (peer.planned && *other < sequence)) {
            continue;
        }
        if opaque || peer.opaque_latched || peer.facts.is_opaque() {
            return Err(Hold::Unknown);
        }
        if !peer.granted && facts.conflicts(&peer.facts) {
            return Err(Hold::Overlap);
        }
    }
    let mut deferred = Vec::new();
    for root in roots {
        let source = reads.get(root.index).copied().unwrap_or(true);
        let mut held = false;
        let granted = state
            .records
            .iter()
            .filter(|(other, peer)| **other != sequence && peer.granted);
        for (_, peer) in granted {
            for claim in std::iter::once(&peer.facts).chain(&peer.reserved) {
                // Reading what another job changes is never interleaved item by item.
                if source && root.reads.conflicts(claim) {
                    return Err(Hold::Overlap);
                }
                held |= root.facts.conflicts(claim);
            }
        }
        if held {
            deferred.push(root.index);
        }
    }
    Ok(deferred)
}

impl Claim {
    pub(crate) fn changes(&self) -> Arc<dyn TransferNamespaceChanges> {
        self.context.clone()
    }
    pub(crate) fn configured_io(&self) -> Arc<Gate> {
        self.context.registry.io.clone()
    }
    pub(crate) fn is_quarantined(&self) -> bool {
        self.context.quarantined.load(Ordering::Acquire)
    }
    pub(crate) fn quarantine(&self) {
        self.context.registry.quarantine(self.context.sequence);
    }
    pub(crate) fn take_diagnostics(&self) -> Vec<FsError> {
        std::mem::take(
            &mut *self
                .context
                .diagnostics
                .lock()
                .unwrap_or_else(|error| error.into_inner()),
        )
    }
    /// Plan roots that the job may read as well as change, as every source is. Only
    /// tests plan this way now: production plans endpoints with `plan_transfer`.
    #[cfg(test)]
    pub(crate) fn plan_union(
        &self,
        pins: Vec<(
            TransferNamespaceAccess,
            Option<Arc<dyn TransferNamespacePin>>,
        )>,
        io: Arc<Permit>,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        let reads = vec![true; pins.len()];
        self.plan(pins, reads, io, cancel)
    }
    /// Plan a transfer's endpoints, given as a (source, destination) pair per selected
    /// root. A source is read by the job; a destination is only changed through the job's
    /// announced changes (and looked up by name), so it can wait per item for a peer.
    pub(crate) fn plan_transfer(
        &self,
        pins: Vec<(
            TransferNamespaceAccess,
            Option<Arc<dyn TransferNamespacePin>>,
        )>,
        io: Arc<Permit>,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        let reads = (0..pins.len()).map(|index| index % 2 == 0).collect();
        self.plan(pins, reads, io, cancel)
    }
    fn plan(
        &self,
        pins: Vec<(
            TransferNamespaceAccess,
            Option<Arc<dyn TransferNamespacePin>>,
        )>,
        reads: Vec<bool>,
        io: Arc<Permit>,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        let store = self.context.native.upgrade().ok_or_else(unconfirmed)?;
        {
            let mut data = store.data.lock().unwrap_or_else(|error| error.into_inner());
            if data.installed {
                return Err(failure(
                    io::ErrorKind::InvalidInput,
                    "Namespace roots are immutable after planning.",
                ));
            }
            data.installed = true;
            data.roots = pins
                .into_iter()
                .map(|(access, pin)| PinEntry { access, pin })
                .collect();
            data.root_identity = data
                .roots
                .iter()
                .enumerate()
                .filter_map(|(root, entry)| {
                    entry
                        .pin
                        .as_ref()
                        .map(|pin| (Arc::as_ptr(pin) as *const () as usize, root))
                })
                .fold(HashMap::new(), |mut identities, (identity, root)| {
                    identities
                        .entry(identity)
                        .or_insert_with(Vec::new)
                        .push(root);
                    identities
                });
        }
        if !io.belongs_to(&self.context.registry.io) || !io.is_shared() {
            let error = failure(
                io::ErrorKind::PermissionDenied,
                "Namespace work requires the configured shared I/O permit.",
            );
            self.context
                .registry
                .opaque(self.context.sequence, error.clone());
            return Err(error);
        }
        {
            let mut state = self
                .context
                .registry
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            let record = state
                .records
                .get_mut(&self.context.sequence)
                .ok_or_else(unconfirmed)?;
            record.io = Arc::downgrade(&io);
            record.planned = true;
            record.reads = reads;
            Inner::bump(&mut state, self.context.sequence)?;
        }
        drop(store);
        let batch = ProbeBatch::single(&self.context.registry, self.context.sequence)?;
        let facts = admitted(&self.context.registry, &io, || batch.refresh(cancel, true))??;
        let fact = facts
            .into_iter()
            .next()
            .expect("one planned namespace record")
            .1;
        let mut state = self
            .context
            .registry
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let record = state
            .records
            .get_mut(&self.context.sequence)
            .ok_or_else(unconfirmed)?;
        if matches!(record.state, FactsState::Closing | FactsState::Quarantined) {
            return Err(unconfirmed());
        }
        record.state = if fact.is_opaque() || record.opaque_latched {
            FactsState::Opaque
        } else {
            FactsState::Stable
        };
        let retired = std::mem::replace(&mut record.facts, fact);
        Inner::bump(&mut state, self.context.sequence)?;
        drop(state);
        drop(retired);
        Ok(())
    }
    #[cfg(test)]
    pub(crate) fn try_grant(&self, cancel: &Cancellation) -> Result<bool, FsError> {
        self.admit(cancel).map(|grant| grant == Grant::Granted)
    }
    pub(crate) fn watch(&self) -> Watch {
        Watch(self.context.registry.clone())
    }
    /// Atomic check-and-claim of this job's footprint (see the module documentation). Only
    /// this job's own pins are probed, outside the lock. The decision and the claim follow
    /// under the registry lock from committed facts, in time bounded by this job's own
    /// selection, whatever granted peers are doing: there is no retry against a peer.
    pub(crate) fn admit(&self, cancel: &Cancellation) -> Result<Grant, FsError> {
        cancelled(cancel)?;
        let io = self.context.io_witness()?;
        let registry = &self.context.registry;
        let sequence = self.context.sequence;
        let batch = {
            let mut state = registry
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if !state
                .records
                .get(&sequence)
                .is_some_and(|record| record.planned)
            {
                return Err(failure(
                    io::ErrorKind::InvalidInput,
                    "Complete namespace roots must be planned before grant.",
                ));
            }
            if state
                .records
                .values()
                .any(|record| record.state == FactsState::Quarantined)
            {
                return Err(blocked_by_quarantine());
            }
            let mut batch = ProbeBatch::reserve(registry, &mut state, vec![sequence])?;
            // Admission probes every own pin, unclaimed ones too, so their errors surface.
            batch.snapshots[0].excluded.clear();
            batch
        };
        let refreshed = admitted(registry, &io, || {
            let snapshot = &batch.snapshots[0];
            // A root this job only reads whose geometry is unknown is never written by it,
            // so it cannot take part in a write-write overlap: it stays unclaimed instead
            // of making the whole job exclusive. An unknown root it changes still does.
            let (unprotected, pins): (Vec<_>, Vec<_>) =
                batch.refresh_one(snapshot)?.into_iter().partition(|pin| {
                    pin.root.is_some()
                        && pin.access == TransferNamespaceAccess::Read
                        && matches!(pin.probe, TransferNamespaceProbe::Unknown)
                });
            let facts = if unprotected.is_empty() {
                Facts::revalidated(&snapshot.prior, pins.clone())?
            } else {
                Facts::subset(pins.clone())?
            };
            let roots = pins
                .iter()
                .filter_map(|pin| pin.root.map(|root| CandidateRoot::new(root, pin)))
                .collect::<Result<Vec<_>, FsError>>()?;
            let unprotected: Vec<_> = unprotected.iter().filter_map(|pin| pin.root).collect();
            Ok::<_, FsError>((pins, facts, roots, unprotected))
        })?;
        drop(batch);
        let (pins, facts, roots, unprotected) = match refreshed {
            Ok(refreshed) => refreshed,
            Err(error) => {
                registry.opaque(sequence, error.clone());
                return Err(error);
            }
        };
        let mut state = registry
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if state
            .records
            .values()
            .any(|record| record.state == FactsState::Quarantined)
        {
            return Err(blocked_by_quarantine());
        }
        let record = state.records.get(&sequence).ok_or_else(unconfirmed)?;
        if !record.planned || record.state == FactsState::Closing {
            return Err(unconfirmed());
        }
        if record.granted {
            return Ok(Grant::Granted);
        }
        let opaque = record.opaque_latched || facts.is_opaque();
        let reads = record.reads.clone();
        let state_after = if opaque {
            FactsState::Opaque
        } else {
            FactsState::Stable
        };
        let deferred = match verdict(&state, sequence, opaque, &facts, &roots, &reads) {
            Ok(deferred) => deferred,
            Err(hold) => {
                // While it waits, younger candidates compare with this fresh footprint.
                let record = state
                    .records
                    .get_mut(&sequence)
                    .expect("candidate remains registered");
                record.state = state_after;
                let retired = std::mem::replace(&mut record.facts, Arc::new(facts));
                Inner::bump(&mut state, sequence)?;
                let seen = state.seen();
                registry.signal();
                drop(state);
                drop(retired);
                return Ok(Grant::Conflict(seen, hold));
            }
        };
        let (reserved_pins, claimed): (Vec<_>, Vec<_>) = pins
            .into_iter()
            .partition(|pin| pin.root.is_some_and(|root| deferred.contains(&root)));
        let claims = if deferred.is_empty() {
            facts
        } else {
            Facts::subset(claimed)?
        };
        let reserved = if reserved_pins.is_empty() {
            None
        } else {
            Some(Arc::new(Facts::subset(reserved_pins.clone())?))
        };
        let granted_at = state.grants.checked_add(1).ok_or_else(unconfirmed)?;
        state.grants = granted_at;
        let record = state
            .records
            .get_mut(&sequence)
            .expect("candidate remains registered");
        record.state = state_after;
        let retired = std::mem::replace(&mut record.facts, Arc::new(claims));
        record.granted = true;
        record.granted_at = granted_at;
        record.deferred = deferred;
        record.reserved_pins = reserved_pins;
        record.reserved = reserved;
        record.unprotected = unprotected;
        Inner::bump(&mut state, sequence)?;
        Inner::restructure(&mut state);
        registry.signal();
        drop(state);
        drop(retired);
        Ok(Grant::Granted)
    }
    /// The final full revalidation of a granted job's own pins, after its last native work
    /// and before its owners close. Opted-in turns refresh only what they announced, so an
    /// external change to an untouched pinned binding (a renamed source or destination
    /// parent) surfaces here: it is latched and reported as a job diagnostic, as it would
    /// have been by a full turn refresh. Never fails the teardown.
    pub(crate) fn revalidate_before_close(&self) {
        let registry = &self.context.registry;
        let sequence = self.context.sequence;
        let granted = registry
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .records
            .get(&sequence)
            .is_some_and(|record| record.granted && record.state != FactsState::Quarantined);
        if !granted {
            return;
        }
        let prepared = self
            .context
            .io_witness()
            .and_then(|io| Ok((io, ProbeBatch::single(registry, sequence)?)));
        let result = prepared.and_then(|(io, batch)| {
            // `refresh` latches and records its own probe failures.
            admitted(registry, &io, || {
                let _ = batch.refresh(&Cancellation::default(), true);
            })
        });
        if let Err(error) = result {
            registry.opaque(sequence, error);
        }
    }
    pub(crate) fn begin_closing(&self) -> Result<(), FsError> {
        let inner = &self.context.registry;
        let mut state = inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let record = state
            .records
            .get_mut(&self.context.sequence)
            .ok_or_else(unconfirmed)?;
        if record.state == FactsState::Quarantined {
            return Err(unconfirmed());
        }
        if record.state != FactsState::Closing {
            record.state = FactsState::Closing;
            Inner::bump(&mut state, self.context.sequence)?;
            inner.signal();
        }
        loop {
            let record = state
                .records
                .get(&self.context.sequence)
                .ok_or_else(unconfirmed)?;
            if record.state == FactsState::Quarantined {
                return Err(unconfirmed());
            }
            if record.probes == 0 && state.turn != Some(self.context.sequence) {
                return Ok(());
            }
            state = inner
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
    }
    pub(crate) fn complete_after_drop(&self) -> Result<(), FsError> {
        let inner = &self.context.registry;
        let mut state = inner
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let record = state
            .records
            .get(&self.context.sequence)
            .ok_or_else(unconfirmed)?;
        if record.state != FactsState::Closing
            || record.probes != 0
            || !record.native_closed
            || record.quarantined.load(Ordering::Acquire)
            || state.turn == Some(self.context.sequence)
        {
            return Err(unconfirmed());
        }
        Inner::bump(&mut state, self.context.sequence)?;
        let retired = state.records.remove(&self.context.sequence);
        Inner::restructure(&mut state);
        self.completed.store(true, Ordering::Release);
        inner.signal();
        drop(state);
        drop(retired);
        Ok(())
    }
    #[cfg(test)]
    fn alias_count(&self) -> usize {
        self.context
            .native
            .upgrade()
            .unwrap()
            .data
            .lock()
            .unwrap()
            .aliases
            .len()
    }
}
impl Drop for Claim {
    fn drop(&mut self) {
        if !self.completed.load(Ordering::Acquire) {
            self.quarantine();
        }
    }
}

impl NativeOwners {
    /// Only the independently audited transfer factory may qualify its native
    /// root/alias owners. Geometry Unknown neither grants nor revokes this proof.
    pub(crate) fn qualify(
        &mut self,
        witness: crate::transfer_composition::TransferRetirementWitness,
    ) {
        self.witness = Some(witness);
    }
    pub(crate) fn retirement_is_checked(&self) -> bool {
        self.witness.is_some()
            || self.store.as_ref().is_none_or(|store| {
                let data = store.data.lock().unwrap_or_else(|error| error.into_inner());
                data.roots
                    .iter()
                    .chain(data.aliases.values())
                    .all(|entry| entry.pin.is_none())
            })
    }
}
impl Drop for NativeOwners {
    fn drop(&mut self) {
        let Some(store) = self.store.take() else {
            return;
        };
        let safe = {
            let state = self
                .context
                .registry
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state
                .records
                .get(&self.context.sequence)
                .is_some_and(|record| {
                    record.state == FactsState::Closing
                        && record.probes == 0
                        && !record.quarantined.load(Ordering::Acquire)
                })
                && state.turn != Some(self.context.sequence)
        };
        if !safe {
            self.context.registry.quarantine(self.context.sequence);
            std::mem::forget(store);
            return;
        }
        let io = match self.context.io_witness() {
            Ok(io) => Some(io),
            Err(_) => {
                let has_native = {
                    let data = store.data.lock().unwrap_or_else(|error| error.into_inner());
                    data.roots
                        .iter()
                        .chain(data.aliases.values())
                        .any(|pin| pin.pin.is_some())
                };
                if has_native {
                    self.context.registry.quarantine(self.context.sequence);
                    std::mem::forget(store);
                    return;
                }
                None
            }
        };
        let store = match Arc::try_unwrap(store) {
            Ok(store) => store,
            Err(store) => {
                self.context.registry.quarantine(self.context.sequence);
                std::mem::forget(store);
                return;
            }
        };
        let data = store
            .data
            .into_inner()
            .unwrap_or_else(|error| error.into_inner());
        let mut pins = PinBatch(Some(
            data.roots
                .into_iter()
                .chain(data.aliases.into_values())
                .collect(),
        ));
        while let Some(pin) = pins.0.as_mut().expect("native batch remains owned").pop() {
            let dropped = checked(|| match &io {
                Some(io) => admitted(&self.context.registry, io, || drop(pin)),
                None => {
                    drop(pin);
                    Ok(())
                }
            })
            .and_then(|result| result);
            if dropped.is_err() {
                self.context.registry.quarantine(self.context.sequence);
                return;
            }
        }
        drop(pins.0.take());
        let mut state = self
            .context
            .registry
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(record) = state.records.get_mut(&self.context.sequence) {
            record.native_closed = true;
        }
        self.context.registry.signal();
    }
}
/// Remaining native owners never unwind out of a failed checked destruction.
struct PinBatch(Option<Vec<PinEntry>>);
impl Drop for PinBatch {
    fn drop(&mut self) {
        if let Some(pins) = self.0.take() {
            std::mem::forget(pins);
        }
    }
}

struct Snapshot {
    sequence: u64,
    native: Weak<PinStore>,
    io: Weak<Permit>,
    roots: Option<Vec<usize>>,
    aliases: Option<Vec<u64>>,
    /// Roots that are not part of this job's facts: deferred at grant and not claimed
    /// yet, or unprotected reads of unknown geometry.
    excluded: Vec<usize>,
    prior: Arc<Facts>,
}
struct ProbeBatch {
    registry: Arc<Inner>,
    snapshots: Vec<Snapshot>,
}
/// Fresh facts per record sequence, in snapshot order.
type Refreshed = Vec<(u64, Arc<Facts>)>;
impl ProbeBatch {
    fn reserve(
        registry: &Arc<Inner>,
        state: &mut State,
        sequences: Vec<u64>,
    ) -> Result<Self, FsError> {
        let mut snapshots = Vec::with_capacity(sequences.len());
        for sequence in sequences {
            let record = state.records.get_mut(&sequence).ok_or_else(unconfirmed)?;
            record.probes = record.probes.checked_add(1).ok_or_else(unconfirmed)?;
            snapshots.push(Snapshot {
                sequence,
                native: record.native.clone(),
                io: record.io.clone(),
                roots: None,
                aliases: None,
                excluded: record
                    .deferred
                    .iter()
                    .chain(&record.unprotected)
                    .copied()
                    .collect(),
                prior: record.facts.clone(),
            });
        }
        Ok(Self {
            registry: registry.clone(),
            snapshots,
        })
    }
    fn single(registry: &Arc<Inner>, sequence: u64) -> Result<Self, FsError> {
        let mut state = registry
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        Self::reserve(registry, &mut state, vec![sequence])
    }
    fn touched(
        registry: &Arc<Inner>,
        sequence: u64,
        roots: &HashSet<usize>,
        aliases: &HashSet<u64>,
    ) -> Result<Self, FsError> {
        let mut batch = Self::single(registry, sequence)?;
        batch.snapshots[0].roots = Some(roots.iter().copied().collect());
        batch.snapshots[0].aliases = Some(aliases.iter().copied().collect());
        Ok(batch)
    }
    fn refresh_touched(self, cancel: &Cancellation) -> Result<Vec<PinFact>, FsError> {
        cancelled(cancel)?;
        self.refresh_one(&self.snapshots[0])
    }
    fn refresh(self, cancel: &Cancellation, latch: bool) -> Result<Refreshed, FsError> {
        let mut facts = Vec::with_capacity(self.snapshots.len());
        for snapshot in &self.snapshots {
            cancelled(cancel)?;
            let result = self
                .refresh_one(snapshot)
                .and_then(|pins| Facts::revalidated(&snapshot.prior, pins));
            match result {
                Ok(value) => facts.push((snapshot.sequence, Arc::new(value))),
                Err(error) => {
                    if latch {
                        self.registry.opaque(snapshot.sequence, error.clone());
                    }
                    return Err(error);
                }
            }
        }
        Ok(facts)
    }
    fn refresh_one(&self, snapshot: &Snapshot) -> Result<Vec<PinFact>, FsError> {
        // Closing retains both resources until this counted probe has dropped
        // its native anchors and the actual configured I/O witness.
        let io = snapshot.io.upgrade().ok_or_else(unconfirmed)?;
        if !io.belongs_to(&self.registry.io) || !io.is_shared() {
            return Err(unconfirmed());
        }
        let store = snapshot.native.upgrade().ok_or_else(unconfirmed)?;
        let pins: Vec<_> = {
            let data = store.data.lock().unwrap_or_else(|error| error.into_inner());
            let excluded = |root: &usize| snapshot.excluded.contains(root);
            let mut pins = match &snapshot.roots {
                Some(roots) => roots
                    .iter()
                    .filter(|root| !excluded(root))
                    .map(|root| {
                        data.roots
                            .get(*root)
                            .cloned()
                            .map(|pin| (Some(*root), None, pin))
                            .ok_or_else(unconfirmed)
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                None => data
                    .roots
                    .iter()
                    .cloned()
                    .enumerate()
                    .filter(|(root, _)| !excluded(root))
                    .map(|(root, pin)| (Some(root), None, pin))
                    .collect(),
            };
            match &snapshot.aliases {
                Some(aliases) => {
                    for alias in aliases {
                        let pin = data.aliases.get(alias).cloned().ok_or_else(unconfirmed)?;
                        pins.push((None, Some(*alias), pin));
                    }
                }
                None => pins.extend(
                    data.aliases
                        .iter()
                        .map(|(alias, pin)| (None, Some(*alias), pin.clone())),
                ),
            }
            pins
        };
        // One batch: an adapter walks an ancestry shared by several pins once.
        let _batch = TransferNamespaceRefreshBatch::begin();
        let mut facts = Vec::with_capacity(pins.len());
        for (root, alias, pin) in &pins {
            let probe = match &pin.pin {
                None => TransferNamespaceProbe::Unknown,
                Some(native) => match checked(|| {
                    let result = native.refresh(&Cancellation::default());
                    #[cfg(test)]
                    if root.is_some()
                        && std::env::var_os("MIDNIGHT_TEST_TRACE_COPY_NAMESPACE").is_some()
                    {
                        let outcome = match &result {
                            Ok(TransferNamespaceProbe::Known(scopes)) => {
                                format!("Known(scopes={})", scopes.len())
                            }
                            Ok(TransferNamespaceProbe::Unknown) => "Unknown".to_owned(),
                            Err(error) => {
                                format!("Error(kind={:?}, message={})", error.kind, error.message)
                            }
                        };
                        eprintln!(
                            "scheduler delegated namespace sequence={} root={root:?} access={:?} refresh={outcome}",
                            snapshot.sequence, pin.access
                        );
                    }
                    result
                }) {
                    Ok(result) => result?,
                    Err(error) => {
                        self.registry.quarantine(snapshot.sequence);
                        return Err(error);
                    }
                },
            };
            facts.push(PinFact {
                root: *root,
                alias: *alias,
                access: pin.access,
                probe,
            });
        }
        drop(pins);
        drop(store);
        drop(io);
        Ok(facts)
    }
}
impl Drop for ProbeBatch {
    fn drop(&mut self) {
        let mut state = self
            .registry
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for snapshot in &self.snapshots {
            if let Some(record) = state.records.get_mut(&snapshot.sequence) {
                record.probes -= 1;
            }
        }
        self.registry.signal();
    }
}

struct Transaction {
    context: Arc<Context>,
    finished: bool,
    touched: HashSet<usize>,
    touched_aliases: HashSet<u64>,
    incremental: bool,
    full_refresh: bool,
}
impl TransferNamespaceChanges for Context {
    fn begin(&self, cancel: &Cancellation) -> Result<Box<dyn TransferNamespaceChange>, FsError> {
        let _io = self.io_witness()?;
        let mut state = self
            .registry
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        loop {
            cancelled(cancel)?;
            let record = state.records.get(&self.sequence).ok_or_else(unconfirmed)?;
            if record.state == FactsState::Quarantined {
                return Err(unconfirmed());
            }
            if !record.granted {
                return Err(failure(
                    io::ErrorKind::PermissionDenied,
                    "Namespace mutation requires the complete granted claim.",
                ));
            }
            if state.turn == Some(self.sequence) {
                return Err(failure(
                    io::ErrorKind::WouldBlock,
                    "Nested namespace helpers must borrow the existing transaction.",
                ));
            }
            if state.turn.is_none() {
                state.turn = Some(self.sequence);
                let record = state
                    .records
                    .get_mut(&self.sequence)
                    .expect("granted record remains registered");
                if record.state != FactsState::Closing {
                    record.state = FactsState::InFlight;
                }
                Inner::bump(&mut state, self.sequence)?;
                drop(state);
                return Ok(Box::new(Transaction {
                    context: Arc::new(self.clone_context()),
                    finished: false,
                    touched: HashSet::new(),
                    touched_aliases: HashSet::new(),
                    incremental: false,
                    full_refresh: false,
                }));
            }
            state = self
                .registry
                .changed
                .wait_timeout(state, POLL)
                .unwrap_or_else(|error| error.into_inner())
                .0;
        }
    }
}
impl Context {
    fn io_witness(&self) -> Result<Arc<Permit>, FsError> {
        let witness = {
            let state = self
                .registry
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            state
                .records
                .get(&self.sequence)
                .ok_or_else(unconfirmed)?
                .io
                .clone()
        };
        let Some(witness) = witness.upgrade() else {
            return Err(unconfirmed());
        };
        if !witness.belongs_to(&self.registry.io) || !witness.is_shared() {
            return Err(unconfirmed());
        }
        Ok(witness)
    }
    fn clone_context(&self) -> Self {
        Self {
            registry: self.registry.clone(),
            sequence: self.sequence,
            native: self.native.clone(),
            quarantined: self.quarantined.clone(),
            diagnostics: self.diagnostics.clone(),
        }
    }
}
impl Transaction {
    fn remove_aliases(&self, aliases: &[u64]) -> Result<(), FsError> {
        if aliases.is_empty() {
            return Ok(());
        }
        let mut state = self
            .context
            .registry
            .state
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        while state
            .records
            .get(&self.context.sequence)
            .ok_or_else(unconfirmed)?
            .probes
            != 0
        {
            state = self
                .context
                .registry
                .changed
                .wait(state)
                .unwrap_or_else(|error| error.into_inner());
        }
        drop(state);
        let store = self.context.native.upgrade().ok_or_else(unconfirmed)?;
        let mut removed = PinBatch(Some(Vec::with_capacity(aliases.len())));
        let mut inconsistent = false;
        {
            let mut data = store.data.lock().unwrap_or_else(|error| error.into_inner());
            for alias in aliases {
                if let Some(pin) = data.aliases.remove(alias) {
                    let identity = pin
                        .pin
                        .as_ref()
                        .map(|pin| Arc::as_ptr(pin) as *const () as usize);
                    // Arm native ownership before checking the pure index, so
                    // an inconsistent entry cannot drop a native pin under locks.
                    removed.0.as_mut().unwrap().push(pin);
                    if let Some(identity) = identity
                        && data.alias_identity.remove(&identity) != Some(*alias)
                    {
                        inconsistent = true;
                    }
                }
            }
        }
        if inconsistent {
            // PinBatch retains these actual owners on error. Their disappearance
            // from the store cannot become an ordinary opaque completion.
            self.context.registry.quarantine(self.context.sequence);
            return Err(unconfirmed());
        }
        while let Some(pin) = removed.0.as_mut().unwrap().pop() {
            if let Err(error) = checked(|| drop(pin)) {
                self.context.registry.quarantine(self.context.sequence);
                return Err(error);
            }
        }
        drop(removed.0.take());
        drop(store);
        Ok(())
    }
}
impl TransferNamespaceChange for Transaction {
    fn reserve_private_child(
        &mut self,
        mut pin: Option<Arc<dyn TransferNamespacePin>>,
        cancel: &Cancellation,
    ) -> Result<bool, FsError> {
        // The record describes this reservation only (see `take_peer_refusal`).
        clear_peer_refusal();
        let io = match self.context.io_witness() {
            Ok(io) => io,
            Err(error) => {
                self.context.registry.quarantine(self.context.sequence);
                std::mem::forget(pin);
                return Err(error);
            }
        };
        let registry = self.context.registry.clone();
        let mut inconsistent = false;
        let result = admitted(&registry, &io, || {
            cancelled(cancel)?;
            let store = self.context.native.upgrade().ok_or_else(unconfirmed)?;
            let identity = pin
                .as_ref()
                .map(|pin| Arc::as_ptr(pin) as *const () as usize);
            let (registered, private_alias, candidate_access, root_access) = {
                let data = store.data.lock().unwrap_or_else(|error| error.into_inner());
                let roots: Vec<usize> = identity
                    .and_then(|identity| data.root_identity.get(&identity))
                    .cloned()
                    .unwrap_or_default();
                let root_access: Vec<_> = roots
                    .iter()
                    .map(|root| (*root, data.roots[*root].access))
                    .collect();
                let private_alias =
                    identity.and_then(|identity| data.alias_identity.get(&identity).copied());
                if let Some(alias) = private_alias
                    && !data
                        .aliases
                        .get(&alias)
                        .and_then(|entry| entry.pin.as_ref())
                        .zip(pin.as_ref())
                        .is_some_and(|(stored, candidate)| Arc::ptr_eq(stored, candidate))
                {
                    inconsistent = true;
                    return Err(unconfirmed());
                }
                let access = if roots.is_empty()
                    || roots
                        .iter()
                        .any(|root| data.roots[*root].access == TransferNamespaceAccess::Write)
                {
                    TransferNamespaceAccess::Write
                } else {
                    TransferNamespaceAccess::Read
                };
                (roots, private_alias, access, root_access)
            };
            let probe = match &pin {
                None => TransferNamespaceProbe::Unknown,
                Some(pin) => match checked(|| pin.refresh(cancel)) {
                    Ok(result) => result?,
                    Err(error) => {
                        self.context.registry.quarantine(self.context.sequence);
                        return Err(error);
                    }
                },
            };
            if matches!(&probe, TransferNamespaceProbe::Known(scopes) if scopes.is_empty())
                && (private_alias.is_none() || !registered.is_empty())
            {
                return Err(failure(
                    io::ErrorKind::InvalidData,
                    "Only a retained private alias may announce a removal certificate.",
                ));
            }
            let fact = PinFact {
                root: None,
                alias: Some(0),
                access: candidate_access,
                probe,
            };
            let candidate = Facts::build(vec![fact.clone()])?;
            let (prior, mapping, mut state) = loop {
                cancelled(cancel)?;
                let (epoch, prior, own_opaque, peers) = {
                    let state = self
                        .context
                        .registry
                        .state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    let record = state
                        .records
                        .get(&self.context.sequence)
                        .ok_or_else(unconfirmed)?;
                    if state.turn != Some(self.context.sequence)
                        || record.state == FactsState::Quarantined
                    {
                        return Err(unconfirmed());
                    }
                    let own = record.granted_at;
                    let peers: Vec<_> = state
                        .records
                        .iter()
                        .filter(|(sequence, record)| {
                            **sequence != self.context.sequence && record.granted
                        })
                        .map(|(sequence, record)| {
                            (
                                Blocker {
                                    sequence: *sequence,
                                    revision: record.revision,
                                    latched: record.opaque_latched,
                                    // Decided below, for a peer that refuses the change.
                                    unknown: false,
                                },
                                record.facts.clone(),
                                // Items a job deferred bind only the jobs granted after it.
                                record.reserved.clone().filter(|_| record.granted_at < own),
                                record.state,
                            )
                        })
                        .collect();
                    (
                        state.revision,
                        record.facts.clone(),
                        record.facts.is_opaque() || record.opaque_latched,
                        peers,
                    )
                };
                #[cfg(test)]
                tests::after_reservation_snapshot();
                let unsupported = candidate.is_opaque() && (!peers.is_empty() || !own_opaque);
                // Closing retains its facts and owners. Its cleanup cannot change
                // names while this transaction holds the mutation turn.
                let blockers: Vec<_> = peers
                    .iter()
                    .filter(|(blocker, facts, reserved, state)| {
                        blocker.latched
                            || *state == FactsState::Quarantined
                            || candidate.conflicts(facts)
                            || reserved
                                .as_ref()
                                .is_some_and(|reserved| candidate.conflicts(reserved))
                    })
                    .map(|(blocker, facts, _, state)| {
                        let unknown = blocker.latched || facts.is_opaque();
                        (
                            Blocker {
                                unknown,
                                ..*blocker
                            },
                            *state,
                        )
                    })
                    .collect();
                let conflicts = !blockers.is_empty();
                let mapping = if unsupported || conflicts {
                    None
                } else {
                    prior.announcement(&fact)
                };
                let state = self
                    .context
                    .registry
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if state.turn != Some(self.context.sequence)
                    || state
                        .records
                        .get(&self.context.sequence)
                        .is_none_or(|record| record.state == FactsState::Quarantined)
                {
                    return Err(unconfirmed());
                }
                if state.revision != epoch {
                    // Only host metadata is re-read; retain the same candidate
                    // pin and I/O witness without waiting or replaying publication.
                    continue;
                }
                if unsupported {
                    return Err(failure(
                        io::ErrorKind::Unsupported,
                        "An unknown announcement requires the already exclusive opaque lane.",
                    ));
                }
                if conflicts {
                    // The adapter reports this as an ordinary error after the turn; the
                    // async owner waits for one of these peers to change instead of
                    // asking the user. The unchanged epoch keeps their revisions current.
                    let refusal = if blockers
                        .iter()
                        .any(|(_, state)| *state == FactsState::Quarantined)
                    {
                        Refusal::Quarantined
                    } else {
                        Refusal::Peer(Blockers(
                            blockers.into_iter().map(|(blocker, _)| blocker).collect(),
                        ))
                    };
                    PEER_REFUSAL.with(|slot| slot.set(Some(refusal)));
                    return Ok(false);
                }
                break (prior, mapping, state);
            };
            // A root deferred at grant becomes this job's claim with this first change of
            // it that no earlier job refused, before the change mutates anything.
            let record = state
                .records
                .get_mut(&self.context.sequence)
                .ok_or_else(unconfirmed)?;
            let claimed: Vec<_> = root_access
                .iter()
                .filter(|(root, _)| record.deferred.contains(root))
                .copied()
                .collect();
            if !claimed.is_empty() {
                let claimed_root = |root: &usize| claimed.iter().any(|(claim, _)| claim == root);
                record.deferred.retain(|root| !claimed_root(root));
                record
                    .reserved_pins
                    .retain(|pin| !pin.root.as_ref().is_some_and(claimed_root));
                record.reserved = if record.reserved_pins.is_empty() {
                    None
                } else {
                    Some(Arc::new(Facts::subset(record.reserved_pins.clone())?))
                };
                prior.update(
                    claimed
                        .iter()
                        .map(|(root, access)| PinFact {
                            root: Some(*root),
                            alias: None,
                            access: *access,
                            probe: fact.probe.clone(),
                        })
                        .collect(),
                )?;
            }
            // Only the exact original registered capability opts in. Existing
            // helpers which announce fresh temporary children refresh every pin of the
            // job when they finish (see `finish`). An opted-in helper must announce
            // EVERY affected original/private binding before mutation, including
            // displaced/recovery bindings. Intersection locates affected roots; it is
            // never a substitute for full coverage. Grants compare these committed facts
            // without probing this job again; untouched pins are revalidated before close.
            if !registered.is_empty() && !candidate.is_opaque() {
                self.incremental = true;
            }
            self.touched.extend(registered.iter().copied());
            if let Some(alias) = private_alias {
                self.touched_aliases.insert(alias);
            }
            match mapping {
                Some(roots) => self.touched.extend(roots),
                None => self.full_refresh = true,
            }
            // Geometry is not ownership: even an equal current object may be
            // replaced during this transaction. Keep every distinct observer;
            // only the exact Arc already in the live store can be reused.
            if registered.is_empty() && private_alias.is_none() {
                let alias = {
                    let mut data = store.data.lock().unwrap_or_else(|error| error.into_inner());
                    let alias = data.next_alias.checked_add(1).ok_or_else(unconfirmed)?;
                    match data.aliases.entry(alias) {
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            entry.insert(PinEntry {
                                access: TransferNamespaceAccess::Write,
                                pin: pin.take(),
                            });
                            if let Some(identity) = identity
                                && data.alias_identity.insert(identity, alias).is_some()
                            {
                                inconsistent = true;
                                return Err(unconfirmed());
                            }
                        }
                        std::collections::btree_map::Entry::Occupied(_) => {
                            inconsistent = true;
                            return Err(unconfirmed());
                        }
                    }
                    data.next_alias = alias;
                    alias
                };
                self.touched_aliases.insert(alias);
                prior.update(vec![PinFact {
                    alias: Some(alias),
                    access: TransferNamespaceAccess::Write,
                    ..fact
                }])?;
            }
            Inner::bump(&mut state, self.context.sequence)?;
            drop(state);
            drop(store);
            Ok(true)
        })
        .and_then(|result| result);
        if inconsistent {
            // All native/registry locks and the synchronous native context have
            // returned. Corrupt indexes cannot establish which owner is safe to
            // destroy: retain both any inserted pin and the unconsumed input.
            self.context.registry.quarantine(self.context.sequence);
            std::mem::forget(pin);
            return Err(unconfirmed());
        }
        // Rejected prospective pins are also actual native owners. Their last
        // destructor returns before the acquired probe witness is released.
        if let Err(error) = checked(|| admitted(&self.context.registry, &io, || drop(pin)))
            .and_then(|result| result)
        {
            self.context.registry.quarantine(self.context.sequence);
            return Err(error);
        }
        drop(io);
        result
    }
    fn finish(mut self: Box<Self>) -> Result<(), FsError> {
        let result = (|| {
            let io = self.context.io_witness()?;
            admitted(&self.context.registry, &io, || {
                // Grants and other jobs' changes compare these committed facts without
                // probing this job, so they must cover everything this turn changed. A
                // turn that announced every affected binding (opted in) refreshes only
                // those, not every pin the job has accumulated; any other turn refreshes
                // all of them. An external change to an untouched pin is reported by the
                // job's final revalidation (`Claim::revalidate_before_close`).
                let incremental = self.incremental && !self.full_refresh;
                let batch = if incremental {
                    ProbeBatch::touched(
                        &self.context.registry,
                        self.context.sequence,
                        &self.touched,
                        &self.touched_aliases,
                    )?
                } else {
                    ProbeBatch::single(&self.context.registry, self.context.sequence)?
                };
                let pins = batch.refresh_touched(&Cancellation::default())?;
                let removed: Vec<_> = pins
                    .iter()
                    .filter_map(|pin| match &pin.probe {
                        TransferNamespaceProbe::Known(scopes) if scopes.is_empty() => pin.alias,
                        _ => None,
                    })
                    .collect();
                self.remove_aliases(&removed)?;
                let (previous, revision) = {
                    let state = self
                        .context
                        .registry
                        .state
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    let record = state
                        .records
                        .get(&self.context.sequence)
                        .ok_or_else(unconfirmed)?;
                    if record.state == FactsState::Quarantined
                        || state.turn != Some(self.context.sequence)
                    {
                        return Err(unconfirmed());
                    }
                    (record.facts.clone(), record.revision)
                };
                // Index construction/updates and disposal of temporary metadata run
                // outside the shared registry mutex. InFlight prevents any grant
                // from observing this record until the checked commit below.
                let replacement = if incremental {
                    previous.update(pins)?;
                    None
                } else {
                    let removed: HashSet<_> = removed.into_iter().collect();
                    Some(Arc::new(Facts::revalidated(
                        &previous,
                        pins.into_iter()
                            .filter(|pin| !pin.alias.is_some_and(|alias| removed.contains(&alias)))
                            .collect(),
                    )?))
                };
                let mut state = self
                    .context
                    .registry
                    .state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                if state.turn != Some(self.context.sequence) {
                    return Err(unconfirmed());
                }
                let record = state
                    .records
                    .get_mut(&self.context.sequence)
                    .ok_or_else(unconfirmed)?;
                if record.state == FactsState::Quarantined || record.revision != revision {
                    return Err(unconfirmed());
                }
                let retired = replacement.map(|facts| std::mem::replace(&mut record.facts, facts));
                if record.state != FactsState::Closing {
                    record.state = if record.facts.is_opaque() || record.opaque_latched {
                        FactsState::Opaque
                    } else {
                        FactsState::Stable
                    };
                }
                Inner::bump(&mut state, self.context.sequence)?;
                drop(state);
                drop(retired);
                Ok(())
            })?
        })();
        if let Err(error) = &result {
            // Preserve native success separately; this failure is latched before
            // returning to the helper which reports its dual outcome.
            self.context
                .registry
                .opaque(self.context.sequence, error.clone());
        }
        {
            let mut state = self
                .context
                .registry
                .state
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some(record) = state.records.get_mut(&self.context.sequence)
                && record.state == FactsState::InFlight
            {
                record.state = FactsState::Opaque;
            }
            if state.turn == Some(self.context.sequence) {
                state.turn = None;
            }
            self.context.registry.signal();
        }
        self.finished = true;
        result
    }
}
impl Drop for Transaction {
    fn drop(&mut self) {
        if !self.finished {
            self.context.registry.quarantine(self.context.sequence);
        }
    }
}

#[cfg(test)]
#[path = "copy_namespace_tests.rs"]
mod tests;
