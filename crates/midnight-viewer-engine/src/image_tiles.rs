//! Lazy source-raster tiles. Camera motion never changes a tile's identity or pixels.
use midnight_core::{
    VIEWER_TILE_BYTES, VIEWER_TILE_CACHE_BYTES, VIEWER_TILE_CACHE_ENTRIES, VIEWER_TILE_EDGE,
    ViewerPixelOwner, ViewerPixels, ViewerTileId,
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, AtomicUsize, Ordering},
    },
};

pub(crate) const EDGE: u32 = VIEWER_TILE_EDGE;
pub(crate) const TILE_BYTES: usize = VIEWER_TILE_BYTES;
const CACHE_BYTES: usize = VIEWER_TILE_CACHE_BYTES;
const TOTAL_BYTES: usize = 64 * 1024 * 1024;
const CACHE_ENTRIES: usize = VIEWER_TILE_CACHE_ENTRIES;
/// `Key::page` of the tiles of a document's Continuous canvas, which holds every page.
pub(crate) const CANVAS: usize = usize::MAX;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub(crate) struct Key {
    /// Image page, document page in the Paged layout, or `CANVAS`.
    pub page: usize,
    pub scale: u64,
    pub x: i32,
    pub y: i32,
}

#[derive(Clone)]
pub(crate) struct Plan {
    pub visible: Vec<Key>,
    /// Prefetched tiles beyond the view in the latest direction of motion, nearest first.
    pub guards: Vec<Key>,
    pub raster_origin: [f64; 2],
    pub raster_size: [u32; 2],
    pub scale: f64,
}

impl Plan {
    /// The visible tiles and one guard row and column in the latest direction.
    pub fn new(
        page: usize,
        scale: f64,
        origin: [f64; 2],
        size: [u32; 2],
        direction: [f64; 2],
    ) -> Self {
        Self::with_depth(page, scale, origin, size, direction, [1, 1])
    }

    /// The visible tiles and `depth` guard columns and rows in the latest direction of
    /// motion (right and down before any motion), including the diagonal corner.
    pub fn with_depth(
        page: usize,
        scale: f64,
        origin: [f64; 2],
        size: [u32; 2],
        direction: [f64; 2],
        depth: [i32; 2],
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
        let ahead = |axis: usize| -> Vec<i32> {
            let depth = depth[axis].max(0);
            if direction[axis] < 0.0 {
                (low[axis] - depth..low[axis]).rev().collect()
            } else {
                (high[axis]..high[axis] + depth).collect()
            }
        };
        let (columns, rows) = (ahead(0), ahead(1));
        for x in &columns {
            for y in low[1]..high[1] {
                guards.push(key(*x, y));
            }
        }
        for y in &rows {
            for x in low[0]..high[0] {
                guards.push(key(x, *y));
            }
        }
        for y in &rows {
            for x in &columns {
                guards.push(key(*x, *y));
            }
        }
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

/// How many tiles the cache keeps, and how many bytes of pixels may exist in all: cached,
/// still shown by a published frame after eviction, or reserved for a render in flight.
#[derive(Clone, Copy)]
struct Budget {
    entries: usize,
    bytes: usize,
    total: usize,
}

pub(crate) struct Tiles {
    entries: HashMap<Key, Entry>,
    bytes: Arc<AtomicUsize>,
    ids: Arc<AtomicU64>,
    clock: u64,
    budget: Budget,
}

impl Tiles {
    pub fn new(ids: Arc<AtomicU64>) -> Self {
        Self {
            entries: HashMap::new(),
            bytes: Arc::new(AtomicUsize::new(0)),
            ids,
            clock: 0,
            budget: Budget {
                entries: CACHE_ENTRIES,
                bytes: CACHE_BYTES,
                total: TOTAL_BYTES,
            },
        }
    }

    /// Keep at most `bytes` of tiles (documents: `viewer_document_tile_cache_bytes`).
    /// Pixels in all, including those still shown or rendering, stay within twice that.
    pub fn set_budget(&mut self, bytes: usize) {
        let bytes = bytes.max(TILE_BYTES) / TILE_BYTES * TILE_BYTES;
        self.budget = Budget {
            entries: bytes / TILE_BYTES,
            bytes,
            total: bytes.saturating_mul(2),
        };
    }

    /// Tiles the cache keeps at most.
    #[cfg(test)]
    pub fn capacity(&self) -> usize {
        self.budget.entries
    }

    pub fn reserve(&self) -> Option<Arc<Charge>> {
        let total = self.budget.total;
        self.bytes
            .try_update(Ordering::AcqRel, Ordering::Relaxed, |bytes| {
                bytes.checked_add(TILE_BYTES).filter(|next| *next <= total)
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

    /// Keep `pixels` under `key`, evicting the least recently used tiles outside
    /// `protected` when the budget is full.
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
        while self.full() {
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
        self.store(key, pixels)
    }

    /// Keep a prefetched tile beyond `protected`: it may replace the least recently used
    /// tile outside `protected`, and is dropped when every kept tile is protected.
    pub fn insert_ahead(
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
        while self.full() {
            let oldest = self
                .entries
                .iter()
                .filter(|(key, _)| !protected.keys().any(|keep| keep == **key))
                .min_by_key(|(_, entry)| entry.touched)
                .map(|(key, _)| *key);
            let Some(oldest) = oldest else {
                return Ok(());
            };
            self.entries.remove(&oldest);
        }
        self.store(key, pixels)
    }

    fn full(&self) -> bool {
        self.entries.len() >= self.budget.entries
            || (self.entries.len() + 1) * TILE_BYTES > self.budget.bytes
    }

    fn store(&mut self, key: Key, pixels: Arc<ViewerPixels>) -> Result<(), String> {
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
