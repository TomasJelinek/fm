//! Replacing an existing output (Shift+F4's empty file, a saved checksum manifest or file
//! list) keeps the original's metadata. Its attributes decide only how the replacement is
//! stored, never whether the confirmed replacement happens, except for files whose
//! contents may not be on this computer and for links.
use super::*;
use std::{fs, os::windows::fs::OpenOptionsExt};
use windows_sys::Win32::System::{
    IO::DeviceIoControl,
    Ioctl::{FSCTL_SET_COMPRESSION, FSCTL_SET_SPARSE},
};

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain(Some(0)).collect()
}
fn attributes(path: &Path) -> u32 {
    fs::metadata(path).unwrap().file_attributes()
}
fn set_attributes(path: &Path, attributes: u32) {
    // SAFETY: a terminated path of an owned fixture.
    assert_ne!(
        unsafe { SetFileAttributesW(wide(path).as_ptr(), attributes) },
        0
    );
}
/// Compresses or decompresses a fixture file or folder (a folder's new files inherit it).
fn set_compression(path: &Path, compressed: bool) {
    let file = fs::OpenOptions::new()
        .access_mode(FILE_READ_DATA | FILE_WRITE_DATA | FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap();
    let format: u16 = if compressed { 1 } else { 0 };
    let mut returned = 0;
    // SAFETY: live handle and fixed-size input for a synchronous control call.
    assert_ne!(
        unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                FSCTL_SET_COMPRESSION,
                (&format as *const u16).cast(),
                2,
                ptr::null_mut(),
                0,
                &mut returned,
                ptr::null_mut(),
            )
        },
        0,
        "{}",
        io::Error::last_os_error()
    );
}
fn folder(path: &Path) -> Directory {
    Directory::from_file(native::open_parent(path).unwrap(), path.to_owned()).unwrap()
}

/// Replaces `name` in `parent` with `contents` as a confirmed replacement does, and
/// returns where the original is kept.
fn replace(parent: &Path, name: &str, contents: &[u8]) -> Result<PathBuf, FsError> {
    let directory = folder(parent);
    let cancel = Cancellation::default();
    let expected = prepare_target(&directory, OsStr::new(name), &cancel)
        .unwrap()
        .unwrap();
    let mut stage = directory.stage_file_preserving_target(
        OsStr::new(name),
        expected,
        TransferReplacementPolicy::RecoverableCurrentEntry,
        &cancel,
    )?;
    stage.write_all(contents).unwrap();
    stage.sync(&cancel)?;
    match stage.publish_replace(OsStr::new(name), &cancel)? {
        TransferPublication::Replaced { path, recovery } => {
            assert_eq!(path, parent.join(name));
            Ok(recovery.path)
        }
        other => panic!("not replaced: {other:?}"),
    }
}

/// Only the original and, after a replacement, its recovery folder: no stage is left.
fn assert_no_stage(parent: &Path) {
    for entry in fs::read_dir(parent).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(!name.to_string_lossy().starts_with(".me-"), "{name:?} left");
    }
}

/// The owner's case: Shift+F4 over a file in a folder compressed with NTFS compression.
#[test]
fn a_compressed_file_in_a_compressed_folder_is_replaced_and_stays_compressed() {
    let temp = crate::test_fixtures::build_temp("preservation-");
    let parent = temp.path().join("compressed");
    fs::create_dir(&parent).unwrap();
    set_compression(&parent, true);
    let target = parent.join("eee");
    fs::write(&target, b"original contents").unwrap();
    assert_ne!(attributes(&target) & FILE_ATTRIBUTE_COMPRESSED, 0);
    let recovery = replace(&parent, "eee", b"").unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"");
    assert_ne!(attributes(&target) & FILE_ATTRIBUTE_COMPRESSED, 0);
    assert_eq!(fs::read(recovery).unwrap(), b"original contents");
    assert_no_stage(&parent);
}

