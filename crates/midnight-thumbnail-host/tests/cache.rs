use midnight_thumbnail_api::Frame;
use midnight_thumbnail_host::{
    cache::{Cache, Key},
    decode::Source,
};
use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

struct SourceVersion {
    version: u8,
    changed: AtomicBool,
}
impl Source for SourceVersion {
    fn length(&self) -> u64 {
        128
    }
    fn version(&self) -> Vec<u8> {
        vec![self.version]
    }
    fn validate(&self) -> io::Result<()> {
        if self.changed.load(Ordering::Relaxed) {
            Err(io::Error::other("changed"))
        } else {
            Ok(())
        }
    }
    fn read_at(&self, _: u64, _: u32) -> io::Result<Vec<u8>> {
        panic!("Cache must not decode or read the source")
    }
}
fn frame(index: u8) -> Frame {
    Frame {
        id: 9,
        width: 1,
        height: 1,
        frame: index,
        rgba: vec![255, 40, 30, 255],
    }
}
fn key(source: &dyn Source, index: u8) -> Key {
    Key::new(
        "fixture",
        b"raw-\xff-opaque",
        source,
        (32, 32),
        "decoder-1-srgb",
        index,
    )
    .unwrap()
}
fn directory() -> tempfile::TempDir {
    let root = std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT")
        .unwrap_or_else(|| env!("CARGO_TARGET_TMPDIR").into());
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

fn record_path(root: &std::path::Path, key: &Key) -> std::path::PathBuf {
    let name = key.filename();
    root.join(&name[..2]).join(name)
}

fn disk_records(root: &std::path::Path) -> Vec<midnight_worker_platform::store::Record> {
    let store = midnight_worker_platform::store::PrivateStore::open(root).unwrap();
    let mut records = store.records().unwrap();
    for index in 0..=255 {
        if let Some(bucket) = store.bucket(&format!("{index:02x}"), false).unwrap() {
            records.extend(bucket.records().unwrap());
        }
    }
    records
}

#[test]
fn another_cache_instance_clearing_disk_invalidates_warm_memory_and_allows_regeneration() {
    let root = directory();
    let path = root.path().join("cache");
    let source = SourceVersion {
        version: 1,
        changed: AtomicBool::new(false),
    };
    let key = key(&source, 0);
    let mut first = Cache::open(&path, 1024 * 1024, 1024).unwrap();
    first.put(&key, &source, frame(0)).unwrap();
    assert!(first.memory_bytes() > 0);
    Cache::open(&path, 1024 * 1024, 0).unwrap().clear().unwrap();
    assert!(first.get(&key, &source).unwrap().is_none());
    assert_eq!(first.memory_bytes(), 0);
    first.put(&key, &source, frame(0)).unwrap();
    assert!(first.get(&key, &source).unwrap().is_some());
    Cache::open(&path, 1024 * 1024, 0).unwrap().clear().unwrap();
    first.clear().unwrap();
    assert_eq!(first.memory_bytes(), 0);
    first.put(&key, &source, frame(0)).unwrap();
}

#[test]
fn already_open_cache_clears_records_published_by_another_instance() {
    let root = directory();
    let path = root.path().join("cache");
    let source = SourceVersion {
        version: 1,
        changed: AtomicBool::new(false),
    };
    let key = key(&source, 0);
    let mut first = Cache::open(&path, 1024 * 1024, 1024).unwrap();
    let mut second = Cache::open(&path, 1024 * 1024, 1024).unwrap();
    first.put(&key, &source, frame(0)).unwrap();
    assert!(second.get(&key, &source).unwrap().is_some());
    second.clear().unwrap();
    assert!(!record_path(&path, &key).exists());
    assert!(disk_records(&path).is_empty());
    assert!(first.get(&key, &source).unwrap().is_none());
}

#[test]
fn live_cache_instances_share_the_disk_budget() {
    let root = directory();
    let path = root.path().join("cache");
    let source = SourceVersion {
        version: 1,
        changed: AtomicBool::new(false),
    };
    let mut first = Cache::open(&path, 600, 0).unwrap();
    let mut second = Cache::open(&path, 600, 0).unwrap();
    for index in 0..6 {
        let cache = if index % 2 == 0 {
            &mut first
        } else {
            &mut second
        };
        cache
            .put(&key(&source, index), &source, frame(index))
            .unwrap();
    }
    let records = disk_records(&path);
    assert!(!records.is_empty(), "measure retained sharded records");
    let bytes: u64 = records.iter().map(|record| record.bytes).sum();
    assert!(bytes <= 600, "shared cache grew to {bytes} bytes");
}

#[test]
fn damaged_generation_forces_inventory_refresh_without_reusing_old_versions() {
    let root = directory();
    let path = root.path().join("cache");
    let source = SourceVersion {
        version: 1,
        changed: AtomicBool::new(false),
    };
    let mut first = Cache::open(&path, 600, 0).unwrap();
    let mut second = Cache::open(&path, 600, 0).unwrap();
    first.put(&key(&source, 0), &source, frame(0)).unwrap();
    second.put(&key(&source, 1), &source, frame(1)).unwrap();
    std::fs::write(path.join("store.lock"), b"interrupted").unwrap();
    first.put(&key(&source, 2), &source, frame(2)).unwrap();
    second.put(&key(&source, 3), &source, frame(3)).unwrap();
    let store = midnight_worker_platform::store::PrivateStore::open(&path).unwrap();
    assert!(store.transaction().unwrap().generation().is_none());
    assert!(
        disk_records(&path)
            .iter()
            .map(|record| record.bytes)
            .sum::<u64>()
            <= 600
    );
}

#[test]
fn source_validation_does_not_hold_the_shared_cache_transaction() {
    struct ReentrantSource(midnight_worker_platform::store::PrivateStore);
    impl Source for ReentrantSource {
        fn length(&self) -> u64 {
            1
        }
        fn version(&self) -> Vec<u8> {
            vec![1]
        }
        fn validate(&self) -> io::Result<()> {
            // Source validation may perform slow provider I/O. Another cache must remain
            // usable while it runs; this independent transaction exposes accidental nesting.
            self.0.transaction().map(drop)
        }
        fn read_at(&self, _: u64, _: u32) -> io::Result<Vec<u8>> {
            unreachable!()
        }
    }
    let root = directory();
    let path = root.path().join("cache");
    let mut cache = Cache::open(&path, 1024 * 1024, 1024).unwrap();
    let source =
        ReentrantSource(midnight_worker_platform::store::PrivateStore::open(&path).unwrap());
    let key = key(&source, 0);
    cache.put(&key, &source, frame(0)).unwrap();
    assert!(cache.get(&key, &source).unwrap().is_some());
}

#[test]
fn all_generated_static_and_animation_frames_survive_reopening_without_decode() {
    let root = directory();
    let path = root.path().join("cache");
    let source = SourceVersion {
        version: 1,
        changed: AtomicBool::new(false),
    };
    {
        let mut cache = Cache::open(&path, 1024 * 1024, 8).unwrap();
        for index in 0..6 {
            cache
                .put(&key(&source, index), &source, frame(index))
                .unwrap();
        }
        assert!(cache.memory_bytes() <= 8);
    }
    let mut cache = Cache::open(&path, 1024 * 1024, 8).unwrap();
    for index in 0..6 {
        let cached = cache.get(&key(&source, index), &source).unwrap().unwrap();
        assert_eq!(cached.rgba, frame(index).rgba);
        assert_eq!(cached.frame, index);
    }
}

#[test]
fn source_change_and_corrupt_record_never_return_a_preview() {
    let root = directory();
    let path = root.path().join("cache");
    let source = SourceVersion {
        version: 1,
        changed: AtomicBool::new(false),
    };
    let old = key(&source, 0);
    let mut cache = Cache::open(&path, 1024 * 1024, 0).unwrap();
    cache.put(&old, &source, frame(0)).unwrap();
    let updated = SourceVersion {
        version: 2,
        changed: AtomicBool::new(false),
    };
    assert!(cache.get(&key(&updated, 0), &updated).unwrap().is_none());
    source.changed.store(true, Ordering::Relaxed);
    assert!(cache.get(&old, &source).is_err());
    source.changed.store(false, Ordering::Relaxed);
    std::fs::write(record_path(&path, &old), b"corrupt").unwrap();
    assert!(cache.get(&old, &source).unwrap().is_none());
}

#[test]
fn disk_eviction_keeps_the_most_recently_used_frame_within_budget() {
    let root = directory();
    let path = root.path().join("cache");
    let source = SourceVersion {
        version: 1,
        changed: AtomicBool::new(false),
    };
    let mut cache = Cache::open(&path, 600, 0).unwrap();
    for index in 0..6 {
        cache
            .put(&key(&source, index), &source, frame(index))
            .unwrap();
    }
    let records = disk_records(&path);
    assert!(!records.is_empty(), "measure retained sharded records");
    let bytes: u64 = records.iter().map(|record| record.bytes).sum();
    assert!(bytes <= 600);
    assert!(cache.get(&key(&source, 5), &source).unwrap().is_some());
    assert!(cache.get(&key(&source, 0), &source).unwrap().is_none());
}

#[test]
fn legacy_v1_records_are_read_in_place_and_only_new_writes_use_buckets() {
    let root = directory();
    let seed = root.path().join("seed");
    let path = root.path().join("cache");
    let source = SourceVersion {
        version: 1,
        changed: AtomicBool::new(false),
    };
    let key = key(&source, 0);
    Cache::open(&seed, 1024 * 1024, 0)
        .unwrap()
        .put(&key, &source, frame(0))
        .unwrap();
    let bytes = std::fs::read(record_path(&seed, &key)).unwrap();
    let flat = midnight_worker_platform::store::PrivateStore::open(&path).unwrap();
    flat.write(&key.filename(), &bytes).unwrap();
    let mut cache = Cache::open(&path, 1024 * 1024, 0).unwrap();
    assert_eq!(
        cache.get(&key, &source).unwrap().unwrap().rgba,
        frame(0).rgba
    );
    assert!(path.join(key.filename()).exists());
    assert!(
        !path.join(&key.filename()[..2]).exists(),
        "reads must not migrate records"
    );
    cache.put(&key, &source, frame(0)).unwrap();
    assert!(record_path(&path, &key).exists());
    assert!(!path.join(key.filename()).exists());
}

#[test]
fn duplicates_keep_their_full_byte_cost_after_reads_and_are_evicted_together() {
    let root = directory();
    let path = root.path().join("cache");
    let source = SourceVersion {
        version: 1,
        changed: AtomicBool::new(false),
    };
    let old = key(&source, 0);
    Cache::open(&path, 1024 * 1024, 0)
        .unwrap()
        .put(&old, &source, frame(0))
        .unwrap();
    let bytes = std::fs::read(record_path(&path, &old)).unwrap();
    let flat = midnight_worker_platform::store::PrivateStore::open(&path).unwrap();
    flat.write(&old.filename(), &bytes).unwrap();
    let limit = bytes.len() as u64 * 3 - 1;
    let mut cache = Cache::open(&path, limit, 0).unwrap();
    assert_eq!(disk_records(&path).len(), 2);
    assert!(cache.get(&old, &source).unwrap().is_some());
    cache.put(&key(&source, 1), &source, frame(1)).unwrap();
    let records = disk_records(&path);
    assert_eq!(records.len(), 1, "both old physical copies must be evicted");
    assert!(records.iter().map(|record| record.bytes).sum::<u64>() <= limit);
    assert!(!path.join(old.filename()).exists());
    assert!(!record_path(&path, &old).exists());
}

#[test]
fn clear_removes_legacy_and_sharded_duplicates_and_corruption_never_falls_back() {
    let root = directory();
    let path = root.path().join("cache");
    let source = SourceVersion {
        version: 1,
        changed: AtomicBool::new(false),
    };
    let key = key(&source, 0);
    let mut cache = Cache::open(&path, 1024 * 1024, 0).unwrap();
    cache.put(&key, &source, frame(0)).unwrap();
    let bytes = std::fs::read(record_path(&path, &key)).unwrap();
    let flat = midnight_worker_platform::store::PrivateStore::open(&path).unwrap();
    flat.write(&key.filename(), &bytes).unwrap();
    cache.clear().unwrap();
    assert!(disk_records(&path).is_empty());
    cache.put(&key, &source, frame(0)).unwrap();
    flat.write(&key.filename(), &bytes).unwrap();
    std::fs::write(record_path(&path, &key), b"corrupt shard").unwrap();
    assert!(cache.get(&key, &source).unwrap().is_none());
    assert!(
        disk_records(&path).is_empty(),
        "fail closed and retire both copies"
    );
}

#[test]
fn duplicate_files_are_counted_toward_the_global_record_limit() {
    let root = directory();
    let path = root.path().join("cache");
    let flat = midnight_worker_platform::store::PrivateStore::open(&path).unwrap();
    let bucket = flat.bucket("00", true).unwrap().unwrap();
    for index in 0..4097 {
        let name = format!("{index:064x}.mtc");
        flat.write(&name, b"").unwrap();
        bucket.write(&name, b"").unwrap();
    }
    assert_eq!(disk_records(&path).len(), 8194);
    let mut cache = Cache::open(&path, 1024 * 1024, 0).unwrap();
    assert_eq!(disk_records(&path).len(), 8192);
    cache.clear().unwrap();
    assert!(disk_records(&path).is_empty());
}

#[test]
#[ignore = "full-cache metadata benchmark; creates 8192 bounded fixture records"]
fn full_cache_generation_fast_path_cost() {
    let root = directory();
    let path = root.path().join("cache");
    let store = midnight_worker_platform::store::PrivateStore::open(&path).unwrap();
    for index in 0..8192 {
        store.write(&format!("{index:064x}.mtc"), b"").unwrap();
    }
    let started = std::time::Instant::now();
    assert_eq!(disk_records(&path).len(), 8192);
    let inventory = started.elapsed();
    let mut cache = Cache::open(&path, 1024 * 1024, 0).unwrap();
    let source = SourceVersion {
        version: 1,
        changed: AtomicBool::new(false),
    };
    let started = std::time::Instant::now();
    for index in 0u64..100 {
        let key = Key::new("fixture", &index.to_le_bytes(), &source, (32, 32), "1", 0).unwrap();
        cache.put(&key, &source, frame(0)).unwrap();
    }
    let puts = started.elapsed();
    assert_eq!(disk_records(&path).len(), 8192);
    eprintln!(
        "8192-record inventory: {inventory:?}; 100 synchronized puts: {puts:?} ({:?}/put)",
        puts / 100
    );
}
