use crate::Package;
use midnight_core::{
    ArchiveContent, ArchiveMount, ArchiveNode, ArchiveParent, ArchiveUnlock, Cancellation,
    EntryKind, FsError, MAX_ARCHIVE_DEPTH, VirtualEntryId,
};
use midnight_filesystem_api::{self as api, Format, Message, Protocol};
use midnight_plugin_host::CapturedFile;
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Condvar, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
/// Active mounts of archives opened from native files for panels, tabs and viewers.
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
const MAX_ACTIVE: usize = 2;
/// Active mounts of archives inside archives (`nested::MAX_NESTED_ACTIVE`).
static NESTED_ACTIVE: AtomicUsize = AtomicUsize::new(0);
/// Active private sessions of searches inside archives (`MAX_SEARCH_ACTIVE`).
static SEARCH_ACTIVE: AtomicUsize = AtomicUsize::new(0);
/// Sessions of searches inside archives that may be active at once, beside the archives
/// of panels, tabs and viewers: Find runs at most two searches at once, and each search
/// opens one archive at a time and releases it before the next.
const MAX_SEARCH_ACTIVE: usize = 2;
const DEADLINE: Duration = Duration::from_secs(20);
/// The kind of every request through a mount whose worker was stopped after a failed
/// request or by its watchdog: a kind that callers do not retry (Copy offers Skip, Skip
/// all and Cancel), since only opening the archive again starts a worker. The request
/// whose failure stopped the worker keeps its own kind, such as a full spool volume's.
const STOPPED: io::ErrorKind = io::ErrorKind::InvalidData;
/// A hard request token has no wake signal; the supervisor observes it this often
/// while such an exchange is active. Idle mounts and token-free exchanges never poll.
const CANCEL_POLL: Duration = Duration::from_millis(10);
/// Process memory of a protocol-2/3 archive worker, the most the worker SDK grants.
/// Nothing limits how many members an archive has or a folder lists: the worker holds
/// the whole index of its archive, at peak about 105 bytes per TAR member, 420 per 7z
/// member and 690 per ZIP member (a million members, measured by the archive plugin's
/// `measure_index_memory`), so a worker opens about nine million TAR, two million 7z
/// or one and a half million ZIP members. Reading a 7z block adds its decoder (up to
/// 384 MiB, which the plugin budgets within this memory).
const WORKER_MEMORY: u64 = 1024 * 1024 * 1024;
/// Process memory of a legacy protocol-1 worker, which keeps its original limits.
const LEGACY_WORKER_MEMORY: u64 = 512 * 1024 * 1024;

#[path = "session_close.rs"]
mod close;

