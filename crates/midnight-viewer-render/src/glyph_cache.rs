use std::collections::{HashMap, VecDeque};
use swash::scale::image::Image;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub(crate) struct GlyphKey {
    pub font: (u64, u32),
    pub size: u32,
    pub coordinates: Vec<i16>,
    pub glyph: u16,
    pub fraction: [u32; 2],
    pub color: [u8; 4],
    pub embolden: bool,
    pub skew: u32,
}

#[derive(Default)]
pub(crate) struct GlyphCache {
    images: HashMap<GlyphKey, Option<Image>>,
    order: VecDeque<GlyphKey>,
    bytes: usize,
}

impl GlyphCache {
    pub(crate) fn get(&self, key: &GlyphKey) -> Option<&Option<Image>> {
        self.images.get(key)
    }

    pub(crate) fn insert(&mut self, key: GlyphKey, image: Option<Image>) {
        const MAX_BYTES: usize = 8 * 1024 * 1024;
        let bytes = image.as_ref().map_or(0, |image| image.data.capacity());
        if bytes > MAX_BYTES || self.images.contains_key(&key) {
            return;
        }
        while self.bytes + bytes > MAX_BYTES || self.images.len() >= 4096 {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(Some(image)) = self.images.remove(&old) {
                self.bytes -= image.data.capacity();
            }
        }
        self.bytes += bytes;
        self.order.push_back(key.clone());
        self.images.insert(key, image);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(glyph: u16) -> GlyphKey {
        GlyphKey {
            font: (1, 0),
            size: 18.0f32.to_bits(),
            coordinates: vec![],
            glyph,
            fraction: [0; 2],
            color: [255; 4],
            embolden: false,
            skew: 0,
        }
    }

    #[test]
    fn cache_keeps_images_and_empty_glyphs_with_bounded_storage() {
        let mut cache = GlyphCache::default();
        cache.insert(
            key(1),
            Some(Image {
                data: vec![7; 3 * 1024 * 1024],
                ..Image::default()
            }),
        );
        assert_eq!(cache.get(&key(1)).unwrap().as_ref().unwrap().data[0], 7);
        cache.insert(
            key(2),
            Some(Image {
                data: vec![9; 3 * 1024 * 1024],
                ..Image::default()
            }),
        );
        cache.insert(
            key(3),
            Some(Image {
                data: vec![11; 3 * 1024 * 1024],
                ..Image::default()
            }),
        );
        assert!(
            cache.get(&key(1)).is_none(),
            "evict oldest before crossing 8 MiB"
        );
        assert!(cache.bytes <= 8 * 1024 * 1024);
        for id in 4..5000 {
            cache.insert(key(id), None);
        }
        assert_eq!(cache.get(&key(4999)).map(Option::is_none), Some(true));
        assert!(cache.images.len() <= 4096);
        assert_eq!(cache.images.len(), cache.order.len());
    }
}
