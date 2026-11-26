use super::*;
use windows_sys::Win32::{
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            ConvertStringSidToSidW,
        },
        GetSecurityDescriptorDacl, GetSecurityDescriptorOwner, SECURITY_ATTRIBUTES,
    },
    Storage::FileSystem::CreateDirectoryW,
    System::IO::DeviceIoControl,
};

const ERROR_SHARING_VIOLATION: i32 = 32;
const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xa000_0003;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain([0]).collect()
}

/// Evaluate the elevation policy against an in-memory SDDL descriptor.
fn evaluate(sddl: &str, rights: u32) -> Result<(), &'static str> {
    evaluate_for(sddl, rights, None)
}

/// The same, trusting `user` (an SDDL SID string) as a build with the shipped digest does.
fn evaluate_for(sddl: &str, rights: u32, user: Option<&str>) -> Result<(), &'static str> {
    let user = user.map(sid);
    let text = wide(sddl);
    let mut raw = ptr::null_mut();
    // SAFETY: a terminated SDDL string and a writable output; LocalAlloc owns the result.
    assert_ne!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                1,
                &mut raw,
                ptr::null_mut(),
            )
        },
        0,
        "{sddl}: {}",
        io::Error::last_os_error()
    );
    let descriptor = Descriptor(raw);
    let mut owner = ptr::null_mut();
    let mut dacl = ptr::null_mut();
    let mut present = 0;
    let mut defaulted = 0;
    // SAFETY: the descriptor stays alive while its owner and DACL pointers are evaluated.
    unsafe {
        assert_ne!(
            GetSecurityDescriptorOwner(descriptor.0, &mut owner, &mut defaulted),
            0
        );
        assert_ne!(
            GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut dacl, &mut defaulted),
            0
        );
    }
    policy(owner, dacl, rights, user.as_ref().map(|user| user.0))
}

fn sid(text: &str) -> Descriptor {
    let text = wide(text);
    let mut sid = ptr::null_mut();
    // SAFETY: a terminated SID string and writable output; LocalAlloc owns the result.
    assert_ne!(
        unsafe { ConvertStringSidToSidW(text.as_ptr(), &mut sid) },
        0
    );
    Descriptor(sid)
}

#[test]
fn trusted_installer_constant_is_the_fixed_service_sid() {
    let text = wide("S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464");
    let mut sid = ptr::null_mut();
    // SAFETY: a terminated SID string and writable output; LocalAlloc owns the result.
    assert_ne!(
        unsafe { ConvertStringSidToSidW(text.as_ptr(), &mut sid) },
        0
    );
    let parsed = Descriptor(sid);
    let constant: PSID = TRUSTED_INSTALLER.as_ptr().cast_mut().cast();
    // SAFETY: both SIDs are valid and only read.
    assert!(unsafe { IsValidSid(constant) != 0 && EqualSid(constant, parsed.0) != 0 });
    assert!(administrative(parsed.0));
}

