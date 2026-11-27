use super::*;
use crate::{
    helper::{Limits, Session},
    protocol::FailureKind,
};
use midnight_core::{ElevatedListingSession, SessionListingError};
use std::os::unix::fs::{PermissionsExt, symlink};

fn channel(socket: UnixStream) -> Channel {
    Channel::new(
        socket,
        Cancellation::default(),
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap()
}
/// The helper's listing of `path` as a session's first request.
fn listed(path: &Path) -> Vec<protocol::WireEntry> {
    Session::default()
        .list(&encode_path(path), &Cancellation::default())
        .result
        .unwrap()
}
fn unique_nonce() -> String {
    let mut nonce = [0; 32];
    getrandom::fill(&mut nonce).unwrap();
    nonce.iter().map(|value| format!("{value:02x}")).collect()
}

#[test]
fn native_names_and_metadata_preserve_raw_unix_identity() {
    let bytes = b"/tmp/raw-\xff-\xc4\x8d";
    assert_eq!(encode_path(&decode_path(bytes).unwrap()), bytes);
    assert!(decode_path(b"x\0y").is_err());
    let directory = tempfile::tempdir().unwrap();
    let name = OsString::from_vec(b"raw-\xff-\xc4\x8d".to_vec());
    let path = directory.path().join(&name);
    fs::write(&path, b"abc").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    let reply = listed(directory.path());
    assert_eq!(reply.len(), 1);
    assert_eq!(reply[0].name, name.as_bytes());
    assert_eq!(reply[0].size, Some(3));
    assert_eq!(reply[0].unix_mode.unwrap() & 0o777, 0o640);
    assert_eq!(reply[0].native_attributes, None);
}

#[test]
fn links_report_their_own_metadata_and_directory_targets() {
    let directory = tempfile::tempdir().unwrap();
    fs::create_dir(directory.path().join("folder")).unwrap();
    symlink("folder", directory.path().join("folder-link")).unwrap();
    symlink("missing", directory.path().join("dangling")).unwrap();
    let reply = listed(directory.path());
    let kind = |name: &str| {
        let entry = reply
            .iter()
            .find(|entry| entry.name == name.as_bytes())
            .unwrap();
        // A link's mode is the link's own (lstat), never its target's.
        if name != "folder" {
            assert_eq!(entry.unix_mode.unwrap() & 0o170000, 0o120000, "{name}");
        }
        entry.kind
    };
    assert_eq!(kind("folder"), 0);
    assert_eq!(kind("folder-link"), 1);
    assert_eq!(kind("dangling"), 3);
}

#[test]
fn helper_outside_a_root_owned_installation_is_refused_with_guidance() {
    // A temporary directory is user-owned or below a world-writable /tmp, so it never qualifies.
    let directory = tempfile::tempdir().unwrap();
    let helper = directory.path().join("midnight-elevated-listing");
    fs::write(&helper, b"#!/bin/sh\n").unwrap();
    fs::set_permissions(&helper, fs::Permissions::from_mode(0o755)).unwrap();
    let error = protected_helper(&helper, None).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    let message = error.to_string();
    assert!(message.contains("only root can change it"), "{message}");
    assert!(message.contains("/opt/midnight-explorer"), "{message}");
    // With the shipped digest the user may own the installation, but no folder above it
    // may be writable by others (the temporary directory's /tmp is).
    let shipped = [0; 32];
    let error = protected_helper(&helper, Some(&shipped)).unwrap_err();
    let message = error.to_string();
    assert!(message.starts_with(requirement(true)), "{message}");
    assert!(
        message.contains("writable by its group or by other users")
            || message.contains("is not owned by you or root")
            || message.contains("is not the authorization helper"),
        "{message}"
    );
}

#[test]
fn root_owned_system_executable_is_a_protected_location() {
    // A root-owned executable in a root-only directory chain on a normal (suid) mount.
    let candidate = ["/usr/bin/env", "/usr/bin/true"]
        .into_iter()
        .map(Path::new)
        .find(|path| fs::symlink_metadata(path).is_ok_and(|value| value.is_file()))
        .expect("a regular system executable");
    protected_helper(candidate, None).unwrap();
}

#[test]
fn private_socket_is_exclusive_and_cleanup_is_exact() {
    let nonce = unique_nonce();
    let socket = PrivateSocket::new(&nonce).unwrap();
    let path = socket.path.clone();
    let parent = path.parent().unwrap().to_owned();
    let value = fs::symlink_metadata(&parent).unwrap();
    assert_eq!(value.mode() & 0o777, 0o700);
    assert_eq!(value.uid(), unsafe { libc::getuid() });
    assert_eq!(
        PrivateSocket::new(&nonce).err().unwrap().kind(),
        io::ErrorKind::AlreadyExists
    );
    assert!(path.exists());
    drop(socket);
    assert!(!path.exists());
    assert!(!parent.exists());
}

#[test]
fn kernel_peer_credentials_and_creation_time_reject_substitutions() {
    let (left, right) = UnixStream::pair().unwrap();
    let pid = std::process::id();
    let uid = unsafe { libc::geteuid() };
    let created = stamp(pid).unwrap();
    verify_peer(&left, pid, uid).unwrap();
    verify_parent(&right, pid, uid, created).unwrap();
    assert!(verify_peer(&left, pid + 1, uid).is_err());
    assert!(verify_peer(&left, pid, uid.wrapping_add(1)).is_err());
    assert!(verify_parent(&right, pid, uid, created.wrapping_add(1)).is_err());
}

#[test]
fn proc_identity_parser_handles_parentheses_spaces_and_non_utf8_comm() {
    let mut stat = b"42 (odd ) name \xff) S".to_vec();
    for _ in 4..22 {
        stat.extend_from_slice(b" 0");
    }
    stat.extend_from_slice(b" 987654321 0 0\n");
    assert_eq!(parse_stamp(&stat).unwrap(), 987654321);
    assert!(parse_stamp(b"42 (truncated) S 0").is_err());
    assert!(parse_stamp(b"no comm").is_err());
}

#[test]
fn nonce_parser_is_bounded_and_handshake_rejects_wrong_nonce() {
    let text = unique_nonce();
    let (nonce, parsed) = parse_nonce(OsStr::new(&text)).unwrap();
    assert_eq!(text, parsed);
    assert!(parse_nonce(OsStr::new("00")).is_err());
    assert!(parse_nonce(OsStr::new(&"z".repeat(64))).is_err());
    let (left, mut right) = UnixStream::pair().unwrap();
    right.write_all(&[0; 32]).unwrap();
    let error = handshake(&mut channel(left), &nonce, false).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
}

fn empty_reply() -> Reply {
    Reply {
        session: false,
        result: Ok(Vec::new()),
    }
}

#[test]
fn handshake_and_framed_reply_survive_peer_close() {
    let (left, right) = UnixStream::pair().unwrap();
    let nonce = [27; 32];
    let worker = thread::spawn(move || {
        let mut helper = channel(right);
        handshake(&mut helper, &nonce, false).unwrap();
        assert_eq!(
            protocol::read_request(&mut helper).unwrap(),
            Request::List(b"/fixture".to_vec())
        );
        protocol::write_reply(&mut helper, &empty_reply()).unwrap();
    });
    let mut parent = channel(left);
    handshake(&mut parent, &nonce, true).unwrap();
    protocol::write_request(&mut parent, &Request::List(b"/fixture".to_vec())).unwrap();
    worker.join().unwrap();
    assert_eq!(protocol::read_reply(&mut parent).unwrap(), empty_reply());
}

#[test]
fn large_frames_cross_many_socket_buffers_through_readiness_waits() {
    let (left, right) = UnixStream::pair().unwrap();
    let entries: Vec<_> = (0..20_000u32)
        .map(|index| protocol::WireEntry {
            name: format!("entry-{index:08}-{}", "x".repeat(200)).into_bytes(),
            kind: 2,
            size: Some(u64::from(index)),
            modified: None,
            native_attributes: None,
            unix_mode: Some(0o100644),
        })
        .collect();
    let expected = entries.len();
    // About 4.5 MiB: both ends repeatedly wait for readiness on full or empty buffers.
    let writer = thread::spawn(move || {
        let reply = Reply {
            session: true,
            result: Ok(entries),
        };
        protocol::write_reply(&mut channel(right), &reply).unwrap();
    });
    let reply = protocol::read_reply(&mut channel(left))
        .unwrap()
        .result
        .unwrap();
    writer.join().unwrap();
    assert_eq!(reply.len(), expected);
    assert_eq!(reply[19_999].size, Some(19_999));
}

#[test]
fn blocked_channel_read_cancels_and_deadline_is_enforced() {
    let (left, _right) = UnixStream::pair().unwrap();
    let mut blocked = channel(left);
    let cancel = blocked.cancel.clone();
    let worker = thread::spawn(move || blocked.read_exact(&mut [0; 1]).unwrap_err());
    cancel.cancel();
    assert_eq!(
        worker.join().unwrap().kind(),
        io::ErrorKind::ConnectionAborted
    );
    let (left, _right) = UnixStream::pair().unwrap();
    let mut expired = Channel::new(left, Cancellation::default(), Instant::now()).unwrap();
    assert_eq!(
        expired.read(&mut [0; 1]).unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
}

#[test]
fn truncated_reply_is_rejected() {
    let (left, mut right) = UnixStream::pair().unwrap();
    right.write_all(b"MELIST02\x20\0\0\0\x01").unwrap();
    drop(right);
    assert_eq!(
        protocol::read_reply(&mut channel(left)).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
}

#[test]
fn monitor_detects_a_second_request_or_closure_and_stops_on_request() {
    let pid = std::process::id();
    let created = stamp(pid).unwrap();
    let far = || Instant::now() + Duration::from_secs(60);
    // A second request wakes the watcher immediately.
    let (left, mut right) = UnixStream::pair().unwrap();
    let (stop, stopped) = UnixStream::pair().unwrap();
    right.write_all(b"unexpected second request").unwrap();
    assert!(requester_gone(&left, &stopped, pid, created, far()));
    // So does the requester closing its end.
    let (left, right) = UnixStream::pair().unwrap();
    drop(right);
    assert!(requester_gone(&left, &stopped, pid, created, far()));
    // Closing the stop end ends watching without a violation.
    let (left, _right) = UnixStream::pair().unwrap();
    drop(stop);
    assert!(!requester_gone(&left, &stopped, pid, created, far()));
    // A changed requester identity is detected on the periodic check.
    let (left, _right) = UnixStream::pair().unwrap();
    let (_stop, stopped) = UnixStream::pair().unwrap();
    assert!(requester_gone(
        &left,
        &stopped,
        pid,
        created.wrapping_add(1),
        far()
    ));
}

#[test]
fn child_budget_is_reserved_before_launch_and_released_only_after_reaping() {
    let first = ChildSlot::acquire().unwrap();
    let second = ChildSlot::acquire().unwrap();
    assert_eq!(
        ChildSlot::acquire().err().unwrap().kind(),
        io::ErrorKind::WouldBlock
    );
    // A request waits briefly for a slot that a just-closed helper frees once it is reaped.
    let releaser = thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        drop(first);
    });
    let third = ChildSlot::acquire_within(RETIRING_WAIT).unwrap();
    releaser.join().unwrap();
    drop(second);
    drop(third);
    let child = RunningChild::spawn_in(
        Command::new("/bin/sleep").arg("10"),
        ChildSlot::acquire().unwrap(),
    )
    .unwrap();
    assert_eq!(CHILDREN.load(Ordering::Acquire), 1);
    let pid = child.pid;
    drop(child);
    let deadline = Instant::now() + Duration::from_secs(3);
    while CHILDREN.load(Ordering::Acquire) != 0 && Instant::now() < deadline {
        thread::sleep(TICK);
    }
    assert_eq!(CHILDREN.load(Ordering::Acquire), 0);
    assert!(!Path::new(&format!("/proc/{pid}")).exists());
    // Spawn failure drops the sender; the waiting reaper releases its reservation.
    assert!(
        RunningChild::spawn_in(
            &mut Command::new("/definitely/missing/helper"),
            ChildSlot::acquire().unwrap()
        )
        .is_err()
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while CHILDREN.load(Ordering::Acquire) != 0 && Instant::now() < deadline {
        thread::sleep(TICK);
    }
    assert_eq!(CHILDREN.load(Ordering::Acquire), 0);
}

#[test]
fn malformed_helper_arguments_fail_before_connecting_or_reading_a_directory() {
    assert!(serve(&[]).is_err());
    assert!(
        serve(&[
            "--linux-channel".into(),
            "bad".into(),
            "1".into(),
            "1".into(),
            "1".into()
        ])
        .is_err()
    );
}

#[test]
fn disconnected_parent_terminates_the_dedicated_helper_process() {
    let socket = PrivateSocket::new(&unique_nonce()).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "native::tests::disconnect_monitor_child",
            "--ignored",
            "--nocapture",
        ])
        .env("MIDNIGHT_TEST_MONITOR_SOCKET", &socket.path)
        .env(
            "MIDNIGHT_TEST_MONITOR_PARENT",
            std::process::id().to_string(),
        )
        .env(
            "MIDNIGHT_TEST_MONITOR_STAMP",
            stamp(std::process::id()).unwrap().to_string(),
        )
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let pid = child.id();
    let (finished, exit) = mpsc::channel();
    let waiter = thread::spawn(move || {
        let mut child = child;
        let _ = finished.send(child.wait());
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    let peer = loop {
        match socket.listener.accept() {
            Ok((peer, _)) => break peer,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "monitor child failed to connect");
                wait_ready(
                    socket.listener.as_raw_fd(),
                    libc::POLLIN,
                    slice(deadline, CANCEL_POLL),
                )
                .unwrap();
            }
            Err(error) => panic!("{error}"),
        }
    };
    drop(peer);
    let status = exit
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|_| {
            // SAFETY: the waiter has not reaped this child, so its PID cannot have been reused.
            unsafe { libc::kill(pid as libc::pid_t, libc::SIGKILL) };
            panic!("disconnected monitor did not terminate helper");
        });
    waiter.join().unwrap();
    assert_eq!(status.unwrap().code(), Some(1));
}

