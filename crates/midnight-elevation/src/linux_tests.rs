use super::*;
use std::os::unix::fs::PermissionsExt;

fn channel(socket: UnixStream) -> Channel {
    Channel::new(
        socket,
        Cancellation::default(),
        Instant::now() + Duration::from_secs(2),
    )
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
    let reply = crate::helper::reply(
        Request {
            path: encode_path(directory.path()),
        },
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(reply.len(), 1);
    assert_eq!(reply[0].name, name.as_bytes());
    assert_eq!(reply[0].size, Some(3));
    assert_eq!(reply[0].unix_mode.unwrap() & 0o777, 0o640);
    assert_eq!(reply[0].native_attributes, None);
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

#[test]
fn one_shot_handshake_and_framed_reply_survive_peer_close() {
    let (left, right) = UnixStream::pair().unwrap();
    let nonce = [27; 32];
    let worker = thread::spawn(move || {
        let mut helper = channel(right);
        handshake(&mut helper, &nonce, false).unwrap();
        assert_eq!(
            protocol::read_request(&mut helper).unwrap().path,
            b"/fixture"
        );
        protocol::write_reply(&mut helper, &Ok(Vec::new())).unwrap();
    });
    let mut parent = channel(left);
    handshake(&mut parent, &nonce, true).unwrap();
    protocol::write_request(
        &mut parent,
        &Request {
            path: b"/fixture".to_vec(),
        },
    )
    .unwrap();
    worker.join().unwrap();
    assert_eq!(protocol::read_reply(&mut parent).unwrap(), Ok(Vec::new()));
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
fn truncated_reply_and_second_request_are_rejected() {
    let (left, mut right) = UnixStream::pair().unwrap();
    right.write_all(b"MELIST01\x20\0\0\0\x01").unwrap();
    drop(right);
    assert_eq!(
        protocol::read_reply(&mut channel(left)).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
    let (left, mut right) = UnixStream::pair().unwrap();
    assert!(!disconnected(&left));
    right.write_all(b"unexpected second request").unwrap();
    assert!(disconnected(&left));
    let (left, right) = UnixStream::pair().unwrap();
    drop(right);
    assert!(disconnected(&left));
}

#[test]
fn child_budget_is_reserved_before_launch_and_released_only_after_reaping() {
    let first = ChildSlot::acquire().unwrap();
    let second = ChildSlot::acquire().unwrap();
    assert_eq!(
        ChildSlot::acquire().err().unwrap().kind(),
        io::ErrorKind::WouldBlock
    );
    drop(first);
    drop(second);
    let child = RunningChild::spawn(Command::new("/bin/sleep").arg("10")).unwrap();
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
    assert!(RunningChild::spawn(&mut Command::new("/definitely/missing/helper")).is_err());
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
    let mut child = Command::new(std::env::current_exe().unwrap())
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
    let deadline = Instant::now() + Duration::from_secs(5);
    let peer = loop {
        match socket.listener.accept() {
            Ok((peer, _)) => break peer,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "monitor child failed to connect");
                thread::sleep(TICK);
            }
            Err(error) => panic!("{error}"),
        }
    };
    drop(peer);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert_eq!(status.code(), Some(1));
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("disconnected monitor did not terminate helper");
        }
        thread::sleep(TICK);
    }
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
    monitor_peer(
        socket,
        parent,
        created,
        Instant::now() + Duration::from_secs(5),
        Cancellation::default(),
    );
    panic!("monitor must exit the dedicated child");
}
