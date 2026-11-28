//! Pooled metadata-only directory authority, explicitly qualified native lookup rules and
//! the checks that a pinned folder's ancestry is unchanged.
use super::*;
use midnight_platform_common::transfer_namespace::current_transfer_namespace_refresh_batch;
use std::{
    cell::RefCell,
    fs::OpenOptions,
    mem::{offset_of, size_of},
    os::windows::{ffi::OsStrExt, fs::OpenOptionsExt, io::AsRawHandle},
};
use windows_sys::{
    Wdk::Storage::FileSystem::{
        FILE_LINK_ENTRY_INFORMATION, FILE_LINKS_INFORMATION, FileHardLinkInformation,
        NtQueryInformationFile,
    },
    Win32::System::{
        IO::{DeviceIoControl, IO_STATUS_BLOCK},
        Ioctl::{
            FILE_FS_PERSISTENT_VOLUME_INFORMATION, FSCTL_QUERY_PERSISTENT_VOLUME_STATE,
            PERSISTENT_VOLUME_STATE_SHORT_NAME_CREATION_DISABLED,
        },
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LookupMode {
    ShortNamesDisabled { case_sensitive: bool },
    Unqualified,
}
impl LookupMode {
    pub(super) fn key(self, name: &OsStr) -> Option<Arc<[u8]>> {
        let Self::ShortNamesDisabled { case_sensitive } = self else {
            return None;
        };
        let mut result = Vec::new();
        for mut unit in name.encode_wide() {
            // An insensitive NTFS directory uses a native upcase table. Only ASCII is
            // qualified here; Unicode lowercasing is not evidence of native equivalence.
            if !case_sensitive {
                if unit > 0x7f {
                    return None;
                }
                if (b'a' as u16..=b'z' as u16).contains(&unit) {
                    unit -= 32;
                }
            }
            result.extend_from_slice(&unit.to_le_bytes());
        }
        Some(result.into())
    }
    fn domain(self) -> Arc<[u8]> {
        match self {
            Self::ShortNamesDisabled { case_sensitive } => {
                let mut domain = b"windows.ntfs.short-names-disabled.v1".to_vec();
                domain.push(u8::from(case_sensitive));
                domain.into()
            }
            Self::Unqualified => Arc::from([]),
        }
    }
}

struct Anchor {
    file: File,
    identity: TransferIdentity,
    physical: PathBuf,
}
struct Volume {
    file: File,
    root: Arc<Anchor>,
}
pub(super) struct Parent {
    anchor: Arc<Anchor>,
    // Flat Arc owners avoid a recursive native-handle ownership chain.
    chain: Vec<Arc<Anchor>>,
    pub(super) snapshot: Arc<TransferNamespaceParent>,
    pub(super) mode: LookupMode,
    volume: Option<Arc<Volume>>,
    /// The held folder's normalized final path with its NT volume name, pinned at capture
    /// when `validate` proves this ancestry by handle (`Parent::proven`): qualified lookup
    /// rules only. Otherwise None, and `validate` opens each level (`validate_levels`).
    pinned: Option<PathBuf>,
}
type Pool<T> = Mutex<HashMap<TransferIdentity, Weak<T>>>;
static ANCHORS: OnceLock<Pool<Anchor>> = OnceLock::new();
static PARENTS: OnceLock<Pool<Parent>> = OnceLock::new();
static VOLUMES: OnceLock<Pool<Volume>> = OnceLock::new();

/// What this thread's current refresh batch (`TransferNamespaceRefreshBatch`) has already
/// checked. A batch changes no names, so a check that held once in it holds for the rest
/// of it: the pins and the stage or output of one change share their folders' ancestry,
/// and checking each of them would otherwise probe every ancestor again. An external rename
/// during a batch is seen by the next batch, as by a check that ran just before the rename.
/// The weak references keep each checked allocation's address unique while it is recorded
/// without keeping its handle open.
#[derive(Default)]
struct BatchChecks {
    batch: u64,
    /// Anchors whose held handle still had the pinned identity.
    anchors: Vec<Weak<Anchor>>,
    /// Child and parent anchors whose name binding was checked.
    edges: Vec<(Weak<Anchor>, Weak<Anchor>)>,
    /// Held folders that still had their pinned path and identity and no pending delete.
    /// An anchor's pinned path follows from its physical path, so the anchor identifies it.
    folders: Vec<Weak<Anchor>>,
    /// Child and parent anchors whose link (parent reference and exact name) still held.
    links: Vec<(Weak<Anchor>, Weak<Anchor>)>,
    /// Roots whose volume path still opened them.
    roots: Vec<Weak<Anchor>>,
    /// Folders whose lookup rules were read, the volume they were read through and the
    /// rules found.
    modes: Vec<(Weak<Anchor>, Option<Weak<Volume>>, LookupMode)>,
    /// Volumes whose persistent state disabled 8.3-name creation and whose root path still
    /// opened their pinned root: the part of the lookup rules that every folder of the
    /// volume shares (`lookup_mode`).
    volumes: Vec<Weak<Volume>>,
    /// Folders that a binding's recorded path still opened, with that path.
    paths: Vec<(Weak<Anchor>, PathBuf)>,
}
/// A batch rarely spans more than a few folders; checks beyond this are made each time.
const MAX_BATCH_CHECKS: usize = 256;

thread_local! {
    static BATCH_CHECKS: RefCell<BatchChecks> = RefCell::default();
}

#[cfg(test)]
thread_local! {
    /// Native ancestor-binding probes made by `Parent::validate` on this thread, for tests:
    /// an open by name (`validate_levels`) or a link query (`proven`) per binding.
    pub(super) static EDGE_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Proofs of a held folder's path, identity and delete state made on this thread.
    pub(super) static FOLDER_PROOFS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    /// Reads of a volume's 8.3 state and root binding made on this thread
    /// (`volume_disables_short_names`).
    pub(super) static VOLUME_PROOFS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Run `check` on this thread's record of its current refresh batch; None outside a batch.
/// The record of an earlier batch is discarded first.
fn batch_checks<T>(check: impl FnOnce(&mut BatchChecks) -> T) -> Option<T> {
    let batch = current_transfer_namespace_refresh_batch()?;
    BATCH_CHECKS.with(|checks| {
        let mut checks = checks.borrow_mut();
        if checks.batch != batch {
            *checks = BatchChecks {
                batch,
                ..BatchChecks::default()
            };
        }
        Some(check(&mut checks))
    })
}

/// Record a check that held in the current batch, if there is one and it has room.
fn remember<T>(list: impl FnOnce(&mut BatchChecks) -> &mut Vec<T>, entry: T) {
    batch_checks(|checks| {
        let list = list(checks);
        if list.len() < MAX_BATCH_CHECKS {
            list.push(entry);
        }
    });
}

fn same(recorded: &Weak<Anchor>, anchor: &Arc<Anchor>) -> bool {
    std::ptr::eq(recorded.as_ptr(), Arc::as_ptr(anchor))
}

fn same_volume(recorded: &Option<Weak<Volume>>, volume: Option<&Arc<Volume>>) -> bool {
    match (recorded, volume) {
        (None, None) => true,
        (Some(recorded), Some(volume)) => std::ptr::eq(recorded.as_ptr(), Arc::as_ptr(volume)),
        _ => false,
    }
}

fn metadata_directory(path: &Path) -> Result<File, FsError> {
    let file = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_TRAVERSE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|e| error(path, e))?;
    let metadata = file.metadata().map_err(|e| error(path, e))?;
    // Cloud Files placeholders (a OneDrive root and every folder below it) carry a
    // non-surrogate reparse tag and are ordinary directories; only links are refused.
    let native = crate::reparse::Native::of(&file, metadata.file_attributes())
        .map_err(|e| error(path, e))?;
    if !native.directory() || native.link() {
        return Err(invalid(
            path,
            "native namespace ancestor is not an ordinary directory",
        ));
    }
    Ok(file)
}

/// Pools are keyed by identity, but an identity alone does not prove the binding: a
/// directory renamed after it was pooled, or a cloned/snapshot volume with the same
/// serial number and file IDs, would hand out a stale or foreign path. A pooled entry
/// is reused only while its GUID physical path also matches; otherwise a fresh entry
/// replaces the slot and existing holders keep (and revalidate) their own.
fn pooled<T>(
    pool: &Pool<T>,
    identity: TransferIdentity,
    current: impl Fn(&T) -> bool,
) -> Option<Arc<T>> {
    // The pool's lock is released at the end of this statement, before a stale entry is
    // dropped: its other holders may have let go meanwhile, and then its Drop locks the
    // pool to remove its own entry.
    let found = pool
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&identity)
        .and_then(Weak::upgrade);
    found.filter(|found| current(found))
}

/// The current pooled entry of `identity`, or else `candidate`, pooled in its place. A
/// stale entry and an unused candidate are dropped after the pool's lock is released, as
/// in `pooled`.
fn pool_or_found<T>(
    pool: &Pool<T>,
    identity: TransferIdentity,
    current: impl Fn(&T) -> bool,
    candidate: Arc<T>,
) -> Arc<T> {
    let stale;
    {
        let mut pool = pool.lock().unwrap_or_else(|e| e.into_inner());
        match pool.get(&identity).and_then(Weak::upgrade) {
            Some(found) if current(&found) => return found,
            found => stale = found,
        }
        pool.insert(identity, Arc::downgrade(&candidate));
    }
    drop(stale);
    candidate
}

fn anchor(path: &Path) -> Result<Arc<Anchor>, FsError> {
    let file = metadata_directory(path)?;
    let identity = identity(&file, path)?;
    let pool = ANCHORS.get_or_init(Default::default);
    let current = |found: &Anchor| found.physical == path;
    if let Some(found) = pooled(pool, identity, current) {
        return Ok(found);
    }
    let candidate = Arc::new(Anchor {
        file,
        identity,
        physical: path.to_owned(),
    });
    Ok(pool_or_found(pool, identity, current, candidate))
}

pub(super) fn capture(
    file: &File,
    path: &Path,
    cancel: &Cancellation,
) -> Result<Arc<Parent>, FsError> {
    checked(cancel, path)?;
    let physical = physical_path(file).map_err(|e| error(path, e))?;
    capture_physical(file, path, physical, cancel)
}

/// `capture` of a folder whose physical path (`physical_path` of `file`) the caller has
/// just read, so that it is not read through the mount manager a second time.
pub(super) fn capture_physical(
    file: &File,
    path: &Path,
    physical: PathBuf,
    cancel: &Cancellation,
) -> Result<Arc<Parent>, FsError> {
    checked(cancel, path)?;
    let expected = identity(file, path)?;
    let pool = PARENTS.get_or_init(Default::default);
    if let Some(found) = pooled(pool, expected, |found: &Parent| {
        found.physical() == physical
    }) {
        return Ok(found);
    }
    let mut chain = Vec::new();
    for component in physical.ancestors().filter(|path| path.has_root()) {
        checked(cancel, path)?;
        chain.push(anchor(component)?);
    }
    let first = chain
        .first()
        .ok_or_else(|| invalid(path, "native parent has no root"))?
        .clone();
    if first.identity != expected {
        return Err(invalid(
            path,
            "native parent changed while capturing its ancestry",
        ));
    }
    let root = chain
        .last()
        .ok_or_else(|| invalid(path, "native parent has no root"))?;
    let volume = open_volume(root).ok();
    let mode = lookup_mode(&first.file, volume.as_ref());
    let snapshot = Arc::new(TransferNamespaceParent {
        identity: expected,
        lookup_domain: mode.domain(),
        ancestors: chain
            .iter()
            .map(|anchor| anchor.identity)
            .collect::<Vec<_>>()
            .into(),
    });
    let mut candidate = Parent {
        anchor: first,
        chain,
        snapshot,
        mode,
        volume,
        pinned: None,
    };
    // A qualified folder proves its ancestry by handle from the path pinned here. Where
    // that proof does not hold at once (a change meanwhile, or link information that this
    // filesystem reports otherwise), the folder keeps the per-level check.
    if matches!(mode, LookupMode::ShortNamesDisabled { .. }) {
        candidate.pinned = nt_path(candidate.file()).ok();
        if !candidate.proven(cancel)? {
            candidate.pinned = None;
        }
    }
    match candidate.pinned {
        Some(_) => candidate.check_rules()?,
        None => candidate.validate_levels(cancel)?,
    }
    Ok(pool_or_found(
        pool,
        expected,
        |found: &Parent| found.physical() == physical,
        Arc::new(candidate),
    ))
}

impl Parent {
    pub(super) fn file(&self) -> &File {
        &self.anchor.file
    }
    /// The ancestor bindings that a check of this folder probes.
    #[cfg(test)]
    pub(super) fn edges(&self) -> usize {
        self.chain.len() - 1
    }
    pub(super) fn physical(&self) -> &Path {
        &self.anchor.physical
    }
    /// Check that the pinned ancestry is unchanged, and this folder's lookup rules. A
    /// qualified folder proves it by handle (`proven`); every other folder opens each level
    /// (`validate_levels`). Within a refresh batch each check is made once (`BatchChecks`).
    pub(super) fn validate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        if self.pinned.is_none() {
            return self.validate_levels(cancel);
        }
        if self.proven(cancel)? {
            return self.check_rules();
        }
        // The per-level check reports the change as it always has. A change that it does
        // not see (a case-only rename in a case-insensitive folder) still counts.
        Err(match self.validate_levels(cancel) {
            Err(error) => error,
            Ok(()) => invalid(
                self.physical(),
                "original namespace ancestor binding changed",
            ),
        })
    }

    /// The handle proof of a qualified folder's pinned ancestry (see "Windows namespace
    /// pins" in `docs/developer/transfers.md`): the held folder keeps its pinned
    /// normalized NT path and identity and no delete is pending on it, and each held level
    /// is still linked, under exactly its pinned name, in the held level above it. Nothing
    /// is opened by name. A failed query or any unexpected reply counts as a change.
    /// Within a refresh batch each part that held is not repeated (`BatchChecks`).
    fn proven(&self, cancel: &Cancellation) -> Result<bool, FsError> {
        let Some(pinned) = &self.pinned else {
            return Ok(false);
        };
        checked(cancel, self.physical())?;
        let known = batch_checks(|checks| {
            checks
                .folders
                .iter()
                .any(|recorded| same(recorded, &self.anchor))
        });
        if known != Some(true) {
            #[cfg(test)]
            FOLDER_PROOFS.with(|proofs| proofs.set(proofs.get() + 1));
            let file = self.file();
            let held = (skipped("final-path")
                || nt_path(file).is_ok_and(|path| path.as_os_str() == pinned.as_os_str()))
                && identity(file, self.physical()).is_ok_and(|found| found == self.anchor.identity)
                && (skipped("delete-pending") || delete_pending(file) == Some(false));
            if !held {
                return Ok(false);
            }
            remember(|checks| &mut checks.folders, Arc::downgrade(&self.anchor));
        }
        if skipped("links") {
            return Ok(true);
        }
        // The root has no held level above it: its binding to the volume is the pinned
        // path's volume name, and the lookup rules check its volume path.
        for edge in self.chain.windows(2) {
            let child = &edge[0];
            let parent = &edge[1];
            let known = batch_checks(|checks| {
                checks
                    .links
                    .iter()
                    .any(|(recorded, above)| same(recorded, child) && same(above, parent))
            });
            if known == Some(true) {
                continue;
            }
            #[cfg(test)]
            EDGE_PROBES.with(|probes| probes.set(probes.get() + 1));
            if !linked(child, parent) {
                return Ok(false);
            }
            remember(
                |checks| &mut checks.links,
                (Arc::downgrade(child), Arc::downgrade(parent)),
            );
        }
        Ok(true)
    }

    /// The per-level check of the whole pinned ancestry: each held anchor's identity, each
    /// ancestor's name binding in its parent (opened by name), the root's volume path and
    /// this folder's lookup rules.
    fn validate_levels(&self, cancel: &Cancellation) -> Result<(), FsError> {
        for anchor in &self.chain {
            checked(cancel, &anchor.physical)?;
            let known =
                batch_checks(|checks| checks.anchors.iter().any(|recorded| same(recorded, anchor)));
            if known == Some(true) {
                continue;
            }
            if identity(&anchor.file, &anchor.physical)? != anchor.identity {
                return Err(invalid(
                    &anchor.physical,
                    "pinned namespace ancestor changed",
                ));
            }
            remember(|checks| &mut checks.anchors, Arc::downgrade(anchor));
        }
        for edge in self.chain.windows(2) {
            let child = &edge[0];
            let parent = &edge[1];
            let known = batch_checks(|checks| {
                checks
                    .edges
                    .iter()
                    .any(|(recorded, above)| same(recorded, child) && same(above, parent))
            });
            if known == Some(true) {
                continue;
            }
            let name = child
                .physical
                .file_name()
                .ok_or_else(|| invalid(&child.physical, "native namespace edge has no basename"))?;
            #[cfg(test)]
            EDGE_PROBES.with(|probes| probes.set(probes.get() + 1));
            let visible = native::open_relative(
                &parent.file,
                name,
                FILE_READ_ATTRIBUTES | SYNCHRONIZE,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                FILE_OPEN,
                0,
            )
            .map_err(|e| error(&child.physical, e))?;
            if identity(&visible, &child.physical)? != child.identity {
                return Err(invalid(
                    &child.physical,
                    "original namespace ancestor binding changed",
                ));
            }
            remember(
                |checks| &mut checks.edges,
                (Arc::downgrade(child), Arc::downgrade(parent)),
            );
        }
        let root = self
            .chain
            .last()
            .ok_or_else(|| invalid(self.physical(), "missing root"))?;
        let known = batch_checks(|checks| checks.roots.iter().any(|recorded| same(recorded, root)));
        if known != Some(true) {
            let visible = metadata_directory(&root.physical)?;
            if identity(&visible, &root.physical)? != root.identity {
                return Err(invalid(
                    &root.physical,
                    "native namespace root binding changed",
                ));
            }
            remember(|checks| &mut checks.roots, Arc::downgrade(root));
        }
        self.check_rules()
    }

    /// This folder's case and 8.3 rules are still the pinned ones. Within a refresh batch
    /// they are read once per folder and volume (`BatchChecks`).
    fn check_rules(&self) -> Result<(), FsError> {
        let recorded = batch_checks(|checks| {
            checks
                .modes
                .iter()
                .find(|(folder, volume, _)| {
                    same(folder, &self.anchor) && same_volume(volume, self.volume.as_ref())
                })
                .map(|(_, _, mode)| *mode)
        })
        .flatten();
        let mode = match recorded {
            Some(mode) => mode,
            None => {
                let mode = lookup_mode(self.file(), self.volume.as_ref());
                remember(
                    |checks| &mut checks.modes,
                    (
                        Arc::downgrade(&self.anchor),
                        self.volume.as_ref().map(Arc::downgrade),
                        mode,
                    ),
                );
                mode
            }
        };
        if mode != self.mode {
            return Err(invalid(
                self.physical(),
                "native case or short-name rules changed",
            ));
        }
        Ok(())
    }

    /// The display path that a binding recorded for this folder still opens it. Within a
    /// refresh batch each path is opened once (`BatchChecks`).
    pub(super) fn check_path(&self, path: &Path) -> Result<(), FsError> {
        let known = batch_checks(|checks| {
            checks
                .paths
                .iter()
                .any(|(recorded, checked)| same(recorded, &self.anchor) && checked == path)
        });
        if known == Some(true) {
            return Ok(());
        }
        let visible = native::open_parent(path).map_err(|e| error(path, e))?;
        if identity(&visible, path)? != self.snapshot.identity {
            return Err(invalid(path, "original pinned parent binding changed"));
        }
        remember(
            |checks| &mut checks.paths,
            (Arc::downgrade(&self.anchor), path.to_owned()),
        );
        Ok(())
    }
}

