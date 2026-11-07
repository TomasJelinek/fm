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
    std::fs::write(path.join(old.filename()), b"corrupt").unwrap();
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
    let bytes: u64 = std::fs::read_dir(&path)
        .unwrap()
        .map(|entry| entry.unwrap().metadata().unwrap().len())
        .sum();
    assert!(bytes <= 600);
    assert!(cache.get(&key(&source, 5), &source).unwrap().is_some());
    assert!(cache.get(&key(&source, 0), &source).unwrap().is_none());
}
