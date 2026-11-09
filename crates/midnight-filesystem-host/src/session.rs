use crate::Package;
use midnight_core::{ArchiveMount, ArchiveNode, Cancellation, EntryKind, FsError, VirtualEntryId};
use midnight_filesystem_api::{self as api, Format, Message};
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
struct Channel {
    input: File,
    output: File,
    request: u64,
}
pub struct Mount {
    id: u64,
    provider: String,
    container: PathBuf,
    source: CapturedFile,
    channel: Mutex<Channel>,
    supervision: Arc<Supervision>,
}
impl std::fmt::Debug for Mount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ArchiveMount")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}
impl Mount {
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
        if source.length() > 64 * 1024 * 1024 {
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
        ACTIVE
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < 2).then_some(n + 1)
            })
            .map_err(|_| {
                fail(io::Error::other(
                    "Two archive mounts are already active; close one first",
                ))
            })?;
        let worker = match midnight_worker_platform::spawn(
            &package.executable,
            midnight_worker_platform::Limits {
                memory_bytes: 512 * 1024 * 1024,
                cpu_seconds: 15,
            },
        ) {
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
        let mut process = worker.process;
        if let Err(error) = std::thread::Builder::new()
            .name("archive-supervisor".into())
            .spawn(move || {
                loop {
                    let expired = supervised
                        .deadline
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .as_ref()
                        .is_some_and(|(deadline, cancel)| {
                            Instant::now() >= *deadline || cancel.is_cancelled()
                        });
                    if supervised.stop.load(Ordering::Acquire) || expired {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                supervised.stop.store(true, Ordering::Release);
                let _ = process.kill();
                let _ = process.wait();
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
            container: path.to_owned(),
            source,
            channel: Mutex::new(Channel {
                input: worker.input,
                output: worker.output,
                request: 0,
            }),
            supervision: state,
        });
        {
            let mut channel = mount.channel.lock().unwrap_or_else(|e| e.into_inner());
            midnight_worker_sdk::read_ready(&mut channel.output).map_err(fail)?;
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
        let fail = |error: io::Error| FsError::from_io("archive", &self.container, error);
        let mut channel = self.channel.lock().unwrap_or_else(|e| e.into_inner());
        if self.supervision.stop.load(Ordering::Acquire) || cancel.is_cancelled() {
            return Err(fail(io::ErrorKind::Interrupted.into()));
        }
        self.validate()?;
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
            api::write_message(&mut channel.input, &message(request)).map_err(fail)?;
            let mut broker_bytes = 0u64;
            loop {
                let reply = api::read_message(&mut channel.output).map_err(fail)?;
                if reply.request() != request {
                    return Err(fail(io::ErrorKind::InvalidData.into()));
                }
                match reply {
                    Message::SourceRead { offset, length, .. } => {
                        broker_bytes = broker_bytes
                            .checked_add(u64::from(length))
                            .ok_or_else(|| fail(io::ErrorKind::InvalidData.into()))?;
                        if broker_bytes > self.source.length()
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
                        api::write_message(
                            &mut channel.input,
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
                        self.validate()?;
                        return Ok(other);
                    }
                }
            }
        })();
        *self
            .supervision
            .deadline
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
        if result.is_err() {
            self.supervision.stop.store(true, Ordering::Release);
        }
        result
    }
}
impl ArchiveMount for Mount {
    fn id(&self) -> u64 {
        self.id
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
                if node.id <= after
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
