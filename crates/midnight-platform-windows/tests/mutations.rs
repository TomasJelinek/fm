#![cfg(windows)]

use midnight_core::{
    Cancellation, DeleteCompletion, DeleteMode, FsError, MutationService, PreparedMutation,
    RenameCompletion,
};
use midnight_platform_windows::WindowsFileSystem;
use std::{
    ffi::OsStr,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

/// The destination of a rename; Windows never leaves staging behind.
fn renamed_cleanly(result: Result<RenameCompletion, FsError>) -> PathBuf {
    let completion = result.unwrap();
    assert!(completion.residue.is_none(), "{:?}", completion.residue);
    completion.destination
}

/// Asserts a completed deletion; Windows never leaves staging behind.
fn deleted_cleanly(result: Result<DeleteCompletion, FsError>) {
    let completion = result.unwrap();
    assert!(completion.residue.is_none(), "{:?}", completion.residue);
}

fn fixture() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("midnight-mutation-")
        .tempdir_in(root)
        .unwrap()
}

fn prepare(path: &Path) -> Arc<dyn PreparedMutation> {
    WindowsFileSystem::new(None)
        .prepare(path, &Cancellation::default())
        .unwrap()
}

#[test]
fn unicode_rename_preserves_bytes_and_refuses_destination_collision() {
    let temp = fixture();
    let source = temp.path().join("文档 🦀.txt");
    let collision = temp.path().join("existing.txt");
    fs::write(&source, b"original").unwrap();
    fs::write(&collision, b"existing").unwrap();
    let item = prepare(&source);
    assert_eq!(item.item().metadata.size, Some(8));
    assert!(
        item.rename(OsStr::new("existing.txt"), &Cancellation::default())
            .is_err()
    );
    assert_eq!(fs::read(&source).unwrap(), b"original");
    assert_eq!(fs::read(&collision).unwrap(), b"existing");
    let renamed = renamed_cleanly(item.rename(
        OsStr::new("e\u{301} renamed 🦀.txt"),
        &Cancellation::default(),
    ));
    assert_eq!(renamed, temp.path().join("e\u{301} renamed 🦀.txt"));
    assert_eq!(fs::read(renamed).unwrap(), b"original");
    assert!(!source.exists());
}

#[test]
fn case_only_rename_changes_the_actual_directory_entry_spelling() {
    let temp = fixture();
    let source = temp.path().join("Case.txt");
    fs::write(&source, b"payload").unwrap();
    renamed_cleanly(prepare(&source).rename(OsStr::new("CASE.txt"), &Cancellation::default()));
    let names: Vec<_> = fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names, vec![OsStr::new("CASE.txt")]);
}

#[test]
fn invalid_names_do_not_mutate_the_source() {
    let temp = fixture();
    let source = temp.path().join("source.txt");
    fs::write(&source, b"payload").unwrap();
    let item = prepare(&source);
    for name in [
        "",
        ".",
        "..",
        "../escape",
        "a\\b",
        "C:escape",
        "stream:ads",
        "a?",
        "a*",
        "a<",
        "a>",
        "a|",
        "a\"",
        "a\0b",
        "a\nb",
        "trailing.",
        "trailing ",
        "CON",
        "con.txt",
        "PRN",
        "AUX",
        "NUL",
        "COM1",
        "lpt9.txt",
        "COM¹",
        "LPT².txt",
        "CONIN$",
        "CONOUT$",
    ] {
        assert_eq!(
            item.rename(OsStr::new(name), &Cancellation::default())
                .unwrap_err()
                .kind,
            io::ErrorKind::InvalidInput,
            "{name:?}"
        );
        assert_eq!(fs::read(&source).unwrap(), b"payload");
    }
}

#[test]
fn roots_parent_components_and_special_namespaces_are_rejected() {
    let service = WindowsFileSystem::new(None);
    for path in [
        "C:\\",
        "C:\\Windows\\..",
        "C:\\Windows\\.",
        "\\\\server\\share",
        "\\\\.\\PhysicalDrive0",
        "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume1",
        "C:\\file:stream",
        "relative.txt",
    ] {
        assert_eq!(
            service
                .prepare(Path::new(path), &Cancellation::default())
                .err()
                .unwrap()
                .kind,
            io::ErrorKind::InvalidInput,
            "{path:?}"
        );
    }
}

