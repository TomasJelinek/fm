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
    let private = caller_private_group(gid);
    for (owner, group, mode, acl, expected) in [
        (uid, gid, 0o40700, false, true),
        (uid, gid, 0o40755, false, true),
        // Without group write, an ACL mask lets no named user or group write.
        (uid, gid, 0o40755, true, true),
        (0, gid, 0o40755, false, true),
        (uid, other_gid, 0o40775, false, false),
        (uid, gid, 0o40777, false, false),
        (uid, other_gid, 0o41777, false, true),
        (uid, other_gid, 0o41777, true, true),
        (0, other_gid, 0o41777, false, true),
        (other_uid, gid, 0o40755, false, false),
        (other_uid, gid, 0o41777, false, false),
        // Group write is the caller's alone only for its private group and only
        // while the group bits are not the mask of an ACL naming other writers.
        (uid, gid, 0o40775, false, private),
        (0, gid, 0o40775, false, private),
        (uid, gid, 0o40775, true, false),
        (0, gid, 0o40775, true, false),
    ] {
        let asked = std::cell::Cell::new(false);
        let verdict = protected_mode(owner, group, mode, || {
            asked.set(true);
            acl
        });
        assert_eq!(
            verdict, expected,
            "owner {owner} group {group} mode {mode:o} acl {acl}"
        );
        // The ACL is read only when group write is what would admit the parent.
        let group_write_decides = (owner == uid || owner == 0)
            && mode & 0o1000 == 0
            && mode & 0o002 == 0
            && mode & 0o020 != 0
            && group == gid
            && private;
        assert_eq!(asked.get(), group_write_decides, "mode {mode:o}");
    }
    assert!(!writable_only_mode(uid, other_gid, 0o40770, || false));
    assert!(!writable_only_mode(uid, gid, 0o41777, || false));
    assert!(!writable_only_mode(other_uid, gid, 0o40700, || false));
    assert!(writable_only_mode(uid, other_gid, 0o40755, || true));
    assert_eq!(writable_only_mode(uid, gid, 0o40775, || false), private);
    assert!(!writable_only_mode(uid, gid, 0o40775, || true));
}

