//! Metadata pins retain pooled parents, never one open descriptor per selected leaf.
use super::*;
use midnight_core::{
    TransferNamespaceExtent, TransferNamespaceParent, TransferNamespacePin, TransferNamespaceProbe,
    TransferNamespaceScope,
};
use midnight_platform_common::transfer_namespace::{
    TransferNamespaceChange, TransferNamespaceChanges, current_transfer_namespace_changes,
};
use std::{
    collections::{BTreeMap, HashMap},
    ffi::OsString,
    sync::{Mutex, OnceLock, Weak},
};

pub(super) struct Change<'a> {
    inner: Option<&'a mut dyn TransferNamespaceChange>,
    complete: bool,
    opted: bool,
}
impl<'a> Change<'a> {
    pub(super) fn as_mut(&mut self) -> Option<&mut (dyn TransferNamespaceChange + 'a)> {
        self.inner.as_deref_mut()
    }
    fn incomplete(&mut self, path: &Path, cancel: &Cancellation) -> Result<(), FsError> {
        self.complete = false;
        if self.opted
            && let Some(transaction) = self.as_mut()
            && !transaction.reserve_private_child(None, cancel)?
        {
            return Err(invalid(path, "incomplete namespace announcement refused"));
        }
        Ok(())
    }
}

pub(super) fn change<T>(
    cancel: &Cancellation,
    operation: impl FnOnce(Change<'_>) -> Result<T, FsError>,
) -> Result<T, FsError> {
    let outcome = midnight_platform_common::transfer_namespace::with_transfer_namespace_change(
        cancel,
        |inner| {
            operation(Change {
                inner,
                complete: true,
                opted: false,
            })
        },
    );
    match (outcome.operation, outcome.refresh) {
        (Err(mut operation), Err(refresh)) => {
            operation
                .message
                .push_str(&format!("; namespace refresh: {refresh}"));
            Err(operation)
        }
        // The installed host adapter records refresh failure as Opaque plus a
        // final-job diagnostic. Never turn a committed native outcome into an
        // apparent pre-commit error; the operation remains the truthful result.
        (operation, _) => operation,
    }
}
const MAX_SPELLING_CACHE: usize = 4096;
const MAX_ANCESTORS: usize = 4096;
const PATH_DIRECTORY: OFlags = OFlags::PATH
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

#[derive(Clone, PartialEq, Eq, Hash)]
struct ParentKey {
    chain: Vec<(TransferIdentity, u64)>,
    domain: Vec<u8>,
}

struct Parent {
    file: Arc<File>,
    key: ParentKey,
    facts: Arc<TransferNamespaceParent>,
    paths: Mutex<HashMap<PathBuf, Weak<PathBuf>>>,
}

fn parent_registry() -> &'static Mutex<HashMap<ParentKey, Weak<Parent>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<ParentKey, Weak<Parent>>>> = OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}

impl Drop for Parent {
    fn drop(&mut self) {
        let mut parents = parent_registry().lock().unwrap_or_else(|e| e.into_inner());
        if parents
            .get(&self.key)
            .is_some_and(|parent| std::ptr::eq(parent.as_ptr(), self))
        {
            parents.remove(&self.key);
        }
        // Native file fields drop only after this metadata-only lock is released.
    }
}

impl Parent {
    fn spelling(&self, path: &Path) -> Arc<PathBuf> {
        let mut paths = self.paths.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(shared) = paths.get(path).and_then(Weak::upgrade) {
            return shared;
        }
        paths.retain(|_, path| path.strong_count() != 0);
        let shared = Arc::new(path.to_owned());
        if paths.len() < MAX_SPELLING_CACHE {
            paths.insert(path.to_owned(), Arc::downgrade(&shared));
        }
        shared
    }
}

fn qualified_lookup(filesystem: u64, flags: Option<u32>) -> bool {
    match filesystem {
        // FS_CASEFOLD_FL is retained by rustix even though IFlags has no named bit.
        // tmpfs can also inherit per-directory casefold. Missing ioctl support
        // is unqualified, even on kernels predating that optional mode.
        0xef53 | 0x0102_1994 => flags.is_some_and(|flags| flags & (0x4000_0000 | 0x0000_0800) == 0),
        _ => false,
    }
}

fn parent_key(
    file: &File,
    path: &Path,
    cancel: &Cancellation,
) -> Result<Option<ParentKey>, FsError> {
    checked(cancel, path)?;
    let filesystem = rustix::fs::fstatfs(file)
        .map_err(|e| error(path, e.into()))?
        .f_type as u64;
    let flags = if matches!(filesystem, 0xef53 | 0x0102_1994) {
        rustix::fs::ioctl_getflags(file)
            .ok()
            .map(|flags| flags.bits())
    } else {
        None
    };
    if !qualified_lookup(filesystem, flags) {
        return Ok(None);
    }
    let mut chain = Vec::new();
    let mut cursor = File::from(
        openat(file, ".", PATH_DIRECTORY, Mode::empty()).map_err(|e| error(path, e.into()))?,
    );
    for _ in 0..MAX_ANCESTORS {
        checked(cancel, path)?;
        let id = identity(&cursor, path)?;
        let Some(mount) = native_location::mount(&cursor, path)? else {
            return Ok(None);
        };
        chain.push((id, mount));
        let parent = File::from(
            openat(&cursor, "..", PATH_DIRECTORY, Mode::empty())
                .map_err(|e| error(path, e.into()))?,
        );
        let Some(parent_mount) = native_location::mount(&parent, path)? else {
            return Ok(None);
        };
        if identity(&parent, path)? == id && parent_mount == mount {
            let mut domain = b"linux-byte-lookup-v1".to_vec();
            domain.extend_from_slice(&filesystem.to_le_bytes());
            return Ok(Some(ParentKey { chain, domain }));
        }
        cursor = parent;
    }
    Ok(None)
}

fn pooled_parent(
    file: &Arc<File>,
    path: &Path,
    cancel: &Cancellation,
) -> Result<Option<Arc<Parent>>, FsError> {
    let Some(key) = parent_key(file, path, cancel)? else {
        return Ok(None);
    };
    let cache = parent_registry();
    let hit = cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key)
        .and_then(Weak::upgrade);
    if let Some(hit) = hit {
        return Ok(Some(hit));
    }
    if !qualified_mount(key.chain[0].1, path)? {
        return Ok(None);
    }
    let facts = Arc::new(TransferNamespaceParent {
        identity: key.chain[0].0,
        lookup_domain: key.domain.clone().into(),
        ancestors: key
            .chain
            .iter()
            .map(|(id, _)| *id)
            .collect::<Vec<_>>()
            .into(),
    });
    let parent = Arc::new(Parent {
        file: file.clone(),
        key: key.clone(),
        facts,
        paths: Mutex::default(),
    });
    let existing = {
        let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(existing) = cache.get(&key).and_then(Weak::upgrade) {
            Some(existing)
        } else {
            // Live identity is not an optional cache: siblings must keep sharing
            // even when many other parents are retained. Final Drop unregisters.
            cache.insert(key, Arc::downgrade(&parent));
            None
        }
    };
    // Redundant native parent ownership is destroyed only after the cache unlocks.
    Ok(Some(existing.unwrap_or(parent)))
}

