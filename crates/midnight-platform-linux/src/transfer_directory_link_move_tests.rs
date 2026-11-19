use super::*;
use crate::LinuxFileSystem;
use midnight_core::{CopyLinkPolicy, PreparedTransferMoveEntry, TransferService};
use std::{fs, os::unix::fs::symlink};

fn fixture() -> tempfile::TempDir {
    crate::test_support::temporary("directory-link-move-")
}

fn fixture_link(parent: &Path) -> PathBuf {
    let target = parent.join("target_文档");
    let alias = parent.join("alias_e\u{301}");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("retained"), b"target contents").unwrap();
    symlink("target_文档", &alias).unwrap();
    alias
}

fn race_capture(path: &Path, cancel: &Cancellation) -> Capture {
    let (parent, parent_path, name) = super::super::move_parent(path, cancel).unwrap();
    let traversal = directory_links::Traversal::new(directory_links::DirectoryLinkMode::Follow);
    let (source, parent) =
        super::super::prepare_candidate(parent, parent_path, &name, None, Some(traversal), cancel)
            .unwrap();
    Capture::prepare(source.original_link().unwrap(), parent, cancel).unwrap()
}

#[test]
fn same_volume_native_link_move_keeps_the_raw_link_for_either_policy() {
    let Some(fixture) = crate::test_support::shm_temporary("link-native-", "same-volume link Move")
    else {
        return;
    };
    for policy in [
        CopyLinkPolicy::FollowDirectoryContents,
        CopyLinkPolicy::EmptyDirectories,
    ] {
        let source = tempfile::tempdir_in(fixture.path()).unwrap();
        let destination_fixture = tempfile::tempdir_in(fixture.path()).unwrap();
        let source_parent = source.path();
        let destination_parent = destination_fixture.path();
        let alias = fixture_link(source_parent);
        let before = fs::symlink_metadata(&alias).unwrap();
        let relative = fs::read_link(&alias).unwrap();
        let cancel = Cancellation::default();
        let filesystem = LinuxFileSystem::new(None);
        let prepared = filesystem
            .prepare_move_source_with_links(&alias, policy, &cancel)
            .unwrap();
        let PreparedTransferMoveEntry::DirectoryLink { capture, .. } = prepared else {
            panic!("expected original link authority");
        };
        let destination = filesystem
            .prepare_move_destination(destination_parent, &cancel)
            .unwrap();
        let name = OsStr::new("renamed_🧪");
        let expected = destination_parent.join(name);
        let outcome = capture
            .try_native_move(&*destination, name, None, &cancel)
            .unwrap();
        assert!(matches!(outcome, TransferNativeMoveOutcome::Moved {
            destination, recovery, ..
        } if destination == expected && recovery.is_empty()));
        let after = fs::symlink_metadata(&expected).unwrap();
        assert!(after.is_symlink());
        assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
        assert_eq!(fs::read_link(&expected).unwrap(), relative);
        assert_eq!(
            fs::metadata(&expected).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
        assert!(fs::symlink_metadata(&alias).is_err());
        assert_eq!(
            fs::read(source_parent.join("target_文档/retained")).unwrap(),
            b"target contents"
        );
        assert_eq!(fs::read_dir(source_parent).unwrap().count(), 1);
        assert_eq!(fs::read_dir(destination_parent).unwrap().count(), 1);
    }
}

#[test]
fn cross_volume_native_link_move_leaves_both_names_for_verified_projection() {
    let Some(fixture) =
        crate::test_support::ext_temporary("link-native-source-", "native link EXDEV")
    else {
        return;
    };
    let Some(target) =
        crate::test_support::shm_temporary("link-native-target-", "native link EXDEV")
    else {
        return;
    };
    let alias = fixture_link(fixture.path());
    let original = fs::symlink_metadata(&alias).unwrap();
    let filesystem = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let prepared = filesystem
        .prepare_move_source_with_links(&alias, CopyLinkPolicy::FollowDirectoryContents, &cancel)
        .unwrap();
    let PreparedTransferMoveEntry::DirectoryLink {
        mut source,
        capture,
    } = prepared
    else {
        panic!("expected original link authority");
    };
    let destination = filesystem
        .prepare_move_destination(target.path(), &cancel)
        .unwrap();
    assert!(matches!(
        capture
            .try_native_move(&*destination, OsStr::new("destination"), None, &cancel)
            .unwrap(),
        TransferNativeMoveOutcome::CrossDevice
    ));
    let still_original = fs::symlink_metadata(&alias).unwrap();
    assert_eq!(
        (original.dev(), original.ino()),
        (still_original.dev(), still_original.ino())
    );
    assert_eq!(fs::read_link(&alias).unwrap(), Path::new("target_文档"));
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 2);
    assert_eq!(fs::read_dir(target.path()).unwrap().count(), 0);
    assert!(source.next_child(&cancel).unwrap().is_some());
}

