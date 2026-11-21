//! Shared codecs for the independent image viewer and thumbnail workers.
//! Call only after the caller has entered its restricted process boundary.
#![forbid(unsafe_code)]

use image::{
    DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage, metadata::Orientation,
};
use moxcms::{ColorProfile, DataColorSpace, Layout};
use std::io::{self, BufRead, Cursor, Read, Seek};
mod avif;
mod svg;
mod window;

/// Original dimensions and RGBA storage are bounded independently of requested thumbnails.
pub(crate) const MAX_DECODED_EDGE: u32 = 32768;
/// Maximum retained original RGBA bytes.
pub(crate) const MAX_DECODED_BYTES: u64 = 128 * 1024 * 1024;

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

/// A decoded still image and what it leaves out.
#[derive(Debug)]
pub struct Decoded {
    /// Oriented straight-alpha sRGB RGBA8 pixels.
    pub pixels: RgbaImage,
    /// For SVG, one short sentence naming content that was skipped instead of drawn
    /// (scripts, HTML in `foreignObject`, linked files, undecodable embedded images), for
    /// a status line: at most 160 printable ASCII characters without `/` or `\`. `None`
    /// when nothing was skipped, and for raster formats.
    pub note: Option<String>,
}

/// Decode a first still frame to oriented straight-alpha sRGB RGBA8.
/// A thumbnail bound preserves aspect ratio and never enlarges the original.
/// SVG rasterizes directly at that size instead of allocating its full original raster.
pub fn decode<R: Read + Seek + ?Sized>(
    stream: &mut R,
    thumbnail: Option<(u32, u32)>,
) -> io::Result<RgbaImage> {
    decode_with_note(stream, thumbnail).map(|decoded| decoded.pixels)
}

/// [`decode`], also returning the note about skipped SVG content.
///
/// Errors: `InvalidData` for damaged or incomplete content, whose text continues the
/// sentence "This image is damaged or incomplete:"; for an SVG limit (`FileTooLarge`) or
/// unsupported construct (`Unsupported`), a complete sentence that starts with "This ".
pub fn decode_with_note<R: Read + Seek + ?Sized>(
    stream: &mut R,
    thumbnail: Option<(u32, u32)>,
) -> io::Result<Decoded> {
    if let Some((width, height)) = thumbnail {
        validate_dimensions(width, height)?;
    }
    // Each broker read crosses a process boundary and validates the captured
    // source. Batch decoder reads without preloading the complete file, and keep
    // fetched bytes across codec seeks.
    let buffered = window::Window::new(stream)?;
    let reader = ImageReader::new(buffered).with_guessed_format()?;
    if reader.format().is_none() {
        return svg::decode(reader.into_inner(), thumbnail);
    }
    Ok(Decoded {
        pixels: raster(reader, thumbnail)?,
        note: None,
    })
}

/// Decode an image that an SVG embeds as a `data:` URI: PNG, JPEG, GIF or WebP, with
/// every limit of a raster file. Never SVG, so embedding cannot recurse.
pub(crate) fn embedded_raster(bytes: &[u8]) -> io::Result<RgbaImage> {
    let reader = ImageReader::new(Cursor::new(bytes)).with_guessed_format()?;
    match reader.format() {
        Some(ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::Gif | ImageFormat::WebP) => {
            raster(reader, None)
        }
        _ => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Embedded image format is not supported",
        )),
    }
}

fn raster<R: BufRead + Seek>(
    mut reader: ImageReader<R>,
    thumbnail: Option<(u32, u32)>,
) -> io::Result<RgbaImage> {
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_DECODED_EDGE);
    limits.max_image_height = Some(MAX_DECODED_EDGE);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
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
    let profile = icc
        .map(|icc| ColorProfile::new_from_slice(&icc))
        .transpose()
        .map_err(io::Error::other)?;
    // CICP-only color signalling (AVIF nclx, PNG cICP) is not converted: image-rs
    // tags every decoded image as sRGB, so its pixels are used as sRGB as before.
    let mut decoded = DynamicImage::from_decoder(decoder).map_err(io::Error::other)?;
    let gray = !decoded.color().has_color();
    let conversion = profile
        .map(|profile| Conversion::new(profile, gray))
        .transpose()?
        .flatten();
    if let Some((width, height)) = thumbnail {
        // Reduce first so orientation only moves the small result.
        let (width, height) = if transposes(orientation) {
            (height, width)
        } else {
            (width, height)
        };
        decoded = reduce(decoded, width, height);
    }
    decoded.apply_orientation(orientation);
    let mut pixels = decoded.into_rgba8();
    if let Some(conversion) = conversion {
        conversion.apply(&mut pixels)?;
    }
    Ok(pixels)
}

fn transposes(orientation: Orientation) -> bool {
    matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    )
}