/// Compression is the original's, not the folder's: a compressed file in a plain folder
/// stays compressed, and a plain file in a compressed folder stays plain.
#[test]
fn the_replacement_is_compressed_exactly_when_the_original_was() {
    let temp = crate::test_fixtures::build_temp("preservation-");
    for (folder_compressed, original_compressed) in [(false, true), (true, false)] {
        let parent = temp
            .path()
            .join(format!("folder-{folder_compressed}-{original_compressed}"));
        fs::create_dir(&parent).unwrap();
        let target = parent.join("data.bin");
        fs::write(&target, vec![7; 300_000]).unwrap();
        set_compression(&target, original_compressed);
        set_compression(&parent, folder_compressed);
        replace(&parent, "data.bin", &[9; 200_000]).unwrap();
        assert_eq!(fs::read(&target).unwrap(), vec![9; 200_000]);
        assert_eq!(
            attributes(&target) & FILE_ATTRIBUTE_COMPRESSED != 0,
            original_compressed,
            "folder compressed: {folder_compressed}"
        );
        assert_no_stage(&parent);
    }
}

/// The user confirmed the replacement of this very file, so its read-only attribute is
/// no reason to refuse; the replacement keeps it, with the original's other attributes.
#[test]
fn read_only_hidden_system_and_unindexed_originals_are_replaced_and_keep_their_attributes() {
    let temp = crate::test_fixtures::build_temp("preservation-");
    for kept in [
        FILE_ATTRIBUTE_READONLY,
        FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM,
        FILE_ATTRIBUTE_NOT_CONTENT_INDEXED | FILE_ATTRIBUTE_READONLY | FILE_ATTRIBUTE_ARCHIVE,
    ] {
        let parent = temp.path().join(format!("attributes-{kept:x}"));
        fs::create_dir(&parent).unwrap();
        let target = parent.join("kept.txt");
        fs::write(&target, b"original").unwrap();
        set_attributes(&target, kept);
        let recovery = replace(&parent, "kept.txt", b"replacement").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"replacement");
        // Windows marks the renamed replacement for backup (Archive), as any changed file.
        let shown = !(FILE_ATTRIBUTE_NORMAL | FILE_ATTRIBUTE_ARCHIVE);
        assert_eq!(attributes(&target) & shown, kept & shown);
        assert_eq!(fs::read(&recovery).unwrap(), b"original");
        set_attributes(&target, FILE_ATTRIBUTE_NORMAL);
        set_attributes(&recovery, FILE_ATTRIBUTE_NORMAL);
        assert_no_stage(&parent);
    }
}

/// A replacement given up after its stage took the original's read-only attribute still
/// removes that stage, and the original stays as it was.
#[test]
fn an_abandoned_read_only_replacement_removes_its_stage() {
    let temp = crate::test_fixtures::build_temp("preservation-");
    let target = temp.path().join("locked.txt");
    fs::write(&target, b"original").unwrap();
    set_attributes(&target, FILE_ATTRIBUTE_READONLY);
    let directory = folder(temp.path());
    let cancel = Cancellation::default();
    let expected = prepare_target(&directory, OsStr::new("locked.txt"), &cancel)
        .unwrap()
        .unwrap();
    let mut stage = directory
        .stage_file_preserving_target(
            OsStr::new("locked.txt"),
            expected,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel,
        )
        .unwrap();
    stage.write_all(b"never published").unwrap();
    stage.sync(&cancel).unwrap();
    stage.cleanup().unwrap();
    drop(stage);
    assert_no_stage(temp.path());
    assert_eq!(fs::read(&target).unwrap(), b"original");
    set_attributes(&target, FILE_ATTRIBUTE_NORMAL);
}

/// A sparse original is replaced; its replacement is written as ordinary data.
#[test]
fn a_sparse_original_is_replaced() {
    let temp = crate::test_fixtures::build_temp("preservation-");
    let target = temp.path().join("sparse.img");
    let file = fs::File::create(&target).unwrap();
    let mut returned = 0;
    // SAFETY: a live writable handle; FSCTL_SET_SPARSE takes no input (it sets the flag).
    assert_ne!(
        unsafe {
            DeviceIoControl(
                file.as_raw_handle(),
                FSCTL_SET_SPARSE,
                ptr::null(),
                0,
                ptr::null_mut(),
                0,
                &mut returned,
                ptr::null_mut(),
            )
        },
        0
    );
    file.set_len(1 << 20).unwrap();
    drop(file);
    assert_ne!(attributes(&target) & FILE_ATTRIBUTE_SPARSE_FILE, 0);
    replace(temp.path(), "sparse.img", b"dense").unwrap();
    assert_eq!(fs::read(&target).unwrap(), b"dense");
    assert_no_stage(temp.path());
}

