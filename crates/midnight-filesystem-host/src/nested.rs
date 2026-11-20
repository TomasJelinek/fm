//! Archives inside archives. The enclosing mount stages the member completely and
//! verified (`Mount::stage_member`), in memory under the staging budget or in a private
//! spool file, and another mount reads that lease as its captured source. The lease is
//! independent of the enclosing mount's member cache and worker; every use of the
//! source revalidates the enclosing chain down to the native file. Neither worker
//! receives a path, and the inner worker sees only bounded source reads.
use crate::staging::Lease;
use midnight_core::{ArchiveContent, ArchiveParent, FsError, MAX_ARCHIVE_DEPTH};
use midnight_plugin_host::{ContentSource, Error};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

/// Mounts of archives inside archives that may be active at once, beside the two
/// archives opened from native files: enough for one panel to reach the deepest level.
pub(crate) const MAX_NESTED_ACTIVE: usize = MAX_ARCHIVE_DEPTH - 1;
/// Revisions longer than an archive address keeps are replaced by a mount-local one.
const MAX_REVISION: usize = 256;
const STABLE: &[u8] = b"MidnightNestedArchive\0v1\0";
const LOCAL: &[u8] = b"MidnightNestedArchive\0local\0";

/// One member's whole verified content, retained after the enclosing mount's member
/// cache evicts its own reference and after that mount's worker has ended.
pub(crate) struct StagedMember {
    lease: Lease,
}
impl StagedMember {
    pub(crate) fn new(lease: Lease) -> Self {
        Self { lease }
    }
}
impl std::fmt::Debug for StagedMember {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StagedMember")
            .field("length", &self.lease.length())
            .finish_non_exhaustive()
    }
}
impl ArchiveContent for StagedMember {
    fn length(&self) -> u64 {
        self.lease.length()
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, FsError> {
        self.lease
            .read(offset, length)
            .map_err(|error| FsError::from_io("read archive member", Path::new(""), error))
    }
}

/// The enclosing archive of a nested mount. The mount reports it as its parent; the
/// mount's source validates through it and gives its mount reference up last.
pub(crate) struct Link {
    parent: Mutex<Option<ArchiveParent>>,
    /// Display path of the inner archive, for errors: `/data/outer.zip/inner.zip`.
    pub(crate) label: PathBuf,
    /// Whether the chain's member IDs and revisions identify this content across
    /// sessions, so that previews of its members may be cached persistently.
    pub(crate) persistent: bool,
}
impl Link {
    pub(crate) fn new(parent: ArchiveParent, persistent: bool) -> Self {
        let label = label(&parent);
        Self {
            parent: Mutex::new(Some(parent)),
            label,
            persistent,
        }
    }
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<ArchiveParent>> {
        self.parent
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
    /// None once the source gave the enclosing archive up.
    pub(crate) fn parent(&self) -> Option<ArchiveParent> {
        self.lock().clone()
    }
    fn mount(&self) -> Option<Arc<dyn midnight_core::ArchiveMount>> {
        self.lock()
            .as_ref()
            .and_then(|parent| parent.location.mount.clone())
    }
    /// Revalidates the enclosing archive, and through it the chain down to the native
    /// file, outside the lock: the UI reads the parent meanwhile.
    fn validate(&self) -> Result<(), FsError> {
        let mount = self.mount().ok_or_else(|| {
            FsError::from_io(
                "archive source changed",
                &self.label,
                io::Error::other("The archive that contains this archive is closed"),
            )
        })?;
        mount.validate()
    }
    /// A cached status of the enclosing session, read under the lock since it performs
    /// no I/O: its unconfirmed cleanup taints this source as well.
    fn cleanup_unconfirmed(&self) -> bool {
        self.lock()
            .as_ref()
            .and_then(|parent| parent.location.mount.as_ref())
            .is_some_and(|mount| mount.cleanup_unconfirmed())
    }
    /// Gives the enclosing archive's mount reference up through its own protocol.
    fn release(&self) -> Result<(), FsError> {
        let parent = self.lock().take();
        match parent.and_then(|mut parent| parent.location.mount.take()) {
            Some(mount) => mount.release_on_worker(),
            None => Ok(()),
        }
    }
}

/// `/data/outer.zip/folder/inner.zip`: presentation only, never opened.
pub(crate) fn label(parent: &ArchiveParent) -> PathBuf {
    let mut text = parent.location.display_text();
    if !text.ends_with(std::path::MAIN_SEPARATOR) {
        text.push(std::path::MAIN_SEPARATOR);
    }
    text.push_str(&parent.name);
    PathBuf::from(text)
}

/// The captured source of a nested mount: the staged member, read through the host's
/// bounded broker like any captured file.
pub(crate) struct NestedSource {
    link: Arc<Link>,
    content: Arc<dyn ArchiveContent>,
    version: Vec<u8>,
    name: String,
    length: u64,
}
impl NestedSource {
    pub(crate) fn new(
        link: Arc<Link>,
        content: Arc<dyn ArchiveContent>,
        version: Vec<u8>,
        name: String,
    ) -> Self {
        let length = content.length();
        Self {
            link,
            content,
            version,
            name,
            length,
        }
    }
}
impl std::fmt::Debug for NestedSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NestedSource")
            .field("label", &self.link.label)
            .field("length", &self.length)
            .finish_non_exhaustive()
    }
}
impl ContentSource for NestedSource {
    fn length(&self) -> u64 {
        self.length
    }
    fn display_name(&self) -> String {
        self.name.clone()
    }
    fn version(&self) -> Vec<u8> {
        self.version.clone()
    }
    fn validate(&self) -> Result<(), Error> {
        self.link
            .validate()
            .map_err(|error| Error(error.to_string()))
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.content.read_at(offset, length).map_err(|error| {
            Error(format!(
                "The staged copy of {} is unreadable: {}",
                self.link.label.display(),
                error.message
            ))
        })
    }
    fn cleanup_unconfirmed(&self) -> bool {
        self.link.cleanup_unconfirmed()
    }
    /// A shared reference only gives up its count. The last one closes the staged
    /// content on this backend worker, then gives the enclosing archive up.
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        let Some(source) = Arc::into_inner(self) else {
            return Ok(());
        };
        let NestedSource { link, content, .. } = source;
        drop(content);
        link.release().map_err(|error| Error(error.to_string()))
    }
}

/// The captured revision of a nested source. A chain whose member IDs are stable gets
/// the enclosing revision and the member, so that it names the same content in a later
/// session; otherwise, or when that would not fit an archive address, the revision is
/// unique to this mount and grants no persistent preview cache authority.
pub(crate) fn revision(parent_version: &[u8], entry: u64, stable: bool) -> (Vec<u8>, bool) {
    if stable && STABLE.len() + 8 + parent_version.len() <= MAX_REVISION {
        let mut version = STABLE.to_vec();
        version.extend_from_slice(&entry.to_le_bytes());
        version.extend_from_slice(parent_version);
        return (version, true);
    }
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let mut version = LOCAL.to_vec();
    version.extend_from_slice(&NEXT.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    (version, false)
}

#[cfg(test)]
#[path = "nested_tests.rs"]
mod tests;
