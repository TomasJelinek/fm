//! LiveValidated revisions detect changes; they do not promise an immutable snapshot.
use crate::{Error, api::viewer, native};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static REVISIONS: AtomicU64 = AtomicU64::new(1);

/// Which difference from a capture's native revision still counts as unchanged. A native
/// revision includes the file's change time (NTFS ChangeTime, Linux ctime), which also
/// moves on metadata-only updates: endpoint protection, for example, adds an alternate
/// data stream to some freshly written files one or two seconds after they are written.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RevisionPolicy {
    /// Every captured field, the change time included, must still match.
    #[default]
    Exact,
    /// A new change time alone is accepted (VIEW-05): identity, length and modification
    /// time must still match (Windows: volume, file ID, length and LastWriteTime; Linux:
    /// device, inode, length and mtime). Viewers read their content under this policy.
    AcceptChangeTimeOnly,
}

/// Optional relative-content capability captured with directory/mount authority.
/// Implementations enforce no-follow containment against held objects. Never part of a plugin ABI.
pub trait ContentResources: Send + Sync + std::fmt::Debug {
    fn capture(&self, segments: &[String]) -> Result<CapturedFile, Error>;

    /// Consume this reference on a backend worker. Known implementations release
    /// only their reference when shared, and acknowledge actual child teardown
    /// when last. An error or unwind must retain every unconfirmed owner.
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        std::mem::forget(self);
        Err(Error("Content resource teardown is unconfirmed".into()))
    }
}

/// Host-owned content provider. Plugins receive the ordinary bounded reader ABI,
/// never this trait, a native path, or the provider implementation.
pub trait ContentSource: Send + Sync + std::fmt::Debug {
    fn length(&self) -> u64;
    fn display_name(&self) -> String;
    fn version(&self) -> Vec<u8>;
    fn validate(&self) -> Result<(), Error>;
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error>;

    /// Cached observation of an already unconfirmed cleanup/callback outcome.
    /// This query performs no I/O, capture, release or ownership probe. False is
    /// not retirement qualification; unknown release implementations still fail.
    fn cleanup_unconfirmed(&self) -> bool {
        false
    }

    /// Cached observation that the captured file was changed: a reader opened on its path
    /// reported another identity, length or revision than the capture's policy accepts,
    /// so repeating a failed read or revalidation is not expected to succeed. This query
    /// performs no I/O. False only means that no such change was observed; a failure for
    /// another reason, such as an unreachable file, leaves it false.
    fn changed_since_capture(&self) -> bool {
        false
    }

    /// Consume this reference on a backend worker. A known shared reference may
    /// return immediately; a last owner must confirm native cleanup before Ok.
    /// Errors/unwinds retain unconfirmed native and accounting tails. Unknown
    /// implementations deliberately retain this reference rather than call Drop.
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        std::mem::forget(self);
        Err(Error("Content source teardown is unconfirmed".into()))
    }

    /// Read against the original captured metadata, validating before and after the read.
    /// A trusted adapter may override this only when its underlying read operation provides
    /// the same live revision/identity checks. Bounds and returned lengths remain host checked.
    /// This Rust host interface never crosses a plugin ABI.
    fn read_checked(
        &self,
        offset: u64,
        length: u32,
        expected_length: u64,
        expected_version: &[u8],
    ) -> Result<Vec<u8>, Error> {
        validate_source(self, expected_length, expected_version)?;
        let bytes = self.read_at(offset, length)?;
        validate_source(self, expected_length, expected_version)?;
        Ok(bytes)
    }
}

#[derive(Clone, Debug)]
pub struct CapturedFile {
    path: PathBuf,
    snapshot: Option<native::Snapshot>,
    /// How later native reads compare the current file with `snapshot`.
    policy: RevisionPolicy,
    source: Option<Arc<dyn ContentSource>>,
    source_version: Vec<u8>,
    source_length: u64,
    revision: u64,
    resources: Option<Arc<dyn ContentResources>>,
}
pub(crate) enum Reader {
    Native {
        file: File,
        snapshot: native::Snapshot,
        policy: RevisionPolicy,
    },
    Provider {
        source: Arc<dyn ContentSource>,
        version: Vec<u8>,
        length: u64,
    },
}

