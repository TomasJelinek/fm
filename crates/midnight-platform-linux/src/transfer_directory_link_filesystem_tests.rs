//! Links followed into another filesystem than their own folder's: a folder and a
//! file on the tmpfs at /dev/shm reached from links in the temporary folder, links
//! from there back, a cycle through both filesystems and the budget of links followed
//! inside one another. Each test prints SKIP where /dev/shm is not a second, writable
//! filesystem.
use super::*;
use crate::LinuxFileSystem;
use midnight_core::{TransferDirectory, TransferService};
use std::{
    collections::BTreeSet,
    fs,
    io::Write,
    os::unix::fs::{MetadataExt, symlink},
};

/// A private folder in the temporary folder and one on another filesystem
/// (/dev/shm), both canonical, or None after printing SKIP.
fn two_filesystems(test: &str) -> Option<(tempfile::TempDir, tempfile::TempDir)> {
    let here = crate::test_support::temporary("link-filesystem-");
    let other = crate::test_support::shm_temporary("link-filesystem-", test)?;
    if device(here.path()) == device(other.path()) {
        eprintln!("SKIP {test}: /dev/shm is on the temporary folder's filesystem");
        return None;
    }
    Some((here, other))
}

fn device(path: &Path) -> u64 {
    fs::metadata(path).unwrap().dev()
}

fn canonical(fixture: &tempfile::TempDir) -> PathBuf {
    fixture.path().canonicalize().unwrap()
}

/// Copies the children of `source` into `destination` as a Copy does: a folder is
/// created and filled, a file staged, written and published, and each source is
/// revalidated once it was read.
fn copy_children(
    source: &mut dyn TransferSource,
    destination: &dyn TransferDirectory,
    cancel: &Cancellation,
) {
    while let Some(mut child) = source.next_child(cancel).unwrap() {
        let path = child.entry().item.path.clone();
        assert_eq!(child.link_not_followed(), None, "{}", path.display());
        let name = path.file_name().unwrap().to_owned();
        match child.entry().item.kind {
            EntryKind::Directory => {
                let folder = destination.create_directory(&name, cancel).unwrap();
                copy_children(child.as_mut(), folder.as_ref(), cancel);
                child.revalidate(cancel).unwrap();
            }
            EntryKind::File => {
                let mut stage = destination.stage_file(cancel).unwrap();
                let mut buffer = [0; 4096];
                loop {
                    let read = child.read_chunk(&mut buffer, cancel).unwrap();
                    if read == 0 {
                        break;
                    }
                    stage.write_all(&buffer[..read]).unwrap();
                }
                child.revalidate(cancel).unwrap();
                stage.sync(cancel).unwrap();
                stage.publish_no_replace(&name, cancel).unwrap();
            }
            kind => panic!("{kind:?} at {}", path.display()),
        }
    }
}

/// Every entry below `root`, relative, with whether it is a folder; links not followed.
fn tree(root: &Path) -> BTreeSet<(PathBuf, bool)> {
    let mut entries = BTreeSet::new();
    let mut pending = vec![root.to_owned()];
    while let Some(folder) = pending.pop() {
        for entry in fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            let kind = fs::symlink_metadata(&path).unwrap().file_type();
            assert!(!kind.is_symlink(), "a link was copied: {}", path.display());
            if kind.is_dir() {
                pending.push(path.clone());
            }
            entries.insert((path.strip_prefix(root).unwrap().to_owned(), kind.is_dir()));
        }
    }
    entries
}

