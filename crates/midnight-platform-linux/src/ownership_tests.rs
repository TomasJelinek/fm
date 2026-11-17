use super::*;
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};

const PASSWD: &[u8] = b"root:x:0:0:root:/root:/bin/bash\n\
alice:x:1000:1000:Alice,,,:/home/alice:/bin/bash\n\
bob:x:1001:1001::/home/bob:/bin/sh\n";

#[test]
fn user_private_group_requires_matching_name_and_no_other_members() {
    let group = b"root:x:0:\nalice:x:1000:\nbob:x:1001:\nusers:x:100:alice,bob\n";
    assert!(private_group_in(PASSWD, group, 1000, 1000));
    // Listing the owner explicitly still leaves the group private.
    assert!(private_group_in(
        PASSWD,
        b"alice:x:1000:alice\n",
        1000,
        1000
    ));
    // A shared supplementary group, another member or another name is not private.
    assert!(!private_group_in(PASSWD, group, 1000, 100));
    assert!(!private_group_in(
        PASSWD,
        b"alice:x:1000:alice,bob\n",
        1000,
        1000
    ));
    assert!(!private_group_in(PASSWD, b"staff:x:1000:\n", 1000, 1000));
    // An unknown user or group never qualifies.
    assert!(!private_group_in(PASSWD, group, 4242, 1000));
    assert!(!private_group_in(PASSWD, b"bob:x:1001:\n", 1000, 1000));
}

#[test]
fn a_shared_primary_group_is_never_private_even_without_listed_members() {
    // openSUSE-style accounts share the primary group `users` without listing it.
    let passwd = b"alice:x:1000:100::/home/alice:/bin/bash\nbob:x:1001:100::/home/bob:/bin/bash\n";
    assert!(!private_group_in(passwd, b"users:x:100:\n", 1000, 100));
    // Another account using a group named after the caller as its primary group.
    let passwd = b"alice:x:1000:1000::/home/alice:/bin/bash\nbob:x:1001:1000::/home/bob:/bin/sh\n";
    assert!(!private_group_in(passwd, b"alice:x:1000:\n", 1000, 1000));
}

#[test]
fn ambiguous_malformed_or_nis_account_data_is_never_private() {
    let group = b"alice:x:1000:\n";
    for passwd in [
        &b"alice:x:1000:1000::/home/alice:/bin/bash\n+::::::\n"[..],
        b"alice:x:1000:1000::/home/alice:/bin/bash\nmalformed\n",
        b"alice:x:1000:1000::/home/alice:/bin/bash\nbob:x:1001:not-a-number::/:/bin/sh\n",
        b"alice:x:1000:1000::/home/alice:/bin/bash\nalias:x:1000:1000::/home/alice:/bin/sh\n",
    ] {
        assert!(!private_group_in(passwd, group, 1000, 1000));
    }
    for group in [
        &b"alice:x:1000:\nalice2:x:1000:\n"[..],
        b"alice:x:1000:\n-bob\n",
        b"alice:x:1000\n",
    ] {
        assert!(!private_group_in(PASSWD, group, 1000, 1000));
    }
}

#[test]
fn protected_parent_accepts_owned_or_sticky_and_refuses_shared_writers() {
    let uid = rustix::process::geteuid().as_raw();
    let gid = rustix::process::getegid().as_raw();
    let other_uid = if uid == 0 { 1 } else { uid + 1 };
    let other_gid = gid.wrapping_add(7919);
    for (owner, group, mode, expected) in [
        (uid, gid, 0o40700, true),
        (uid, gid, 0o40755, true),
        (0, gid, 0o40755, true),
        (uid, other_gid, 0o40775, false),
        (uid, gid, 0o40777, false),
        (uid, other_gid, 0o41777, true),
        (0, other_gid, 0o41777, true),
        (other_uid, gid, 0o40755, false),
        (other_uid, gid, 0o41777, false),
    ] {
        assert_eq!(
            protected_parent(owner, group, mode),
            expected,
            "owner {owner} group {group} mode {mode:o}"
        );
    }
    // Group write is accepted exactly when the caller's own group is private.
    assert_eq!(
        protected_parent(uid, gid, 0o40775),
        caller_private_group(gid)
    );
    assert!(!writable_only_by_caller(uid, other_gid, 0o40770));
    assert!(!writable_only_by_caller(uid, gid, 0o41777));
    assert!(!writable_only_by_caller(other_uid, gid, 0o40700));
    assert!(writable_only_by_caller(uid, other_gid, 0o40755));
}