#[test]
#[ignore = "subprocess-only helper; invoked by the parent-disconnect regression"]
fn disconnect_monitor_child() {
    let path = std::env::var_os("MIDNIGHT_TEST_MONITOR_SOCKET").unwrap();
    let parent: u32 = std::env::var("MIDNIGHT_TEST_MONITOR_PARENT")
        .unwrap()
        .parse()
        .unwrap();
    let created: u64 = std::env::var("MIDNIGHT_TEST_MONITOR_STAMP")
        .unwrap()
        .parse()
        .unwrap();
    let socket = UnixStream::connect(path).unwrap();
    verify_parent(&socket, parent, unsafe { libc::getuid() }, created).unwrap();
    let _monitor = Monitor::start(
        &socket,
        parent,
        created,
        Instant::now() + Duration::from_secs(30),
        Cancellation::default(),
        exit_helper,
    )
    .unwrap();
    // The monitor exits this dedicated process when the requester disconnects.
    loop {
        thread::park();
    }
}

/// A folder tree in a fresh temporary folder: the authorized folder with `child/grandchild`,
/// a file, a symbolic link to its own `child` and one to a sibling folder outside it.
struct Tree {
    _base: tempfile::TempDir,
    root: PathBuf,
    sibling: PathBuf,
}

fn tree() -> Tree {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("authorized");
    let sibling = base.path().join("authorized-sibling");
    fs::create_dir_all(root.join("child").join("grandchild")).unwrap();
    fs::write(root.join("file.txt"), b"file").unwrap();
    fs::create_dir_all(sibling.join("nested")).unwrap();
    fs::write(sibling.join("secret.txt"), b"secret").unwrap();
    symlink("child", root.join("inside")).unwrap();
    symlink(&sibling, root.join("outside")).unwrap();
    symlink("../authorized-sibling", root.join("relative-outside")).unwrap();
    Tree {
        _base: base,
        root,
        sibling,
    }
}