fn open_volume(root: &Arc<Anchor>) -> Result<Arc<Volume>, FsError> {
    let pool = VOLUMES.get_or_init(Default::default);
    // NTFS roots share file ID 5; the GUID root path distinguishes cloned volumes.
    let current = |found: &Volume| found.root.physical == root.physical;
    if let Some(found) = pooled(pool, root.identity, current) {
        return Ok(found);
    }
    let path: Vec<_> = root.physical.as_os_str().encode_wide().collect();
    // Retain the physical volume GUID qualification, but query the filesystem's
    // already pinned root directory. A DASD volume-device handle does not serve
    // this FSCTL on the qualified native path.
    if !path.starts_with(&r"\\?\Volume{".encode_utf16().collect::<Vec<_>>()) {
        return Err(unsupported(
            &root.physical,
            "native volume rule query unavailable",
        ));
    }
    // Duplicate the existing authority rather than reopening its display path or
    // broadening access. The duplicate retains the same directory object/rights.
    let file = root
        .file
        .try_clone()
        .map_err(|e| error(&root.physical, e))?;
    let volume = Arc::new(Volume {
        file,
        root: root.clone(),
    });
    Ok(pool_or_found(pool, root.identity, current, volume))
}

// Remove only this allocation's weak entry. Per-insert whole-pool scans would turn a
// large plan with many native parents into quadratic work.
impl Drop for Anchor {
    fn drop(&mut self) {
        if let Some(pool) = ANCHORS.get() {
            let mut pool = pool.lock().unwrap_or_else(|e| e.into_inner());
            if pool
                .get(&self.identity)
                .is_some_and(|value| std::ptr::eq(value.as_ptr(), self))
            {
                pool.remove(&self.identity);
            }
        }
    }
}
impl Drop for Parent {
    fn drop(&mut self) {
        if let Some(pool) = PARENTS.get() {
            let mut pool = pool.lock().unwrap_or_else(|e| e.into_inner());
            if pool
                .get(&self.snapshot.identity)
                .is_some_and(|value| std::ptr::eq(value.as_ptr(), self))
            {
                pool.remove(&self.snapshot.identity);
            }
        }
    }
}
impl Drop for Volume {
    fn drop(&mut self) {
        if let Some(pool) = VOLUMES.get() {
            let mut pool = pool.lock().unwrap_or_else(|e| e.into_inner());
            if pool
                .get(&self.root.identity)
                .is_some_and(|value| std::ptr::eq(value.as_ptr(), self))
            {
                pool.remove(&self.root.identity);
            }
        }
    }
}

