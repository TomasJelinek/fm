//! Image decoding belongs exclusively in the restricted provider process.
#![forbid(unsafe_code)]
use image::{DynamicImage, ImageDecoder, ImageReader, metadata::Cicp};
use midnight_thumbnail_sdk::{
    ReadSeek,
    api::{Frame, Request},
};
use std::io::{self, BufReader};

/// Decode one still image. Image-rs limits are backed by the OS worker's hard resource limits.
pub fn decode(request: &Request, stream: &mut dyn ReadSeek) -> io::Result<Frame> {
    midnight_thumbnail_sdk::api::rgba_length(request.width, request.height)?;
    if request.frame != 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Image provider exposes a still preview",
        ));
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(32768);
    limits.max_image_height = Some(32768);
    limits.max_alloc = Some(128 * 1024 * 1024);
    let mut reader = ImageReader::new(BufReader::new(stream)).with_guessed_format()?;
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(io::Error::other)?;
    if decoder.total_bytes() > 128 * 1024 * 1024 {
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
    let mut image = DynamicImage::from_decoder(decoder).map_err(io::Error::other)?;
    image.apply_orientation(orientation);
    if icc.is_none() {
        image
            .apply_color_space(Cicp::SRGB, Default::default())
            .map_err(io::Error::other)?;
    }
    let image = image
        .thumbnail(
            request.width.min(image.width()),
            request.height.min(image.height()),
        )
        .to_rgba8();
    let (width, height) = image.dimensions();
    let mut rgba = image.into_raw();
    if let Some(icc) = icc {
        let profile = moxcms::ColorProfile::new_from_slice(&icc).map_err(io::Error::other)?;
        // RGB layouts preserve alpha. Unsupported profile models fail rather than mislabel colors.
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
        let mut srgb = vec![0; rgba.len()];
        transform
            .transform(&rgba, &mut srgb)
            .map_err(io::Error::other)?;
        rgba = srgb;
    }
    Ok(Frame {
        id: request.id,
        width,
        height,
        frame: request.frame,
        rgba,
    })
}