/// The idle-progress watchdog of the one active exchange. A token is hard request
/// cancellation that terminates the worker; only the startup exchange carries one.
/// Listings and member reads carry none, so only the deadline, a failed exchange or
/// mount teardown can end them.
struct Watch {
    deadline: Instant,
    cancel: Option<Cancellation>,
}
impl Watch {
    /// Time left before expiry, or None once idle progress or hard cancellation ended it.
    fn remaining(&self, now: Instant) -> Option<Duration> {
        if self.cancelled() {
            return None;
        }
        self.deadline
            .checked_duration_since(now)
            .filter(|remaining| !remaining.is_zero())
    }
    fn cancelled(&self) -> bool {
        self.cancel.as_ref().is_some_and(Cancellation::is_cancelled)
    }
}
/// Why the worker of a session was ended. The first cause is kept, and every later
/// request reports it: only a cancelled startup is a cancellation.
#[derive(Debug)]
enum Stop {
    /// A failed exchange (a protocol violation, or a worker, pipe, source or staging
    /// failure), with its failure message.
    Failed(String),
    /// The exchange made no progress before its idle deadline.
    Expired,
    /// The startup exchange was hard-cancelled by the caller that owns the mount.
    Cancelled,
    /// Mount teardown.
    Closed,
}
impl Stop {
    /// A failure message bounded for retention in the session and in later errors.
    fn failed(message: &str) -> Self {
        const MAX_BYTES: usize = 512;
        let mut end = message.len().min(MAX_BYTES);
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        Self::Failed(message[..end].to_owned())
    }
}
struct Supervision {
    stop: AtomicBool,
    cause: OnceLock<Stop>,
    watch: Mutex<Option<Watch>>,
    changed: Condvar,
    /// Protocol 4: the worker accepted a password for this session (`Mount::unlock`).
    unlocked: AtomicBool,
    /// Protocol 4: a listing needed the password (encrypted 7z names) since the last
    /// unlock.
    listing_locked: AtomicBool,
}
impl Supervision {
    fn new(watch: Option<Watch>) -> Self {
        Self {
            stop: AtomicBool::new(false),
            cause: OnceLock::new(),
            watch: Mutex::new(watch),
            changed: Condvar::new(),
            unlocked: AtomicBool::new(false),
            listing_locked: AtomicBool::new(false),
        }
    }
    fn lock(&self) -> MutexGuard<'_, Option<Watch>> {
        self.watch.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn stopped(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
    /// Mount teardown or a failed exchange: wake the supervisor to end the worker.
    fn request_stop(&self, cause: Stop) {
        // Recorded before the flag, so whoever observes the stop finds its cause.
        let _ = self.cause.set(cause);
        self.stop.store(true, Ordering::Release);
        // Notify under the lock: a supervisor between its check and wait cannot miss it.
        let _watch = self.lock();
        self.changed.notify_all();
    }
    /// The error of a request that finds the worker stopped. It names the cause: a
    /// stopped worker is a failure, not a cancellation, unless startup was cancelled.
    /// A worker stopped after a failed request or by its watchdog never serves this
    /// mount again, so that error is `STOPPED`, which callers (Copy) do not retry; it
    /// says to open the archive again.
    fn stopped_error(&self) -> io::Error {
        match self.cause.get() {
            Some(Stop::Cancelled) => io::ErrorKind::Interrupted.into(),
            Some(Stop::Expired) => io::Error::new(
                STOPPED,
                format!(
                    "Archive worker made no progress for {} seconds and was stopped; open the archive again",
                    DEADLINE.as_secs()
                ),
            ),
            Some(Stop::Failed(reason)) => io::Error::new(
                STOPPED,
                format!(
                    "Archive worker was stopped after a failed request ({reason}); open the archive again"
                ),
            ),
            Some(Stop::Closed) | None => io::Error::other("Archive session is closed"),
        }
    }
    /// Arm the watchdog for one exchange; this wakes an idle supervisor.
    fn begin(&self, now: Instant, cancel: Option<&Cancellation>) {
        self.progress_at(now, cancel);
        self.changed.notify_all();
    }
    fn finish(&self) {
        *self.lock() = None;
    }
    #[cfg(test)]
    fn expired_at(&self, now: Instant) -> bool {
        self.lock()
            .as_ref()
            .is_some_and(|watch| watch.remaining(now).is_none())
    }
    /// Renewal only extends the deadline; a waiting supervisor rechecks when it wakes.
    fn progress_at(&self, now: Instant, cancel: Option<&Cancellation>) {
        *self.lock() = Some(Watch {
            deadline: now + DEADLINE,
            cancel: cancel.cloned(),
        });
    }
    /// Supervisor only: block until teardown, deadline expiry or hard cancellation, and
    /// record the last two as the cause unless a stop request recorded one first.
    fn wait_for_stop_or_expiry(&self) {
        let mut watch = self.lock();
        loop {
            if self.stopped() {
                return;
            }
            let timeout = match watch.as_ref() {
                None => None,
                Some(active) => {
                    let Some(remaining) = active.remaining(Instant::now()) else {
                        let _ = self.cause.set(if active.cancelled() {
                            Stop::Cancelled
                        } else {
                            Stop::Expired
                        });
                        return;
                    };
                    Some(match active.cancel {
                        Some(_) => remaining.min(CANCEL_POLL),
                        None => remaining,
                    })
                }
            };
            watch = match timeout {
                None => self.changed.wait(watch).unwrap_or_else(|e| e.into_inner()),
                Some(timeout) => {
                    self.changed
                        .wait_timeout(watch, timeout)
                        .unwrap_or_else(|e| e.into_inner())
                        .0
                }
            };
        }
    }
}
struct Channel {
    input: File,
    output: File,
    request: u64,
}
/// Hooks of a member exchange that stages content for an archive inside this archive.
/// They run under the session's locks and must not block.
struct Observer<'a> {
    /// Member bytes staged so far.
    progress: &'a dyn Fn(u64),
    /// The member's verified lease, once the exchange has it.
    kept: &'a mut Option<crate::staging::Lease>,
}
pub struct Mount {
    id: u64,
    provider: String,
    provider_version: String,
    stable_entry_ids: bool,
    format: Format,
    /// The native file this mount's bytes come from; for an archive inside an archive,
    /// the file of the outermost archive.
    container: PathBuf,
    source: Option<CapturedFile>,
    channel: Mutex<Channel>,
    supervision: Arc<Supervision>,
    protocol: Protocol,
    staging: Arc<Mutex<Option<crate::staging::Staging>>>,
    close: Arc<close::CloseReceipt>,
    /// The enclosing archive of an archive opened from a member of another archive.
    nested: Option<Arc<crate::nested::Link>>,
}
impl std::fmt::Debug for Mount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveMount")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
impl Mount {
    fn source(&self) -> &CapturedFile {
        match &self.source {
            Some(source) => source,
            None => unreachable!("archive source already consumed by last-owner release"),
        }
    }
    #[cfg(test)]
    pub(crate) fn staging_present(&self) -> bool {
        self.staging
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
    }
    #[cfg(test)]
    pub(crate) fn supervisor_closed_within(&self, timeout: Duration) -> bool {
        self.close.closed_within(timeout)
    }
    /// Open a native archive locator using content captured by its filesystem provider.
    /// The locator is retained for navigation/errors, never opened by this route.
    pub fn open_captured(
        package: &Package,
        format: Format,
        locator: &Path,
        source: CapturedFile,
        cancel: &Cancellation,
    ) -> Result<Arc<Self>, FsError> {
        Self::open_source(
            package,
            format,
            locator,
            source,
            cancel,
            close::Allowance::Native,
            None,
        )
    }

    /// `open_captured` for the private session of a search inside archives (Find). It
    /// counts in the searches' own allowance (`MAX_SEARCH_ACTIVE`) instead of the two
    /// archives that panels, tabs and viewers share, so neither waits for the other.
    pub fn open_for_search(
        package: &Package,
        format: Format,
        locator: &Path,
        source: CapturedFile,
        cancel: &Cancellation,
    ) -> Result<Arc<Self>, FsError> {
        Self::open_source(
            package,
            format,
            locator,
            source,
            cancel,
            close::Allowance::Search,
            None,
        )
    }