impl CapturedFile {
    /// Observe only the backing source's cached cleanup status, without opening
    /// content or inspecting unrelated resource/cache ownership.
    pub fn cleanup_unconfirmed(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| source.cleanup_unconfirmed())
    }

    /// Whether the backing source found that the captured file was changed
    /// (`ContentSource::changed_since_capture`), so that repeating a failed read or
    /// revalidation is not expected to succeed. No I/O. An in-process native capture
    /// reports false.
    pub fn changed_since_capture(&self) -> bool {
        self.source
            .as_ref()
            .is_some_and(|source| source.changed_since_capture())
    }

    /// Explicit backend release of this capture's content, then resource authority.
    /// Plain native path/snapshot fields are metadata; they need no close protocol.
    pub fn release_on_worker(self) -> Result<(), Error> {
        let mut pending = PendingContent(Some(self));
        let file = pending.0.as_mut().expect("captured content remains owned");
        if let Some(source) = file.source.take() {
            source.release_on_worker()?;
        }
        if let Some(resources) = file.resources.take() {
            resources.release_on_worker()?;
        }
        drop(pending.0.take());
        Ok(())
    }

    pub fn with_resources(mut self, resources: Arc<dyn ContentResources>) -> Self {
        self.resources = Some(resources);
        self
    }
    pub fn resources(&self) -> Option<Arc<dyn ContentResources>> {
        self.resources.clone()
    }
    /// Backend only. Opens briefly to obtain stable native identity, then closes.
    /// Every later read requires the exact captured revision.
    pub fn capture(path: &Path) -> Result<Self, Error> {
        Self::capture_with(path, RevisionPolicy::Exact)
    }
    /// `capture` whose later reads and revalidation compare the current file with the
    /// captured revision under `policy`. The revision bytes (`stable_version`) are the
    /// same for every policy.
    pub fn capture_with(path: &Path, policy: RevisionPolicy) -> Result<Self, Error> {
        if !path.is_absolute() {
            return Err(Error("Captured path must be absolute".into()));
        }
        let file = native::open_read(path)?;
        let snapshot = native::snapshot(&file)?;
        let revision = REVISIONS
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| Error("Revision IDs exhausted".into()))?;
        Ok(Self {
            path: path.into(),
            snapshot: Some(snapshot),
            policy,
            source: None,
            source_version: Vec::new(),
            source_length: 0,
            revision,
            resources: None,
        })
    }
    /// The absolute path this capture was taken from (`capture`), or empty when a
    /// provider source supplies the content (`from_source`). Diagnostic only: callers
    /// read content through this capture's revision-checked reads, never by opening
    /// this path themselves.
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn length(&self) -> u64 {
        self.source.as_ref().map_or_else(
            || self.snapshot.as_ref().map_or(0, |s| s.length),
            |_| self.source_length,
        )
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Stable native identity and revision bytes for host-owned cache keys.
    /// Versioned separately for each platform; not a portable filesystem address.
    pub fn stable_version(&self) -> Vec<u8> {
        self.source.as_ref().map_or_else(
            || {
                self.snapshot
                    .as_ref()
                    .map_or_else(Vec::new, |s| s.version_bytes())
            },
            |_| self.source_version.clone(),
        )
    }
    /// Validate the captured native identity against the current path.
    pub fn revalidate(&self) -> Result<(), Error> {
        self.open().map(|_| ())
    }
    /// Bounded broker read. Reopening validates both pathname replacement and
    /// before/after handle metadata against the original captured revision.
    pub fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        if let Some(source) = &self.source {
            return read_source(
                source.as_ref(),
                offset,
                length,
                self.source_length,
                &self.source_version,
            );
        }
        self.open()?.read_at(offset, length)
    }
    pub fn display_name(&self) -> String {
        if let Some(source) = &self.source {
            return source.display_name();
        }
        self.path
            .file_name()
            .unwrap_or(self.path.as_os_str())
            .to_string_lossy()
            .into_owned()
    }
    /// Backend only; reads at most 4096 bytes through the same revision validation.
    pub fn preferred_basic_mode(&self) -> Result<u16, Error> {
        let bytes = self.read_at(0, 4096)?;
        Ok(if crate::text_detection::is_text(&bytes) {
            viewer::MODE_TEXT
        } else {
            viewer::MODE_BINARY
        })
    }
    pub(crate) fn open(&self) -> Result<Reader, Error> {
        if let Some(source) = &self.source {
            validate_source(source.as_ref(), self.source_length, &self.source_version)?;
            return Ok(Reader::Provider {
                source: source.clone(),
                version: self.source_version.clone(),
                length: self.source_length,
            });
        }
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or_else(|| Error("Missing content identity".into()))?;
        let file = native::open_read(&self.path)?;
        let current = native::snapshot(&file)?;
        if !snapshot.matches(&current, self.policy) {
            return Err(native_changed("open", &self.path, snapshot, &current));
        }
        Ok(Reader::Native {
            file,
            snapshot: snapshot.clone(),
            policy: self.policy,
        })
    }
    /// Capture a non-native content lease without fabricating a host filesystem path.
    /// The source applies its own revision policy in `validate` and `read_checked`.
    pub fn from_source(source: Arc<dyn ContentSource>) -> Result<Self, Error> {
        let source_version = source.version();
        let source_length = source.length();
        if source_version.is_empty() || source_version.len() > 4096 {
            return Err(Error("Invalid provider revision".into()));
        }
        validate_source(source.as_ref(), source_length, &source_version)?;
        let revision = REVISIONS
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
            .map_err(|_| Error("Revision IDs exhausted".into()))?;
        Ok(Self {
            path: PathBuf::new(),
            snapshot: None,
            policy: RevisionPolicy::Exact,
            source: Some(source),
            source_version,
            source_length,
            revision,
            resources: None,
        })
    }
}
// This owner never invokes a remaining native/resource destructor on an error or
// unwind. The child's consuming release contract owns any reference already taken.
struct PendingContent<T>(Option<T>);
impl<T> Drop for PendingContent<T> {
    fn drop(&mut self) {
        if let Some(value) = self.0.take() {
            std::mem::forget(value);
        }
    }
}
impl Reader {
    pub(crate) fn read_at(&mut self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        if let Self::Provider {
            source,
            version,
            length: source_length,
        } = self
        {
            return read_source(source.as_ref(), offset, length, *source_length, version);
        }
        check_read_bounds(offset, length)?;
        let Self::Native {
            file,
            snapshot,
            policy,
        } = self
        else {
            return Err(Error("Invalid content reader".into()));
        };
        let current = native::snapshot(file)?;
        if !snapshot.matches(&current, *policy) {
            return Err(native_changed("read_before", file, snapshot, &current));
        }
        let length = snapshot
            .length
            .saturating_sub(offset)
            .min(u64::from(length)) as usize;
        let mut bytes = vec![0; length];
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut bytes)?;
        let current = native::snapshot(file)?;
        if !snapshot.matches(&current, *policy) {
            return Err(native_changed("read_after", file, snapshot, &current));
        }
        Ok(bytes)
    }
}
fn check_read_bounds(offset: u64, length: u32) -> Result<(), Error> {
    if length == 0
        || length as usize > viewer::MAX_PAGE_BYTES
        || offset.checked_add(u64::from(length)).is_none()
    {
        return Err(Error("Invalid read bounds".into()));
    }
    Ok(())
}