fn mount_is_filesystem_root(bytes: &[u8], mount: u64) -> bool {
    bytes.split(|byte| *byte == b'\n').any(|line| {
        let mut fields = line
            .split(|byte| *byte == b' ')
            .filter(|field| !field.is_empty());
        let id = fields
            .next()
            .and_then(|field| std::str::from_utf8(field).ok())
            .and_then(|field| field.parse::<u64>().ok());
        fields.next();
        fields.next();
        id == Some(mount) && fields.next() == Some(b"/".as_slice())
    })
}

fn qualified_mount(mount: u64, path: &Path) -> Result<bool, FsError> {
    use std::io::Read;
    // A subdirectory bind mount hides physical ancestors from '..'. Refuse
    // positive geometry there. A live parent FD pins the qualified mount ID;
    // its immutable root qualification may be shared by sibling metadata pins.
    let file = match File::open("/proc/self/mountinfo") {
        Ok(file) => file,
        Err(_) => return Ok(false),
    };
    const MAX_MOUNT_METADATA: u64 = 4 * 1024 * 1024;
    let mut bytes = Vec::new();
    file.take(MAX_MOUNT_METADATA + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| error(path, e))?;
    Ok(bytes.len() as u64 <= MAX_MOUNT_METADATA && mount_is_filesystem_root(&bytes, mount))
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Witness {
    identity: TransferIdentity,
    kind: EntryKind,
    revision: (u64, i64, i64, i64, i64),
}

fn inspect(parent: &File, name: &OsStr, path: &Path) -> Result<Option<Witness>, FsError> {
    let file = match openat2(
        parent,
        name,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
        RESOLVE,
    ) {
        Ok(file) => File::from(file),
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(e) => return Err(error(path, e.into())),
    };
    let entry = entry(&file, path)?;
    let meta = file.metadata().map_err(|e| error(path, e))?;
    Ok(Some(Witness {
        identity: entry.identity,
        kind: entry.item.kind,
        revision: (
            meta.len(),
            meta.mtime(),
            meta.mtime_nsec(),
            meta.ctime(),
            meta.ctime_nsec(),
        ),
    }))
}

struct State {
    expected: Option<Witness>,
    failure: Option<FsError>,
    removed: bool,
    closed: bool,
}

#[derive(Default)]
struct JobData {
    sentinel: Weak<Pin>,
    roots: BTreeMap<usize, Weak<Pin>>,
    objects: HashMap<TransferIdentity, BTreeMap<usize, Weak<Pin>>>,
    parents: HashMap<TransferIdentity, BTreeMap<usize, Weak<Bindings>>>,
}
struct Job {
    key: usize,
    data: Mutex<JobData>,
}
type JobEntry = (Weak<dyn TransferNamespaceChanges>, Weak<Job>);
fn jobs() -> &'static Mutex<HashMap<usize, JobEntry>> {
    static JOBS: OnceLock<Mutex<HashMap<usize, JobEntry>>> = OnceLock::new();
    JOBS.get_or_init(Mutex::default)
}
impl Job {
    fn get(owner: &Arc<dyn TransferNamespaceChanges>) -> Arc<Self> {
        let key = Arc::as_ptr(owner) as *const () as usize;
        let weak = Arc::downgrade(owner);
        let hit = jobs()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&key)
            .filter(|(owner, _)| owner.ptr_eq(&weak))
            .and_then(|(_, job)| job.upgrade());
        if let Some(hit) = hit {
            return hit;
        }
        let fresh = Arc::new(Self {
            key,
            data: Mutex::default(),
        });
        let existing = {
            let mut jobs = jobs().lock().unwrap_or_else(|e| e.into_inner());
            let existing = jobs
                .get(&key)
                .filter(|(owner, _)| owner.ptr_eq(&weak))
                .and_then(|(_, job)| job.upgrade());
            if existing.is_none() {
                jobs.insert(key, (weak, Arc::downgrade(&fresh)));
            }
            existing
        };
        existing.unwrap_or(fresh)
    }
    fn add_pin(&self, pin: &Arc<Pin>) {
        let expected = pin.state.lock().unwrap_or_else(|e| e.into_inner()).expected;
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        if !pin.private {
            data.roots
                .insert(Arc::as_ptr(pin) as usize, Arc::downgrade(pin));
            if data.sentinel.strong_count() == 0 {
                data.sentinel = Arc::downgrade(pin);
            }
        }
        for id in pin
            .original
            .into_iter()
            .chain(expected.map(|w| w.identity))
            .collect::<std::collections::HashSet<_>>()
        {
            data.objects
                .entry(id)
                .or_default()
                .insert(Arc::as_ptr(pin) as usize, Arc::downgrade(pin));
        }
    }
    fn changed_pin(
        &self,
        pin: &Arc<Pin>,
        before: Option<TransferIdentity>,
        after: Option<TransferIdentity>,
    ) {
        if before == after {
            return;
        }
        let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(before) = before.filter(|id| Some(*id) != pin.original)
            && let Some(entries) = data.objects.get_mut(&before)
        {
            entries.remove(&(Arc::as_ptr(pin) as usize));
            if entries.is_empty() {
                data.objects.remove(&before);
            }
        }
        if let Some(after) = after.filter(|id| Some(*id) != pin.original) {
            data.objects
                .entry(after)
                .or_default()
                .insert(Arc::as_ptr(pin) as usize, Arc::downgrade(pin));
        }
    }
    fn sentinel(&self) -> Option<Arc<Pin>> {
        let weak = {
            let mut data = self.data.lock().unwrap_or_else(|e| e.into_inner());
            if data.sentinel.strong_count() == 0 {
                data.sentinel = data
                    .roots
                    .first_key_value()
                    .map(|(_, pin)| pin.clone())
                    .unwrap_or_default();
            }
            data.sentinel.clone()
        };
        weak.upgrade()
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        let mut jobs = jobs().lock().unwrap_or_else(|e| e.into_inner());
        if jobs
            .get(&self.key)
            .is_some_and(|(_, job)| std::ptr::eq(job.as_ptr(), self))
        {
            jobs.remove(&self.key);
        }
    }
}