#[test]
fn only_administrative_owners_and_writers_are_accepted() {
    let trusted_installer = "S-1-5-80-956008885-3418522649-1831038044-1853292631-2271478464";
    let installer_owned = format!("O:{trusted_installer}D:P(A;;FA;;;{trusted_installer})");
    for (sddl, rights) in [
        // Program Files style: administrators modify, users read and execute.
        (
            "O:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;0x1200a9;;;BU)",
            MODIFY_RIGHTS,
        ),
        ("O:SYD:P(A;;FA;;;SY)(A;;0x1200a9;;;WD)", MODIFY_RIGHTS),
        (installer_owned.as_str(), MODIFY_RIGHTS),
        // Deny and inherit-only entries grant nothing on this object.
        ("O:BAD:P(D;;FA;;;WD)(A;;FA;;;BA)", MODIFY_RIGHTS),
        ("O:BAD:P(A;OICIIO;FA;;;WD)(A;;FA;;;BA)", MODIFY_RIGHTS),
        // A drive root lets users add folders, but cannot replace an existing ancestor.
        (
            "O:SYD:P(A;;FA;;;BA)(A;;0x4;;;AU)(A;;0x2;;;BU)",
            REPLACE_RIGHTS,
        ),
    ] {
        assert_eq!(evaluate(sddl, rights), Ok(()), "{sddl}");
    }
    for (sddl, rights, reason) in [
        ("O:BUD:P(A;;FA;;;BA)", REPLACE_RIGHTS, "owned"),
        ("O:WDD:P(A;;FA;;;BA)", REPLACE_RIGHTS, "owned"),
        (
            "O:BAD:P(A;;FA;;;BA)(A;;FA;;;BU)",
            REPLACE_RIGHTS,
            "non-administrative",
        ),
        (
            "O:BAD:P(A;;0x10000;;;WD)",
            REPLACE_RIGHTS,
            "non-administrative",
        ),
        (
            "O:BAD:P(A;;0x40000;;;AU)",
            REPLACE_RIGHTS,
            "non-administrative",
        ),
        (
            "O:BAD:P(A;;0x40;;;BU)",
            REPLACE_RIGHTS,
            "non-administrative",
        ),
        // A folder extracted below a drive root inherits Modify for Authenticated Users:
        // plugins load from it, but the elevated helper never runs from it.
        (
            "O:BAD:P(A;;FA;;;SY)(A;;FA;;;BA)(A;ID;0x1301bf;;;AU)",
            MODIFY_RIGHTS,
            "non-administrative",
        ),
        (
            "O:SYD:AI(A;ID;FA;;;BA)(A;ID;0x1301bf;;;AU)(A;ID;0x1200a9;;;BU)",
            REPLACE_RIGHTS,
            "non-administrative",
        ),
        // DLL planting into the helper's own folder, or rewriting the helper.
        ("O:BAD:P(A;;0x2;;;BU)", MODIFY_RIGHTS, "non-administrative"),
        ("O:BAD:P(A;;0x4;;;AU)", MODIFY_RIGHTS, "non-administrative"),
        (
            "O:BAD:NO_ACCESS_CONTROL",
            REPLACE_RIGHTS,
            "access control list",
        ),
        (
            "O:BAD:P(XA;;FA;;;WD;(Member_of {SID(BA)}))",
            REPLACE_RIGHTS,
            "unsupported",
        ),
    ] {
        let refused = evaluate(sddl, rights).unwrap_err();
        assert!(refused.contains(reason), "{sddl}: {refused}");
    }
}