fn read_source(
    source: &dyn ContentSource,
    offset: u64,
    length: u32,
    expected_length: u64,
    expected_version: &[u8],
) -> Result<Vec<u8>, Error> {
    check_read_bounds(offset, length)?;
    let length = expected_length
        .saturating_sub(offset)
        .min(u64::from(length)) as u32;
    // EOF still validates the captured source, without issuing an invalid native read.
    if length == 0 {
        validate_source(source, expected_length, expected_version)?;
        return Ok(Vec::new());
    }
    let bytes = source.read_checked(offset, length, expected_length, expected_version)?;
    if bytes.len() != length as usize {
        return Err(Error(
            "Provider returned an inconsistent read length".into(),
        ));
    }
    Ok(bytes)
}

fn native_changed(
    stage: &str,
    source: &impl std::fmt::Debug,
    expected: &native::Snapshot,
    current: &native::Snapshot,
) -> Error {
    // Keep both snapshots before the potentially long, Debug-escaped source path.
    const LIMIT: usize = 4096;
    const TRUNCATED: &str = "... (truncated)";
    let mut message = format!(
        "{} [stage={stage}; expected={expected:?}; current={current:?}; source={source:?}]",
        changed(),
    );
    if message.len() > LIMIT {
        let mut end = LIMIT - TRUNCATED.len();
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
        message.push_str(TRUNCATED);
    }
    Error(message)
}