struct PinOptions {
    kind: EntryKind,
    source: bool,
    private: bool,
}

pub(super) struct Pin {
    _bindings: Arc<Bindings>,
    parent: Option<Arc<Parent>>,
    parent_path: Arc<PathBuf>,
    parent_identity: TransferIdentity,
    name: OsString,
    original: Option<TransferIdentity>,
    extent: TransferNamespaceExtent,
    source: bool,
    private: bool,
    state: Mutex<State>,
}

impl Pin {
    fn capture(
        bindings: Arc<Bindings>,
        parent: &Arc<File>,
        parent_path: &Path,
        name: &OsStr,
        options: PinOptions,
        cancel: &Cancellation,
    ) -> Result<Arc<Self>, FsError> {
        let PinOptions {
            kind,
            source,
            private,
        } = options;
        validate_name(name, parent_path)?;
        checked(cancel, parent_path)?;
        let witness = inspect(parent, name, &parent_path.join(name))?;
        if source && witness.is_none() {
            return Err(invalid(parent_path, "planned source disappeared"));
        }
        if private && kind != EntryKind::File && witness.is_some() {
            return Err(error(
                &parent_path.join(name),
                io::ErrorKind::AlreadyExists.into(),
            ));
        }
        if private
            && kind == EntryKind::File
            && witness.is_some_and(|witness| witness.kind != EntryKind::File)
        {
            return Err(error(
                &parent_path.join(name),
                io::ErrorKind::AlreadyExists.into(),
            ));
        }
        let pooled = match openat2(
            CWD,
            parent_path,
            PATH_DIRECTORY,
            Mode::empty(),
            ResolveFlags::NO_SYMLINKS,
        ) {
            Ok(visible) => {
                let visible = File::from(visible);
                if identity(&visible, parent_path)? != identity(parent, parent_path)?
                    || native_location::mount(&visible, parent_path)?
                        != native_location::mount(parent, parent_path)?
                {
                    return Err(invalid(
                        parent_path,
                        "namespace parent binding changed during capture",
                    ));
                }
                pooled_parent(parent, parent_path, cancel)?
            }
            Err(rustix::io::Errno::LOOP) => None,
            Err(rustix::io::Errno::NOTDIR)
                if std::fs::symlink_metadata(parent_path)
                    .is_ok_and(|metadata| metadata.file_type().is_symlink()) =>
            {
                None
            }
            Err(e) => return Err(error(parent_path, e.into())),
        };
        let parent_path = pooled.as_ref().map_or_else(
            || Arc::new(parent_path.to_owned()),
            |parent| parent.spelling(parent_path),
        );
        let parent_identity = identity(parent, &parent_path)?;
        Ok(Arc::new(Self {
            _bindings: bindings,
            parent: pooled,
            parent_path,
            parent_identity,
            name: name.to_owned(),
            original: witness.map(|w| w.identity),
            extent: if kind == EntryKind::Directory {
                TransferNamespaceExtent::Subtree
            } else {
                TransferNamespaceExtent::Entry
            },
            source,
            private,
            state: Mutex::new(State {
                expected: witness,
                failure: None,
                removed: false,
                closed: false,
            }),
        }))
    }
}

