//! Read-only virtual locations never contain a guessed native member path.
use crate::{Cancellation, EntryKind, FsError};
use std::sync::Arc;

/// Archive levels one path may open, counting the archive opened from a native file:
/// archives inside archives open at most `MAX_ARCHIVE_DEPTH - 1` levels below it.
pub const MAX_ARCHIVE_DEPTH: usize = 4;

/// Why an archive at the deepest level does not open an archive inside it.
pub fn archive_depth_message() -> String {
    format!(
        "Archives open inside archives at most {MAX_ARCHIVE_DEPTH} levels deep. Copy this \
         archive to a folder with F5 to open it there."
    )
}

/// The session of an archive opened from a native file, as a refused open names the
/// sessions that count (`Event::ArchiveSessionsFull`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveSession {
    /// The session's `ArchiveMount::id`.
    pub id: u64,
    /// The native archive file that the session reads.
    pub container: std::path::PathBuf,
}

/// The enclosing archive of an archive that was opened from one of its members.
#[derive(Clone, Debug)]
pub struct ArchiveParent {
    /// The enclosing archive's folder that holds the member, with that archive's mount.
    pub location: ArchiveLocation,
    /// The member read as the inner archive, in the enclosing archive's mount.
    pub member: VirtualEntryId,
    /// The member's display name, which names the inner archive's root.
    pub name: String,
}

/// The verified content of one archive member, retained by the backend independently of
/// its mount's member cache and worker (`ArchiveMount::stage_member`).
pub trait ArchiveContent: Send + Sync + std::fmt::Debug {
    fn length(&self) -> u64;
    /// At most `length` bytes from `offset`; fewer only at the end of the content.
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, FsError>;
}

/// Persistent provider address. Raw member components belong to the provider;
/// they are never joined onto the native container path. Session IDs are absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveAddress {
    pub provider: String,
    /// Exact decoder for new addresses; legacy addresses infer only built-in suffixes.
    pub format: Option<crate::ArchiveFormat>,
    pub container: std::path::PathBuf,
    pub source_version: Vec<u8>,
    pub components: Vec<Vec<u8>>,
}
impl ArchiveAddress {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.provider.is_empty()
            || self.provider.len() > 128
            || !self
                .provider
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
        {
            return Err("Invalid archive provider identity.");
        }
        if !self.container.is_absolute()
            || self.container.as_os_str().as_encoded_bytes().contains(&0)
        {
            return Err("Invalid native archive container path.");
        }
        if self.source_version.is_empty() || self.source_version.len() > 256 {
            return Err("Invalid archive source revision.");
        }
        if self.components.len() > 256
            || self.components.iter().map(Vec::len).sum::<usize>() > 64 * 1024
            || self.components.iter().any(|part| {
                part.is_empty()
                    || part == b"."
                    || part == b".."
                    || part.iter().any(|b| b"/\\:\0".contains(b))
            })
        {
            return Err("Invalid archive member locator.");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VirtualEntryId {
    pub mount: u64,
    pub entry: u64,
}

/// Archive timestamps retain whether the format supplied a timezone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveTimestamp {
    Utc {
        seconds: i64,
        nanoseconds: u32,
    },
    Local {
        year: i32,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
        second: u8,
    },
}
impl ArchiveTimestamp {
    pub fn is_valid(self) -> bool {
        match self {
            Self::Utc { nanoseconds, .. } => nanoseconds < 1_000_000_000,
            Self::Local {
                year,
                month,
                day,
                hour,
                minute,
                second,
            } => {
                let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
                let days = match month {
                    1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
                    4 | 6 | 9 | 11 => 30,
                    2 if leap => 29,
                    2 => 28,
                    _ => 0,
                };
                (1..=9999).contains(&year)
                    && day > 0
                    && day <= days
                    && hour < 24
                    && minute < 60
                    && second < 60
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct ArchiveNode {
    pub id: VirtualEntryId,
    pub parent: u64,
    pub raw_name: Arc<[u8]>,
    pub display_name: String,
    pub kind: EntryKind,
    pub size: Option<u64>,
    pub readable: bool,
    pub modified: Option<ArchiveTimestamp>,
    /// Encryption and the reason a member cannot be read, as protocol-4 providers
    /// report them; empty for older providers.
    pub access: ArchiveAccess,
}

/// How an archive member's content is encrypted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ArchiveEncryption {
    #[default]
    None,
    /// Traditional PKWARE ("ZipCrypto") encryption.
    ZipCrypto,
    Aes128,
    Aes192,
    /// WinZip AES-256 (AE-1 or AE-2).
    Aes256,
    /// 7-Zip AES-256.
    SevenZipAes256,
    /// Another method, such as PKWARE strong encryption.
    Unknown,
}
impl ArchiveEncryption {
    /// The method's name as users know it; empty for `None`.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "",
            Self::ZipCrypto => "ZipCrypto",
            Self::Aes128 => "AES-128",
            Self::Aes192 => "AES-192",
            Self::Aes256 => "AES-256",
            Self::SevenZipAes256 => "7-Zip AES-256",
            Self::Unknown => "unknown method",
        }
    }
}

/// Why an archive member that is not a folder cannot be read.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ArchiveUnreadable {
    /// Encrypted with a method that the provider cannot decrypt.
    Encryption,
    /// Compressed with the named method, which the provider cannot decode.
    Compression(String),
    Link,
    UnsafeName,
    /// Several records share the name, and the reader cannot tell them apart.
    AmbiguousName,
    SpecialFile,
    /// Decoding it needs more than the provider's bounds (such as a dictionary).
    Limit,
}

/// A member's encryption and readability as its provider reports them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct ArchiveAccess {
    /// A file: how its content is encrypted. A folder: `None`.
    pub encryption: ArchiveEncryption,
    /// A folder: whether a member below it is encrypted.
    pub contains_encrypted: bool,
    /// Why a member that is not a folder is unreadable; `None` when it is readable,
    /// or when an older provider gives no reason.
    pub unreadable: Option<ArchiveUnreadable>,
}

