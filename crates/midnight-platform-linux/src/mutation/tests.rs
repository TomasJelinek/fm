use crate::LinuxFileSystem;
use midnight_core::{Cancellation, DeleteMode, EntryKind, MutationService, PreparedMutation};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::ErrorKind,
    os::unix::{ffi::OsStringExt, fs::symlink},
    path::Path,
};

#[test]
fn rename_keeps_raw_bytes_unicode_and_native_backslashes_without_overwriting() {
    let fixture = tempfile::tempdir().unwrap();
    let original = fixture
        .path()
        .join(OsString::from_vec(b"raw-\xff.txt".to_vec()));
    fs::write(&original, b"source").unwrap();
    let cancel = Cancellation::default();
    let service = LinuxFileSystem::new(None);
    let prepared = service.prepare(&original, &cancel).unwrap();
    assert_eq!(prepared.item().path, original);
    assert_eq!(prepared.item().kind, EntryKind::File);
    assert_eq!(prepared.item().metadata.size, Some(6));
    let collision = fixture.path().join("occupied");
    fs::write(&collision, b"destination").unwrap();
    assert_eq!(
        prepared
            .rename(OsStr::new("occupied"), &cancel)
            .unwrap_err()
            .kind,
        ErrorKind::AlreadyExists
    );
    assert_eq!(fs::read(&original).unwrap(), b"source");
    assert_eq!(fs::read(&collision).unwrap(), b"destination");
    let renamed = prepared
        .rename(OsStr::new("文档 👩🏽‍💻\\literal.txt"), &cancel)
        .unwrap();
    assert_eq!(fs::read(&renamed).unwrap(), b"source");
    assert!(!original.exists());
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 2);
}

#[test]
fn rename_case_only_and_same_name_use_native_semantics() {
    let fixture = tempfile::tempdir().unwrap();
    let original = fixture.path().join("Case.txt");
    fs::write(&original, b"case").unwrap();
    let cancel = Cancellation::default();
    let service = LinuxFileSystem::new(None);
    let prepared = service.prepare(&original, &cancel).unwrap();
    assert_eq!(
        prepared.rename(OsStr::new("Case.txt"), &cancel).unwrap(),
        original
    );
    let renamed = prepared.rename(OsStr::new("case.txt"), &cancel).unwrap();
    assert_eq!(fs::read(renamed).unwrap(), b"case");
    assert!(!original.exists());
}

#[test]
fn invalid_basename_and_root_inputs_leave_source_intact() {
    let fixture = tempfile::tempdir().unwrap();
    let original = fixture.path().join("source");
    fs::write(&original, b"source").unwrap();
    let cancel = Cancellation::default();
    let service = LinuxFileSystem::new(None);
    let prepared = service.prepare(&original, &cancel).unwrap();
    for name in ["", ".", "..", "a/b", "/absolute", "nul\0tail"] {
        assert_eq!(
            prepared.rename(OsStr::new(name), &cancel).unwrap_err().kind,
            ErrorKind::InvalidInput,
            "accepted invalid basename {name:?}"
        );
    }
    for path in [
        Path::new("/"),
        Path::new("/tmp/.."),
        Path::new("."),
        Path::new(""),
    ] {
        assert!(service.prepare(path, &cancel).is_err());
    }
    assert_eq!(fs::read(original).unwrap(), b"source");
}

#[test]
fn changed_source_or_parent_identity_never_mutates_replacement() {
    let fixture = tempfile::tempdir().unwrap();
    let parent = fixture.path().join("parent");
    fs::create_dir(&parent).unwrap();
    let original = parent.join("source");
    fs::write(&original, b"source").unwrap();
    let cancel = Cancellation::default();
    let service = LinuxFileSystem::new(None);
    let prepared = service.prepare(&original, &cancel).unwrap();
    fs::remove_file(&original).unwrap();
    fs::write(&original, b"replacement").unwrap();
    assert!(prepared.rename(OsStr::new("renamed"), &cancel).is_err());
    assert!(
        prepared
            .delete(DeleteMode::Permanent, &cancel, &mut |_| {})
            .is_err()
    );
    assert_eq!(fs::read(&original).unwrap(), b"replacement");
    let prepared = service.prepare(&original, &cancel).unwrap();
    fs::rename(&parent, fixture.path().join("moved-parent")).unwrap();
    fs::create_dir(&parent).unwrap();
    fs::write(&original, b"new parent content").unwrap();
    assert!(
        prepared
            .delete(DeleteMode::Permanent, &cancel, &mut |_| {})
            .is_err()
    );
    assert_eq!(fs::read(original).unwrap(), b"new parent content");
    assert_eq!(
        fs::read(fixture.path().join("moved-parent/source")).unwrap(),
        b"replacement"
    );
}

