use super::super::{SpoolContent, SpoolRoot, create_private_spool, spool_roots};
use super::*;
use midnight_worker_platform::spool::PrivateSpoolDirectory;
use std::{
    fs::Permissions,
    io,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};

fn session(variables: &[(&str, &str)], uid: Option<u32>) -> Session {
    from_environment(
        |name| {
            variables
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| OsString::from(value))
        },
        uid,
        PathBuf::from("/tmp"),
    )
}

fn kept(path: impl Into<PathBuf>) -> SpoolRoot {
    SpoolRoot {
        path: path.into(),
        per_process: false,
    }
}

fn per_process(path: impl Into<PathBuf>) -> SpoolRoot {
    SpoolRoot {
        path: path.into(),
        per_process: true,
    }
}

#[test]
fn spool_roots_use_xdg_directories_first_and_the_temporary_directory_last() {
    let desktop = session(
        &[
            ("XDG_CACHE_HOME", "/home/u/cache"),
            ("HOME", "/home/u"),
            ("XDG_RUNTIME_DIR", "/run/user/1000"),
        ],
        Some(1000),
    );
    assert_eq!(
        spool_roots(&desktop, 42, "file-list", SpoolContent::Large),
        [
            kept("/home/u/cache/midnight-explorer/file-list"),
            kept("/run/user/1000/midnight-explorer/file-list"),
            kept("/tmp/midnight-explorer-1000-file-list"),
            per_process("/tmp/midnight-explorer-file-list-42"),
        ]
    );
    // Small reports skip the disk-backed cache.
    assert_eq!(
        spool_roots(&desktop, 42, "reports", SpoolContent::Reports),
        [
            kept("/run/user/1000/midnight-explorer/reports"),
            kept("/tmp/midnight-explorer-1000-reports"),
            per_process("/tmp/midnight-explorer-reports-42"),
        ]
    );
    // Relative XDG variables are ignored; the cache defaults to ~/.cache.
    assert_eq!(
        spool_roots(
            &session(
                &[
                    ("XDG_CACHE_HOME", "relative"),
                    ("HOME", "/home/u"),
                    ("XDG_RUNTIME_DIR", "run"),
                ],
                Some(1000),
            ),
            7,
            "file-list",
            SpoolContent::Large,
        ),
        [
            kept("/home/u/.cache/midnight-explorer/file-list"),
            kept("/tmp/midnight-explorer-1000-file-list"),
            per_process("/tmp/midnight-explorer-file-list-7"),
        ]
    );
    // su/sudo/ssh/container sessions without a home, XDG directories or
    // procfs still keep the per-process temporary root.
    assert_eq!(
        spool_roots(
            &session(&[("HOME", "relative")], None),
            7,
            "file-list",
            SpoolContent::Large
        ),
        [per_process("/tmp/midnight-explorer-file-list-7")]
    );
}

#[test]
fn effective_uid_is_the_owner_of_files_this_process_creates() {
    let file = tempfile::tempfile().unwrap();
    assert_eq!(effective_uid(), Some(file.metadata().unwrap().uid()));
}

/// Regular files that this process holds open below `directory`. An anonymous
/// spool has no directory entry, but its `/proc/self/fd` link still names the
/// directory that it was created in, with any link on the way resolved.
fn open_files_below(directory: &Path) -> usize {
    std::fs::read_dir("/proc/self/fd")
        .unwrap()
        .filter_map(Result::ok)
        .filter(|descriptor| {
            let descriptor = descriptor.path();
            std::fs::read_link(&descriptor).is_ok_and(|file| file.starts_with(directory))
                && std::fs::metadata(&descriptor).is_ok_and(|metadata| metadata.is_file())
        })
        .count()
}