impl ArchiveNode {
    /// Whether opening this member, or a folder of members, may need its archive's
    /// password: an encrypted readable file, or a folder holding an encrypted member.
    pub fn needs_password(&self) -> bool {
        match self.kind {
            EntryKind::Directory => self.access.contains_encrypted,
            _ => self.readable && self.access.encryption != ArchiveEncryption::None,
        }
    }

    /// Why this member cannot be read, as one sentence that names it: the reason its
    /// provider reports, or a general one from an older provider.
    pub fn unreadable_reason(&self) -> String {
        let name = crate::visible_name(&self.display_name);
        match (&self.access.unreadable, self.kind) {
            (_, EntryKind::Directory) => format!("{name} is a folder."),
            (Some(ArchiveUnreadable::Encryption), _) => format!(
                "{name} is encrypted with {}, which the archive plugin cannot decrypt.",
                match self.access.encryption {
                    ArchiveEncryption::None | ArchiveEncryption::Unknown => {
                        "an unsupported method (such as PKWARE strong encryption)"
                    }
                    other => other.label(),
                }
            ),
            (Some(ArchiveUnreadable::Compression(method)), _) => format!(
                "{name} is compressed with {method}, which the archive plugin cannot \
                 decompress."
            ),
            (Some(ArchiveUnreadable::Link), _) | (None, EntryKind::Symlink) => {
                format!("{name} is a link inside the archive; links are not opened.")
            }
            (Some(ArchiveUnreadable::UnsafeName), _) => format!(
                "{name} has an unsafe name (an absolute path, `..`, a backslash, a drive \
                 colon or NUL), so it is never read."
            ),
            (Some(ArchiveUnreadable::AmbiguousName), _) => format!(
                "Several records of the archive are named {name}, and they cannot be \
                 told apart."
            ),
            (Some(ArchiveUnreadable::SpecialFile), _) | (None, EntryKind::Other) => {
                format!("{name} is a device, pipe or other special record.")
            }
            (Some(ArchiveUnreadable::Limit), _) => {
                format!("Reading {name} needs more memory than the archive worker may use.")
            }
            (None, _) => format!(
                "{name} cannot be read by this archive plugin: it may be encrypted, use an \
                 unsupported compression method or have an unsafe name (a newer archive \
                 plugin names the reason)."
            ),
        }
    }
}

