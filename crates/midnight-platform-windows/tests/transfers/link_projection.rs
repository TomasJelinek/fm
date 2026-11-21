use super::fixture;
use midnight_core::{
    Cancellation, CopyLinkPolicy, EntryKind, PreparedTransferMoveEntry,
    TransferDirectoryLinkRemoval, TransferNativeMoveOutcome, TransferService,
    TransferSourceRemoval,
};
use midnight_platform_windows::WindowsFileSystem;
use std::{ffi::OsStr, fs, path::Path};

#[path = "../../src/test_fixtures.rs"]
mod native_fixture;

fn junction(link: &Path, target: &Path) {
    fs::create_dir(link).unwrap();
    native_fixture::create_junction(link, target).unwrap();
}

#[test]
fn following_a_directory_link_rejects_redirects_at_any_target_component() {
    let temp = fixture();
    let referent = temp.path().join("referent");
    fs::create_dir_all(referent.join("nested")).unwrap();
    fs::write(referent.join("sentinel"), b"root data").unwrap();
    fs::write(referent.join("nested/sentinel"), b"nested data").unwrap();
    let redirect = temp.path().join("redirect");
    junction(&redirect, &referent);
    let cancel = Cancellation::default();
    let provider = WindowsFileSystem::new(None);
    for (name, target, bytes) in [
        ("final-redirect", redirect.clone(), b"root data".as_slice()),
        (
            "intermediate-redirect",
            redirect.join("nested"),
            b"nested data".as_slice(),
        ),
    ] {
        let link = temp.path().join(name);
        junction(&link, &target);
        assert_eq!(fs::read(link.join("sentinel")).unwrap(), bytes);
        match provider.prepare_transfer_source_with_links(
            &link,
            CopyLinkPolicy::FollowDirectoryContents,
            &cancel,
        ) {
            Err(error) => assert_eq!(error.kind, std::io::ErrorKind::Unsupported),
            Ok(_) => panic!("the second redirect was followed"),
        }
        let mut empty = provider
            .prepare_transfer_source_with_links(&link, CopyLinkPolicy::EmptyDirectories, &cancel)
            .unwrap();
        assert!(empty.next_child(&cancel).unwrap().is_none());
        drop(empty);
        fs::remove_dir(link).unwrap();
    }
    assert_eq!(fs::read(referent.join("sentinel")).unwrap(), b"root data");
    assert_eq!(
        fs::read(referent.join("nested/sentinel")).unwrap(),
        b"nested data"
    );
    fs::remove_dir(redirect).unwrap();
}

#[test]
fn native_move_renames_the_original_junction_under_both_policies() {
    for policy in [
        CopyLinkPolicy::FollowDirectoryContents,
        CopyLinkPolicy::EmptyDirectories,
    ] {
        let temp = fixture();
        let target = temp.path().join("referent");
        let to = temp.path().join("destination");
        let from = temp.path().join("original 文档");
        fs::create_dir(&target).unwrap();
        fs::create_dir(&to).unwrap();
        fs::write(target.join("sentinel"), b"referent data").unwrap();
        junction(&from, &target);
        let original_payload = fs::read_link(&from).unwrap();
        let cancel = Cancellation::default();
        let provider = WindowsFileSystem::new(None);
        let PreparedTransferMoveEntry::DirectoryLink { source, capture } = provider
            .prepare_move_source_with_links(&from, policy, &cancel)
            .unwrap()
        else {
            panic!("expected original directory link authority");
        };
        let original = capture.entry().identity;
        let directory = provider.prepare_transfer_destination(&to, &cancel).unwrap();
        let name = OsStr::new("moved 文档");
        let moved = to.join(name);
        for _ in 0..2 {
            assert!(matches!(
                capture.try_native_move(directory.as_ref(), name, None, &cancel).unwrap(),
                // Both folders saved (WIN-6), and the repeated call reports the same.
                TransferNativeMoveOutcome::Moved {
                    destination, recovery, durability_confirmed: true
                } if destination == moved && recovery.is_empty()
            ));
        }
        assert!(capture.remove_original(&cancel).is_err());
        drop((capture, source));
        let actual = directory.inspect(name, &cancel).unwrap().unwrap();
        assert_eq!(actual.identity, original);
        assert_eq!(actual.item.kind, EntryKind::DirectoryLink);
        assert_eq!(fs::read_link(&moved).unwrap(), original_payload);
        assert_eq!(
            fs::symlink_metadata(&from).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"referent data");
        fs::remove_dir(moved).unwrap();
    }
}