fn changed() -> Error {
    Error("Source changed since capture (LiveValidated revision)".into())
}
fn validate_source(
    source: &(impl ContentSource + ?Sized),
    length: u64,
    version: &[u8],
) -> Result<(), Error> {
    source.validate()?;
    if source.length() != length || source.version() != version {
        return Err(changed());
    }
    Ok(())
}

#[cfg(test)]
#[path = "checked_read_tests.rs"]
mod checked_read_tests;

#[cfg(test)]
#[path = "content_retirement_tests.rs"]
mod content_retirement_tests;

#[cfg(test)]
mod provider_tests {
    use super::*;
    #[derive(Debug, Default)]
    struct Source {
        version: AtomicU64,
        oversized: std::sync::atomic::AtomicBool,
        changed: std::sync::atomic::AtomicBool,
    }
    impl ContentSource for Source {
        fn changed_since_capture(&self) -> bool {
            self.changed.load(Ordering::Acquire)
        }
        fn length(&self) -> u64 {
            5
        }
        fn display_name(&self) -> String {
            "same.txt".into()
        }
        fn version(&self) -> Vec<u8> {
            self.version.load(Ordering::Acquire).to_le_bytes().to_vec()
        }
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
        fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
            if self.oversized.load(Ordering::Acquire) {
                return Ok(vec![0; length as usize + 1]);
            }
            Ok(b"hello"[offset as usize..offset as usize + length as usize].to_vec())
        }
    }
    #[test]
    fn provider_reader_bounds_and_revision_are_captured_without_native_path() {
        let source = Arc::new(Source::default());
        let captured = CapturedFile::from_source(source.clone()).unwrap();
        assert!(captured.path().as_os_str().is_empty());
        assert_eq!(captured.display_name(), "same.txt");
        assert_eq!(captured.read_at(1, 3).unwrap(), b"ell");
        assert_eq!(captured.read_at(3, 99).unwrap(), b"lo");
        assert!(captured.read_at(99, 1).unwrap().is_empty());
        assert!(captured.read_at(0, 0).is_err());
        assert!(captured.read_at(0, 256 * 1024 + 1).is_err());
        assert!(captured.read_at(u64::MAX, 1).is_err());
        let revision = captured.stable_version();
        source.version.store(1, Ordering::Release);
        assert_eq!(captured.stable_version(), revision);
        assert!(captured.read_at(0, 1).is_err());
        assert!(captured.revalidate().is_err());
    }
    /// Archive mounts report a capture whose file was changed as a failure that a Retry
    /// cannot undo; only the source can observe it.
    #[test]
    fn a_capture_reports_the_lasting_change_its_source_observed() {
        let source = Arc::new(Source::default());
        let captured = CapturedFile::from_source(source.clone()).unwrap();
        assert!(!captured.changed_since_capture());
        source.changed.store(true, Ordering::Release);
        assert!(captured.changed_since_capture());
    }
    #[test]
    fn provider_cannot_return_more_than_the_broker_requested() {
        let source = Arc::new(Source::default());
        source.oversized.store(true, Ordering::Release);
        let captured = CapturedFile::from_source(source).unwrap();
        assert!(captured.read_at(0, 2).is_err());
    }
}