/// The `FsError::operation` of an archive read that needs its mount's password: the
/// provider keeps the mount, and the caller may ask for the password
/// (`ArchiveMount::unlock`) and try again.
pub const ARCHIVE_PASSWORD_OPERATION: &str = "archive password";
/// The message of that error.
pub const ARCHIVE_PASSWORD_REQUIRED: &str =
    "This archive member is password-protected, and no password was entered for its archive.";

/// The error of an archive read that needs its mount's password.
pub fn archive_password_required(path: &std::path::Path) -> FsError {
    FsError::from_io(
        ARCHIVE_PASSWORD_OPERATION,
        path,
        std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            ARCHIVE_PASSWORD_REQUIRED,
        ),
    )
}

/// Whether `error` says that an archive read needs its mount's password.
pub fn is_archive_password_required(error: &FsError) -> bool {
    error.operation == ARCHIVE_PASSWORD_OPERATION
        && error.kind == std::io::ErrorKind::PermissionDenied
}

/// A password typed for an archive. Kept in memory only: its UTF-8 bytes are zeroized
/// when it is dropped, and `Debug` never prints them.
#[derive(Clone)]
pub struct ArchivePassword(zeroize::Zeroizing<String>);
impl ArchivePassword {
    pub fn new(text: String) -> Self {
        Self(zeroize::Zeroizing::new(text))
    }
    pub fn as_bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
impl std::fmt::Debug for ArchivePassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ArchivePassword(<redacted>)")
    }
}

/// The outcome of `ArchiveMount::unlock`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ArchiveUnlock {
    /// The password opens the mount's encrypted members; the mount keeps it.
    Unlocked,
    WrongPassword,
    /// The archive has no encrypted member.
    NotEncrypted,
    /// Its encrypted members use only methods the provider cannot decrypt.
    Unsupported,
}