#[test]
fn native_directory_link_move_keeps_both_objects_on_collision_and_replacement() {
    use midnight_core::TransferReplacementPolicy;
    let temp = fixture();
    let target = temp.path().join("referent");
    let to = temp.path().join("destination");
    let from = temp.path().join("original");
    fs::create_dir(&target).unwrap();
    fs::create_dir(&to).unwrap();
    fs::write(target.join("sentinel"), b"referent").unwrap();
    let winner = to.join("occupied");
    fs::write(&winner, b"destination winner").unwrap();
    junction(&from, &target);
    let original_payload = fs::read_link(&from).unwrap();
    let cancel = Cancellation::default();
    let provider = WindowsFileSystem::new(None);
    let PreparedTransferMoveEntry::DirectoryLink { source, capture } = provider
        .prepare_move_source_with_links(&from, CopyLinkPolicy::EmptyDirectories, &cancel)
        .unwrap()
    else {
        panic!("expected original directory link authority");
    };
    let directory = provider.prepare_transfer_destination(&to, &cancel).unwrap();
    let name = OsStr::new("occupied");
    let consent = directory.prepare_target(name, &cancel).unwrap().unwrap();
    for replace in [
        None,
        Some((consent, TransferReplacementPolicy::RecoverableCurrentEntry)),
    ] {
        assert_eq!(
            capture
                .try_native_move(directory.as_ref(), name, replace, &cancel)
                .unwrap_err()
                .kind,
            std::io::ErrorKind::AlreadyExists
        );
    }
    assert_eq!(fs::read(&winner).unwrap(), b"destination winner");
    assert_eq!(fs::read_link(&from).unwrap(), original_payload);
    assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"referent");
    drop((capture, source));
    fs::remove_dir(from).unwrap();
}

#[test]
#[ignore = "requires permission to create Windows directory symlinks"]
fn native_move_keeps_a_relative_directory_symlink_payload_between_parents() {
    use std::{os::windows::fs::symlink_dir, path::PathBuf};
    let temp = fixture();
    let left = temp.path().join("left");
    let right = temp.path().join("right");
    fs::create_dir_all(left.join("referent")).unwrap();
    fs::create_dir_all(right.join("referent")).unwrap();
    fs::write(left.join("referent/sentinel"), b"left").unwrap();
    fs::write(right.join("referent/sentinel"), b"right").unwrap();
    let left_parent = left.join("parent");
    let right_parent = right.join("parent");
    fs::create_dir(&left_parent).unwrap();
    fs::create_dir(&right_parent).unwrap();
    let from = left_parent.join("link");
    let moved = right_parent.join("link");
    symlink_dir(r"..\referent", &from).unwrap();
    assert_eq!(fs::read(from.join("sentinel")).unwrap(), b"left");
    let cancel = Cancellation::default();
    let provider = WindowsFileSystem::new(None);
    let PreparedTransferMoveEntry::DirectoryLink { source, capture } = provider
        .prepare_move_source_with_links(&from, CopyLinkPolicy::FollowDirectoryContents, &cancel)
        .unwrap()
    else {
        panic!("expected original directory link authority");
    };
    let original = capture.entry().identity;
    let directory = provider
        .prepare_transfer_destination(&right_parent, &cancel)
        .unwrap();
    let name = OsStr::new("link");
    assert!(matches!(
        capture.try_native_move(directory.as_ref(), name, None, &cancel).unwrap(),
        TransferNativeMoveOutcome::Moved { destination, .. } if destination == moved
    ));
    drop((capture, source));
    assert_eq!(
        directory.inspect(name, &cancel).unwrap().unwrap().identity,
        original
    );
    assert_eq!(
        fs::read_link(&moved).unwrap(),
        PathBuf::from(r"..\referent")
    );
    assert_eq!(fs::read(moved.join("sentinel")).unwrap(), b"right");
    assert_eq!(fs::read(left.join("referent/sentinel")).unwrap(), b"left");
    assert_eq!(fs::read(right.join("referent/sentinel")).unwrap(), b"right");
    fs::remove_dir(moved).unwrap();
}