impl TransferNamespacePin for Pin {
    fn refresh(&self, cancel: &Cancellation) -> Result<TransferNamespaceProbe, FsError> {
        checked(cancel, &self.parent_path)?;
        let retirement = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            (state.removed, state.closed)
        };
        if self.private && retirement.0 {
            return Ok(if retirement.1 {
                TransferNamespaceProbe::Known(Arc::default())
            } else {
                TransferNamespaceProbe::Unknown
            });
        }
        let Some(parent) = &self.parent else {
            return Ok(TransferNamespaceProbe::Unknown);
        };
        let visible = File::from(
            openat2(
                CWD,
                self.parent_path.as_path(),
                PATH_DIRECTORY,
                Mode::empty(),
                ResolveFlags::NO_SYMLINKS,
            )
            .map_err(|e| error(&self.parent_path, e.into()))?,
        );
        if identity(&visible, &self.parent_path)? != self.parent_identity
            || native_location::mount(&visible, &self.parent_path)? != Some(parent.key.chain[0].1)
            || parent_key(&parent.file, &self.parent_path, cancel)?.as_ref() != Some(&parent.key)
        {
            return Err(invalid(
                &self.parent_path,
                "original namespace parent or ancestry changed",
            ));
        }
        let observed = inspect(&parent.file, &self.name, &self.parent_path.join(&self.name))?;
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(failure) = &state.failure {
            return Err(failure.clone());
        }
        let matches = match (state.expected, observed) {
            (None, None) => true,
            (Some(expected), Some(actual)) => {
                expected.identity == actual.identity
                    && expected.kind == actual.kind
                    && (!self.source
                        || actual.kind == EntryKind::Directory
                        || expected.revision == actual.revision)
            }
            _ => false,
        };
        if !matches {
            return Err(invalid(
                &self.parent_path.join(&self.name),
                "original namespace object or revision changed",
            ));
        }
        let current = state.expected.map(|w| w.identity);
        let mut facts = vec![TransferNamespaceScope {
            parent: parent.facts.clone(),
            lookup_key: self.name.as_bytes().into(),
            object: current.or(self.original.filter(|_| self.source)),
            extent: self.extent,
        }];
        if let Some(original) = self
            .original
            .filter(|original| Some(*original) != facts[0].object)
        {
            facts.push(TransferNamespaceScope {
                parent: parent.facts.clone(),
                lookup_key: self.name.as_bytes().into(),
                object: Some(original),
                extent: self.extent,
            });
        }
        Ok(TransferNamespaceProbe::Known(facts.into()))
    }
}

