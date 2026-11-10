//! Lazy source-raster tiles. Camera motion never changes a tile's identity or pixels.
use midnight_core::{ViewerPixelOwner, ViewerPixels, ViewerTileId};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

pub(crate) const EDGE: u32 = 256;
const TILE_BYTES: usize = EDGE as usize * EDGE as usize * 4;
const CACHE_BYTES: usize = 32 * 1024 * 1024;
const TOTAL_BYTES: usize = 64 * 1024 * 1024;
const CACHE_ENTRIES: usize = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) struct Key {
    pub page: usize,
    pub scale: u64,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone)]
pub(crate) struct Plan {
    pub visible: Vec<Key>,
    pub guards: Vec<Key>,
    pub raster_origin: [f64; 2],
    pub raster_size: [u32; 2],
    pub scale: f64,
}

impl Plan {
    pub fn new(
        page: usize,
        scale: f64,
        origin: [f64; 2],
        size: [u32; 2],
        direction: [f64; 2],
    ) -> Self {
        let raster_origin = origin.map(|value| value * scale);
        let low = raster_origin.map(|value| (value / f64::from(EDGE)).floor() as i32);
        let high = std::array::from_fn::<_, 2, _>(|axis| {
            // Uniform density rounds the output edge down. Cover the remaining
            // fractional physical pixel too, so its stretched edge never blanks.
            ((raster_origin[axis] + f64::from(size[axis]) + 1.0) / f64::from(EDGE)).ceil() as i32
        });
        let mut visible = Vec::new();
        let mut guards = Vec::new();
        let key = |x, y| Key {
            page,
            scale: scale.to_bits(),
            x,
            y,
        };
        for y in low[1]..high[1] {
            for x in low[0]..high[0] {
                visible.push(key(x, y));
            }
        }
        // One row/column in the latest direction, including the diagonal corner.
        let horizontal = if direction[0] < 0.0 {
            low[0] - 1
        } else {
            high[0]
        };
        let vertical = if direction[1] < 0.0 {
            low[1] - 1
        } else {
            high[1]
        };
        for y in low[1]..high[1] {
            guards.push(key(horizontal, y));
        }
        for x in low[0]..high[0] {
            guards.push(key(x, vertical));
        }
        guards.push(key(horizontal, vertical));
        Self {
            visible,
            guards,
            raster_origin,
            raster_size: size,
            scale,
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = Key> + '_ {
        self.visible.iter().chain(&self.guards).copied()
    }
}

#[derive(Debug)]
pub(crate) struct Charge {
    bytes: Arc<AtomicUsize>,
    amount: usize,
}
impl ViewerPixelOwner for Charge {}
impl Drop for Charge {
    fn drop(&mut self) {
        self.bytes.fetch_sub(self.amount, Ordering::AcqRel);
    }
}

struct Entry {
    id: ViewerTileId,
    pixels: Arc<ViewerPixels>,
    touched: u64,
}

pub(crate) struct Tiles {
    entries: HashMap<Key, Entry>,
    bytes: Arc<AtomicUsize>,
    ids: Arc<AtomicU64>,
    clock: u64,
}

impl Tiles {
    pub fn new(ids: Arc<AtomicU64>) -> Self {
        Self {
            entries: HashMap::new(),
            bytes: Arc::new(AtomicUsize::new(0)),
            ids,
            clock: 0,
        }
    }

    pub fn reserve(&self) -> Option<Arc<Charge>> {
        self.bytes
            .try_update(Ordering::AcqRel, Ordering::Relaxed, |bytes| {
                bytes
                    .checked_add(TILE_BYTES)
                    .filter(|next| *next <= TOTAL_BYTES)
            })
            .ok()?;
        Some(Arc::new(Charge {
            bytes: self.bytes.clone(),
            amount: TILE_BYTES,
        }))
    }

    pub fn contains(&self, key: Key) -> bool {
        self.entries.contains_key(&key)
    }

    pub fn get(&mut self, key: Key) -> Option<(ViewerTileId, Arc<ViewerPixels>)> {
        let entry = self.entries.get_mut(&key)?;
        self.clock = self.clock.saturating_add(1);
        entry.touched = self.clock;
        Some((entry.id, entry.pixels.clone()))
    }

    pub fn insert(
        &mut self,
        key: Key,
        pixels: Arc<ViewerPixels>,
        protected: &Plan,
    ) -> Result<(), String> {
        if pixels.rgba().len() != TILE_BYTES {
            return Err("Invalid image tile byte length.".into());
        }
        if self.contains(key) {
            return Ok(());
        }
        while self.entries.len() >= CACHE_ENTRIES
            || (self.entries.len() + 1) * TILE_BYTES > CACHE_BYTES
        {
            let oldest = self
                .entries
                .iter()
                .filter(|(key, _)| !protected.keys().any(|keep| keep == **key))
                .min_by_key(|(_, entry)| entry.touched)
                .map(|(key, _)| *key);
            let Some(oldest) = oldest else {
                return Err("Image tile working set exceeds its cache.".into());
            };
            self.entries.remove(&oldest);
        }
        let id = next_id(&self.ids)?;
        self.clock = self.clock.saturating_add(1);
        self.entries.insert(
            key,
            Entry {
                id: ViewerTileId(id),
                pixels,
                touched: self.clock,
            },
        );
        Ok(())
    }
}

pub(crate) fn next_id(ids: &AtomicU64) -> Result<u64, String> {
    ids.try_update(Ordering::AcqRel, Ordering::Relaxed, |id| id.checked_add(1))
        .map(|id| id + 1)
        .map_err(|_| "Viewer tile identity exhausted.".into())
}

#[cfg(test)]
#[path = "image_tiles_tests.rs"]
mod tests;