#[test]
#[ignore = "requires NATIVE_MOVE_OTHER_VOLUME_ROOT on a distinct local Windows volume"]
fn native_directory_link_move_uses_the_stub_volume_and_leaves_cross_device_unchanged() {
    let temp = fixture();
    let other = tempfile::tempdir_in(
        std::env::var_os("NATIVE_MOVE_OTHER_VOLUME_ROOT").expect("owned second volume fixture"),
    )
    .unwrap();
    let referent = other.path().join("referent");
    fs::create_dir(&referent).unwrap();
    fs::write(referent.join("sentinel"), b"referent").unwrap();
    let from = temp.path().join("link");
    junction(&from, &referent);
    let cancel = Cancellation::default();
    let provider = WindowsFileSystem::new(None);
    let PreparedTransferMoveEntry::DirectoryLink { source, capture } = provider
        .prepare_move_source_with_links(&from, CopyLinkPolicy::FollowDirectoryContents, &cancel)
        .unwrap()
    else {
        panic!("expected original directory link authority");
    };
    let to = provider
        .prepare_transfer_destination(other.path(), &cancel)
        .unwrap();
    let original = capture.entry().identity;
    assert_ne!(original.device, to.identity().device);
    assert_eq!(source.native_identity(), None);
    assert_eq!(source.native_original_identity(), Some(original));
    let name = OsStr::new("absent");
    assert!(matches!(
        capture
            .try_native_move(to.as_ref(), name, None, &cancel)
            .unwrap(),
        TransferNativeMoveOutcome::CrossDevice
    ));
    assert!(to.inspect(name, &cancel).unwrap().is_none());
    assert_eq!(fs::read(from.join("sentinel")).unwrap(), b"referent");
    let projection = capture.prepare_copy_projection(&cancel).unwrap();
    assert_eq!(projection.native_original_identity(), Some(original));
    assert_eq!(
        projection.native_identity().unwrap().device,
        to.identity().device
    );
    drop((capture, projection, source));
    fs::remove_dir(from).unwrap();
}

