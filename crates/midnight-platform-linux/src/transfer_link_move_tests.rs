//! Move of preserved links: a rename of the link itself on one filesystem, and the
//! recreation that replaces it across filesystems, exercised here on one filesystem.
use super::*;
use crate::LinuxFileSystem;
use midnight_core::{CopyLinkPolicy, TransferService};
use std::{fs, os::unix::fs::symlink};

fn fixture() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/linux-move-links");
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root.canonicalize().unwrap()).unwrap()
}

fn private_names(directory: &Path) -> Vec<OsString> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| name.as_bytes().starts_with(b".midnight-"))
        .collect()
}

/// The provider's own capture of a preserved link, for the cross-filesystem branch.
fn concrete(link: &Path) -> Capture {
    let cancel = Cancellation::default();
    let parent_path = link.parent().unwrap();
    let parent_file = Arc::new(File::open(parent_path).unwrap());
    let source = source::Source::open_below(
        parent_file.clone(),
        parent_path.to_owned(),
        link.file_name().unwrap(),
        None,
        directory_links::Traversal::for_policy(CopyLinkPolicy::Preserve),
        &cancel,
    )
    .unwrap();
    let parent =
        destination::Directory::from_file(parent_file.try_clone().unwrap(), parent_path.to_owned())
            .unwrap();
    Capture::prepare(source.link_leaf().unwrap(), parent, &cancel).unwrap()
}

#[test]
fn same_filesystem_move_renames_every_kind_of_link_itself() {
    let fixture = fixture();
    let source = fixture.path().join("source");
    let destination = fixture.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    fs::create_dir(fixture.path().join("folder")).unwrap();
    fs::write(fixture.path().join("folder/inside.txt"), b"inside").unwrap();
    fs::write(fixture.path().join("file.txt"), b"content").unwrap();
    let cancel = Cancellation::default();
    let filesystem = LinuxFileSystem::new(None);
    let directory = filesystem
        .prepare_move_destination(&destination, &cancel)
        .unwrap();
    for (name, target) in [
        ("to-folder", "../folder"),
        ("to-file", "../file.txt"),
        ("dangling", "../does-not-exist"),
        ("to-self", "."),
    ] {
        let link = source.join(name);
        symlink(target, &link).unwrap();
        let PreparedTransferMoveEntry::Ordinary(prepared) = filesystem
            .prepare_move_source_with_links(&link, CopyLinkPolicy::Preserve, &cancel)
            .unwrap()
        else {
            panic!("{name}: a preserved link is moved itself");
        };
        assert_eq!(prepared.source.entry().item.kind, EntryKind::Symlink);
        assert_eq!(
            prepared.capture.entry().identity,
            prepared.source.entry().identity
        );
        let outcome = prepared
            .capture
            .try_native_move(directory.as_ref(), OsStr::new(name), None, &cancel)
            .unwrap();
        assert!(
            matches!(&outcome, TransferNativeMoveOutcome::Moved { destination: moved, .. }
                if *moved == destination.join(name)),
            "{name}: {outcome:?}"
        );
        assert!(
            fs::symlink_metadata(&link).is_err(),
            "{name} left its source"
        );
        assert_eq!(
            fs::read_link(destination.join(name)).unwrap(),
            Path::new(target)
        );
    }
    assert!(private_names(&source).is_empty());
    assert!(private_names(&destination).is_empty());
    assert_eq!(
        fs::read(fixture.path().join("file.txt")).unwrap(),
        b"content"
    );
    assert_eq!(
        fs::read(fixture.path().join("folder/inside.txt")).unwrap(),
        b"inside"
    );
}

#[test]
fn recreation_publishes_the_verified_link_before_removing_the_original() {
    let fixture = fixture();
    let source = fixture.path().join("source");
    let destination = fixture.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    let link = source.join("link");
    symlink("../relative/target", &link).unwrap();
    let cancel = Cancellation::default();
    let directory = LinuxFileSystem::new(None)
        .prepare_move_destination(&destination, &cancel)
        .unwrap();
    let outcome = concrete(&link)
        .recreate(directory.as_ref(), OsStr::new("link"), None, &cancel)
        .unwrap();
    assert!(
        matches!(&outcome, TransferNativeMoveOutcome::Moved { destination: moved, recovery, .. }
            if *moved == destination.join("link") && recovery.is_empty()),
        "{outcome:?}"
    );
    assert!(fs::symlink_metadata(&link).is_err());
    assert_eq!(
        fs::read_link(destination.join("link")).unwrap(),
        Path::new("../relative/target")
    );
    assert!(private_names(&source).is_empty());
    assert!(private_names(&destination).is_empty());
}