fn names(entries: &[protocol::WireEntry]) -> Vec<PathBuf> {
    let mut names: Vec<_> = entries
        .iter()
        .map(|entry| decode_path(&entry.name).unwrap())
        .collect();
    names.sort();
    names
}

/// The helper's containment, run unprivileged: its first listing makes the folder the
/// session's root; later requests are served for that folder and the folders inside it only,
/// decided by name before anything is opened and on the opened folder's final location.
#[test]
fn the_helper_lists_the_authorized_folder_and_its_subfolders_only() {
    let tree = tree();
    let cancel = Cancellation::default();
    let mut session = Session::default();
    let mut list = |path: &Path| session.list(&encode_path(path), &cancel);
    let reply = list(&tree.root);
    assert!(reply.session, "the first listing establishes the session");
    assert_eq!(
        names(&reply.result.unwrap()),
        ["child", "file.txt", "inside", "outside", "relative-outside"].map(PathBuf::from)
    );
    for path in [
        tree.root.clone(),
        tree.root.join("child"),
        tree.root.join("child").join("grandchild"),
        // A link whose target is inside the folder leads to a subfolder.
        tree.root.join("inside"),
        tree.root.join("inside").join("grandchild"),
    ] {
        let reply = list(&path);
        assert!(reply.session);
        assert!(
            reply.result.is_ok(),
            "{}: {:?}",
            path.display(),
            reply.result
        );
    }
    assert_eq!(
        names(&list(&tree.root.join("inside")).result.unwrap()),
        [PathBuf::from("grandchild")]
    );
    for path in [
        tree.root.parent().unwrap().to_owned(),
        tree.root.join(".."),
        tree.root.join("child").join("..").join(".."),
        tree.root.join("..").join("authorized-sibling"),
        tree.sibling.clone(),
        tree.root.join("outside"),
        tree.root.join("outside").join("nested"),
        tree.root.join("relative-outside"),
        PathBuf::from("/proc"),
    ] {
        let reply = list(&path);
        assert!(reply.session, "a refusal keeps the session");
        let failure = reply.result.unwrap_err();
        assert_eq!(
            failure.kind,
            FailureKind::Outside,
            "{}: {}",
            path.display(),
            failure.message
        );
    }
    // A file and a missing folder inside it are refused for their own reasons.
    for path in [tree.root.join("file.txt"), tree.root.join("missing")] {
        let reply = list(&path);
        assert!(reply.session);
        assert_eq!(
            reply.result.unwrap_err().kind,
            FailureKind::Other,
            "{}",
            path.display()
        );
    }
}