#[test]
fn follow_junction_discovers_and_reads_the_projected_tree_at_the_logical_occurrence() {
    let temp = fixture();
    let source = temp.path().join("source");
    let target = temp.path().join("physical");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(target.join("篇.txt"), b"verified source").unwrap();
    let link = source.join("别名");
    junction(&link, &target);
    let cancel = Cancellation::default();
    let provider = WindowsFileSystem::new(None);
    let actual = provider.prepare_transfer_source(&target, &cancel).unwrap();
    let mut tree = provider
        .prepare_transfer_source_with_links(
            &source,
            CopyLinkPolicy::FollowDirectoryContents,
            &cancel,
        )
        .unwrap();

    let mut discovered = Vec::new();
    tree.discover(&cancel, &mut |item| {
        discovered.push((item.path.clone(), item.kind, item.metadata.size));
    })
    .unwrap();
    assert_eq!(
        discovered,
        [
            (source.clone(), EntryKind::Directory, None),
            (link.clone(), EntryKind::Directory, None),
            (link.join("篇.txt"), EntryKind::File, Some(15)),
        ]
    );
    let mut projected = tree.next_child(&cancel).unwrap().unwrap();
    assert_eq!(projected.entry().item.path, link);
    assert_eq!(projected.native_identity(), actual.native_identity());
    assert!(projected.native_namespace_scope(&cancel).unwrap().is_none());
    assert!(projected.next_move_child(&cancel).is_err());
    let destination = provider
        .prepare_transfer_destination(&target, &cancel)
        .unwrap();
    assert!(
        destination
            .reject_source_ancestor(projected.entry().identity, &cancel)
            .is_err()
    );
    let mut child = projected.next_child(&cancel).unwrap().unwrap();
    drop(projected);
    drop(tree);
    child.revalidate(&cancel).unwrap();
    assert_eq!(child.entry().item.path, link.join("篇.txt"));
    let mut bytes = [0; 15];
    assert_eq!(child.read_chunk(&mut bytes, &cancel).unwrap(), 15);
    assert_eq!(&bytes, b"verified source");
    drop(child);
    fs::remove_dir(&link).unwrap();
    assert_eq!(fs::read(target.join("篇.txt")).unwrap(), b"verified source");
}

#[test]
fn empty_junction_is_a_leaf_even_when_its_referent_does_not_exist() {
    let temp = fixture();
    let target = temp.path().join("missing");
    let link = temp.path().join("logical");
    junction(&link, &target);
    let cancel = Cancellation::default();
    let provider = WindowsFileSystem::new(None);
    let mut source = provider
        .prepare_transfer_source_with_links(&link, CopyLinkPolicy::EmptyDirectories, &cancel)
        .unwrap();
    assert_eq!(source.entry().item.path, link);
    assert_eq!(source.entry().item.kind, EntryKind::Directory);
    assert!(source.next_child(&cancel).unwrap().is_none());
    assert!(source.open_child(OsStr::new("missing"), &cancel).is_err());
    let mut discovered = Vec::new();
    source
        .discover(&cancel, &mut |item| {
            discovered.push((item.path.clone(), item.kind));
        })
        .unwrap();
    assert_eq!(discovered, [(link.clone(), EntryKind::Directory)]);
    assert!(!target.exists());
    assert!(provider.prepare_transfer_source(&link, &cancel).is_err());
    assert!(
        provider
            .prepare_transfer_source_with_links(
                &link,
                CopyLinkPolicy::FollowDirectoryContents,
                &cancel
            )
            .is_err()
    );
    drop(source);
    fs::remove_dir(link).unwrap();
}

#[test]
fn follow_refuses_a_recursive_junction_during_iteration_and_discovery() {
    let temp = fixture();
    let source = temp.path().join("source");
    fs::create_dir(&source).unwrap();
    let link = source.join("cycle");
    junction(&link, &source);
    let cancel = Cancellation::default();
    let mut source = WindowsFileSystem::new(None)
        .prepare_transfer_source_with_links(
            &source,
            CopyLinkPolicy::FollowDirectoryContents,
            &cancel,
        )
        .unwrap();
    assert!(source.discover(&cancel, &mut |_| {}).is_err());
    assert!(source.next_child(&cancel).is_err());
    drop(source);
    fs::remove_dir(link).unwrap();
}

#[test]
fn sibling_junctions_share_a_referent_but_keep_distinct_original_identities() {
    let temp = fixture();
    let target = temp.path().join("target");
    let first = temp.path().join("first");
    let second = temp.path().join("second");
    fs::create_dir(&target).unwrap();
    junction(&first, &target);
    junction(&second, &target);
    let cancel = Cancellation::default();
    let provider = WindowsFileSystem::new(None);
    let prepare = |path: &Path| {
        provider
            .prepare_transfer_source_with_links(
                path,
                CopyLinkPolicy::FollowDirectoryContents,
                &cancel,
            )
            .unwrap()
    };
    let a = prepare(&first);
    let b = prepare(&second);
    assert_eq!(a.native_identity(), b.native_identity());
    assert_ne!(a.native_original_identity(), b.native_original_identity());
    assert_ne!(a.native_original_identity(), a.native_identity());
    assert_ne!(b.native_original_identity(), b.native_identity());
    drop((a, b));
    fs::remove_dir(first).unwrap();
    fs::remove_dir(second).unwrap();
}

