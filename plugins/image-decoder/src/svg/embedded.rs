//! Raster images that an SVG embeds as `data:` URIs.
//!
//! usvg decodes the base64 of each `data:` URI and asks this resolver for an image. PNG,
//! JPEG, GIF and WebP go through the crate's raster path with every limit of a raster
//! file (32768-pixel edges, 128 MiB of pixels, ICC to sRGB, orientation) and come back as
//! a PNG of those pixels, so resvg only ever decodes images that this crate encoded.
//! Identical data is decoded once, and the pixels of all distinct images share one budget.
//! Linked images (file paths and URLs) reach the string resolver, which opens nothing.
use super::Omission;
use image::{
    ExtendedColorType, ImageEncoder,
    codecs::png::{CompressionType, FilterType, PngEncoder},
};
use resvg::usvg::{ImageHrefResolver, ImageKind};
use std::{
    collections::HashMap,
    hash::{DefaultHasher, Hash, Hasher},
    sync::{Arc, Mutex},
};

/// Decoded pixels of all distinct embedded images together: one raster file's limit.
pub(super) const MAX_EMBEDDED_BYTES: u64 = crate::MAX_DECODED_BYTES;

/// The resolver's state, shared with the decoder that reads its results afterwards.
#[derive(Clone, Default)]
pub(super) struct Images(Arc<Mutex<State>>);

/// Source data and its PNG, `None` when it was not decoded.
type Entry = (Arc<Vec<u8>>, Option<Arc<Vec<u8>>>);

#[derive(Default)]
struct State {
    /// Decoded images by hash of their source data.
    decoded: HashMap<u64, Vec<Entry>>,
    pixel_bytes: u64,
    retained: u64,
    skipped: bool,
}

impl Images {
    pub(super) fn resolver(&self) -> ImageHrefResolver<'static> {
        let images = self.clone();
        ImageHrefResolver {
            resolve_data: Box::new(move |_, data, _| images.resolve(data)),
            resolve_string: Box::new(|_, _| None),
        }
    }

    /// Bytes of encoded images that the converted tree keeps.
    pub(super) fn retained(&self) -> u64 {
        self.0.lock().map_or(0, |state| state.retained)
    }

    pub(super) fn omission(&self) -> Option<Omission> {
        let skipped = match self.0.lock() {
            Ok(state) => state.skipped,
            Err(_) => true,
        };
        skipped.then_some(Omission::EmbeddedImages)
    }

    fn resolve(&self, data: Arc<Vec<u8>>) -> Option<ImageKind> {
        let mut state = self.0.lock().ok()?;
        let mut hasher = DefaultHasher::new();
        data.hash(&mut hasher);
        let key = hasher.finish();
        let known = state.decoded.get(&key).and_then(|entries| {
            entries
                .iter()
                .find(|(source, _)| **source == *data)
                .map(|(_, png)| png.clone())
        });
        let png = match known {
            Some(png) => png,
            None => {
                let png = state.decode(&data);
                state
                    .decoded
                    .entry(key)
                    .or_default()
                    .push((data, png.clone()));
                png
            }
        };
        png.map(ImageKind::PNG)
    }
}

impl State {
    /// Decode a new image within the shared budget, or record that one was skipped.
    fn decode(&mut self, data: &[u8]) -> Option<Arc<Vec<u8>>> {
        let png = crate::embedded_raster(data).ok().and_then(|pixels| {
            let bytes = u64::from(pixels.width()) * u64::from(pixels.height()) * 4;
            if self.pixel_bytes.saturating_add(bytes) > MAX_EMBEDDED_BYTES {
                return None;
            }
            self.pixel_bytes += bytes;
            let mut png = Vec::new();
            PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::NoFilter)
                .write_image(
                    pixels.as_raw(),
                    pixels.width(),
                    pixels.height(),
                    ExtendedColorType::Rgba8,
                )
                .ok()?;
            self.retained += png.len() as u64;
            Some(Arc::new(png))
        });
        self.skipped |= png.is_none();
        png
    }
}