/// A filesystem mounted on a folder inside the authorized folder is another volume: the
/// root folder's own subfolders are listed, the separately mounted `/proc` is not.
#[test]
fn a_mount_inside_the_authorized_folder_is_outside_it() {
    let cancel = Cancellation::default();
    let mut session = Session::default();
    assert!(session.list(b"/", &cancel).session);
    let reply = session.list(b"/etc", &cancel);
    assert!(reply.result.is_ok(), "{:?}", reply.result);
    let reply = session.list(b"/proc", &cancel);
    assert!(reply.session);
    assert_eq!(reply.result.unwrap_err().kind, FailureKind::Outside);
}

#[test]
fn a_failed_first_listing_establishes_no_session() {
    let base = tempfile::tempdir().unwrap();
    let mut session = Session::default();
    let reply = session.list(
        &encode_path(&base.path().join("missing")),
        &Cancellation::default(),
    );
    assert!(!reply.session);
    assert!(reply.result.is_err());
    assert!(!session.established());
}

fn ignore_violation() {}

/// Both ends of a session in this process: the helper's loop on a thread over a socket pair,
/// and the requester's connection after the first listing of `root`.
fn session_over_socket(
    root: &Path,
    idle: Duration,
    lifetime: Duration,
) -> (Connection, thread::JoinHandle<io::Result<()>>) {
    let (requester, helper_end) = UnixStream::pair().unwrap();
    let parent = std::process::id();
    let created = stamp(parent).unwrap();
    let helper = thread::spawn(move || {
        let limits = Limits {
            request: Duration::from_secs(30),
            idle,
            lifetime,
        };
        let started = Instant::now();
        let mut helper = SocketRequester {
            channel: Channel::new(
                helper_end,
                Cancellation::default(),
                started + limits.request,
            )
            .unwrap(),
            parent,
            created,
            terminate: ignore_violation,
        };
        helper::serve_session(&mut helper, started, started + limits.request, &limits)
    });
    let mut connection = Connection {
        channel: channel(requester),
        _child: None,
    };
    let reply = connection
        .exchange(&Request::List(encode_path(root)), &Cancellation::default())
        .unwrap();
    assert!(reply.session);
    reply.result.unwrap();
    (connection, helper)
}