#[test]
fn replaced_source_is_rejected_for_rename_and_permanent_delete() {
    let temp = fixture();
    let source = temp.path().join("source");
    fs::write(&source, b"original").unwrap();
    let item = prepare(&source);
    fs::rename(&source, temp.path().join("moved")).unwrap();
    fs::write(&source, b"replacement").unwrap();
    let renamed = item
        .rename(OsStr::new("renamed"), &Cancellation::default())
        .unwrap_err();
    let deleted = item
        .delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {})
        .unwrap_err();
    // Final, as on Linux: retrying would only check the replacement again, so a
    // Delete reports it instead of offering Retry.
    assert_eq!(renamed.kind, io::ErrorKind::InvalidInput, "{renamed:?}");
    assert_eq!(deleted.kind, io::ErrorKind::InvalidInput, "{deleted:?}");
    assert_eq!(fs::read(&source).unwrap(), b"replacement");
    assert_eq!(fs::read(temp.path().join("moved")).unwrap(), b"original");
}

#[test]
fn nested_permanent_delete_reports_entries_and_honors_partial_cancellation() {
    let temp = fixture();
    let root = temp.path().join("tree");
    fs::create_dir_all(root.join("nested")).unwrap();
    for name in ["a", "b", "nested/c"] {
        fs::write(root.join(name), b"payload").unwrap();
    }
    let item = prepare(&root);
    let cancel = Cancellation::default();
    let mut visited = Vec::new();
    let error = item
        .delete(DeleteMode::Permanent, &cancel, &mut |path| {
            visited.push(path.to_owned());
            cancel.cancel();
        })
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::Interrupted);
    assert_eq!(visited.len(), 1);
    assert!(root.exists());
    deleted_cleanly(prepare(&root).delete(
        DeleteMode::Permanent,
        &Cancellation::default(),
        &mut |_| {},
    ));
    assert!(!root.exists());
}

#[test]
fn cancelled_requests_leave_the_fixture_untouched() {
    let temp = fixture();
    let path = temp.path().join("source");
    fs::write(&path, b"payload").unwrap();
    let item = prepare(&path);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        WindowsFileSystem::new(None)
            .prepare(&path, &cancel)
            .is_err()
    );
    assert_eq!(
        item.rename(OsStr::new("new"), &cancel).unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
    for mode in [DeleteMode::Permanent, DeleteMode::Trash] {
        assert_eq!(
            item.delete(mode, &cancel, &mut |_| {}).unwrap_err().kind,
            io::ErrorKind::Interrupted
        );
    }
    assert_eq!(fs::read(path).unwrap(), b"payload");
}

#[test]
fn paths_beyond_max_path_can_be_renamed_and_deleted() {
    let temp = fixture();
    let mut root = fs::canonicalize(temp.path()).unwrap();
    for _ in 0..8 {
        root.push("文档-long-path-component-0123456789");
    }
    fs::create_dir_all(&root).unwrap();
    let source = root.join("source.txt");
    fs::write(&source, b"long").unwrap();
    let renamed = renamed_cleanly(
        prepare(&source).rename(OsStr::new("renamed 🦀.txt"), &Cancellation::default()),
    );
    assert_eq!(fs::read(&renamed).unwrap(), b"long");
    deleted_cleanly(prepare(&renamed).delete(
        DeleteMode::Permanent,
        &Cancellation::default(),
        &mut |_| {},
    ));
    assert!(!renamed.exists());
}

#[test]
fn recursive_delete_removes_a_junction_without_touching_its_target() {
    let temp = fixture();
    let target = temp.path().join("outside");
    let tree = temp.path().join("tree");
    fs::create_dir(&target).unwrap();
    fs::create_dir(&tree).unwrap();
    fs::write(target.join("sentinel"), b"keep me").unwrap();
    let junction = tree.join("junction");
    create_junction(&junction, &target);
    assert_eq!(fs::read(junction.join("sentinel")).unwrap(), b"keep me");
    assert_eq!(
        prepare(&junction).item().kind,
        midnight_core::EntryKind::DirectoryLink
    );
    deleted_cleanly(prepare(&tree).delete(
        DeleteMode::Permanent,
        &Cancellation::default(),
        &mut |_| {},
    ));
    assert!(!tree.exists());
    assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"keep me");
}