fn persistent_short_names_disabled(
    query_result: i32,
    returned: u32,
    response: &FILE_FS_PERSISTENT_VOLUME_INFORMATION,
) -> bool {
    // Version and FlagMask describe the request; successful native replies may
    // leave those output fields zero. Only consume policy flags after a complete
    // successful reply, never from an error or a caller-preinitialized buffer.
    query_result != 0
        && returned >= size_of::<FILE_FS_PERSISTENT_VOLUME_INFORMATION>() as u32
        && response.VolumeFlags & PERSISTENT_VOLUME_STATE_SHORT_NAME_CREATION_DISABLED != 0
}

/// Test seam for the namespace tests (see `docs/developer/transfers.md`): with
/// `short-names-disabled` they run their qualified (incremental) branches on an NTFS
/// volume that still creates 8.3 names (their fixtures never use short names); with
/// `unqualified` they run the full-refresh branches on any volume. The forced mode is
/// process-wide because pooled parent bindings and their mode are shared by
/// concurrently running tests.
#[cfg(test)]
pub(super) const TEST_LOOKUP_ENV: &str = "MIDNIGHT_TEST_NAMESPACE_LOOKUP";

/// `Some(true)`: assume 8.3-name creation is disabled; `Some(false)`: unqualified.
fn forced_lookup() -> Option<bool> {
    #[cfg(test)]
    {
        match std::env::var(TEST_LOOKUP_ENV).as_deref() {
            Ok("short-names-disabled") => Some(true),
            Ok("unqualified") => Some(false),
            _ => None,
        }
    }
    #[cfg(not(test))]
    {
        None
    }
}