/// Host-owned session; only backend workers call these methods.
pub trait ArchiveMount: Send + Sync + std::fmt::Debug {
    /// A cleanup failure or unwind already observed by this composition. Implementations
    /// read cached monotonic status and may forward along their backing data-source edge;
    /// this query performs no native I/O, wait, release, capture or ownership probe.
    /// False grants no teardown capability and does not certify successful destruction.
    fn cleanup_unconfirmed(&self) -> bool {
        false
    }
    /// Consume this reference on the current backend worker. Ok certifies that every
    /// cleanup responsibility acquired by consuming it has actually completed. References
    /// still held by another tab/viewer/operation remain that owner's responsibility.
    /// Concrete implementations use atomic Arc::into_inner: shared releases return
    /// immediately; the last owner must finish native cleanup, including any independent
    /// supervisor acknowledgement, before Ok. This call must not need a new I/O slot or
    /// enqueue teardown behind the worker/domain currently waiting for it.
    ///
    /// Opaque implementations retain the incoming Arc and fail closed. Ordinary Drop
    /// return alone is not a release receipt for an arbitrary deferred native owner.
    /// Known implementations fail if cleanup_unconfirmed is already true, before the
    /// atomic shared-reference success shortcut, and never destroy that tainted owner.
    /// A host session retains the incoming reference (the filesystem host ends its
    /// worker once every reference was retained that way). A wrapper only gives up its
    /// count when shared; its last owner passes its source reference to the source's
    /// own failing release, then gives up its other references to the session as a
    /// quarantined release does, and keeps its registrations and permits retained, so
    /// the session's last reference still reaches the host.
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        std::mem::forget(self);
        Err(FsError::from_io(
            "release archive",
            std::path::Path::new(""),
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Archive provider cannot confirm worker-owned native teardown",
            ),
        ))
    }
    /// Give up this reference on the current backend worker inside a retirement whose
    /// cleanup is already unconfirmed (quarantined). The caller keeps its other owners
    /// retained and has already recorded the failure, so this returns nothing.
    ///
    /// Never blocks: it must not wait for native teardown, a supervisor, an I/O slot or
    /// the worker/domain that may be waiting for the caller. Giving up a shared
    /// reference is a decrement and destroys nothing. Only the call that consumes the
    /// last owner (atomic Arc::into_inner, so exactly one of any concurrent callers) may
    /// act on what that owner holds, and only as far as quarantine allows. A wrapper
    /// gives up its source reference the same way, then its other references to the
    /// session, and keeps its registrations and permits retained. A session owner
    /// whose cleanup_unconfirmed is true hands its host reference to the host's
    /// failing release_on_worker, so the session may end its worker and return its
    /// process permit while the tainted source and the owner's other resources stay
    /// retained. An untainted last session owner stays retained, since the quarantined
    /// caller cannot certify its native teardown.
    ///
    /// The default retains the incoming Arc, which keeps a last owner's host session
    /// and worker until process exit.
    fn release_quarantined(self: Arc<Self>) {
        std::mem::forget(self);
    }
    fn id(&self) -> u64;
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError>;
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError>;
    /// Cancel one preview read without invalidating a shared mount. Legacy providers drain
    /// an admitted read under an independent token, then suppress its canceled result.
    fn read_cooperative(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, FsError> {
        let interrupted = || {
            FsError::from_io(
                "archive preview",
                std::path::Path::new(""),
                std::io::ErrorKind::Interrupted.into(),
            )
        };
        if cancelled() {
            return Err(interrupted());
        }
        let bytes = self.read(entry, offset, length, &Cancellation::default())?;
        if cancelled() {
            return Err(interrupted());
        }
        Ok(bytes)
    }
    fn validate(&self) -> Result<(), FsError>;

    /// Whether a password was verified for this mount (`unlock`). Reads cached state
    /// only, so the UI may ask it. Wrappers forward it.
    fn unlocked(&self) -> bool {
        false
    }

    /// Whether this mount's names are encrypted and its last listing needed the
    /// password (7z): such a listing is empty until `unlock`. Cached state only.
    fn listing_locked(&self) -> bool {
        false
    }

    /// Backend only: verify `password` for this mount's encrypted members and keep it
    /// for the mount's lifetime when it opens them. An archive inside an archive is a
    /// mount of its own, with its own password. Providers without passwords refuse.
    /// Cancelling `cancel` ends the exchange; a provider may stop its worker for it.
    fn unlock(
        &self,
        _password: &ArchivePassword,
        _cancel: &Cancellation,
    ) -> Result<ArchiveUnlock, FsError> {
        Err(FsError::from_io(
            "archive password",
            std::path::Path::new(""),
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "This archive plugin cannot open password-protected members",
            ),
        ))
    }

    /// Backend-only native container locator held by this mount. This is not a member path.
    /// Revalidate the mount before using it for storage policy; absent for non-native sources.
    fn native_container_locator(&self) -> Option<&std::path::Path> {
        None
    }

    /// Backend-only persistent-cache authority for one member of this exact source revision.
    /// A supported identity includes the container, source revision, provider implementation
    /// version, format and member discriminator, and survives reopening the same archive.
    /// Session IDs and display names are insufficient. Implementations validate the expected
    /// source revision before returning an identity. Older providers grant no such authority.
    fn persistent_member_identity(
        &self,
        _entry: u64,
        _expected_source_version: &[u8],
    ) -> Result<Option<Vec<u8>>, FsError> {
        Ok(None)
    }

    /// The enclosing archive when this mount reads a member of another archive as an
    /// archive; None for an archive opened from a native file. Wrappers forward it.
    fn parent(&self) -> Option<ArchiveParent> {
        None
    }

    /// Backend only: decode one member completely, verify it and keep its content as a
    /// lease that stays readable independently of this mount's member cache and worker,
    /// for an archive inside this archive to read as its source. `cancelled` stops the
    /// staging between bounded steps without ending the shared worker; `progress`
    /// receives the member bytes staged so far. Providers without such leases refuse.
    fn stage_member(
        &self,
        _entry: u64,
        _cancelled: &dyn Fn() -> bool,
        _progress: &dyn Fn(u64),
    ) -> Result<Arc<dyn ArchiveContent>, FsError> {
        Err(FsError::from_io(
            "open archive",
            std::path::Path::new(""),
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "This archive provider cannot open archives inside archives",
            ),
        ))
    }
}

