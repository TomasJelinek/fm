//! Native binding witnesses. Immutable parent authorities are pooled across jobs; owned
//! namespace transitions are shared only by the weak identity of the installed job adapter.
use super::*;
use midnight_core::{
    TransferNamespaceExtent, TransferNamespaceParent, TransferNamespacePin, TransferNamespaceProbe,
    TransferNamespaceScope,
};
use midnight_platform_common::transfer_namespace::{
    TransferNamespaceChange, TransferNamespaceChanges, TransferNamespaceRefreshBatch,
    current_transfer_namespace_changes, with_transfer_namespace_change,
};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    ffi::{OsStr, OsString},
    os::windows::ffi::OsStrExt,
    path::PathBuf,
    sync::{
        Mutex, OnceLock, Weak,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use windows_sys::Wdk::Storage::FileSystem::FILE_OPEN;

#[path = "transfer_namespace_parent.rs"]
mod parent;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Slot {
    parent: TransferIdentity,
    key: Arc<[u8]>,
}
#[derive(Clone)]
struct Binding {
    parent: Arc<parent::Parent>,
    parent_path: PathBuf,
    name: OsString,
    slot: Slot,
    object: Option<TransferIdentity>,
    revision: Option<destination::recovery::Revision>,
    original: bool,
    delete_pending: bool,
}
struct Pin {
    bindings: Mutex<Vec<Binding>>,
    extent: TransferNamespaceExtent,
    original: bool,
    job: Option<Arc<Job>>,
}
#[derive(Default)]
struct Index {
    slots: HashMap<Slot, HashMap<usize, Weak<Pin>>>,
    pins: HashMap<usize, PinIndex>,
    roots: BTreeMap<usize, Weak<Pin>>,
    sentinel: Option<usize>,
    objects: HashMap<TransferIdentity, HashMap<usize, Weak<Pin>>>,
    parents: HashMap<TransferIdentity, ParentPins>,
    ancestors: HashMap<TransferIdentity, HashSet<TransferIdentity>>,
}
struct ParentPins {
    facts: Arc<TransferNamespaceParent>,
    pins: HashMap<usize, Weak<Pin>>,
}
struct RelatedPin {
    pin: Weak<Pin>,
    subtree: bool,
    covers: bool,
}
struct PinIndex {
    weak: Weak<Pin>,
    slots: HashSet<Slot>,
    owners: HashMap<TransferIdentity, usize>,
    objects: HashSet<TransferIdentity>,
    parents: HashSet<TransferIdentity>,
}
struct Job {
    adapter: Weak<dyn TransferNamespaceChanges>,
    index: Mutex<Index>,
    failure: Mutex<Option<FsError>>,
    owners: Mutex<HashMap<TransferIdentity, OwnerRecord>>,
    removed_directories: Mutex<HashSet<TransferIdentity>>,
}
#[derive(Default)]
struct OwnerRecord {
    references: usize,
    completed: bool,
    states: HashMap<usize, Arc<OwnerState>>,
}
struct OwnerState {
    objects: Arc<[TransferIdentity]>,
    remaining: AtomicUsize,
    /// Observed only by tests; production decisions use `Job::owners_closed`.
    #[cfg(test)]
    closed: AtomicBool,
    unconfirmed: AtomicBool,
}
/// This field MUST be last in each native owner: Rust drops its File/Arc/guard fields
/// first. It retains metadata only. Clones count separately, so a surviving directory
/// clone or returned readback guard cannot certify another owner's final close.
pub(super) struct NativeOwner {
    _job: Option<Arc<Job>>,
    state: Option<Arc<OwnerState>>,
}
impl Clone for NativeOwner {
    fn clone(&self) -> Self {
        if let Some(state) = &self.state {
            state.remaining.fetch_add(1, Ordering::Relaxed);
        }
        Self {
            _job: self._job.clone(),
            state: self.state.clone(),
        }
    }
}
impl Drop for NativeOwner {
    fn drop(&mut self) {
        if let Some(state) = &self.state {
            if std::thread::panicking() {
                state.unconfirmed.store(true, Ordering::Release);
            }
            if state.remaining.fetch_sub(1, Ordering::AcqRel) == 1
                && !state.unconfirmed.load(Ordering::Acquire)
            {
                #[cfg(test)]
                state.closed.store(true, Ordering::Release);
                if let Some(job) = &self._job {
                    for object in state.objects.iter() {
                        job.close_owner(*object, Arc::as_ptr(state) as usize);
                    }
                }
            }
        }
    }
}
pub(super) fn owner(objects: &[TransferIdentity]) -> NativeOwner {
    let Some(job) = current_job() else {
        return NativeOwner {
            _job: None,
            state: None,
        };
    };
    let state = Arc::new(OwnerState {
        objects: objects.into(),
        remaining: AtomicUsize::new(1),
        #[cfg(test)]
        closed: AtomicBool::new(false),
        unconfirmed: AtomicBool::new(false),
    });
    {
        let mut owners = job.owners.lock().unwrap_or_else(|e| e.into_inner());
        for object in objects {
            owners
                .entry(*object)
                .or_default()
                .states
                .insert(Arc::as_ptr(&state) as usize, state.clone());
        }
    }
    NativeOwner {
        _job: Some(job),
        state: Some(state),
    }
}
pub(super) fn owner_for(file: &File, path: &Path) -> NativeOwner {
    let Some(job) = current_job() else {
        return NativeOwner {
            _job: None,
            state: None,
        };
    };
    match identity(file, path) {
        Ok(identity) => owner(&[identity]),
        Err(error) => {
            // Native creation already succeeded. Retain the diagnostic for finish/host
            // teardown instead of losing the created object through a constructor Err.
            job.fail(error);
            NativeOwner {
                _job: Some(job),
                state: None,
            }
        }
    }
}
pub(super) fn try_owner_for(file: &File, path: &Path) -> Result<NativeOwner, FsError> {
    if current_job().is_none() {
        return Ok(NativeOwner {
            _job: None,
            state: None,
        });
    }
    identity(file, path).map(|identity| owner(&[identity]))
}
/// A folder's pooled parent authority, kept while a job changes names in it: each change
/// there finds it pooled instead of capturing the folder's ancestry again (a Copy job's
/// staging folder, whose stages are published out of it one by one).
pub(super) struct HeldParent {
    _parent: Arc<parent::Parent>,
}
pub(super) fn hold_parent(
    file: &File,
    path: &Path,
    cancel: &Cancellation,
) -> Result<HeldParent, FsError> {
    parent::capture(file, path, cancel).map(|parent| HeldParent { _parent: parent })
}
// The address is only a private cache bucket, never operation authority. Weak::ptr_eq
// verifies the actual Arc allocation, including when a retired address is reused.
static JOBS: OnceLock<Mutex<HashMap<usize, Weak<Job>>>> = OnceLock::new();
fn current_job() -> Option<Arc<Job>> {
    let adapter = current_transfer_namespace_changes()?;
    let weak = Arc::downgrade(&adapter);
    let key = Arc::as_ptr(&adapter) as *const () as usize;
    let mut jobs = JOBS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if let Some(job) = jobs.get(&key).and_then(Weak::upgrade)
        && Weak::ptr_eq(&job.adapter, &weak)
    {
        return Some(job);
    }
    let job = Arc::new(Job {
        adapter: weak,
        index: Mutex::new(Index::default()),
        failure: Mutex::new(None),
        owners: Mutex::new(HashMap::new()),
        removed_directories: Mutex::new(HashSet::new()),
    });
    jobs.insert(key, Arc::downgrade(&job));
    Some(job)
}
impl Drop for Job {
    fn drop(&mut self) {
        let key = self.adapter.as_ptr() as *const () as usize;
        if let Some(jobs) = JOBS.get() {
            let mut jobs = jobs.lock().unwrap_or_else(|e| e.into_inner());
            if jobs
                .get(&key)
                .is_some_and(|job| std::ptr::eq(job.as_ptr(), self))
            {
                jobs.remove(&key);
            }
        }
    }
}
impl Job {
    fn owners_closed(&self, object: TransferIdentity) -> bool {
        self.owners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&object)
            .is_some_and(|owners| owners.completed && owners.states.is_empty())
    }
    fn close_owner(&self, object: TransferIdentity, state: usize) {
        let mut owners = self.owners.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(record) = owners.get_mut(&object) {
            // Called only by the last normally destroyed member of this actual native
            // owner group. Sticky unconfirmed groups never reach this path.
            record.states.remove(&state);
            record.completed = true;
            if record.references == 0 && record.states.is_empty() {
                owners.remove(&object);
                self.removed_directories
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .remove(&object);
            }
        }
    }
    fn removed_parent(&self, parent: TransferIdentity) -> bool {
        self.removed_directories
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&parent)
    }
    fn fail(&self, error: FsError) {
        let mut failure = self.failure.lock().unwrap_or_else(|e| e.into_inner());
        if failure.is_none() {
            *failure = Some(error);
        }
    }
    fn register(&self, pin: &Arc<Pin>, _binding: &Binding) {
        let bindings = pin.bindings.lock().unwrap_or_else(|e| e.into_inner());
        self.sync_pin(
            Arc::as_ptr(pin) as usize,
            &bindings,
            Some(Arc::downgrade(pin)),
            pin.original,
        );
    }
    // The caller holds this pin's bindings lock (or exclusively owns its Drop).
    // Track direct parent and object references so a removed parent's compact close
    // proof outlives any child alias that still needs it. Work is local to this pin.
    fn sync_pin(&self, key: usize, bindings: &[Binding], weak: Option<Weak<Pin>>, original: bool) {
        let mut slots = HashSet::new();
        let mut references = HashMap::<TransferIdentity, usize>::new();
        let mut objects = HashSet::new();
        let mut parents = HashMap::new();
        for binding in bindings {
            slots.insert(binding.slot.clone());
            *references.entry(binding.slot.parent).or_default() += 1;
            if let Some(object) = binding.object {
                *references.entry(object).or_default() += 1;
                objects.insert(object);
            }
            parents.insert(binding.slot.parent, binding.parent.snapshot.clone());
        }
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let previous = index.pins.remove(&key);
        if let Some(previous) = &previous {
            for slot in previous.slots.difference(&slots) {
                let empty = index.slots.get_mut(slot).is_some_and(|pins| {
                    pins.remove(&key);
                    pins.is_empty()
                });
                if empty {
                    index.slots.remove(slot);
                }
            }
        }
        let weak = weak.or_else(|| previous.as_ref().map(|pin| pin.weak.clone()));
        if let Some(weak) = &weak {
            for slot in &slots {
                index
                    .slots
                    .entry(slot.clone())
                    .or_default()
                    .insert(key, weak.clone());
            }
        }
        if let Some(previous) = &previous {
            for object in previous.objects.difference(&objects) {
                let empty = index.objects.get_mut(object).is_some_and(|pins| {
                    pins.remove(&key);
                    pins.is_empty()
                });
                if empty {
                    index.objects.remove(object);
                }
            }
            for parent in previous
                .parents
                .iter()
                .filter(|parent| !parents.contains_key(*parent))
            {
                let empty = index.parents.get_mut(parent).is_some_and(|group| {
                    group.pins.remove(&key);
                    group.pins.is_empty()
                });
                if empty {
                    let group = index.parents.remove(parent).expect("indexed native parent");
                    for ancestor in group.facts.ancestors.iter() {
                        let empty = index.ancestors.get_mut(ancestor).is_some_and(|parents| {
                            parents.remove(parent);
                            parents.is_empty()
                        });
                        if empty {
                            index.ancestors.remove(ancestor);
                        }
                    }
                }
            }
        }
        if let Some(weak) = &weak {
            for object in &objects {
                index
                    .objects
                    .entry(*object)
                    .or_default()
                    .insert(key, weak.clone());
            }
            for (parent, facts) in &parents {
                if !index.parents.contains_key(parent) {
                    for ancestor in facts.ancestors.iter() {
                        index
                            .ancestors
                            .entry(*ancestor)
                            .or_default()
                            .insert(*parent);
                    }
                    index.parents.insert(
                        *parent,
                        ParentPins {
                            facts: facts.clone(),
                            pins: HashMap::new(),
                        },
                    );
                }
                index
                    .parents
                    .get_mut(parent)
                    .expect("indexed native parent")
                    .pins
                    .insert(key, weak.clone());
            }
            if original && !bindings.is_empty() {
                index.roots.insert(key, weak.clone());
                if index.sentinel.is_none() {
                    index.sentinel = Some(key);
                }
            }
        }
        if bindings.is_empty() {
            index.roots.remove(&key);
            if index.sentinel == Some(key) {
                index.sentinel = index.roots.first_key_value().map(|(key, _)| *key);
            }
        }
        let mut owners = self.owners.lock().unwrap_or_else(|e| e.into_inner());
        let mut retired = Vec::new();
        if let Some(previous) = &previous {
            for (object, old) in &previous.owners {
                let next = references.get(object).copied().unwrap_or_default();
                if next < *old {
                    let record = owners
                        .get_mut(object)
                        .expect("registered native owner reference");
                    record.references -= old - next;
                    if record.references == 0 && record.states.is_empty() {
                        owners.remove(object);
                        retired.push(*object);
                    }
                }
            }
        }
        for (object, next) in &references {
            let old = previous
                .as_ref()
                .and_then(|pin| pin.owners.get(object))
                .copied()
                .unwrap_or_default();
            if *next > old {
                owners.entry(*object).or_default().references += next - old;
            }
        }
        if !retired.is_empty() {
            let mut removed = self
                .removed_directories
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            for object in retired {
                removed.remove(&object);
            }
        }
        if !bindings.is_empty() {
            index.pins.insert(
                key,
                PinIndex {
                    weak: weak.expect("registered native pin"),
                    slots,
                    owners: references,
                    objects,
                    parents: parents.into_keys().collect(),
                },
            );
        }
    }
    fn announcement_pins(&self, bindings: &[Binding], private: bool) -> Option<Vec<Arc<Pin>>> {
        if bindings
            .iter()
            .any(|binding| binding.parent.mode.key(&binding.name).is_none())
        {
            return None;
        }
        let (sentinel, related) = {
            let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
            let sentinel = index
                .sentinel
                .and_then(|key| index.roots.get(&key))
                .cloned()?;
            let mut related = Vec::with_capacity(bindings.len());
            for binding in bindings {
                if index
                    .parents
                    .get(&binding.slot.parent)
                    .is_some_and(|group| {
                        group.facts.lookup_domain != binding.parent.snapshot.lookup_domain
                            || group.facts.ancestors != binding.parent.snapshot.ancestors
                    })
                {
                    return None;
                }
                let mut pins = Vec::new();
                pins.extend(
                    index
                        .slots
                        .get(&binding.slot)
                        .into_iter()
                        .flat_map(|pins| pins.values())
                        .map(|pin| RelatedPin {
                            pin: pin.clone(),
                            subtree: false,
                            covers: true,
                        }),
                );
                if let Some(object) = binding.object {
                    pins.extend(
                        index
                            .objects
                            .get(&object)
                            .into_iter()
                            .flat_map(|pins| pins.values())
                            .map(|pin| RelatedPin {
                                pin: pin.clone(),
                                subtree: false,
                                covers: true,
                            }),
                    );
                    // One group per retained parent, not one entry per sibling for
                    // every ancestor. Directory changes also affect private descendants.
                    for parent in index.ancestors.get(&object).into_iter().flatten() {
                        if let Some(group) = index.parents.get(parent) {
                            pins.extend(group.pins.values().map(|pin| RelatedPin {
                                pin: pin.clone(),
                                subtree: false,
                                covers: false,
                            }));
                        }
                    }
                }
                for ancestor in binding.parent.snapshot.ancestors.iter() {
                    pins.extend(
                        index
                            .objects
                            .get(ancestor)
                            .into_iter()
                            .flat_map(|pins| pins.values())
                            .map(|pin| RelatedPin {
                                pin: pin.clone(),
                                subtree: true,
                                covers: true,
                            }),
                    );
                }
                related.push(pins);
            }
            (sentinel, related)
        };
        // Upgrade/probe/drop native capabilities only after the metadata lock is gone.
        let sentinel = sentinel.upgrade()?;
        if !sentinel.original {
            return None;
        }
        let mut announcements = BTreeMap::new();
        announcements.insert(Arc::as_ptr(&sentinel) as usize, sentinel);
        for pins in related {
            let mut covered = private;
            for related in pins {
                let Some(pin) = related.pin.upgrade() else {
                    continue;
                };
                if related.subtree && !matches!(pin.extent, TransferNamespaceExtent::Subtree) {
                    continue;
                }
                if pin
                    .job
                    .as_ref()
                    .is_none_or(|job| !std::ptr::eq(job.as_ref(), self))
                {
                    return None;
                }
                covered |= related.covers;
                // Preserve the exact allocation originally handed to the host. Its
                // alias identity lookup must reuse the retained private claim.
                announcements.insert(Arc::as_ptr(&pin) as usize, pin);
            }
            if !covered {
                return None;
            }
        }
        Some(announcements.into_values().collect())
    }
    fn affected(&self, slots: &[&Slot]) -> Vec<Arc<Pin>> {
        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let mut result = HashMap::new();
        for slot in slots {
            for (key, weak) in index
                .slots
                .get(*slot)
                .into_iter()
                .flat_map(|pins| pins.iter())
            {
                if let Some(pin) = weak.upgrade() {
                    result.insert(*key, pin);
                }
            }
        }
        result.into_values().collect()
    }
    fn created(&self, binding: Binding) {
        if let Some(object) = binding.object {
            self.removed_directories
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&object);
        }
        for pin in self.affected(&[&binding.slot]) {
            let changed = {
                let mut bindings = pin.bindings.lock().unwrap_or_else(|e| e.into_inner());
                bindings
                    .iter_mut()
                    .find(|old| old.slot == binding.slot && old.object.is_none())
                    .map(|old| {
                        old.object = binding.object;
                        old.revision = None;
                        old.delete_pending = false;
                        old.clone()
                    })
            };
            if let Some(changed) = changed {
                self.register(&pin, &changed);
            }
        }
    }
    fn renamed(&self, old: &Binding, new: Binding) {
        let Some(object) = old.object else {
            return;
        };
        for pin in self.affected(&[&old.slot, &new.slot]) {
            // A successful case-only rename can leave the old spelling resolving to the
            // same object, including on an unqualified alias mode. Probe the original
            // spelling instead of manufacturing case or 8.3 equivalence.
            let previous = pin
                .bindings
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            let observations = previous
                .into_iter()
                .filter(|binding| binding.slot == old.slot && binding.object == Some(object))
                .map(|binding| {
                    let visible = native::open_relative(
                        binding.parent.file(),
                        &binding.name,
                        FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                        FILE_OPEN,
                        0,
                    );
                    let remains = match visible {
                        Ok(file) => match identity(&file, &binding.parent_path.join(&binding.name))
                        {
                            Ok(identity) => identity == object,
                            Err(error) => {
                                self.fail(error);
                                true
                            }
                        },
                        Err(error) if error.kind() == io::ErrorKind::NotFound => false,
                        Err(failure) => {
                            self.fail(error(&binding.parent_path.join(&binding.name), failure));
                            true
                        }
                    };
                    (binding.name, remains)
                })
                .collect::<Vec<_>>();
            let changed = {
                let mut bindings = pin.bindings.lock().unwrap_or_else(|e| e.into_inner());
                let mut owned = false;
                let mut source_revision = false;
                for binding in bindings.iter_mut() {
                    if binding.slot == old.slot && binding.object == Some(object) {
                        owned = true;
                        source_revision |= binding.revision.is_some();
                        let remains = observations
                            .iter()
                            .find(|(name, _)| name == &binding.name)
                            .is_some_and(|(_, remains)| *remains);
                        if remains {
                            if binding.revision.is_some() {
                                binding.revision = new.revision;
                            }
                        } else {
                            binding.object = None;
                            binding.revision = None;
                        }
                        binding.delete_pending = false;
                    }
                }
                if let Some(target) = bindings.iter_mut().find(|binding| {
                    binding.slot == new.slot
                        && (binding.object.is_none() || binding.object == Some(object))
                }) {
                    // The same job's exact destination slot changes only after native success.
                    target.object = Some(object);
                    target.delete_pending = false;
                    target.revision = if source_revision { new.revision } else { None };
                    Some(target.clone())
                } else if owned {
                    let mut alias = new.clone();
                    if !source_revision {
                        alias.revision = None;
                    }
                    bindings.push(alias.clone());
                    Some(alias)
                } else {
                    None
                }
            };
            if let Some(changed) = changed {
                self.register(&pin, &changed);
            }
            pin.prune(|binding| {
                binding.original || binding.object.is_some() || binding.slot == new.slot
            });
        }
    }
    fn removed(&self, old: &Binding, directory: bool) {
        for pin in self.affected(&[&old.slot]) {
            let mut bindings = pin.bindings.lock().unwrap_or_else(|e| e.into_inner());
            for binding in bindings.iter_mut() {
                if binding.slot == old.slot && binding.object == old.object {
                    binding.delete_pending = true;
                    binding.revision = None;
                }
            }
        }
        if directory && let Some(object) = old.object {
            self.removed_directories
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(object);
        }
    }
}

