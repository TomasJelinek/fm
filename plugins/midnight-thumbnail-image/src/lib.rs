//! Image decoding belongs exclusively in the restricted provider process.
#![forbid(unsafe_code)]
use midnight_thumbnail_sdk::{
    ReadSeek,
    api::{Frame, Request},
};
use std::io;

/// Decode one still image. Image-rs limits are backed by the OS worker's hard resource limits.
pub fn decode(request: &Request, stream: &mut dyn ReadSeek) -> io::Result<Frame> {
    midnight_thumbnail_sdk::api::rgba_length(request.width, request.height)?;
    if request.frame != 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Image provider exposes a still preview",
        ));
    }
    let image = midnight_image_decoder::decode(stream, Some((request.width, request.height)))?;
    let (width, height) = image.dimensions();
    let rgba = image.into_raw();
    Ok(Frame {
        id: request.id,
        width,
        height,
        frame: request.frame,
        rgba,
    })
}