#[derive(Clone, Debug)]
pub struct ArchiveLocation {
    /// Absent for a saved tab awaiting backend restoration or an unavailable plugin.
    pub mount: Option<Arc<dyn ArchiveMount>>,
    pub address: ArchiveAddress,
    /// Root is 1; member and parent IDs are opaque within this mount.
    pub directory: u64,
    pub trail: Vec<(u64, String)>,
}
impl ArchiveLocation {
    /// Presentation only: provider members retain their opaque identities and
    /// must never be passed to native filesystem APIs using this string. Inside an
    /// archive that lies in another archive it names the whole path, for example
    /// `/data/outer.zip/folder/inner.zip/sub`.
    pub fn display_text(&self) -> String {
        let enclosing = self.enclosing();
        let outermost = enclosing.last().map_or(self, |parent| &parent.location);
        let native: std::path::PathBuf = outermost.address.container.components().collect();
        let mut text = native.to_string_lossy().into_owned();
        // From the outermost archive inwards: its folders, then the member opened
        // as the next archive, and last this location's own folders.
        for parent in enclosing.iter().rev() {
            for (_, name) in parent.location.trail.iter().skip(1) {
                push_display_component(&mut text, name);
            }
            push_display_component(&mut text, &parent.name);
        }
        for (_, name) in self.trail.iter().skip(1) {
            push_display_component(&mut text, name);
        }
        text
    }

    /// The enclosing archive of this location's mount (`ArchiveMount::parent`).
    pub fn parent(&self) -> Option<ArchiveParent> {
        self.mount.as_ref()?.parent()
    }

    /// Every enclosing archive, innermost first. A chain is never longer than the
    /// deepest path that can be opened, so a malformed chain cannot loop.
    pub fn enclosing(&self) -> Vec<ArchiveParent> {
        let mut chain: Vec<ArchiveParent> = Vec::new();
        let mut next = self.parent();
        while let Some(parent) = next {
            if chain.len() >= MAX_ARCHIVE_DEPTH {
                break;
            }
            next = parent.location.parent();
            chain.push(parent);
        }
        chain
    }

    /// Archive levels of this location: 1 in an archive opened from a native file.
    pub fn depth(&self) -> usize {
        self.enclosing().len() + 1
    }

    /// The folder of the archive opened from a native file that leads to this location:
    /// the location itself unless it lies in an archive inside an archive. An archive
    /// inside an archive is opened for the session only, so saved tabs and Hot Paths
    /// keep this location instead.
    pub fn outermost(&self) -> ArchiveLocation {
        self.enclosing()
            .pop()
            .map_or_else(|| self.clone(), |parent| parent.location)
    }

    pub fn unavailable(address: ArchiveAddress) -> Self {
        let mut trail = vec![(
            0,
            address
                .container
                .file_name()
                .unwrap_or(address.container.as_os_str())
                .to_string_lossy()
                .into_owned(),
        )];
        trail.extend(
            address
                .components
                .iter()
                .map(|name| (0, String::from_utf8_lossy(name).into_owned())),
        );
        Self {
            mount: None,
            address,
            directory: 0,
            trail,
        }
    }
    pub fn mount_id(&self) -> Option<u64> {
        self.mount.as_ref().map(|m| m.id())
    }
}

fn push_display_component(text: &mut String, name: &str) {
    if !text.ends_with(std::path::MAIN_SEPARATOR) {
        text.push(std::path::MAIN_SEPARATOR);
    }
    text.push_str(name);
}

#[derive(Clone, Debug)]
pub enum ArchiveAction {
    Open {
        path: std::path::PathBuf,
        format: crate::ArchiveFormat,
    },
    List(ArchiveLocation),
    /// Open a member of a mounted archive as an archive inside it; `parent` names the
    /// mounted folder that holds the member.
    OpenMember {
        parent: ArchiveParent,
        format: crate::ArchiveFormat,
    },
}

#[derive(Clone, Debug)]
pub struct ArchiveListing {
    pub location: ArchiveLocation,
    pub entries: Arc<[crate::Entry]>,
    pub index: Arc<crate::DirectoryIndex>,
}

#[derive(Clone, Debug)]
pub enum ArchiveSelectionAction {
    Toggle(crate::EntryId),
    Remove(Arc<std::collections::HashSet<crate::EntryId>>),
    Bulk(crate::SelectionOperation),
    Range {
        first: usize,
        last: usize,
        additive: bool,
    },
    Ranges {
        ranges: crate::SelectionRanges,
        mode: crate::SelectionMode,
    },
    /// Select or deselect displayed members whose names match the masks.
    Mask(Arc<crate::MaskSelection>),
}

#[derive(Clone, Debug, Default)]
pub struct ArchiveSelectionResult {
    pub selected: Arc<std::collections::HashSet<crate::EntryId>>,
    pub totals: ArchiveSelectionTotals,
}