fn slot(parent: &parent::Parent, name: &OsStr) -> Slot {
    let key = parent.mode.key(name).unwrap_or_else(|| {
        name.encode_wide()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>()
            .into()
    });
    Slot {
        parent: parent.snapshot.identity,
        key,
    }
}
fn binding(
    directory: Arc<parent::Parent>,
    parent_path: &Path,
    name: &OsStr,
    cancel: &Cancellation,
) -> Result<Binding, FsError> {
    checked(cancel, parent_path)?;
    validate_name(name).map_err(|e| error(parent_path, e))?;
    let path = parent_path.join(name);
    let (object, actual_name) = match native::open_relative(
        directory.file(),
        name,
        FILE_READ_ATTRIBUTES | SYNCHRONIZE,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        FILE_OPEN,
        0,
    ) {
        Ok(file) => {
            let object = identity(&file, &path)?;
            let actual = parent::physical_path(&file).map_err(|e| error(&path, e))?;
            let actual = actual
                .file_name()
                .ok_or_else(|| invalid(&path, "native namespace leaf has no basename"))?
                .to_owned();
            (Some(object), actual)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => (None, name.to_owned()),
        Err(e) => return Err(error(&path, e)),
    };
    Ok(Binding {
        slot: slot(&directory, &actual_name),
        parent: directory,
        parent_path: parent_path.to_owned(),
        name: name.to_owned(),
        object,
        revision: None,
        original: false,
        delete_pending: false,
    })
}

pub(super) fn child_pin(
    file: &File,
    path: &Path,
    name: &OsStr,
    kind: EntryKind,
    cancel: &Cancellation,
) -> Result<Arc<dyn TransferNamespacePin>, FsError> {
    let directory = parent::capture(file, path, cancel)?;
    let binding = binding(directory, path, name, cancel)?;
    Ok(make_pin(binding, kind, true, current_job()))
}
pub(super) fn source_pin(
    file: &File,
    path: &Path,
    name: &OsStr,
    entry: &TransferEntry,
    cancel: &Cancellation,
) -> Result<Arc<dyn TransferNamespacePin>, FsError> {
    let directory = parent::capture(file, path, cancel)?;
    let mut binding = binding(directory, path, name, cancel)?;
    if binding.object != Some(entry.identity) {
        return Err(invalid(
            &path.join(name),
            "source namespace binding changed",
        ));
    }
    if entry.item.kind == EntryKind::File {
        let file = native::open_relative(
            binding.parent.file(),
            name,
            FILE_READ_ATTRIBUTES | SYNCHRONIZE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN,
            0,
        )
        .map_err(|e| error(&path.join(name), e))?;
        if identity(&file, &path.join(name))? != entry.identity {
            return Err(invalid(
                &path.join(name),
                "source changed before revision capture",
            ));
        }
        binding.revision = Some(destination::recovery::Revision::of(
            &file,
            &path.join(name),
        )?);
    }
    Ok(make_pin(binding, entry.item.kind, true, current_job()))
}
fn make_pin(
    mut binding: Binding,
    kind: EntryKind,
    original: bool,
    job: Option<Arc<Job>>,
) -> Arc<dyn TransferNamespacePin> {
    binding.original = original;
    let pin = Arc::new(Pin {
        extent: if kind == EntryKind::Directory {
            TransferNamespaceExtent::Subtree
        } else {
            TransferNamespaceExtent::Entry
        },
        bindings: Mutex::new(vec![binding.clone()]),
        original,
        job,
    });
    if let Some(job) = &pin.job {
        job.register(&pin, &binding);
    }
    pin
}
impl Pin {
    fn prune(&self, mut keep: impl FnMut(&Binding) -> bool) {
        let retired = {
            let mut bindings = self.bindings.lock().unwrap_or_else(|e| e.into_inner());
            let mut retained = Vec::with_capacity(bindings.len());
            let mut retired = Vec::new();
            for binding in std::mem::take(&mut *bindings) {
                if keep(&binding) {
                    retained.push(binding);
                } else {
                    retired.push(binding);
                }
            }
            *bindings = retained;
            if let Some(job) = &self.job {
                job.sync_pin(self as *const Self as usize, &bindings, None, self.original);
            }
            retired
        };
        // Native metadata anchors retire under the caller's execution witness, outside
        // the index/binding locks. Removed random names do not accumulate in the cache.
        drop(retired);
    }
}
impl Drop for Pin {
    fn drop(&mut self) {
        let key = self as *const Self as usize;
        if let Some(job) = &self.job {
            job.sync_pin(key, &[], None, self.original);
        }
    }
}
impl TransferNamespacePin for Pin {
    fn refresh(&self, cancel: &Cancellation) -> Result<TransferNamespaceProbe, FsError> {
        if let Some(job) = &self.job
            && let Some(error) = job
                .failure
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        {
            return Err(error);
        }
        let bindings = self
            .bindings
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let mut scopes = Vec::with_capacity(bindings.len());
        let mut unknown = false;
        for mut binding in bindings {
            // A private pin witnesses an object this job created (a stage, then its
            // published output). Once every native owner of that object has closed, the
            // job never touches it again: retire the binding instead of re-proving it on
            // every later refresh. Otherwise published outputs accumulate one alias each,
            // a full refresh costs O(published files) (quadratic over a job), and an
            // external change to a finished output later fails the job's refresh.
            // Namespace claims on those outputs remain with the original pins covering
            // them (destination entry or subtree roots).
            if let Some(job) = &self.job
                && !self.original
                && !binding.delete_pending
                && binding
                    .object
                    .is_some_and(|object| job.owners_closed(object))
            {
                self.prune(|old| old.slot != binding.slot || old.object != binding.object);
                continue;
            }
            if let Some(job) = &self.job
                && !binding.original
                && job.removed_parent(binding.slot.parent)
            {
                if job.owners_closed(binding.slot.parent)
                    && (binding.object.is_none()
                        || (binding.delete_pending
                            && binding
                                .object
                                .is_some_and(|object| job.owners_closed(object))))
                {
                    self.prune(|old| old.slot != binding.slot);
                    continue;
                }
                unknown = true;
                continue;
            }
            binding.parent.validate(cancel)?;
            binding.parent.check_path(&binding.parent_path)?;
            let path = binding.parent_path.join(&binding.name);
            match native::open_relative(
                binding.parent.file(),
                &binding.name,
                FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_OPEN,
                0,
            ) {
                Ok(file) => {
                    if Some(identity(&file, &path)?) != binding.object {
                        return Err(invalid(&path, "original namespace leaf binding changed"));
                    }
                    if binding.delete_pending {
                        unknown = true;
                    }
                    // Copy and Move source pins only: a moved ChangeTime alone is no
                    // change (VIEW-05).
                    if let Some(revision) = binding.revision
                        && !revision.matches(
                            destination::recovery::Revision::of(&file, &path)?,
                            midnight_core::TransferRevisionPolicy::AcceptChangeTimeOnly,
                        )
                    {
                        return Err(invalid(&path, "original source revision changed"));
                    }
                }
                Err(e)
                    if e.kind() == io::ErrorKind::NotFound
                        && (binding.object.is_none() || binding.delete_pending) =>
                {
                    if binding.delete_pending {
                        if !self.job.as_ref().is_some_and(|job| {
                            binding
                                .object
                                .is_some_and(|object| job.owners_closed(object))
                        }) {
                            unknown = true;
                            continue;
                        }
                        binding.object = None;
                        binding.delete_pending = false;
                        if binding.original {
                            let mut retained =
                                self.bindings.lock().unwrap_or_else(|e| e.into_inner());
                            if let Some(old) = retained
                                .iter_mut()
                                .find(|old| old.slot == binding.slot && old.delete_pending)
                            {
                                *old = binding.clone();
                            }
                            if let Some(job) = &self.job {
                                job.sync_pin(
                                    self as *const Self as usize,
                                    &retained,
                                    None,
                                    self.original,
                                );
                            }
                        } else {
                            self.prune(|old| old.slot != binding.slot || !old.delete_pending);
                            continue;
                        }
                    }
                }
                Err(e) if binding.delete_pending && e.kind() == io::ErrorKind::PermissionDenied => {
                    // Windows can refuse metadata opens until the last DELETE owner closes.
                    // This known owned transition is unqualified, not a false absence proof.
                    unknown = true;
                    continue;
                }
                Err(e) => return Err(error(&path, e)),
            }
            match binding.parent.mode.key(&binding.name) {
                Some(lookup_key) => scopes.push(TransferNamespaceScope {
                    parent: binding.parent.snapshot.clone(),
                    lookup_key,
                    object: binding.object,
                    extent: self.extent,
                }),
                None => unknown = true,
            }
        }
        if unknown {
            Ok(TransferNamespaceProbe::Unknown)
        } else {
            Ok(TransferNamespaceProbe::Known(scopes.into()))
        }
    }
}

pub(super) struct Mutation<'a> {
    change: Option<&'a mut dyn TransferNamespaceChange>,
    job: Option<Arc<Job>>,
    complete: bool,
    opted: bool,
    unknown_reserved: bool,
    /// Pins reserved in this turn by announcements and private creations; each is
    /// reserved at most once.
    announced: HashSet<usize>,
    failure: Option<FsError>,
}
pub(super) enum Creation {
    Planned,
    Private(EntryKind),
}
/// Finish failures are already retained by the installed host adapter. Preserve native
/// commit outcomes; append the retained diagnostic when the native operation also failed.
pub(super) fn change<T>(
    cancel: &Cancellation,
    operation: impl FnOnce(&mut Mutation<'_>) -> Result<T, FsError>,
) -> Result<T, FsError> {
    let outcome = with_transfer_namespace_change(cancel, |change| {
        operation(&mut Mutation {
            change,
            job: current_job(),
            complete: true,
            opted: false,
            unknown_reserved: false,
            announced: HashSet::new(),
            failure: None,
        })
    });
    match (outcome.operation, outcome.refresh) {
        (Ok(value), Ok(()) | Err(_)) => Ok(value),
        (Err(error), Ok(())) => Err(error),
        (Err(mut error), Err(refresh)) => {
            error
                .message
                .push_str(&format!("; namespace refresh failed: {refresh}"));
            Err(error)
        }
    }
}
fn as_io(error: FsError) -> io::Error {
    io::Error::new(error.kind, error)
}
impl Mutation<'_> {
    fn ready(&self) -> Result<(), FsError> {
        self.failure.clone().map_or(Ok(()), Err)
    }
    fn reserve(
        &mut self,
        pin: Option<Arc<dyn TransferNamespacePin>>,
        cancel: &Cancellation,
    ) -> Result<bool, FsError> {
        let result = match self.change.as_mut() {
            Some(change) => change.reserve_private_child(pin, cancel),
            None => Err(invalid(
                Path::new(""),
                "native job has no namespace mutation turn",
            )),
        };
        if let Err(error) = &result {
            self.failure = Some(error.clone());
        }
        result
    }
    fn incomplete(&mut self, path: &Path, cancel: &Cancellation) -> Result<(), FsError> {
        self.ready()?;
        self.complete = false;
        if self.opted && !self.unknown_reserved {
            if !self.reserve(None, cancel)? {
                let failure = error(path, io::ErrorKind::WouldBlock.into());
                self.failure = Some(failure.clone());
                return Err(failure);
            }
            self.unknown_reserved = true;
        }
        Ok(())
    }
    fn announce(
        &mut self,
        bindings: &[Binding],
        private: bool,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.ready()?;
        if self.job.is_none() {
            return Ok(());
        }
        if self.change.is_none() {
            let failure = invalid(path, "native job has no namespace mutation turn");
            self.failure = Some(failure.clone());
            return Err(failure);
        }
        if !self.complete {
            return self.incomplete(path, cancel);
        }
        let pins = self
            .job
            .as_ref()
            .and_then(|job| job.announcement_pins(bindings, private));
        let Some(pins) = pins else {
            return self.incomplete(path, cancel);
        };
        // All endpoints and displaced/descendant original and private pins were collected
        // before the first exact registered Arc can activate incremental refresh.
        self.opted = true;
        for pin in pins {
            let key = Arc::as_ptr(&pin) as usize;
            if !self.announced.contains(&key) {
                if !self.reserve(Some(pin), cancel)? {
                    let failure = error(path, io::ErrorKind::WouldBlock.into());
                    self.failure = Some(failure.clone());
                    return Err(failure);
                }
                self.announced.insert(key);
            }
        }
        Ok(())
    }
    pub(super) fn prepare_file(
        &mut self,
        source: &File,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.ready()?;
        if self.job.is_none() {
            return Ok(());
        }
        // Announcement only: nothing changes a name in this batch (`parent::BatchChecks`).
        let _batch = TransferNamespaceRefreshBatch::begin();
        let binding = self.held_binding(source, cancel)?;
        let path = binding.parent_path.join(&binding.name);
        self.announce(&[binding], false, &path, cancel)
    }
    pub(super) fn prepare_rename(
        &mut self,
        source: &File,
        parent_file: &File,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.ready()?;
        if self.job.is_none() {
            return Ok(());
        }
        // Announcement only: nothing changes a name in this batch (`parent::BatchChecks`).
        let _batch = TransferNamespaceRefreshBatch::begin();
        let old = self.held_binding(source, cancel)?;
        let path = parent::physical_path(parent_file).map_err(|e| error(Path::new(""), e))?;
        let directory = parent::capture(parent_file, &path, cancel)?;
        directory.validate(cancel)?;
        let target = binding(directory, &path, name, cancel)?;
        self.announce(&[old, target], false, &path.join(name), cancel)
    }
    #[cfg(test)]
    pub(super) fn create_relative(
        &mut self,
        parent_file: &File,
        name: &OsStr,
        kind: EntryKind,
        cancel: &Cancellation,
        create: impl FnOnce() -> io::Result<File>,
    ) -> io::Result<File> {
        if self.job.is_none() {
            return create();
        }
        let path = parent::physical_path(parent_file)?;
        self.create(
            parent_file,
            &path,
            name,
            Creation::Private(kind),
            cancel,
            create,
        )
    }
    pub(super) fn create(
        &mut self,
        parent_file: &File,
        parent_path: &Path,
        name: &OsStr,
        creation: Creation,
        cancel: &Cancellation,
        create: impl FnOnce() -> io::Result<File>,
    ) -> io::Result<File> {
        self.ready().map_err(as_io)?;
        let Some(job) = self.job.clone() else {
            return create();
        };
        // The binding checks and the announcement up to the native creation are one
        // refresh batch: nothing changes a name before it (`parent::BatchChecks`).
        let batch = TransferNamespaceRefreshBatch::begin();
        let directory = parent::capture(parent_file, parent_path, cancel).map_err(as_io)?;
        directory.validate(cancel).map_err(as_io)?;
        let prospective = binding(directory, parent_path, name, cancel).map_err(as_io)?;
        let private = matches!(creation, Creation::Private(_));
        if private && prospective.object.is_some() {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        self.announce(
            std::slice::from_ref(&prospective),
            private,
            &parent_path.join(name),
            cancel,
        )
        .map_err(as_io)?;
        if let Creation::Private(kind) = creation {
            let pin = make_pin(prospective.clone(), kind, false, Some(job.clone()));
            let key = Arc::as_ptr(&pin) as *const () as usize;
            let accepted = self.reserve(Some(pin), cancel).map_err(as_io)?;
            if !accepted {
                return Err(io::ErrorKind::AlreadyExists.into());
            }
            // The reserved private object covers every later change beneath it in this
            // turn (the capture or recovery rename into it, publication or restoration
            // out of it, its removal), so it is never reserved again here. A second
            // reservation would follow this turn's first mutation, and its failure (a
            // peer latched or quarantined since this one, a cancellation, a refresh
            // error) would latch and veto the turn's own compensation. The host keeps
            // every pin it accepts until the turn finishes, so this address stays unique.
            self.announced.insert(key);
        }
        drop(batch);
        let file = create()?;
        match identity(&file, &parent_path.join(name)) {
            Ok(object) => job.created(Binding {
                object: Some(object),
                ..prospective
            }),
            Err(error) => job.fail(error),
        }
        Ok(file)
    }
    fn held_binding(&self, file: &File, cancel: &Cancellation) -> Result<Binding, FsError> {
        let actual = parent::physical_path(file).map_err(|e| error(Path::new(""), e))?;
        let parent_path = actual
            .parent()
            .ok_or_else(|| invalid(&actual, "native root mutation"))?;
        let name = actual
            .file_name()
            .ok_or_else(|| invalid(&actual, "native root mutation"))?;
        let parent_file = native::open_parent(parent_path).map_err(|e| error(parent_path, e))?;
        let directory = parent::capture(&parent_file, parent_path, cancel)?;
        directory.validate(cancel)?;
        let visible = binding(directory, parent_path, name, cancel)?;
        if visible.object != Some(identity(file, &actual)?) {
            return Err(invalid(
                &actual,
                "held native binding changed before mutation",
            ));
        }
        Ok(visible)
    }
    pub(super) fn rename(
        &mut self,
        source: &File,
        parent_file: &File,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> io::Result<()> {
        self.ready().map_err(as_io)?;
        let Some(job) = self.job.clone() else {
            return destination::recovery::rename_into_untracked(source, parent_file, name);
        };
        // One refresh batch up to the native rename, as for a creation.
        let batch = TransferNamespaceRefreshBatch::begin();
        let old = self.held_binding(source, cancel).map_err(as_io)?;
        let path = parent::physical_path(parent_file)?;
        let directory = parent::capture(parent_file, &path, cancel).map_err(as_io)?;
        directory.validate(cancel).map_err(as_io)?;
        let target = binding(directory.clone(), &path, name, cancel).map_err(as_io)?;
        self.announce(&[old.clone(), target], false, &path.join(name), cancel)
            .map_err(as_io)?;
        let mut new = Binding {
            slot: slot(&directory, name),
            parent: directory,
            parent_path: path,
            name: name.to_owned(),
            object: old.object,
            revision: None,
            original: false,
            delete_pending: false,
        };
        drop(batch);
        destination::recovery::rename_into_untracked(source, parent_file, name)?;
        match destination::recovery::Revision::of(source, &new.parent_path.join(name)) {
            Ok(revision) => new.revision = Some(revision),
            Err(error) => job.fail(error),
        }
        job.renamed(&old, new);
        Ok(())
    }
    pub(super) fn remove(
        &mut self,
        source: &File,
        cancel: &Cancellation,
        remove: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<()> {
        self.ready().map_err(as_io)?;
        let Some(job) = self.job.clone() else {
            return remove();
        };
        // One refresh batch up to the native removal, as for a creation.
        let batch = TransferNamespaceRefreshBatch::begin();
        let old = self.held_binding(source, cancel).map_err(as_io)?;
        self.announce(
            std::slice::from_ref(&old),
            false,
            &old.parent_path.join(&old.name),
            cancel,
        )
        .map_err(as_io)?;
        let directory = source.metadata()?.is_dir();
        drop(batch);
        remove()?;
        job.removed(&old, directory);
        Ok(())
    }
}

#[cfg(test)]
#[path = "transfer_namespace_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "transfer_namespace_batch_tests.rs"]
mod batch_tests;
