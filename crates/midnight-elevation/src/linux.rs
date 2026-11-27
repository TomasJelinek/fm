//! Polkit launches one read-only helper over an authenticated, private Unix socket. After its
//! first listing the helper serves the authorization session over the same socket.
use crate::{
    Cause, HelperDigest, helper,
    protocol::{self, Reply, Request},
    refusal,
};
use midnight_core::Cancellation;
use std::{
    ffi::{CString, OsStr, OsString},
    fs::{self, DirBuilder},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, RawFd},
        unix::{
            ffi::{OsStrExt, OsStringExt},
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
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

#[path = "linux_listing.rs"]
mod listing;
pub(crate) use listing::{Location, list_directory, open_directory};

const TIMEOUT: Duration = protocol::REQUEST_TIMEOUT;
const TICK: Duration = Duration::from_millis(10);
/// How long a cancelled session request still waits for its reply, which keeps the session's
/// channel in step; a reply that takes longer ends the session.
const CANCEL_GRACE: Duration = Duration::from_secs(2);
/// How long ending a session waits to hand the helper its close request.
const CLOSE_WAIT: Duration = Duration::from_millis(200);
/// How long a request waits for closed helpers to exit when they fill the budget: a session
/// that ended just before (its pane left the folder or retries it) frees its slot this way.
const RETIRING_WAIT: Duration = Duration::from_secs(2);
/// Bound on how long cancellation or the deadline can stay unnoticed while waiting for I/O.
const CANCEL_POLL: Duration = Duration::from_millis(50);
/// Fallback re-check of the requester's identity; a closed socket wakes the monitor at once.
const PEER_CHECK: Duration = Duration::from_millis(250);
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
/// pkexec runs the helper as root. Only root may be able to change the helper or any ancestor,
/// and ownership on a nosuid mount (removable or user-mounted storage) vouches for nothing.
/// With the `shipped` helper's digest the current user may own them too, and the helper's
/// content must have that digest. pkexec starts the helper by its path, so code running as
/// the user could still exchange it after this check; such code could equally run pkexec
/// itself, and no other account can.
pub(crate) fn protected_helper(helper: &Path, shipped: Option<&HelperDigest>) -> io::Result<()> {
    let requirement = requirement(shipped.is_some());
    // SAFETY: a process identity query without pointer arguments or side effects.
    let user = shipped.map(|_| unsafe { libc::getuid() });
    for component in helper.ancestors() {
        let value = fs::symlink_metadata(component)?;
        let reason = if value.file_type().is_symlink() {
            "is a symbolic link"
        } else if value.uid() != 0 && Some(value.uid()) != user {
            if user.is_some() {
                "is not owned by you or root"
            } else {
                "is not owned by root"
            }
        } else if value.mode() & 0o022 != 0 {
            "is writable by its group or by other users"
        } else {
            continue;
        };
        return Err(unprotected(requirement, component, reason));
    }
    let value = fs::symlink_metadata(helper)?;
    if !value.is_file() || value.mode() & 0o111 == 0 {
        return Err(unprotected(
            requirement,
            helper,
            "is not an executable regular file",
        ));
    }
    if mounted_nosuid(helper)? {
        return Err(unprotected(
            requirement,
            helper,
            "is on a filesystem mounted nosuid, such as removable or user-mounted storage",
        ));
    }
    if let Some(shipped) = shipped {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(helper)?;
        crate::verify_shipped(&mut file, helper, shipped, requirement)?;
    }
    Ok(())
}
/// Where a build without the shipped helper's digest needs the helper: a refused helper
/// location and a missing helper both explain authorization with this sentence.
const ROOT_REQUIREMENT: &str = "Retry with authorization needs Midnight Explorer installed \
    where only root can change it, such as /opt/midnight-explorer installed with sudo.";
/// Where a build that records the shipped helper's digest needs it.
const SHIPPED_REQUIREMENT: &str = "Retry with authorization needs the authorization helper \
    shipped with this version of Midnight Explorer, in a folder that only you and root can \
    change.";

/// The sentence that explains a refused or missing helper; `shipped`: the build records the
/// shipped helper's digest.
pub(crate) fn requirement(shipped: bool) -> &'static str {
    if shipped {
        SHIPPED_REQUIREMENT
    } else {
        ROOT_REQUIREMENT
    }
}

