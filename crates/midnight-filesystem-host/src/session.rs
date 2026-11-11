use crate::Package;
use midnight_core::{ArchiveMount, ArchiveNode, Cancellation, EntryKind, FsError, VirtualEntryId};
use midnight_filesystem_api::{self as api, Format, Message, Protocol};
use midnight_plugin_host::CapturedFile;
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

static NEXT_ID: AtomicU64 = AtomicU64::new(1);
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
const DEADLINE: Duration = Duration::from_secs(20);

struct Supervision {
    stop: AtomicBool,
    deadline: Mutex<Option<(Instant, Cancellation)>>,
}
impl Supervision {
    fn expired_at(&self, now: Instant) -> bool {
        self.deadline
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_some_and(|(deadline, cancel)| now >= *deadline || cancel.is_cancelled())
    }

    fn progress_at(&self, now: Instant, cancel: &Cancellation) {
        *self.deadline.lock().unwrap_or_else(|e| e.into_inner()) =
            Some((now + DEADLINE, cancel.clone()));
    }
}
struct Channel {
    input: File,
    output: File,
    request: u64,
}
pub struct Mount {
    id: u64,
    provider: String,
    provider_version: String,
    stable_entry_ids: bool,
    format: Format,
    container: PathBuf,
    source: CapturedFile,
    channel: Mutex<Channel>,
    supervision: Arc<Supervision>,
    protocol: Protocol,
    staging: Arc<Mutex<Option<crate::staging::Staging>>>,
}
impl std::fmt::Debug for Mount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveMount")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
impl Mount {
    #[cfg(test)]
    pub(crate) fn staging_present(&self) -> bool {
        self.staging
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_some()
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
        let fail = |error: io::Error| FsError::from_io("open archive", locator, error);
        if cancel.is_cancelled() {
            return Err(fail(io::ErrorKind::Interrupted.into()));
        }
        if !locator.is_absolute() || locator.as_os_str().as_encoded_bytes().contains(&0) {
            return Err(fail(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Archive container locator must be an absolute native path",
            )));
        }
        if package.protocol == Protocol::V1 && source.length() > 64 * 1024 * 1024 {
            return Err(fail(io::Error::other(
                "Archive source exceeds 64 MiB mount budget",
            )));
        }
        if source.stable_version().len() > 256 {
            return Err(fail(io::Error::new(
                io::ErrorKind::InvalidData,
                "Archive source revision exceeds address budget",
            )));
        }
        source.revalidate().map_err(|e| fail(io::Error::other(e)))?;
        if cancel.is_cancelled() {
            return Err(fail(io::ErrorKind::Interrupted.into()));
        }
        Self::start(package, format, locator, source, cancel)
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
            source_version: self.source.stable_version(),
            components: Vec::new(),
        }
    }
    pub fn open(
        package: &Package,
        format: Format,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<Self>, FsError> {
        let fail = |error: io::Error| FsError::from_io("open archive", path, error);
        if cancel.is_cancelled() {
            return Err(fail(io::ErrorKind::Interrupted.into()));
        }
        let source = CapturedFile::capture(path).map_err(|e| fail(io::Error::other(e)))?;
        Self::open_captured(package, format, path, source, cancel)
    }

    fn start(
        package: &Package,
        format: Format,
        path: &Path,
        source: CapturedFile,
        cancel: &Cancellation,
    ) -> Result<Arc<Self>, FsError> {
        let fail = |error: io::Error| FsError::from_io("open archive", path, error);
        let staging = Arc::new(Mutex::new(if package.protocol.supports_staging() {
            Some(
                crate::staging::Staging::open(
                    &std::env::temp_dir().join("MidnightExplorer-archive-spool"),
                )
                .map_err(fail)?,
            )
        } else {
            None
        }));
        ACTIVE
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 2).then_some(n + 1)
            })
            .map_err(|_| {
                fail(io::Error::other(
                    "Two archive mounts are already active; close one first",
                ))
            })?;
        let spawned = if package.protocol.supports_staging() {
            midnight_worker_platform::spawn_with_policy(
                &package.executable,
                midnight_worker_platform::ResourcePolicy {
                    memory_bytes: 512 * 1024 * 1024,
                    cpu: midnight_worker_platform::CpuBudget::NoAdditionalLimit,
                },
            )
        } else {
            midnight_worker_platform::spawn(
                &package.executable,
                midnight_worker_platform::Limits {
                    memory_bytes: 512 * 1024 * 1024,
                    cpu_seconds: 15,
                },
            )
        };
        let worker = match spawned {
            Ok(value) => value,
            Err(error) => {
                ACTIVE.fetch_sub(1, Ordering::AcqRel);
                return Err(fail(error));
            }
        };
        let state = Arc::new(Supervision {
            stop: AtomicBool::new(false),
            deadline: Mutex::new(Some((Instant::now() + DEADLINE, cancel.clone()))),
        });
        let supervised = state.clone();
        let cleanup = staging.clone();
        let mut process = worker.process;
        if let Err(error) = std::thread::Builder::new()
            .name("archive-supervisor".into())
            .spawn(move || {
                loop {
                    let expired = supervised.expired_at(Instant::now());
                    if supervised.stop.load(Ordering::Acquire) || expired {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                supervised.stop.store(true, Ordering::Release);
                let _ = process.kill();
                let _ = process.wait();
                // Final content-file releases stay on the supervisor after in-flight I/O.
                cleanup.lock().unwrap_or_else(|e| e.into_inner()).take();
                ACTIVE.fetch_sub(1, Ordering::AcqRel);
            })
        {
            ACTIVE.fetch_sub(1, Ordering::AcqRel);
            return Err(fail(error));
        }
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        let mount = Arc::new(Self {
            id,
            provider: package.id.clone(),
            provider_version: package.version.clone(),
            stable_entry_ids: package.stable_entry_ids,
            format,
            container: path.to_owned(),
            source,
            channel: Mutex::new(Channel {
                input: worker.input,
                output: worker.output,
                request: 0,
            }),
            supervision: state,
            protocol: package.protocol,
            staging,
        });
        {
            let mut channel = mount.channel.lock().unwrap_or_else(|e| e.into_inner());
            if package.protocol.supports_staging() {
                midnight_worker_sdk::read_policy_ready(&mut channel.output).map_err(fail)?;
            } else {
                midnight_worker_sdk::read_ready(&mut channel.output).map_err(fail)?;
            }
        }
        match mount.exchange(cancel, |request| Message::Open {
            request,
            format,
            length: mount.source.length(),
        })? {
            Message::Opened { root: 1, .. } => Ok(mount),
            _ => Err(fail(io::Error::other("Invalid archive open response"))),
        }
    }

    fn exchange(
        &self,
        cancel: &Cancellation,
        message: impl FnOnce(u64) -> Message,
    ) -> Result<Message, FsError> {
        self.exchange_read(cancel, None, message)
    }
    fn exchange_read(
        &self,
        cancel: &Cancellation,
        cooperative: Option<&dyn Fn() -> bool>,
        message: impl FnOnce(u64) -> Message,
    ) -> Result<Message, FsError> {
        let fail = |error: io::Error| FsError::from_io("archive", &self.container, error);
        let interrupted = || fail(io::ErrorKind::Interrupted.into());
        if cooperative.is_some_and(|cancelled| cancelled()) {
            return Err(interrupted());
        }
        let mut channel = self.channel.lock().unwrap_or_else(|e| e.into_inner());
        if cooperative.is_some_and(|cancelled| cancelled()) {
            return Err(interrupted());
        }
        let mut canceled_safely = false;
        let mut cancel_sent = false;
        self.ensure_active(cancel).map_err(fail)?;
        self.validate()
            .inspect_err(|_| self.supervision.stop.store(true, Ordering::Release))?;
        *self
            .supervision
            .deadline
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some((Instant::now() + DEADLINE, cancel.clone()));
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
                    let bytes = lease.read(offset, length).map_err(fail)?;
                    self.validate()?;
                    self.ensure_active(cancel).map_err(fail)?;
                    if cooperative.is_some_and(|cancelled| cancelled()) {
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
                    .map_err(fail)?;
                if reply.request() != request {
                    return Err(fail(io::ErrorKind::InvalidData.into()));
                }
                if self.supervision.stop.load(Ordering::Acquire) || cancel.is_cancelled() {
                    return Err(fail(io::ErrorKind::Interrupted.into()));
                }
                if cancel_sent {
                    if !matches!(&reply, Message::Failed { code: api::ErrorCode::Cancelled, .. }) {
                        return Err(fail(io::ErrorKind::InvalidData.into()));
                    }
                    self.validate()?;
                    self.ensure_active(cancel).map_err(fail)?;
                    self.staging.lock().unwrap_or_else(|e| e.into_inner()).as_mut()
                        .ok_or_else(interrupted)?.abandon_member(request).map_err(fail)?;
                    canceled_safely = true;
                    return Err(interrupted());
                }
                if cooperative.is_some() {
                    let checkpoint = self.staging.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
                        .ok_or_else(interrupted)?.cancellation_checkpoint(&operation, &reply).map_err(fail)?;
                    if checkpoint && cooperative.is_some_and(|cancelled| cancelled()) {
                        self.validate()?;
                        self.ensure_active(cancel).map_err(fail)?;
                        api::write_message_with_protocol(&mut channel.input, self.protocol,
                            &Message::Failed { request, code: api::ErrorCode::Cancelled, message: "Preview read cancelled".into() }).map_err(fail)?;
                        cancel_sent = true;
                        continue;
                    }
                }
                match reply {
                    Message::SourceRead { offset, length, .. } => {
                        broker_bytes = broker_bytes
                            .checked_add(u64::from(length))
                            .ok_or_else(|| fail(io::ErrorKind::InvalidData.into()))?;
                        if (self.protocol == Protocol::V1 && broker_bytes > self.source.length())
                            || offset
                                .checked_add(u64::from(length))
                                .is_none_or(|end| end > self.source.length())
                        {
                            return Err(fail(io::Error::other("Archive source broker budget")));
                        }
                        let bytes = self
                            .source
                            .read_at(offset, length)
                            .map_err(|e| fail(io::Error::other(e)))?;
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
                    Message::Failed { code, message, .. } => {
                        return Err(fail(io::Error::new(
                            match code {
                                api::ErrorCode::Cancelled => io::ErrorKind::Interrupted,
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
                            let staging = staging
                                .as_mut()
                                .ok_or_else(|| fail(io::ErrorKind::Interrupted.into()))?;
                            if let Some(reply) = staging.nested(&operation, &other).map_err(fail)? {
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
                            self.ensure_active(cancel).map_err(fail)?;
                            let lease = if cooperative.is_some() {
                                staging.terminal_with_cache(&operation, &other, false)
                            } else { staging.terminal(&operation, &other) }.map_err(fail)?;
                            if let (Some(lease), Message::Read { offset, length, .. }) =
                                (lease, &operation)
                            {
                                let bytes = lease.read(*offset, *length).map_err(fail)?;
                                self.validate()?;
                                self.ensure_active(cancel).map_err(fail)?;
                                if cooperative.is_some_and(|cancelled| cancelled()) {
                                    staging.abandon_member(request).map_err(fail)?;
                                    canceled_safely = true;
                                    return Err(interrupted());
                                }
                                if cooperative.is_some() && let Message::Read { entry, .. } = operation {
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
                        self.validate()?;
                        return Ok(other);
                    }
                }
            }
        })()
        .and_then(|reply| {
            // Source validation and native spool I/O may outlive the watchdog. The
            // completed result is publishable only while this operation remains active.
            self.ensure_active(cancel).map_err(fail)?;
            Ok(reply)
        });
        *self
            .supervision
            .deadline
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        if result.is_err() && !canceled_safely {
            self.supervision.stop.store(true, Ordering::Release);
        }
        result
    }
    fn ensure_active(&self, cancel: &Cancellation) -> io::Result<()> {
        if self.supervision.stop.load(Ordering::Acquire) || cancel.is_cancelled() {
            Err(io::ErrorKind::Interrupted.into())
        } else {
            Ok(())
        }
    }
    fn progress(&self, cancel: &Cancellation) {
        // Only completed byte I/O or monotonic decoded work renews the idle watchdog.
        self.supervision.progress_at(Instant::now(), cancel);
    }
}
impl ArchiveMount for Mount {
    fn id(&self) -> u64 {
        self.id
    }
    fn native_container_locator(&self) -> Option<&Path> {
        Some(&self.container)
    }
    fn persistent_member_identity(
        &self,
        entry: u64,
        expected_source_version: &[u8],
    ) -> Result<Option<Vec<u8>>, FsError> {
        let invalid = || {
            FsError::from_io(
                "archive cache identity",
                &self.container,
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Archive member or source revision changed",
                ),
            )
        };
        if entry < 2 || expected_source_version != self.source.stable_version() {
            return Err(invalid());
        }
        self.validate()?;
        if !self.stable_entry_ids {
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
        identity.extend_from_slice(&self.source.length().to_le_bytes());
        identity.extend_from_slice(&entry.to_le_bytes());
        Ok(Some(identity))
    }
    fn validate(&self) -> Result<(), FsError> {
        self.source.revalidate().map_err(|e| {
            FsError::from_io(
                "archive source changed",
                &self.container,
                io::Error::other(e),
            )
        })
    }
    fn list(&self, directory: u64, cancel: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        let mut result = Vec::new();
        let mut after = 0;
        let mut names = 0usize;
        loop {
            let Message::Entries { entries, more, .. } =
                self.exchange(cancel, |request| Message::List {
                    request,
                    directory,
                    after,
                })?
            else {
                return Err(FsError::from_io(
                    "archive list",
                    &self.container,
                    io::ErrorKind::InvalidData.into(),
                ));
            };
            if more && entries.is_empty() {
                return Err(FsError::from_io(
                    "archive list",
                    &self.container,
                    io::ErrorKind::InvalidData.into(),
                ));
            }
            for node in entries {
                names = names
                    .saturating_add(node.name.len())
                    .saturating_add(node.display.len());
                if node.id < 2
                    || node.id <= after
                    || node.id == directory
                    || node.parent != directory
                    || result.len() >= 100_000
                    || names > 8 * 1024 * 1024
                {
                    return Err(FsError::from_io(
                        "archive list",
                        &self.container,
                        io::ErrorKind::InvalidData.into(),
                    ));
                }
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
        let interrupted = || {
            FsError::from_io(
                "archive preview",
                &self.container,
                io::ErrorKind::Interrupted.into(),
            )
        };
        if cancelled() {
            return Err(interrupted());
        }
        let independent = Cancellation::default();
        if self.protocol != Protocol::V3 {
            let bytes = self.read(entry, offset, length, &independent)?;
            return if cancelled() {
                Err(interrupted())
            } else {
                Ok(bytes)
            };
        }
        match self.exchange_read(&independent, Some(cancelled), |request| Message::Read {
            request,
            entry,
            offset,
            length,
        })? {
            Message::Content { bytes, .. } if bytes.len() <= length as usize => Ok(bytes),
            _ => Err(FsError::from_io(
                "archive read",
                &self.container,
                io::ErrorKind::InvalidData.into(),
            )),
        }
    }
    fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
        cancel: &Cancellation,
    ) -> Result<Vec<u8>, FsError> {
        match self.exchange(cancel, |request| Message::Read {
            request,
            entry,
            offset,
            length,
        })? {
            Message::Content { bytes, .. } if bytes.len() <= length as usize => Ok(bytes),
            _ => Err(FsError::from_io(
                "archive read",
                &self.container,
                io::ErrorKind::InvalidData.into(),
            )),
        }
    }
}
impl Drop for Mount {
    fn drop(&mut self) {
        // Process wait and permit release stay on the supervisor, never a UI destructor.
        self.supervision.stop.store(true, Ordering::Release);
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
