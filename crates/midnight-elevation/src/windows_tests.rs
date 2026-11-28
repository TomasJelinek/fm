use super::*;
use crate::{
    helper::{Limits, Session},
    protocol::FailureKind,
};
use midnight_core::{ElevatedListingSession, SessionListingError};
use std::{
    fs::{self, OpenOptions},
    os::windows::fs::OpenOptionsExt,
};
use windows_sys::Win32::{
    Security::{
        GetTokenInformation, RevertToSelf, SecurityIdentification, TOKEN_QUERY,
        TokenImpersonationLevel,
    },
    Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT},
    System::{
        IO::DeviceIoControl,
        Pipes::ImpersonateNamedPipeClient,
        Threading::{GetCurrentThread, OpenThreadToken},
    },
};

/// Create an NTFS junction natively, without a shell; the target need not exist.
pub(super) fn create_junction(link: &Path, target: &Path) -> io::Result<()> {
    const FSCTL_SET_REPARSE_POINT: u32 = 0x0009_00a4;
    const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xa000_0003;
    fs::create_dir(link)?;
    let result = (|| {
        let directory = OpenOptions::new()
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(link)?;
        let units: Vec<u16> = target.as_os_str().encode_wide().collect();
        let print = units.strip_prefix(&[92, 92, 63, 92][..]).unwrap_or(&units);
        let substitute: Vec<u16> = r"\??\"
            .encode_utf16()
            .chain(print.iter().copied())
            .collect();
        let names: Vec<u16> = substitute
            .iter()
            .chain(&[0])
            .chain(print)
            .chain(&[0])
            .copied()
            .collect();
        let mut buffer = Vec::new();
        buffer.extend(IO_REPARSE_TAG_MOUNT_POINT.to_le_bytes());
        buffer.extend(u16::try_from(8 + names.len() * 2).unwrap().to_le_bytes());
        buffer.extend([0, 0, 0, 0]);
        buffer.extend(u16::try_from(substitute.len() * 2).unwrap().to_le_bytes());
        buffer.extend(
            u16::try_from((substitute.len() + 1) * 2)
                .unwrap()
                .to_le_bytes(),
        );
        buffer.extend(u16::try_from(print.len() * 2).unwrap().to_le_bytes());
        buffer.extend(names.iter().flat_map(|unit| unit.to_le_bytes()));
        let mut returned = 0;
        // SAFETY: a live directory handle and a complete mount-point reparse buffer.
        if unsafe {
            DeviceIoControl(
                directory.as_raw_handle(),
                FSCTL_SET_REPARSE_POINT,
                buffer.as_ptr().cast(),
                buffer.len() as u32,
                ptr::null_mut(),
                0,
                &mut returned,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_dir(link);
    }
    result
}

fn own_process() -> OwnedHandle {
    // SAFETY: querying our own process identity does not transfer a handle.
    query_process(unsafe { GetCurrentProcessId() }).unwrap()
}

/// A connected server/client pair; both ends are overlapped channels.
fn connected(deadline: Duration, cancel: Cancellation) -> (Channel, Channel) {
    let (_, text) = nonce().unwrap();
    let name = pipe_name(&text).unwrap();
    let pipe = server(&name).unwrap();
    let own = own_process();
    let deadline = Instant::now() + deadline;
    let client = connect_client(&name, own.as_raw_handle(), deadline).unwrap();
    let mut server = Channel::new(pipe, cancel, deadline).unwrap();
    server.connect(own.as_raw_handle()).unwrap();
    let client = Channel::new(client, Cancellation::default(), deadline).unwrap();
    (server, client)
}

#[test]
fn windows_paths_preserve_unpaired_surrogates_and_reject_nul() {
    let path = PathBuf::from(OsString::from_wide(&[b'C' as u16, 58, 92, 0xd800]));
    assert_eq!(decode_path(&encode_path(&path)).unwrap(), path);
    assert!(decode_path(&[0]).is_err());
    assert!(decode_path(&[0, 0]).is_err());
}

#[test]
fn authenticated_channel_reads_unicode_directory_without_elevation() {
    // The helper lists ordinary absolute paths only (navigation resolves `..` first).
    let qa = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join("_build")
        .join("qa")
        .join("elevation");
    fs::create_dir_all(&qa).unwrap();
    let fixture = tempfile::Builder::new()
        .prefix("pipe-")
        .tempdir_in(qa)
        .unwrap();
    fs::write(fixture.path().join("Příliš_中文_🧪.txt"), b"test").unwrap();
    fs::create_dir(fixture.path().join("složka")).unwrap();
    let (nonce, text) = nonce().unwrap();
    let pipe = server(&pipe_name(&text).unwrap()).unwrap();
    // SAFETY: querying our own process identity does not transfer a handle.
    let pid = unsafe { GetCurrentProcessId() };
    let created = stamp(unsafe { GetCurrentProcess() }).unwrap();
    let args = vec![
        "--pipe".into(),
        text.into(),
        pid.to_string().into(),
        created.to_string().into(),
    ];
    let helper = thread::spawn(move || serve_with(&args, || {}, &Limits::SESSION));
    let peer = own_process();
    let mut channel = Channel::new(
        pipe,
        Cancellation::default(),
        Instant::now() + Duration::from_secs(10),
    )
    .unwrap();
    channel.connect(peer.as_raw_handle()).unwrap();
    let mut peer_pid = 0;
    // SAFETY: the live pipe and an initialized output.
    assert_ne!(
        unsafe { GetNamedPipeClientProcessId(channel.pipe().as_raw_handle(), &mut peer_pid) },
        0
    );
    assert_eq!(peer_pid, pid);
    channel.write_all(&nonce).unwrap();
    let mut proof = [0; 32];
    channel.read_exact(&mut proof).unwrap();
    assert_eq!(proof, nonce);
    protocol::write_request(&mut channel, &Request::List(encode_path(fixture.path()))).unwrap();
    let reply = protocol::read_reply(&mut channel).unwrap();
    assert!(reply.session, "the listed folder is the session's root");
    let mut entries = reply.result.unwrap();
    entries.sort_by(|left, right| left.name.cmp(&right.name));
    assert_eq!(entries.len(), 2);
    let file = entries
        .iter()
        .find(|entry| decode_path(&entry.name).unwrap() == Path::new("Příliš_中文_🧪.txt"))
        .unwrap();
    assert_eq!((file.kind, file.size), (2, Some(4)));
    assert!(file.modified.is_some() && file.native_attributes.is_some());
    let folder = entries
        .iter()
        .find(|entry| decode_path(&entry.name).unwrap() == Path::new("složka"))
        .unwrap();
    assert_eq!((folder.kind, folder.size), (0, None));
    // The session lists the subfolder over the same channel, then ends on request.
    protocol::write_request(
        &mut channel,
        &Request::List(encode_path(&fixture.path().join("složka"))),
    )
    .unwrap();
    let reply = protocol::read_reply(&mut channel).unwrap();
    assert!(reply.session);
    assert_eq!(reply.result.unwrap(), Vec::new());
    protocol::write_request(&mut channel, &Request::Close).unwrap();
    helper.join().unwrap().unwrap();
}

#[test]
fn complete_buffered_reply_survives_a_closed_peer() {
    let (mut server, mut client) = connected(Duration::from_secs(10), Cancellation::default());
    let reply = Reply {
        session: true,
        result: Ok(vec![]),
    };
    protocol::write_reply(&mut client, &reply).unwrap();
    drop(client);
    assert_eq!(protocol::read_reply(&mut server).unwrap(), reply);
    // After the buffered frame, the closed peer reads as end of stream.
    assert_eq!(
        server.read_exact(&mut [0]).unwrap_err().kind(),
        io::ErrorKind::UnexpectedEof
    );
}

#[test]
fn pending_reads_and_writes_honor_cancellation_and_the_deadline() {
    let cancel = Cancellation::default();
    let (mut server, _client) = connected(Duration::from_secs(10), cancel.clone());
    cancel.cancel();
    assert_eq!(
        server.read_exact(&mut [0]).unwrap_err().kind(),
        io::ErrorKind::ConnectionAborted
    );
    assert_eq!(
        server.write_all(&[1]).unwrap_err().kind(),
        io::ErrorKind::ConnectionAborted
    );
    // Nothing arrives: the read pends until the deadline, then is cancelled and awaited.
    let (mut server, _client) = connected(Duration::from_millis(300), Cancellation::default());
    let started = Instant::now();
    assert_eq!(
        server.read_exact(&mut [0]).unwrap_err().kind(),
        io::ErrorKind::TimedOut
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    // Nobody reads: a write larger than the pipe buffer pends until the deadline.
    let (mut server, _client) = connected(Duration::from_millis(300), Cancellation::default());
    assert_eq!(
        server
            .write_all(&vec![7; 4 * 1024 * 1024])
            .unwrap_err()
            .kind(),
        io::ErrorKind::TimedOut
    );
}

#[test]
fn elevated_client_grants_the_pipe_server_identification_only() {
    let (mut server, mut client) = connected(Duration::from_secs(10), Cancellation::default());
    client.write_all(&[1]).unwrap();
    server.read_exact(&mut [0]).unwrap();
    struct Revert;
    impl Drop for Revert {
        fn drop(&mut self) {
            // SAFETY: ends this test thread's impersonation.
            unsafe { RevertToSelf() };
        }
    }
    // SAFETY: the server end has read client data, which impersonation requires.
    assert_ne!(
        unsafe { ImpersonateNamedPipeClient(server.pipe().as_raw_handle()) },
        0,
        "{}",
        io::Error::last_os_error()
    );
    let _revert = Revert;
    let mut token = ptr::null_mut();
    // SAFETY: open this thread's impersonation token for query; ownership is taken below.
    assert_ne!(
        unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) },
        0,
        "{}",
        io::Error::last_os_error()
    );
    let token = handle(token).unwrap();
    let mut level = 0i32;
    let mut length = 0;
    // SAFETY: a live token and an output exactly the size of SECURITY_IMPERSONATION_LEVEL.
    assert_ne!(
        unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenImpersonationLevel,
                (&mut level as *mut i32).cast(),
                size_of::<i32>() as u32,
                &mut length,
            )
        },
        0
    );
    assert_eq!(level, SecurityIdentification);
}