/// The lookup rules of `directory` on `volume`. The folder's filesystem and case rules are
/// read each time; the volume's part (`volume_disables_short_names`) is read once per
/// volume within a refresh batch (`BatchChecks`), as a folder's root binding is.
fn lookup_mode(directory: &File, volume: Option<&Arc<Volume>>) -> LookupMode {
    let forced = forced_lookup();
    let Some(volume) = volume.filter(|_| forced != Some(false)) else {
        return LookupMode::Unqualified;
    };
    let mut fs_name = [0u16; 32];
    // SAFETY: live native directory, writable bounded filesystem-name buffer; other
    // optional outputs are null. Failure means unqualified lookup, never inferred NTFS.
    let volume_result = unsafe {
        GetVolumeInformationByHandleW(
            directory.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            fs_name.as_mut_ptr(),
            fs_name.len() as u32,
        )
    };
    let filesystem = fs_name.split(|u| *u == 0).next().unwrap_or_default();
    if volume_result == 0 || !filesystem.iter().copied().eq("NTFS".encode_utf16()) {
        return LookupMode::Unqualified;
    }
    let mut case = FILE_CASE_SENSITIVE_INFO::default();
    // SAFETY: live native directory and correctly sized writable metadata structure.
    let case_result = unsafe {
        GetFileInformationByHandleEx(
            directory.as_raw_handle(),
            FileCaseSensitiveInfo,
            (&mut case as *mut FILE_CASE_SENSITIVE_INFO).cast(),
            size_of::<FILE_CASE_SENSITIVE_INFO>() as u32,
        )
    };
    if case_result == 0 || case.Flags & !1 != 0 {
        return LookupMode::Unqualified;
    }
    let mode = LookupMode::ShortNamesDisabled {
        case_sensitive: case.Flags & 1 != 0,
    };
    // The volume's part holds for every folder of the batch on it once it held for one.
    let known = batch_checks(|checks| {
        checks
            .volumes
            .iter()
            .any(|recorded| std::ptr::eq(recorded.as_ptr(), Arc::as_ptr(volume)))
    });
    if known == Some(true) {
        return mode;
    }
    if !volume_disables_short_names(volume, forced) {
        return LookupMode::Unqualified;
    }
    remember(|checks| &mut checks.volumes, Arc::downgrade(volume));
    mode
}