/// A directory that another account owns. Run as root, a new owner-only
/// directory given to `nobody`, which root can still open, so only the owner
/// check refuses it. Without root no other account's owner-only directory can be
/// opened, so `/` stands in: root owns it, but its group and other permissions
/// are refused as well, and the owner check is not isolated.
fn foreign_directory(parent: &Path) -> PathBuf {
    if effective_uid() != Some(0) {
        return PathBuf::from("/");
    }
    let foreign = parent.join("foreign");
    std::fs::create_dir(&foreign).unwrap();
    std::fs::set_permissions(&foreign, Permissions::from_mode(0o700)).unwrap();
    std::os::unix::fs::chown(&foreign, Some(65_534), Some(65_534)).unwrap();
    foreign
}

#[test]
fn linked_foreign_and_shared_roots_are_refused_for_the_last_candidate() {
    let temporary = tempfile::tempdir().unwrap();
    // This user's own private directory, reached through a link that another
    // account could have planted under the expected name.
    let target = temporary.path().join("target");
    // A directory listing cannot show an anonymous spool; its descriptor can.
    let control = PrivateSpoolDirectory::open(&target)
        .unwrap()
        .create()
        .unwrap();
    assert_eq!(std::fs::read_dir(&target).unwrap().count(), 0);
    assert_eq!(open_files_below(&target), 1);
    drop(control);
    assert_eq!(open_files_below(&target), 0);
    let linked = temporary.path().join("linked");
    std::os::unix::fs::symlink(&target, &linked).unwrap();
    let foreign = foreign_directory(temporary.path());
    let owner = std::fs::metadata(&foreign).unwrap();
    assert_ne!(Some(owner.uid()), effective_uid(), "{foreign:?}");
    if effective_uid() == Some(0) {
        assert_eq!(owner.mode() & 0o077, 0, "only its owner can refuse it");
    }
    // This process can open the foreign directory: the private-root check
    // refuses it, not a missing permission.
    std::fs::read_dir(&foreign).unwrap();
    assert_eq!(
        PrivateSpoolDirectory::open(&foreign)
            .err()
            .map(|error| error.kind()),
        Some(io::ErrorKind::PermissionDenied)
    );
    // An own directory that other accounts may enter.
    let shared = temporary.path().join("shared");
    std::fs::create_dir(&shared).unwrap();
    std::fs::set_permissions(&shared, Permissions::from_mode(0o755)).unwrap();
    let last = temporary.path().join("midnight-explorer-reports-7");
    let roots = [
        kept(&linked),
        kept(&foreign),
        kept(&shared),
        per_process(&last),
    ];
    let mut spool = create_private_spool(&roots, PrivateSpoolDirectory::create).unwrap();
    let metadata = std::fs::symlink_metadata(&last).unwrap();
    assert!(metadata.is_dir());
    assert_eq!(metadata.mode() & 0o777, 0o700);
    // The anonymous spool was created in the last candidate, neither behind the
    // link nor in another refused root (`/` contains every path).
    assert_eq!(open_files_below(&last), 1);
    for refused in [&target, &foreign, &shared] {
        if !last.starts_with(refused) {
            assert_eq!(open_files_below(refused), 0, "{refused:?}");
        }
    }
    assert_eq!(spool.append(0, "東京 list".as_bytes()).unwrap(), 11);
    spool.finish().unwrap();
    let mut bytes = [0; 11];
    assert_eq!(spool.read_at(0, &mut bytes).unwrap(), 11);
    assert_eq!(&bytes, "東京 list".as_bytes());
    // The refused directories were not changed.
    assert!(std::fs::symlink_metadata(&linked).unwrap().is_symlink());
    assert_eq!(std::fs::metadata(&shared).unwrap().mode() & 0o777, 0o755);
    drop(spool);
    assert!(
        !last.exists(),
        "the last spool removes its per-process root"
    );
    let error = create_private_spool(&roots[..3], PrivateSpoolDirectory::create)
        .err()
        .expect("no private root among the refused candidates");
    assert!(error.to_string().contains("shared"), "{error}");
}