fn read(
    session: &dyn ElevatedListingSession,
    path: &Path,
) -> Result<Vec<PathBuf>, SessionListingError> {
    let mut paths = Vec::new();
    session
        .read_directory(path, &Cancellation::default(), &mut |entry| {
            paths.push(entry.native_path().unwrap().to_owned());
            true
        })
        .map(|()| {
            paths.sort();
            paths
        })
}

/// One consent, many listings: the requester's session lists subfolders over the same
/// socket without another helper, the helper refuses what is outside even when asked
/// directly, and closing the session ends the helper.
#[test]
fn a_session_serves_many_requests_over_one_socket_until_it_is_closed() {
    let tree = tree();
    let (mut connection, helper) =
        session_over_socket(&tree.root, Duration::from_secs(30), Duration::from_secs(60));
    let reply = connection
        .exchange(
            &Request::List(encode_path(&tree.sibling)),
            &Cancellation::default(),
        )
        .unwrap();
    assert!(reply.session);
    assert_eq!(reply.result.unwrap_err().kind, FailureKind::Outside);
    let session = crate::session::connected(&tree.root, connection, Duration::from_secs(60));
    assert_eq!(session.root(), tree.root);
    let child = tree.root.join("child");
    assert_eq!(read(&*session, &child).unwrap(), [child.join("grandchild")]);
    assert_eq!(
        read(&*session, &tree.root.join("inside")).unwrap(),
        [tree.root.join("inside").join("grandchild")]
    );
    for outside in [tree.root.join("outside"), tree.sibling.clone()] {
        match read(&*session, &outside) {
            Err(SessionListingError::Outside(error)) => {
                assert!(error.is_outside_authorization(), "{error}");
                assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
                assert_eq!(error.path, outside);
            }
            other => panic!("{}: {other:?}", outside.display()),
        }
    }
    match read(&*session, &tree.root.join("missing")) {
        Err(SessionListingError::Failed(error)) => {
            assert_eq!(error.operation, "authorized directory listing");
        }
        other => panic!("{other:?}"),
    }
    assert!(session.is_live(), "refusals and failures keep the session");
    assert_eq!(read(&*session, &tree.root).unwrap().len(), 5);
    drop(session);
    helper.join().unwrap().unwrap();
}