#[test]
fn access_acls_are_detected_through_readable_and_path_descriptors() {
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("shared");
    fs::create_dir(&directory).unwrap();
    let open = |flags: OFlags| {
        File::from(
            openat(
                rustix::fs::CWD,
                &directory,
                flags | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .unwrap(),
        )
    };
    let readable = open(OFlags::RDONLY);
    let pinned = open(OFlags::PATH);
    assert!(!may_have_access_acl(&readable));
    assert!(!may_have_access_acl(&pinned));
    if !crate::test_support::share_through_access_acl(&directory, "access ACL detection") {
        return;
    }
    assert!(may_have_access_acl(&readable));
    assert!(may_have_access_acl(&pinned));
    assert!(!protected_parent(&pinned).unwrap());
    assert!(!writable_only_by_caller(&readable).unwrap());
    crate::test_support::remove_access_acl(&directory);
    assert!(!may_have_access_acl(&readable));
    assert!(!may_have_access_acl(&pinned));
    let group = fs::metadata(&directory).unwrap().gid();
    assert_eq!(
        protected_parent(&pinned).unwrap(),
        caller_private_group(group)
    );
}

#[test]
fn a_verdict_being_computed_never_blocks_cached_checks() {
    use std::{sync::mpsc, thread};
    // Elapses only on failure: a lock held while computing blocks the cached check.
    const DEADLINE: Duration = Duration::from_secs(10);
    let validity = Duration::from_secs(600);
    let cache = VerdictCache::new();
    let shared = &cache;
    assert!(cache.get((1, 1), validity, || true));
    let (entered, computing) = mpsc::channel();
    let (finish, finished) = mpsc::channel::<()>();
    thread::scope(|scope| {
        let slow = scope.spawn(move || {
            shared.get((2, 2), validity, move || {
                entered.send(()).unwrap();
                finished.recv_timeout(DEADLINE).is_ok()
            })
        });
        computing.recv_timeout(DEADLINE).unwrap();
        assert!(cache.get((1, 1), validity, || panic!(
            "a cached check waited for another verdict's computation"
        )));
        finish.send(()).unwrap();
        assert!(slow.join().unwrap());
    });
    assert!(cache.get((2, 2), validity, || false));
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
    let parent = File::open(temp.path()).unwrap();
    let private = created(temp.path(), "private");
    assert_eq!(
        check_created_directory(&parent, &private).unwrap(),
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
        check_created_directory(&parent, &restricted).unwrap(),
        CreatedDirectory::Private
    );
    assert_eq!(
        fs::metadata(temp.path().join("restricted")).unwrap().mode() & 0o777,
        0o700
    );
}

/// The Linux Rename plugin carries the same table
/// (`plugins/midnight-filesystem-native-linux/src/ownership.rs`): both code bases
/// must admit the same mount-provided stage modes and read the same ACLs.
#[test]
fn mount_provided_stage_modes_are_never_wider_than_an_unprotected_parent() {
    const T: bool = true;
    const F: bool = false;
    // u, g: the effective user and group; o: another group; r: another owner;
    // p: whether g is the caller's private group; s: whether u is root.
    let u = rustix::process::geteuid().as_raw();
    let g = rustix::process::getegid().as_raw();
    let o = g.wrapping_add(7919);
    let r = u.wrapping_add(1);
    let p = caller_private_group(g);
    let s = u == 0;
    // (stage, its ACL, parent, its ACL) -> (admitted, stage ACL read, parent ACL read)
    for (stage, stage_acl, parent, parent_acl, expected) in [
        // Only the caller can write the stage, whatever its parent.
        ((u, g, 0o40755), F, (u, g, 0o40700), F, (T, F, F)),
        ((u, o, 0o40750), F, (u, g, 0o40700), F, (T, F, F)),
        // Without group write, an ACL mask lets no named user or group write.
        ((u, g, 0o40755), T, (u, g, 0o40700), F, (T, F, F)),
        // Private-group write, but only on a stage without an access ACL.
        ((u, g, 0o40775), F, (u, g, 0o40700), F, (p, p, F)),
        ((u, g, 0o40775), T, (u, g, 0o40700), F, (F, p, F)),
        // Wider than the parent: refused without reading an ACL.
        ((u, g, 0o40777), F, (u, g, 0o40755), F, (F, F, F)),
        ((u, g, 0o40757), F, (u, g, 0o40755), F, (F, F, F)),
        ((u, o, 0o40775), F, (u, g, 0o40755), F, (F, F, F)),
        // Exactly as open as a parent other users can already change ...
        ((u, g, 0o40777), F, (u, g, 0o40777), F, (T, T, T)),
        ((u, o, 0o40775), F, (u, o, 0o40775), F, (T, T, T)),
        // ... while neither carries an access ACL.
        ((u, g, 0o40777), T, (u, g, 0o40777), F, (F, T, F)),
        ((u, g, 0o40777), F, (u, g, 0o40777), T, (F, T, T)),
        // Another parent owner, group or mode, or a protected (sticky) parent.
        ((u, g, 0o40777), F, (r, g, 0o40777), F, (F, F, F)),
        ((u, g, 0o40777), F, (0, g, 0o40777), F, (s, s, s)),
        ((u, g, 0o40777), F, (u, o, 0o40777), F, (F, F, F)),
        ((u, g, 0o40777), F, (u, g, 0o41777), F, (F, F, F)),
        ((u, g, 0o41777), F, (u, g, 0o41777), F, (F, F, F)),
        // A private group admits the stage alone; any other group leaves the
        // parent unprotected, so the stage is then exactly as open as it.
        ((u, g, 0o40775), F, (u, g, 0o40775), F, (T, T, !p)),
    ] {
        let asked = (std::cell::Cell::new(false), std::cell::Cell::new(false));
        let verdict = mount_mode_admitted(
            stage,
            || {
                asked.0.set(true);
                stage_acl
            },
            parent,
            || {
                asked.1.set(true);
                parent_acl
            },
        );
        assert_eq!(
            (verdict, asked.0.get(), asked.1.get()),
            expected,
            "stage {:?} mode {:o} acl {stage_acl}, parent {:?} mode {:o} acl {parent_acl}",
            (stage.0, stage.1),
            stage.2,
            (parent.0, parent.1),
            parent.2
        );
    }
}

/// The parent's group and access ACL take part in the equally-open-parent
/// case; prints SKIP when the temporary directory is of another group (a
/// set-group-ID ancestor) or carries an access ACL (a default ACL above it).
fn equally_open_parent_supported(parent: &File, test: &str) -> bool {
    let group = parent.metadata().unwrap().gid();
    if group != rustix::process::getegid().as_raw() {
        eprintln!(
            "SKIP {test}: the temporary directory's group {group} is not the effective group"
        );
        return false;
    }
    if may_have_access_acl(parent) {
        eprintln!("SKIP {test}: the temporary directory carries an access ACL");
        return false;
    }
    true
}

#[test]
fn mount_provided_modes_are_admitted_only_as_far_as_the_parent_allows() {
    let temp = tempfile::tempdir().unwrap();
    let parent = File::open(temp.path()).unwrap();
    let directory = created(temp.path(), "fat");
    let check =
        |mode| simulation::with_mount_mode(mode, || check_created_directory(&parent, &directory));
    // A parent other users cannot change: vfat/exfat with a 022 mask is admitted,
    // a stage other users could change is not.
    assert_eq!(
        check(0o755).unwrap(),
        CreatedDirectory::MountProvided(0o755)
    );
    for mode in [0o777, 0o757] {
        assert_eq!(
            check(mode).unwrap_err().kind(),
            io::ErrorKind::Unsupported,
            "mode {mode:o}"
        );
    }
    if equally_open_parent_supported(&parent, "mount-provided modes as open as their parent") {
        // A share mounted with dir_mode=0777: every folder is equally open.
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o777)).unwrap();
        assert_eq!(
            check(0o777).unwrap(),
            CreatedDirectory::MountProvided(0o777)
        );
        // A sticky parent is protected, so nothing wider than the caller's is admitted.
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o1777)).unwrap();
        for mode in [0o777, 0o1777] {
            assert_eq!(
                check(mode).unwrap_err().kind(),
                io::ErrorKind::Unsupported,
                "mode {mode:o}"
            );
        }
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o700)).unwrap();
    }
    let error = simulation::with_foreign_owner(|| {
        check_created_directory(&parent, &directory).unwrap_err()
    });
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    fs::write(temp.path().join("file"), b"not a directory").unwrap();
    let file = File::open(temp.path().join("file")).unwrap();
    assert_eq!(
        check_created_directory(&parent, &file).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
}

/// With a POSIX access ACL the group bits of a mount-provided 0775 mode are the
/// mask of named writers, so the directory is not the caller's alone.
#[test]
fn a_mount_provided_stage_shared_through_an_access_acl_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let parent = File::open(temp.path()).unwrap();
    let path = temp.path().join("shared");
    let directory = created(temp.path(), "shared");
    if !crate::test_support::share_through_access_acl(&path, "ACL-shared mount-provided modes") {
        return;
    }
    let check =
        || simulation::with_mount_mode(0o775, || check_created_directory(&parent, &directory));
    assert_eq!(check().unwrap_err().kind(), io::ErrorKind::Unsupported);
    // The ACL alone made the difference: without it the private group may write.
    crate::test_support::remove_access_acl(&path);
    let group = fs::metadata(&path).unwrap().gid();
    assert_eq!(check().is_ok(), caller_private_group(group));
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
