//! Image and PDF decoding belong exclusively in the restricted provider process.
#![forbid(unsafe_code)]
use midnight_thumbnail_sdk::{
    ReadSeek,
    api::{Frame, Request},
};
use std::io;

mod pdf;
mod signature;
mod source;

pub use pdf::MAX_SOURCE_BYTES as MAX_PDF_SOURCE_BYTES;

/// Decode one still image, or draw the first page of a PDF document. The content
/// decides which: no file name crosses the protocol. Image-rs limits and the PDF
/// source budget are backed by the OS worker's hard resource limits.
pub fn decode(request: &Request, stream: &mut dyn ReadSeek) -> io::Result<Frame> {
    midnight_thumbnail_sdk::api::rgba_length(request.width, request.height)?;
    if request.frame != 0 {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Image provider exposes a still preview",
        ));
    }
    let mut stream = source::Prefixed::new(stream, pdf::SNIFF_BYTES)?;
    let (width, height, rgba) = if pdf::sniff(stream.prefix()) {
        pdf::first_page(&mut stream, request.width, request.height)?
    } else {
        let image =
            midnight_image_decoder::decode(&mut stream, Some((request.width, request.height)))?;
        let (width, height) = image.dimensions();
        (width, height, image.into_raw())
    };
    Ok(Frame {
        id: request.id,
        width,
        height,
        frame: request.frame,
        rgba,
    })
}