    /// Open an archive that is a member of another archive. `content` is that member as
    /// the enclosing mount staged and verified it (`ArchiveMount::stage_member`). The new
    /// mount reads it as its captured source, revalidates the enclosing chain on every
    /// use, reports `parent` as its enclosing archive and gives it up on release. Its
    /// errors name the member's path inside the enclosing archives.
    pub fn open_nested(
        package: &Package,
        format: Format,
        parent: ArchiveParent,
        content: Arc<dyn ArchiveContent>,
        cancel: &Cancellation,
    ) -> Result<Arc<Self>, FsError> {
        let label = crate::nested::label(&parent);
        let fail = |error: io::Error| FsError::from_io("open archive", &label, error);
        let Some(enclosing) = parent.location.mount.clone() else {
            return Err(fail(io::Error::other(
                "The archive that contains this archive is closed",
            )));
        };
        if parent.location.depth() >= MAX_ARCHIVE_DEPTH {
            return Err(fail(io::Error::new(
                io::ErrorKind::Unsupported,
                midnight_core::archive_depth_message(),
            )));
        }
        let member = parent.member;
        if member.entry < 2 || member.mount != enclosing.id() {
            return Err(fail(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Archive member belongs to another archive session",
            )));
        }
        // The enclosing provider grants persistent identities only for stable member
        // IDs of its validated revision; the nested revision inherits that authority.
        let parent_version = parent.location.address.source_version.clone();
        let stable = enclosing
            .persistent_member_identity(member.entry, &parent_version)?
            .is_some();
        drop(enclosing);
        let (version, persistent) = crate::nested::revision(&parent_version, member.entry, stable);
        let locator = parent.location.address.container.clone();
        let name = parent.name.clone();
        let link = Arc::new(crate::nested::Link::new(parent, persistent));
        let source = Arc::new(crate::nested::NestedSource::new(
            link.clone(),
            content,
            version,
            name,
        ));
        let captured = match CapturedFile::from_source(source.clone()) {
            Ok(captured) => captured,
            Err(error) => {
                let error = fail(io::Error::other(error.0));
                return match midnight_plugin_host::ContentSource::release_on_worker(source) {
                    Ok(()) => Err(error),
                    Err(cleanup) => Err(fail(io::Error::other(format!(
                        "{}; archive source teardown unconfirmed: {}",
                        error.message, cleanup.0
                    )))),
                };
            }
        };
        drop(source);
        Self::open_source(
            package,
            format,
            &locator,
            captured,
            cancel,
            close::Allowance::Nested,
            Some(link),
        )
    }

    /// `allowance` is `Nested` exactly for a mount with an enclosing archive (`nested`).
    fn open_source(
        package: &Package,
        format: Format,
        locator: &Path,
        source: CapturedFile,
        cancel: &Cancellation,
        allowance: close::Allowance,
        nested: Option<Arc<crate::nested::Link>>,
    ) -> Result<Arc<Self>, FsError> {
        let label = nested
            .as_ref()
            .map_or_else(|| locator.to_owned(), |link| link.label.clone());
        let fail = |error: io::Error| FsError::from_io("open archive", &label, error);
        let mut source = close::CapturedInput::new(source);
        let prepared = (|| {
            if cancel.is_cancelled() {
                return Err(fail(io::ErrorKind::Interrupted.into()));
            }
            if !locator.is_absolute() || locator.as_os_str().as_encoded_bytes().contains(&0) {
                return Err(fail(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Archive container locator must be an absolute native path",
                )));
            }
            if package.protocol == Protocol::V1 && source.get().length() > 64 * 1024 * 1024 {
                return Err(fail(io::Error::other(
                    "Archive source exceeds 64 MiB mount budget",
                )));
            }
            if source.get().stable_version().len() > 256 {
                return Err(fail(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Archive source revision exceeds address budget",
                )));
            }
            source.revalidate().map_err(|e| fail(io::Error::other(e)))?;
            if cancel.is_cancelled() {
                return Err(fail(io::ErrorKind::Interrupted.into()));
            }
            Self::start(
                package,
                format,
                locator,
                &mut source,
                cancel,
                allowance,
                nested,
            )
        })();
        prepared.map_err(|error| source.finish_error(error))
    }

    /// The path that errors name: the native file, or for an archive inside an archive
    /// the member's path inside the enclosing archives. Presentation only.
    fn error_path(&self) -> &Path {
        self.nested
            .as_ref()
            .map_or(&self.container, |link| &link.label)
    }

    /// A failed revalidation or read of the captured source. When the capture found that
    /// its archive file was changed, the kind is `InvalidData`: repeating the request is
    /// not expected to succeed, so callers do not offer to retry (Copy). Other failures,
    /// such as an unreachable file, keep `Other`.
    fn source_failure(&self, error: midnight_plugin_host::Error) -> io::Error {
        let kind = if self.source().changed_since_capture() {
            io::ErrorKind::InvalidData
        } else {
            io::ErrorKind::Other
        };
        io::Error::new(kind, error)
    }

    pub fn address(&self) -> midnight_core::ArchiveAddress {
        midnight_core::ArchiveAddress {
            provider: self.provider.clone(),
            format: Some(match self.format {
                Format::Zip => midnight_core::ArchiveFormat::Zip,
                Format::Tar => midnight_core::ArchiveFormat::Tar,
                Format::Gzip => midnight_core::ArchiveFormat::Gzip,
                Format::TarGzip => midnight_core::ArchiveFormat::TarGzip,
                Format::SevenZip => midnight_core::ArchiveFormat::SevenZip,
                Format::Xz => midnight_core::ArchiveFormat::Xz,
                Format::TarXz => midnight_core::ArchiveFormat::TarXz,
                Format::Bzip2 => midnight_core::ArchiveFormat::Bzip2,
                Format::TarBzip2 => midnight_core::ArchiveFormat::TarBzip2,
                Format::Zstd => midnight_core::ArchiveFormat::Zstd,
                Format::TarZstd => midnight_core::ArchiveFormat::TarZstd,
            }),
            container: self.container.clone(),
            source_version: self.source().stable_version(),
            components: Vec::new(),
        }
    }
    fn start(
        package: &Package,
        format: Format,
        path: &Path,
        source: &mut close::CapturedInput,
        cancel: &Cancellation,
        allowance: close::Allowance,
        nested: Option<Arc<crate::nested::Link>>,
    ) -> Result<Arc<Self>, FsError> {
        debug_assert_eq!(nested.is_some(), allowance == close::Allowance::Nested);
        let label = nested
            .as_ref()
            .map_or_else(|| path.to_owned(), |link| link.label.clone());
        let fail = |error: io::Error| FsError::from_io("open archive", &label, error);
        let receipt = Arc::new(close::CloseReceipt::new());
        receipt.observe_cleanup(source.cleanup_unconfirmed());
        // Discovery validated the package at startup; its files may change before a
        // mount launches code. Static imports resolve from the package before the
        // worker enters containment, so every package object is rechecked here.
        crate::discovery::validate_package(&package.directory, &package.executable)
            .map_err(fail)?;
        let staging = Arc::new(Mutex::new(if package.protocol.supports_staging() {
            Some(crate::staging::Staging::open_private().map_err(fail)?)
        } else {
            None
        }));
        let accounting = close::Accounting::reserve(allowance).map_err(fail)?;
        let spawned = if package.protocol.supports_staging() {
            midnight_worker_platform::spawn_with_policy(
                &package.executable,
                midnight_worker_platform::ResourcePolicy {
                    memory_bytes: WORKER_MEMORY,
                    cpu: midnight_worker_platform::CpuBudget::NoAdditionalLimit,
                },
            )
        } else {
            midnight_worker_platform::spawn(
                &package.executable,
                midnight_worker_platform::Limits {
                    memory_bytes: LEGACY_WORKER_MEMORY,
                    cpu_seconds: 15,
                },
            )
        };
        let worker = spawned.map_err(fail)?;
        // Startup readiness is bounded and hard-cancellable: this caller owns the mount.
        let state = Arc::new(Supervision::new(Some(Watch {
            deadline: Instant::now() + DEADLINE,
            cancel: Some(cancel.clone()),
        })));
        let supervised = state.clone();
        let supervisor_receipt = receipt.clone();
        // The startup owner remains recoverable if creating the existing supervisor
        // thread fails. It is never captured as an unarmed closure-local process.
        let owner = Arc::new(Mutex::new(Some(close::Supervisor::new(
            worker.process,
            staging.clone(),
            accounting,
        ))));
        let supervisor_owner = owner.clone();
        if let Err(error) = std::thread::Builder::new()
            .name("archive-supervisor".into())
            .spawn(move || {
                let owner = supervisor_owner
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .take();
                if let Some(owner) = owner {
                    close::supervise(owner, supervised, supervisor_receipt);
                } else {
                    supervisor_receipt.failed("Archive supervisor owner missing".into());
                }
            })
        {
            state.request_stop(Stop::Closed);
            let retained = owner.lock().unwrap_or_else(|e| e.into_inner()).take();
            if let Some(retained) = retained {
                // start() already runs on a backend worker; do not need another thread
                // or release process accounting before actual fallback destruction.
                close::supervise(retained, state.clone(), receipt.clone());
            } else {
                receipt.failed("Archive startup owner could not be recovered".into());
            }
            let mut error = fail(error);
            if let Err(cleanup) = receipt.wait() {
                error.kind = io::ErrorKind::Other;
                error
                    .message
                    .push_str(&format!("; archive cleanup unconfirmed: {cleanup}"));
            }
            return Err(error);
        }
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let mount = Arc::new(Self {
            id,
            provider: package.id.clone(),
            provider_version: package.version.clone(),
            stable_entry_ids: package.stable_entry_ids,
            format,
            container: path.to_owned(),
            source: Some(source.take()),
            channel: Mutex::new(Channel {
                input: worker.input,
                output: worker.output,
                request: 0,
            }),
            supervision: state,
            protocol: package.protocol,
            staging,
            close: receipt,
            nested,
        });
        let opened = (|| {
            {
                let mut channel = mount.channel.lock().unwrap_or_else(|e| e.into_inner());
                if package.protocol.supports_staging() {
                    midnight_worker_sdk::read_policy_ready(&mut channel.output).map_err(fail)?;
                } else {
                    midnight_worker_sdk::read_ready(&mut channel.output).map_err(fail)?;
                }
            }
            match mount.exchange_read(Some(cancel), None, |request| Message::Open {
                request,
                format,
                length: mount.source().length(),
            })? {
                Message::Opened { root: 1, .. } => Ok(()),
                _ => Err(fail(io::Error::other("Invalid archive open response"))),
            }
        })();
        match opened {
            Ok(()) => Ok(mount),
            Err(error) => match mount.release_on_worker() {
                Ok(()) => Err(error),
                Err(mut cleanup) => {
                    cleanup
                        .message
                        .push_str(&format!("; original archive open failure: {error}"));
                    Err(cleanup)
                }
            },
        }
    }

    /// One serialized request. `cancel` is hard cancellation: cancelling it during the
    /// exchange terminates the worker, so only startup, whose caller still owns the
    /// whole mount, uses it. `cancelled` is a member read's own cancellation, which
    /// preserves the mount: protocol 3 delivers it cooperatively at staging
    /// checkpoints, earlier protocols finish the exchange first, and a cancelled read
    /// never returns or caches newly staged content. Every other failure ends the
    /// worker, including a terminal reply that breaks a wire rule of its request, so a
    /// misbehaving worker never keeps serving the other owners of the mount. Requests
    /// that then find the worker stopped report why (`Supervision::stopped_error`).
    fn exchange_read(
        &self,
        cancel: Option<&Cancellation>,
        cancelled: Option<&dyn Fn() -> bool>,
        message: impl FnOnce(u64) -> Message,
    ) -> Result<Message, FsError> {
        self.exchange_observed(cancel, cancelled, None, message)
    }
    /// `exchange_read` that also reports a staged member's progress and keeps its
    /// lease (`Observer`).
    fn exchange_observed(
        &self,
        cancel: Option<&Cancellation>,
        cancelled: Option<&dyn Fn() -> bool>,
        mut observer: Option<Observer<'_>>,
        message: impl FnOnce(u64) -> Message,
    ) -> Result<Message, FsError> {
        let fail = |error: io::Error| FsError::from_io("archive", self.error_path(), error);
        let interrupted = || fail(io::ErrorKind::Interrupted.into());
        let hard_cancelled = || cancel.is_some_and(Cancellation::is_cancelled);
        let is_cancelled = || cancelled.is_some_and(|check| check());
        let cooperative = cancelled.filter(|_| self.protocol.cancels_members());
        if is_cancelled() {
            return Err(interrupted());
        }
        let mut channel = self.channel.lock().unwrap_or_else(|e| e.into_inner());
        if is_cancelled() {
            return Err(interrupted());
        }
        let mut canceled_safely = false;
        let mut cancel_sent = false;
        // A member stage that the spool volume cannot take, refused through the
        // cooperative cancellation acknowledgement (`Staging::admit_member`).
        let mut refused = None;
        self.ensure_active(cancel, cancelled).map_err(fail)?;
        self.validate()
            .inspect_err(|error| self.supervision.request_stop(Stop::failed(&error.message)))?;
        self.supervision.begin(Instant::now(), cancel);
        let result = (|| {
            channel.request = channel
                .request
                .checked_add(1)
                .ok_or_else(|| fail(io::Error::other("Archive request IDs exhausted")))?;
            let request = channel.request;
            let operation = message(request);
            if let Message::Read {
                entry,
                offset,
                length,
                ..
            } = operation
                && self.protocol.supports_staging()
            {
                let cached = self
                    .staging
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .as_mut()
                    .and_then(|staging| staging.cached(entry));
                if let Some(lease) = cached {
                    if let Some(observer) = observer.as_mut() {
                        *observer.kept = Some(lease.clone());
                    }
                    let bytes = lease.read(offset, length).map_err(fail)?;
                    self.validate()?;
                    self.ensure_active(cancel, cancelled).map_err(fail)?;
                    if is_cancelled() {
                        canceled_safely = true;
                        return Err(interrupted());
                    }
                    return Ok(Message::Content {
                        request,
                        eof: bytes.len() < length as usize,
                        bytes,
                    });
                }
            }
            api::write_message_with_protocol(&mut channel.input, self.protocol, &operation)
                .map_err(fail)?;
            let mut broker_bytes = 0u64;
            loop {
                let reply = api::read_message_with_protocol(&mut channel.output, self.protocol)
                    .map_err(|error| fail(self.ended_by_itself(error)))?;
                if reply.request() != request {
                    return Err(fail(io::ErrorKind::InvalidData.into()));
                }
                self.ensure_active(cancel, cancelled).map_err(fail)?;
                // Staging is gone only once the supervisor has ended the worker.
                let ended = || fail(self.ended(cancel, cancelled));
                if cancel_sent {
                    if !matches!(&reply, Message::Failed { code: api::ErrorCode::Cancelled, .. }) {
                        return Err(fail(io::ErrorKind::InvalidData.into()));
                    }
                    self.validate()?;
                    self.ensure_active(cancel, cancelled).map_err(fail)?;
                    self.staging.lock().unwrap_or_else(|e| e.into_inner()).as_mut()
                        .ok_or_else(ended)?.abandon_member(request).map_err(fail)?;
                    canceled_safely = true;
                    // The worker gave the refused member up and serves the mount on.
                    return Err(refused.take().map_or_else(interrupted, fail));
                }
                if let Some(cooperative) = cooperative {
                    let checkpoint = self.staging.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
                        .ok_or_else(ended)?.cancellation_checkpoint(&operation, &reply).map_err(fail)?;
                    let cancelling = checkpoint && cooperative();
                    // A member stage that does not fit the spool volume is refused before
                    // it writes anything, keeping the worker for the mount's other users.
                    if checkpoint && !cancelling {
                        refused = self.staging.lock().unwrap_or_else(|e| e.into_inner()).as_mut()
                            .ok_or_else(ended)?.admit_member(&reply).err();
                    }
                    if cancelling || refused.is_some() {
                        self.validate()?;
                        self.ensure_active(cancel, cancelled).map_err(fail)?;
                        let message = if cancelling {
                            "Preview read cancelled"
                        } else {
                            "Archive spool space is insufficient"
                        };
                        api::write_message_with_protocol(&mut channel.input, self.protocol,
                            &Message::Failed { request, code: api::ErrorCode::Cancelled, message: message.into() }).map_err(fail)?;
                        cancel_sent = true;
                        continue;
                    }
                }
                match reply {
                    Message::SourceRead { offset, length, .. } => {
                        broker_bytes = broker_bytes
                            .checked_add(u64::from(length))
                            .ok_or_else(|| fail(io::ErrorKind::InvalidData.into()))?;
                        if (self.protocol == Protocol::V1 && broker_bytes > self.source().length())
                            || offset
                                .checked_add(u64::from(length))
                                .is_none_or(|end| end > self.source().length())
                        {
                            return Err(fail(io::Error::other("Archive source broker budget")));
                        }
                        let result = self.source().read_at(offset, length);
                        self.close.observe_cleanup(self.source().cleanup_unconfirmed());
                        let bytes = result.map_err(|e| fail(self.source_failure(e)))?;
                        if bytes.len() != length as usize {
                            return Err(fail(io::ErrorKind::UnexpectedEof.into()));
                        }
                        self.progress(cancel);
                        api::write_message_with_protocol(
                            &mut channel.input,
                            self.protocol,
                            &Message::SourceData { request, bytes },
                        )
                        .map_err(fail)?;
                    }
                    // Only the acknowledgement of a cancellation that this host sent
                    // (handled above) is a cancellation; an unrequested one breaks the
                    // protocol.
                    Message::Failed {
                        code: api::ErrorCode::Cancelled,
                        message,
                        ..
                    } => {
                        return Err(fail(io::Error::new(
                            io::ErrorKind::InvalidData,
                            format!("Archive worker reported an unrequested cancellation ({message})"),
                        )));
                    }
                    // Protocol 4: a member or listing that needs the mount's password (or
                    // another one). The worker staged nothing and serves on, so the session
                    // is kept; a stage it began makes the reply a protocol violation.
                    Message::Failed {
                        code: code @ (api::ErrorCode::PasswordRequired | api::ErrorCode::WrongPassword),
                        message,
                        ..
                    } => {
                        let staged_nothing = matches!(operation, Message::Read { .. } | Message::List { .. })
                            && self.staging.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
                                .is_some_and(|staging| staging.staged_nothing(request));
                        if !staged_nothing {
                            return Err(fail(io::Error::new(
                                io::ErrorKind::InvalidData,
                                "Archive worker asked for a password where the protocol allows none",
                            )));
                        }
                        self.validate()?;
                        self.ensure_active(cancel, cancelled).map_err(fail)?;
                        canceled_safely = true;
                        if matches!(operation, Message::List { .. }) {
                            self.supervision.listing_locked.store(true, Ordering::Release);
                        }
                        return Err(if code == api::ErrorCode::PasswordRequired {
                            midnight_core::archive_password_required(self.error_path())
                        } else {
                            FsError::from_io(
                                "archive read",
                                self.error_path(),
                                io::Error::new(io::ErrorKind::PermissionDenied, message),
                            )
                        });
                    }
                    Message::Failed { code, message, .. } => {
                        return Err(fail(io::Error::new(
                            match code {
                                api::ErrorCode::Unsupported | api::ErrorCode::ReadOnly => {
                                    io::ErrorKind::Unsupported
                                }
                                api::ErrorCode::NotFound => io::ErrorKind::NotFound,
                                _ => io::ErrorKind::InvalidData,
                            },
                            message,
                        )));
                    }
                    other => {
                        if self.protocol.supports_staging() {
                            let mut staging =
                                self.staging.lock().unwrap_or_else(|e| e.into_inner());
                            let staging = staging.as_mut().ok_or_else(ended)?;
                            if let Some(reply) = staging.nested(&operation, &other).map_err(fail)? {
                                if let (Some(observer), Message::StageAck { next_offset, .. }) =
                                    (observer.as_ref(), &reply)
                                {
                                    (observer.progress)(*next_offset);
                                }
                                if matches!(
                                    &reply,
                                    Message::StageAck { .. } | Message::StageProgressAck { .. }
                                ) || matches!(&reply, Message::SpoolData { bytes, .. } if !bytes.is_empty())
                                {
                                    self.progress(cancel);
                                }
                                api::write_message_with_protocol(
                                    &mut channel.input,
                                    self.protocol,
                                    &reply,
                                )
                                .map_err(fail)?;
                                continue;
                            }
                            self.validate()?;
                            self.ensure_active(cancel, cancelled).map_err(fail)?;
                            let lease = if cancelled.is_some() {
                                staging.terminal_with_cache(&operation, &other, false)
                            } else { staging.terminal(&operation, &other) }.map_err(fail)?;
                            if let (Some(lease), Message::Read { offset, length, .. }) =
                                (lease, &operation)
                            {
                                let bytes = lease.read(*offset, *length).map_err(fail)?;
                                self.validate()?;
                                self.ensure_active(cancel, cancelled).map_err(fail)?;
                                if is_cancelled() {
                                    staging.abandon_member(request).map_err(fail)?;
                                    canceled_safely = true;
                                    return Err(interrupted());
                                }
                                if let Some(observer) = observer.as_mut() {
                                    *observer.kept = Some(lease.clone());
                                }
                                if cancelled.is_some() && let Message::Read { entry, .. } = operation {
                                    staging.cache_member(entry, lease);
                                }
                                return Ok(Message::Content {
                                    request,
                                    eof: bytes.len() < *length as usize,
                                    bytes,
                                });
                            }
                        } else if !matches!(
                            (&operation, &other),
                            (Message::Open { .. }, Message::Opened { root: 1, .. })
                                | (Message::List { .. }, Message::Entries { .. })
                                | (Message::Read { .. }, Message::Content { .. })
                        ) {
                            return Err(fail(io::ErrorKind::InvalidData.into()));
                        }
                        if !keeps_wire_rules(&operation, &other) {
                            return Err(fail(io::ErrorKind::InvalidData.into()));
                        }
                        self.validate()?;
                        return Ok(other);
                    }
                }
            }
        })()
        .and_then(|reply| {
            // Source validation and native spool I/O may outlive the watchdog. The
            // completed result is publishable only while this operation remains active.
            self.ensure_active(cancel, cancelled).map_err(fail)?;
            Ok(reply)
        });
        // Idle expiry or a cancelled startup ends the worker from the supervisor, and
        // the pipe failure that this causes is reported by that cause instead.
        let result = match result {
            Err(_) if !canceled_safely && self.supervision.stopped() => {
                Err(fail(self.ended(cancel, cancelled)))
            }
            result => result,
        };
        self.supervision.finish();
        if let Err(error) = &result
            && !canceled_safely
        {
            self.supervision.request_stop(if hard_cancelled() {
                Stop::Cancelled
            } else {
                Stop::failed(&error.message)
            });
        }
        result
    }
    /// Hard cancellation or a stopped worker ends the exchange (`ended`).
    fn ensure_active(
        &self,
        cancel: Option<&Cancellation>,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> io::Result<()> {
        if cancel.is_some_and(Cancellation::is_cancelled) || self.supervision.stopped() {
            Err(self.ended(cancel, cancelled))
        } else {
            Ok(())
        }
    }
    /// Why an exchange cannot go on. A request that was cancelled itself stays a
    /// cancellation; otherwise the worker was stopped, and the stop's cause is reported.
    fn ended(
        &self,
        cancel: Option<&Cancellation>,
        cancelled: Option<&dyn Fn() -> bool>,
    ) -> io::Error {
        if cancel.is_some_and(Cancellation::is_cancelled) || cancelled.is_some_and(|check| check())
        {
            io::ErrorKind::Interrupted.into()
        } else {
            self.supervision.stopped_error()
        }
    }
    fn progress(&self, cancel: Option<&Cancellation>) {
        // Only completed byte I/O or monotonic decoded work renews the idle watchdog.
        self.supervision.progress_at(Instant::now(), cancel);
    }
    /// A reply cut off because the worker's pipe closed: the worker ended by itself (a
    /// request whose worker the supervisor stopped reports that stop's cause instead).
    /// An archive whose index outgrows the worker's memory ends it this way.
    fn ended_by_itself(&self, error: io::Error) -> io::Error {
        if error.kind() != io::ErrorKind::UnexpectedEof {
            return error;
        }
        let memory = if self.protocol.supports_staging() {
            WORKER_MEMORY
        } else {
            LEGACY_WORKER_MEMORY
        };
        io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "The archive worker ended unexpectedly, for example because the archive \
                 needs more than the {} MiB of memory that the worker may use",
                memory / (1024 * 1024)
            ),
        )
    }
    /// Protocol-3 member read with cooperative cancellation; earlier protocols drain an
    /// admitted read under no hard token and then discard its cancelled result.
    fn read_preserving_mount(
        &self,
        operation: &'static str,
        entry: u64,
        offset: u64,
        length: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, FsError> {
        let interrupted = || {
            FsError::from_io(
                operation,
                self.error_path(),
                io::ErrorKind::Interrupted.into(),
            )
        };
        if cancelled() {
            return Err(interrupted());
        }
        // The exchange never returns content longer than requested: keeps_wire_rules
        // checks protocol 1, and protocols 2/3 read through a host-bounded lease.
        match self.exchange_read(None, Some(cancelled), |request| Message::Read {
            request,
            entry,
            offset,
            length,
        })? {
            Message::Content { .. } if cancelled() => Err(interrupted()),
            Message::Content { bytes, .. } => Ok(bytes),
            _ => Err(self.wrong_reply("archive read")),
        }
    }
    /// A terminal reply of the wrong type for its request. `exchange_read` returns only
    /// Content for a Read and Entries for a List (the protocol-1 reply match, the staged
    /// `terminal` exchange and the member cache), so this is a fail-closed backstop that,
    /// like every other protocol violation, ends the worker and records why.
    fn wrong_reply(&self, operation: &'static str) -> FsError {
        let error = FsError::from_io(
            operation,
            self.error_path(),
            io::ErrorKind::InvalidData.into(),
        );
        self.supervision.request_stop(Stop::failed(&error.message));
        error
    }
    /// `ArchiveMount::stage_member`: one ordinary member read, cancelled cooperatively
    /// like any other, that keeps the member's verified lease. A member the cache already
    /// holds is not decoded again. The lease belongs to the caller from then on: the
    /// cache may evict its own reference, and the lease outlives this mount's worker.
    fn stage(
        &self,
        entry: u64,
        cancelled: &dyn Fn() -> bool,
        progress: &dyn Fn(u64),
    ) -> Result<Arc<dyn ArchiveContent>, FsError> {
        let fail = |error: io::Error| FsError::from_io("open archive", self.error_path(), error);
        if !self.protocol.supports_staging() {
            return Err(fail(io::Error::new(
                io::ErrorKind::Unsupported,
                "Opening an archive inside this archive needs a newer archive plugin",
            )));
        }
        if entry < 2 {
            return Err(fail(io::ErrorKind::InvalidInput.into()));
        }
        if cancelled() {
            return Err(fail(io::ErrorKind::Interrupted.into()));
        }
        let mut kept = None;
        let observer = Observer {
            progress,
            kept: &mut kept,
        };
        // Any one byte: the exchange stages the whole member before it answers.
        let reply = self.exchange_observed(None, Some(cancelled), Some(observer), |request| {
            Message::Read {
                request,
                entry,
                offset: 0,
                length: 1,
            }
        })?;
        match (reply, kept) {
            (Message::Content { .. }, _) if cancelled() => {
                Err(fail(io::ErrorKind::Interrupted.into()))
            }
            (Message::Content { .. }, Some(lease)) => {
                Ok(Arc::new(crate::nested::StagedMember::new(lease)))
            }
            _ => Err(self.wrong_reply("archive stage")),
        }
    }
}
/// A protocol-4 node's encryption and unreadability; older protocols report none.
fn access(
    encryption: api::Encryption,
    contains_encrypted: bool,
    unreadable: Option<api::Unreadable>,
) -> midnight_core::ArchiveAccess {
    use midnight_core::{ArchiveEncryption as E, ArchiveUnreadable as U};
    midnight_core::ArchiveAccess {
        encryption: match encryption {
            api::Encryption::None => E::None,
            api::Encryption::ZipCrypto => E::ZipCrypto,
            api::Encryption::Aes128 => E::Aes128,
            api::Encryption::Aes192 => E::Aes192,
            api::Encryption::Aes256 => E::Aes256,
            api::Encryption::SevenZipAes256 => E::SevenZipAes256,
            api::Encryption::Unknown => E::Unknown,
        },
        contains_encrypted,
        unreadable: unreadable.map(|reason| match reason {
            api::Unreadable::Encryption => U::Encryption,
            api::Unreadable::Compression(method) => U::Compression(method),
            api::Unreadable::Link => U::Link,
            api::Unreadable::UnsafeName => U::UnsafeName,
            api::Unreadable::AmbiguousName => U::AmbiguousName,
            api::Unreadable::SpecialFile => U::SpecialFile,
            api::Unreadable::Limit => U::Limit,
        }),
    }
}