#[test]
fn move_follow_option_preserves_an_ordinary_same_volume_native_rename() {
    let temp = fixture();
    let source = temp.path().join("ordinary");
    let destination_path = temp.path().join("destination");
    fs::write(&source, b"rename in place").unwrap();
    fs::create_dir(&destination_path).unwrap();
    let cancel = Cancellation::default();
    let provider = WindowsFileSystem::new(None);
    let PreparedTransferMoveEntry::Ordinary(prepared) = provider
        .prepare_move_source_with_links(&source, CopyLinkPolicy::FollowDirectoryContents, &cancel)
        .unwrap()
    else {
        panic!("ordinary source should keep native Move authority");
    };
    let identity = prepared.source.native_identity().unwrap();
    let destination = provider
        .prepare_transfer_destination(&destination_path, &cancel)
        .unwrap();
    assert!(matches!(
        prepared
            .capture
            .try_native_move(destination.as_ref(), OsStr::new("new"), None, &cancel)
            .unwrap(),
        TransferNativeMoveOutcome::Moved { recovery, .. } if recovery.is_empty()
    ));
    assert_eq!(
        destination
            .inspect(OsStr::new("new"), &cancel)
            .unwrap()
            .unwrap()
            .identity,
        identity
    );
    drop(prepared);
    assert!(!source.exists());
    assert_eq!(
        fs::read(destination_path.join("new")).unwrap(),
        b"rename in place"
    );
}

#[test]
fn move_projection_removes_only_its_original_junction_after_publication() {
    use midnight_platform_common::transfer::{CopyPhase, VerifiedFileCopy};

    let temp = fixture();
    let root = temp.path().join("source");
    let target = temp.path().join("target");
    let destination_path = temp.path().join("materialized");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&target).unwrap();
    fs::create_dir(&destination_path).unwrap();
    fs::write(target.join("sentinel"), b"referent data").unwrap();
    let link = root.join("junction");
    junction(&link, &target);
    let cancel = Cancellation::default();
    let provider = WindowsFileSystem::new(None);
    let PreparedTransferMoveEntry::Ordinary(mut root) = provider
        .prepare_move_source_with_links(&root, CopyLinkPolicy::FollowDirectoryContents, &cancel)
        .unwrap()
    else {
        panic!("expected ordinary Move root");
    };
    let mut discovered = Vec::new();
    root.source
        .discover(&cancel, &mut |item| discovered.push(item.path.clone()))
        .unwrap();
    assert_eq!(
        discovered,
        [
            link.parent().unwrap().to_owned(),
            link.clone(),
            link.join("sentinel")
        ]
    );
    let PreparedTransferMoveEntry::DirectoryLink {
        mut source,
        capture,
    } = root
        .source
        .next_move_child_with_links(CopyLinkPolicy::FollowDirectoryContents, &cancel)
        .unwrap()
        .unwrap()
    else {
        panic!("expected original-only directory link removal capability");
    };
    assert_eq!(capture.entry().item.kind, EntryKind::DirectoryLink);
    assert_eq!(
        Some(capture.entry().identity),
        source.native_original_identity()
    );
    assert_eq!(source.native_identity(), None);
    assert!(source.next_move_child(&cancel).is_err());
    assert!(source.next_child(&cancel).is_err());
    let mut projection = capture.prepare_copy_projection(&cancel).unwrap();
    assert_eq!(
        projection.native_original_identity(),
        Some(capture.entry().identity)
    );
    let projected_identity = projection.native_identity().unwrap();
    assert_ne!(projected_identity, capture.entry().identity);
    assert!(projection.next_move_child(&cancel).is_err());
    let file = projection.next_child(&cancel).unwrap().unwrap();
    assert_eq!(file.native_original_identity(), file.native_identity());
    let destination = provider
        .prepare_transfer_destination(&destination_path, &cancel)
        .unwrap();
    let mut copy = VerifiedFileCopy::new(file, destination.stage_file(&cancel).unwrap()).unwrap();
    while copy.phase() != CopyPhase::ReadyToPublish {
        copy.step(&cancel).unwrap();
    }
    copy.publish(OsStr::new("sentinel"), &cancel).unwrap();
    drop(copy);
    assert_eq!(
        fs::read(destination_path.join("sentinel")).unwrap(),
        b"referent data"
    );
    let interrupted = Cancellation::default();
    interrupted.cancel();
    assert_eq!(
        capture.remove_original(&interrupted).unwrap_err().kind,
        std::io::ErrorKind::Interrupted
    );
    assert!(fs::symlink_metadata(&link).is_ok());
    // The removal closes the link's deleting handle: the link is gone, its folder saved,
    // while the capture, its projection and the source are still held.
    assert!(matches!(
        capture.remove_original(&cancel).unwrap(),
        TransferDirectoryLinkRemoval::Removed {
            state: TransferSourceRemoval::Unlinked,
            durability_confirmed: true,
            recovery,
            error: None,
        } if recovery.is_empty()
    ));
    assert!(fs::symlink_metadata(&link).is_err());
    drop((capture, projection, source, root));
    assert!(!link.exists());
    assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"referent data");
}

