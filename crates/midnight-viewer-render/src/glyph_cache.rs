use std::collections::{HashMap, VecDeque};
use swash::scale::image::Image;

/// Fixed-size identity of one rendered glyph image at one raster phase.
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub(crate) struct GlyphKey {
    pub font: (u64, u32),
    pub size: u32,
    /// Interned normalized variation coordinates; see `GlyphPainter`.
    pub coordinates: u32,
    pub glyph: u16,
    pub fraction: [u32; 2],
    /// Brush of color-font glyphs. Zero for other fonts: their coverage masks
    /// are painted with the brush later and do not depend on it.
    pub color: [u8; 4],
    pub embolden: bool,
    pub skew: u32,
}

const MAX_BYTES: usize = 8 * 1024 * 1024;
const MAX_ENTRIES: usize = 32 * 1024;

struct Cached {
    image: Option<Image>,
    referenced: bool,
}

/// Phase images bounded by bytes, including per-entry bookkeeping. Eviction is
/// second-chance (CLOCK): a glyph drawn again since the last sweep stays, so a
/// repainted raster keeps its working set while scrolled-out glyphs age out.
#[derive(Default)]
pub(crate) struct GlyphCache {
    images: HashMap<GlyphKey, Cached>,
    order: VecDeque<GlyphKey>,
    bytes: usize,
}

fn entry_bytes(image: &Option<Image>) -> usize {
    // The key is stored in the map and in the clock queue.
    2 * std::mem::size_of::<GlyphKey>()
        + std::mem::size_of::<Cached>()
        + image.as_ref().map_or(0, |image| image.data.capacity())
}

impl GlyphCache {
    pub(crate) fn get(&mut self, key: &GlyphKey) -> Option<&Option<Image>> {
        let entry = self.images.get_mut(key)?;
        entry.referenced = true;
        Some(&entry.image)
    }

    pub(crate) fn insert(&mut self, key: GlyphKey, image: Option<Image>) {
        let bytes = entry_bytes(&image);
        if bytes > MAX_BYTES || self.images.contains_key(&key) {
            return;
        }
        // Each pass either evicts or clears one reference bit, so the sweep ends
        // after at most two passes over the queue.
        while self.bytes + bytes > MAX_BYTES || self.images.len() >= MAX_ENTRIES {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            let Some(entry) = self.images.get_mut(&old) else {
                continue;
            };
            if std::mem::take(&mut entry.referenced) {
                self.order.push_back(old);
                continue;
            }
            if let Some(entry) = self.images.remove(&old) {
                self.bytes -= entry_bytes(&entry.image);
            }
        }
        self.bytes += bytes;
        self.order.push_back(key);
        self.images.insert(
            key,
            Cached {
                image,
                referenced: false,
            },
        );
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(glyph: u16) -> GlyphKey {
        GlyphKey {
            font: (1, 0),
            size: 18.0f32.to_bits(),
            coordinates: 0,
            glyph,
            fraction: [0; 2],
            color: [0; 4],
            embolden: false,
            skew: 0,
        }
    }

    fn image(bytes: usize, value: u8) -> Option<Image> {
        Some(Image {
            data: vec![value; bytes],
            ..Image::default()
        })
    }

    #[test]
    fn cache_keeps_images_and_empty_glyphs_with_bounded_storage() {
        let mut cache = GlyphCache::default();
        cache.insert(key(1), image(3 * 1024 * 1024, 7));
        assert_eq!(cache.get(&key(1)).unwrap().as_ref().unwrap().data[0], 7);
        cache.insert(key(2), image(3 * 1024 * 1024, 9));
        cache.insert(key(3), image(3 * 1024 * 1024, 11));
        assert!(
            cache.get(&key(2)).is_none(),
            "evict the oldest unused image before crossing 8 MiB"
        );
        assert!(
            cache.get(&key(1)).is_some(),
            "a used image gets a second chance"
        );
        assert!(cache.bytes <= MAX_BYTES);
        for id in 4..40_000 {
            cache.insert(key(id), None);
        }
        assert_eq!(cache.get(&key(39_999)).map(Option::is_none), Some(true));
        assert!(cache.images.len() <= MAX_ENTRIES);
        assert!(cache.bytes <= MAX_BYTES);
        assert_eq!(cache.images.len(), cache.order.len());
    }

    #[test]
    fn repeatedly_drawn_working_set_survives_a_stream_of_new_glyphs() {
        // Small masks: the byte budget, not a fixed entry count, bounds the cache.
        let mut cache = GlyphCache::default();
        let working: Vec<_> = (0..8_000).map(key).collect();
        for key in &working {
            cache.insert(*key, image(200, 1));
        }
        // A repaint of the same raster hits every entry; then scrolling streams
        // in more new glyphs than the cache can hold besides the working set.
        for round in 0..4u16 {
            assert!(working.iter().all(|key| cache.get(key).is_some()));
            for id in 0..6_000 {
                let mut fresh = key(id);
                fresh.font.1 = 1 + u32::from(round);
                cache.insert(fresh, image(200, 2));
            }
        }
        assert!(working.iter().all(|key| cache.get(key).is_some()));
        assert!(cache.bytes <= MAX_BYTES && cache.images.len() <= MAX_ENTRIES);
    }
}