/// Whether a terminal reply keeps the wire rules of its own request. The wire
/// reference (filesystem-api/PROTOCOL.md) requires member IDs from 2 in increasing
/// order, past the request's cursor, and content no longer than requested. The host
/// listing contract (docs/developer/FILESYSTEM_API.md) adds that each member belongs
/// to the listed directory and is not that directory, and that only a final page may
/// be empty. Staged protocols receive content through a lease that the host bounds
/// instead.
fn keeps_wire_rules(operation: &Message, reply: &Message) -> bool {
    match (operation, reply) {
        (
            Message::List {
                directory, after, ..
            },
            Message::Entries { entries, more, .. },
        ) => {
            (!*more || !entries.is_empty())
                && entries
                    .iter()
                    .try_fold(*after, |previous, node| {
                        (node.id >= 2
                            && node.id > previous
                            && node.id != *directory
                            && node.parent == *directory)
                            .then_some(node.id)
                    })
                    .is_some()
        }
        (Message::Read { length, .. }, Message::Content { bytes, .. }) => {
            bytes.len() <= *length as usize
        }
        _ => true,
    }
}
impl ArchiveMount for Mount {
    fn cleanup_unconfirmed(&self) -> bool {
        self.close.cleanup_unconfirmed() || self.source().cleanup_unconfirmed()
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
        close::release(self)
    }
    fn id(&self) -> u64 {
        self.id
    }
    fn native_container_locator(&self) -> Option<&Path> {
        Some(&self.container)
    }
    fn parent(&self) -> Option<ArchiveParent> {
        self.nested.as_ref()?.parent()
    }
    fn stage_member(
        &self,
        entry: u64,
        cancelled: &dyn Fn() -> bool,
        progress: &dyn Fn(u64),
    ) -> Result<Arc<dyn ArchiveContent>, FsError> {
        self.stage(entry, cancelled, progress)
    }
    fn unlocked(&self) -> bool {
        self.supervision.unlocked.load(Ordering::Acquire)
    }
    fn listing_locked(&self) -> bool {
        self.supervision.listing_locked.load(Ordering::Acquire)
    }
    /// Protocol 4 `Unlock`: one exchange, which a protocol-1–3 worker cannot take. The
    /// password crosses the pipe in one frame that the codec zeroizes; the host keeps
    /// no copy, and the worker keeps it for the session.
    fn unlock(&self, password: &midnight_core::ArchivePassword) -> Result<ArchiveUnlock, FsError> {
        let fail =
            |error: io::Error| FsError::from_io("archive password", self.error_path(), error);
        if !self.protocol.supports_passwords() {
            return Err(fail(io::Error::new(
                io::ErrorKind::Unsupported,
                "This archive plugin cannot open password-protected members; a newer one can",
            )));
        }
        if password.is_empty() || password.as_bytes().len() > api::MAX_PASSWORD {
            return Err(fail(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("An archive password has 1 to {} bytes", api::MAX_PASSWORD),
            )));
        }
        let reply = self.exchange_read(None, None, |request| Message::Unlock {
            request,
            password: api::Password::new(password.as_bytes().to_vec()),
        })?;
        let Message::UnlockReply { status, .. } = reply else {
            return Err(self.wrong_reply("archive password"));
        };
        Ok(match status {
            api::UnlockStatus::Unlocked => {
                self.supervision.unlocked.store(true, Ordering::Release);
                self.supervision
                    .listing_locked
                    .store(false, Ordering::Release);
                ArchiveUnlock::Unlocked
            }
            api::UnlockStatus::WrongPassword => ArchiveUnlock::WrongPassword,
            api::UnlockStatus::NotEncrypted => ArchiveUnlock::NotEncrypted,
            api::UnlockStatus::Unsupported => ArchiveUnlock::Unsupported,
        })
    }
    fn persistent_member_identity(
        &self,
        entry: u64,
        expected_source_version: &[u8],
    ) -> Result<Option<Vec<u8>>, FsError> {
        let invalid = || {
            FsError::from_io(
                "archive cache identity",
                self.error_path(),
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Archive member or source revision changed",
                ),
            )
        };
        if entry < 2 || expected_source_version != self.source().stable_version() {
            return Err(invalid());
        }
        self.validate()?;
        // An archive inside an archive inherits the enclosing chain's authority.
        if !self.stable_entry_ids || self.nested.as_ref().is_some_and(|link| !link.persistent) {
            return Ok(None);
        }
        let mut identity = b"MidnightArchiveMember\0v1\0".to_vec();
        for part in [
            self.provider.as_bytes(),
            self.provider_version.as_bytes(),
            self.container.as_os_str().as_encoded_bytes(),
            expected_source_version,
        ] {
            if part.len() > 128 * 1024 {
                return Err(invalid());
            }
            identity.extend_from_slice(&(part.len() as u64).to_le_bytes());
            identity.extend_from_slice(part);
        }
        identity.push(self.format as u8);
        identity.extend_from_slice(&self.source().length().to_le_bytes());
        identity.extend_from_slice(&entry.to_le_bytes());
        Ok(Some(identity))
    }
    fn validate(&self) -> Result<(), FsError> {
        let result = self.source().revalidate();
        self.close
            .observe_cleanup(self.source().cleanup_unconfirmed());
        result.map_err(|e| {
            FsError::from_io(
                "archive source changed",
                self.error_path(),
                self.source_failure(e),
            )
        })
    }
    /// Each page is one bounded exchange, and the listing pages on for as long as the
    /// worker says more members follow: a folder of any size lists completely.
    /// Cancellation ends the listing between pages and discards a conforming in-flight
    /// page, rather than terminating a worker that other tabs, viewers and jobs share.
    /// The idle-progress watchdog still ends a stuck worker, and a page that breaks a
    /// wire rule fails its exchange, which ends the worker too.
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        let interrupted = || {
            FsError::from_io(
                "archive list",
                self.error_path(),
                io::ErrorKind::Interrupted.into(),
            )
        };
        let mut result = Vec::new();
        let mut after = 0;
        loop {
            if cancel.is_cancelled() {
                return Err(interrupted());
            }
            let page = self.exchange_read(None, None, |request| Message::List {
                request,
                directory,
                after,
            })?;
            if cancel.is_cancelled() {
                return Err(interrupted());
            }
            let Message::Entries { entries, more, .. } = page else {
                return Err(self.wrong_reply("archive list"));
            };
            // exchange_read rejects an empty page that announces more, so every page
            // that continues the listing advances the cursor.
            for node in entries {
                after = node.id;
                result.push(ArchiveNode {
                    modified: node.modified.map(|timestamp| match timestamp {
                        api::Timestamp::Utc {
                            seconds,
                            nanoseconds,
                        } => midnight_core::ArchiveTimestamp::Utc {
                            seconds,
                            nanoseconds,
                        },
                        api::Timestamp::Local {
                            year,
                            month,
                            day,
                            hour,
                            minute,
                            second,
                        } => midnight_core::ArchiveTimestamp::Local {
                            year,
                            month,
                            day,
                            hour,
                            minute,
                            second,
                        },
                    }),
                    id: VirtualEntryId {
                        mount: self.id,
                        entry: node.id,
                    },
                    parent: node.parent,
                    raw_name: node.name.into(),
                    display_name: node.display,
                    kind: match node.kind {
                        api::Kind::Directory => EntryKind::Directory,
                        api::Kind::File => EntryKind::File,
                        api::Kind::Link => EntryKind::Symlink,
                        api::Kind::Other => EntryKind::Other,
                    },
                    size: node.size,
                    readable: node.readable,
                    access: access(node.encryption, node.contains_encrypted, node.unreadable),
                });
            }
            if !more {
                return Ok(result);
            }
        }
    }
    fn read_cooperative(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, FsError> {
        self.read_preserving_mount("archive preview", entry, offset, length, cancelled)
    }
    /// Cancelling one read never ends the worker that other tabs, viewers and jobs
    /// share: protocol 3 cancels the member cooperatively, and earlier protocols, which
    /// cannot interrupt a member read, finish the exchange under the idle-progress
    /// watchdog and discard its result.
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        self.read_preserving_mount("archive read", entry, offset, length, &|| {
            cancel.is_cancelled()
        })
    }
}
impl Drop for Mount {
    fn drop(&mut self) {
        // Process wait and permit release stay on the supervisor, never a UI destructor.
        self.supervision.request_stop(Stop::Closed);
    }
}

#[cfg(test)]
#[path = "watchdog_tests.rs"]
mod watchdog_tests;

#[cfg(test)]
#[path = "session_completion_tests.rs"]
mod completion_tests;

#[cfg(test)]
#[path = "member_identity_tests.rs"]
mod member_identity_tests;

#[cfg(test)]
#[path = "cooperative_tests.rs"]
mod cooperative_tests;

#[cfg(test)]
#[path = "nested_session_tests.rs"]
mod nested_session_tests;

#[cfg(test)]
#[path = "source_change_tests.rs"]
mod source_change_tests;

#[cfg(test)]
#[path = "password_tests.rs"]
mod password_tests;