/// An original encrypted with EFS gets an encrypted replacement, in an encrypted folder or
/// not. Skipped where this account cannot encrypt files.
#[test]
fn an_encrypted_original_gets_an_encrypted_replacement() {
    let temp = crate::test_fixtures::build_temp("preservation-");
    for folder_encrypted in [false, true] {
        let parent = temp.path().join(format!("efs-{folder_encrypted}"));
        fs::create_dir(&parent).unwrap();
        let target = parent.join("secret.txt");
        fs::write(&target, b"original secret").unwrap();
        // SAFETY: terminated paths of owned fixtures.
        if unsafe { EncryptFileW(wide(&target).as_ptr()) } == 0 {
            eprintln!(
                "skipped: EFS is unavailable here ({})",
                io::Error::last_os_error()
            );
            return;
        }
        // SAFETY: as above.
        if folder_encrypted && unsafe { EncryptFileW(wide(&parent).as_ptr()) } == 0 {
            eprintln!(
                "skipped: EFS is unavailable here ({})",
                io::Error::last_os_error()
            );
            return;
        }
        let recovery = replace(&parent, "secret.txt", b"new secret").unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new secret");
        assert_ne!(attributes(&target) & FILE_ATTRIBUTE_ENCRYPTED, 0);
        assert_eq!(fs::read(recovery).unwrap(), b"original secret");
        assert_no_stage(&parent);
    }
}

/// A file whose contents may not be on this computer (offline, or recalled on access as
/// cloud files are) is not replaced: the replacement would never download it, but the
/// original would only be kept as a placeholder. The reason names it, and nothing changes.
#[test]
fn an_offline_original_is_refused_with_its_reason() {
    let temp = crate::test_fixtures::build_temp("preservation-");
    let target = temp.path().join("cloud.docx");
    fs::write(&target, b"original").unwrap();
    set_attributes(&target, FILE_ATTRIBUTE_OFFLINE);
    let refused = replace(temp.path(), "cloud.docx", b"replacement").unwrap_err();
    assert_eq!(refused.kind, io::ErrorKind::Unsupported);
    assert!(
        refused.message.contains("may not be on this computer"),
        "{}",
        refused.message
    );
    set_attributes(&target, FILE_ATTRIBUTE_NORMAL);
    assert_eq!(fs::read(&target).unwrap(), b"original");
    assert_no_stage(temp.path());
}

/// A Copy's Replace of a compressed, read-only file (it keeps no metadata of the original)
/// works too, and leaves the original in recovery.
#[test]
fn a_copy_replaces_a_compressed_read_only_file() {
    let temp = crate::test_fixtures::build_temp("preservation-");
    let parent = temp.path().join("compressed");
    fs::create_dir(&parent).unwrap();
    set_compression(&parent, true);
    let target = parent.join("report.txt");
    fs::write(&target, b"original").unwrap();
    set_attributes(&target, FILE_ATTRIBUTE_READONLY);
    let directory = folder(&parent);
    let cancel = Cancellation::default();
    let expected = prepare_target(&directory, OsStr::new("report.txt"), &cancel)
        .unwrap()
        .unwrap();
    let mut stage = directory
        .stage_file_for_target(
            OsStr::new("report.txt"),
            expected,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel,
        )
        .unwrap();
    stage.write_all(b"copied").unwrap();
    stage.sync(&cancel).unwrap();
    let TransferPublication::Replaced { recovery, .. } = stage
        .publish_replace(OsStr::new("report.txt"), &cancel)
        .unwrap()
    else {
        panic!("not replaced");
    };
    assert_eq!(fs::read(&target).unwrap(), b"copied");
    assert_ne!(attributes(&target) & FILE_ATTRIBUTE_COMPRESSED, 0);
    set_attributes(&recovery.path, FILE_ATTRIBUTE_NORMAL);
    assert_eq!(fs::read(&recovery.path).unwrap(), b"original");
    assert_no_stage(&parent);
}
