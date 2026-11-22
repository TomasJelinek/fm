//! Private persistent RGBA cache; a successful write is required before a generated frame is used.
use crate::{
    cache_store::{Record, Store},
    decode::Source,
};
use midnight_thumbnail_api::{self as api, Frame, Message};
use midnight_worker_platform::store::Transaction;
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io,
    path::Path,
    sync::Arc,
    time::{Duration, SystemTime},
};

pub const DEFAULT_DISK_BYTES: u64 = 512 * 1024 * 1024;
/// Warm decoded frames kept by the backend service. Presentation keeps its own pixel budget.
pub const DEFAULT_MEMORY_BYTES: usize = 4 * 1024 * 1024;
/// Bounded physical decode sizes, retaining the original 96-pixel cache variant.
pub const RESOLUTION_EDGES: [u32; 6] = [96, 128, 192, 256, 384, api::MAX_EDGE];

pub fn resolution_edge(requested: u32) -> u32 {
    RESOLUTION_EDGES
        .into_iter()
        .find(|edge| *edge >= requested)
        .unwrap_or(api::MAX_EDGE)
}

/// Coverage records the decode request, even when a tiny source produces fewer pixels.
pub struct CachedFrame {
    pub frame: Arc<Frame>,
    pub edge: u32,
}

const MAX_KEY_BYTES: usize = 16 * 1024;
const MAX_RECORDS: usize = 8192;
const MAX_RECORD_BYTES: usize = api::MAX_FRAME_BYTES + MAX_KEY_BYTES + 128;
/// While no other instance changed the shared inventory, a warm hit refreshes its
/// record's on-disk recency at most this often instead of writing on every lookup.
const TOUCH_INTERVAL: Duration = Duration::from_secs(60);

/// The captured source failed its revision check while the cache looked it up or stored
/// its frame: the file changed after its capture, or it cannot be reached. Not a failure
/// of the cache; the error keeps the source's own kind and message.
#[derive(Debug)]
pub struct SourceFailure(io::Error);

impl std::fmt::Display for SourceFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(formatter)
    }
}

impl std::error::Error for SourceFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

fn source_failure(error: io::Error) -> io::Error {
    if is_source_failure(&error) {
        return error;
    }
    io::Error::new(error.kind(), SourceFailure(error))
}