/// A hostile requester's frames after the session was established: malformed or oversized
/// frames end the helper, a frame that never completes ends it at the idle limit, and a
/// request sent while a folder is listed ends the helper's process.
#[test]
fn hostile_frames_end_the_session_helper_in_bounded_time() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static TERMINATED: AtomicUsize = AtomicUsize::new(0);
    let tree = tree();
    let start = |idle: Duration| {
        let (requester, helper_end) = connected(Duration::from_secs(30), Cancellation::default());
        let helper = thread::spawn(move || {
            let limits = Limits {
                request: Duration::from_secs(30),
                idle,
                lifetime: Duration::from_secs(60),
            };
            let mut helper = PipeRequester {
                io: helper_end,
                peer: own_process(),
                terminate: || {
                    TERMINATED.fetch_add(1, Ordering::SeqCst);
                },
            };
            let started = Instant::now();
            helper::serve_session(&mut helper, started, started + limits.request, &limits)
        });
        (requester, helper)
    };
    let establish = |requester: &mut Channel| {
        protocol::write_request(requester, &Request::List(encode_path(&tree.root))).unwrap();
        assert!(protocol::read_reply(requester).unwrap().session);
    };
    let mut oversized = b"MELIST02".to_vec();
    oversized.extend_from_slice(&(protocol::MAX_FRAME as u32 + 1).to_le_bytes());
    let mut unknown = b"MELIST02".to_vec();
    unknown.extend_from_slice(&1u32.to_le_bytes());
    unknown.push(9);
    for hostile in [
        b"MELIST01\x01\0\0\0\x01".to_vec(),
        b"XXXXXXXXXXXXXXXXXXXX".to_vec(),
        oversized,
        unknown,
    ] {
        let (mut requester, helper) = start(Duration::from_secs(30));
        establish(&mut requester);
        requester.write_all(&hostile).unwrap();
        let error = helper.join().unwrap().unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{hostile:?}");
    }
    // A path the helper cannot decode is refused; the session continues.
    let (mut requester, helper) = start(Duration::from_secs(30));
    establish(&mut requester);
    for path in [vec![b'C'], vec![0, 0], Vec::new()] {
        protocol::write_request(&mut requester, &Request::List(path)).unwrap();
        let reply = protocol::read_reply(&mut requester).unwrap();
        assert!(reply.session);
        assert_eq!(reply.result.unwrap_err().kind, FailureKind::Other);
    }
    protocol::write_request(&mut requester, &Request::Close).unwrap();
    helper.join().unwrap().unwrap();
    // Half a frame, then silence: the helper ends at its idle limit.
    let (mut requester, helper) = start(Duration::from_millis(300));
    establish(&mut requester);
    let started = Instant::now();
    requester.write_all(b"MELIST02\xff\0\0\0partial").unwrap();
    helper.join().unwrap().unwrap();
    assert!(started.elapsed() < Duration::from_secs(10));
    // Two requests at once: the second arrives while the first is listed, which ends the
    // helper's process (here: the test's terminate), and nothing more is served.
    let before = TERMINATED.load(Ordering::SeqCst);
    let (mut requester, helper) = start(Duration::from_secs(30));
    establish(&mut requester);
    let mut both = Vec::new();
    protocol::write_request(&mut both, &Request::List(encode_path(&tree.root))).unwrap();
    protocol::write_request(&mut both, &Request::List(encode_path(&tree.sibling))).unwrap();
    requester.write_all(&both).unwrap();
    assert!(helper.join().unwrap().is_err());
    assert!(TERMINATED.load(Ordering::SeqCst) > before);
}

