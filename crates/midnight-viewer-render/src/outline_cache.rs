//! Hinted monochrome paths are independent of raster phase. Keep their exact
//! geometry bounded, and reject offscreen ink before allocating a glyph mask.
use crate::glyph_cache::GlyphKey;
use std::collections::{HashMap, VecDeque};
use swash::{
    scale::{
        Scaler, Source,
        image::{Content, Image},
    },
    zeno::{Bounds, Mask, Origin, Point, Scratch, Transform, Vector, Verb},
};

const MAX_BYTES: usize = 2 * 1024 * 1024;
const MAX_ENTRIES: usize = 512;

struct Path {
    points: Box<[Point]>,
    verbs: Box<[Verb]>,
    bounds: Bounds,
}

impl Path {
    fn render(
        &self,
        key: &GlyphKey,
        origin: [f64; 2],
        size: [u32; 2],
        scratch: &mut Scratch,
    ) -> Option<Image> {
        let fraction = key.fraction.map(f32::from_bits);
        // These are the actual hinted/synthesized control-point bounds, including
        // accents and negative bearings. Two pixels cover phase and mask edges.
        let left = origin[0] + f64::from(self.bounds.min.x + fraction[0]);
        let top = origin[1] - f64::from(self.bounds.max.y - fraction[1]);
        let right = origin[0] + f64::from(self.bounds.max.x + fraction[0]);
        let bottom = origin[1] - f64::from(self.bounds.min.y - fraction[1]);
        if self.points.is_empty()
            || left >= f64::from(size[0]) + 2.0
            || top >= f64::from(size[1]) + 2.0
            || right <= -2.0
            || bottom <= -2.0
        {
            return None;
        }
        let offset = Vector::new(fraction[0], -fraction[1]);
        let mut image = Image::new();
        image.placement = Mask::with_scratch((&*self.points, &*self.verbs), scratch)
            .origin(Origin::BottomLeft)
            .offset(offset)
            .render_offset(offset)
            .inspect(|format, width, height| {
                image.data.resize(format.buffer_size(width, height), 0)
            })
            .render_into(&mut image.data, None);
        image.content = Content::Mask;
        image.source = Source::Outline;
        Some(image)
    }
}

#[derive(Default)]
pub(super) struct OutlineCache {
    paths: HashMap<GlyphKey, Path>,
    order: VecDeque<GlyphKey>,
    bytes: usize,
    scratch: Scratch,
}

fn path_key(key: &GlyphKey) -> GlyphKey {
    let mut key = key.clone();
    key.fraction = [0; 2];
    key.color = [0; 4];
    key
}

fn entry_bytes(key: &GlyphKey, path: &Path) -> usize {
    // Boxed paths have no unused vector capacity. Both stored keys own coords.
    std::mem::size_of::<Path>()
        + 2 * std::mem::size_of::<GlyphKey>()
        + std::mem::size_of_val(path.points.as_ref())
        + std::mem::size_of_val(path.verbs.as_ref())
        + 2 * key.coordinates.capacity() * std::mem::size_of::<i16>()
}

impl OutlineCache {
    /// Some(None) is a cached path with no visible ink, never a missing glyph.
    pub(super) fn get(
        &mut self,
        key: &GlyphKey,
        origin: [f64; 2],
        size: [u32; 2],
    ) -> Option<Option<Image>> {
        let path = self.paths.get(&path_key(key))?;
        Some(path.render(key, origin, size, &mut self.scratch))
    }

    pub(super) fn build(
        &mut self,
        key: &GlyphKey,
        scaler: &mut Scaler<'_>,
        origin: [f64; 2],
        size: [u32; 2],
    ) -> Option<Option<Image>> {
        // Keep the established source preference/fallback for mixed and bitmap
        // fonts completely unchanged. Do not cache unsupported/negative entries.
        if scaler.has_color_outlines() || scaler.has_color_bitmaps() || scaler.has_bitmaps() {
            return None;
        }
        let mut outline = scaler.scale_outline(key.glyph)?;
        if key.embolden {
            let strength = f32::from_bits(key.size) * 0.02;
            outline.embolden(strength, strength);
        }
        let skew = f32::from_bits(key.skew);
        if skew != 0.0 {
            outline.transform(&Transform::new(
                1.0,
                0.0,
                skew.to_radians().tan(),
                1.0,
                0.0,
                0.0,
            ));
        }
        if outline
            .points()
            .iter()
            .any(|p| !p.x.is_finite() || !p.y.is_finite())
        {
            return None;
        }
        let path = Path {
            bounds: outline.bounds(),
            points: outline.points().into(),
            verbs: outline.verbs().into(),
        };
        let image = path.render(key, origin, size, &mut self.scratch);
        self.insert(path_key(key), path);
        Some(image)
    }

    fn insert(&mut self, key: GlyphKey, path: Path) {
        let bytes = entry_bytes(&key, &path);
        if bytes > MAX_BYTES || self.paths.contains_key(&key) {
            return;
        }
        while self.bytes + bytes > MAX_BYTES || self.paths.len() >= MAX_ENTRIES {
            let Some(old) = self.order.pop_front() else {
                break;
            };
            if let Some(path) = self.paths.remove(&old) {
                self.bytes -= entry_bytes(&old, &path);
            }
        }
        self.bytes += bytes;
        self.order.push_back(key.clone());
        self.paths.insert(key, path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(glyph: u16) -> GlyphKey {
        GlyphKey {
            font: (1, 0),
            size: 19.0f32.to_bits(),
            coordinates: vec![1, 2],
            glyph,
            fraction: [0; 2],
            color: [255; 4],
            embolden: false,
            skew: 0.0f32.to_bits(),
        }
    }
    fn path(points: usize) -> Path {
        Path {
            points: vec![Point::new(0.0, 0.0); points].into(),
            verbs: vec![Verb::MoveTo; points].into(),
            bounds: Bounds::default(),
        }
    }
    #[test]
    fn storage_bounds_include_path_bytes_and_variation_keys() {
        let mut cache = OutlineCache::default();
        cache.insert(key(0), path(MAX_BYTES / std::mem::size_of::<Point>() + 1));
        assert!(
            cache.paths.is_empty(),
            "one oversized outline must not enter the cache"
        );
        for glyph in 1..1500 {
            cache.insert(key(glyph), path(256));
        }
        assert!(cache.bytes <= MAX_BYTES);
        assert!(cache.paths.len() <= MAX_ENTRIES);
        assert_eq!(cache.paths.len(), cache.order.len());
        assert!(cache.paths.contains_key(&key(1499)));
        assert!(!cache.paths.contains_key(&key(1)));
    }
    #[test]
    fn phase_and_brush_share_paths_but_font_variation_size_and_synthesis_do_not() {
        let mut cache = OutlineCache::default();
        let original = key(1);
        cache.insert(path_key(&original), path(1));
        let mut phase = original.clone();
        phase.fraction = [0.25f32.to_bits(), 0.75f32.to_bits()];
        phase.color = [13; 4];
        assert!(cache.paths.contains_key(&path_key(&phase)));
        for variant in 0..5 {
            let mut key = original.clone();
            match variant {
                0 => key.font.1 += 1,
                1 => key.coordinates[0] += 1,
                2 => key.size = 21.0f32.to_bits(),
                3 => key.embolden = true,
                _ => key.skew = 12.0f32.to_bits(),
            }
            assert!(!cache.paths.contains_key(&path_key(&key)));
        }
    }
}
