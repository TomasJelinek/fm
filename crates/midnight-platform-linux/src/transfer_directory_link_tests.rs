use super::*;
use crate::LinuxFileSystem;
use midnight_core::CopyLinkPolicy;
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    os::unix::{ffi::OsStringExt, fs::symlink},
};

fn fixture() -> tempfile::TempDir {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/linux-copy-directory-links");
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root.canonicalize().unwrap()).unwrap()
}

#[test]
fn followed_aliases_keep_logical_paths_and_independent_discovery() {
    let fixture = fixture();
    let root = fixture.path().join("selected");
    let target_name = OsString::from_vec(b"target_\xff".to_vec());
    let target = fixture.path().join(&target_name);
    let first_alias = root.join("alias_e\u{301}");
    let second_alias = root.join("same target 🧪");
    let leaf = "nested_文档.txt";
    fs::create_dir(&root).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(target.join(leaf), b"payload").unwrap();
    let stored = Path::new("..").join(&target_name);
    symlink(&stored, &first_alias).unwrap();
    symlink(&stored, &second_alias).unwrap();

    let cancel = Cancellation::default();
    let filesystem = LinuxFileSystem::new(None);
    let mut source = filesystem
        .prepare_transfer_source_with_links(&root, CopyLinkPolicy::FollowDirectoryContents, &cancel)
        .unwrap();
    let first = source.next_child(&cancel).unwrap().unwrap();
    let mut discovered = Vec::new();
    source
        .discover(&cancel, &mut |item| discovered.push(item.clone()))
        .unwrap();
    let second = source.next_child(&cancel).unwrap().unwrap();
    assert!(source.next_child(&cancel).unwrap().is_none());
    assert_eq!(
        BTreeSet::from([
            first.entry().item.path.clone(),
            second.entry().item.path.clone(),
        ]),
        BTreeSet::from([first_alias.clone(), second_alias.clone()])
    );
    let expected_files = BTreeSet::from([first_alias.join(leaf), second_alias.join(leaf)]);
    assert_eq!(
        discovered
            .iter()
            .filter(|item| item.kind == EntryKind::File && item.metadata.size == Some(7))
            .map(|item| item.path.clone())
            .collect::<BTreeSet<_>>(),
        expected_files
    );
    assert_eq!(discovered.len(), 5);
    let physical = identity_of(&fs::metadata(&target).unwrap());
    for alias in [&first, &second] {
        assert_eq!(alias.entry().item.kind, EntryKind::Directory);
        assert_eq!(alias.native_identity(), Some(physical));
        let stub = identity_of(&fs::symlink_metadata(&alias.entry().item.path).unwrap());
        assert_eq!(alias.native_original_identity(), Some(stub));
        let child = alias.open_child(OsStr::new(leaf), &cancel).unwrap();
        assert_eq!(child.entry().item.path, alias.entry().item.path.join(leaf));
    }
    let mut selected_alias = filesystem
        .prepare_transfer_source_with_links(
            &first_alias,
            CopyLinkPolicy::FollowDirectoryContents,
            &cancel,
        )
        .unwrap();
    let mut selected_child = selected_alias.next_child(&cancel).unwrap().unwrap();
    let mut buffer = [0; 7];
    assert_eq!(selected_child.read_chunk(&mut buffer, &cancel).unwrap(), 7);
    assert_eq!(&buffer, b"payload");
}

#[test]
fn empty_projection_keeps_the_original_identity_and_yields_no_children() {
    let fixture = fixture();
    let target = fixture.path().join("directory");
    let alias = fixture.path().join("alias");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("child"), b"untouched").unwrap();
    symlink("directory", &alias).unwrap();

    let cancel = Cancellation::default();
    let mut source = LinuxFileSystem::new(None)
        .prepare_transfer_source_with_links(&alias, CopyLinkPolicy::EmptyDirectories, &cancel)
        .unwrap();
    let original = identity_of(&fs::symlink_metadata(&alias).unwrap());
    assert_eq!(source.entry().item.kind, EntryKind::Directory);
    assert_eq!(source.entry().item.path, alias);
    assert_eq!(source.native_identity(), Some(original));
    assert_eq!(source.native_original_identity(), Some(original));
    assert!(source.next_child(&cancel).unwrap().is_none());
    assert!(source.open_child(OsStr::new("child"), &cancel).is_err());
    let mut discovered = Vec::new();
    source
        .discover(&cancel, &mut |item| discovered.push(item.path.clone()))
        .unwrap();
    assert_eq!(discovered, [alias]);
}