#[test]
fn move_projection_rejects_aliased_parent_and_retains_both_targets() {
    let temp = fixture();
    let parent = temp.path().join("original-parent");
    let replacement = temp.path().join("replacement-parent");
    let target = temp.path().join("target");
    for path in [&parent, &replacement, &target] {
        fs::create_dir(path).unwrap();
    }
    let original_link = parent.join("junction");
    junction(&original_link, &target);
    fs::write(replacement.join("junction"), b"replacement entry").unwrap();
    fs::write(target.join("sentinel"), b"original target").unwrap();

    let cancel = Cancellation::default();
    let provider = WindowsFileSystem::new(None);
    let prepare = |path: &Path| {
        provider.prepare_move_source_with_links(
            path,
            CopyLinkPolicy::FollowDirectoryContents,
            &cancel,
        )
    };
    let PreparedTransferMoveEntry::DirectoryLink { source, capture } =
        prepare(&original_link).unwrap()
    else {
        panic!("expected directory link capability through an ordinary parent");
    };
    capture.revalidate(&cancel).unwrap();

    let alias = temp.path().join("parent-alias");
    let retired_alias = temp.path().join("retired-alias");
    junction(&alias, &parent);
    assert_eq!(
        fs::read(alias.join("junction/sentinel")).unwrap(),
        b"original target"
    );
    match prepare(&alias.join("junction")) {
        Err(error) => assert_eq!(error.kind, std::io::ErrorKind::Unsupported),
        Ok(_) => panic!("a junction ancestor must fail before Move is admitted"),
    }
    fs::rename(&alias, &retired_alias).unwrap();
    junction(&alias, &replacement);
    capture.revalidate(&cancel).unwrap();
    assert!(fs::symlink_metadata(&original_link).is_ok());
    assert_eq!(
        fs::read(alias.join("junction")).unwrap(),
        b"replacement entry"
    );
    assert_eq!(
        fs::read(target.join("sentinel")).unwrap(),
        b"original target"
    );
    drop((capture, source));
    fs::remove_dir(original_link).unwrap();
    fs::remove_dir(alias).unwrap();
    fs::remove_dir(retired_alias).unwrap();
}