#[test]
fn links_into_another_filesystem_are_followed_and_their_targets_copied() {
    let Some((here, other)) = two_filesystems("links into another filesystem") else {
        return;
    };
    let (here_root, other_root) = (canonical(&here), canonical(&other));
    // On the other filesystem a folder and a file; on this one a folder to come back to.
    let target = other_root.join("target 📁");
    fs::create_dir_all(target.join("nested")).unwrap();
    fs::write(target.join("nested/leaf.txt"), b"on another filesystem").unwrap();
    fs::write(other_root.join("plain.txt"), b"a file on tmpfs").unwrap();
    let home = here_root.join("home");
    fs::create_dir(&home).unwrap();
    fs::write(home.join("back.txt"), b"back on the first filesystem").unwrap();
    // Below the target, a link back to this filesystem and one within its own.
    symlink(&home, target.join("back")).unwrap();
    symlink(other_root.join("plain.txt"), target.join("within")).unwrap();
    // Absolute links, and a relative one that climbs to `/` and down again.
    let folder = here_root.join("folder");
    fs::create_dir(&folder).unwrap();
    symlink(&target, folder.join("to-folder")).unwrap();
    symlink(other_root.join("plain.txt"), folder.join("to-file")).unwrap();
    let climb: PathBuf = folder
        .components()
        .skip(1)
        .map(|_| Component::ParentDir)
        .collect();
    symlink(
        climb.join(target.strip_prefix("/").unwrap()),
        folder.join("relative"),
    )
    .unwrap();

    let cancel = Cancellation::default();
    let filesystem = LinuxFileSystem::new(None);
    let mut source = filesystem
        .prepare_transfer_source_with_links(&folder, CopyLinkPolicy::FollowLinks, &cancel)
        .unwrap();
    // Discovery totals what the copy reads, through every link.
    let mut discovered = BTreeSet::new();
    source
        .discover(&cancel, &mut |item| {
            let path = item.path.strip_prefix(&folder).unwrap().to_owned();
            discovered.insert((path, item.kind == EntryKind::Directory));
        })
        .unwrap();
    let target_tree = [
        ("", true),
        ("nested", true),
        ("nested/leaf.txt", false),
        ("back", true),
        ("back/back.txt", false),
        ("within", false),
    ];
    let mut expected = BTreeSet::from([(PathBuf::new(), true), ("to-file".into(), false)]);
    for alias in ["to-folder", "relative"] {
        for (path, is_folder) in target_tree {
            expected.insert((Path::new(alias).join(path), is_folder));
        }
    }
    let expected_copy: BTreeSet<_> = expected
        .iter()
        .filter(|(path, _)| !path.as_os_str().is_empty())
        .cloned()
        .collect();
    assert_eq!(discovered, expected);

    // The folder copied through the platform's own staging: no link arrives.
    let copy = here_root.join("copy");
    fs::create_dir(&copy).unwrap();
    let destination = filesystem
        .prepare_transfer_destination(&copy, &cancel)
        .unwrap();
    copy_children(source.as_mut(), destination.as_ref(), &cancel);
    assert_eq!(tree(&copy), expected_copy);
    for alias in ["to-folder", "relative"] {
        let copied = copy.join(alias);
        assert_eq!(
            fs::read(copied.join("nested/leaf.txt")).unwrap(),
            b"on another filesystem"
        );
        assert_eq!(fs::read(copied.join("within")).unwrap(), b"a file on tmpfs");
        assert_eq!(
            fs::read(copied.join("back/back.txt")).unwrap(),
            b"back on the first filesystem"
        );
    }
    assert_eq!(fs::read(copy.join("to-file")).unwrap(), b"a file on tmpfs");

    // A followed folder is the target itself, held on its own filesystem.
    let source = filesystem
        .prepare_transfer_source_with_links(&folder, CopyLinkPolicy::FollowLinks, &cancel)
        .unwrap();
    let alias = source.open_child(OsStr::new("to-folder"), &cancel).unwrap();
    assert_eq!(
        alias.native_identity(),
        Some(identity_of(&fs::metadata(&target).unwrap()))
    );
    assert_eq!(alias.native_identity().unwrap().device, device(&other_root));
    let nested = alias.open_child(OsStr::new("nested"), &cancel).unwrap();
    let leaf = nested.open_child(OsStr::new("leaf.txt"), &cancel).unwrap();
    leaf.revalidate(&cancel).unwrap();
    // Another folder under the target's name: the link no longer leads to what is
    // being copied, and everything below it is refused.
    fs::rename(&target, other_root.join("moved away")).unwrap();
    fs::create_dir_all(target.join("nested")).unwrap();
    fs::write(target.join("nested/leaf.txt"), b"on another filesystem").unwrap();
    assert!(alias.revalidate(&cancel).is_err());
    assert!(nested.revalidate(&cancel).is_err());
    assert!(leaf.revalidate(&cancel).is_err());
}

