//! WIN-11: thumbnails capture under `RevisionPolicy::AcceptChangeTimeOnly`, as viewers do.
//! A moved change time alone keeps the capture valid and its cached frame a hit; a
//! content change invalidates both. In-process captures of real files.
use super::*;
use crate::service::{NATIVE_REVISIONS, captured_source};
use midnight_plugin_host::{CapturedFile, RevisionPolicy};

/// Private stores require owner-only ancestry, so fixtures live under the repository build.
fn store() -> tempfile::TempDir {
    let root = std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT").unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../_build/cache-tests")
            .into_os_string()
    });
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(std::fs::canonicalize(root).unwrap()).unwrap()
}

/// Move only the change time, as Defender's data loss prevention does when it tags a new
/// file and restores its write time: Windows' read-only attribute and NTFS ChangeTime,
/// Linux's mode and ctime. The revision includes neither attributes nor mode.
fn move_change_time(path: &Path) {
    // Leave the clock past the write's timestamp, which may be as coarse as a tick.
    std::thread::sleep(Duration::from_millis(50));
    let writable = std::fs::metadata(path).unwrap().permissions();
    let mut read_only = writable.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(path, read_only).unwrap();
    std::fs::set_permissions(path, writable).unwrap();
}

fn frame() -> Frame {
    Frame {
        id: 0,
        width: 1,
        height: 1,
        frame: 0,
        rgba: vec![7, 8, 9, 255],
    }
}

fn capture(path: &Path, policy: RevisionPolicy) -> Arc<dyn Source> {
    captured_source(CapturedFile::capture_with(path, policy).unwrap(), false)
}

fn key(source: &dyn Source) -> Key {
    Key::new("fixture", b"picture", source, (96, 96), "v1", 0).unwrap()
}

/// A fresh file (no extension: endpoint protection rescans some file types).
fn picture() -> (tempfile::TempDir, std::path::PathBuf) {
    let files = tempfile::tempdir().unwrap();
    let path = files.path().join("picture");
    std::fs::write(&path, b"pixels").unwrap();
    (files, path)
}

#[test]
fn a_moved_change_time_keeps_a_tolerant_capture_and_its_cached_frame() {
    let (_files, path) = picture();
    let store = store();
    let mut cache = Cache::open(&store.path().join("cache"), 1024 * 1024, 1024 * 1024).unwrap();
    let source = capture(&path, NATIVE_REVISIONS);
    let key = key(&*source);
    // The tag can land while the frame is decoded: storing it still succeeds.
    move_change_time(&path);
    cache.put(&key, &*source, frame()).unwrap();
    move_change_time(&path);
    let hit = cache.get(&key, &*source).unwrap().expect("a cache hit");
    assert_eq!(*hit, frame());
    // A cache opened again (no warm memory) reads the record for the same capture.
    drop(cache);
    let mut cache = Cache::open(&store.path().join("cache"), 1024 * 1024, 0).unwrap();
    assert_eq!(*cache.get(&key, &*source).unwrap().unwrap(), frame());
    assert!(!source.changed_since_capture());
}

/// What WIN-11 met: an exact capture fails on a moved change time. The failure is the
/// source's, never the cache's (which the service reports as a notice).
#[test]
fn an_exact_capture_fails_on_a_moved_change_time_as_its_source_not_the_cache() {
    let (_files, path) = picture();
    let store = store();
    let mut cache = Cache::open(&store.path().join("cache"), 1024 * 1024, 1024 * 1024).unwrap();
    let source = capture(&path, RevisionPolicy::Exact);
    let key = key(&*source);
    cache.put(&key, &*source, frame()).unwrap();
    move_change_time(&path);
    let error = cache.get(&key, &*source).unwrap_err();
    assert!(is_source_failure(&error), "{error}");
    let error = cache.put(&key, &*source, frame()).unwrap_err();
    assert!(is_source_failure(&error), "{error}");
}

#[test]
fn a_content_change_invalidates_the_capture_and_its_cached_frame() {
    let (_files, path) = picture();
    let store = store();
    let mut cache = Cache::open(&store.path().join("cache"), 1024 * 1024, 1024 * 1024).unwrap();
    let source = capture(&path, NATIVE_REVISIONS);
    let old = key(&*source);
    cache.put(&old, &*source, frame()).unwrap();
    // The same length with a new write time.
    std::thread::sleep(Duration::from_millis(50));
    std::fs::write(&path, b"Pixels").unwrap();
    let error = cache.get(&old, &*source).unwrap_err();
    assert!(is_source_failure(&error), "{error}");
    let fresh = capture(&path, NATIVE_REVISIONS);
    let new = key(&*fresh);
    assert_ne!(
        new.filename(),
        old.filename(),
        "a new revision is a new key"
    );
    assert!(
        cache.get(&new, &*fresh).unwrap().is_none(),
        "no frame of the old content"
    );
}