impl Drop for Pin {
    fn drop(&mut self) {
        {
            let mut bindings = self
                ._bindings
                .pins
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if let Some(entries) = bindings.get_mut(&self.name) {
                entries.retain(|pin| pin.strong_count() != 0);
                if entries.is_empty() {
                    bindings.remove(&self.name);
                }
            }
        }
        if let Some(job) = &self._bindings.job {
            let current = self
                .state
                .get_mut()
                .unwrap_or_else(|e| e.into_inner())
                .expected
                .map(|w| w.identity);
            let mut data = job.data.lock().unwrap_or_else(|e| e.into_inner());
            data.roots.remove(&(self as *const Self as usize));
            for id in self.original.into_iter().chain(current) {
                if let Some(entries) = data.objects.get_mut(&id) {
                    entries.remove(&(self as *const Self as usize));
                    if entries.is_empty() {
                        data.objects.remove(&id);
                    }
                }
            }
        }
    }
}

type BindingKey = (ParentKey, usize);
type BindingEntry = (Weak<dyn TransferNamespaceChanges>, Weak<Bindings>);

fn bindings_registry() -> &'static Mutex<HashMap<BindingKey, BindingEntry>> {
    static REGISTRY: OnceLock<Mutex<HashMap<BindingKey, BindingEntry>>> = OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}

#[derive(Default)]
pub(super) struct Bindings {
    pins: Mutex<HashMap<OsString, Vec<Weak<Pin>>>>,
    registry_key: Option<BindingKey>,
    job: Option<Arc<Job>>,
}

impl Drop for Bindings {
    fn drop(&mut self) {
        let Some(key) = &self.registry_key else {
            return;
        };
        if let Some(job) = &self.job {
            let mut data = job.data.lock().unwrap_or_else(|e| e.into_inner());
            for (id, _) in &key.0.chain {
                if let Some(parents) = data.parents.get_mut(id) {
                    parents.remove(&(self as *const Self as usize));
                    if parents.is_empty() {
                        data.parents.remove(id);
                    }
                }
            }
        }
        let mut bindings = bindings_registry()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if bindings
            .get(key)
            .is_some_and(|(_, binding)| std::ptr::eq(binding.as_ptr(), self))
        {
            bindings.remove(key);
        }
    }
}

