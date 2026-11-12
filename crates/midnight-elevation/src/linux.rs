//! Polkit launches one read-only helper over an authenticated, private Unix socket.
use crate::protocol::{self, Reply, Request};
use midnight_core::Cancellation;
use std::{
    ffi::{OsStr, OsString},
    fs::{self, DirBuilder},
    io::{self, Read, Write},
    os::{
        fd::AsRawFd,
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::{DirBuilderExt, MetadataExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const TIMEOUT: Duration = Duration::from_secs(300);
const TICK: Duration = Duration::from_millis(10);
// A cancelled native authorization prompt retains its slot until the process really exits.
static CHILDREN: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn encode_path(path: &Path) -> Vec<u8> {
    path.as_os_str().as_bytes().to_vec()
}
pub(crate) fn decode_path(bytes: &[u8]) -> io::Result<PathBuf> {
    if bytes.contains(&0) {
        return Err(protocol::invalid("NUL in native path"));
    }
    Ok(PathBuf::from(OsString::from_vec(bytes.to_vec())))
}
pub(crate) fn metadata(value: &fs::Metadata) -> (Option<u32>, Option<u32>) {
    (None, Some(value.mode()))
}
pub(crate) fn is_redirected(_: &fs::Metadata) -> bool {
    false
}
fn denied(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
}
fn parse_number<T: std::str::FromStr>(text: &OsStr) -> io::Result<T> {
    text.to_str()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| protocol::invalid("Invalid helper process identity"))
}
fn parse_nonce(text: &OsStr) -> io::Result<([u8; 32], String)> {
    let text = text
        .to_str()
        .ok_or_else(|| protocol::invalid("Invalid channel nonce"))?;
    if text.len() != 64 || !text.bytes().all(|v| v.is_ascii_hexdigit()) {
        return Err(protocol::invalid("Invalid channel nonce"));
    }
    let mut raw = [0; 32];
    for (index, value) in raw.iter_mut().enumerate() {
        *value = u8::from_str_radix(&text[index * 2..index * 2 + 2], 16)
            .map_err(|_| protocol::invalid("Invalid channel nonce"))?;
    }
    Ok((raw, text.to_owned()))
}
fn stamp(pid: u32) -> io::Result<u64> {
    // Bound procfs input too; comm can contain spaces and closing parentheses.
    let mut bytes = Vec::new();
    fs::File::open(format!("/proc/{pid}/stat"))?
        .take(4097)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        return Err(protocol::invalid("Oversized process identity"));
    }
    parse_stamp(&bytes)
}
fn parse_stamp(bytes: &[u8]) -> io::Result<u64> {
    let close = bytes
        .iter()
        .rposition(|v| *v == b')')
        .ok_or_else(|| protocol::invalid("Invalid process identity"))?;
    // Field 3 (state) follows comm; starttime is field 22.
    let field = bytes[close + 1..]
        .split(|v| v.is_ascii_whitespace())
        .filter(|v| !v.is_empty())
        .nth(19)
        .ok_or_else(|| protocol::invalid("Missing process creation time"))?;
    std::str::from_utf8(field)
        .ok()
        .and_then(|v| v.parse().ok())
        .ok_or_else(|| protocol::invalid("Invalid process creation time"))
}
fn credentials(stream: &UnixStream) -> io::Result<libc::ucred> {
    let mut value = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: the live socket and correctly sized initialized output storage stay valid.
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut value as *mut libc::ucred).cast(),
            &mut length,
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    if length as usize != size_of::<libc::ucred>() || value.pid <= 0 {
        return Err(denied("Missing Unix peer identity"));
    }
    Ok(value)
}
fn verify_peer(stream: &UnixStream, pid: u32, uid: u32) -> io::Result<()> {
    let peer = credentials(stream)?;
    if peer.pid as u32 != pid || peer.uid != uid {
        return Err(denied("Unexpected authorization channel peer"));
    }
    Ok(())
}
fn verify_parent(stream: &UnixStream, pid: u32, uid: u32, created: u64) -> io::Result<()> {
    verify_peer(stream, pid, uid)?;
    if stamp(pid)? != created {
        return Err(denied("Requesting process changed"));
    }
    Ok(())
}
fn channel_path(nonce: &str) -> PathBuf {
    PathBuf::from(format!("/tmp/me-elevated-{nonce}/channel"))
}
struct PrivateSocket {
    path: PathBuf,
    listener: UnixListener,
}
impl PrivateSocket {
    fn new(nonce: &str) -> io::Result<Self> {
        let tmp = fs::symlink_metadata("/tmp")?;
        if !tmp.is_dir() || tmp.uid() != 0 || tmp.mode() & 0o1000 == 0 {
            return Err(denied(
                "Authorization requires a root-owned sticky /tmp directory",
            ));
        }
        let path = channel_path(nonce);
        let parent = path
            .parent()
            .ok_or_else(|| protocol::invalid("Missing channel parent"))?;
        DirBuilder::new().mode(0o700).create(parent)?;
        let listener = match UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(error) => {
                let _ = fs::remove_dir(parent);
                return Err(error);
            }
        };
        let socket = Self { path, listener };
        socket.listener.set_nonblocking(true)?;
        Ok(socket)
    }
}
impl Drop for PrivateSocket {
    fn drop(&mut self) {
        // Remove only the exact objects created here, never recursively follow a directory.
        let _ = fs::remove_file(&self.path);
        if let Some(parent) = self.path.parent() {
            let _ = fs::remove_dir(parent);
        }
    }
}
fn trusted_pkexec() -> io::Result<()> {
    for path in Path::new("/usr/bin/pkexec").ancestors() {
        let value = fs::symlink_metadata(path)?;
        if value.file_type().is_symlink() || value.uid() != 0 || value.mode() & 0o022 != 0 {
            return Err(denied("Untrusted system authorization executable"));
        }
    }
    let value = fs::metadata("/usr/bin/pkexec")?;
    if !value.is_file() || value.mode() & 0o111 == 0 {
        return Err(denied("System authorization executable is not executable"));
    }
    Ok(())
}
struct ChildSlot;
impl ChildSlot {
    fn acquire() -> io::Result<Self> {
        CHILDREN
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 2).then_some(count + 1)
            })
            .map(|_| Self)
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Two authorization requests are still active",
                )
            })
    }
}
impl Drop for ChildSlot {
    fn drop(&mut self) {
        CHILDREN.fetch_sub(1, Ordering::AcqRel);
    }
}
struct RunningChild {
    pid: u32,
    stop: Cancellation,
    status: Arc<Mutex<Option<ExitStatus>>>,
}
impl RunningChild {
    fn spawn(command: &mut Command) -> io::Result<Self> {
        let slot = ChildSlot::acquire()?;
        let stop = Cancellation::default();
        let status = Arc::new(Mutex::new(None));
        let monitor_stop = stop.clone();
        let monitor_status = status.clone();
        let (send, receive) = mpsc::sync_channel::<Child>(1);
        // Start the bounded reaper before creating a child, so thread allocation failure
        // cannot strand a process. The permit remains here until wait really succeeds.
        thread::Builder::new()
            .name("authorized-listing-child".into())
            .spawn(move || {
                let _slot = slot;
                let Ok(mut child) = receive.recv() else {
                    return;
                };
                let mut attempted_kill = false;
                loop {
                    if monitor_stop.is_cancelled() && !attempted_kill {
                        let _ = child.kill();
                        attempted_kill = true;
                    }
                    match child.try_wait() {
                        Ok(Some(value)) => {
                            *monitor_status.lock().unwrap_or_else(|e| e.into_inner()) = Some(value);
                            break;
                        }
                        Ok(None) => {}
                        Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                        // Retain the permit and retry rather than permit unbounded helper launches.
                        Err(_) => {}
                    }
                    thread::sleep(TICK);
                }
            })?;
        let child = command.spawn()?;
        let pid = child.id();
        // The receiver has no fallible work before recv; still reclaim on unexpected loss.
        if let Err(error) = send.send(child) {
            let mut child = error.0;
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::other("Authorization reaper ended"));
        }
        Ok(Self { pid, stop, status })
    }
    fn check_alive(&self) -> io::Result<()> {
        let status = *self.status.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(status) = status {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("Authorization helper exited ({status})"),
            ));
        }
        Ok(())
    }
}
impl Drop for RunningChild {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

fn check(cancel: &Cancellation, deadline: Instant) -> io::Result<()> {
    if cancel.is_cancelled() {
        return Err(io::Error::new(
            io::ErrorKind::ConnectionAborted,
            "Authorization request cancelled",
        ));
    }
    if Instant::now() >= deadline {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "Authorization helper timeout",
        ));
    }
    Ok(())
}
struct Channel {
    socket: UnixStream,
    cancel: Cancellation,
    deadline: Instant,
}
impl Channel {
    fn new(socket: UnixStream, cancel: Cancellation, deadline: Instant) -> io::Result<Self> {
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            cancel,
            deadline,
        })
    }
}
impl Read for Channel {
    fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
        loop {
            check(&self.cancel, self.deadline)?;
            match self.socket.read(data) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(TICK),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }
}
impl Write for Channel {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        loop {
            check(&self.cancel, self.deadline)?;
            match self.socket.write(data) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(TICK),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        check(&self.cancel, self.deadline)
    }
}
fn handshake(channel: &mut Channel, nonce: &[u8; 32], parent: bool) -> io::Result<()> {
    if parent {
        channel.write_all(nonce)?;
    }
    let mut proof = [0; 32];
    channel.read_exact(&mut proof)?;
    if &proof != nonce {
        return Err(denied("Authorization channel nonce mismatch"));
    }
    if !parent {
        channel.write_all(nonce)?;
    }
    Ok(())
}