/// Whether a cache operation failed because of its source, not the cache.
pub fn is_source_failure(error: &io::Error) -> bool {
    error
        .get_ref()
        .is_some_and(|inner| inner.is::<SourceFailure>())
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Key {
    encoded: Vec<u8>,
    version: Vec<u8>,
    length: u64,
    dimensions: (u32, u32),
    frame: u8,
}

impl Key {
    pub fn new(
        provider: &str,
        identity: &[u8],
        source: &dyn Source,
        dimensions: (u32, u32),
        decoder: &str,
        frame: u8,
    ) -> io::Result<Self> {
        api::rgba_length(dimensions.0, dimensions.1)?;
        if frame >= api::MAX_ANIMATION_FRAMES {
            return Err(invalid("Invalid frame index"));
        }
        source.validate().map_err(source_failure)?;
        let version = source.version();
        let length = source.length();
        let mut encoded = b"MidnightThumbnailKey\0v1\0RGBA8-straight-sRGB\0".to_vec();
        for component in [
            provider.as_bytes(),
            identity,
            version.as_slice(),
            decoder.as_bytes(),
        ] {
            if component.len() > MAX_KEY_BYTES
                || encoded.len() + component.len() + 4 > MAX_KEY_BYTES - 17
            {
                return Err(invalid("Thumbnail identity exceeds cache key bound"));
            }
            encoded.extend_from_slice(&(component.len() as u32).to_le_bytes());
            encoded.extend_from_slice(component);
        }
        encoded.extend_from_slice(&length.to_le_bytes());
        encoded.extend_from_slice(&dimensions.0.to_le_bytes());
        encoded.extend_from_slice(&dimensions.1.to_le_bytes());
        encoded.push(frame);
        Ok(Self {
            encoded,
            version,
            length,
            dimensions,
            frame,
        })
    }
    pub fn filename(&self) -> String {
        format!("{}.mtc", blake3::hash(&self.encoded).to_hex())
    }
    fn with_edge(&self, edge: u32) -> Self {
        let mut key = self.clone();
        let dimensions = key.encoded.len() - 9;
        key.encoded[dimensions..dimensions + 4].copy_from_slice(&edge.to_le_bytes());
        key.encoded[dimensions + 4..dimensions + 8].copy_from_slice(&edge.to_le_bytes());
        key.dimensions = (edge, edge);
        key
    }
    /// A source that fails its revision check is a `SourceFailure`, not a cache failure.
    fn validate(&self, source: &dyn Source) -> io::Result<()> {
        source.validate().map_err(source_failure)?;
        if self.version != source.version() || self.length != source.length() {
            return Err(source_failure(invalid("Source version changed")));
        }
        Ok(())
    }
    fn validate_frame(&self, frame: &Frame) -> io::Result<()> {
        if frame.frame != self.frame
            || frame.width > self.dimensions.0
            || frame.height > self.dimensions.1
            || api::rgba_length(frame.width, frame.height)? != frame.rgba.len()
        {
            return Err(invalid("Frame does not match cache key"));
        }
        Ok(())
    }
}

/// Persistent records with running totals and an oldest-first index, so budget checks
/// are constant time and each eviction is logarithmic in the record count.
#[derive(Default)]
struct Inventory {
    records: HashMap<String, Record>,
    by_age: BTreeSet<(SystemTime, String)>,
    bytes: u64,
    copies: usize,
}
impl Inventory {
    fn from_records(records: Vec<Record>) -> io::Result<Self> {
        let mut inventory = Self::default();
        for record in records {
            inventory.insert(record)?;
        }
        Ok(inventory)
    }
    fn insert(&mut self, record: Record) -> io::Result<()> {
        self.remove(&record.name);
        self.bytes = self
            .bytes
            .checked_add(record.bytes)
            .ok_or_else(|| invalid("Thumbnail cache byte total overflow"))?;
        self.copies += record.copies;
        self.by_age.insert((record.modified, record.name.clone()));
        self.records.insert(record.name.clone(), record);
        Ok(())
    }
    fn remove(&mut self, name: &str) {
        if let Some(record) = self.records.remove(name) {
            self.by_age.remove(&(record.modified, record.name));
            self.bytes -= record.bytes;
            self.copies -= record.copies;
        }
    }
    fn touch(&mut self, name: &str, modified: SystemTime) {
        if let Some(record) = self.records.get_mut(name) {
            self.by_age.remove(&(record.modified, name.to_owned()));
            record.modified = modified;
            self.by_age.insert((modified, name.to_owned()));
        }
    }
    fn modified(&self, name: &str) -> Option<SystemTime> {
        self.records.get(name).map(|record| record.modified)
    }
    fn contains(&self, name: &str) -> bool {
        self.records.contains_key(name)
    }
    fn oldest(&self) -> Option<String> {
        self.by_age.first().map(|(_, name)| name.clone())
    }
    fn names(&self) -> Vec<String> {
        self.records.keys().cloned().collect()
    }
}

struct MemoryRecord {
    key: Key,
    frame: Arc<Frame>,
    used: u64,
    bytes: usize,
}
/// Warm frames by record name with a least-recently-used index. The name is the hash of
/// the complete key, and the key itself is compared on every hit.
#[derive(Default)]
struct Memory {
    records: HashMap<String, MemoryRecord>,
    by_use: BTreeMap<u64, String>,
    bytes: usize,
    limit: usize,
    tick: u64,
}
impl Memory {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            ..Self::default()
        }
    }
    fn contains(&self, name: &str, key: &Key) -> bool {
        self.records
            .get(name)
            .is_some_and(|record| record.key == *key)
    }
    fn get(&mut self, name: &str, key: &Key) -> Option<Arc<Frame>> {
        let record = self
            .records
            .get_mut(name)
            .filter(|record| record.key == *key)?;
        self.tick = self.tick.wrapping_add(1);
        self.by_use.remove(&record.used);
        record.used = self.tick;
        self.by_use.insert(self.tick, name.to_owned());
        Some(record.frame.clone())
    }
    fn remove(&mut self, name: &str) {
        if let Some(record) = self.records.remove(name) {
            self.by_use.remove(&record.used);
            self.bytes -= record.bytes;
        }
    }
    fn insert(&mut self, name: String, key: Key, frame: Arc<Frame>) {
        let bytes = frame.rgba.len()
            + key.encoded.len()
            + key.version.len()
            + std::mem::size_of::<MemoryRecord>();
        self.remove(&name);
        if bytes > self.limit {
            return;
        }
        while self.bytes + bytes > self.limit {
            let Some((_, oldest)) = self.by_use.pop_first() else {
                break;
            };
            if let Some(record) = self.records.remove(&oldest) {
                self.bytes -= record.bytes;
            }
        }
        self.tick = self.tick.wrapping_add(1);
        self.bytes += bytes;
        self.by_use.insert(self.tick, name.clone());
        self.records.insert(
            name,
            MemoryRecord {
                key,
                frame,
                used: self.tick,
                bytes,
            },
        );
    }
    fn retain(&mut self, keep: impl Fn(&str) -> bool) {
        let removed: Vec<String> = self
            .records
            .keys()
            .filter(|name| !keep(name))
            .cloned()
            .collect();
        for name in removed {
            self.remove(&name);
        }
    }
    fn clear(&mut self) {
        self.records.clear();
        self.by_use.clear();
        self.bytes = 0;
    }
}