fn created(parent: &Path, name: &str) -> File {
    rustix::fs::mkdir(parent.join(name), Mode::RWXU).unwrap();
    File::from(
        openat(
            rustix::fs::CWD,
            parent.join(name),
            OFlags::PATH | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap(),
    )
}

#[test]
fn created_directories_are_private_and_restrictive_modes_are_corrected() {
    let temp = tempfile::tempdir().unwrap();
    let private = created(temp.path(), "private");
    assert_eq!(
        check_created_directory(&private).unwrap(),
        CreatedDirectory::Private
    );
    // A restrictive umask or default ACL can create less than 0700.
    let restricted = created(temp.path(), "restricted");
    fs::set_permissions(
        temp.path().join("restricted"),
        fs::Permissions::from_mode(0o500),
    )
    .unwrap();
    assert_eq!(
        check_created_directory(&restricted).unwrap(),
        CreatedDirectory::Private
    );
    assert_eq!(
        fs::metadata(temp.path().join("restricted")).unwrap().mode() & 0o777,
        0o700
    );
}

#[test]
fn mount_provided_modes_are_accepted_but_foreign_owners_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let directory = created(temp.path(), "fat");
    assert_eq!(
        simulation::with_mount_mode(0o755, || check_created_directory(&directory).unwrap()),
        CreatedDirectory::MountProvided(0o755)
    );
    let error = simulation::with_foreign_owner(|| check_created_directory(&directory).unwrap_err());
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    fs::write(temp.path().join("file"), b"not a directory").unwrap();
    let file = File::open(temp.path().join("file")).unwrap();
    assert_eq!(
        check_created_directory(&file).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

#[test]
fn refused_directories_are_removed_only_while_empty_and_unreplaced() {
    let temp = tempfile::tempdir().unwrap();
    let parent = File::open(temp.path()).unwrap();
    let pinned = |name: &str| {
        let metadata = fs::symlink_metadata(temp.path().join(name)).unwrap();
        Some((metadata.dev(), metadata.ino()))
    };
    fs::create_dir(temp.path().join("refused")).unwrap();
    let identity = pinned("refused");
    remove_refused_directory(&parent, OsStr::new("refused"), identity).unwrap();
    assert!(!temp.path().join("refused").exists());

    fs::create_dir(temp.path().join("replaced")).unwrap();
    let identity = pinned("replaced");
    fs::rename(temp.path().join("replaced"), temp.path().join("moved")).unwrap();
    fs::create_dir(temp.path().join("replaced")).unwrap();
    assert!(remove_refused_directory(&parent, OsStr::new("replaced"), identity).is_err());
    assert!(temp.path().join("replaced").is_dir());

    fs::create_dir(temp.path().join("occupied")).unwrap();
    fs::write(temp.path().join("occupied/kept"), b"kept").unwrap();
    assert!(remove_refused_directory(&parent, OsStr::new("occupied"), None).is_err());
    assert_eq!(
        fs::read(temp.path().join("occupied/kept")).unwrap(),
        b"kept"
    );

    fs::write(temp.path().join("file"), b"kept").unwrap();
    assert!(remove_refused_directory(&parent, OsStr::new("file"), None).is_err());
    assert_eq!(fs::read(temp.path().join("file")).unwrap(), b"kept");
}
