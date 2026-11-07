#![cfg(target_os = "linux")]
use midnight_worker_platform::store::PrivateStore;
use std::os::unix::fs::{PermissionsExt, symlink};

#[test]
fn cache_never_follows_root_or_record_symlinks() {
    let parent = std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT")
        .unwrap_or_else(|| env!("CARGO_TARGET_TMPDIR").into());
    std::fs::create_dir_all(&parent).unwrap();
    let root = tempfile::tempdir_in(parent).unwrap();
    let outside = root.path().join("outside");
    std::fs::create_dir(&outside).unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o700)).unwrap();
    let linked_root = root.path().join("linked");
    symlink(&outside, &linked_root).unwrap();
    assert!(PrivateStore::open(&linked_root.join("nested")).is_err());
    assert!(!outside.join("nested").exists());
    let cache = root.path().join("cache");
    let store = PrivateStore::open(&cache).unwrap();
    let sentinel = outside.join("sentinel");
    std::fs::write(&sentinel, b"do not touch").unwrap();
    let name = format!("{}.mtc", "c".repeat(64));
    symlink(&sentinel, cache.join(&name)).unwrap();
    assert!(store.read(&name, 128).is_err());
    assert!(store.write(&name, b"replace").is_err());
    assert!(store.remove(&name).is_err());
    assert_eq!(std::fs::read(sentinel).unwrap(), b"do not touch");
}
