use super::*;
use crate::{WindowsFileSystem, test_fixtures::build_temp};
use midnight_core::{DeleteMode, MutationService};
use std::{
    cell::Cell,
    ffi::c_void,
    fs,
    io::{Read, Write},
    os::windows::io::OwnedHandle,
    sync::atomic::{AtomicUsize, Ordering},
};
use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, ERROR_SHARING_VIOLATION, HANDLE, INVALID_HANDLE_VALUE},
    Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, AddAccessAllowedAce, AddAccessDeniedAce,
        CreateWellKnownSid, DACL_SECURITY_INFORMATION, GetLengthSid, GetTokenInformation,
        InitializeAcl, InitializeSecurityDescriptor, RevertToSelf, SE_DACL_PROTECTED,
        SECURITY_DESCRIPTOR, SECURITY_IMPERSONATION_LEVEL, SECURITY_MAX_SID_SIZE,
        SecurityImpersonation, SetKernelObjectSecurity, SetSecurityDescriptorControl,
        SetSecurityDescriptorDacl, TOKEN_QUERY, TokenImpersonationLevel, WinWorldSid,
    },
    System::Threading::{GetCurrentThread, OpenThreadToken},
};

// The pipe-server half of the SQOS test only; the crate does not enable windows-sys
// `Win32_System_Pipes` for these two test calls.
#[link(name = "kernel32", kind = "raw-dylib")]
unsafe extern "system" {
    fn CreateNamedPipeW(
        name: *const u16,
        open_mode: u32,
        pipe_mode: u32,
        max_instances: u32,
        out_buffer_size: u32,
        in_buffer_size: u32,
        default_timeout: u32,
        security_attributes: *const c_void,
    ) -> HANDLE;
}
#[link(name = "advapi32", kind = "raw-dylib")]
unsafe extern "system" {
    fn ImpersonateNamedPipeClient(pipe: HANDLE) -> i32;
}

fn os(code: i32) -> io::Error {
    io::Error::from_raw_os_error(code)
}

/// Ends the test thread's impersonation, also when an assertion unwinds.
struct Reverted;
impl Drop for Reverted {
    fn drop(&mut self) {
        // SAFETY: ends this thread's impersonation; harmless when none is active.
        unsafe { RevertToSelf() };
    }
}

/// Creates a fresh named pipe of this test, lets `open` connect to it relative to the
/// named-pipe root directory, and returns the impersonation level the pipe server
/// obtains from that client.
fn level_seen_by_pipe_server(
    open: impl FnOnce(&File, &OsStr) -> io::Result<File>,
) -> SECURITY_IMPERSONATION_LEVEL {
    const PIPE_ACCESS_DUPLEX: u32 = 0x3;
    const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;
    const PIPE_REJECT_REMOTE_CLIENTS: u32 = 0x8;
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let name = format!(
        "midnight-sqos-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    );
    let wide: Vec<u16> = format!(r"\\.\pipe\{name}")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    // SAFETY: a NUL-terminated pipe name and default security; the result is checked.
    let server = unsafe {
        CreateNamedPipeW(
            wide.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_REJECT_REMOTE_CLIENTS,
            1,
            64,
            64,
            0,
            ptr::null(),
        )
    };
    assert_ne!(
        server,
        INVALID_HANDLE_VALUE,
        "{}",
        io::Error::last_os_error()
    );
    // SAFETY: a successful CreateNamedPipeW returned one owned server handle.
    let mut server = unsafe { File::from_raw_handle(server) };
    let pipes = OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(r"\\.\pipe\")
        .unwrap();
    let mut client = open(&pipes, OsStr::new(&name)).expect("the client reaches the pipe");
    // A pipe server can impersonate only after it read the client's data.
    client.write_all(b"x").unwrap();
    let mut byte = [0u8];
    server.read_exact(&mut byte).unwrap();
    // SAFETY: the connected server handle owned above.
    let impersonated = unsafe { ImpersonateNamedPipeClient(server.as_raw_handle()) };
    assert_ne!(impersonated, 0, "{}", io::Error::last_os_error());
    let _reverted = Reverted;
    let mut token = ptr::null_mut();
    // SAFETY: the current-thread pseudo handle and a writable handle output. The access
    // check uses the process context (OpenAsSelf): an identification-level token cannot
    // open itself.
    let opened = unsafe { OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, 1, &mut token) };
    assert_ne!(opened, 0, "{}", io::Error::last_os_error());
    // SAFETY: a successful OpenThreadToken returned one owned token handle.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut level: SECURITY_IMPERSONATION_LEVEL = 0;
    let mut returned = 0u32;
    // SAFETY: a live token with TOKEN_QUERY and an exactly sized writable output.
    let queried = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenImpersonationLevel,
            (&mut level as *mut SECURITY_IMPERSONATION_LEVEL).cast(),
            size_of::<SECURITY_IMPERSONATION_LEVEL>() as u32,
            &mut returned,
        )
    };
    assert_ne!(queried, 0, "{}", io::Error::last_os_error());
    level
}