#[test]
fn permanent_delete_recurses_and_never_follows_directory_or_dangling_links() {
    let fixture = tempfile::tempdir().unwrap();
    let outside = fixture.path().join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("keep"), b"keep").unwrap();
    let tree = fixture.path().join("tree");
    fs::create_dir_all(tree.join("child/deeper")).unwrap();
    fs::write(tree.join("child/deeper/file"), b"delete").unwrap();
    symlink(&outside, tree.join("link")).unwrap();
    symlink(fixture.path().join("missing"), tree.join("dangling")).unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None).prepare(&tree, &cancel).unwrap();
    let mut progress = Vec::new();
    prepared
        .delete(DeleteMode::Permanent, &cancel, &mut |path| {
            progress.push(path.to_owned());
        })
        .unwrap();
    assert!(!tree.exists());
    assert_eq!(fs::read(outside.join("keep")).unwrap(), b"keep");
    assert_eq!(progress.len(), 6);
    assert_eq!(progress.last(), Some(&tree));
    assert!(progress.iter().all(|path| path.starts_with(&tree)));
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
}

#[test]
fn deleting_top_directory_symlink_only_unlinks_that_symlink() {
    let fixture = tempfile::tempdir().unwrap();
    let target = fixture.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("keep"), b"keep").unwrap();
    let link = fixture.path().join("link");
    symlink(&target, &link).unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None).prepare(&link, &cancel).unwrap();
    assert!(matches!(
        prepared.item().kind,
        EntryKind::Symlink | EntryKind::DirectoryLink
    ));
    prepared
        .delete(DeleteMode::Permanent, &cancel, &mut |_| {})
        .unwrap();
    assert!(!link.exists());
    assert_eq!(fs::read(target.join("keep")).unwrap(), b"keep");
}

#[test]
fn cancellation_before_mutation_has_no_effect_and_partial_delete_restores_remaining_tree() {
    let fixture = tempfile::tempdir().unwrap();
    let tree = fixture.path().join("tree");
    fs::create_dir(&tree).unwrap();
    for name in ["a", "b", "c"] {
        fs::write(tree.join(name), name).unwrap();
    }
    let cancel = Cancellation::default();
    let service = LinuxFileSystem::new(None);
    let prepared = service.prepare(&tree, &cancel).unwrap();
    cancel.cancel();
    assert_eq!(
        prepared
            .rename(OsStr::new("renamed"), &cancel)
            .unwrap_err()
            .kind,
        ErrorKind::Interrupted
    );
    assert_eq!(
        prepared
            .delete(DeleteMode::Permanent, &cancel, &mut |_| {})
            .unwrap_err()
            .kind,
        ErrorKind::Interrupted
    );
    assert_eq!(fs::read_dir(&tree).unwrap().count(), 3);
    let cancel = Cancellation::default();
    let mut count = 0;
    assert_eq!(
        prepared
            .delete(DeleteMode::Permanent, &cancel, &mut |_| {
                count += 1;
                cancel.cancel();
            })
            .unwrap_err()
            .kind,
        ErrorKind::Interrupted
    );
    assert_eq!(count, 1);
    assert_eq!(fs::read_dir(tree).unwrap().count(), 2);
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
}

#[test]
fn rename_supports_native_paths_longer_than_260_bytes() {
    let fixture = tempfile::tempdir().unwrap();
    let mut parent = fixture.path().to_owned();
    for i in 0..8 {
        parent.push(format!("level-{i}-{}", "long".repeat(10)));
    }
    fs::create_dir_all(&parent).unwrap();
    let source = parent.join("source");
    fs::write(&source, b"long path").unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare(&source, &cancel)
        .unwrap();
    let destination = prepared.rename(OsStr::new("renamed"), &cancel).unwrap();
    assert_eq!(fs::read(destination).unwrap(), b"long path");
}