/// Cached totals for a provider selection. Bulk updates are prepared on workers;
/// a single cached row toggle updates these counters in constant time.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArchiveSelectionTotals {
    files: usize,
    folders: usize,
    known_bytes: u128,
    unknown_files: usize,
}
impl ArchiveSelectionTotals {
    pub fn files(self) -> usize {
        self.files
    }
    pub fn folders(self) -> usize {
        self.folders
    }
    pub fn file_bytes(self) -> Option<u64> {
        if self.unknown_files == 0 {
            u64::try_from(self.known_bytes).ok()
        } else {
            None
        }
    }
    pub fn change(&mut self, kind: EntryKind, size: Option<u64>, add: bool) {
        let adjust = |value: &mut usize| {
            *value = if add {
                value.saturating_add(1)
            } else {
                value.saturating_sub(1)
            };
        };
        if matches!(kind, EntryKind::Directory | EntryKind::DirectoryLink) {
            adjust(&mut self.folders);
        } else {
            adjust(&mut self.files);
            if kind == EntryKind::File
                && let Some(size) = size
            {
                self.known_bytes = if add {
                    self.known_bytes.saturating_add(u128::from(size))
                } else {
                    self.known_bytes.saturating_sub(u128::from(size))
                };
            } else {
                adjust(&mut self.unknown_files);
            }
        }
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    #[test]
    fn cached_totals_keep_unknown_and_overflow_distinct_and_recover_after_removal() {
        let mut totals = ArchiveSelectionTotals::default();
        totals.change(EntryKind::File, Some(u64::MAX), true);
        totals.change(EntryKind::File, Some(1), true);
        totals.change(EntryKind::Directory, None, true);
        assert_eq!(
            (totals.files(), totals.folders(), totals.file_bytes()),
            (2, 1, None)
        );
        totals.change(EntryKind::File, Some(1), false);
        assert_eq!(totals.file_bytes(), Some(u64::MAX));
        totals.change(EntryKind::Symlink, Some(1), true);
        assert_eq!(totals.file_bytes(), None);
        totals.change(EntryKind::Symlink, Some(1), false);
        assert_eq!(totals.file_bytes(), Some(u64::MAX));
    }
}

#[cfg(test)]
mod release_tests {
    use super::*;
    use std::sync::{
        Barrier,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Debug)]
    struct Opaque(Arc<AtomicUsize>);
    impl ArchiveMount for Opaque {
        fn id(&self) -> u64 {
            1
        }
        fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
            panic!("release must not list")
        }
        fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
            panic!("release must not read")
        }
        fn validate(&self) -> Result<(), FsError> {
            panic!("release must not probe")
        }
    }
    impl Drop for Opaque {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn opaque_default_retains_the_incoming_owner_instead_of_certifying_plain_drop() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let mount: Arc<dyn ArchiveMount> = Arc::new(Opaque(dropped.clone()));
        assert!(!mount.cleanup_unconfirmed());
        let error = mount.release_on_worker().unwrap_err();
        assert_eq!(error.kind, std::io::ErrorKind::Unsupported);
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn opaque_default_quarantined_release_retains_even_the_last_owner() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let mount: Arc<dyn ArchiveMount> = Arc::new(Opaque(dropped.clone()));
        let other = Arc::downgrade(&mount);
        mount.release_quarantined();
        assert_eq!(dropped.load(Ordering::SeqCst), 0);
        assert!(
            other.upgrade().is_some(),
            "the default keeps the forgotten reference counted"
        );
    }

    #[derive(Debug)]
    struct Synchronous(Opaque);
    impl ArchiveMount for Synchronous {
        fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
            if let Some(owner) = Arc::into_inner(self) {
                drop(owner);
            }
            Ok(())
        }
        fn id(&self) -> u64 {
            self.0.id()
        }
        fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
            self.0.list(directory, cancel)
        }
        fn read(
            &self,
            entry: u64,
            offset: u64,
            length: u32,
            cancel: &Cancellation,
        ) -> Result<Vec<u8>, FsError> {
            self.0.read(entry, offset, length, cancel)
        }
        fn validate(&self) -> Result<(), FsError> {
            self.0.validate()
        }
    }

    #[test]
    fn audited_atomic_consumption_assigns_exactly_one_last_owner_under_concurrent_release() {
        let dropped = Arc::new(AtomicUsize::new(0));
        let first: Arc<dyn ArchiveMount> = Arc::new(Synchronous(Opaque(dropped.clone())));
        let second = first.clone();
        let start = Arc::new(Barrier::new(3));
        let workers = [first, second].map(|owner| {
            let start = start.clone();
            std::thread::spawn(move || {
                start.wait();
                owner.release_on_worker()
            })
        });
        start.wait();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }
        assert_eq!(dropped.load(Ordering::SeqCst), 1);
    }
}