/// The volume's part of the lookup rules: its persistent state disables 8.3-name creation,
/// and its root path still opens the pinned root.
fn volume_disables_short_names(volume: &Volume, forced: Option<bool>) -> bool {
    #[cfg(test)]
    VOLUME_PROOFS.with(|proofs| proofs.set(proofs.get() + 1));
    let request = FILE_FS_PERSISTENT_VOLUME_INFORMATION {
        FlagMask: PERSISTENT_VOLUME_STATE_SHORT_NAME_CREATION_DISABLED,
        Version: 1,
        ..Default::default()
    };
    let mut response = FILE_FS_PERSISTENT_VOLUME_INFORMATION::default();
    let mut returned = 0;
    // SAFETY: live duplicate of the pinned filesystem-root directory. Fixed-size
    // input/output buffers remain valid for synchronous DeviceIoControl; no overlapped I/O.
    let query_result = unsafe {
        DeviceIoControl(
            volume.file.as_raw_handle(),
            FSCTL_QUERY_PERSISTENT_VOLUME_STATE,
            (&request as *const FILE_FS_PERSISTENT_VOLUME_INFORMATION).cast(),
            size_of::<FILE_FS_PERSISTENT_VOLUME_INFORMATION>() as u32,
            (&mut response as *mut FILE_FS_PERSISTENT_VOLUME_INFORMATION).cast(),
            size_of::<FILE_FS_PERSISTENT_VOLUME_INFORMATION>() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if forced != Some(true) && !persistent_short_names_disabled(query_result, returned, &response) {
        return false;
    }
    // The root still belongs to the originally pinned volume after the query. A path
    // rebound cannot silently supply a different volume's alias policy.
    let root_identity = metadata_directory(&volume.root.physical)
        .and_then(|file| identity(&file, &volume.root.physical));
    root_identity.ok() == Some(volume.root.identity)
}

pub(super) fn physical_path(file: &File) -> io::Result<PathBuf> {
    use super::super::destination::final_path;
    final_path(file, VOLUME_NAME_GUID).or_else(|_| final_path(file, VOLUME_NAME_DOS))
}

/// The held entry's normalized final path with its NT volume name: what a qualified
/// folder's handle proof pins and compares (`Parent::proven`).
fn nt_path(file: &File) -> io::Result<PathBuf> {
    super::super::destination::final_path(file, FILE_NAME_NORMALIZED | VOLUME_NAME_NT)
}

/// Whether a delete is pending on the held entry; None when the query fails.
fn delete_pending(file: &File) -> Option<bool> {
    let mut standard = FILE_STANDARD_INFO::default();
    // SAFETY: live handle and a writable structure of its exact native size.
    let queried = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileStandardInfo,
            (&mut standard as *mut FILE_STANDARD_INFO).cast(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    };
    (queried != 0).then_some(standard.DeletePending)
}

/// Room for one `FileHardLinkInformation` entry with the longest NTFS name (255 units).
const LINKS_BYTES: usize = 768;
#[repr(C, align(8))]
struct Links([u8; LINKS_BYTES]);

/// Whether the held directory `child` is still linked, under exactly its pinned name
/// (case-sensitive), in the held directory `parent`. A directory has one link, and
/// `FileHardLinkInformation` names the file reference of the directory that holds it;
/// more than one entry, a reply that does not fit or a failed query is no proof.
fn linked(child: &Anchor, parent: &Anchor) -> bool {
    let (Some(name), Some(reference)) =
        (child.physical.file_name(), file_reference(parent.identity))
    else {
        return false;
    };
    if child.identity.device != parent.identity.device {
        return false;
    }
    let mut links = Links([0; LINKS_BYTES]);
    let mut status = IO_STATUS_BLOCK::default();
    // SAFETY: live handle; the writable 8-byte-aligned buffer and the status block outlive
    // this synchronous query, which writes at most the given length.
    let result = unsafe {
        NtQueryInformationFile(
            child.file.as_raw_handle(),
            &mut status,
            links.0.as_mut_ptr().cast(),
            LINKS_BYTES as u32,
            FileHardLinkInformation,
        )
    };
    // Also STATUS_BUFFER_OVERFLOW, a warning: the names did not fit.
    if result < 0 {
        return false;
    }
    let reply = &links.0[..status.Information.min(LINKS_BYTES)];
    let entry = offset_of!(FILE_LINKS_INFORMATION, Entry);
    let entries =
        field(reply, offset_of!(FILE_LINKS_INFORMATION, EntriesReturned)).map(u32::from_le_bytes);
    let found = field(
        reply,
        entry + offset_of!(FILE_LINK_ENTRY_INFORMATION, ParentFileId),
    )
    .map(i64::from_le_bytes);
    let units = field(
        reply,
        entry + offset_of!(FILE_LINK_ENTRY_INFORMATION, FileNameLength),
    )
    .map(u32::from_le_bytes);
    let (Some(1), Some(found), Some(units)) = (entries, found, units) else {
        return false;
    };
    let start = entry + offset_of!(FILE_LINK_ENTRY_INFORMATION, FileName);
    let Some(stored) = reply.get(start..start + 2 * units as usize) else {
        return false;
    };
    found == reference
        && stored
            .as_chunks::<2>()
            .0
            .iter()
            .map(|unit| u16::from_le_bytes(*unit))
            .eq(name.encode_wide())
}

/// `N` bytes of a native reply at `offset`, if the reply has them.
fn field<const N: usize>(reply: &[u8], offset: usize) -> Option<[u8; N]> {
    reply.get(offset..offset + N)?.try_into().ok()
}

/// The 64-bit NTFS file reference in a `FileIdInfo` identity, which NTFS extends to 128
/// bits with zeros; None for any other identity.
fn file_reference(identity: TransferIdentity) -> Option<i64> {
    let (reference, extension) = identity.object.split_at(8);
    if extension.iter().any(|byte| *byte != 0) {
        return None;
    }
    reference.try_into().ok().map(i64::from_le_bytes)
}

/// Test seam for red runs of the proof tests (`transfer_namespace_proof_tests.rs`): a
/// comma-separated list of the parts of the handle proof to leave out: `final-path`,
/// `delete-pending` or `links`. Process-wide, read once.
#[cfg(test)]
const TEST_PROOF_SKIP_ENV: &str = "MIDNIGHT_TEST_NAMESPACE_PROOF_SKIP";

/// Whether a red test run leaves this part of the handle proof out; never outside tests.
fn skipped(part: &str) -> bool {
    #[cfg(test)]
    {
        static SKIPPED: OnceLock<String> = OnceLock::new();
        SKIPPED
            .get_or_init(|| std::env::var(TEST_PROOF_SKIP_ENV).unwrap_or_default())
            .split(',')
            .any(|skipped| skipped == part)
    }
    #[cfg(not(test))]
    {
        let _ = part;
        false
    }
}

#[cfg(test)]
#[path = "transfer_namespace_parent_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "transfer_namespace_pool_tests.rs"]
mod pool_tests;

#[cfg(test)]
#[path = "transfer_namespace_proof_tests.rs"]
mod proof_tests;