#[test]
fn trash_preserves_raw_paths_and_complete_directories_in_private_isolated_storage() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = tempfile::tempdir().unwrap();
    let trash = fixture.path().join("data/Trash");
    let source = fixture
        .path()
        .join(OsString::from_vec(b"raw-\xff %\n".to_vec()));
    fs::create_dir(&source).unwrap();
    fs::write(source.join("inside"), b"recoverable").unwrap();
    let cancel = Cancellation::default();
    let prepared = super::Prepared::new(&source, Some(trash.clone()), &cancel).unwrap();
    let mut progress = Vec::new();
    prepared
        .delete(DeleteMode::Trash, &cancel, &mut |path| {
            progress.push(path.to_owned())
        })
        .unwrap();
    assert!(!source.exists());
    assert_eq!(progress.as_slice(), std::slice::from_ref(&source));
    for directory in [&trash, &trash.join("files"), &trash.join("info")] {
        assert_eq!(
            fs::metadata(directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    let entries = fs::read_dir(trash.join("files"))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        fs::read(entries[0].path().join("inside")).unwrap(),
        b"recoverable"
    );
    let mut info_name = entries[0].file_name();
    info_name.push(".trashinfo");
    let info = fs::read_to_string(trash.join("info").join(info_name)).unwrap();
    assert!(info.starts_with("[Trash Info]\nPath=/"));
    assert!(info.contains("/raw-%FF%20%25%0A\nDeletionDate="));
    let date = info
        .lines()
        .find_map(|line| line.strip_prefix("DeletionDate="))
        .unwrap();
    assert!(chrono::NaiveDateTime::parse_from_str(date, "%Y-%m-%dT%H:%M:%S").is_ok());
    assert_eq!(fs::read_dir(trash.join("info")).unwrap().count(), 1);
}

#[test]
fn trash_never_overwrites_an_earlier_copy_of_the_same_name() {
    let fixture = tempfile::tempdir().unwrap();
    let trash = fixture.path().join("Trash");
    let source = fixture.path().join("same-name");
    let cancel = Cancellation::default();
    for content in [b"first", b"other"] {
        fs::write(&source, content).unwrap();
        super::Prepared::new(&source, Some(trash.clone()), &cancel)
            .unwrap()
            .delete(DeleteMode::Trash, &cancel, &mut |_| {})
            .unwrap();
    }
    let mut contents = fs::read_dir(trash.join("files"))
        .unwrap()
        .map(|entry| fs::read(entry.unwrap().path()).unwrap())
        .collect::<Vec<_>>();
    contents.sort();
    assert_eq!(contents, [b"first", b"other"]);
    assert_eq!(fs::read_dir(trash.join("info")).unwrap().count(), 2);
}

#[test]
fn unavailable_or_symlinked_trash_never_falls_back_to_permanent_deletion() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    fs::write(&source, b"keep").unwrap();
    let cancel = Cancellation::default();
    let missing = super::Prepared::new(&source, None, &cancel).unwrap();
    assert_eq!(
        missing
            .delete(DeleteMode::Trash, &cancel, &mut |_| {})
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    let outside = fixture.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let trash = fixture.path().join("Trash");
    symlink(&outside, &trash).unwrap();
    let prepared = super::Prepared::new(&source, Some(trash), &cancel).unwrap();
    assert!(
        prepared
            .delete(DeleteMode::Trash, &cancel, &mut |_| {})
            .is_err()
    );
    assert_eq!(fs::read(&source).unwrap(), b"keep");
    assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
}

#[test]
fn public_world_writable_trash_is_rejected_without_changing_its_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    fs::write(&source, b"keep").unwrap();
    let trash = fixture.path().join("Trash");
    fs::create_dir(&trash).unwrap();
    fs::set_permissions(&trash, fs::Permissions::from_mode(0o777)).unwrap();
    let cancel = Cancellation::default();
    let prepared = super::Prepared::new(&source, Some(trash.clone()), &cancel).unwrap();
    assert!(
        prepared
            .delete(DeleteMode::Trash, &cancel, &mut |_| {})
            .is_err()
    );
    assert_eq!(fs::read(source).unwrap(), b"keep");
    assert_eq!(
        fs::metadata(&trash).unwrap().permissions().mode() & 0o777,
        0o777
    );
    assert_eq!(fs::read_dir(trash).unwrap().count(), 0);
}