#[test]
fn a_session_ends_when_idle_too_old_or_disconnected() {
    let tree = tree();
    // The helper ends an idle session by itself; its requester learns it when it asks.
    let (mut connection, helper) = session_over_socket(
        &tree.root,
        Duration::from_millis(200),
        Duration::from_secs(60),
    );
    helper.join().unwrap().unwrap();
    assert!(
        connection
            .exchange(
                &Request::List(encode_path(&tree.root)),
                &Cancellation::default()
            )
            .is_err()
    );
    // So does a session whose lifetime passed, however recently it was used.
    let (connection, helper) = session_over_socket(
        &tree.root,
        Duration::from_secs(30),
        Duration::from_millis(300),
    );
    helper.join().unwrap().unwrap();
    let session = crate::session::connected(&tree.root, connection, Duration::from_secs(60));
    assert!(matches!(
        read(&*session, &tree.root),
        Err(SessionListingError::Ended)
    ));
    assert!(!session.is_live());
    // The requester ends an idle session without asking, which ends its helper.
    let (connection, helper) =
        session_over_socket(&tree.root, Duration::from_secs(30), Duration::from_secs(60));
    let session = crate::session::connected(&tree.root, connection, Duration::from_millis(100));
    thread::sleep(Duration::from_millis(300));
    assert!(!session.is_live());
    assert!(matches!(
        read(&*session, &tree.root),
        Err(SessionListingError::Ended)
    ));
    helper.join().unwrap().unwrap();
    // A requester that disconnects without a close request ends it too.
    let (requester, helper_end) = UnixStream::pair().unwrap();
    let parent = std::process::id();
    let created = stamp(parent).unwrap();
    let helper = thread::spawn(move || {
        let started = Instant::now();
        let mut helper = SocketRequester {
            channel: Channel::new(helper_end, Cancellation::default(), started + TIMEOUT).unwrap(),
            parent,
            created,
            terminate: ignore_violation,
        };
        helper::serve_session(&mut helper, started, started + TIMEOUT, &Limits::SESSION)
    });
    let mut requester = channel(requester);
    protocol::write_request(&mut requester, &Request::List(encode_path(&tree.root))).unwrap();
    assert!(protocol::read_reply(&mut requester).unwrap().session);
    drop(requester);
    helper.join().unwrap().unwrap();
}

