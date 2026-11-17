use super::*;
use midnight_worker_platform::store::PrivateStore;

struct Fixed;
impl Source for Fixed {
    fn length(&self) -> u64 {
        1
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> io::Result<()> {
        Ok(())
    }
    fn read_at(&self, _: u64, _: u32) -> io::Result<Vec<u8>> {
        unreachable!("the cache never reads source content")
    }
}

/// Private stores require owner-only ancestry, so fixtures live under the repository build.
fn directory() -> tempfile::TempDir {
    let root = std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT").unwrap_or_else(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../_build/cache-tests")
            .into_os_string()
    });
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(std::fs::canonicalize(root).unwrap()).unwrap()
}

fn frame(index: u8) -> Frame {
    Frame {
        id: 0,
        width: 1,
        height: 1,
        frame: 0,
        rgba: vec![index; 4],
    }
}

fn key(index: u8) -> Key {
    Key::new("fixture", &[index], &Fixed, (32, 32), "v1", 0).unwrap()
}

fn generation(path: &Path) -> Option<u64> {
    PrivateStore::open(path)
        .unwrap()
        .transaction()
        .unwrap()
        .generation()
}

fn physical_records(path: &Path) -> usize {
    let store = PrivateStore::open(path).unwrap();
    let mut count = store.records().unwrap().len();
    for index in 0..=255 {
        if let Some(bucket) = store.bucket(&format!("{index:02x}"), false).unwrap() {
            count += bucket.records().unwrap().len();
        }
    }
    count
}

#[test]
fn duplicate_files_are_counted_toward_the_global_record_limit() {
    // The same accounting applies to the production MAX_RECORDS bound; a small injected
    // limit keeps the fixture to ten files instead of eight thousand.
    let root = directory();
    let path = root.path().join("cache");
    let flat = PrivateStore::open(&path).unwrap();
    let bucket = flat.bucket("00", true).unwrap().unwrap();
    for index in 0..5 {
        let name = format!("{index:064x}.mtc");
        flat.write(&name, b"").unwrap();
        bucket.write(&name, b"").unwrap();
    }
    assert_eq!(physical_records(&path), 10);
    let mut cache = Cache::open_bounded(&path, 1024 * 1024, 0, 8).unwrap();
    assert_eq!(
        physical_records(&path),
        8,
        "both physical copies of the oldest name are evicted together"
    );
    assert_eq!(cache.disk.copies, 8);
    cache.clear().unwrap();
    assert_eq!(physical_records(&path), 0);
    assert_eq!(
        Cache::open(&path, 1024 * 1024, 0).unwrap().record_limit,
        MAX_RECORDS
    );
}

#[test]
fn lookups_leave_the_shared_generation_until_a_damaged_record_is_removed() {
    let root = directory();
    let path = root.path().join("cache");
    let mut cache = Cache::open(&path, 1024 * 1024, 1024).unwrap();
    cache.put(&key(1), &Fixed, frame(1)).unwrap();
    cache.put(&key(2), &Fixed, frame(2)).unwrap();
    // Opening may evict, so it publishes a generation of its own.
    let mut other = Cache::open(&path, 1024 * 1024, 0).unwrap();
    let published = generation(&path);
    assert!(published.is_some());
    for _ in 0..3 {
        // Memory hits (verified after the other instance's change), disk hits and
        // synchronized misses are read-only for every other instance's inventory.
        assert!(cache.get(&key(1), &Fixed).unwrap().is_some());
        assert!(other.get(&key(2), &Fixed).unwrap().is_some());
        assert!(other.get(&key(9), &Fixed).unwrap().is_none());
        assert!(other.get_covering(&key(9), &Fixed).unwrap().is_none());
    }
    assert_eq!(generation(&path), published);
    let name = key(2).filename();
    std::fs::write(path.join(&name[..2]).join(&name), b"damaged").unwrap();
    assert!(other.get(&key(2), &Fixed).unwrap().is_none());
    assert_eq!(
        generation(&path),
        published.map(|value| value + 1),
        "removing a damaged record is an inventory change other instances must observe"
    );
    assert!(cache.get(&key(2), &Fixed).unwrap().is_none());
}

#[test]
fn memory_evicts_the_least_recently_used_frame_first() {
    let size = |key: &Key| 4 + key.encoded.len() + key.version.len() + size_of::<MemoryRecord>();
    let limit = size(&key(1)) * 2;
    let mut memory = Memory::new(limit);
    memory.insert(key(1).filename(), key(1), Arc::new(frame(1)));
    memory.insert(key(2).filename(), key(2), Arc::new(frame(2)));
    assert!(memory.get(&key(1).filename(), &key(1)).is_some());
    memory.insert(key(3).filename(), key(3), Arc::new(frame(3)));
    assert!(memory.contains(&key(1).filename(), &key(1)));
    assert!(
        !memory.contains(&key(2).filename(), &key(2)),
        "the least recently used frame leaves first"
    );
    assert!(memory.contains(&key(3).filename(), &key(3)));
    assert!(memory.bytes <= limit);
    assert_eq!(memory.records.len(), memory.by_use.len());
    // A name collision with a different key is never served.
    assert!(memory.get(&key(3).filename(), &key(1)).is_none());
}

#[test]
fn inventory_tracks_totals_and_the_oldest_record_across_touches() {
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let record = |name: &str, age: u64, copies: usize| Record {
        name: name.into(),
        bytes: 10 * copies as u64,
        modified: base + Duration::from_secs(age),
        copies,
    };
    let mut inventory = Inventory::from_records(vec![
        record("a", 1, 1),
        record("b", 2, 2),
        record("c", 3, 1),
    ])
    .unwrap();
    assert_eq!((inventory.bytes, inventory.copies), (40, 4));
    assert_eq!(inventory.oldest().as_deref(), Some("a"));
    inventory.touch("a", base + Duration::from_secs(9));
    assert_eq!(inventory.oldest().as_deref(), Some("b"));
    inventory.remove("b");
    assert_eq!((inventory.bytes, inventory.copies), (20, 2));
    assert_eq!(inventory.oldest().as_deref(), Some("c"));
    // Replacing a record keeps the totals and the age index exact.
    inventory.insert(record("c", 0, 1)).unwrap();
    assert_eq!((inventory.bytes, inventory.copies), (20, 2));
    assert_eq!(inventory.by_age.len(), 2);
    assert_eq!(inventory.oldest().as_deref(), Some("c"));
}