#[test]
fn rollback_collision_preserves_both_replacement_and_recoverable_staged_entry() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    fs::write(&source, b"selected").unwrap();
    let cancel = Cancellation::default();
    let prepared = super::Prepared::new(&source, None, &cancel).unwrap();
    let staged = super::Staged::create(&prepared).unwrap();
    staged.capture(&prepared, &cancel).unwrap();
    fs::write(&source, b"replacement").unwrap();
    let failure = staged.restore(
        &prepared,
        super::error(&source, ErrorKind::Interrupted, "cancelled"),
    );
    assert_eq!(fs::read(&source).unwrap(), b"replacement");
    let recovery = staged.path.join(super::STAGED_NAME);
    assert_eq!(fs::read(&recovery).unwrap(), b"selected");
    assert!(failure.message.contains(recovery.to_str().unwrap()));
}

#[test]
fn staged_identity_is_checked_before_any_unlink() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    fs::write(&source, b"selected").unwrap();
    let cancel = Cancellation::default();
    let prepared = super::Prepared::new(&source, None, &cancel).unwrap();
    let staged = super::Staged::create(&prepared).unwrap();
    staged.capture(&prepared, &cancel).unwrap();
    let staged_path = staged.path.join(super::STAGED_NAME);
    fs::rename(&staged_path, staged.path.join("saved")).unwrap();
    fs::write(&staged_path, b"replacement").unwrap();
    assert!(
        super::tree::delete(
            &staged.directory,
            prepared.identity,
            &source,
            &cancel,
            &mut |_| {}
        )
        .is_err()
    );
    assert_eq!(fs::read(staged_path).unwrap(), b"replacement");
    assert_eq!(fs::read(staged.path.join("saved")).unwrap(), b"selected");
}

#[test]
fn cross_filesystem_trash_returns_unsupported_and_keeps_source() {
    use std::os::unix::fs::MetadataExt;
    let mut first = None;
    let mut pair = None;
    for parent in [
        std::env::temp_dir(),
        "/dev/shm".into(),
        "/tmp".into(),
        "/var/tmp".into(),
    ] {
        // Canonical parents also avoid symlink ancestors rejected by mutation setup.
        let Ok(parent) = fs::canonicalize(parent) else {
            continue;
        };
        let Ok(candidate) = tempfile::Builder::new()
            .prefix("midnight-cross-filesystem-")
            .tempdir_in(parent)
        else {
            continue;
        };
        let device = fs::metadata(candidate.path()).unwrap().dev();
        if let Some((fixture, previous_device)) = first.take() {
            if previous_device != device {
                pair = Some((fixture, candidate));
                break;
            }
            first = Some((fixture, previous_device));
        } else {
            first = Some((candidate, device));
        }
    }
    let Some((fixture, separate)) = pair else {
        eprintln!(
            "SKIP cross_filesystem_trash: no two writable distinct filesystems among \
             the configured temporary directory, /dev/shm, /tmp and /var/tmp"
        );
        return;
    };
    assert_ne!(
        fs::metadata(fixture.path()).unwrap().dev(),
        fs::metadata(separate.path()).unwrap().dev(),
        "fixture needs a second filesystem"
    );
    eprintln!("QUALIFIED cross_filesystem_trash: owned fixtures are on distinct filesystems");
    let source = fixture.path().join("source");
    fs::write(&source, b"keep").unwrap();
    let cancel = Cancellation::default();
    let trash = separate.path().join("Trash");
    let prepared = super::Prepared::new(&source, Some(trash.clone()), &cancel).unwrap();
    assert_eq!(
        prepared
            .delete(DeleteMode::Trash, &cancel, &mut |_| {})
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    assert_eq!(fs::read(source).unwrap(), b"keep");
    assert_eq!(fs::read_dir(trash.join("info")).unwrap().count(), 0);
    assert_eq!(fs::read_dir(trash.join("files")).unwrap().count(), 0);
}

#[test]
fn cancelled_trash_creates_nothing_and_trash_cannot_contain_itself() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    fs::create_dir(&source).unwrap();
    let cancel = Cancellation::default();
    let trash = source.join("data/Trash");
    let prepared = super::Prepared::new(&source, Some(trash.clone()), &cancel).unwrap();
    cancel.cancel();
    assert_eq!(
        prepared
            .delete(DeleteMode::Trash, &cancel, &mut |_| {})
            .unwrap_err()
            .kind,
        ErrorKind::Interrupted
    );
    assert!(!trash.exists());
    let cancel = Cancellation::default();
    assert_eq!(
        prepared
            .delete(DeleteMode::Trash, &cancel, &mut |_| {})
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
    assert!(source.is_dir());
    assert_eq!(fs::read_dir(source).unwrap().count(), 0);
}

#[test]
fn symlinked_trash_files_and_info_directories_are_rejected() {
    use std::os::unix::fs::PermissionsExt;
    for child in ["files", "info"] {
        let fixture = tempfile::tempdir().unwrap();
        let source = fixture.path().join("source");
        fs::write(&source, b"keep").unwrap();
        let trash = fixture.path().join("Trash");
        fs::create_dir(&trash).unwrap();
        fs::set_permissions(&trash, fs::Permissions::from_mode(0o700)).unwrap();
        let outside = fixture.path().join("outside");
        fs::create_dir(&outside).unwrap();
        symlink(&outside, trash.join(child)).unwrap();
        let cancel = Cancellation::default();
        let prepared = super::Prepared::new(&source, Some(trash), &cancel).unwrap();
        assert!(
            prepared
                .delete(DeleteMode::Trash, &cancel, &mut |_| {})
                .is_err()
        );
        assert_eq!(fs::read(source).unwrap(), b"keep");
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }
}

#[test]
fn trash_infrastructure_is_rejected_before_reserving_metadata_or_staging() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    for child in ["info", "files"] {
        let fixture = tempfile::tempdir().unwrap();
        let trash = fixture.path().join("Trash");
        for path in [&trash, &trash.join("files"), &trash.join("info")] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let payload = trash.join("files/kept");
        let metadata = trash.join("info/kept.trashinfo");
        fs::write(&payload, b"recoverable payload").unwrap();
        fs::write(
            &metadata,
            b"[Trash Info]\nPath=/original\nDeletionDate=2026-11-05T12:00:00\n",
        )
        .unwrap();
        let original_metadata = fs::read(&metadata).unwrap();
        let source = trash.join(child);
        let original_identity = fs::metadata(&source).unwrap();
        let cancel = Cancellation::default();
        let prepared = super::Prepared::new(&source, Some(trash.clone()), &cancel).unwrap();
        let mut progress = Vec::new();
        let failure = prepared
            .delete(DeleteMode::Trash, &cancel, &mut |path| {
                progress.push(path.to_owned())
            })
            .unwrap_err();
        assert_eq!(failure.kind, ErrorKind::InvalidInput);
        assert!(failure.message.contains("trash infrastructure"));
        let retained_identity = fs::metadata(&source).unwrap();
        assert_eq!(retained_identity.dev(), original_identity.dev());
        assert_eq!(retained_identity.ino(), original_identity.ino());
        assert_eq!(fs::read(&payload).unwrap(), b"recoverable payload");
        assert_eq!(fs::read(&metadata).unwrap(), original_metadata);
        assert_eq!(fs::read_dir(trash.join("files")).unwrap().count(), 1);
        assert_eq!(fs::read_dir(trash.join("info")).unwrap().count(), 1);
        assert_eq!(fs::read_dir(&trash).unwrap().count(), 2);
        assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
        assert!(progress.is_empty());
    }
}