pub(crate) fn list(helper: &Path, request: &Request, cancel: &Cancellation) -> io::Result<Reply> {
    check(cancel, Instant::now() + TIMEOUT)?;
    trusted_pkexec()?;
    let mut nonce = [0; 32];
    getrandom::fill(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
    let text: String = nonce.iter().map(|v| format!("{v:02x}")).collect();
    let socket = PrivateSocket::new(&text)?;
    let parent = std::process::id();
    // SAFETY: these process identity queries have no pointer arguments or side effects.
    let uid = unsafe { libc::getuid() };
    if uid != unsafe { libc::geteuid() } {
        return Err(denied("Unexpected caller credentials"));
    }
    let created = stamp(parent)?;
    let mut command = Command::new("/usr/bin/pkexec");
    command
        .arg("--disable-internal-agent")
        .arg(helper)
        .arg("--linux-channel")
        .arg(&text)
        .arg(parent.to_string())
        .arg(uid.to_string())
        .arg(created.to_string())
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let child = RunningChild::spawn(&mut command)?;
    let deadline = Instant::now() + TIMEOUT;
    let connection = loop {
        check(cancel, deadline)?;
        match socket.listener.accept() {
            Ok((stream, _)) => {
                // Reject unrelated connections without sending either nonce or directory path.
                if verify_peer(&stream, child.pid, 0).is_ok() {
                    break stream;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                child.check_alive()?;
                thread::sleep(TICK);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    };
    let mut channel = Channel::new(connection, cancel.clone(), deadline)?;
    handshake(&mut channel, &nonce, true)?;
    protocol::write_request(&mut channel, request)?;
    // Read buffered bytes even when the helper has already exited; framing detects truncation.
    protocol::read_reply(&mut channel)
}

fn disconnected(socket: &UnixStream) -> bool {
    let mut byte = 0u8;
    // SAFETY: socket is live and the one-byte output remains valid through this nonblocking peek.
    let count = unsafe {
        libc::recv(
            socket.as_raw_fd(),
            (&mut byte as *mut u8).cast(),
            1,
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    };
    if count >= 0 {
        return true;
    } // EOF or an unauthorized second request.
    !matches!(
        io::Error::last_os_error().kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
    )
}
pub(crate) fn serve(args: &[OsString]) -> io::Result<()> {
    if args.len() != 5 || args[0] != "--linux-channel" {
        return Err(protocol::invalid("Invalid helper arguments"));
    }
    let (nonce, text) = parse_nonce(&args[1])?;
    let parent: u32 = parse_number(&args[2])?;
    let uid: u32 = parse_number(&args[3])?;
    let created: u64 = parse_number(&args[4])?;
    // SAFETY: process identity calls have no pointer arguments or side effects.
    if parent == 0 || unsafe { libc::geteuid() } != 0 || unsafe { libc::getppid() } as u32 != parent
    {
        return Err(denied(
            "Authorization helper requires its original parent and root identity",
        ));
    }
    let invoking_uid = std::env::var_os("PKEXEC_UID")
        .ok_or_else(|| denied("Missing system authorization identity"))?;
    if parse_number::<u32>(&invoking_uid)? != uid {
        return Err(denied("System authorization identity changed"));
    }
    let path = channel_path(&text);
    let directory = fs::symlink_metadata(
        path.parent()
            .ok_or_else(|| protocol::invalid("Missing channel parent"))?,
    )?;
    if !directory.is_dir() || directory.uid() != uid || directory.mode() & 0o777 != 0o700 {
        return Err(denied("Authorization channel directory is not private"));
    }
    let connection = UnixStream::connect(&path)?;
    verify_parent(&connection, parent, uid, created)?;
    let cancel = Cancellation::default();
    let deadline = Instant::now() + TIMEOUT;
    let mut channel = Channel::new(connection, cancel.clone(), deadline)?;
    handshake(&mut channel, &nonce, false)?;
    let request = protocol::read_request(&mut channel)?;
    let monitor_socket = channel.socket.try_clone()?;
    let monitor_cancel = cancel.clone();
    let monitor = thread::Builder::new()
        .name("authorized-listing-peer".into())
        .spawn(move || monitor_peer(monitor_socket, parent, created, deadline, monitor_cancel))?;
    let reply = crate::helper::reply(request, &cancel);
    let result = protocol::write_reply(&mut channel, &reply);
    cancel.cancel();
    let _ = monitor.join();
    result
}

fn monitor_peer(
    socket: UnixStream,
    parent: u32,
    created: u64,
    deadline: Instant,
    cancel: Cancellation,
) {
    while !cancel.is_cancelled() {
        if Instant::now() >= deadline
            || disconnected(&socket)
            || !stamp(parent).is_ok_and(|value| value == created)
        {
            cancel.cancel();
            // This runs only inside the dedicated read-only helper process. Exiting
            // also ends a blocked native directory read; no application thread or
            // mutable transaction is terminated. The parent still waits/reaps it.
            std::process::exit(1);
        }
        thread::sleep(TICK);
    }
}

#[cfg(test)]
#[path = "linux_tests.rs"]
mod tests;