impl Bindings {
    pub(super) fn for_parent(parent: &Arc<File>, path: &Path) -> Result<Arc<Self>, FsError> {
        let Some(owner) = current_transfer_namespace_changes() else {
            return Ok(Arc::new(Self::default()));
        };
        let Some(key) = parent_key(parent, path, &Cancellation::default())? else {
            return Ok(Arc::new(Self::default()));
        };
        let weak = Arc::downgrade(&owner);
        let key = (key, Arc::as_ptr(&owner) as *const () as usize);
        let job = Job::get(&owner);
        let fresh = Arc::new(Self {
            pins: Mutex::default(),
            registry_key: Some(key.clone()),
            job: Some(job.clone()),
        });
        let hit = {
            let mut cache = bindings_registry()
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let hit = cache
                .get(&key)
                .filter(|(existing, _)| existing.ptr_eq(&weak))
                .and_then(|(_, bindings)| bindings.upgrade());
            if hit.is_none() {
                cache.insert(key.clone(), (weak, Arc::downgrade(&fresh)));
            }
            hit
        };
        if let Some(hit) = hit {
            return Ok(hit);
        }
        {
            let mut data = job.data.lock().unwrap_or_else(|e| e.into_inner());
            for (id, _) in &key.0.chain {
                data.parents
                    .entry(*id)
                    .or_default()
                    .insert(Arc::as_ptr(&fresh) as usize, Arc::downgrade(&fresh));
            }
        }
        Ok(fresh)
    }

    pub(super) fn pin(
        self: &Arc<Self>,
        parent: &Arc<File>,
        path: &Path,
        name: &OsStr,
        kind: EntryKind,
        source: bool,
        cancel: &Cancellation,
    ) -> Result<Arc<Pin>, FsError> {
        self.pin_with_kind(
            parent,
            path,
            name,
            PinOptions {
                kind,
                source,
                private: false,
            },
            cancel,
        )
    }