/// The outermost link's folder on another mount than the root's: a link to its
/// own filesystem is walked within it, and one back to the root's filesystem is
/// followed across, however the kernel would start an absolute text.
#[test]
fn links_from_another_filesystem_lead_within_it_and_back() {
    let Some((here, other)) = two_filesystems("links from another filesystem") else {
        return;
    };
    let (here_root, other_root) = (canonical(&here), canonical(&other));
    let home = here_root.join("home");
    fs::create_dir_all(home.join("inner")).unwrap();
    fs::write(home.join("inner/back.txt"), b"on the first filesystem").unwrap();
    let target = other_root.join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("own.txt"), b"on the same tmpfs").unwrap();
    let folder = other_root.join("folder");
    fs::create_dir(&folder).unwrap();
    symlink(&target, folder.join("inside")).unwrap();
    symlink(&home, folder.join("to-home")).unwrap();
    symlink("../target/own.txt", folder.join("relative.txt")).unwrap();

    let cancel = Cancellation::default();
    let filesystem = LinuxFileSystem::new(None);
    let mut source = filesystem
        .prepare_transfer_source_with_links(&folder, CopyLinkPolicy::FollowLinks, &cancel)
        .unwrap();
    let copy = here_root.join("copy");
    fs::create_dir(&copy).unwrap();
    let destination = filesystem
        .prepare_transfer_destination(&copy, &cancel)
        .unwrap();
    copy_children(source.as_mut(), destination.as_ref(), &cancel);
    let expected = [
        ("inside", true),
        ("inside/own.txt", false),
        ("relative.txt", false),
        ("to-home", true),
        ("to-home/inner", true),
        ("to-home/inner/back.txt", false),
    ]
    .map(|(path, is_folder)| (PathBuf::from(path), is_folder));
    assert_eq!(tree(&copy), BTreeSet::from(expected));
    assert_eq!(
        fs::read(copy.join("inside/own.txt")).unwrap(),
        b"on the same tmpfs"
    );
    assert_eq!(
        fs::read(copy.join("relative.txt")).unwrap(),
        b"on the same tmpfs"
    );
    assert_eq!(
        fs::read(copy.join("to-home/inner/back.txt")).unwrap(),
        b"on the first filesystem"
    );
}

#[test]
fn a_cycle_through_another_filesystem_is_kept_as_a_link() {
    let Some((here, other)) = two_filesystems("a cycle through another filesystem") else {
        return;
    };
    let (here_root, other_root) = (canonical(&here), canonical(&other));
    let start = here_root.join("start");
    let away = other_root.join("away");
    fs::create_dir(&start).unwrap();
    fs::create_dir(&away).unwrap();
    fs::write(away.join("file.txt"), b"copied once").unwrap();
    symlink(&away, start.join("to-away")).unwrap();
    symlink(&start, away.join("to-start")).unwrap();

    let cancel = Cancellation::default();
    let filesystem = LinuxFileSystem::new(None);
    let mut source = filesystem
        .prepare_transfer_source_with_links(&start, CopyLinkPolicy::FollowLinks, &cancel)
        .unwrap();
    let mut discovered = Vec::new();
    source
        .discover(&cancel, &mut |item| {
            discovered.push((
                item.path.strip_prefix(&start).unwrap().to_owned(),
                item.kind,
            ));
        })
        .unwrap();
    discovered.sort_by(|left, right| left.0.cmp(&right.0));
    let expected = [
        ("", EntryKind::Directory),
        ("to-away", EntryKind::Directory),
        ("to-away/file.txt", EntryKind::File),
        ("to-away/to-start", EntryKind::Symlink),
    ]
    .map(|(path, kind)| (PathBuf::from(path), kind));
    assert_eq!(discovered, expected);
    let mut followed = source.next_child(&cancel).unwrap().unwrap();
    assert_eq!(followed.entry().item.kind, EntryKind::Directory);
    assert_eq!(followed.native_identity().unwrap().device, device(&away));
    let mut cycle = None;
    while let Some(child) = followed.next_child(&cancel).unwrap() {
        if child.entry().item.kind == EntryKind::Symlink {
            cycle = child.link_not_followed();
        }
    }
    let cycle = cycle.expect("the link back is kept as a link");
    assert!(cycle.contains("contains it"), "{cycle}");

    // FollowDirectoryContents refuses the same cycle as an error.
    let mut projected = filesystem
        .prepare_transfer_source_with_links(
            &start,
            CopyLinkPolicy::FollowDirectoryContents,
            &cancel,
        )
        .unwrap();
    assert!(projected.discover(&cancel, &mut |_| {}).is_err());
    let mut followed = projected.next_child(&cancel).unwrap().unwrap();
    let refused = loop {
        match followed.next_child(&cancel) {
            Ok(Some(_)) => {}
            Ok(None) => panic!("the link back to an ancestor was followed"),
            Err(error) => break error,
        }
    };
    assert_eq!(refused.kind, io::ErrorKind::InvalidInput, "{refused}");
}