/// The filter-mediated reopen re-evaluates whatever reparse point the pinned object
/// carries now. Should a symbolic link to a named pipe replace a filtered file's reparse
/// point after classification, the open reaches that pipe before the caller's identity
/// check rejects it; its server may identify this process but never impersonate it.
/// Planting such a symbolic link needs a privilege the test cannot assume, and Windows
/// refuses to follow a junction into the named-pipe device, so the test connects the
/// open that `reopen_mediated` performs (`open_identifying`) to the pipe by name.
#[test]
fn a_mediated_reopen_reaching_a_named_pipe_never_lends_impersonation() {
    let access = FILE_READ_DATA | FILE_WRITE_DATA;
    let share = FILE_SHARE_READ | FILE_SHARE_WRITE;
    // Control: the same open without a quality of service lends full impersonation.
    let plain = |parent: &File, name: &OsStr| {
        create_relative(
            parent,
            name,
            access,
            share,
            FILE_OPEN,
            FILE_NON_DIRECTORY_FILE,
            None,
        )
    };
    assert_eq!(level_seen_by_pipe_server(plain), SecurityImpersonation);
    let mediated = |parent: &File, name: &OsStr| open_identifying(parent, name, access, share);
    assert_eq!(level_seen_by_pipe_server(mediated), SecurityIdentification);
}

#[test]
fn confirmed_deletion_falls_back_only_where_the_filesystem_lacks_posix_disposition() {
    let classic = Cell::new(0);
    let probed = Cell::new(0);
    let run = |extended: io::Result<()>, read_only: bool| {
        dispose(
            || extended,
            || {
                classic.set(classic.get() + 1);
                Ok(())
            },
            || {
                probed.set(probed.get() + 1);
                Ok(read_only)
            },
        )
    };
    // Supported: neither the classic class nor the attribute probe is used.
    assert!(run(Ok(()), false).is_ok());
    assert_eq!((classic.get(), probed.get()), (0, 0));
    // FAT32/exFAT report the extended class as unsupported in one of three ways.
    for code in [1, 50, 87] {
        assert!(run(Err(os(code)), false).is_ok(), "{code}");
    }
    assert_eq!((classic.get(), probed.get()), (3, 3));
    // Other failures, such as a denied deletion, are never retried another way.
    let error = run(Err(os(5)), false).unwrap_err();
    assert_eq!(error.raw_os_error(), Some(5));
    // The classic class cannot ignore Read-only: confirmed deletion reports it.
    let error = run(Err(os(87)), true).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert!(error.to_string().contains("Read-only"), "{error}");
    assert_eq!(classic.get(), 3);
}

#[test]
fn copy_and_move_refuse_filesystems_without_stable_identities_with_an_actionable_error() {
    // FAT file indices follow directory-record positions and change when a transfer
    // renames its own stages, so Copy/Move never fall back to them.
    for code in [1, 50, 87] {
        let error = without_stable_identity(os(code));
        assert_eq!(error.kind(), io::ErrorKind::Unsupported, "{code}");
        let message = error.to_string();
        assert!(
            message.contains("Copy and Move are not supported") && message.contains("FAT32"),
            "{message}"
        );
    }
    let denied = without_stable_identity(os(5));
    assert_eq!(denied.raw_os_error(), Some(5), "other errors are unchanged");
}