#[path = "../src/test_fixtures.rs"]
mod test_fixtures;

/// A native junction (FSCTL_SET_REPARSE_POINT) instead of a PowerShell cold start.
fn create_junction(junction: &Path, target: &Path) {
    fs::create_dir(junction).unwrap();
    test_fixtures::create_junction(junction, target).unwrap();
}

#[test]
fn parent_replacement_does_not_retarget_the_prepared_entry() {
    let temp = fixture();
    let parent = temp.path().join("parent");
    let original = temp.path().join("original");
    fs::create_dir(&original).unwrap();
    create_junction(&parent, &original);
    fs::write(parent.join("source"), b"original").unwrap();
    let item = prepare(&parent.join("source"));
    fs::remove_dir(&parent).unwrap();
    fs::create_dir(&parent).unwrap();
    fs::write(parent.join("source"), b"replacement").unwrap();
    assert!(
        item.delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {})
            .is_err()
    );
    assert_eq!(fs::read(parent.join("source")).unwrap(), b"replacement");
    assert_eq!(fs::read(original.join("source")).unwrap(), b"original");
}

#[test]
fn confirmed_permanent_delete_removes_read_only_files_without_changing_other_links() {
    let temp = fixture();
    let path = temp.path().join("readonly");
    let retained = temp.path().join("retained link");
    fs::write(&path, b"retained through another name").unwrap();
    fs::hard_link(&path, &retained).unwrap();
    let original = fs::metadata(&path).unwrap().permissions();
    let mut permissions = original.clone();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();
    let result =
        prepare(&path).delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {});
    let retained_readonly = fs::metadata(&retained).unwrap().permissions().readonly();
    fs::set_permissions(&retained, original).unwrap();
    deleted_cleanly(result);
    assert!(!path.exists());
    assert!(retained_readonly);
    assert_eq!(
        fs::read(retained).unwrap(),
        b"retained through another name"
    );
}

/// The filesystem name of the volume holding `path` ("NTFS", "FAT32", "exFAT", ...).
fn filesystem_name(path: &Path) -> String {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, GetVolumeInformationByHandleW,
    };
    let directory = fs::OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap();
    let mut name = [0u16; 64];
    // SAFETY: a live directory handle and a writable bounded name buffer; the other
    // optional outputs are null.
    let succeeded = unsafe {
        GetVolumeInformationByHandleW(
            directory.as_raw_handle(),
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            name.as_mut_ptr(),
            name.len() as u32,
        )
    };
    assert_ne!(succeeded, 0, "{}", io::Error::last_os_error());
    let end = name
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(name.len());
    String::from_utf16_lossy(&name[..end])
}

