//! A published copy gets the source's permission bits; staging stays private until then.
use super::*;
use std::{fs, io::Write, os::unix::fs::PermissionsExt};

fn mode(path: &Path) -> u32 {
    fs::metadata(path).unwrap().permissions().mode() & 0o7777
}

/// The test process's umask, as the kernel reports it.
fn umask() -> u32 {
    let status = fs::read_to_string("/proc/self/status").unwrap();
    let value = status
        .lines()
        .find_map(|line| line.strip_prefix("Umask:"))
        .unwrap();
    u32::from_str_radix(value.trim(), 8).unwrap()
}

#[test]
fn published_copies_take_source_permissions_and_merged_folders_keep_theirs() {
    // A private parent of ours: Copy creates folders only where others cannot write.
    let temp = crate::test_support::temporary("permissions-");
    let cancel = Cancellation::default();
    let destination = Directory::from_file(
        File::from(openat(CWD, temp.path(), DIRECTORY, Mode::empty()).unwrap()),
        temp.path().to_owned(),
    )
    .unwrap();

    // The usual process umask (022, 002, 027 or 077) removes write bits for others: the
    // source modes below have every bit, so a missing umask step shows as extra bits.
    let umask = umask();
    if umask & 0o077 == 0 {
        eprintln!("umask {umask:o} removes no bit for others: the umask step is unobservable");
    }

    // A file stays owner-only while staged and gets the source's bits when published;
    // the setuid bit of the source is not copied.
    let mut stage = destination.create_stage(false, &cancel).unwrap();
    stage.write_all(b"#!/bin/sh\n").unwrap();
    assert_eq!(
        stage.file().unwrap().metadata().unwrap().mode() & 0o7777,
        0o600
    );
    stage.set_published_permissions(0o104_777, &cancel).unwrap();
    stage
        .publish_no_replace(OsStr::new("script"), &cancel)
        .unwrap();
    assert_eq!(mode(&temp.path().join("script")), 0o777 & !umask);

    // A copied folder stays private until its contents are copied, then gets the
    // source folder's bits; its sticky bit is not copied.
    let folder = destination
        .create_directory(OsStr::new("folder"), &cancel)
        .unwrap();
    assert_eq!(mode(&temp.path().join("folder")), 0o700);
    folder
        .set_published_permissions(0o041_777, &cancel)
        .unwrap();
    assert_eq!(mode(&temp.path().join("folder")), 0o777 & !umask);

    // A folder that existed before keeps its own permissions when a copy merges into it.
    let existing = temp.path().join("existing");
    fs::create_dir(&existing).unwrap();
    fs::set_permissions(&existing, fs::Permissions::from_mode(0o711)).unwrap();
    let expected = destination
        .inspect(OsStr::new("existing"), &cancel)
        .unwrap()
        .unwrap();
    let merged = destination
        .open_merge_directory(OsStr::new("existing"), &expected, &cancel)
        .unwrap();
    merged
        .set_published_permissions(0o040_777, &cancel)
        .unwrap();
    assert_eq!(mode(&existing), 0o711);
}