#[test]
fn recreation_never_replaces_an_existing_name_without_consent() {
    let fixture = fixture();
    let source = fixture.path().join("source");
    let destination = fixture.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    let link = source.join("link");
    symlink("target", &link).unwrap();
    fs::write(destination.join("link"), b"existing").unwrap();
    let cancel = Cancellation::default();
    let directory = LinuxFileSystem::new(None)
        .prepare_move_destination(&destination, &cancel)
        .unwrap();
    let error = concrete(&link)
        .recreate(directory.as_ref(), OsStr::new("link"), None, &cancel)
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::AlreadyExists, "{error:?}");
    assert_eq!(fs::read_link(&link).unwrap(), Path::new("target"));
    assert_eq!(fs::read(destination.join("link")).unwrap(), b"existing");
    assert!(private_names(&destination).is_empty());
}

#[test]
fn recreation_with_consent_keeps_the_replaced_file_in_recovery() {
    let fixture = fixture();
    let source = fixture.path().join("source");
    let destination = fixture.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    let link = source.join("link");
    symlink("target", &link).unwrap();
    fs::write(destination.join("link"), b"displaced").unwrap();
    let cancel = Cancellation::default();
    let directory = LinuxFileSystem::new(None)
        .prepare_move_destination(&destination, &cancel)
        .unwrap();
    if !directory.capabilities().link_replacement {
        eprintln!("SKIP link replacement: the fixture filesystem is not ext");
        return;
    }
    let target = directory
        .prepare_target(OsStr::new("link"), &cancel)
        .unwrap()
        .unwrap();
    let outcome = concrete(&link)
        .recreate(
            directory.as_ref(),
            OsStr::new("link"),
            Some((target, TransferReplacementPolicy::RecoverableCurrentEntry)),
            &cancel,
        )
        .unwrap();
    let TransferNativeMoveOutcome::Moved { recovery, .. } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(recovery.len(), 1);
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"displaced");
    assert_eq!(
        fs::read_link(destination.join("link")).unwrap(),
        Path::new("target")
    );
    assert!(fs::symlink_metadata(&link).is_err());
}

#[test]
fn same_filesystem_move_replaces_a_consented_file_and_keeps_it() {
    let fixture = fixture();
    let source = fixture.path().join("source");
    let destination = fixture.path().join("destination");
    fs::create_dir(&source).unwrap();
    fs::create_dir(&destination).unwrap();
    let link = source.join("link");
    symlink("elsewhere", &link).unwrap();
    fs::write(destination.join("link"), b"displaced").unwrap();
    let cancel = Cancellation::default();
    let filesystem = LinuxFileSystem::new(None);
    let directory = filesystem
        .prepare_move_destination(&destination, &cancel)
        .unwrap();
    let PreparedTransferMoveEntry::Ordinary(prepared) = filesystem
        .prepare_move_source_with_links(&link, CopyLinkPolicy::Preserve, &cancel)
        .unwrap()
    else {
        panic!("a preserved link is moved itself");
    };
    if !directory.supports_native_move_replacement(prepared.source.entry().identity) {
        eprintln!("SKIP native link replacement: unqualified fixture filesystem");
        return;
    }
    let target = directory
        .prepare_target(OsStr::new("link"), &cancel)
        .unwrap()
        .unwrap();
    let outcome = prepared
        .capture
        .try_native_move(
            directory.as_ref(),
            OsStr::new("link"),
            Some((target, TransferReplacementPolicy::RecoverableCurrentEntry)),
            &cancel,
        )
        .unwrap();
    let TransferNativeMoveOutcome::Moved { recovery, .. } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(recovery.len(), 1);
    assert_eq!(fs::read(&recovery[0].path).unwrap(), b"displaced");
    assert_eq!(
        fs::read_link(destination.join("link")).unwrap(),
        Path::new("elsewhere")
    );
    assert!(fs::symlink_metadata(&link).is_err());
}

#[test]
fn follow_links_is_refused_for_move() {
    let fixture = fixture();
    let link = fixture.path().join("link");
    symlink("target", &link).unwrap();
    let error = LinuxFileSystem::new(None)
        .prepare_move_source_with_links(
            &link,
            CopyLinkPolicy::FollowLinks,
            &Cancellation::default(),
        )
        .err()
        .unwrap();
    assert_eq!(error.kind, io::ErrorKind::Unsupported);
    assert_eq!(fs::read_link(&link).unwrap(), Path::new("target"));
}
