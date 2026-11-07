//! Retained image pixels and viewport rendering in the restricted provider process.
#![forbid(unsafe_code)]

use image::{DynamicImage, ImageDecoder, ImageReader, RgbaImage, metadata::Cicp};
use midnight_image_viewer_sdk::{
    Image, ReadSeek,
    api::{self, Frame, Metadata, Viewport},
};
use std::io::{self, BufReader};

mod sampling;

/// A single oriented, color-managed original image, owned entirely by the restricted process.
pub struct DecodedImage {
    pixels: RgbaImage,
}

/// Decode the first still frame using exactly the codecs and color engine used for thumbnails.
/// Input allocation, decoded dimensions, profile size and final RGBA storage are independently bounded.
pub fn decode(stream: &mut dyn ReadSeek) -> io::Result<DecodedImage> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(api::MAX_DECODED_EDGE);
    limits.max_image_height = Some(api::MAX_DECODED_EDGE);
    limits.max_alloc = Some(api::MAX_DECODED_BYTES);
    let mut reader = ImageReader::new(BufReader::new(stream)).with_guessed_format()?;
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(io::Error::other)?;
    let (width, height) = decoder.dimensions();
    Metadata { width, height }.validate()?;
    if decoder.total_bytes() > api::MAX_DECODED_BYTES {
        return Err(io::Error::other("Decoded image exceeds allocation budget"));
    }
    let orientation = decoder.orientation().map_err(io::Error::other)?;
    let icc = decoder.icc_profile().map_err(io::Error::other)?;
    if icc
        .as_ref()
        .is_some_and(|profile| profile.len() > 4 * 1024 * 1024)
    {
        return Err(io::Error::other("Embedded color profile exceeds budget"));
    }
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(io::Error::other)?;
    decoded.apply_orientation(orientation);
    if icc.is_none() {
        decoded
            .apply_color_space(Cicp::SRGB, Default::default())
            .map_err(io::Error::other)?;
    }
    let mut pixels = decoded.into_rgba8();
    if let Some(icc) = icc {
        let profile = moxcms::ColorProfile::new_from_slice(&icc).map_err(io::Error::other)?;
        if profile.color_space != moxcms::DataColorSpace::Rgb {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Unsupported ICC color model",
            ));
        }
        let transform = profile
            .create_transform_8bit(
                moxcms::Layout::Rgba,
                &moxcms::ColorProfile::new_srgb(),
                moxcms::Layout::Rgba,
                Default::default(),
            )
            .map_err(io::Error::other)?;
        // A row-sized conversion buffer avoids retaining a second full decoded image.
        let stride = pixels.width() as usize * 4;
        let mut row = vec![0; stride];
        for bytes in pixels.as_mut().chunks_exact_mut(stride) {
            transform
                .transform(bytes, &mut row)
                .map_err(io::Error::other)?;
            bytes.copy_from_slice(&row);
        }
    }
    Ok(DecodedImage { pixels })
}

impl Image for DecodedImage {
    fn metadata(&self) -> Metadata {
        Metadata {
            width: self.pixels.width(),
            height: self.pixels.height(),
        }
    }

    fn render(&self, viewport: Viewport) -> io::Result<Frame> {
        sampling::render(&self.pixels, viewport)
    }
}
