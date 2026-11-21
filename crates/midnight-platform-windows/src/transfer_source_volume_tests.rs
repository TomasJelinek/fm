//! A junction to a folder on another local volume is followed under
//! FollowDirectoryContents exactly as one on the same volume, and a junction back to a
//! folder that holds it is a cycle across volumes too. These need an owned folder on
//! another local volume than the repository's `_build`: NATIVE_MOVE_OTHER_VOLUME_ROOT,
//! as for the cross-volume Move tests; without it they print SKIP.
use super::*;
use crate::test_fixtures::{build_temp, create_junction};
use midnight_core::TransferService;
use std::fs;

/// A fresh folder below NATIVE_MOVE_OTHER_VOLUME_ROOT, or None after printing SKIP.
fn other_volume(test: &str) -> Option<tempfile::TempDir> {
    let Some(root) = std::env::var_os("NATIVE_MOVE_OTHER_VOLUME_ROOT") else {
        eprintln!("SKIP {test}: NATIVE_MOVE_OTHER_VOLUME_ROOT names no folder on another volume");
        return None;
    };
    Some(tempfile::tempdir_in(root).unwrap())
}

fn junction(link: &Path, target: &Path) {
    fs::create_dir(link).unwrap();
    create_junction(link, target).unwrap();
}

#[test]
fn a_junction_to_a_folder_on_another_volume_is_followed_and_a_cycle_back_refused() {
    let test = "a junction to another volume";
    let Some(other) = other_volume(test) else {
        return;
    };
    let here = build_temp("other-volume-");
    let target = other.path().join("target 📁");
    fs::create_dir_all(target.join("nested")).unwrap();
    fs::write(target.join("nested").join("leaf.txt"), b"on another volume").unwrap();
    let selected = here.path().join("selected");
    fs::create_dir(&selected).unwrap();
    let link = selected.join("away");
    junction(&link, &target);
    let cancel = Cancellation::default();
    let provider = crate::WindowsFileSystem::new(None);
    let volume = |path: &Path| {
        provider
            .prepare_transfer_source(path, &cancel)
            .unwrap()
            .native_identity()
            .unwrap()
            .device
    };
    if volume(&target) == volume(&selected) {
        eprintln!("SKIP {test}: NATIVE_MOVE_OTHER_VOLUME_ROOT is on the volume of _build");
        fs::remove_dir(&link).unwrap();
        return;
    }

    let mut tree = provider
        .prepare_transfer_source_with_links(
            &selected,
            CopyLinkPolicy::FollowDirectoryContents,
            &cancel,
        )
        .unwrap();
    let mut discovered = Vec::new();
    tree.discover(&cancel, &mut |item| {
        discovered.push((item.path.clone(), item.kind));
    })
    .unwrap();
    assert_eq!(
        discovered,
        [
            (selected.clone(), EntryKind::Directory),
            (link.clone(), EntryKind::Directory),
            (link.join("nested"), EntryKind::Directory),
            (link.join("nested").join("leaf.txt"), EntryKind::File),
        ]
    );
    let projected = tree.next_child(&cancel).unwrap().unwrap();
    assert_eq!(projected.entry().item.path, link);
    assert_eq!(projected.native_identity().unwrap().device, volume(&target));
    let nested = projected.open_child(OsStr::new("nested"), &cancel).unwrap();
    let mut leaf = nested.open_child(OsStr::new("leaf.txt"), &cancel).unwrap();
    let mut bytes = [0; 32];
    let read = leaf.read_chunk(&mut bytes, &cancel).unwrap();
    assert_eq!(&bytes[..read], b"on another volume");
    leaf.revalidate(&cancel).unwrap();
    drop((leaf, nested, projected, tree));

    // Back from the other volume to the folder that holds the first junction.
    let back = target.join("back");
    junction(&back, &selected);
    let mut cyclic = provider
        .prepare_transfer_source_with_links(
            &selected,
            CopyLinkPolicy::FollowDirectoryContents,
            &cancel,
        )
        .unwrap();
    assert!(cyclic.discover(&cancel, &mut |_| {}).is_err());
    let projected = cyclic.next_child(&cancel).unwrap().unwrap();
    let refused = projected
        .open_child(OsStr::new("back"), &cancel)
        .err()
        .expect("a junction back to an ancestor is refused");
    assert!(refused.message.contains("cycle"), "{}", refused.message);
    drop((projected, cyclic));
    fs::remove_dir(&back).unwrap();
    fs::remove_dir(&link).unwrap();
    assert_eq!(
        fs::read(target.join("nested").join("leaf.txt")).unwrap(),
        b"on another volume"
    );
}