#[test]
fn same_volume_native_link_move_refuses_replacement_before_staging() {
    let Some(fixture) = crate::test_support::shm_temporary("link-native-", "link Move replacement")
    else {
        return;
    };
    let alias = fixture_link(fixture.path());
    let target = fixture.path().join("existing");
    fs::write(&target, b"existing destination").unwrap();
    let filesystem = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let prepared = filesystem
        .prepare_move_source_with_links(&alias, CopyLinkPolicy::EmptyDirectories, &cancel)
        .unwrap();
    let PreparedTransferMoveEntry::DirectoryLink { capture, .. } = prepared else {
        panic!("expected original link authority");
    };
    let destination = filesystem
        .prepare_move_destination(fixture.path(), &cancel)
        .unwrap();
    let name = OsStr::new("existing");
    let replacement = destination.prepare_target(name, &cancel).unwrap().unwrap();
    let result = capture.try_native_move(
        &*destination,
        name,
        Some((
            replacement,
            TransferReplacementPolicy::RecoverableCurrentEntry,
        )),
        &cancel,
    );
    assert!(matches!(result, Err(failure) if failure.kind == io::ErrorKind::Unsupported));
    assert_eq!(fs::read_link(alias).unwrap(), Path::new("target_文档"));
    assert_eq!(fs::read(target).unwrap(), b"existing destination");
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 3);
}

#[test]
fn qualified_local_mount_keeps_relative_link_and_descendant_paths() {
    let Some(fixture) = crate::test_support::shm_temporary("link-parent-anchor-", "mounted link")
    else {
        return;
    };
    let parent = fixture.path().join("source");
    let target = fixture.path().join("target_文档");
    fs::create_dir(&parent).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(target.join("child"), b"contents").unwrap();
    let alias = parent.join("alias");
    symlink("../target_文档", &alias).unwrap();
    let cancel = Cancellation::default();
    let filesystem = LinuxFileSystem::new(None);
    let mut source = filesystem
        .prepare_transfer_source_with_links(
            &alias,
            CopyLinkPolicy::FollowDirectoryContents,
            &cancel,
        )
        .unwrap();
    source.revalidate(&cancel).unwrap();
    let child = source.next_child(&cancel).unwrap().unwrap();
    assert_eq!(child.entry().item.path, alias.join("child"));
    child.revalidate(&cancel).unwrap();
}

#[test]
fn rebound_link_parent_cannot_cross_into_a_foreign_mount() {
    let Some(fixture) = crate::test_support::ext_temporary("link-anchor-source-", "rebound mount")
    else {
        return;
    };
    let Some(foreign) = crate::test_support::shm_temporary("link-anchor-target-", "rebound mount")
    else {
        return;
    };
    let parent = fixture.path().join("source");
    fs::create_dir(&parent).unwrap();
    let alias = fixture_link(&parent);
    let cancel = Cancellation::default();
    let filesystem = LinuxFileSystem::new(None);
    let source = filesystem
        .prepare_transfer_source_with_links(
            &alias,
            CopyLinkPolicy::FollowDirectoryContents,
            &cancel,
        )
        .unwrap();
    let retained = fixture.path().join("retained");
    fs::rename(&parent, &retained).unwrap();
    symlink(foreign.path(), &parent).unwrap();
    assert!(matches!(source.revalidate(&cancel), Err(failure)
        if failure.kind == io::ErrorKind::CrossesDevices));
    assert_eq!(
        fs::read_link(retained.join(alias.file_name().unwrap())).unwrap(),
        Path::new("target_文档")
    );
    assert_eq!(fs::read_dir(foreign.path()).unwrap().count(), 0);
}