#[cfg(test)]
mod nested_tests {
    use super::*;

    /// A mount the tests only name; its parent is its enclosing archive, if any.
    #[derive(Debug)]
    struct Named(u64, Option<ArchiveParent>);
    impl ArchiveMount for Named {
        fn id(&self) -> u64 {
            self.0
        }
        fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
            unreachable!()
        }
        fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
            unreachable!()
        }
        fn validate(&self) -> Result<(), FsError> {
            Ok(())
        }
        fn parent(&self) -> Option<ArchiveParent> {
            self.1.clone()
        }
    }

    fn location(mount: Named, trail: &[&str]) -> ArchiveLocation {
        ArchiveLocation {
            mount: Some(Arc::new(mount)),
            address: ArchiveAddress {
                provider: "midnight.fs.archive".into(),
                format: Some(crate::ArchiveFormat::Zip),
                container: std::env::temp_dir().join("outer.zip"),
                source_version: vec![1],
                components: trail[1..]
                    .iter()
                    .map(|name| name.as_bytes().to_vec())
                    .collect(),
            },
            directory: trail.len() as u64,
            trail: trail
                .iter()
                .enumerate()
                .map(|(index, name)| (index as u64 + 1, (*name).to_owned()))
                .collect(),
        }
    }

    /// `name` opened from a member of `location`, shown at its folder `trail`.
    fn inside(location: ArchiveLocation, id: u64, name: &str, trail: &[&str]) -> ArchiveLocation {
        let parent = ArchiveParent {
            member: VirtualEntryId {
                mount: location.mount_id().unwrap(),
                entry: 9,
            },
            location,
            name: name.into(),
        };
        let mut trail = trail.to_vec();
        trail.insert(0, name);
        self::location(Named(id, Some(parent)), &trail)
    }

    #[test]
    fn nested_locations_name_the_whole_path_and_keep_the_outermost_folder() {
        let outer = location(Named(1, None), &["outer.zip", "folder"]);
        let inner = inside(outer.clone(), 2, "inner.tar.gz", &["sub"]);
        let innermost = inside(inner.clone(), 3, "deep.zip", &[]);
        let separator = std::path::MAIN_SEPARATOR;
        let base: std::path::PathBuf = std::env::temp_dir()
            .join("outer.zip")
            .components()
            .collect();
        let base = base.to_string_lossy();
        assert_eq!(outer.display_text(), format!("{base}{separator}folder"));
        assert_eq!(
            inner.display_text(),
            format!("{base}{separator}folder{separator}inner.tar.gz{separator}sub")
        );
        assert_eq!(
            innermost.display_text(),
            format!(
                "{base}{separator}folder{separator}inner.tar.gz{separator}sub{separator}deep.zip"
            )
        );
        assert_eq!((outer.depth(), inner.depth(), innermost.depth()), (1, 2, 3));
        let saved = innermost.outermost();
        assert_eq!(saved.mount_id(), Some(1));
        assert_eq!(saved.address.components, [b"folder".to_vec()]);
        assert_eq!(outer.outermost().mount_id(), Some(1));
        assert!(outer.parent().is_none());
        assert_eq!(innermost.parent().unwrap().member.mount, 2);
    }

    #[test]
    fn a_long_parent_chain_stays_bounded() {
        let mut location = location(Named(1, None), &["outer.zip"]);
        for id in 2..20 {
            location = inside(location, id, "again.zip", &[]);
        }
        assert_eq!(location.enclosing().len(), MAX_ARCHIVE_DEPTH);
        assert_eq!(location.depth(), MAX_ARCHIVE_DEPTH + 1);
        assert!(location.display_text().ends_with("again.zip"));
    }
}
