use midnight_worker_platform::store::PrivateStore;

fn directory() -> tempfile::TempDir {
    let root = std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT")
        .unwrap_or_else(|| env!("CARGO_TARGET_TMPDIR").into());
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

#[test]
fn private_store_roundtrips_atomically_and_rejects_path_traversal_and_oversize() {
    let root = directory();
    let path = root.path().join("čínština-秘密-📁").join("cache");
    let name = format!("{}.mtc", "a".repeat(64));
    let store = PrivateStore::open(&path).unwrap();
    store.write(&name, b"first").unwrap();
    store.write(&name, b"replacement").unwrap();
    assert_eq!(store.read(&name, 32).unwrap().unwrap(), b"replacement");
    assert!(store.read(&name, 2).is_err());
    for invalid in ["../outside", "C:\\outside", "a/b", "", "README.md"] {
        assert!(store.write(invalid, b"no").is_err());
        assert!(store.read(invalid, 32).is_err());
        assert!(store.remove(invalid).is_err());
    }
    assert_eq!(store.records().unwrap().len(), 1);
    store.remove(&name).unwrap();
    assert!(store.read(&name, 32).unwrap().is_none());
}

#[test]
fn long_unicode_cache_paths_are_not_truncated() {
    let root = directory();
    let mut path = root.path().to_owned();
    for index in 0..9 {
        path.push(format!("{index}-長い-{}", "component".repeat(4)));
    }
    let store = PrivateStore::open(&path).unwrap();
    let name = format!("{}.mtc", "b".repeat(64));
    store.write(&name, b"long path").unwrap();
    assert_eq!(store.read(&name, 32).unwrap().unwrap(), b"long path");
}