#[test]
fn private_link_stage_reuses_the_captured_parent_mount_guard() {
    let Some(fixture) =
        crate::test_support::ext_temporary("link-stage-parent-", "link stage mount")
    else {
        return;
    };
    let Some(foreign) =
        crate::test_support::shm_temporary("link-stage-foreign-", "link stage mount")
    else {
        return;
    };
    let parent = fixture.path().join("source");
    fs::create_dir(&parent).unwrap();
    let alias = fixture_link(&parent);
    let cancel = Cancellation::default();
    let capture = race_capture(&alias, &cancel);
    let stage = capture.parent.source_stage(&cancel).unwrap();
    stage.revalidate(&cancel).unwrap();
    let staged_name = stage.path().file_name().unwrap();

    let retained = fixture.path().join("retained");
    fs::rename(&parent, &retained).unwrap();
    symlink(foreign.path(), &parent).unwrap();

    let parent_failure = capture.parent.check_recovery_parent(&cancel).unwrap_err();
    assert_eq!(parent_failure.kind, io::ErrorKind::CrossesDevices);
    let stage_failure = stage.revalidate(&cancel).unwrap_err();
    assert_eq!(stage_failure.kind, io::ErrorKind::CrossesDevices);
    let creation_failure = capture
        .parent
        .source_stage(&cancel)
        .err()
        .expect("rebinding the original parent must refuse stage creation");
    assert_eq!(creation_failure.kind, io::ErrorKind::CrossesDevices);
    assert!(retained.join(staged_name).is_dir());
    assert!(
        fs::symlink_metadata(retained.join(alias.file_name().unwrap()))
            .unwrap()
            .is_symlink()
    );
    assert_eq!(fs::read_dir(foreign.path()).unwrap().count(), 0);
}

#[test]
fn qualified_link_move_removes_only_the_stub_and_keeps_the_referent() {
    let fixture = fixture();
    let alias = fixture_link(fixture.path());
    let filesystem = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let prepared = filesystem
        .prepare_move_source_with_links(&alias, CopyLinkPolicy::EmptyDirectories, &cancel)
        .unwrap();
    let PreparedTransferMoveEntry::DirectoryLink {
        mut source,
        capture,
    } = prepared
    else {
        panic!("expected separate directory link authority");
    };
    assert_eq!(capture.entry().item.kind, EntryKind::DirectoryLink);
    assert_eq!(capture.entry().item.path, alias);
    assert_eq!(
        source.native_original_identity(),
        Some(capture.entry().identity)
    );
    assert!(source.next_child(&cancel).unwrap().is_none());
    assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
    let outcome = capture.remove_original(&cancel).unwrap();
    assert!(matches!(
        outcome,
        TransferDirectoryLinkRemoval::Removed {
            state: TransferSourceRemoval::Unlinked,
            recovery,
            error: None,
            ..
        } if recovery.is_empty()
    ));
    assert!(fs::symlink_metadata(&alias).is_err());
    assert!(matches!(
        capture.remove_original(&cancel).unwrap(),
        TransferDirectoryLinkRemoval::Removed {
            state: TransferSourceRemoval::Unlinked,
            ..
        }
    ));
    assert_eq!(
        fs::read(fixture.path().join("target_文档/retained")).unwrap(),
        b"target contents"
    );
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
}