/// A cancelled request whose reply arrives within the grace keeps the session in step; one
/// whose reply does not arrive in time fails, and the requester's session then ends.
#[test]
fn a_cancelled_request_keeps_the_session_only_when_its_reply_arrives_soon() {
    let tree = tree();
    let (mut connection, helper) =
        session_over_socket(&tree.root, Duration::from_secs(30), Duration::from_secs(60));
    let cancelled = Cancellation::default();
    cancelled.cancel();
    let reply = connection
        .exchange(&Request::List(encode_path(&tree.root)), &cancelled)
        .unwrap();
    assert_eq!(names(&reply.result.unwrap()).len(), 5);
    let reply = connection
        .exchange(
            &Request::List(encode_path(&tree.root.join("child"))),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(names(&reply.result.unwrap()), [PathBuf::from("grandchild")]);
    drop(connection);
    helper.join().unwrap().unwrap();
    // Nobody answers: the cancelled request gives up after the grace.
    let (requester, _silent) = UnixStream::pair().unwrap();
    let mut connection = Connection {
        channel: channel(requester),
        _child: None,
    };
    let started = Instant::now();
    let error = connection
        .exchange(&Request::List(encode_path(&tree.root)), &cancelled)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
    assert!(started.elapsed() >= CANCEL_GRACE);
}