/// Reduce without enlarging. image-rs's box filter averages straight-alpha color
/// channels independently, which darkens or tints edges next to transparent
/// pixels; colors are therefore averaged with alpha weights.
fn reduce(image: DynamicImage, width: u32, height: u32) -> DynamicImage {
    if width >= image.width() && height >= image.height() {
        // Nothing to reduce: keep the decoded samples exactly (an icon smaller than
        // its cell must not pass through 8-bit premultiplication).
        return image;
    }
    let (width, height) = (width.min(image.width()), height.min(image.height()));
    if !image.color().has_alpha() {
        return image.thumbnail(width, height);
    }
    let mut pixels = image.into_rgba8();
    for pixel in pixels.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        for channel in &mut pixel.0[..3] {
            *channel = ((u32::from(*channel) * alpha + 127) / 255) as u8;
        }
    }
    let mut reduced = DynamicImage::ImageRgba8(pixels)
        .thumbnail(width, height)
        .into_rgba8();
    for pixel in reduced.pixels_mut() {
        let alpha = u32::from(pixel[3]);
        if alpha == 0 {
            pixel.0 = [0; 4];
            continue;
        }
        for channel in &mut pixel.0[..3] {
            *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
        }
    }
    DynamicImage::ImageRgba8(reduced)
}

/// Embedded ICC handling. RGB profiles, and gray profiles of gray images, are
/// converted to sRGB; anything else is rejected rather than claiming sRGB, except
/// CMYK (see `new`).
struct Conversion {
    transform: std::sync::Arc<moxcms::Transform8BitExecutor>,
    gray: bool,
}

impl Conversion {
    fn new(profile: ColorProfile, gray_image: bool) -> io::Result<Option<Self>> {
        let srgb = ColorProfile::new_srgb();
        let source = match profile.color_space {
            DataColorSpace::Rgb => Layout::Rgba,
            // A gray profile describes only gray samples.
            DataColorSpace::Gray if gray_image => Layout::GrayAlpha,
            // image-rs has already converted CMYK samples to RGB without a profile,
            // exactly as for CMYK files that carry none; the CMYK profile cannot
            // describe those RGB samples, so it is not applied.
            DataColorSpace::Cmyk => return Ok(None),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "Unsupported ICC color model",
                ));
            }
        };
        let transform = profile
            .create_transform_8bit(source, &srgb, Layout::Rgba, Default::default())
            .map_err(io::Error::other)?;
        Ok(Some(Self {
            transform,
            gray: source == Layout::GrayAlpha,
        }))
    }

    fn apply(&self, pixels: &mut RgbaImage) -> io::Result<()> {
        // Row-sized conversion buffers avoid retaining a second full image.
        let stride = pixels.width() as usize * 4;
        let mut row = vec![0; stride];
        let mut gray = Vec::new();
        for bytes in pixels.as_mut().chunks_exact_mut(stride) {
            let source = if self.gray {
                // Gray samples were replicated into R, G and B.
                gray.clear();
                gray.extend(
                    bytes
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .flat_map(|pixel| [pixel[0], pixel[3]]),
                );
                gray.as_slice()
            } else {
                &*bytes
            };
            self.transform
                .transform(source, &mut row)
                .map_err(io::Error::other)?;
            bytes.copy_from_slice(&row);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_profile_policy_by_model() {
        let profile = |space| {
            let mut profile = ColorProfile::new_srgb();
            profile.color_space = space;
            profile
        };
        assert!(
            Conversion::new(profile(DataColorSpace::Rgb), false)
                .unwrap()
                .is_some()
        );
        let gray = ColorProfile::new_gray_with_gamma(1.0);
        assert!(Conversion::new(gray.clone(), true).unwrap().is_some());
        assert_eq!(
            Conversion::new(gray, false).err().unwrap().kind(),
            io::ErrorKind::Unsupported
        );
        assert!(
            Conversion::new(profile(DataColorSpace::Cmyk), false)
                .unwrap()
                .is_none()
        );
        for space in [DataColorSpace::Lab, DataColorSpace::YCbr] {
            assert_eq!(
                Conversion::new(profile(space), false).err().unwrap().kind(),
                io::ErrorKind::Unsupported
            );
        }
    }

    #[test]
    fn alpha_weighted_reduction_keeps_edge_color_and_reduces_before_orientation() {
        // Opaque red beside transparent black: the plain box filter yields (128,0,0,128).
        let mut image = RgbaImage::new(4, 2);
        for (x, _, pixel) in image.enumerate_pixels_mut() {
            *pixel = if x < 2 {
                image::Rgba([255, 0, 0, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            };
        }
        let reduced = reduce(DynamicImage::ImageRgba8(image), 1, 1).into_rgba8();
        assert_eq!(reduced.get_pixel(0, 0).0, [255, 0, 0, 128]);
        assert!(transposes(Orientation::Rotate90));
        assert!(!transposes(Orientation::FlipVertical));
    }

    #[test]
    fn images_within_the_bound_keep_their_exact_samples() {
        // A faint pixel loses most of its color precision through 8-bit
        // premultiplication; an icon smaller than its cell must not be touched.
        let icon =
            DynamicImage::ImageRgba8(RgbaImage::from_pixel(16, 16, image::Rgba([201, 37, 99, 4])));
        assert_eq!(reduce(icon.clone(), 96, 96), icon);
        assert_eq!(reduce(icon.clone(), 16, 16), icon);
        let mut wide = RgbaImage::from_pixel(32, 16, image::Rgba([201, 37, 99, 4]));
        wide.put_pixel(0, 0, image::Rgba([0, 0, 0, 255]));
        let wide = DynamicImage::ImageRgba8(wide);
        // One bound below the image still reduces it.
        assert_eq!(reduce(wide.clone(), 16, 96).width(), 16);
        let gray = DynamicImage::ImageLumaA16(image::ImageBuffer::from_pixel(
            8,
            8,
            image::LumaA([40_000u16, 300]),
        ));
        assert_eq!(reduce(gray.clone(), 8, 8), gray);
    }
}