pub struct Cache {
    store: Store,
    disk: Inventory,
    disk_limit: u64,
    record_limit: usize,
    disk_generation: Option<u64>,
    memory: Memory,
}
impl Cache {
    pub fn open(root: &Path, disk_limit: u64, memory_limit: usize) -> io::Result<Self> {
        Self::open_bounded(root, disk_limit, memory_limit, MAX_RECORDS)
    }
    /// `record_limit` counts physical files, including duplicate legacy copies.
    fn open_bounded(
        root: &Path,
        disk_limit: u64,
        memory_limit: usize,
        record_limit: usize,
    ) -> io::Result<Self> {
        let store = Store::open(root)?;
        let mut transaction = store.transaction()?;
        let disk = Inventory::from_records(store.records()?)?;
        let mut cache = Self {
            store,
            disk,
            disk_limit,
            record_limit,
            disk_generation: None,
            memory: Memory::new(memory_limit),
        };
        transaction.advance()?;
        cache.evict_disk(0, 0)?;
        cache.disk_generation = transaction.generation();
        Ok(cache)
    }
    pub fn get(&mut self, key: &Key, source: &dyn Source) -> io::Result<Option<Arc<Frame>>> {
        self.get_matching(key, source, false)
            .map(|hit| hit.map(|hit| hit.frame))
    }
    /// Reuse sufficient cached decode coverage without opening or decoding source content.
    pub fn get_covering(
        &mut self,
        key: &Key,
        source: &dyn Source,
    ) -> io::Result<Option<CachedFrame>> {
        self.get_matching(key, source, true)
    }
    fn get_matching(
        &mut self,
        key: &Key,
        source: &dyn Source,
        covering: bool,
    ) -> io::Result<Option<CachedFrame>> {
        key.validate(source)?;
        let mut transaction = self.store.transaction()?;
        let synchronized =
            transaction.generation().is_some() && transaction.generation() == self.disk_generation;
        // Read-only hits leave the shared generation alone, so other instances keep their
        // inventories; removing a damaged record publishes a new generation first.
        let mut lookup = Lookup {
            synchronized,
            transaction: &mut transaction,
            advanced: false,
        };
        let result = (|| {
            if let Some(frame) = self.get_locked(key, &mut lookup)? {
                return Ok(Some(CachedFrame {
                    frame,
                    edge: key.dimensions.0.max(key.dimensions.1),
                }));
            }
            if covering && key.dimensions.0 == key.dimensions.1 {
                for edge in RESOLUTION_EDGES
                    .into_iter()
                    .filter(|edge| *edge > key.dimensions.0)
                {
                    if let Some(frame) = self.get_locked(&key.with_edge(edge), &mut lookup)? {
                        return Ok(Some(CachedFrame { frame, edge }));
                    }
                }
            }
            Ok(None)
        })();
        if synchronized && result.is_ok() {
            self.disk_generation = transaction.generation();
        }
        drop(transaction);
        key.validate(source)?;
        result
    }
    fn get_locked(&mut self, key: &Key, lookup: &mut Lookup<'_>) -> io::Result<Option<Arc<Frame>>> {
        let name = key.filename();
        if self.memory.contains(&name, key) {
            // An unchanged generation proves the record still exists; only refresh its
            // on-disk recency occasionally. After another instance's change, touching
            // verifies that it was not evicted or cleared meanwhile.
            let now = SystemTime::now();
            let recent = lookup.synchronized
                && self.disk.modified(&name).is_some_and(|modified| {
                    now.duration_since(modified)
                        .is_ok_and(|age| age < TOUCH_INTERVAL)
                });
            if !recent {
                match self.store.touch(&name) {
                    Ok(()) => self.disk.touch(&name, now),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {
                        self.memory.remove(&name);
                        self.disk.remove(&name);
                        return Ok(None);
                    }
                    Err(error) => return Err(error),
                }
            }
            return Ok(self.memory.get(&name, key));
        }
        // A synchronized inventory is exact, so absent records need no file probes.
        if lookup.synchronized && !self.disk.contains(&name) {
            return Ok(None);
        }
        let bytes = match self.store.read(&name, MAX_RECORD_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                self.remove_damaged(&name, lookup)?;
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let Some(bytes) = bytes else {
            self.disk.remove(&name);
            return Ok(None);
        };
        let frame = match decode_record(key, &bytes) {
            Ok(frame) => frame,
            Err(_) => {
                self.remove_damaged(&name, lookup)?;
                return Ok(None);
            }
        };
        self.remember_disk(&name)?;
        let frame = Arc::new(frame);
        self.memory.insert(name, key.clone(), frame.clone());
        Ok(Some(frame))
    }
    fn remove_damaged(&mut self, name: &str, lookup: &mut Lookup<'_>) -> io::Result<()> {
        if !lookup.advanced {
            lookup.transaction.advance()?;
            lookup.advanced = true;
        }
        self.remove_record(name)?;
        self.disk.remove(name);
        Ok(())
    }
    pub fn put(&mut self, key: &Key, source: &dyn Source, frame: Frame) -> io::Result<Arc<Frame>> {
        key.validate(source)?;
        let mut transaction = self.store.transaction()?;
        if transaction.generation().is_none() || transaction.generation() != self.disk_generation {
            self.refresh_disk()?;
        }
        transaction.advance()?;
        let result = self.put_locked(key, frame);
        if result.is_ok() {
            self.disk_generation = transaction.generation();
        }
        drop(transaction);
        key.validate(source)?;
        result
    }
    fn put_locked(&mut self, key: &Key, mut frame: Frame) -> io::Result<Arc<Frame>> {
        key.validate_frame(&frame)?;
        // Request identifiers belong to one view generation and must not enter reusable cache data.
        frame.id = 0;
        let mut bytes = b"MTC1".to_vec();
        bytes.extend_from_slice(&(key.encoded.len() as u32).to_le_bytes());
        bytes.extend_from_slice(&key.encoded);
        api::write_message(&mut bytes, &Message::Frame(frame.clone()))?;
        let checksum = blake3::hash(&bytes);
        bytes.extend_from_slice(checksum.as_bytes());
        if bytes.len() as u64 > self.disk_limit {
            return Err(invalid("Cache budget cannot retain generated frame"));
        }
        let name = key.filename();
        self.evict_disk(bytes.len() as u64, 1)?;
        self.store.write(&name, &bytes)?;
        self.remember_disk(&name)?;
        let frame = Arc::new(frame);
        self.memory.insert(name, key.clone(), frame.clone());
        Ok(frame)
    }
    pub fn memory_bytes(&self) -> usize {
        self.memory.bytes
    }
    pub fn clear(&mut self) -> io::Result<()> {
        let mut transaction = self.store.transaction()?;
        self.refresh_disk()?;
        transaction.advance()?;
        self.memory.clear();
        for name in self.disk.names() {
            self.remove_record(&name)?;
            self.disk.remove(&name);
        }
        self.disk_generation = transaction.generation();
        Ok(())
    }
    fn refresh_disk(&mut self) -> io::Result<()> {
        self.disk = Inventory::from_records(self.store.records()?)?;
        let disk = &self.disk;
        self.memory.retain(|name| disk.contains(name));
        Ok(())
    }
    fn remove_record(&self, name: &str) -> io::Result<()> {
        match self.store.remove(name) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
    fn remember_disk(&mut self, name: &str) -> io::Result<()> {
        let record = self
            .store
            .record(name)?
            .ok_or_else(|| invalid("Thumbnail cache record disappeared"))?;
        self.disk.insert(record)
    }
    fn evict_disk(&mut self, extra: u64, count: usize) -> io::Result<()> {
        while self.disk.bytes.saturating_add(extra) > self.disk_limit
            || self.disk.copies + count > self.record_limit
        {
            let Some(name) = self.disk.oldest() else {
                break;
            };
            self.remove_record(&name)?;
            self.disk.remove(&name);
            // Memory must not keep an artifact that was evicted from persistent storage.
            self.memory.remove(&name);
        }
        Ok(())
    }
}

/// One lookup's view of the shared transaction.
struct Lookup<'a> {
    synchronized: bool,
    transaction: &'a mut Transaction,
    advanced: bool,
}

fn decode_record(key: &Key, bytes: &[u8]) -> io::Result<Frame> {
    if bytes.len() < 40 || &bytes[..4] != b"MTC1" {
        return Err(invalid("Invalid cache header"));
    }
    let end = bytes.len() - 32;
    if blake3::hash(&bytes[..end]).as_bytes() != &bytes[end..] {
        return Err(invalid("Corrupt cached artifact"));
    }
    let length = u32::from_le_bytes(bytes[4..8].try_into().expect("four bytes")) as usize;
    if length != key.encoded.len() || bytes.get(8..8 + length) != Some(key.encoded.as_slice()) {
        return Err(invalid("Cache identity mismatch"));
    }
    let mut payload = bytes
        .get(8 + length..end)
        .ok_or_else(|| invalid("Truncated cached frame"))?;
    let Message::Frame(frame) = api::read_message(&mut payload)? else {
        return Err(invalid("Cached object is not a frame"));
    };
    if !payload.is_empty() {
        return Err(invalid("Trailing cache data"));
    }
    key.validate_frame(&frame)?;
    Ok(frame)
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
#[path = "cache_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "cache_revision_tests.rs"]
mod revision_tests;
