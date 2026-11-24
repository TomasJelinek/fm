//! The workers' grant on a file that inherits only its folder's permissions, as a release ZIP
//! extracted into the user's profile leaves its files.
use super::super::worker_may_load;
use crate::{LoadGrant, grant_worker_load, prepare_worker};
use std::{
    cell::Cell,
    io,
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::{Path, PathBuf},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::{
        Authorization::{
            ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
            ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
            SE_FILE_OBJECT, SetNamedSecurityInfoW,
        },
        DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, GetTokenInformation,
        PROTECTED_DACL_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};

const WORKERS: &str =
    "S-1-15-2-210492624-3469538162-618285203-1868907-2672626540-1166629363-1118814998";

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

/// This user's SID in string form.
fn user_sid() -> String {
    let mut token = null_mut();
    // SAFETY: this process's pseudo handle; the token is owned below on success.
    assert_ne!(
        unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) },
        0
    );
    // SAFETY: a token handle that the call above returned.
    let _token = unsafe { std::os::windows::io::OwnedHandle::from_raw_handle(token) };
    let mut buffer = vec![0u64; 64];
    let mut length = 0;
    // SAFETY: a live token and an 8-byte aligned writable buffer of the given size.
    assert_ne!(
        unsafe {
            GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                (buffer.len() * 8) as u32,
                &mut length,
            )
        },
        0
    );
    // SAFETY: TokenUser filled the buffer with a TOKEN_USER whose SID lies inside it.
    let sid = unsafe { (*buffer.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    sid_string(sid)
}

fn sid_string(sid: *mut core::ffi::c_void) -> String {
    let mut text = null_mut();
    // SAFETY: a valid SID; Windows allocates the terminated string.
    assert_ne!(unsafe { ConvertSidToStringSidW(sid, &mut text) }, 0);
    // SAFETY: the string is terminated and freed once below.
    unsafe {
        let length = (0..).take_while(|&index| *text.add(index) != 0).count();
        let value = String::from_utf16(std::slice::from_raw_parts(text, length)).unwrap();
        LocalFree(text.cast());
        value
    }
}

/// Give `path` the protected DACL `sddl`.
fn set_dacl(path: &Path, sddl: &str) {
    let text: Vec<u16> = sddl.encode_utf16().chain([0]).collect();
    let mut descriptor = null_mut();
    // SAFETY: a terminated SDDL string; Windows allocates the descriptor.
    assert_ne!(
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                1,
                &mut descriptor,
                null_mut(),
            )
        },
        0
    );
    let (mut present, mut defaulted, mut acl) = (0, 0, null_mut());
    // SAFETY: a live descriptor and writable outputs.
    assert_ne!(
        unsafe { GetSecurityDescriptorDacl(descriptor, &mut present, &mut acl, &mut defaulted) },
        0
    );
    let name = wide(path);
    // SAFETY: a terminated path and the descriptor's ACL, live for the call.
    let status = unsafe {
        SetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            acl,
            null(),
        )
    };
    // SAFETY: allocated by Windows for this caller.
    unsafe { LocalFree(descriptor) };
    assert_eq!(status, 0);
}

/// `path`'s DACL in SDDL form, with its control flags (`D:P` protected, `D:AI` inheriting).
fn dacl(path: &Path) -> String {
    let name = wide(path);
    let mut descriptor = null_mut();
    // SAFETY: a terminated path; Windows allocates the descriptor.
    let status = unsafe {
        GetNamedSecurityInfoW(
            name.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            null_mut(),
            null_mut(),
            null_mut(),
            null_mut(),
            &mut descriptor,
        )
    };
    assert_eq!(status, 0);
    let (mut text, mut length) = (null_mut(), 0);
    // SAFETY: a live descriptor; Windows allocates the terminated string and its length.
    assert_ne!(
        unsafe {
            ConvertSecurityDescriptorToStringSecurityDescriptorW(
                descriptor,
                1,
                DACL_SECURITY_INFORMATION,
                &mut text,
                &mut length,
            )
        },
        0
    );
    // SAFETY: `length` units including the terminator were written at `text`; both
    // allocations are freed once.
    unsafe {
        let units = std::slice::from_raw_parts(text, length.saturating_sub(1) as usize);
        let value = String::from_utf16(units).unwrap();
        LocalFree(text.cast());
        LocalFree(descriptor);
        value.trim_end_matches('\0').to_owned()
    }
}

/// The entries of an SDDL DACL, without its control flags.
fn entries(dacl: &str) -> &str {
    &dacl[dacl.find('(').unwrap_or(dacl.len())..]
}

/// A folder like one in the user's profile: the user, SYSTEM and Administrators with full
/// control, inherited by everything below it, and nothing for AppContainers.
fn profile_like() -> tempfile::TempDir {
    let folder = tempfile::tempdir().unwrap();
    let user = user_sid();
    set_dacl(
        folder.path(),
        &format!("D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)"),
    );
    folder
}

/// A file extracted into `folder`: it inherits the folder's entries and has none of its own.
fn extracted(folder: &Path, name: &str) -> PathBuf {
    let path = folder.join(name);
    std::fs::write(&path, b"library").unwrap();
    path
}

