use super::*;
use crate::test_fixtures::{build_temp, deny_to_everyone};
use std::{
    fs::{self, OpenOptions},
    os::windows::fs::OpenOptionsExt,
};
use windows_sys::Win32::Foundation::{
    ERROR_DISK_FULL, ERROR_IO_DEVICE, ERROR_SHARING_VIOLATION, ERROR_UNEXP_NET_ERR,
};

/// A folder handle as a Move pins one: attributes and traversal only, sharing reading and
/// writing.
fn pinned(path: &Path) -> File {
    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES | FILE_TRAVERSE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap()
}

fn os(code: u32) -> io::Error {
    io::Error::from_raw_os_error(code as i32)
}

#[test]
fn a_folder_pinned_only_to_read_is_flushed_through_a_new_handle_that_may_add_to_it() {
    let temp = build_temp("folder-flush-");
    let path = temp.path().join("folder 文档");
    fs::create_dir(&path).unwrap();
    let folder = pinned(&path);
    // Why the folder is reopened: Windows refuses to flush it through the pin itself.
    assert_eq!(
        folder.sync_all().unwrap_err().raw_os_error(),
        Some(ERROR_ACCESS_DENIED as i32)
    );
    assert_eq!(save(&folder), Saved::Flushed);
    let id = identity(&folder, &path).unwrap();
    assert!(saved(&[(&folder, id), (&folder, id)]));
}

#[test]
fn a_folder_is_flushed_through_either_add_right_and_stays_unsaved_without_both() {
    let temp = build_temp("folder-flush-denied-");
    // Neither right is needed to remove the fixture, so its DACL is not restored.
    for (name, denied, expected) in [
        ("files", FILE_ADD_FILE, Saved::Flushed),
        ("subfolders", FILE_ADD_SUBDIRECTORY, Saved::Flushed),
        ("both", FILE_ADD_FILE | FILE_ADD_SUBDIRECTORY, Saved::No),
    ] {
        let path = temp.path().join(name);
        fs::create_dir(&path).unwrap();
        let folder = pinned(&path);
        deny_to_everyone(&path, denied).unwrap();
        assert_eq!(save(&folder), expected, "{name}");
        let id = identity(&folder, &path).unwrap();
        assert_eq!(saved(&[(&folder, id)]), expected != Saved::No, "{name}");
    }
}

#[test]
fn a_folder_that_another_program_holds_without_write_sharing_is_not_saved() {
    let temp = build_temp("folder-flush-held-");
    let path = temp.path().join("held");
    fs::create_dir(&path).unwrap();
    let folder = pinned(&path);
    let other = OpenOptions::new()
        .access_mode(FILE_LIST_DIRECTORY)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(&path)
        .unwrap();
    assert_eq!(
        reopen(&folder).unwrap_err().raw_os_error(),
        Some(ERROR_SHARING_VIOLATION as i32)
    );
    assert_eq!(save(&folder), Saved::No);
    let id = identity(&folder, &path).unwrap();
    let free = temp.path().join("free");
    fs::create_dir(&free).unwrap();
    let free_folder = pinned(&free);
    let free_id = identity(&free_folder, &free).unwrap();
    // One unsaved folder leaves the whole set unsaved, and the other is still flushed.
    assert!(!saved(&[(&folder, id), (&free_folder, free_id)]));
    drop(other);
    assert!(saved(&[(&folder, id), (&free_folder, free_id)]));
}

#[test]
fn only_an_unsupported_folder_flush_counts_as_saved_without_a_flush() {
    assert_eq!(after_flush(Ok(())), Saved::Flushed);
    for code in [
        ERROR_INVALID_FUNCTION,
        ERROR_NOT_SUPPORTED,
        ERROR_INVALID_PARAMETER,
        ERROR_ACCESS_DENIED,
    ] {
        assert_eq!(after_flush(Err(os(code))), Saved::Unsupported, "{code}");
    }
    for code in [ERROR_IO_DEVICE, ERROR_DISK_FULL, ERROR_UNEXP_NET_ERR] {
        assert_eq!(after_flush(Err(os(code))), Saved::No, "{code}");
    }
    assert_eq!(after_flush(Err(io::Error::other("no code"))), Saved::No);
    // Opening: only a filesystem without such opens is unsupported; an account that may
    // add nothing, or a program denying write sharing, leaves the folder unsaved.
    for code in [ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED] {
        assert_eq!(after_open(&os(code)), Saved::Unsupported, "{code}");
    }
    for code in [
        ERROR_ACCESS_DENIED,
        ERROR_SHARING_VIOLATION,
        ERROR_IO_DEVICE,
    ] {
        assert_eq!(after_open(&os(code)), Saved::No, "{code}");
    }
}
