//! Link texts resolved within the link's mount, and the budget of followed
//! links. The kernel resolves a text itself whenever it can, so these call the
//! walk directly: it must resolve as the kernel does on any mount.
use super::*;
use std::{fs, os::unix::fs::symlink};

fn fixture() -> tempfile::TempDir {
    let root =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/linux-copy-directory-links");
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root.canonicalize().unwrap()).unwrap()
}

fn identity_at(path: &Path) -> TransferIdentity {
    identity_of(&fs::metadata(path).unwrap())
}

#[test]
fn a_link_text_is_walked_within_its_mount_as_the_kernel_resolves_it() {
    let fixture = fixture();
    let folder = fixture.path().join("folder");
    let inner = folder.join("inner 文件夹");
    fs::create_dir_all(&inner).unwrap();
    fs::write(inner.join("leaf.txt"), b"leaf").unwrap();
    symlink("inner 文件夹", folder.join("relative")).unwrap();
    symlink(&inner, folder.join("absolute")).unwrap();
    symlink(folder.join("absolute"), folder.join("chain")).unwrap();
    symlink("../folder/relative/leaf.txt", folder.join("up")).unwrap();
    symlink("loop", folder.join("loop")).unwrap();
    symlink("missing", folder.join("dangling")).unwrap();
    symlink("/proc", folder.join("to-proc")).unwrap();
    let parent = File::from(openat(CWD, &folder, PATH_DIRECTORY, Mode::empty()).unwrap());
    let anchor = MountAnchor::capture(&parent, &folder, &Cancellation::default()).unwrap();
    let walk = |text: &OsStr, directory: bool| {
        anchor
            .walk(&parent, text, directory, &folder, false)
            .unwrap()
            .map(|target| identity(&target, &folder).unwrap())
    };
    let reached = |text: &str| walk(OsStr::new(text), false);
    let leaf = Ok(identity_at(&inner.join("leaf.txt")));
    assert_eq!(reached("relative"), Ok(identity_at(&inner)));
    assert_eq!(walk(inner.as_os_str(), true), Ok(identity_at(&inner)));
    assert_eq!(reached("chain/leaf.txt"), leaf);
    assert_eq!(reached("absolute/../relative/./leaf.txt"), leaf);
    assert_eq!(reached("up"), leaf);
    assert_eq!(walk(inner.join("leaf.txt").as_os_str(), false), leaf);
    assert_eq!(reached("loop"), Err(Errno::LOOP));
    assert_eq!(reached("dangling"), Err(Errno::NOENT));
    assert_eq!(reached(""), Err(Errno::NOENT));
    assert_eq!(reached("relative/leaf.txt/"), Err(Errno::NOTDIR));
    assert_eq!(walk(OsStr::new("up"), true), Err(Errno::NOTDIR));
    // procfs is another mount, named directly or reached through a link.
    assert_eq!(reached("/proc/self"), Err(Errno::XDEV));
    assert_eq!(reached("to-proc/self"), Err(Errno::XDEV));
}

#[test]
fn only_links_followed_inside_one_another_count_against_the_budget() {
    let identity = |object: u64| TransferIdentity {
        device: 1,
        object: {
            let mut bytes = [0; 16];
            bytes[..8].copy_from_slice(&object.to_le_bytes());
            bytes
        },
    };
    let path = Path::new("/folder");
    for mode in [DirectoryLinkMode::Preserve, DirectoryLinkMode::Empty] {
        let mut traversal = Traversal::new(mode);
        for object in 0..1_000 {
            traversal
                .enter_directory(identity(object), path, object % 2 == 0)
                .unwrap();
        }
        assert!(traversal.directories.is_none(), "no ancestry to keep");
    }
    for mode in [DirectoryLinkMode::Follow, DirectoryLinkMode::FollowLinks] {
        let mut traversal = Traversal::new(mode);
        // Ordinary folders are not counted, however deep.
        for object in 0..100_000 {
            traversal
                .enter_directory(identity(object), path, false)
                .unwrap();
        }
        for object in 200_000..200_000 + MAX_LINK_DEPTH as u64 {
            traversal
                .enter_directory(identity(object), path, true)
                .unwrap();
        }
        let refused = traversal
            .enter_directory(identity(300_000), path, true)
            .unwrap_err();
        assert_eq!(refused.kind, io::ErrorKind::Unsupported);
        assert!(refused.message.contains("128 links"), "{}", refused.message);
        // A followed link back to any folder on the path is a cycle.
        let cycle = traversal
            .enter_directory(identity(3), path, true)
            .unwrap_err();
        assert_eq!(cycle.kind, io::ErrorKind::InvalidInput);
        // A path of 100,128 folders is released one folder after another; a
        // nested drop per folder would overflow the test thread's stack.
        drop(traversal);
    }
}
