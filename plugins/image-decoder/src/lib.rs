//! Shared codecs for the independent image viewer and thumbnail workers.
//! Call only after the caller has entered its restricted process boundary.
#![forbid(unsafe_code)]

use image::{DynamicImage, ImageDecoder, ImageReader, RgbaImage, metadata::Cicp};
use std::io::{self, BufReader, Read, Seek};
mod avif;
mod svg;

/// Original dimensions and RGBA storage are bounded independently of requested thumbnails.
pub const MAX_DECODED_EDGE: u32 = 32768;
/// Maximum retained original RGBA bytes.
pub const MAX_DECODED_BYTES: u64 = 128 * 1024 * 1024;

fn validate_dimensions(width: u32, height: u32) -> io::Result<()> {
    if width == 0
        || height == 0
        || width > MAX_DECODED_EDGE
        || height > MAX_DECODED_EDGE
        || u64::from(width) * u64::from(height) * 4 > MAX_DECODED_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Image dimensions exceed budget",
        ));
    }
    Ok(())
}

/// Decode a first still frame to oriented straight-alpha sRGB RGBA8.
/// A thumbnail bound preserves aspect ratio and never enlarges the original.
/// SVG rasterizes directly at that size instead of allocating its full original raster.
pub fn decode<R: Read + Seek + ?Sized>(
    stream: &mut R,
    thumbnail: Option<(u32, u32)>,
) -> io::Result<RgbaImage> {
    if let Some((width, height)) = thumbnail {
        validate_dimensions(width, height)?;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODED_EDGE);
    limits.max_image_height = Some(MAX_DECODED_EDGE);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    // Each broker read crosses a process boundary and validates the captured
    // source. Batch decoder row reads without preloading the complete file.
    let buffered = BufReader::with_capacity(256 * 1024, stream);
    let mut reader = ImageReader::new(buffered).with_guessed_format()?;
    if reader.format().is_none() {
        return svg::decode(reader.into_inner(), thumbnail);
    }
    let mut decoder: Box<dyn ImageDecoder + '_> =
        if reader.format() == Some(image::ImageFormat::Avif) {
            let mut decoder = avif::decoder(reader.into_inner())?;
            decoder.set_limits(limits).map_err(io::Error::other)?;
            Box::new(decoder)
        } else {
            reader.limits(limits);
            Box::new(reader.into_decoder().map_err(io::Error::other)?)
        };
    let (width, height) = decoder.dimensions();
    validate_dimensions(width, height)?;
    if decoder.total_bytes() > MAX_DECODED_BYTES {
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
    if let Some((width, height)) = thumbnail {
        decoded = decoded.thumbnail(width.min(decoded.width()), height.min(decoded.height()));
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
    Ok(pixels)
}
