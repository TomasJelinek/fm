//! Read leases for live source validation, with thread-directed blocked SIGIO.
//! A lease can time out after a break request; it is not an immutable snapshot.
use super::*;
use std::{
    os::fd::AsRawFd,
    sync::{OnceLock, mpsc},
};

pub(super) struct ReadLease {
    file: Arc<File>,
}
struct Request {
    file: Arc<File>,
    response: mpsc::SyncSender<io::Result<()>>,
}

impl ReadLease {
    pub(super) fn acquire(file: Arc<File>, path: &Path) -> Result<Arc<Self>, FsError> {
        let (response, received) = mpsc::sync_channel(1);
        owner()
            .map_err(|message| unsupported(path, message))?
            .send(Request {
                file: file.clone(),
                response,
            })
            .map_err(|_| unsupported(path, "Move lease worker is unavailable"))?;
        received
            .recv()
            .map_err(|_| unsupported(path, "Move lease worker ended during admission"))?
            .map_err(|failure| {
                unsupported(
                    path,
                    &format!("Cannot retain a source read lease (an existing writer or writable mapping may be active): {failure}. Source removal refused."),
                )
            })?;
        let lease = Arc::new(Self { file });
        lease.revalidate(path)?;
        Ok(lease)
    }

    pub(super) fn revalidate(&self, path: &Path) -> Result<(), FsError> {
        // SAFETY: F_GETLEASE takes only a live descriptor, no user pointers. A
        // pending break already reports its target type, not the old read lease.
        let state = unsafe { libc::fcntl(self.file.as_raw_fd(), libc::F_GETLEASE) };
        if state != libc::F_RDLCK {
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
        // SAFETY: release only this owned open-description lease. Drop never
        // changes source names or deletes data; final file close also ends a lease.
        unsafe { libc::fcntl(self.file.as_raw_fd(), libc::F_SETLEASE, libc::F_UNLCK) };
    }
}

fn owner() -> Result<&'static mpsc::SyncSender<Request>, &'static str> {
    static OWNER: OnceLock<Result<mpsc::SyncSender<Request>, String>> = OnceLock::new();
    OWNER
        .get_or_init(|| {
            let (send, receive) = mpsc::sync_channel::<Request>(64);
            std::thread::Builder::new()
                .name("midnight-move-lease".into())
                .spawn(move || {
                    let setup = block_sigio();
                    while let Ok(request) = receive.recv() {
                        let result = match &setup {
                            Ok(tid) => grant(&request.file, *tid),
                            Err(failure) => {
                                Err(io::Error::new(failure.kind(), failure.to_string()))
                            }
                        };
                        let granted = result.is_ok();
                        if request.response.send(result).is_err() && granted {
                            // SAFETY: the receiver disappeared. Do not leave a
                            // granted lease behind on its still-held source.
                            unsafe {
                                libc::fcntl(
                                    request.file.as_raw_fd(),
                                    libc::F_SETLEASE,
                                    libc::F_UNLCK,
                                )
                            };
                        }
                    }
                })
                .map_err(|failure| failure.to_string())?;
            // One bounded process-lifetime actor, not a thread per file/job. It
            // holds no files between requests and changes no other thread's mask.
            Ok(send)
        })
        .as_ref()
        .map_err(String::as_str)
}

fn block_sigio() -> io::Result<libc::pid_t> {
    // SAFETY: the complete native signal set is initialized before use. This
    // dedicated thread never unmasks SIGIO; no process-wide disposition changes.
    unsafe {
        let mut signals = std::mem::zeroed::<libc::sigset_t>();
        if libc::sigemptyset(&mut signals) != 0 || libc::sigaddset(&mut signals, libc::SIGIO) != 0 {
            return Err(io::Error::last_os_error());
        }
        let result = libc::pthread_sigmask(libc::SIG_BLOCK, &signals, std::ptr::null_mut());
        if result != 0 {
            return Err(io::Error::from_raw_os_error(result));
        }
        Ok(libc::syscall(libc::SYS_gettid) as libc::pid_t)
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
    // preserves this explicit owner. SIGIO remains blocked on that owner thread.
    if unsafe { libc::fcntl(file.as_raw_fd(), F_SETOWN_EX, &owner as *const Owner) } != 0
        || unsafe { libc::fcntl(file.as_raw_fd(), libc::F_SETLEASE, libc::F_RDLCK) } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
