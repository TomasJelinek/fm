//! Real disk fixtures below the repository's `_build/qa`. A passing test removes its
//! fixture; a failing one keeps it as evidence (with any `EVIDENCE.txt`), and so does
//! every test while `MIDNIGHT_KEEP_TEST_FIXTURES=1` is set. Standard library only: the
//! UI and comparator tests include this file too, and each including module keeps its
//! own copy.
#![allow(dead_code)]
use std::{
    fs, io,
    ops::Deref,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// `_build/qa/operations-find`, the shared parent of the real Copy, Move, Delete, Find
/// and UI fixtures.
pub fn operations_find() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/operations-find")
}

/// Whether `MIDNIGHT_KEEP_TEST_FIXTURES=1` asks to keep every fixture.
pub fn keep_requested() -> bool {
    std::env::var_os("MIDNIGHT_KEEP_TEST_FIXTURES").is_some_and(|value| value == "1")
}

/// The end of the fixture at `root`: kept when its test fails (the thread is panicking)
/// or keeping was requested, otherwise removed. A failed removal is repeated after
/// clearing read-only attributes and giving handles that close elsewhere (an engine's
/// cleanup worker, a virus scanner) a moment; if it still fails, the path is named
/// instead of failing the test.
pub fn finish(root: &Path) {
    if std::thread::panicking() || keep_requested() {
        eprintln!("kept test fixture {}", root.display());
        return;
    }
    let mut failure = None;
    for attempt in 0..5 {
        match fs::remove_dir_all(root) {
            Ok(()) => return,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return,
            Err(error) => failure = Some(error),
        }
        make_writable(root);
        std::thread::sleep(Duration::from_millis(25 << attempt));
    }
    if let Some(error) = failure {
        eprintln!("could not remove test fixture {}: {error}", root.display());
    }
}

/// Makes everything below `path` removable, without following links or junctions.
fn make_writable(path: &Path) {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return;
    };
    if metadata.file_type().is_symlink() {
        return;
    }
    set_writable(path, &metadata);
    if metadata.is_dir()
        && let Ok(entries) = fs::read_dir(path)
    {
        for entry in entries.flatten() {
            make_writable(&entry.path());
        }
    }
}

/// On Windows `set_readonly(false)` clears only the read-only attribute.
#[cfg(windows)]
#[allow(clippy::permissions_set_readonly_false)]
fn set_writable(path: &Path, metadata: &fs::Metadata) {
    let mut permissions = metadata.permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        let _ = fs::set_permissions(path, permissions);
    }
}

#[cfg(unix)]
fn set_writable(path: &Path, metadata: &fs::Metadata) {
    use std::os::unix::fs::PermissionsExt;
    let mode = metadata.permissions().mode();
    let owner = if metadata.is_dir() { 0o700 } else { 0o600 };
    if mode & owner != owner {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(mode | owner));
    }
}

/// A new, empty folder `<prefix><process>-<time>-<n>` below `parent` (created first if
/// needed), with a canonical path.
pub fn create_in(parent: &Path, prefix: &str) -> PathBuf {
    fs::create_dir_all(parent).unwrap();
    let parent = fs::canonicalize(parent).unwrap();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    // Parallel tests can read the same clock tick: number the collisions.
    (0..1_000)
        .find_map(|attempt| {
            let path = parent.join(format!("{prefix}{}-{stamp}-{attempt}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => Some(path),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => None,
                Err(error) => panic!("cannot create {}: {error}", path.display()),
            }
        })
        .expect("a unique fixture name")
}

/// A fixture folder that ends with its owner (see `finish`).
pub struct Folder(PathBuf);

impl Folder {
    /// A new folder below `parent`, named `<prefix>...` (see `create_in`).
    pub fn new(parent: &Path, prefix: &str) -> Self {
        Self(create_in(parent, prefix))
    }

    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Deref for Folder {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        finish(&self.0);
    }
}