#[test]
fn the_workers_get_one_read_and_execute_entry_on_an_extracted_file_and_nothing_else() {
    let folder = profile_like();
    let library = extracted(folder.path(), "avcodec-63.dll");
    let before = dacl(&library);
    assert!(before.starts_with("D:AI(A;ID;FA;;;"), "{before}");
    assert!(!before.contains(WORKERS), "{before}");
    assert!(!worker_may_load(&library).unwrap());

    assert_eq!(
        grant_worker_load(&library, |_| true).unwrap(),
        LoadGrant::Added
    );
    assert!(worker_may_load(&library).unwrap());
    // Exactly one explicit entry was added, before the inherited ones: read and execute
    // (0x1200A9) for the workers' SID, without inheritance flags. Everything else is as it
    // was, inheritance included.
    let after = dacl(&library);
    assert_eq!(
        after,
        before.replacen("D:AI", &format!("D:AI(A;;0x1200a9;;;{WORKERS})"), 1)
    );
    // The folder is never changed.
    assert!(!dacl(folder.path()).contains(WORKERS));

    // Again: nothing to do and nothing changed.
    assert_eq!(
        grant_worker_load(&library, |_| panic!("not asked")).unwrap(),
        LoadGrant::Present
    );
    assert_eq!(dacl(&library), after);
}

#[test]
fn a_file_with_its_own_protected_permissions_keeps_them_and_their_protection() {
    let folder = profile_like();
    let library = extracted(folder.path(), "avutil-61.dll");
    let user = user_sid();
    set_dacl(&library, &format!("D:P(A;;FA;;;{user})(A;;FA;;;SY)"));
    let before = dacl(&library);
    assert_eq!(
        grant_worker_load(&library, |_| true).unwrap(),
        LoadGrant::Added
    );
    // The entry joins the explicit ones, and the DACL stays protected. (Windows marks the
    // DACL as auto-inherited, `AI`, as `Set-Acl` does; nothing is inherited while it is
    // protected.)
    let after = dacl(&library);
    let entry = format!("(A;;0x1200a9;;;{WORKERS})");
    assert_eq!(after.matches(&entry).count(), 1, "{after}");
    assert_eq!(entries(&after.replacen(&entry, "", 1)), entries(&before));
    assert!(after.starts_with("D:P"), "{after}");
    assert!(worker_may_load(&library).unwrap());
}

#[test]
fn a_file_the_caller_may_not_change_keeps_its_permissions() {
    let folder = profile_like();
    let library = extracted(folder.path(), "swscale-10.dll");
    let before = dacl(&library);
    let asked = Cell::new(Vec::new());
    let refuse = |path: &Path| {
        let mut list = asked.take();
        list.push(path.to_owned());
        asked.set(list);
        false
    };
    assert_eq!(
        grant_worker_load(&library, refuse).unwrap(),
        LoadGrant::Refused
    );
    assert_eq!(asked.take(), std::slice::from_ref(&library));
    assert_eq!(dacl(&library), before);
    assert!(!worker_may_load(&library).unwrap());
}

#[test]
fn a_file_whose_permissions_the_user_may_not_change_is_reported_and_unchanged() {
    let folder = profile_like();
    let library = extracted(folder.path(), "swresample-7.dll");
    let user = user_sid();
    // Read and execute only, like a file of an administrator's installation that does not
    // grant AppContainers: neither the user, as its owner (OWNER RIGHTS replaces the owner's
    // implicit right to change permissions), nor an elevated administrator may change its
    // permissions. The folder still lets the user delete it.
    set_dacl(
        &library,
        &format!("D:P(A;;0x1200a9;;;{user})(A;;0x1200a9;;;OW)"),
    );
    let before = dacl(&library);
    let error = grant_worker_load(&library, |_| true).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
    assert_eq!(dacl(&library), before);
}

#[test]
fn a_prepared_worker_and_its_loadable_files_are_not_checked_again_in_the_run() {
    let folder = profile_like();
    let worker = folder.path().join("helper.exe");
    let library = extracted(folder.path(), "avformat-63.dll");
    let other = extracted(folder.path(), "other.dll");
    let listed = &Cell::new(0);
    let asked = &Cell::new(0);
    let files = || {
        listed.set(listed.get() + 1);
        vec![library.clone(), other.clone()]
    };
    let ask = |allow: bool| {
        move |_: &Path| {
            asked.set(asked.get() + 1);
            allow
        }
    };
    // A shared installation: both files are refused, and nothing changes.
    let before = dacl(&library);
    assert!(prepare_worker(&worker, files, ask(false)).is_err());
    assert_eq!((listed.get(), asked.get()), (1, 2));
    assert_eq!(dacl(&library), before);
    // Refusals are not kept: securing the installation takes effect at the next launch.
    prepare_worker(&worker, files, ask(true)).unwrap();
    assert_eq!((listed.get(), asked.get()), (2, 4));
    assert!(worker_may_load(&library).unwrap() && worker_may_load(&other).unwrap());
    // Prepared: later launches neither list nor check the files.
    prepare_worker(&worker, files, ask(true)).unwrap();
    assert_eq!((listed.get(), asked.get()), (2, 4));
    // Another worker that loads a file already found loadable checks only its own files.
    let second = folder.path().join("second.exe");
    let own = extracted(folder.path(), "own.dll");
    prepare_worker(&second, || vec![library.clone(), own.clone()], ask(true)).unwrap();
    assert_eq!(asked.get(), 5);
}