#[test]
fn permanent_deletion_skips_children_another_process_removed_first() {
    let temp = build_temp("delete-race-");
    let root = temp.path().join("tree");
    fs::create_dir(&root).unwrap();
    let names: Vec<_> = (0..6).map(|index| format!("child-{index}.txt")).collect();
    for name in &names {
        fs::write(root.join(name), b"x").unwrap();
    }
    let cancel = Cancellation::default();
    let prepared = WindowsFileSystem::new(None)
        .prepare(&root, &cancel)
        .unwrap();
    // After the first removal, another process deletes the remaining snapshotted
    // children: the deletion still reaches its requested end state.
    let mut reported = Vec::new();
    let completion = prepared
        .delete(DeleteMode::Permanent, &cancel, &mut |path| {
            if reported.is_empty() {
                for name in &names {
                    let _ = fs::remove_file(root.join(name));
                }
            }
            reported.push(path.to_owned());
        })
        .unwrap();
    assert!(completion.residue.is_none());
    assert!(!root.exists());
    assert_eq!(
        reported.len(),
        2,
        "one child and the directory: {reported:?}"
    );
    assert_eq!(reported.last(), Some(&root));
}

fn millis(waits: &[Duration]) -> Vec<u128> {
    waits.iter().map(Duration::as_millis).collect()
}

