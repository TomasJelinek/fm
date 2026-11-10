use super::*;

fn directory() -> tempfile::TempDir {
    let root = std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT").unwrap_or_else(|| {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../_build/cache-store-tests")
            .into_os_string()
    });
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(std::fs::canonicalize(root).unwrap()).unwrap()
}

fn name(prefix: &str) -> String {
    format!("{prefix}{}.mtc", "1".repeat(62))
}

#[test]
fn new_records_use_lazy_buckets_and_legacy_reads_do_not_migrate() {
    let fixture = directory();
    let path = fixture.path().join("v1");
    let root = PrivateStore::open(&path).unwrap();
    let legacy = name("ab");
    root.write(&legacy, b"legacy").unwrap();
    let store = Store::open(&path).unwrap();
    assert_eq!(store.read(&legacy, 32).unwrap().unwrap(), b"legacy");
    assert!(path.join(&legacy).exists());
    assert!(!path.join("ab").exists());
    store.write(&name("cd"), b"new").unwrap();
    assert_eq!(
        std::fs::read(path.join("cd").join(name("cd"))).unwrap(),
        b"new"
    );
    assert!(!path.join(name("cd")).exists());
    assert!(!path.join("ef").exists());
    assert!(!path.join("cd/store.lock").exists());
    store.write(&legacy, b"replacement").unwrap();
    assert!(!path.join(&legacy).exists());
    assert_eq!(store.read(&legacy, 32).unwrap().unwrap(), b"replacement");
    assert_eq!(
        store
            .records()
            .unwrap()
            .iter()
            .map(|record| record.copies)
            .sum::<usize>(),
        2
    );
}

#[test]
fn duplicate_physical_records_count_in_full_and_removal_deletes_both() {
    let fixture = directory();
    let path = fixture.path().join("v1");
    let root = PrivateStore::open(&path).unwrap();
    let name = name("ab");
    root.write(&name, b"old").unwrap();
    root.bucket("ab", true)
        .unwrap()
        .unwrap()
        .write(&name, b"newer")
        .unwrap();
    let store = Store::open(&path).unwrap();
    let records = store.records().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].name, name);
    assert_eq!(records[0].bytes, 8);
    assert_eq!(records[0].copies, 2);
    assert_eq!(store.record(&name).unwrap().unwrap().bytes, 8);
    assert_eq!(store.read(&name, 32).unwrap().unwrap(), b"newer");
    store.remove(&name).unwrap();
    assert!(!path.join(&name).exists());
    assert!(!path.join("ab").join(&name).exists());
    assert!(store.records().unwrap().is_empty());
    store.remove(&name).unwrap();
}

#[test]
fn shard_errors_do_not_fall_back_and_noncanonical_records_fail_closed() {
    let fixture = directory();
    let path = fixture.path().join("v1");
    let root = PrivateStore::open(&path).unwrap();
    let name = name("ab");
    root.write(&name, b"old").unwrap();
    root.bucket("ab", true)
        .unwrap()
        .unwrap()
        .write(&name, b"too large")
        .unwrap();
    let store = Store::open(&path).unwrap();
    assert_eq!(
        store.read(&name, 4).unwrap_err().kind(),
        io::ErrorKind::InvalidData
    );
    root.bucket("cd", true)
        .unwrap()
        .unwrap()
        .write(&name, b"wrong bucket")
        .unwrap();
    assert_eq!(
        store.records().err().unwrap().kind(),
        io::ErrorKind::InvalidData
    );
    for invalid in ["../outside", "ab/record", "", "AB0123.mtc", "C:\\outside"] {
        assert!(store.read(invalid, 32).is_err());
        assert!(store.touch(invalid).is_err());
        assert!(store.write(invalid, b"no").is_err());
        assert!(store.record(invalid).is_err());
        assert!(store.remove(invalid).is_err());
    }
}

#[test]
fn adapter_uses_the_legacy_root_lock_and_generation() {
    let fixture = directory();
    let path = fixture.path().join("v1");
    let root = PrivateStore::open(&path).unwrap();
    let store = Store::open(&path).unwrap();
    let mut transaction = root.transaction().unwrap();
    let before = transaction.generation().unwrap();
    transaction.advance().unwrap();
    drop(transaction);
    assert_eq!(store.transaction().unwrap().generation(), Some(before + 1));
    assert!(!path.join("00").exists());
}
