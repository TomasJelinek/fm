use super::*;
use crate::LinuxFileSystem;
use midnight_core::CopyLinkPolicy;
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    os::unix::{
        ffi::OsStringExt,
        fs::{PermissionsExt, symlink},
    },
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
    // Preserve never projects: the link itself is the source.
    let preserved = filesystem
        .prepare_transfer_source_with_links(&directory_alias, CopyLinkPolicy::Preserve, &cancel)
        .unwrap();
    assert_eq!(preserved.entry().item.kind, EntryKind::Symlink);
    assert_eq!(
        preserved.native_original_identity(),
        preserved.native_identity()
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

#[test]
fn follow_links_copies_a_file_link_as_its_target_content() {
    let fixture = fixture();
    fs::write(fixture.path().join("plain.txt"), b"plain text").unwrap();
    let alias = fixture.path().join("to-file");
    symlink("plain.txt", &alias).unwrap();
    let cancel = Cancellation::default();
    let mut source = LinuxFileSystem::new(None)
        .prepare_transfer_source_with_links(&alias, CopyLinkPolicy::FollowLinks, &cancel)
        .unwrap();
    let target = identity_of(&fs::metadata(fixture.path().join("plain.txt")).unwrap());
    let link = identity_of(&fs::symlink_metadata(&alias).unwrap());
    assert_eq!(source.entry().item.kind, EntryKind::File);
    assert_eq!(source.entry().item.path, alias);
    assert_eq!(source.entry().item.metadata.size, Some(10));
    assert_eq!(source.native_identity(), Some(target));
    assert_eq!(source.native_original_identity(), Some(link));
    assert!(source.link_not_followed().is_none());
    let mut buffer = [0; 32];
    let read = source.read_chunk(&mut buffer, &cancel).unwrap();
    assert_eq!(&buffer[..read], b"plain text");
    source.revalidate(&cancel).unwrap();
    // A changed target is a changed source, not a silent new revision.
    fs::write(fixture.path().join("plain.txt"), b"other content").unwrap();
    assert!(source.revalidate(&cancel).is_err());
}

#[test]
fn follow_links_keeps_links_it_cannot_follow_as_links_with_a_reason() {
    let fixture = fixture();
    let folder = fixture.path().join("folder");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("plain.txt"), b"plain").unwrap();
    symlink("missing", folder.join("dangling")).unwrap();
    symlink("loop", folder.join("loop")).unwrap();
    symlink(".", folder.join("to-self")).unwrap();
    symlink("plain.txt", folder.join("to-file")).unwrap();
    let cancel = Cancellation::default();
    let mut source = LinuxFileSystem::new(None)
        .prepare_transfer_source_with_links(&folder, CopyLinkPolicy::FollowLinks, &cancel)
        .unwrap();
    let mut discovered = Vec::new();
    source
        .discover(&cancel, &mut |item| {
            discovered.push((item.path.file_name().unwrap().to_owned(), item.kind))
        })
        .unwrap();
    discovered.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        discovered,
        [
            ("dangling".into(), EntryKind::Symlink),
            ("folder".into(), EntryKind::Directory),
            ("loop".into(), EntryKind::Symlink),
            ("plain.txt".into(), EntryKind::File),
            ("to-file".into(), EntryKind::File),
            ("to-self".into(), EntryKind::Symlink),
        ]
    );
    let mut children = Vec::new();
    while let Some(child) = source.next_child(&cancel).unwrap() {
        let name = child.entry().item.path.file_name().unwrap().to_owned();
        let note = child.link_not_followed();
        if child.entry().item.kind == EntryKind::Symlink {
            let representation = child.link_representation(&cancel).unwrap();
            assert!(representation.linux_target().is_some());
        }
        children.push((name, child.entry().item.kind, note));
    }
    let note = |name: &str| {
        children
            .iter()
            .find(|child| child.0 == name)
            .and_then(|child| child.2.clone())
            .unwrap_or_default()
    };
    assert!(note("dangling").contains("does not exist"), "{children:?}");
    assert!(note("loop").contains("itself"), "{children:?}");
    assert!(note("to-self").contains("contains it"), "{children:?}");
    assert!(note("to-file").is_empty() && note("plain.txt").is_empty());
}