#[test]
fn system_folder_executable_is_a_protected_location() {
    let system = std::env::var_os("SystemRoot").expect("SystemRoot");
    let executable = Path::new(&system)
        .join("System32")
        .join("cmd.exe")
        .canonicalize()
        .unwrap();
    let pins = pin_helper(&executable, None).unwrap();
    assert_eq!(pins.len(), executable.ancestors().count());
    // The plain spelling names the same pinned file, so the Shell receives an ordinary path.
    let launch = shell_path(&executable, &pins[0]);
    assert!(!launch.as_os_str().to_string_lossy().starts_with(r"\\?\"));
    assert_eq!(launch.file_name(), executable.file_name());
}

#[test]
fn user_owned_installation_is_refused_with_an_actionable_message() {
    let root = tempfile::tempdir().unwrap();
    let helper = root.path().join("midnight-elevated-listing.exe");
    std::fs::write(&helper, b"fixture").unwrap();
    let error = pin_helper(&helper.canonicalize().unwrap(), None).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    let message = error.to_string();
    assert!(
        message.contains("only administrators can change it"),
        "{message}"
    );
    assert!(message.contains("Program Files"), "{message}");
    // Which check refuses first depends on the temporary folder: one that standard users
    // may change (as on CI runners) is refused before the helper's owner is checked.
    assert!(
        message.contains("is not owned by SYSTEM")
            || message.contains("lets a non-administrative account change"),
        "{message}"
    );
    // The refused helper is not left pinned: it can be removed immediately.
    std::fs::remove_file(&helper).unwrap();
}

#[test]
fn network_and_relative_helper_paths_are_refused_before_any_access() {
    for path in [
        r"\\?\UNC\server\share\midnight-elevated-listing.exe",
        r"\\server\share\midnight-elevated-listing.exe",
        r"\\?\Volume{00000000-0000-0000-0000-000000000000}\helper.exe",
        "midnight-elevated-listing.exe",
    ] {
        let error = local_fixed_drive(Path::new(path), requirement(true)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{path}");
    }
}

#[test]
fn plain_shell_spelling_is_used_only_when_it_cannot_name_another_object() {
    assert_eq!(
        plain_drive_path(Path::new(r"\\?\C:\Program Files\Midnight\helper.exe")),
        Some(PathBuf::from(r"C:\Program Files\Midnight\helper.exe"))
    );
    for verbatim_only in [
        r"\\?\C:\Program Files\dots.\helper.exe",
        r"\\?\C:\Program Files\space \helper.exe",
        r"\\?\UNC\server\share\helper.exe",
        r"C:\Program Files\helper.exe",
    ] {
        assert_eq!(
            plain_drive_path(Path::new(verbatim_only)),
            None,
            "{verbatim_only}"
        );
    }
    let long = format!(r"\\?\C:\{}\helper.exe", "x".repeat(260));
    assert_eq!(plain_drive_path(Path::new(&long)), None);
    // A different file is never substituted for the pinned helper.
    let root = tempfile::tempdir().unwrap();
    let first = root.path().join("first.exe");
    let second = root.path().join("second.exe");
    std::fs::write(&first, b"first").unwrap();
    std::fs::write(&second, b"second").unwrap();
    let pinned = File::open(&first).unwrap();
    assert!(same_file(&first, &pinned));
    assert!(!same_file(&second, &pinned));
    assert!(!same_file(&root.path().join("missing.exe"), &pinned));
}

#[test]
fn pins_refuse_replacement_but_let_folders_gain_entries() {
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("installation");
    std::fs::create_dir(&folder).unwrap();
    let helper = folder.join("helper.exe");
    std::fs::write(&helper, b"fixture").unwrap();
    let outside = root.path().join("outside.txt");
    std::fs::write(&outside, b"unrelated").unwrap();
    let refusal = |result: io::Result<()>| result.unwrap_err().raw_os_error();
    // The folder pin alone: NTFS also refuses to rename a folder while an entry below it is
    // open (ERROR_ACCESS_DENIED), so the helper is pinned only afterwards.
    let folder_pin = pin_folder(&folder).unwrap();
    // Write sharing: other processes can still move entries into a pinned ancestor.
    std::fs::rename(&outside, folder.join("added.txt")).unwrap();
    // No delete sharing, and traverse access subjects the pin to share checks: the folder can
    // be neither renamed nor removed.
    assert_eq!(
        refusal(std::fs::rename(&folder, root.path().join("moved"))),
        Some(ERROR_SHARING_VIOLATION)
    );
    // Read sharing only: the helper can be neither rewritten, renamed nor deleted.
    let helper_pin = pin_file(&helper).unwrap();
    assert_eq!(
        refusal(OpenOptions::new().write(true).open(&helper).map(drop)),
        Some(ERROR_SHARING_VIOLATION)
    );
    assert_eq!(
        refusal(std::fs::rename(&helper, folder.join("renamed.exe"))),
        Some(ERROR_SHARING_VIOLATION)
    );
    assert_eq!(
        refusal(std::fs::remove_file(&helper)),
        Some(ERROR_SHARING_VIOLATION)
    );
    drop(helper_pin);
    drop(folder_pin);
    // Released pins refuse nothing.
    std::fs::rename(&folder, root.path().join("moved")).unwrap();
}

#[test]
fn only_storage_reparse_tags_leave_an_ordinary_object() {
    const ARCHIVE: u32 = 0x20;
    let reparse = FILE_ATTRIBUTE_REPARSE_POINT;
    let directory = FILE_ATTRIBUTE_DIRECTORY;
    assert_eq!(object_policy(ARCHIVE, 0, false), Ok(()));
    assert_eq!(object_policy(directory, 0, true), Ok(()));
    // WOF compression (CompactOS, `compact /exe`) and Data Deduplication keep the content
    // under the file's own permissions.
    for tag in STORAGE_TAGS {
        assert_eq!(object_policy(ARCHIVE | reparse, tag, false), Ok(()));
    }
    for tag in [
        0xa000_000c,                // symbolic link
        IO_REPARSE_TAG_MOUNT_POINT, // junction or mount point
        0x9000_001a,                // Cloud Files placeholder
        0x9000_f01a,                // Cloud Files placeholder with provider flags
        0x9000_001c,                // projected file system
        0x8000_001b,                // app execution alias
        0x8000_0021,                // OneDrive
        0x0000_1234,                // unknown third-party tag
    ] {
        assert_eq!(
            object_policy(reparse, tag, false),
            Err("is a link or reparse point"),
            "{tag:#x}"
        );
        assert_eq!(
            object_policy(directory | reparse, tag, true),
            Err("is a link or reparse point"),
            "{tag:#x}"
        );
    }
    // The helper must be a file and every ancestor a folder.
    assert_eq!(
        object_policy(directory, 0, false),
        Err("is not an ordinary file or folder")
    );
    assert_eq!(
        object_policy(ARCHIVE, 0, true),
        Err("is not an ordinary file or folder")
    );
}

/// Issue one buffered file-system control to `file`; empty buffers are passed as null.
fn control(file: &File, code: u32, input: &[u32], output: &mut [u32]) -> io::Result<()> {
    let input_buffer = if input.is_empty() {
        ptr::null()
    } else {
        input.as_ptr().cast()
    };
    let output_buffer = if output.is_empty() {
        ptr::null_mut()
    } else {
        output.as_mut_ptr().cast()
    };
    let mut returned = 0;
    // SAFETY: a live handle and input/output buffers valid for their stated sizes.
    if unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            code,
            input_buffer,
            size_of_val(input) as u32,
            output_buffer,
            size_of_val(output) as u32,
            &mut returned,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[test]
fn reparse_tags_are_read_from_the_pinned_handles() {
    const FSCTL_SET_EXTERNAL_BACKING: u32 = 0x0009_030c;
    const FSCTL_GET_EXTERNAL_BACKING: u32 = 0x0009_0310;
    const WOF_PROVIDER_FILE: u32 = 2;
    let root = tempfile::tempdir().unwrap();
    // A junction is refused from its own attributes; its target is never opened.
    let junction = root.path().join("junction");
    super::super::tests::create_junction(&junction, &root.path().join("missing")).unwrap();
    let (attributes, tag) = attribute_tag(&pin_folder(&junction).unwrap()).unwrap();
    assert_eq!(tag, IO_REPARSE_TAG_MOUNT_POINT);
    assert_eq!(
        object_policy(attributes, tag, true),
        Err("is a link or reparse point")
    );
    // Compress a helper as `compact /exe:xpress4k` does: WOF_EXTERNAL_INFO { version 1,
    // WOF_PROVIDER_FILE } and FILE_PROVIDER_EXTERNAL_INFO_V1 { version 1, XPRESS4K, 0 }.
    let helper = root.path().join("compressed.exe");
    std::fs::write(&helper, vec![0; 64 * 1024]).unwrap();
    let writable = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&helper)
        .unwrap();
    match control(
        &writable,
        FSCTL_SET_EXTERNAL_BACKING,
        &[1, WOF_PROVIDER_FILE, 1, 0, 0],
        &mut [],
    ) {
        // ERROR_INVALID_FUNCTION or ERROR_NOT_SUPPORTED: no WOF on this temporary volume.
        Err(error) if matches!(error.raw_os_error(), Some(1 | 50)) => {
            eprintln!("WOF unavailable for the temporary directory: {error}");
            return;
        }
        result => result.unwrap(),
    }
    let mut backing = [0u32; 5];
    control(&writable, FSCTL_GET_EXTERNAL_BACKING, &[], &mut backing).unwrap();
    assert_eq!(backing[1], WOF_PROVIDER_FILE);
    drop(writable);
    // The filter presents such a file as compressed or exposes its WOF tag, depending on the
    // Windows version. Either way it is an ordinary file: only its owner (the current user)
    // refuses it here, not a reparse point.
    let (attributes, tag) = attribute_tag(&pin_file(&helper).unwrap()).unwrap();
    assert!(tag == 0 || tag == IO_REPARSE_TAG_WOF, "{tag:#x}");
    assert_eq!(object_policy(attributes, tag, false), Ok(()));
    let message = pin_helper(&helper.canonicalize().unwrap(), None)
        .unwrap_err()
        .to_string();
    assert!(message.contains("is not owned by SYSTEM"), "{message}");
}

#[test]
fn a_shipped_digest_trusts_the_current_user_but_no_other_account() {
    let user = "S-1-5-21-1000-2000-3000-1001";
    let other = "S-1-5-21-1000-2000-3000-1002";
    let own = format!("O:{user}D:P(A;;FA;;;SY)(A;;FA;;;BA)(A;;FA;;;{user})");
    // A per-user installation: the user owns and may change it, administrators too.
    assert_eq!(evaluate_for(&own, MODIFY_RIGHTS, Some(user)), Ok(()));
    let refused = evaluate(&own, MODIFY_RIGHTS).unwrap_err();
    assert!(refused.contains("is not owned by SYSTEM"), "{refused}");
    for (sddl, reason) in [
        // Another user, Users, Authenticated Users or Everyone may change it.
        (format!("O:{other}D:P(A;;FA;;;{user})"), "owned by you"),
        (
            format!("O:{user}D:P(A;;FA;;;{user})(A;;FA;;;{other})"),
            "another",
        ),
        (
            format!("O:{user}D:P(A;;FA;;;{user})(A;;0x2;;;BU)"),
            "another",
        ),
        (format!("O:{user}D:P(A;ID;0x1301bf;;;AU)"), "another"),
        (format!("O:{user}D:P(A;;0x40000;;;WD)"), "another"),
    ] {
        let refused = evaluate_for(&sddl, MODIFY_RIGHTS, Some(user)).unwrap_err();
        assert!(refused.contains(reason), "{sddl}: {refused}");
    }
}

/// The current user's SID string.
fn current_user_sid() -> String {
    let user = CurrentUser::query().unwrap();
    let mut text = ptr::null_mut();
    // SAFETY: a valid SID and a writable output; LocalAlloc owns the result.
    assert_ne!(unsafe { ConvertSidToStringSidW(user.sid(), &mut text) }, 0);
    let owned = Descriptor(text.cast());
    // SAFETY: a terminated UTF-16 string that `owned` keeps alive.
    let length = (0..).take_while(|&i| unsafe { *text.add(i) } != 0).count();
    let sid = String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) }).unwrap();
    drop(owned);
    sid
}

