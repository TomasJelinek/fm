//! Private persistent RGBA cache; a successful write is required before a generated frame is used.
use crate::decode::Source;
use midnight_thumbnail_api::{self as api, Frame, Message};
use midnight_worker_platform::store::{PrivateStore, Record};
use std::{collections::HashMap, io, path::Path, sync::Arc, time::SystemTime};

pub const DEFAULT_DISK_BYTES: u64 = 512 * 1024 * 1024;
pub const DEFAULT_MEMORY_BYTES: usize = 64 * 1024 * 1024;

const MAX_KEY_BYTES: usize = 16 * 1024;
const MAX_RECORDS: usize = 8192;
const MAX_RECORD_BYTES: usize = api::MAX_FRAME_BYTES + MAX_KEY_BYTES + 128;

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
        source.validate()?;
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
    fn validate(&self, source: &dyn Source) -> io::Result<()> {
        source.validate()?;
        if self.version != source.version() || self.length != source.length() {
            return Err(invalid("Source version changed"));
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

struct MemoryRecord {
    frame: Arc<Frame>,
    used: u64,
    bytes: usize,
}
pub struct Cache {
    store: PrivateStore,
    disk: HashMap<String, Record>,
    disk_bytes: u64,
    disk_limit: u64,
    disk_generation: Option<u64>,
    memory: HashMap<Key, MemoryRecord>,
    memory_bytes: usize,
    memory_limit: usize,
    tick: u64,
}
impl Cache {
    pub fn open(root: &Path, disk_limit: u64, memory_limit: usize) -> io::Result<Self> {
        let store = PrivateStore::open(root)?;
        let mut transaction = store.transaction()?;
        let records = store.records()?;
        let disk_bytes = records.iter().map(|record| record.bytes).sum();
        let mut cache = Self {
            store,
            disk: records
                .into_iter()
                .map(|record| (record.name.clone(), record))
                .collect(),
            disk_bytes,
            disk_limit,
            disk_generation: None,
            memory: HashMap::new(),
            memory_bytes: 0,
            memory_limit,
            tick: 0,
        };
        transaction.advance()?;
        cache.evict_disk(0, 0)?;
        cache.disk_generation = transaction.generation();
        Ok(cache)
    }
    pub fn get(&mut self, key: &Key, source: &dyn Source) -> io::Result<Option<Arc<Frame>>> {
        key.validate(source)?;
        let mut transaction = self.store.transaction()?;
        let synchronized =
            transaction.generation().is_some() && transaction.generation() == self.disk_generation;
        transaction.advance()?;
        let result = self.get_locked(key);
        if synchronized && result.is_ok() {
            self.disk_generation = transaction.generation();
        }
        drop(transaction);
        key.validate(source)?;
        result
    }
    fn get_locked(&mut self, key: &Key) -> io::Result<Option<Arc<Frame>>> {
        self.tick = self.tick.wrapping_add(1);
        let name = key.filename();
        if let Some(record) = self.memory.get_mut(key) {
            match self.store.touch(&name) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    if let Some(removed) = self.memory.remove(key) {
                        self.memory_bytes -= removed.bytes;
                    }
                    self.forget_disk(&name);
                    return Ok(None);
                }
                Err(error) => return Err(error),
            }
            if let Some(record) = self.disk.get_mut(&name) {
                record.modified = SystemTime::now();
            }
            record.used = self.tick;
            return Ok(Some(record.frame.clone()));
        }
        let bytes = match self.store.read(&name, MAX_RECORD_BYTES) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::InvalidData => {
                self.remove_record(&name)?;
                self.forget_disk(&name);
                return Ok(None);
            }
            Err(error) => return Err(error),
        };
        let Some(bytes) = bytes else {
            self.forget_disk(&name);
            return Ok(None);
        };
        let frame = match decode_record(key, &bytes) {
            Ok(frame) => frame,
            Err(_) => {
                self.remove_record(&name)?;
                self.forget_disk(&name);
                return Ok(None);
            }
        };
        self.forget_disk(&name);
        self.disk_bytes += bytes.len() as u64;
        self.disk.insert(
            name.clone(),
            Record {
                name,
                bytes: bytes.len() as u64,
                modified: SystemTime::now(),
            },
        );
        let frame = Arc::new(frame);
        self.remember(key.clone(), frame.clone());
        Ok(Some(frame))
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
        self.forget_disk(&name);
        self.disk_bytes += bytes.len() as u64;
        self.disk.insert(
            name.clone(),
            Record {
                name,
                bytes: bytes.len() as u64,
                modified: SystemTime::now(),
            },
        );
        let frame = Arc::new(frame);
        self.remember(key.clone(), frame.clone());
        Ok(frame)
    }
    pub fn memory_bytes(&self) -> usize {
        self.memory_bytes
    }
    pub fn clear(&mut self) -> io::Result<()> {
        let mut transaction = self.store.transaction()?;
        self.refresh_disk()?;
        transaction.advance()?;
        self.memory.clear();
        self.memory_bytes = 0;
        let names: Vec<_> = self.disk.keys().cloned().collect();
        for name in names {
            self.remove_record(&name)?;
            self.forget_disk(&name);
        }
        self.disk_generation = transaction.generation();
        Ok(())
    }
    fn refresh_disk(&mut self) -> io::Result<()> {
        let records = self.store.records()?;
        self.disk_bytes = records.iter().map(|record| record.bytes).sum();
        self.disk = records
            .into_iter()
            .map(|record| (record.name.clone(), record))
            .collect();
        self.memory
            .retain(|key, _| self.disk.contains_key(&key.filename()));
        self.memory_bytes = self.memory.values().map(|record| record.bytes).sum();
        Ok(())
    }
    fn remove_record(&self, name: &str) -> io::Result<()> {
        match self.store.remove(name) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            result => result,
        }
    }
    fn forget_disk(&mut self, name: &str) {
        if let Some(record) = self.disk.remove(name) {
            self.disk_bytes -= record.bytes;
        }
    }
    fn evict_disk(&mut self, extra: u64, count: usize) -> io::Result<()> {
        while self.disk_bytes.saturating_add(extra) > self.disk_limit
            || self.disk.len() + count > MAX_RECORDS
        {
            let Some(name) = self
                .disk
                .values()
                .min_by_key(|record| record.modified)
                .map(|record| record.name.clone())
            else {
                break;
            };
            self.remove_record(&name)?;
            self.forget_disk(&name);
            // Memory must not keep an artifact that was evicted from persistent storage.
            if let Some(key) = self
                .memory
                .keys()
                .find(|key| key.filename() == name)
                .cloned()
            {
                self.memory_bytes -= self
                    .memory
                    .remove(&key)
                    .expect("existing memory record")
                    .bytes;
            }
        }
        Ok(())
    }
    fn remember(&mut self, key: Key, frame: Arc<Frame>) {
        let bytes = frame.rgba.len()
            + key.encoded.len()
            + key.version.len()
            + std::mem::size_of::<MemoryRecord>();
        if let Some(previous) = self.memory.remove(&key) {
            self.memory_bytes -= previous.bytes;
        }
        if bytes > self.memory_limit {
            return;
        }
        while self.memory_bytes + bytes > self.memory_limit {
            let Some(oldest) = self
                .memory
                .iter()
                .min_by_key(|(_, record)| record.used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.memory_bytes -= self
                .memory
                .remove(&oldest)
                .expect("existing memory record")
                .bytes;
        }
        self.tick = self.tick.wrapping_add(1);
        self.memory_bytes += bytes;
        self.memory.insert(
            key,
            MemoryRecord {
                frame,
                used: self.tick,
                bytes,
            },
        );
    }
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