#[test]
fn helper_rejects_untrusted_startup_arguments_before_connecting() {
    assert!(serve(&[]).is_err());
    assert!(serve(&["--pipe".into(), "bad".into(), "1".into(), "1".into()]).is_err());
}

/// The helper's listing of `path` as a session's first request.
fn listed(path: &Path) -> Vec<protocol::WireEntry> {
    Session::default()
        .list(&encode_path(path), &Cancellation::default())
        .result
        .unwrap()
}

#[test]
fn listing_metadata_comes_from_enumeration_without_following_links() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file.bin"), b"12345").unwrap();
    fs::create_dir(root.path().join("folder")).unwrap();
    let reply = listed(root.path());
    let file = reply
        .iter()
        .find(|entry| decode_path(&entry.name).unwrap() == Path::new("file.bin"))
        .unwrap();
    assert_eq!((file.kind, file.size), (2, Some(5)));
    assert!(file.native_attributes.is_some() && file.modified.is_some());
    let folder = reply
        .iter()
        .find(|entry| decode_path(&entry.name).unwrap() == Path::new("folder"))
        .unwrap();
    assert_eq!(folder.kind, 0);
}

#[test]
fn junctions_are_directory_links_without_opening_their_targets() {
    let root = tempfile::tempdir().unwrap();
    create_junction(&root.path().join("dangling"), &root.path().join("missing")).unwrap();
    let reply = listed(root.path());
    assert_eq!(reply.len(), 1);
    // The target does not exist; following it would have reported a file link instead.
    assert_eq!(reply[0].kind, 1);
    assert_eq!(reply[0].size, None);
}