/// Removable-media qualification. This suite cannot create a FAT volume without
/// administrator rights, so point `MIDNIGHT_TEST_FAT_DIRECTORY` at a writable directory
/// on FAT32/exFAT media and run `cargo test -p midnight-platform-windows --test
/// mutations removable_media -- --ignored --nocapture`. The test works in a fresh
/// subdirectory it removes again. FAT has no reparse-tag, file-ID or POSIX disposition
/// classes: classification falls back to basic attributes, deletion to the classic
/// disposition (refusing Read-only entries) and Copy/Move refuse the volume. On any
/// other filesystem the test checks the ordinary behaviour instead, so a dry run on NTFS
/// exercises the same steps.
#[test]
#[ignore = "opt-in: needs MIDNIGHT_TEST_FAT_DIRECTORY on FAT32/exFAT media"]
fn removable_media_creation_discovery_and_permanent_deletion() {
    use midnight_core::TransferService;
    let base = std::env::var_os("MIDNIGHT_TEST_FAT_DIRECTORY")
        .expect("set MIDNIGHT_TEST_FAT_DIRECTORY to a writable directory on FAT32/exFAT media");
    let filesystem = filesystem_name(Path::new(&base));
    let fat = ["FAT", "FAT32", "exFAT"].contains(&filesystem.as_str());
    eprintln!("qualifying {filesystem} (FAT expectations: {fat})");
    let temp = tempfile::Builder::new()
        .prefix("midnight-media-")
        .tempdir_in(&base)
        .unwrap();
    let cancel = Cancellation::default();
    let filesystem_service = WindowsFileSystem::new(None);
    // F7 and empty-file creation classify every entered directory.
    let base_directory = filesystem_service
        .prepare_create(temp.path(), &cancel)
        .unwrap();
    let nested = base_directory
        .create(Path::new("tree\\nested"), &cancel)
        .unwrap();
    assert!(nested.is_dir());
    let tree = temp.path().join("tree");
    filesystem_service
        .prepare_create(&tree, &cancel)
        .unwrap()
        .create_file(OsStr::new("empty.txt"), &cancel)
        .unwrap();
    fs::write(nested.join("data.bin"), vec![7u8; 70_000]).unwrap();
    let read_only = tree.join("read-only.txt");
    fs::write(&read_only, b"read-only").unwrap();
    let writable = fs::metadata(&read_only).unwrap().permissions();
    let mut permissions = writable.clone();
    permissions.set_readonly(true);
    fs::set_permissions(&read_only, permissions).unwrap();
    // Discovery lists the whole tree from enumeration records.
    let mut discovered = Vec::new();
    prepare(&tree)
        .discover(&cancel, &mut |item| discovered.push(item.path.clone()))
        .unwrap();
    assert_eq!(discovered.len(), 5, "{discovered:?}");
    // A rename and a leaf deletion.
    let renamed = renamed_cleanly(
        prepare(&tree.join("empty.txt"))
            .rename(OsStr::new("renamed to a longer name.txt"), &cancel),
    );
    deleted_cleanly(prepare(&renamed).delete(DeleteMode::Permanent, &cancel, &mut |_| {}));
    assert!(!renamed.exists());
    // Copy and Move need stable identities.
    let transfer = filesystem_service.prepare_transfer_destination(temp.path(), &cancel);
    if fat {
        let error = transfer.err().expect("FAT has no stable file identities");
        assert_eq!(error.kind, io::ErrorKind::Unsupported, "{error:?}");
    } else {
        transfer.unwrap();
    }
    // Recursive permanent deletion: FAT refuses the Read-only entry and keeps it.
    let deleted = prepare(&tree).delete(DeleteMode::Permanent, &cancel, &mut |_| {});
    if fat {
        let error = deleted.expect_err("the classic disposition cannot ignore Read-only");
        assert_eq!(error.kind, io::ErrorKind::PermissionDenied, "{error:?}");
        assert!(read_only.exists());
        fs::set_permissions(&read_only, writable).unwrap();
        deleted_cleanly(prepare(&tree).delete(DeleteMode::Permanent, &cancel, &mut |_| {}));
    } else {
        deleted_cleanly(deleted);
    }
    assert!(!tree.exists());
}

#[test]
fn confirmed_permanent_delete_removes_read_only_empty_directories() {
    let temp = fixture();
    let path = temp.path().join("readonly folder");
    fs::create_dir(&path).unwrap();
    let original = fs::metadata(&path).unwrap().permissions();
    let mut permissions = original.clone();
    permissions.set_readonly(true);
    fs::set_permissions(&path, permissions).unwrap();
    let result =
        prepare(&path).delete(DeleteMode::Permanent, &Cancellation::default(), &mut |_| {});
    if path.exists() {
        fs::set_permissions(&path, original).unwrap();
    }
    deleted_cleanly(result);
    assert!(!path.exists());
}

