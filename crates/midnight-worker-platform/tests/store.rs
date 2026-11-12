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

#[test]
fn private_buckets_are_lazy_and_accept_only_a_lowercase_hex_prefix() {
    let root = directory();
    let path = root.path().join("cache");
    let store = PrivateStore::open(&path).unwrap();
    assert!(store.bucket("ab", false).unwrap().is_none());
    assert!(!path.join("ab").exists());
    for invalid in ["", "a", "abc", "AB", "gg", "..", "a/b", "a\\b", "C:"] {
        assert!(store.bucket(invalid, false).is_err());
        assert!(store.bucket(invalid, true).is_err());
    }
    let name = format!("ab{}.mtc", "1".repeat(62));
    let bucket = store.bucket("ab", true).unwrap().unwrap();
    bucket.write(&name, b"sharded").unwrap();
    assert_eq!(
        std::fs::read(path.join("ab").join(&name)).unwrap(),
        b"sharded"
    );
    drop(bucket);
    let bucket = store.bucket("ab", false).unwrap().unwrap();
    assert_eq!(bucket.read(&name, 16).unwrap().unwrap(), b"sharded");
    assert_eq!(bucket.record(&name).unwrap().unwrap().bytes, 7);
    assert_eq!(bucket.records().unwrap().len(), 1);
    assert!(store.records().unwrap().is_empty());
    assert!(!path.join("ab").join("store.lock").exists());
    bucket.remove(&name).unwrap();
    assert!(bucket.record(&name).unwrap().is_none());
}

#[test]
fn independent_store_transactions_wait_for_the_current_owner() {
    let root = directory();
    let path = root.path().join("cache");
    let first = PrivateStore::open(&path).unwrap();
    let second = PrivateStore::open(&path).unwrap();
    let transaction = first.transaction().unwrap();
    let (started, ready) = std::sync::mpsc::channel();
    let (done, completed) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started.send(()).unwrap();
        let _transaction = second.transaction().unwrap();
        done.send(()).unwrap();
    });
    ready.recv().unwrap();
    assert!(
        completed
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err()
    );
    drop(transaction);
    completed
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    worker.join().unwrap();
}

#[test]
fn store_transaction_excludes_another_process_and_releases_on_drop() {
    if std::env::var_os("MIDNIGHT_STORE_LOCK_TEST").is_some() {
        store_transaction_child();
        return;
    }
    let root = directory();
    let path = root.path().join("cache");
    let store = PrivateStore::open(&path).unwrap();
    let transaction = store.transaction().unwrap();
    let child = |busy: bool| {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "store_transaction_excludes_another_process_and_releases_on_drop",
            ])
            .env("MIDNIGHT_STORE_LOCK_TEST", &path)
            .env("MIDNIGHT_STORE_LOCK_BUSY", if busy { "1" } else { "0" })
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
    };
    child(true);
    drop(transaction);
    child(false);
}

fn store_transaction_child() {
    let path = std::env::var_os("MIDNIGHT_STORE_LOCK_TEST").unwrap();
    let store = PrivateStore::open(std::path::Path::new(&path)).unwrap();
    if std::env::var("MIDNIGHT_STORE_LOCK_BUSY").unwrap() == "1" {
        assert_eq!(
            store.transaction().err().unwrap().kind(),
            std::io::ErrorKind::WouldBlock
        );
    } else {
        assert!(store.transaction().is_ok());
    }
}