/// Folders on alternating filesystems, each holding a link to the next: every
/// followed link counts against the budget, and every followed folder is one
/// level of the descriptor budget.
#[test]
fn links_followed_across_filesystems_count_against_the_budget() {
    static LEVELS: descriptor_budget::Levels = descriptor_budget::Levels::new(64);
    let _levels = descriptor_budget::scoped(&LEVELS);
    let Some((here, other)) = two_filesystems("the link budget across filesystems") else {
        return;
    };
    let (here_root, other_root) = (canonical(&here), canonical(&other));
    let folder = |index: usize| {
        let root = if index.is_multiple_of(2) {
            &here_root
        } else {
            &other_root
        };
        root.join(format!("folder {index}"))
    };
    for index in 0..=MAX_LINK_DEPTH + 1 {
        fs::create_dir(folder(index)).unwrap();
    }
    for index in 0..=MAX_LINK_DEPTH {
        symlink(folder(index + 1), folder(index).join("next")).unwrap();
    }
    let cancel = Cancellation::default();
    let mut source = LinuxFileSystem::new(None)
        .prepare_transfer_source_with_links(&folder(0), CopyLinkPolicy::FollowLinks, &cancel)
        .unwrap();
    for index in 1..=MAX_LINK_DEPTH {
        source = source.open_child(OsStr::new("next"), &cancel).unwrap();
        assert_eq!(source.entry().item.kind, EntryKind::Directory);
        assert_eq!(
            source.native_identity().unwrap().device,
            device(&folder(index))
        );
        // The folder before it was released.
        assert_eq!(LEVELS.held(), 1);
    }
    let refused = source
        .open_child(OsStr::new("next"), &cancel)
        .err()
        .expect("the link past the budget is refused");
    assert_eq!(refused.kind, io::ErrorKind::Unsupported);
    assert!(refused.message.contains("128 links"), "{}", refused.message);
}

/// The kernel's own interface filesystems are named by the `statfs` type that their
/// sources give their superblocks, so no link is followed into one: Xen's hypervisor
/// interface (`XENFS_SUPER_MAGIC`, linux/magic.h) and the USB gadget filesystems
/// (`FUNCTIONFS_MAGIC` in drivers/usb/gadget/function/f_fs.c, `GADGETFS_MAGIC` in
/// drivers/usb/gadget/legacy/inode.c), whose endpoint files block or change the device.
#[test]
fn xen_and_usb_gadget_interface_filesystems_are_system_filesystems() {
    for (magic, name) in [
        (0xabba_1974, "xenfs"),
        (0x0a64_7361, "functionfs"),
        (0xaee7_1ee7, "gadgetfs"),
    ] {
        assert_eq!(system_filesystem_named(magic), Some(name), "{magic:#x}");
    }
    // Filesystems that hold files are followed: ext4 and tmpfs.
    for magic in [0xef53, 0x0102_1994] {
        assert_eq!(system_filesystem_named(magic), None, "{magic:#x}");
    }
}