fn unprotected(requirement: &str, path: &Path, reason: &str) -> io::Error {
    refusal(
        io::ErrorKind::PermissionDenied,
        Cause::Location,
        format!("{requirement} {} {reason}.", path.display()),
    )
}
/// The system's authorization program, checked as an authorized listing checks it.
pub(crate) fn system_authorization() -> io::Result<()> {
    trusted_pkexec()
        .map_err(|error| refusal(error.kind(), Cause::System, format!("{PKEXEC}: {error}")))
}
fn mounted_nosuid(path: &Path) -> io::Result<bool> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| protocol::invalid("NUL in helper path"))?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: a terminated path and writable output storage, read only after success.
    if unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: statvfs succeeded and initialized the structure.
    Ok(unsafe { stats.assume_init() }.f_flag & libc::ST_NOSUID != 0)
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
/// The system authorization executable.
pub(crate) const PKEXEC: &str = "/usr/bin/pkexec";

/// Only a pkexec that root alone can change may run anything as root for the application:
/// it and every folder above it is no link, belongs to root and is neither group- nor
/// other-writable, and it is an executable regular file.
pub(crate) fn trusted_pkexec() -> io::Result<()> {
    for path in Path::new(PKEXEC).ancestors() {
        let value = fs::symlink_metadata(path)?;
        if value.file_type().is_symlink() || value.uid() != 0 || value.mode() & 0o022 != 0 {
            return Err(denied("Untrusted system authorization executable"));
        }
    }
    let value = fs::metadata(PKEXEC)?;
    if !value.is_file() || value.mode() & 0o111 == 0 {
        return Err(denied("System authorization executable is not executable"));
    }
    Ok(())
}
/// One of the two authorization processes the application runs at a time (a prompt, a live
/// session's helper or the profile installation), held until the process was reaped.
pub(crate) struct ChildSlot;
impl ChildSlot {
    pub(crate) fn acquire() -> io::Result<Self> {
        Self::acquire_within(Duration::ZERO)
    }
    /// Wait up to `wait` for a slot that a just-closed helper frees once it is reaped.
    fn acquire_within(wait: Duration) -> io::Result<Self> {
        let deadline = Instant::now() + wait;
        loop {
            let acquired = CHILDREN.try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < 2).then_some(count + 1)
            });
            if acquired.is_ok() {
                return Ok(Self);
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "Two authorizations are still active. Finish or cancel the system's \
                     authorization prompt, or leave a folder listed with authorization, first.",
                ));
            }
            thread::sleep(TICK);
        }
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
    /// Start `command` in `slot`, which the reaper holds until the process was reaped.
    fn spawn_in(command: &mut Command, slot: ChildSlot) -> io::Result<Self> {
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
fn slice(deadline: Instant, longest: Duration) -> Duration {
    deadline
        .saturating_duration_since(Instant::now())
        .min(longest)
}
fn poll_millis(timeout: Duration) -> libc::c_int {
    libc::c_int::try_from(timeout.as_millis()).unwrap_or(libc::c_int::MAX)
}
/// Wait at most `timeout` for readiness. Readiness, hang-up, errors and signals return early;
/// the caller retries its nonblocking operation and re-checks cancellation.
fn wait_ready(descriptor: RawFd, events: libc::c_short, timeout: Duration) -> io::Result<()> {
    let mut entry = libc::pollfd {
        fd: descriptor,
        events,
        revents: 0,
    };
    // SAFETY: one initialized entry for a descriptor that stays open during the call.
    if unsafe { libc::poll(&mut entry, 1, poll_millis(timeout)) } < 0 {
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    Ok(())
}
struct Channel {
    socket: UnixStream,
    cancel: Cancellation,
    deadline: Instant,
    /// How long operations continue after `cancel` (`begin`).
    grace: Duration,
    /// When an operation first saw `cancel`.
    cancelled: Option<Instant>,
}
impl Channel {
    fn new(socket: UnixStream, cancel: Cancellation, deadline: Instant) -> io::Result<Self> {
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            cancel,
            deadline,
            grace: Duration::ZERO,
            cancelled: None,
        })
    }
    /// Bound the following operations by `cancel` and `deadline`. Operations go on for
    /// `grace` after `cancel`, so that a reply that is nearly complete can still be read.
    fn begin(&mut self, cancel: Cancellation, deadline: Instant, grace: Duration) {
        self.cancel = cancel;
        self.deadline = deadline;
        self.grace = grace;
        self.cancelled = None;
    }
    fn check(&mut self) -> io::Result<()> {
        if self.cancel.is_cancelled() {
            let since = *self.cancelled.get_or_insert_with(Instant::now);
            if since.elapsed() < self.grace {
                return check(&Cancellation::default(), self.deadline);
            }
        }
        check(&self.cancel, self.deadline)
    }
    fn wait(&self, events: libc::c_short) -> io::Result<()> {
        wait_ready(
            self.socket.as_raw_fd(),
            events,
            slice(self.deadline, CANCEL_POLL),
        )
    }
}
impl Read for Channel {
    fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
        loop {
            self.check()?;
            match self.socket.read(data) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => self.wait(libc::POLLIN)?,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }
}
impl Write for Channel {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        loop {
            self.check()?;
            match self.socket.write(data) {
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => self.wait(libc::POLLOUT)?,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        self.check()
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

/// The requester's end of a helper's session. Dropping it hands the helper a close request
/// and closes the socket (the helper ends on either); the authorization slot is released once
/// the helper was reaped.
pub(crate) struct Connection {
    channel: Channel,
    /// `None` only for an in-process test helper, which takes no slot.
    _child: Option<RunningChild>,
}

impl Connection {
    /// One more request of the session. Cancellation is honoured once the reply has not
    /// arrived within a short grace, which ends the session.
    pub(crate) fn exchange(
        &mut self,
        request: &Request,
        cancel: &Cancellation,
    ) -> io::Result<Reply> {
        self.channel
            .begin(cancel.clone(), Instant::now() + TIMEOUT, CANCEL_GRACE);
        protocol::write_request(&mut self.channel, request)?;
        protocol::read_reply(&mut self.channel)
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.channel.begin(
            Cancellation::default(),
            Instant::now() + CLOSE_WAIT,
            Duration::ZERO,
        );
        let _ = protocol::write_request(&mut self.channel, &Request::Close);
    }
}

/// Ask for consent, start the helper and send it the session's first request.
pub(crate) fn start(
    helper: &Path,
    request: &Request,
    cancel: &Cancellation,
    _: Option<&HelperDigest>,
) -> io::Result<(Connection, Reply)> {
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
    let mut command = Command::new(PKEXEC);
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
    let child = RunningChild::spawn_in(&mut command, ChildSlot::acquire_within(RETIRING_WAIT)?)?;
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
                wait_ready(
                    socket.listener.as_raw_fd(),
                    libc::POLLIN,
                    slice(deadline, CANCEL_POLL),
                )?;
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    };
    // The connected session needs no path: remove the private socket and its folder.
    drop(socket);
    let mut channel = Channel::new(connection, cancel.clone(), deadline)?;
    // The nonce is visible in the socket path and command line; SO_PEERCRED, PKEXEC_UID and
    // the /proc creation time bind the channel. Matching it rejects a stale or foreign helper.
    handshake(&mut channel, &nonce, true)?;
    protocol::write_request(&mut channel, request)?;
    // Read buffered bytes even when the helper has already exited; framing detects truncation.
    let reply = protocol::read_reply(&mut channel)?;
    Ok((
        Connection {
            channel,
            _child: Some(child),
        },
        reply,
    ))
}

/// The helper's end of the socket, with the requester's identity for its watch.
struct SocketRequester {
    channel: Channel,
    parent: u32,
    created: u64,
    terminate: fn(),
}
impl Read for SocketRequester {
    fn read(&mut self, data: &mut [u8]) -> io::Result<usize> {
        self.channel.read(data)
    }
}
impl Write for SocketRequester {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.channel.write(data)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.channel.flush()
    }
}
impl helper::Requester for SocketRequester {
    type Watch = Monitor;
    fn set_deadline(&mut self, deadline: Instant) {
        self.channel
            .begin(Cancellation::default(), deadline, Duration::ZERO);
    }
    fn watch(&self, deadline: Instant, cancel: &Cancellation) -> io::Result<Monitor> {
        Monitor::start(
            &self.channel.socket,
            self.parent,
            self.created,
            deadline,
            cancel.clone(),
            self.terminate,
        )
    }
}

fn exit_helper() {
    std::process::exit(1);
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
    let started = Instant::now();
    let deadline = started + TIMEOUT;
    let mut channel = Channel::new(connection, Cancellation::default(), deadline)?;
    handshake(&mut channel, &nonce, false)?;
    let mut requester = SocketRequester {
        channel,
        parent,
        created,
        terminate: exit_helper,
    };
    helper::serve_session(&mut requester, started, deadline, &helper::Limits::SESSION)
}

/// Ends the dedicated helper process when its requester disconnects, sends anything while a
/// request is being served, changes identity or the deadline passes, even while a native
/// directory read is blocked. Socket readiness wakes it immediately.
struct Monitor {
    stop: Option<UnixStream>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Monitor {
    fn start(
        socket: &UnixStream,
        parent: u32,
        created: u64,
        deadline: Instant,
        cancel: Cancellation,
        terminate: fn(),
    ) -> io::Result<Self> {
        let socket = socket.try_clone()?;
        let (stop, stopped) = UnixStream::pair()?;
        let thread = thread::Builder::new()
            .name("authorized-listing-peer".into())
            .spawn(move || {
                if requester_gone(&socket, &stopped, parent, created, deadline) {
                    cancel.cancel();
                    // This runs only inside the dedicated read-only helper process. Exiting
                    // also ends a blocked native directory read; no application thread or
                    // mutable transaction is terminated. The parent still waits/reaps it.
                    terminate();
                }
            })?;
        Ok(Self {
            stop: Some(stop),
            thread: Some(thread),
        })
    }
    fn finish(&mut self) {
        // Closing our end wakes the watcher's poll on the paired socket.
        drop(self.stop.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Returns `false` once stopped; `true` when the requester disconnected or sent data, its
/// process identity changed, or the deadline passed.
fn requester_gone(
    socket: &UnixStream,
    stopped: &UnixStream,
    parent: u32,
    created: u64,
    deadline: Instant,
) -> bool {
    loop {
        let mut entries = [
            libc::pollfd {
                fd: socket.as_raw_fd(),
                events: libc::POLLIN | libc::POLLRDHUP,
                revents: 0,
            },
            libc::pollfd {
                fd: stopped.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let timeout = poll_millis(slice(deadline, PEER_CHECK));
        // SAFETY: two initialized entries for descriptors that stay open during the call.
        if unsafe { libc::poll(entries.as_mut_ptr(), 2, timeout) } < 0
            && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted
        {
            return true;
        }
        if entries[1].revents != 0 {
            return false;
        }
        if entries[0].revents != 0
            || Instant::now() >= deadline
            || !stamp(parent).is_ok_and(|value| value == created)
        {
            return true;
        }
    }
}

#[cfg(test)]
#[path = "linux_tests.rs"]
mod tests;