    pub(super) fn private_pin(
        self: &Arc<Self>,
        parent: &Arc<File>,
        path: &Path,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<Pin>, FsError> {
        self.pin_with_kind(
            parent,
            path,
            name,
            PinOptions {
                kind: EntryKind::Directory,
                source: false,
                private: true,
            },
            cancel,
        )
    }

    /// Observe a publication binding independently of covering original roots.
    /// Existing regular targets retain their original object across replacement;
    /// this Entry pin never becomes an empty publication/close certificate.
    pub(super) fn private_file_pin(
        self: &Arc<Self>,
        parent: &Arc<File>,
        path: &Path,
        name: &OsStr,
        cancel: &Cancellation,
    ) -> Result<Arc<Pin>, FsError> {
        self.pin_with_kind(
            parent,
            path,
            name,
            PinOptions {
                kind: EntryKind::File,
                source: false,
                private: true,
            },
            cancel,
        )
    }

    fn pin_with_kind(
        self: &Arc<Self>,
        parent: &Arc<File>,
        path: &Path,
        name: &OsStr,
        options: PinOptions,
        cancel: &Cancellation,
    ) -> Result<Arc<Pin>, FsError> {
        let pin = Pin::capture(self.clone(), parent, path, name, options, cancel)?;
        {
            let mut bindings = self.pins.lock().unwrap_or_else(|e| e.into_inner());
            let entries = bindings.entry(name.to_owned()).or_default();
            entries.retain(|entry| entry.strong_count() != 0);
            entries.push(Arc::downgrade(&pin));
        }
        if let Some(job) = &self.job {
            job.add_pin(&pin);
        }
        Ok(pin)
    }

    /// Announce retained capabilities, not reconstructed geometry. The caller
    /// invokes this for every changed binding before the first native name change.
    pub(super) fn announce(
        &self,
        parent: &File,
        path: &Path,
        name: &OsStr,
        change: &mut Change<'_>,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        if change.inner.is_none() {
            return Ok(());
        }
        if !change.complete {
            return change.incomplete(path, cancel);
        }
        let Some(owner) = current_transfer_namespace_changes() else {
            return Ok(());
        };
        let job = Job::get(&owner);
        let Some(sentinel) = job.sentinel() else {
            return change.incomplete(path, cancel);
        };
        if self
            .job
            .as_ref()
            .is_none_or(|binding_job| !Arc::ptr_eq(binding_job, &job))
        {
            // Another endpoint may already have announced its original roots.
            // A foreign/unqualified observer set cannot silently complete that
            // fast turn without reporting this binding.
            return change.incomplete(path, cancel);
        }
        let key = parent_key(parent, path, cancel)?;
        let Some(key) = key.filter(|key| {
            self.registry_key
                .as_ref()
                .is_some_and(|registered| &registered.0 == key)
        }) else {
            // A prior nested helper may already have opted in. Unknown cannot be
            // silently omitted; the host either grants exclusive fallback or refuses.
            return change.incomplete(path, cancel);
        };
        let observed = inspect(parent, name, &path.join(name))?;
        let mut pins = self
            .pins
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
            .cloned()
            .unwrap_or_default();
        let parents = {
            let data = job.data.lock().unwrap_or_else(|e| e.into_inner());
            // Same-object aliases retain authority even under different names.
            if let Some(witness) = observed {
                pins.extend(
                    data.objects
                        .get(&witness.identity)
                        .into_iter()
                        .flat_map(|pins| pins.values())
                        .cloned(),
                );
            }
            // A selected subtree owns child bindings. Its original Arc is the
            // refresh witness when a descendant is created, moved or removed.
            for (ancestor, _) in &key.chain {
                pins.extend(
                    data.objects
                        .get(ancestor)
                        .into_iter()
                        .flat_map(|pins| pins.values())
                        .cloned(),
                );
            }
            observed
                .filter(|w| w.kind == EntryKind::Directory)
                .and_then(|w| {
                    data.parents
                        .get(&w.identity)
                        .map(|parents| parents.values().cloned().collect::<Vec<_>>())
                })
                .unwrap_or_default()
        };
        // Moving a selected directory also changes retained descendant parent
        // bindings. Snapshot weak groups first; no native owner drops under locks.
        for parent in parents.into_iter().filter_map(|parent| parent.upgrade()) {
            let children: Vec<_> = parent
                .pins
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .values()
                .flatten()
                .cloned()
                .collect();
            pins.extend(children);
        }
        let mut affected = BTreeMap::new();
        for pin in pins.into_iter().filter_map(|pin| pin.upgrade()) {
            // Private bindings may materialize or change independently of their
            // covering original root. Reuse the exact retained Arc so the host
            // touches its existing alias without adding a duplicate observer.
            affected.insert(Arc::as_ptr(&pin) as usize, pin);
        }
        affected.insert(Arc::as_ptr(&sentinel) as usize, sentinel);
        // Completeness is established before the first exact Arc opts the turn in.
        change.opted = true;
        let Some(transaction) = change.as_mut() else {
            return Ok(());
        };
        for pin in affected.into_values() {
            if !transaction.reserve_private_child(Some(pin), cancel)? {
                return Err(error(&path.join(name), io::ErrorKind::WouldBlock.into()));
            }
        }
        Ok(())
    }

    pub(super) fn changed(&self, parent: &File, path: &Path, name: &OsStr) {
        let pins: Vec<_> = {
            let mut bindings = self.pins.lock().unwrap_or_else(|e| e.into_inner());
            let pins: Vec<_> = bindings
                .get(name)
                .into_iter()
                .flatten()
                .filter_map(Weak::upgrade)
                .collect();
            if pins.is_empty() {
                bindings.remove(name);
            }
            pins
        };
        if pins.is_empty() {
            return;
        }
        let observed = inspect(parent, name, &path.join(name));
        for pin in pins {
            let mut state = pin.state.lock().unwrap_or_else(|e| e.into_inner());
            let before = state.expected.map(|w| w.identity);
            match &observed {
                Ok(value) => {
                    state.expected = *value;
                }
                Err(failure) => state.failure = Some(failure.clone()),
            }
            let after = state.expected.map(|w| w.identity);
            drop(state);
            if let Some(job) = &self.job {
                job.changed_pin(&pin, before, after);
            }
        }
    }

    pub(super) fn retired(&self, name: &OsStr, closed: bool) {
        let pins: Vec<_> = self
            .pins
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(name)
            .into_iter()
            .flatten()
            .filter_map(Weak::upgrade)
            .collect();
        for pin in pins {
            if pin.private {
                let mut state = pin.state.lock().unwrap_or_else(|e| e.into_inner());
                state.removed = true;
                state.closed = closed;
            }
        }
    }
}

#[cfg(test)]
#[path = "transfer_namespace_tests.rs"]
mod tests;