#[test]
fn follow_links_keeps_a_link_to_another_mount_as_a_link() {
    let fixture = fixture();
    let alias = fixture.path().join("external mount");
    symlink("/proc", &alias).unwrap();
    let source = LinuxFileSystem::new(None)
        .prepare_transfer_source_with_links(
            &alias,
            CopyLinkPolicy::FollowLinks,
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(source.entry().item.kind, EntryKind::Symlink);
    assert!(
        source
            .link_not_followed()
            .is_some_and(|note| note.contains("another filesystem"))
    );
}

/// Absolute link texts name the fixture from `/`. On a fixture on its own mount
/// (as the build volume of the container) the kernel refuses them under
/// RESOLVE_NO_XDEV, and the walk within the mount follows them.
#[test]
fn follow_links_follows_absolute_links_to_targets_on_their_own_filesystem() {
    let fixture = fixture();
    let folder = fixture.path().join("folder");
    let target = fixture.path().join("target 📁");
    fs::create_dir(&folder).unwrap();
    fs::create_dir_all(target.join("nested")).unwrap();
    fs::write(
        target.join("nested").join("leaf.txt"),
        b"through an absolute link",
    )
    .unwrap();
    fs::write(fixture.path().join("plain.txt"), b"plain").unwrap();
    symlink(&target, folder.join("to-folder")).unwrap();
    symlink(fixture.path().join("plain.txt"), folder.join("to-file")).unwrap();
    // An absolute link to an absolute link.
    symlink(folder.join("to-folder"), folder.join("chain")).unwrap();
    let cancel = Cancellation::default();
    let mut source = LinuxFileSystem::new(None)
        .prepare_transfer_source_with_links(&folder, CopyLinkPolicy::FollowLinks, &cancel)
        .unwrap();
    let mut discovered = BTreeSet::new();
    source
        .discover(&cancel, &mut |item| {
            discovered.insert(item.path.strip_prefix(&folder).unwrap().to_owned());
        })
        .unwrap();
    for alias in ["to-folder", "chain"] {
        assert!(
            discovered.contains(&Path::new(alias).join("nested/leaf.txt")),
            "{discovered:?}"
        );
    }
    let mut children = Vec::new();
    while let Some(child) = source.next_child(&cancel).unwrap() {
        assert_eq!(child.link_not_followed(), None);
        children.push((child.entry().item.path.clone(), child.entry().item.kind));
    }
    children.sort_by(|left, right| left.0.cmp(&right.0));
    assert_eq!(
        children,
        [
            (folder.join("chain"), EntryKind::Directory),
            (folder.join("to-file"), EntryKind::File),
            (folder.join("to-folder"), EntryKind::Directory),
        ]
    );
    // Content below a followed folder is revalidated through the absolute link.
    let alias = source.open_child(OsStr::new("chain"), &cancel).unwrap();
    assert_eq!(
        alias.native_identity(),
        Some(identity_of(&fs::metadata(&target).unwrap()))
    );
    let nested = alias.open_child(OsStr::new("nested"), &cancel).unwrap();
    let mut leaf = nested.open_child(OsStr::new("leaf.txt"), &cancel).unwrap();
    let mut buffer = [0; 64];
    let read = leaf.read_chunk(&mut buffer, &cancel).unwrap();
    assert_eq!(&buffer[..read], b"through an absolute link");
    leaf.revalidate(&cancel).unwrap();
    nested.revalidate(&cancel).unwrap();
    let mut file = source.open_child(OsStr::new("to-file"), &cancel).unwrap();
    let read = file.read_chunk(&mut buffer, &cancel).unwrap();
    assert_eq!(&buffer[..read], b"plain");
    file.revalidate(&cancel).unwrap();
}

#[test]
fn follow_links_keeps_links_to_targets_it_cannot_read_as_links() {
    if rustix::process::geteuid().is_root() {
        // Permission bits do not restrict root.
        return;
    }
    let fixture = fixture();
    let folder = fixture.path().join("folder");
    fs::create_dir(&folder).unwrap();
    // Each can be opened as a path, but not copied: a folder that cannot be
    // listed, one that cannot be entered, and a file that cannot be read.
    let targets = [
        ("unlistable", 0o300, true),
        ("unenterable", 0o600, true),
        ("unreadable.txt", 0o200, false),
    ];
    for (name, mode, directory) in targets {
        let target = fixture.path().join(name);
        if directory {
            fs::create_dir(&target).unwrap();
        } else {
            fs::write(&target, b"secret").unwrap();
        }
        fs::set_permissions(&target, fs::Permissions::from_mode(mode)).unwrap();
        symlink(Path::new("..").join(name), folder.join(name)).unwrap();
    }
    let cancel = Cancellation::default();
    let outcome = (|| {
        let mut source = LinuxFileSystem::new(None).prepare_transfer_source_with_links(
            &folder,
            CopyLinkPolicy::FollowLinks,
            &cancel,
        )?;
        let mut discovered = 0;
        source.discover(&cancel, &mut |_| discovered += 1)?;
        let mut children = Vec::new();
        while let Some(child) = source.next_child(&cancel)? {
            children.push((
                child.entry().item.kind,
                child.link_not_followed().unwrap_or_default(),
            ));
        }
        Ok::<_, FsError>((discovered, children))
    })();
    for (name, _, _) in targets {
        fs::set_permissions(fixture.path().join(name), fs::Permissions::from_mode(0o700)).unwrap();
    }
    let (discovered, children) = outcome.unwrap();
    assert_eq!(discovered, 4, "the folder and three links");
    assert_eq!(children.len(), 3);
    for (kind, note) in children {
        assert_eq!(kind, EntryKind::Symlink, "{note}");
        assert!(note.contains("permission denied"), "{note}");
    }
}

/// Only links followed inside one another are limited; a Linux Copy always
/// sends Preserve or FollowLinks, so ordinary depth must not be.
#[test]
fn copy_traversals_have_no_depth_limit_for_ordinary_folders() {
    // The folders a transfer holds open are bounded by the process's open-file limit
    // instead (`descriptor_budget`); this test's own levels keep it independent of the
    // limit it runs under.
    static LEVELS: descriptor_budget::Levels = descriptor_budget::Levels::new(1024);
    let _levels = descriptor_budget::scoped(&LEVELS);
    let fixture = fixture();
    let root = fixture.path().join("deep");
    let mut bottom = root.clone();
    for _ in 0..150 {
        bottom.push("d");
    }
    fs::create_dir_all(&bottom).unwrap();
    fs::write(bottom.join("leaf.txt"), b"150 folders down").unwrap();
    symlink("leaf.txt", bottom.join("link")).unwrap();
    let cancel = Cancellation::default();
    for policy in [CopyLinkPolicy::Preserve, CopyLinkPolicy::FollowLinks] {
        let mut source = LinuxFileSystem::new(None)
            .prepare_transfer_source_with_links(&root, policy, &cancel)
            .unwrap();
        for _ in 0..150 {
            source = source.next_child(&cancel).unwrap().expect("a folder");
            assert_eq!(source.entry().item.kind, EntryKind::Directory);
        }
        let mut leaves = Vec::new();
        while let Some(mut child) = source.next_child(&cancel).unwrap() {
            let mut buffer = [0; 32];
            let content = if child.entry().item.kind == EntryKind::File {
                let read = child.read_chunk(&mut buffer, &cancel).unwrap();
                buffer[..read].to_vec()
            } else {
                Vec::new()
            };
            let name = child.entry().item.path.file_name().unwrap().to_owned();
            leaves.push((name, child.entry().item.kind, content));
        }
        leaves.sort_by(|left, right| left.0.cmp(&right.0));
        let content = b"150 folders down".to_vec();
        let link = if policy == CopyLinkPolicy::Preserve {
            (EntryKind::Symlink, Vec::new())
        } else {
            (EntryKind::File, content.clone())
        };
        assert_eq!(
            leaves,
            [
                (OsString::from("leaf.txt"), EntryKind::File, content),
                (OsString::from("link"), link.0, link.1)
            ],
            "{policy:?}"
        );
        // Recursive totals stop at their own budget, as without links; the
        // copy does not depend on them.
        let discovery = LinuxFileSystem::new(None)
            .prepare_transfer_source_with_links(&root, policy, &cancel)
            .unwrap()
            .discover(&cancel, &mut |_| {})
            .unwrap_err();
        assert!(
            discovery.message.contains("depth budget"),
            "{}",
            discovery.message
        );
    }
}
