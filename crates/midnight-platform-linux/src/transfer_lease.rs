//! Read leases for live source validation, with thread-directed blocked SIGIO.
//! A lease can time out after a break request; it is not an immutable snapshot.
//!
//! One bounded worker grants leases. A second dedicated thread blocks only its own
//! SIGIO and owns every lease's thread-directed break notification; no process-wide
//! handler or disposition is installed. When another process opens a leased source
//! for writing, that thread releases the lease at once and records the break, so the
//! writer is not stalled for the kernel's lease-break timeout. The Move then refuses
//! source removal exactly as for any other observed break.
use super::*;
use std::{
    os::fd::{AsRawFd, RawFd},
    sync::{
        OnceLock, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

pub(super) struct ReadLease {
    file: Arc<File>,
    watch: Arc<Watch>,
}
struct Request {
    file: Arc<File>,
    response: mpsc::SyncSender<io::Result<Arc<File>>>,
}

/// Shared with the break watcher, which never owns the leased file. The lease
/// clears `descriptor` under this lock before its file can close, so the watcher
/// never acts on a descriptor number that could already name another file.
struct Watch {
    descriptor: Mutex<Option<RawFd>>,
    broken: AtomicBool,
}

impl ReadLease {
    pub(super) fn acquire(file: Arc<File>, path: &Path) -> Result<Arc<Self>, FsError> {
        let owner = owner().map_err(|message| unsupported(path, message))?;
        Self::acquire_with(file, path, owner)
    }

    fn acquire_with(
        file: Arc<File>,
        path: &Path,
        owner: &mpsc::SyncSender<Request>,
    ) -> Result<Arc<Self>, FsError> {
        let (response, received) = mpsc::sync_channel(1);
        owner
            .send(Request { file, response })
            .map_err(|_| unsupported(path, "Move lease worker is unavailable"))?;
        let file = received
            .recv()
            .map_err(|_| unsupported(path, "Move lease worker ended during admission"))?
            .map_err(|failure| {
                unsupported(
                    path,
                    &format!("Cannot retain a source read lease (an existing writer or writable mapping may be active): {failure}. Source removal refused."),
                )
            })?;
        let watch = Arc::new(Watch {
            descriptor: Mutex::new(Some(file.as_raw_fd())),
            broken: AtomicBool::new(false),
        });
        watched()
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .register(&watch);
        let lease = Arc::new(Self { file, watch });
        // A break that raced registration still reports its pending target type here.
        lease.revalidate(path)?;
        Ok(lease)
    }

    pub(super) fn revalidate(&self, path: &Path) -> Result<(), FsError> {
        // SAFETY: F_GETLEASE takes only a live descriptor, no user pointers. A
        // pending break already reports its target type, not the old read lease.
        let state = unsafe { libc::fcntl(self.file.as_raw_fd(), libc::F_GETLEASE) };
        if self.watch.broken.load(Ordering::Acquire) || state != libc::F_RDLCK {
            return Err(invalid(
                path,
                "The source read lease was broken or could not be checked; source removal refused",
            ));
        }
        Ok(())
    }
}

impl Drop for ReadLease {
    fn drop(&mut self) {
        // The watcher can no longer reach this descriptor once it is cleared.
        self.watch
            .descriptor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        // SAFETY: release only this owned open-description lease. Drop never
        // changes source names or deletes data; final file close also ends a lease.
        unsafe { libc::fcntl(self.file.as_raw_fd(), libc::F_SETLEASE, libc::F_UNLCK) };
    }
}

/// Live lease watches; entries die with their leases.
#[derive(Default)]
struct Watched(Vec<Weak<Watch>>);
impl Watched {
    fn register(&mut self, watch: &Arc<Watch>) {
        self.0.retain(|watch| watch.strong_count() != 0);
        self.0.push(Arc::downgrade(watch));
    }
}
fn watched() -> &'static Mutex<Watched> {
    static WATCHED: OnceLock<Mutex<Watched>> = OnceLock::new();
    WATCHED.get_or_init(Mutex::default)
}

/// Releases every lease whose break is pending. Called after each SIGIO, which
/// coalesces; a break arriving during the scan raises another one.
fn release_broken_leases() {
    let watches: Vec<_> = {
        let mut watched = watched().lock().unwrap_or_else(|e| e.into_inner());
        watched.0.retain(|watch| watch.strong_count() != 0);
        watched.0.iter().filter_map(Weak::upgrade).collect()
    };
    for watch in watches {
        let descriptor = watch.descriptor.lock().unwrap_or_else(|e| e.into_inner());
        let Some(descriptor) = *descriptor else {
            continue;
        };
        // SAFETY: the lease cannot close this descriptor while its lock is held.
        // A pending break reports its target type; releasing ends the break wait.
        unsafe {
            if libc::fcntl(descriptor, libc::F_GETLEASE) != libc::F_RDLCK {
                watch.broken.store(true, Ordering::Release);
                libc::fcntl(descriptor, libc::F_SETLEASE, libc::F_UNLCK);
            }
        }
    }
}

/// The break watcher's thread ID: the owner of every lease's notifications.
fn break_watcher() -> Result<libc::pid_t, &'static str> {
    static WATCHER: OnceLock<Result<libc::pid_t, String>> = OnceLock::new();
    WATCHER
        .get_or_init(|| {
            let (ready, started) = mpsc::sync_channel(1);
            std::thread::Builder::new()
                .name("midnight-lease-break".into())
                .spawn(move || match block_sigio() {
                    Ok(tid) => {
                        let _ = ready.send(Ok(tid));
                        while wait_for_sigio() {
                            release_broken_leases();
                        }
                    }
                    Err(failure) => {
                        let _ = ready.send(Err(failure.to_string()));
                    }
                })
                .map_err(|failure| failure.to_string())?;
            // One process-lifetime thread: it holds no files and changes no other
            // thread's signal mask.
            started
                .recv()
                .map_err(|_| "Move lease break watcher ended during startup".to_owned())?
        })
        .as_ref()
        .map(|tid| *tid)
        .map_err(String::as_str)
}