#[test]
fn only_a_transient_access_refusal_is_retried_within_its_one_second_budget() {
    let denied = || os(ERROR_ACCESS_DENIED as i32);
    // A refusal that persists: 10 ms, then twice as long each time, one second in all.
    let (mut attempts, mut waits) = (0, Vec::new());
    let error = retry_refusal(
        || {
            attempts += 1;
            Err(denied())
        },
        || true,
        &mut |delay| {
            waits.push(delay);
            Ok(())
        },
    )
    .unwrap_err();
    assert_eq!(error.raw_os_error(), Some(ERROR_ACCESS_DENIED as i32));
    assert_eq!(millis(&waits), [10, 20, 40, 80, 160, 320, 370]);
    assert_eq!(waits.iter().sum::<Duration>(), RENAME_RETRY_BUDGET);
    assert_eq!(attempts, 8);
    // A refusal that passes: the same rename then succeeds.
    let (mut refusals, mut waits) = (2, Vec::new());
    retry_refusal(
        || match refusals {
            0 => Ok(()),
            _ => {
                refusals -= 1;
                Err(denied())
            }
        },
        || true,
        &mut |delay| {
            waits.push(delay);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(millis(&waits), [10, 20]);
    // A refusal the parent directory's DACL explains, and any other failure, is returned at
    // once.
    for (error, transient) in [
        (denied(), false),
        (os(ERROR_SHARING_VIOLATION as i32), true),
        (os(ERROR_ALREADY_EXISTS as i32), true),
        (io::ErrorKind::PermissionDenied.into(), true),
    ] {
        let (mut attempts, mut waited) = (0, false);
        let code = error.raw_os_error();
        let mut error = Some(error);
        let returned = retry_refusal(
            || {
                attempts += 1;
                Err(error.take().unwrap())
            },
            || transient,
            &mut |_| {
                waited = true;
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(returned.raw_os_error(), code);
        assert_eq!((attempts, waited), (1, false), "{returned}");
    }
    // Cancellation during a wait ends the retries with its own error.
    let mut attempts = 0;
    let error = retry_refusal(
        || {
            attempts += 1;
            Err(denied())
        },
        || true,
        &mut |_| Err(io::ErrorKind::Interrupted.into()),
    )
    .unwrap_err();
    assert_eq!((error.kind(), attempts), (io::ErrorKind::Interrupted, 1));
}

/// Holds an entry below `directory` open with full sharing, as a virus scanner holds a
/// file it scans. The entry is a subdirectory: no scanner opens it on its own, so the
/// test alone decides when it closes.
fn open_below(directory: &Path) -> File {
    let entry = directory.join("open");
    fs::create_dir(&entry).unwrap();
    OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(entry)
        .unwrap()
}

#[test]
fn a_directory_rename_waits_until_no_entry_below_it_is_open() {
    let temp = build_temp("rename-open-entry-");
    let folder = temp.path().join("folder");
    fs::create_dir(&folder).unwrap();
    let mut scanner = Some(open_below(&folder));
    let parent = open_parent(temp.path()).unwrap();
    let source = open_child(&parent, OsStr::new("folder"), true).unwrap();
    // Windows refuses to rename a directory while an entry below it is open, whatever
    // that handle's sharing.
    let refusal = rename_once(&source, OsStr::new("renamed")).unwrap_err();
    assert_eq!(refusal.raw_os_error(), Some(ERROR_ACCESS_DENIED as i32));
    let mut waits = Vec::new();
    rename_with(&source, &parent, OsStr::new("renamed"), &mut |delay| {
        waits.push(delay);
        // The scanner finishes during the first wait.
        scanner.take();
        Ok(())
    })
    .unwrap();
    assert_eq!(waits, [RENAME_RETRY_FIRST]);
    drop(source);
    assert!(!folder.exists());
    assert!(temp.path().join("renamed/open").is_dir());
}

#[test]
fn a_cancelled_directory_rename_stops_waiting_for_an_open_entry() {
    let temp = build_temp("rename-open-cancel-");
    let folder = temp.path().join("folder");
    fs::create_dir(&folder).unwrap();
    let _scanner = open_below(&folder);
    let parent = open_parent(temp.path()).unwrap();
    let source = open_child(&parent, OsStr::new("folder"), true).unwrap();
    let cancel = Cancellation::default();
    cancel.cancel();
    let error = rename(&source, &parent, OsStr::new("renamed"), &cancel).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::Interrupted);
    drop(source);
    assert!(folder.join("open").is_dir());
    // A live job waits the whole delay.
    let started = Instant::now();
    pause(RENAME_RETRY_FIRST, &Cancellation::default()).unwrap();
    assert!(started.elapsed() >= RENAME_RETRY_FIRST);
}

/// Gives the fixture directory `path` a protected DACL that denies `denied` to Everyone
/// and allows Everyone all other access, so the fixture stays removable.
fn deny_everyone(path: &Path, denied: u32) {
    let file = OpenOptions::new()
        .access_mode(READ_CONTROL | WRITE_DAC)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .unwrap();
    let mut everyone = [0u64; (SECURITY_MAX_SID_SIZE as usize).div_ceil(8)];
    let mut sid_size = SECURITY_MAX_SID_SIZE;
    let sid = everyone.as_mut_ptr().cast();
    // SAFETY: an aligned writable buffer of SECURITY_MAX_SID_SIZE bytes.
    let created = unsafe { CreateWellKnownSid(WinWorldSid, ptr::null_mut(), sid, &mut sid_size) };
    assert_ne!(created, 0, "{}", io::Error::last_os_error());
    // SAFETY: the valid SID created above. Allowed and denied ACEs share one layout.
    let ace =
        size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>() + unsafe { GetLengthSid(sid) } as usize;
    let length = size_of::<ACL>() + 2 * ace;
    let mut acl = vec![0u64; length.div_ceil(8)];
    let acl = acl.as_mut_ptr().cast::<ACL>();
    let mut descriptor = SECURITY_DESCRIPTOR::default();
    let descriptor = (&mut descriptor as *mut SECURITY_DESCRIPTOR).cast();
    // SAFETY: `acl` covers `length` writable bytes; it, the SID and the descriptor
    // (initialized in place, SECURITY_DESCRIPTOR_REVISION = 1) outlive these calls.
    let set = unsafe {
        InitializeAcl(acl, length as u32, ACL_REVISION) != 0
            && AddAccessDeniedAce(acl, ACL_REVISION, denied, sid) != 0
            && AddAccessAllowedAce(acl, ACL_REVISION, FILE_ALL_ACCESS, sid) != 0
            && InitializeSecurityDescriptor(descriptor, 1) != 0
            && SetSecurityDescriptorDacl(descriptor, 1, acl, 0) != 0
            && SetSecurityDescriptorControl(descriptor, SE_DACL_PROTECTED, SE_DACL_PROTECTED) != 0
            && SetKernelObjectSecurity(file.as_raw_handle(), DACL_SECURITY_INFORMATION, descriptor)
                != 0
    };
    assert!(set, "{}", io::Error::last_os_error());
}

#[test]
fn a_rename_the_parent_directory_denies_is_reported_without_waiting() {
    let temp = build_temp("rename-denied-parent-");
    // Besides DELETE, NTFS checks the right to add a subdirectory (for a folder) or a file
    // (for a file) to the parent. Each case denies only its own right, so a folder probe
    // would pass for the file and a file probe for the folder.
    for (name, denied) in [("folder", FILE_ADD_SUBDIRECTORY), ("file", FILE_ADD_FILE)] {
        let parent_path = temp.path().join(format!("{name}-parent"));
        fs::create_dir(&parent_path).unwrap();
        if name == "folder" {
            fs::create_dir(parent_path.join(name)).unwrap();
        } else {
            // Empty, so no virus scanner holds it while it is opened for DELETE below.
            File::create(parent_path.join(name)).unwrap();
        }
        deny_everyone(&parent_path, denied);
        let parent = open_parent(&parent_path).unwrap();
        let source = open_child(&parent, OsStr::new(name), true).unwrap();
        let mut waited = false;
        let error = rename_with(&source, &parent, OsStr::new("renamed"), &mut |_| {
            waited = true;
            Ok(())
        })
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(ERROR_ACCESS_DENIED as i32));
        assert!(!waited, "{name}");
        drop(source);
        assert!(parent_path.join(name).exists());
    }
}

#[test]
fn file_identity_combines_volume_and_file_id() {
    let temp = build_temp("identity-");
    let path = temp.path().join("file");
    fs::write(&path, b"x").unwrap();
    let first = identity(&File::open(&path).unwrap()).unwrap();
    let second = identity(&File::open(&path).unwrap()).unwrap();
    assert!(first == second);
    // Where FileIdInfo exists, mutation and transfer identities have the same form, so
    // a Rename replacement can compare its prepared identity with transfer captures.
    assert!(local_identity(&File::open(&path).unwrap()).unwrap() == first);
    let legacy = legacy_identity(&File::open(&path).unwrap()).unwrap();
    // On NTFS the 64-bit index is the low half of the 128-bit identifier.
    assert_eq!(legacy.id[..8], first.id[..8]);
    assert_eq!(legacy.volume as u32, first.volume as u32);
}

#[test]
fn metadata_snapshot_counts_encoded_name_bytes_and_keeps_no_spare_capacity() {
    let temp = build_temp("snapshot-");
    fs::create_dir(temp.path().join("listed")).unwrap();
    for name in ["ab", "文"] {
        fs::write(temp.path().join("listed").join(name), b"x").unwrap();
    }
    let parent = open_parent(temp.path()).unwrap();
    // Each snapshot reads its own listing handle from the start.
    let snapshot = |cancel: &Cancellation, max_names, max_bytes| {
        let listing = open_listing(&parent, OsStr::new("listed")).unwrap();
        bounded_child_names(&listing, cancel, max_names, max_bytes)
    };
    let cancel = Cancellation::default();
    // "ab" is 2 bytes and "文" 3 (`OsStr::len`, WTF-8): 5 in all, where UTF-16 counts 6.
    let mut names = snapshot(&cancel, 2, 5).unwrap();
    assert_eq!(names.capacity(), names.len());
    assert!(names.iter().all(|name| name.capacity() == name.len()));
    names.sort();
    assert_eq!(names, ["ab", "文"]);
    for (max_names, max_bytes) in [(2, 4), (1, 5)] {
        let refusal = snapshot(&cancel, max_names, max_bytes).unwrap_err();
        assert_eq!(
            refusal.kind(),
            io::ErrorKind::Other,
            "{max_names} names, {max_bytes} bytes: {refusal}"
        );
    }
    cancel.cancel();
    let cancelled = snapshot(&cancel, 2, 5).unwrap_err();
    assert_eq!(cancelled.kind(), io::ErrorKind::Interrupted);
    assert_eq!(cancelled.to_string(), "metadata listing cancelled");
}
