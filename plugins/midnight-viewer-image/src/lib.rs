//! Retained image pixels and viewport rendering in the restricted provider process.
#![forbid(unsafe_code)]

use image::RgbaImage;
use midnight_image_viewer_sdk::{
    Image, ReadSeek,
    api::{Frame, Metadata, Viewport},
};
use std::io;

mod frame_cache;
mod sampling;

/// A single oriented, color-managed original image, owned entirely by the restricted process.
pub struct DecodedImage {
    pixels: RgbaImage,
    frames: frame_cache::FrameCache,
}

/// Decode the first still frame using exactly the codecs and color engine used for thumbnails.
/// Input allocation, decoded dimensions, profile size and final RGBA storage are independently bounded.
pub fn decode(stream: &mut dyn ReadSeek) -> io::Result<DecodedImage> {
    let pixels = midnight_image_decoder::decode(stream, None)?;
    Ok(DecodedImage {
        pixels,
        frames: Default::default(),
    })
}

impl Image for DecodedImage {
    fn metadata(&self) -> Metadata {
        Metadata {
            width: self.pixels.width(),
            height: self.pixels.height(),
        }
    }

    fn render(&self, viewport: Viewport) -> io::Result<Frame> {
        self.frames
            .render(viewport, || sampling::render(&self.pixels, viewport))
    }
}
