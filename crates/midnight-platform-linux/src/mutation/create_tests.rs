use crate::LinuxFileSystem;
use midnight_core::{Cancellation, MutationService};
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::ErrorKind,
    os::unix::{ffi::OsStringExt, fs::symlink},
    path::Path,
};

#[test]
fn nested_creation_reuses_intermediates_and_preserves_unicode_and_raw_native_names() {
    let fixture = tempfile::tempdir().unwrap();
    fs::create_dir(fixture.path().join("existing")).unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare_create(fixture.path(), &cancel)
        .unwrap();
    let relative = Path::new("existing/文档 👩🏽‍💻/nested\\literal");
    let created = prepared.create(relative, &cancel).unwrap();
    assert_eq!(created, fixture.path().join(relative));
    assert!(created.is_dir());
    let raw = OsString::from_vec(b"raw-\xff/child".to_vec());
    assert!(prepared.create(Path::new(&raw), &cancel).unwrap().is_dir());
}

#[test]
fn final_collision_never_replaces_existing_file_directory_or_symlink() {
    let fixture = tempfile::tempdir().unwrap();
    fs::write(fixture.path().join("file"), b"keep").unwrap();
    fs::create_dir(fixture.path().join("directory")).unwrap();
    symlink("directory", fixture.path().join("link")).unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare_create(fixture.path(), &cancel)
        .unwrap();
    for name in ["file", "directory", "link"] {
        assert_eq!(
            prepared.create(Path::new(name), &cancel).unwrap_err().kind,
            ErrorKind::AlreadyExists
        );
    }
    assert_eq!(fs::read(fixture.path().join("file")).unwrap(), b"keep");
    assert!(
        fs::symlink_metadata(fixture.path().join("link"))
            .unwrap()
            .is_symlink()
    );
}

#[test]
fn create_rejects_empty_targets_and_nul_before_any_write() {
    let fixture = tempfile::tempdir().unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare_create(fixture.path(), &cancel)
        .unwrap();
    for path in ["", ".", "..", "/", "good/.", "good/..", "good/nul\0name"] {
        assert_eq!(
            prepared.create(Path::new(path), &cancel).unwrap_err().kind,
            ErrorKind::InvalidInput,
            "accepted {path:?}"
        );
    }
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 0);
}

#[test]
fn create_accepts_absolute_and_parent_relative_paths_outside_the_captured_base() {
    let fixture = tempfile::tempdir().unwrap();
    let panel = fixture.path().join("panel");
    fs::create_dir(&panel).unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare_create(&panel, &cancel)
        .unwrap();
    let absolute = fixture.path().join("absolute/文档 🦀/new");
    assert_eq!(prepared.create(&absolute, &cancel).unwrap(), absolute);
    assert!(absolute.is_dir());
    let created = prepared
        .create(Path::new("../sibling/文档/new"), &cancel)
        .unwrap();
    assert_eq!(
        fs::canonicalize(created).unwrap(),
        fs::canonicalize(fixture.path().join("sibling/文档/new")).unwrap()
    );
    assert_eq!(fs::read_dir(panel).unwrap().count(), 0);
}

