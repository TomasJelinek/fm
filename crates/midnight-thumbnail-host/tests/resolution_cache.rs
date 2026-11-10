use midnight_thumbnail_api::Frame;
use midnight_thumbnail_host::{
    cache::{Cache, Key, resolution_edge},
    decode::Source,
};
use std::{
    io,
    sync::atomic::{AtomicU8, Ordering},
};

struct MetadataOnly(AtomicU8);
impl Source for MetadataOnly {
    fn length(&self) -> u64 {
        128
    }
    fn version(&self) -> Vec<u8> {
        vec![self.0.load(Ordering::Acquire)]
    }
    fn validate(&self) -> io::Result<()> {
        Ok(())
    }
    fn read_at(&self, _: u64, _: u32) -> io::Result<Vec<u8>> {
        panic!("resolution cache hits/misses must not read source content")
    }
}
fn key(source: &dyn Source, identity: &[u8], edge: u32, frame: u8, decoder: &str) -> Key {
    Key::new("fixture", identity, source, (edge, edge), decoder, frame).unwrap()
}
fn pixels(mark: u8) -> Frame {
    Frame {
        id: 0,
        width: 8,
        height: 2,
        frame: 0,
        rgba: vec![mark; 8 * 2 * 4],
    }
}

#[test]
fn resolution_upgrade_misses_individually_and_retains_all_old_cache_records() {
    let parent = std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT")
        .unwrap_or_else(|| env!("CARGO_TARGET_TMPDIR").into());
    std::fs::create_dir_all(&parent).unwrap();
    let root = tempfile::tempdir_in(parent).unwrap();
    let path = root.path().join("cache");
    let source = MetadataOnly(AtomicU8::new(1));
    let low = key(&source, b"picture", 96, 0, "decoder1");
    let other = key(&source, b"other-picture", 96, 0, "decoder1");
    let large = key(&source, b"picture", 256, 0, "decoder1");
    let mut cache = Cache::open(&path, 1024 * 1024, 1024).unwrap();
    cache.put(&low, &source, pixels(1)).unwrap();
    cache.put(&other, &source, pixels(2)).unwrap();
    assert!(cache.get_covering(&large, &source).unwrap().is_none());
    assert_eq!(cache.get(&low, &source).unwrap().unwrap().rgba[0], 1);
    assert_eq!(cache.get(&other, &source).unwrap().unwrap().rgba[0], 2);
    cache.put(&large, &source, pixels(3)).unwrap();
    drop(cache);

    let mut cache = Cache::open(&path, 1024 * 1024, 1024).unwrap();
    for _ in 0..2 {
        let hit = cache
            .get_covering(&key(&source, b"picture", 192, 0, "decoder1"), &source)
            .unwrap()
            .unwrap();
        assert_eq!(
            hit.edge, 256,
            "coverage is not the source image's eight-pixel width"
        );
        assert_eq!(hit.frame.rgba[0], 3);
        assert!(
            cache
                .get_covering(&key(&source, b"picture", 384, 0, "decoder1"), &source)
                .unwrap()
                .is_none()
        );
    }
    assert_eq!(cache.get(&low, &source).unwrap().unwrap().rgba[0], 1);
    assert_eq!(cache.get(&other, &source).unwrap().unwrap().rgba[0], 2);
    assert!(
        cache
            .get_covering(&key(&source, b"picture", 192, 1, "decoder1"), &source)
            .unwrap()
            .is_none()
    );
    assert!(
        cache
            .get_covering(&key(&source, b"picture", 192, 0, "decoder2"), &source)
            .unwrap()
            .is_none()
    );
    source.0.store(2, Ordering::Release);
    assert!(cache.get_covering(&low, &source).is_err());
    assert!(
        cache
            .get_covering(&key(&source, b"picture", 96, 0, "decoder1"), &source)
            .unwrap()
            .is_none()
    );
}

#[test]
fn physical_resolution_buckets_preserve_default_and_bound_high_dpi_requests() {
    for (requested, expected) in [
        (48, 96),
        (72, 96),
        (96, 96),
        (97, 128),
        (128, 128),
        (144, 192),
        (193, 256),
        (300, 384),
        (512, 512),
        (1024, 512),
    ] {
        assert_eq!(resolution_edge(requested), expected);
    }
}