#[test]
fn ordinary_move_root_passes_policy_to_a_link_child_with_read_only_descent() {
    let fixture = fixture();
    let root = fixture.path().join("source");
    fs::create_dir(&root).unwrap();
    let alias = root.join("alias");
    fs::create_dir(fixture.path().join("target")).unwrap();
    fs::write(fixture.path().join("target/child"), b"outside source").unwrap();
    symlink("../target", &alias).unwrap();
    let filesystem = LinuxFileSystem::new(None);
    let cancel = Cancellation::default();
    let prepared = filesystem
        .prepare_move_source_with_links(&root, CopyLinkPolicy::FollowDirectoryContents, &cancel)
        .unwrap();
    let PreparedTransferMoveEntry::Ordinary(mut prepared) = prepared else {
        panic!("expected ordinary native Move root");
    };
    let child = prepared
        .source
        .next_move_child_with_links(CopyLinkPolicy::FollowDirectoryContents, &cancel)
        .unwrap()
        .unwrap();
    let PreparedTransferMoveEntry::DirectoryLink {
        mut source,
        capture,
    } = child
    else {
        panic!("expected typed directory link child");
    };
    assert!(
        source
            .next_move_child_with_links(CopyLinkPolicy::FollowDirectoryContents, &cancel)
            .is_err()
    );
    let child = source.next_child(&cancel).unwrap().unwrap();
    assert_eq!(child.entry().item.path, alias.join("child"));
    drop(capture);
    drop(source);
    assert!(fs::symlink_metadata(&alias).unwrap().is_symlink());
    assert_eq!(fs::read_dir(root).unwrap().count(), 1);
}

#[test]
fn cancellation_after_staging_restores_the_original_link() {
    let fixture = fixture();
    let alias = fixture_link(fixture.path());
    let cancel = Cancellation::default();
    let capture = race_capture(&alias, &cancel);
    let outcome = capture.remove_with(&cancel, || {}, || cancel.cancel());
    assert!(matches!(
        outcome,
        Ok(TransferDirectoryLinkRemoval::RecoveryRequired { recovery, error })
            if recovery.is_empty() && error.kind == io::ErrorKind::Interrupted
    ));
    assert!(matches!(
        capture.remove_original(&Cancellation::default()).unwrap(),
        TransferDirectoryLinkRemoval::RecoveryRequired { recovery, .. } if recovery.is_empty()
    ));
    assert_eq!(fs::read_link(alias).unwrap(), Path::new("target_文档"));
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 2);
}

#[test]
fn raced_in_entry_and_new_source_winner_are_preserved_with_a_recovery_receipt() {
    let fixture = fixture();
    let alias = fixture_link(fixture.path());
    let original = fixture.path().join("externally retained original");
    let cancel = Cancellation::default();
    let capture = race_capture(&alias, &cancel);
    let outcome = capture
        .remove_with(
            &cancel,
            || {
                fs::rename(&alias, &original).unwrap();
                fs::write(&alias, b"raced in before capture").unwrap();
            },
            || fs::write(&alias, b"new original-name winner").unwrap(),
        )
        .unwrap();
    let TransferDirectoryLinkRemoval::RecoveryRequired { recovery, .. } = outcome else {
        panic!("expected a retained unexpected source");
    };
    assert_eq!(recovery.len(), 1);
    assert_ne!(recovery[0].identity, Some(capture.entry().identity));
    assert_eq!(
        fs::read(&recovery[0].path).unwrap(),
        b"raced in before capture"
    );
    assert_eq!(fs::read(&alias).unwrap(), b"new original-name winner");
    assert_eq!(fs::read_link(original).unwrap(), Path::new("target_文档"));
}

#[test]
fn a_symlink_with_another_native_name_is_refused_before_staging() {
    let fixture = fixture();
    let alias = fixture_link(fixture.path());
    let other = fixture.path().join("another name");
    fs::hard_link(&alias, &other).unwrap();
    let result = LinuxFileSystem::new(None).prepare_move_source_with_links(
        &alias,
        CopyLinkPolicy::EmptyDirectories,
        &Cancellation::default(),
    );
    assert!(matches!(result, Err(failure) if failure.kind == io::ErrorKind::Unsupported));
    assert_eq!(fs::read_link(alias).unwrap(), Path::new("target_文档"));
    assert_eq!(fs::read_link(other).unwrap(), Path::new("target_文档"));
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 3);
}