#[test]
fn a_replaced_link_binding_invalidates_already_opened_descendants() {
    let fixture = fixture();
    let directory = fixture.path().join("source");
    let target = fixture.path().join("target");
    let nested_target = fixture.path().join("nested target");
    let alias = directory.join("alias");
    fs::create_dir(&directory).unwrap();
    fs::create_dir(&target).unwrap();
    fs::create_dir(&nested_target).unwrap();
    fs::write(nested_target.join("child"), b"same referent").unwrap();
    symlink("../target", &alias).unwrap();
    symlink("../nested target", target.join("nested alias")).unwrap();

    let cancel = Cancellation::default();
    let parent = LinuxFileSystem::new(None)
        .prepare_transfer_source_with_links(
            &directory,
            CopyLinkPolicy::FollowDirectoryContents,
            &cancel,
        )
        .unwrap();
    let projected = parent.open_child(OsStr::new("alias"), &cancel).unwrap();
    let nested = projected
        .open_child(OsStr::new("nested alias"), &cancel)
        .unwrap();
    let child = nested.open_child(OsStr::new("child"), &cancel).unwrap();
    fs::rename(&alias, directory.join("retained original")).unwrap();
    symlink("../target", &alias).unwrap();

    assert!(projected.revalidate(&cancel).is_err());
    assert!(child.revalidate(&cancel).is_err());
}

#[test]
fn a_rebound_target_fails_even_when_the_link_object_and_payload_stay_the_same() {
    let fixture = fixture();
    let target = fixture.path().join("target");
    let alias = fixture.path().join("alias");
    fs::create_dir(&target).unwrap();
    symlink("target", &alias).unwrap();

    let cancel = Cancellation::default();
    let source = LinuxFileSystem::new(None)
        .prepare_transfer_source_with_links(
            &alias,
            CopyLinkPolicy::FollowDirectoryContents,
            &cancel,
        )
        .unwrap();
    fs::rename(&target, fixture.path().join("retained target")).unwrap();
    fs::create_dir(&target).unwrap();

    assert!(source.revalidate(&cancel).is_err());
}

#[test]
fn an_ancestor_alias_cannot_create_a_recursive_projection() {
    let fixture = fixture();
    let directory = fixture.path().join("source");
    fs::create_dir(&directory).unwrap();
    symlink(".", directory.join("ancestor")).unwrap();

    let cancel = Cancellation::default();
    let filesystem = LinuxFileSystem::new(None);
    let mut follow = filesystem
        .prepare_transfer_source_with_links(
            &directory,
            CopyLinkPolicy::FollowDirectoryContents,
            &cancel,
        )
        .unwrap();
    assert!(follow.discover(&cancel, &mut |_| {}).is_err());
    assert!(follow.next_child(&cancel).is_err());

    let empty = filesystem
        .prepare_transfer_source_with_links(&directory, CopyLinkPolicy::EmptyDirectories, &cancel)
        .unwrap();
    let mut alias = empty.open_child(OsStr::new("ancestor"), &cancel).unwrap();
    assert_eq!(alias.entry().item.kind, EntryKind::Directory);
    assert!(alias.next_child(&cancel).unwrap().is_none());
}

#[test]
fn legacy_modes_and_unqualified_links_refuse_native_projection() {
    let fixture = fixture();
    let target = fixture.path().join("directory");
    let directory_alias = fixture.path().join("directory alias");
    let file_alias = fixture.path().join("file alias");
    let missing_alias = fixture.path().join("missing alias");
    fs::create_dir(&target).unwrap();
    fs::write(fixture.path().join("file"), b"data").unwrap();
    symlink("directory", &directory_alias).unwrap();
    symlink("file", &file_alias).unwrap();
    symlink("missing", &missing_alias).unwrap();
    let cancel = Cancellation::default();
    let filesystem = LinuxFileSystem::new(None);

    assert!(
        filesystem
            .prepare_transfer_source(&directory_alias, &cancel)
            .is_err()
    );
    assert!(
        filesystem
            .prepare_transfer_source_with_links(&directory_alias, CopyLinkPolicy::Preserve, &cancel)
            .is_err()
    );
    for policy in [
        CopyLinkPolicy::FollowDirectoryContents,
        CopyLinkPolicy::EmptyDirectories,
    ] {
        for alias in [&file_alias, &missing_alias] {
            assert!(
                filesystem
                    .prepare_transfer_source_with_links(alias, policy, &cancel)
                    .is_err()
            );
        }
    }
}

#[test]
fn a_link_target_across_a_mount_boundary_is_refused() {
    let fixture = fixture();
    let alias = fixture.path().join("external mount");
    symlink("/proc", &alias).unwrap();

    let result = LinuxFileSystem::new(None).prepare_transfer_source_with_links(
        &alias,
        CopyLinkPolicy::FollowDirectoryContents,
        &Cancellation::default(),
    );
    assert!(matches!(result, Err(failure) if failure.kind == io::ErrorKind::Unsupported));
}