/// A folder tree in a fresh temporary folder: the authorized folder with
/// `child\grandchild`, a file, a junction to its own `child` and one to a sibling folder
/// outside it.
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
    create_junction(&root.join("inside"), &root.join("child")).unwrap();
    create_junction(&root.join("outside"), &sibling).unwrap();
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

/// The helper's containment, run unelevated: its first listing makes the folder the
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
        ["child", "file.txt", "inside", "outside"].map(PathBuf::from)
    );
    for path in [
        tree.root.clone(),
        tree.root.join("child"),
        tree.root.join("child").join("grandchild"),
        // A junction whose target is inside the folder leads to a subfolder.
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
    // Another volume: the system folder through a junction (on this volume or another one,
    // its final path is outside), and another drive by name.
    let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
    create_junction(&tree.root.join("system"), &system).unwrap();
    let drive = tree.root.to_string_lossy().chars().next().unwrap();
    let other_drive = PathBuf::from(format!(
        r"{}:\authorized",
        if drive.eq_ignore_ascii_case(&'C') {
            'D'
        } else {
            'C'
        }
    ));
    for path in [
        tree.root.parent().unwrap().to_owned(),
        tree.root.join(".."),
        tree.root.join("child").join("..").join(".."),
        tree.root
            .join("child")
            .join("..")
            .join("..")
            .join("authorized-sibling"),
        tree.sibling.clone(),
        tree.root.join("outside"),
        tree.root.join("outside").join("nested"),
        tree.root.join("system"),
        other_drive,
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

/// The names of `path`'s listing when it succeeds, for refusal checks that accept any
/// failure.
fn listed_names(session: &mut Session, path: &Path) -> Option<Vec<PathBuf>> {
    let reply = session.list(&encode_path(path), &Cancellation::default());
    assert!(
        reply.session,
        "{}: a refusal keeps the session",
        path.display()
    );
    reply.result.ok().map(|entries| names(&entries))
}

/// What a requester could spell instead of an ordinary name below the authorized folder:
/// other namespaces and prefixes, stream syntax, trailing dots and spaces, `..` hidden in a
/// stream name and 8.3 short names. None of them lists anything outside the folder.
#[test]
fn unusual_spellings_never_list_outside_the_authorized_folder() {
    let tree = tree();
    let mut session = Session::default();
    assert!(
        session
            .list(&encode_path(&tree.root), &Cancellation::default())
            .session
    );
    let root = tree.root.to_str().unwrap().to_owned();
    let base = tree.root.parent().unwrap();
    let root_names = names(&listed(&tree.root));
    let outside = [
        PathBuf::from("authorized-sibling"),
        PathBuf::from("nested"),
        PathBuf::from("secret.txt"),
    ];
    let mut spellings: Vec<PathBuf> = [
        format!(r"\\?\{root}\outside"),
        format!(r"\??\{root}\outside"),
        format!(r"\\.\{root}\outside"),
        format!(r"\\?\GLOBALROOT\??\{root}\outside"),
        format!(r"\\localhost\{}$\{}\outside", &root[..1], &root[3..]),
        format!(r"{root}\outside::$INDEX_ALLOCATION"),
        format!(r"{root}\outside:$I30:$INDEX_ALLOCATION"),
        format!(r"{root}\outside:stream"),
        format!(r"{root}\outside."),
        format!(r"{root}\outside "),
        format!(r"{root}\..::$INDEX_ALLOCATION"),
        format!(r"{root}\...::$INDEX_ALLOCATION"),
        format!(r"{root}\child\..::$INDEX_ALLOCATION\.."),
        format!(r"{root}\child\..::$INDEX_ALLOCATION"),
        format!(r"{root}\..."),
        format!(r"{root}/outside/nested"),
        format!(r"{root}\child/../../authorized-sibling"),
        format!(r"{root}\outside\nested\.."),
        format!(r"{root}\inside\..\..\authorized-sibling"),
        format!(r"{root}\NUL"),
        format!(r"{root}\CON\..\.."),
    ]
    .map(PathBuf::from)
    .into();
    // The 8.3 short name of the junction that leads outside, where the volume keeps them.
    let long: Vec<u16> = tree
        .root
        .join("outside")
        .as_os_str()
        .encode_wide()
        .chain([0])
        .collect();
    let mut short = vec![0u16; 1024];
    // SAFETY: a terminated path and a writable buffer of the stated length.
    let length = unsafe {
        windows_sys::Win32::Storage::FileSystem::GetShortPathNameW(
            long.as_ptr(),
            short.as_mut_ptr(),
            short.len() as u32,
        )
    } as usize;
    if length > 0 && length < short.len() {
        spellings.push(PathBuf::from(OsString::from_wide(&short[..length])));
    }
    for path in &spellings {
        if let Some(listed) = listed_names(&mut session, path) {
            for name in &outside {
                assert!(
                    !listed.contains(name),
                    "{} listed {listed:?}",
                    path.display()
                );
            }
            assert!(
                listed.is_empty()
                    || listed == root_names
                    || listed == [PathBuf::from("grandchild")],
                "{} listed {listed:?}",
                path.display()
            );
        }
    }
    // The base folder itself, by any of these spellings, is never listed.
    assert!(listed_names(&mut session, base).is_none());
}

/// Links that appear after the consent: a junction created inside the folder, and the folder
/// itself exchanged for a junction (by an account that can change its parent). The opened
/// folder's final location decides, so both lead outside and are refused.
#[test]
fn junctions_created_after_consent_cannot_lead_outside() {
    let tree = tree();
    let mut session = Session::default();
    assert!(
        session
            .list(&encode_path(&tree.root), &Cancellation::default())
            .session
    );
    let base = tree.root.parent().unwrap().to_owned();
    create_junction(&tree.root.join("late"), &tree.sibling).unwrap();
    create_junction(&tree.root.join("child").join("up"), &base).unwrap();
    create_junction(&tree.root.join("again"), &tree.root).unwrap();
    for path in [
        tree.root.join("late"),
        tree.root.join("late").join("nested"),
        tree.root.join("child").join("up"),
        tree.root
            .join("child")
            .join("up")
            .join("authorized-sibling"),
    ] {
        let failure = session
            .list(&encode_path(&path), &Cancellation::default())
            .result
            .unwrap_err();
        assert_eq!(failure.kind, FailureKind::Outside, "{}", path.display());
    }
    // A junction to the folder itself stays inside it.
    assert_eq!(
        listed_names(&mut session, &tree.root.join("again").join("child")),
        Some(vec![PathBuf::from("grandchild"), PathBuf::from("up")])
    );
    // The authorized folder moved away and a junction to the sibling put at its path.
    let moved = base.join("moved");
    fs::rename(&tree.root, &moved).unwrap();
    create_junction(&tree.root, &tree.sibling).unwrap();
    for path in [tree.root.clone(), tree.root.join("nested")] {
        let failure = session
            .list(&encode_path(&path), &Cancellation::default())
            .result
            .unwrap_err();
        assert_eq!(failure.kind, FailureKind::Outside, "{}", path.display());
    }
    fs::remove_dir(&tree.root).unwrap();
    fs::rename(&moved, &tree.root).unwrap();
}

/// A request that names a file inside the authorized folder is refused without opening the
/// file for its content: a file that another process holds without sharing its data stays
/// out of the helper's reach, and the refusal says it is no folder.
#[test]
fn a_file_inside_the_folder_is_refused_without_opening_its_content() {
    let tree = tree();
    let mut session = Session::default();
    assert!(
        session
            .list(&encode_path(&tree.root), &Cancellation::default())
            .session
    );
    let file = tree.root.join("file.txt");
    let _exclusive = OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(0)
        .open(&file)
        .unwrap();
    let failure = session
        .list(&encode_path(&file), &Cancellation::default())
        .result
        .unwrap_err();
    assert_eq!(failure.kind, FailureKind::Other);
    assert!(
        failure.message.contains("not a folder"),
        "{}",
        failure.message
    );
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

/// Both ends of a session in this process: the helper's loop on a thread over a real pipe,
/// and the requester's connection after the first listing of `root`.
fn session_over_pipe(
    root: &Path,
    idle: Duration,
    lifetime: Duration,
) -> (Connection, thread::JoinHandle<io::Result<()>>) {
    let (requester, helper_end) = connected(Duration::from_secs(30), Cancellation::default());
    let helper = thread::spawn(move || {
        let limits = Limits {
            request: Duration::from_secs(30),
            idle,
            lifetime,
        };
        let mut helper = PipeRequester {
            io: helper_end,
            peer: own_process(),
            terminate: || {},
        };
        let started = Instant::now();
        helper::serve_session(&mut helper, started, started + limits.request, &limits)
    });
    let mut connection = Connection {
        io: requester,
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
/// channel without another helper, the helper refuses what is outside even when asked
/// directly, and closing the session ends the helper.
#[test]
fn a_session_serves_many_requests_over_one_channel_until_it_is_closed() {
    let tree = tree();
    let (mut connection, helper) =
        session_over_pipe(&tree.root, Duration::from_secs(30), Duration::from_secs(60));
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
    assert_eq!(
        read(&*session, &child.join("grandchild")).unwrap(),
        Vec::<PathBuf>::new()
    );
    for outside in [tree.root.join("outside"), tree.sibling.clone()] {
        match read(&*session, &outside) {
            Err(SessionListingError::Outside(error)) => {
                assert!(error.is_outside_authorization(), "{error}");
                assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
                assert_eq!(error.path, outside);
                assert!(
                    error.message.contains(&tree.root.display().to_string()),
                    "{error}"
                );
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
    assert_eq!(read(&*session, &tree.root).unwrap().len(), 4);
    drop(session);
    helper.join().unwrap().unwrap();
}

#[test]
fn a_session_ends_when_idle_too_old_or_disconnected() {
    let tree = tree();
    // The helper ends an idle session by itself; its requester learns it when it asks.
    let (mut connection, helper) = session_over_pipe(
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
    let (connection, helper) = session_over_pipe(
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
        session_over_pipe(&tree.root, Duration::from_secs(30), Duration::from_secs(60));
    let session = crate::session::connected(&tree.root, connection, Duration::from_millis(100));
    thread::sleep(Duration::from_millis(300));
    assert!(!session.is_live());
    assert!(matches!(
        read(&*session, &tree.root),
        Err(SessionListingError::Ended)
    ));
    helper.join().unwrap().unwrap();
    // A requester that disconnects without a close request ends it too.
    let (mut requester, helper_end) = connected(Duration::from_secs(30), Cancellation::default());
    let helper = thread::spawn(move || {
        let mut helper = PipeRequester {
            io: helper_end,
            peer: own_process(),
            terminate: || {},
        };
        let started = Instant::now();
        helper::serve_session(
            &mut helper,
            started,
            started + Duration::from_secs(30),
            &Limits::SESSION,
        )
    });
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
        session_over_pipe(&tree.root, Duration::from_secs(30), Duration::from_secs(60));
    let cancelled = Cancellation::default();
    cancelled.cancel();
    let reply = connection
        .exchange(&Request::List(encode_path(&tree.root)), &cancelled)
        .unwrap();
    assert_eq!(names(&reply.result.unwrap()).len(), 4);
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
    let (requester, _silent) = connected(Duration::from_secs(30), Cancellation::default());
    let mut connection = Connection {
        io: requester,
        _child: None,
    };
    let started = Instant::now();
    let error = connection
        .exchange(&Request::List(encode_path(&tree.root)), &cancelled)
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::ConnectionAborted);
    assert!(started.elapsed() >= CANCEL_GRACE);
    assert!(started.elapsed() < CANCEL_GRACE + Duration::from_secs(5));
}