#[test]
fn create_nested_unicode_directories_keeps_existing_intermediates_and_refuses_final_collision() {
    let temp = fixture();
    fs::create_dir(temp.path().join("existing")).unwrap();
    let base = WindowsFileSystem::new(None)
        .prepare_create(temp.path(), &Cancellation::default())
        .unwrap();
    let created = base
        .create(
            Path::new("existing/文档 🦀/e\u{301}"),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(created, temp.path().join("existing/文档 🦀/e\u{301}"));
    assert!(created.is_dir());
    assert_eq!(
        base.create(
            Path::new("existing/文档 🦀/e\u{301}"),
            &Cancellation::default()
        )
        .unwrap_err()
        .kind,
        io::ErrorKind::AlreadyExists
    );
}

#[test]
fn create_rejects_invalid_names_and_cancelled_requests_without_changes() {
    let temp = fixture();
    let base = WindowsFileSystem::new(None)
        .prepare_create(temp.path(), &Cancellation::default())
        .unwrap();
    for path in [
        "",
        ".",
        "..",
        "good/.",
        "good/..",
        "\\\\.\\PhysicalDrive0",
        "\\\\?\\GLOBALROOT\\Device\\HarddiskVolume1",
        "one/NUL.txt",
        "one/stream:ads",
        "one/trailing.",
    ] {
        assert_eq!(
            base.create(Path::new(path), &Cancellation::default())
                .unwrap_err()
                .kind,
            io::ErrorKind::InvalidInput,
            "{path:?}"
        );
    }
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        WindowsFileSystem::new(None)
            .prepare_create(temp.path(), &cancel)
            .is_err()
    );
    assert_eq!(
        base.create(Path::new("one/two"), &cancel).unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[test]
fn create_accepts_absolute_and_parent_relative_paths_outside_the_captured_base() {
    let temp = fixture();
    let panel = temp.path().join("panel");
    fs::create_dir(&panel).unwrap();
    let cancel = Cancellation::default();
    let base = WindowsFileSystem::new(None)
        .prepare_create(&panel, &cancel)
        .unwrap();
    let absolute = temp.path().join("absolute/文档 🦀/new");
    assert_eq!(base.create(&absolute, &cancel).unwrap(), absolute);
    assert!(absolute.is_dir());
    let relative = base
        .create(Path::new("../sibling/文档/new"), &cancel)
        .unwrap();
    assert_eq!(
        fs::canonicalize(relative).unwrap(),
        fs::canonicalize(temp.path().join("sibling/文档/new")).unwrap()
    );
    assert_eq!(fs::read_dir(panel).unwrap().count(), 0);
}

#[test]
fn create_walks_parent_components_without_collapsing_missing_directories_or_links() {
    let temp = fixture();
    let cancel = Cancellation::default();
    let base = WindowsFileSystem::new(None)
        .prepare_create(temp.path(), &cancel)
        .unwrap();
    let created = base
        .create(Path::new("first/../second/./leaf"), &cancel)
        .unwrap();
    assert!(temp.path().join("first").is_dir());
    assert_eq!(
        fs::canonicalize(created).unwrap(),
        fs::canonicalize(temp.path().join("second/leaf")).unwrap()
    );
    fs::create_dir(temp.path().join("outside")).unwrap();
    create_junction(&temp.path().join("link"), &temp.path().join("outside"));
    assert!(
        base.create(Path::new("link/../forbidden"), &cancel)
            .is_err()
    );
    assert!(!temp.path().join("forbidden").exists());
    assert_eq!(
        fs::read_dir(temp.path().join("outside")).unwrap().count(),
        0
    );
}

#[test]
fn create_resolves_root_relative_and_same_drive_relative_paths_against_the_panel() {
    let temp = fixture();
    let panel = temp.path().join("panel");
    fs::create_dir(&panel).unwrap();
    let cancel = Cancellation::default();
    let base = WindowsFileSystem::new(None)
        .prepare_create(&panel, &cancel)
        .unwrap();
    let absolute = temp.path().join("root-relative/new");
    let prefix = absolute.components().next().unwrap();
    let root_relative = absolute
        .components()
        .skip(1)
        .collect::<std::path::PathBuf>();
    assert_eq!(base.create(&root_relative, &cancel).unwrap(), absolute);
    let drive_relative = format!("{}same-drive/new", prefix.as_os_str().to_str().unwrap());
    assert_eq!(
        base.create(Path::new(&drive_relative), &cancel).unwrap(),
        panel.join("same-drive/new")
    );
    assert!(absolute.is_dir());
    assert!(panel.join("same-drive/new").is_dir());
}

#[test]
fn create_absolute_targets_still_honor_cancellation_and_replaced_base_identity() {
    let temp = fixture();
    let panel = temp.path().join("panel");
    fs::create_dir(&panel).unwrap();
    let cancel = Cancellation::default();
    let base = WindowsFileSystem::new(None)
        .prepare_create(&panel, &cancel)
        .unwrap();
    let absolute = temp.path().join("outside/new");
    cancel.cancel();
    assert_eq!(
        base.create(&absolute, &cancel).unwrap_err().kind,
        io::ErrorKind::Interrupted
    );
    assert!(!temp.path().join("outside").exists());
    fs::rename(&panel, temp.path().join("moved-panel")).unwrap();
    fs::create_dir(&panel).unwrap();
    assert!(base.create(&absolute, &Cancellation::default()).is_err());
    assert!(!temp.path().join("outside").exists());
}

#[test]
fn create_absolute_long_path_preserves_native_unicode_without_replacing_final_entries() {
    use std::os::windows::ffi::OsStringExt;
    let temp = fixture();
    let panel = temp.path().join("panel");
    fs::create_dir(&panel).unwrap();
    let cancel = Cancellation::default();
    let base = WindowsFileSystem::new(None)
        .prepare_create(&panel, &cancel)
        .unwrap();
    let mut target = temp.path().join("outside");
    for _ in 0..10 {
        target.push("文档-long-directory-0123456789");
    }
    target.push(std::ffi::OsString::from_wide(&[b'r' as u16, 0xd800]));
    assert_eq!(base.create(&target, &cancel).unwrap(), target);
    assert!(target.is_dir());
    assert_eq!(
        base.create(&target, &cancel).unwrap_err().kind,
        io::ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read_dir(panel).unwrap().count(), 0);
}

#[test]
fn create_does_not_follow_an_existing_junction_or_replace_a_file() {
    let temp = fixture();
    let outside = temp.path().join("outside");
    let base_path = temp.path().join("base");
    fs::create_dir(&outside).unwrap();
    fs::create_dir(&base_path).unwrap();
    create_junction(&base_path.join("link"), &outside);
    fs::write(base_path.join("file"), b"keep").unwrap();
    let base = WindowsFileSystem::new(None)
        .prepare_create(&base_path, &Cancellation::default())
        .unwrap();
    assert!(
        base.create(Path::new("link/escaped"), &Cancellation::default())
            .is_err()
    );
    assert!(
        base.create(Path::new("file/subdirectory"), &Cancellation::default())
            .is_err()
    );
    assert!(!outside.join("escaped").exists());
    assert_eq!(fs::read(base_path.join("file")).unwrap(), b"keep");
}

#[test]
fn create_supports_paths_beyond_max_path_and_leaves_intermediates_on_os_failure() {
    let temp = fixture();
    let base = WindowsFileSystem::new(None)
        .prepare_create(temp.path(), &Cancellation::default())
        .unwrap();
    let mut relative = std::path::PathBuf::new();
    for _ in 0..10 {
        relative.push("文档-long-directory-0123456789");
    }
    assert!(
        base.create(&relative, &Cancellation::default())
            .unwrap()
            .is_dir()
    );
    assert!(
        base.create(
            &Path::new("partial").join("x".repeat(300)),
            &Cancellation::default()
        )
        .is_err()
    );
    assert!(temp.path().join("partial").is_dir());
}

#[test]
fn rename_replacement_requires_consent_and_preserves_the_displaced_file() {
    let temp = fixture();
    let source = temp.path().join("source.txt");
    let destination = temp.path().join("existing.txt");
    fs::write(&source, b"selected source").unwrap();
    fs::write(&destination, b"consented original").unwrap();
    let item = prepare(&source);
    let cancel = Cancellation::default();
    assert!(item.rename(OsStr::new("existing.txt"), &cancel).is_err());
    let replacement = item
        .prepare_rename_replacement(OsStr::new("existing.txt"), &cancel)
        .unwrap();
    assert_eq!(fs::read(&source).unwrap(), b"selected source");
    assert_eq!(fs::read(&destination).unwrap(), b"consented original");
    let outcome = replacement
        .movement
        .capture
        .try_native_move(
            replacement.directory.as_ref(),
            OsStr::new("existing.txt"),
            Some((
                replacement.target.clone(),
                midnight_core::TransferReplacementPolicy::RecoverableCurrentEntry,
            )),
            &cancel,
        )
        .unwrap();
    let midnight_core::TransferNativeMoveOutcome::Moved {
        destination: renamed,
        recovery,
        ..
    } = outcome
    else {
        panic!("replacement did not commit: {outcome:?}");
    };
    assert_eq!(renamed, destination);
    assert!(!source.exists());
    assert_eq!(fs::read(&destination).unwrap(), b"selected source");
    assert_eq!(recovery.len(), 1);
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"consented original");
}

#[test]
fn rename_replacement_cancel_and_stale_destination_preserve_source_bytes() {
    for change in [false, true] {
        let temp = fixture();
        let source = temp.path().join("source.txt");
        let destination = temp.path().join("existing.txt");
        fs::write(&source, b"selected source").unwrap();
        fs::write(&destination, b"consented original").unwrap();
        let item = prepare(&source);
        let cancel = Cancellation::default();
        let replacement = item
            .prepare_rename_replacement(OsStr::new("existing.txt"), &cancel)
            .unwrap();
        if change {
            fs::write(&destination, b"changed after consent").unwrap();
        } else {
            cancel.cancel();
        }
        assert!(
            replacement
                .movement
                .capture
                .try_native_move(
                    replacement.directory.as_ref(),
                    OsStr::new("existing.txt"),
                    Some((
                        replacement.target.clone(),
                        midnight_core::TransferReplacementPolicy::RecoverableCurrentEntry
                    )),
                    &cancel,
                )
                .is_err()
        );
        assert_eq!(fs::read(&source).unwrap(), b"selected source");
        assert_eq!(
            fs::read(&destination).unwrap(),
            if change {
                b"changed after consent".as_slice()
            } else {
                b"consented original".as_slice()
            }
        );
    }
}

#[test]
fn rename_replacement_rejects_self_hardlinks_directories_and_replaced_sources() {
    let temp = fixture();
    let source = temp.path().join("source.txt");
    fs::write(&source, b"selected source").unwrap();
    let item = prepare(&source);
    fs::hard_link(&source, temp.path().join("alias.txt")).unwrap();
    fs::create_dir(temp.path().join("folder")).unwrap();
    fs::write(temp.path().join("existing.txt"), b"other").unwrap();
    for name in ["source.txt", "alias.txt", "folder"] {
        assert!(
            item.prepare_rename_replacement(OsStr::new(name), &Cancellation::default())
                .is_err(),
            "{name}"
        );
    }
    fs::rename(&source, temp.path().join("original.txt")).unwrap();
    fs::write(&source, b"new source winner").unwrap();
    assert!(
        item.prepare_rename_replacement(OsStr::new("existing.txt"), &Cancellation::default())
            .is_err()
    );
    assert_eq!(fs::read(&source).unwrap(), b"new source winner");
    assert_eq!(
        fs::read(temp.path().join("original.txt")).unwrap(),
        b"selected source"
    );
    assert_eq!(
        fs::read(temp.path().join("existing.txt")).unwrap(),
        b"other"
    );
}

#[test]
fn rename_rejects_distinct_hardlink_aliases_even_when_target_case_differs() {
    let temp = fixture();
    let source = temp.path().join("source.txt");
    let alias = temp.path().join("alias.txt");
    fs::write(&source, b"selected source").unwrap();
    fs::hard_link(&source, &alias).unwrap();
    let item = prepare(&source);
    assert_eq!(
        renamed_cleanly(item.rename(OsStr::new("source.txt"), &Cancellation::default())),
        source
    );
    for name in ["alias.txt", "ALIAS.TXT"] {
        assert_eq!(
            item.rename(OsStr::new(name), &Cancellation::default())
                .unwrap_err()
                .kind,
            io::ErrorKind::AlreadyExists,
            "{name} is another entry, not a case change of the source"
        );
        assert_eq!(fs::read(&source).unwrap(), b"selected source");
        assert_eq!(fs::read(&alias).unwrap(), b"selected source");
        let mut names: Vec<_> = fs::read_dir(temp.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![OsStr::new("alias.txt"), OsStr::new("source.txt")]
        );
    }
}

#[test]
fn rename_case_only_with_another_hardlink_keeps_native_unicode_spelling() {
    let temp = fixture();
    let source = temp.path().join("Žluťoučký.txt");
    let alias = temp.path().join("alias.txt");
    fs::write(&source, b"same object").unwrap();
    fs::hard_link(&source, &alias).unwrap();
    let target = renamed_cleanly(
        prepare(&source).rename(OsStr::new("ŽLUŤOUČKÝ.TXT"), &Cancellation::default()),
    );
    assert_eq!(target, temp.path().join("ŽLUŤOUČKÝ.TXT"));
    assert_eq!(fs::read(&target).unwrap(), b"same object");
    assert_eq!(fs::read(&alias).unwrap(), b"same object");
    let names: Vec<_> = fs::read_dir(temp.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 2);
    assert!(names.iter().any(|name| name == OsStr::new("ŽLUŤOUČKÝ.TXT")));
    assert!(names.iter().any(|name| name == OsStr::new("alias.txt")));
}
