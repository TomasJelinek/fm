use super::*;
use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
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
    let qa = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/elevation");
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
    let helper = thread::spawn(move || serve_with_monitor(&args, || {}));
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
    protocol::write_request(
        &mut channel,
        &Request {
            path: encode_path(fixture.path()),
        },
    )
    .unwrap();
    let mut entries = protocol::read_reply(&mut channel).unwrap().unwrap();
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
    helper.join().unwrap().unwrap();
}

#[test]
fn complete_buffered_reply_survives_a_closed_peer() {
    let (mut server, mut client) = connected(Duration::from_secs(10), Cancellation::default());
    let reply: Reply = Ok(vec![]);
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

#[test]
fn helper_rejects_untrusted_startup_arguments_before_connecting() {
    assert!(serve(&[]).is_err());
    assert!(serve(&["--pipe".into(), "bad".into(), "1".into(), "1".into()]).is_err());
}

#[test]
fn listing_metadata_comes_from_enumeration_without_following_links() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("file.bin"), b"12345").unwrap();
    fs::create_dir(root.path().join("folder")).unwrap();
    let reply = crate::helper::reply(
        Request {
            path: encode_path(root.path()),
        },
        &Cancellation::default(),
    )
    .unwrap();
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
    let reply = crate::helper::reply(
        Request {
            path: encode_path(root.path()),
        },
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!(reply.len(), 1);
    // The target does not exist; following it would have reported a file link instead.
    assert_eq!(reply[0].kind, 1);
    assert_eq!(reply[0].size, None);
}