fn owner() -> Result<&'static mpsc::SyncSender<Request>, &'static str> {
    static OWNER: OnceLock<Result<mpsc::SyncSender<Request>, String>> = OnceLock::new();
    OWNER
        .get_or_init(|| {
            let (send, receive) = mpsc::sync_channel::<Request>(64);
            std::thread::Builder::new()
                .name("midnight-move-lease".into())
                .spawn(move || serve(receive, |_| {}))
                .map_err(|failure| failure.to_string())?;
            // One bounded process-lifetime actor, not a thread per file/job. It
            // holds no files between requests and changes no thread's mask.
            Ok(send)
        })
        .as_ref()
        .map_err(String::as_str)
}

fn serve(receive: mpsc::Receiver<Request>, mut after_reply: impl FnMut(bool)) {
    while let Ok(Request { file, response }) = receive.recv() {
        let result = match break_watcher() {
            Ok(tid) => grant(&file, tid),
            Err(failure) => Err(io::Error::other(failure)),
        };
        let granted = result.is_ok();
        let reply = match result {
            Ok(()) => Ok(file),
            Err(failure) => {
                // Error acknowledgement cannot leave a deferred worker owner.
                drop(file);
                Err(failure)
            }
        };
        if let Err(mpsc::SendError(Ok(file))) = response.send(reply) {
            // SAFETY: a disconnected receiver returned the actual owned file.
            // Unlock this open description before releasing our last reference.
            unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLEASE, libc::F_UNLCK) };
            drop(file);
        }
        // At this boundary the worker owns no file, even if it never runs again.
        after_reply(granted);
    }
}

fn block_sigio() -> io::Result<libc::pid_t> {
    let signals = sigio_set()?;
    // SAFETY: the initialized signal set outlives the call. This dedicated
    // thread never unmasks SIGIO; no process-wide disposition changes.
    let result = unsafe { libc::pthread_sigmask(libc::SIG_BLOCK, &signals, std::ptr::null_mut()) };
    if result != 0 {
        return Err(io::Error::from_raw_os_error(result));
    }
    // SAFETY: gettid has no arguments and cannot fail.
    Ok(unsafe { libc::syscall(libc::SYS_gettid) } as libc::pid_t)
}

fn sigio_set() -> io::Result<libc::sigset_t> {
    // SAFETY: the complete native signal set is initialized before use.
    unsafe {
        let mut signals = std::mem::zeroed::<libc::sigset_t>();
        if libc::sigemptyset(&mut signals) != 0 || libc::sigaddset(&mut signals, libc::SIGIO) != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(signals)
    }
}

/// Waits for this thread's blocked SIGIO; false ends the watcher on a native failure.
fn wait_for_sigio() -> bool {
    let Ok(signals) = sigio_set() else {
        return false;
    };
    loop {
        // SAFETY: the signal set is initialized and SIGIO is blocked on this thread,
        // so the pending notification is consumed here instead of being delivered.
        if unsafe { libc::sigwaitinfo(&signals, std::ptr::null_mut()) } == libc::SIGIO {
            return true;
        }
        if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            return false;
        }
    }
}

// Linux UAPI f_owner_ex and F_SETOWN_EX/F_OWNER_TID. libc does not expose these
// definitions on every supported GNU target; the native C layout uses int/pid_t.
#[repr(C)]
struct Owner {
    kind: libc::c_int,
    pid: libc::pid_t,
}
const F_SETOWN_EX: libc::c_int = 15;
const F_OWNER_TID: libc::c_int = 0;

fn grant(file: &File, tid: libc::pid_t) -> io::Result<()> {
    let owner = Owner {
        kind: F_OWNER_TID,
        pid: tid,
    };
    // SAFETY: the fixed native owner record and file remain live through fcntl.
    // Configure thread-only notification BEFORE granting a lease; lease setup
    // preserves this explicit owner. SIGIO remains blocked on that watcher thread.
    if unsafe { libc::fcntl(file.as_raw_fd(), F_SETOWN_EX, &owner as *const Owner) } != 0
        || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLEASE, libc::F_RDLCK) } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(test)]
#[path = "transfer_lease_handoff_tests.rs"]
mod tests;