/// Create a test folder whose own access list is `sddl`, inherited by its entries.
fn folder_with(path: &Path, sddl: &str) {
    let text = wide(sddl);
    let mut raw = ptr::null_mut();
    // SAFETY: a terminated SDDL string and a writable output; LocalAlloc owns the result.
    assert_ne!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                1,
                &mut raw,
                ptr::null_mut(),
            )
        },
        0,
        "{sddl}"
    );
    let descriptor = Descriptor(raw);
    let attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor.0,
        bInheritHandle: 0,
    };
    let name: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    // SAFETY: a terminated path and a descriptor that lives through the call.
    assert_ne!(
        unsafe { CreateDirectoryW(name.as_ptr(), &attributes) },
        0,
        "{}: {}",
        path.display(),
        io::Error::last_os_error()
    );
}

fn digest(bytes: &[u8]) -> HelperDigest {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}

/// A per-user installation in the build directory: a folder that only the current user,
/// SYSTEM and Administrators can change, and the same folder with write access for
/// Authenticated Users (both the test's own folders, removed with the temporary directory).
#[test]
fn shipped_helper_is_checked_by_content_through_its_pin() {
    let user = current_user_sid();
    let own = format!("O:{user}D:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)(A;OICI;FA;;;{user})");
    let root = tempfile::tempdir().unwrap();
    let folder = root.path().join("own");
    folder_with(&folder, &own);
    let helper = folder.join("midnight-elevated-listing.exe");
    std::fs::write(&helper, b"shipped helper").unwrap();
    let shipped = digest(b"shipped helper");
    let current = CurrentUser::query().unwrap();
    let sid = Some(current.sid());
    let requirement = requirement(true);

    // The folder and the helper: the user's own, accepted with the digest only.
    let folder_pin = pin_folder(&folder).unwrap();
    assert_eq!(check_security(&folder_pin, MODIFY_RIGHTS, sid), Ok(()));
    assert!(check_security(&folder_pin, MODIFY_RIGHTS, None).is_err());
    let pin = pin_file(&helper).unwrap();
    assert_eq!(check_security(&pin, MODIFY_RIGHTS, sid), Ok(()));
    crate::verify_shipped(&mut &pin, &helper, &shipped, requirement).unwrap();
    // While pinned, the verified helper can be neither rewritten nor replaced.
    let refusal = |result: io::Result<()>| result.unwrap_err().raw_os_error();
    assert_eq!(
        refusal(OpenOptions::new().write(true).open(&helper).map(drop)),
        Some(ERROR_SHARING_VIOLATION)
    );
    assert_eq!(
        refusal(std::fs::rename(&helper, folder.join("replaced.exe"))),
        Some(ERROR_SHARING_VIOLATION)
    );
    drop(pin);
    drop(folder_pin);

    // A modified copy is refused by its content.
    std::fs::write(&helper, b"modified helper").unwrap();
    let error = crate::verify_shipped(
        &mut &pin_file(&helper).unwrap(),
        &helper,
        &shipped,
        requirement,
    )
    .unwrap_err();
    let message = error.to_string();
    assert!(message.starts_with(requirement), "{message}");
    assert!(
        message.contains("is not the authorization helper"),
        "{message}"
    );
    std::fs::write(&helper, b"shipped helper").unwrap();

    // Another account may write: refused although the user owns it.
    let shared = root.path().join("shared");
    folder_with(&shared, &format!("{own}(A;OICI;0x1301bf;;;AU)"));
    let refused = check_security(&pin_folder(&shared).unwrap(), MODIFY_RIGHTS, sid).unwrap_err();
    assert!(refused.contains("another non-administrative"), "{refused}");

    // A link in the helper's path: refused from its own attributes, after the helper itself
    // (reached through the junction) passed its owner, access and content checks.
    let junction = root.path().join("link");
    super::super::tests::create_junction(&junction, &folder).unwrap();
    let message = pin_helper(
        &junction.join("midnight-elevated-listing.exe"),
        Some(&shipped),
    )
    .unwrap_err()
    .to_string();
    assert!(message.contains("is a link or reparse point"), "{message}");
    assert!(
        message.contains(&junction.display().to_string()),
        "{message}"
    );
}