#[test]
fn create_walks_parent_components_without_collapsing_missing_directories_or_links() {
    let fixture = tempfile::tempdir().unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare_create(fixture.path(), &cancel)
        .unwrap();
    let created = prepared
        .create(Path::new("first/../second/./leaf/"), &cancel)
        .unwrap();
    assert!(fixture.path().join("first").is_dir());
    assert_eq!(
        fs::canonicalize(created).unwrap(),
        fs::canonicalize(fixture.path().join("second/leaf")).unwrap()
    );
    fs::create_dir(fixture.path().join("outside")).unwrap();
    symlink("outside", fixture.path().join("link")).unwrap();
    assert!(
        prepared
            .create(Path::new("link/../forbidden"), &cancel)
            .is_err()
    );
    assert!(!fixture.path().join("forbidden").exists());
    assert_eq!(
        fs::read_dir(fixture.path().join("outside"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn create_absolute_targets_still_honor_cancellation_and_replaced_base_identity() {
    let fixture = tempfile::tempdir().unwrap();
    let panel = fixture.path().join("panel");
    fs::create_dir(&panel).unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare_create(&panel, &cancel)
        .unwrap();
    let absolute = fixture.path().join("outside/new");
    cancel.cancel();
    assert_eq!(
        prepared.create(&absolute, &cancel).unwrap_err().kind,
        ErrorKind::Interrupted
    );
    assert!(!fixture.path().join("outside").exists());
    fs::rename(&panel, fixture.path().join("moved-panel")).unwrap();
    fs::create_dir(&panel).unwrap();
    assert!(
        prepared
            .create(&absolute, &Cancellation::default())
            .is_err()
    );
    assert!(!fixture.path().join("outside").exists());
}

#[test]
fn create_absolute_workspace_target_may_cross_mounts_from_the_prepared_base() {
    // WSL runs native fixtures on ext4 while the workspace is on its mounted
    // Windows volume. On other hosts this also exercises the absolute-root walk.
    let base = tempfile::tempdir().unwrap();
    let workspace_tmp = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp");
    fs::create_dir_all(&workspace_tmp).unwrap();
    let target = tempfile::tempdir_in(fs::canonicalize(workspace_tmp).unwrap()).unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare_create(base.path(), &cancel)
        .unwrap();
    let absolute = target.path().join("explicit-mount/文档/new");
    assert_eq!(prepared.create(&absolute, &cancel).unwrap(), absolute);
    assert!(absolute.is_dir());
    assert_eq!(fs::read_dir(base.path()).unwrap().count(), 0);
}

#[test]
fn create_refuses_intermediate_links_and_a_replaced_base_directory() {
    let fixture = tempfile::tempdir().unwrap();
    let outside = fixture.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let base = fixture.path().join("base");
    fs::create_dir(&base).unwrap();
    symlink(&outside, base.join("escape")).unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare_create(&base, &cancel)
        .unwrap();
    assert!(prepared.create(Path::new("escape/child"), &cancel).is_err());
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    fs::rename(&base, fixture.path().join("moved-base")).unwrap();
    fs::create_dir(&base).unwrap();
    assert!(prepared.create(Path::new("child"), &cancel).is_err());
    assert_eq!(fs::read_dir(&base).unwrap().count(), 0);
    assert!(!fixture.path().join("moved-base/child").exists());
}

#[test]
fn create_honors_cancellation_and_reports_created_intermediates_on_later_failure() {
    let fixture = tempfile::tempdir().unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare_create(fixture.path(), &cancel)
        .unwrap();
    cancel.cancel();
    assert_eq!(
        prepared
            .create(Path::new("none/child"), &cancel)
            .unwrap_err()
            .kind,
        ErrorKind::Interrupted
    );
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 0);
    assert!(
        LinuxFileSystem::new(None)
            .prepare_create(fixture.path(), &cancel)
            .is_err()
    );
    let cancel = Cancellation::default();
    let path = format!("created/{}", "n".repeat(300));
    let failure = prepared.create(Path::new(&path), &cancel).unwrap_err();
    assert!(fixture.path().join("created").is_dir());
    assert!(failure.message.contains("created"));
    assert_eq!(
        fs::read_dir(fixture.path().join("created"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn create_file_preserves_unicode_raw_bytes_and_literal_unix_backslashes() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = tempfile::tempdir().unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare_create(fixture.path(), &cancel)
        .unwrap();
    let reference = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(fixture.path().join("default-permissions"))
        .unwrap();
    let expected_permissions = reference.metadata().unwrap().permissions().mode() & 0o777;
    for name in [
        OsString::from("文档 e\u{301} 👩🏽‍💻.txt"),
        OsString::from_vec(b"raw-\xff".to_vec()),
        OsString::from("literal\\name"),
    ] {
        let path = prepared.create_file(&name, &cancel).unwrap();
        assert_eq!(path, fixture.path().join(&name));
        let metadata = fs::symlink_metadata(&path).unwrap();
        assert!(metadata.is_file());
        assert_eq!(metadata.len(), 0);
        assert_eq!(metadata.permissions().mode() & 0o111, 0);
        assert_eq!(metadata.permissions().mode() & 0o777, expected_permissions);
        assert!(fs::read(path).unwrap().is_empty());
    }
}

#[test]
fn create_file_never_replaces_or_truncates_existing_entries() {
    let fixture = tempfile::tempdir().unwrap();
    fs::write(fixture.path().join("file"), b"keep nonempty bytes").unwrap();
    fs::create_dir(fixture.path().join("directory")).unwrap();
    fs::write(fixture.path().join("directory/kept"), b"keep child").unwrap();
    symlink("file", fixture.path().join("link")).unwrap();
    symlink("absent", fixture.path().join("dangling")).unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare_create(fixture.path(), &cancel)
        .unwrap();
    for name in ["file", "directory", "link", "dangling"] {
        assert_eq!(
            prepared
                .create_file(OsStr::new(name), &cancel)
                .unwrap_err()
                .kind,
            ErrorKind::AlreadyExists,
            "collision {name}"
        );
    }
    assert_eq!(
        fs::read(fixture.path().join("file")).unwrap(),
        b"keep nonempty bytes"
    );
    assert_eq!(
        fs::read(fixture.path().join("directory/kept")).unwrap(),
        b"keep child"
    );
    assert_eq!(
        fs::read_link(fixture.path().join("link")).unwrap(),
        Path::new("file")
    );
    assert_eq!(
        fs::read_link(fixture.path().join("dangling")).unwrap(),
        Path::new("absent")
    );
    assert!(!fixture.path().join("absent").exists());
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 4);
}

#[test]
fn create_file_rejects_empty_dot_parent_nul_and_path_components_without_writes() {
    let fixture = tempfile::tempdir().unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare_create(fixture.path(), &cancel)
        .unwrap();
    for name in [
        "",
        ".",
        "..",
        "/absolute",
        "../escape",
        "nested/file",
        "file/",
        "nul\0name",
    ] {
        assert_eq!(
            prepared
                .create_file(OsStr::new(name), &cancel)
                .unwrap_err()
                .kind,
            ErrorKind::InvalidInput,
            "accepted {name:?}"
        );
    }
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 0);
}

#[test]
fn create_file_honors_cancellation_before_any_write() {
    let fixture = tempfile::tempdir().unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare_create(fixture.path(), &cancel)
        .unwrap();
    cancel.cancel();
    assert_eq!(
        prepared
            .create_file(OsStr::new("cancelled"), &cancel)
            .unwrap_err()
            .kind,
        ErrorKind::Interrupted
    );
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 0);
}

#[test]
fn create_file_rejects_a_replaced_base_without_touching_either_directory() {
    let fixture = tempfile::tempdir().unwrap();
    let base = fixture.path().join("base");
    fs::create_dir(&base).unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare_create(&base, &cancel)
        .unwrap();
    let moved = fixture.path().join("moved-base");
    fs::rename(&base, &moved).unwrap();
    fs::create_dir(&base).unwrap();
    assert_eq!(
        prepared
            .create_file(OsStr::new("new"), &cancel)
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
    assert_eq!(fs::read_dir(&base).unwrap().count(), 0);
    assert_eq!(fs::read_dir(&moved).unwrap().count(), 0);
}

#[test]
fn create_file_concurrent_same_name_has_exactly_one_winner() {
    use std::sync::{Arc, Barrier};
    let fixture = tempfile::tempdir().unwrap();
    let prepared = LinuxFileSystem::new(None)
        .prepare_create(fixture.path(), &Cancellation::default())
        .unwrap();
    let gate = Arc::new(Barrier::new(8));
    let workers = (0..8)
        .map(|_| {
            let prepared = prepared.clone();
            let gate = gate.clone();
            std::thread::spawn(move || {
                gate.wait();
                prepared.create_file(OsStr::new("one-winner"), &Cancellation::default())
            })
        })
        .collect::<Vec<_>>();
    let results = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    assert!(
        results
            .iter()
            .filter_map(|result| result.as_ref().err())
            .all(|failure| failure.kind == ErrorKind::AlreadyExists)
    );
    assert_eq!(
        fs::metadata(fixture.path().join("one-winner"))
            .unwrap()
            .len(),
        0
    );
    assert_eq!(fs::read_dir(fixture.path()).unwrap().count(), 1);
}