#[test]
fn deep_tree_uses_native_limits_without_a_small_application_depth_limit() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("tree");
    let mut deepest = source.clone();
    for _ in 0..256 {
        deepest.push("d");
    }
    fs::create_dir_all(&deepest).unwrap();
    fs::write(deepest.join("file"), b"deep").unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare(&source, &cancel)
        .unwrap();
    let mut count = 0;
    prepared
        .delete(DeleteMode::Permanent, &cancel, &mut |_| count += 1)
        .unwrap();
    assert_eq!(count, 258);
    assert!(!source.exists());
}

#[test]
fn special_files_are_pinned_without_opening_their_contents() {
    let fixture = tempfile::tempdir().unwrap();
    let fifo = fixture.path().join("pipe");
    rustix::fs::mkfifoat(rustix::fs::CWD, &fifo, rustix::fs::Mode::RWXU).unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None).prepare(&fifo, &cancel).unwrap();
    assert_eq!(prepared.item().kind, EntryKind::Other);
    prepared
        .delete(DeleteMode::Permanent, &cancel, &mut |_| {})
        .unwrap();
    assert!(!fifo.exists());
}

#[test]
fn xdg_trash_location_uses_only_absolute_environment_paths() {
    let home = Path::new("/home/person");
    assert_eq!(
        super::trash::home_trash(Some(home), Some("/data".into())),
        Some("/data/Trash".into())
    );
    for value in [None, Some("".into()), Some("relative".into())] {
        assert_eq!(
            super::trash::home_trash(Some(home), value),
            Some("/home/person/.local/share/Trash".into())
        );
    }
    assert_eq!(super::trash::home_trash(None, None), None);
}
