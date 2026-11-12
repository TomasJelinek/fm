//! Public runtime for restricted image-viewer providers.
#![forbid(unsafe_code)]

pub use midnight_image_viewer_api as api;
use std::io::{self, Read, Seek, SeekFrom, Write};
mod document;
pub use document::{Document, serve_document};

/// An opaque seekable source. Its length is captured; it has no filesystem name or handle.
pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

/// Provider-owned retained pixels. Rendering must not access or decode the original source again.
pub trait Image {
    fn metadata(&self) -> api::Metadata;
    fn render(&self, viewport: api::Viewport) -> io::Result<api::Frame>;
}

struct Broker<'a, R, W> {
    input: &'a mut R,
    output: &'a mut W,
    length: u64,
    position: u64,
}

impl<'a, R: Read, W: Write> Broker<'a, R, W> {
    fn new(input: &'a mut R, output: &'a mut W, length: u64) -> Self {
        Self {
            input,
            output,
            length,
            position: 0,
        }
    }
}

impl<R: Read, W: Write> Read for Broker<'_, R, W> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let length = (self.length - self.position)
            .min(bytes.len() as u64)
            .min(api::MAX_READ_BYTES as u64) as usize;
        if length == 0 {
            return Ok(0);
        }
        api::write_message(
            self.output,
            &api::Message::Read {
                offset: self.position,
                length: length as u32,
            },
        )?;
        let api::Message::Data(data) = api::read_message(self.input)? else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid source-read reply",
            ));
        };
        if data.len() != length {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Captured source read changed length",
            ));
        }
        bytes[..length].copy_from_slice(&data);
        self.position += length as u64;
        Ok(length)
    }
}

impl<R: Read, W: Write> Seek for Broker<'_, R, W> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(delta) => self.length.checked_add_signed(delta),
        }
        .filter(|position| *position <= self.length)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Seek outside captured source")
        })?;
        Ok(self.position)
    }
}

/// Enter the required native sandbox, decode once, then render bounded viewports until EOF.
/// Sandbox startup failure is terminal and occurs before any untrusted content is parsed.
pub fn serve<I: Image>(decode: impl FnOnce(&mut dyn ReadSeek) -> io::Result<I>) -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    midnight_worker_sdk::enter(&mut input, &mut output)?;
    session(&mut input, &mut output, decode)
}

fn session<R: Read, W: Write, I: Image>(
    input: &mut R,
    output: &mut W,
    decode: impl FnOnce(&mut dyn ReadSeek) -> io::Result<I>,
) -> io::Result<()> {
    let api::Message::Open { source_length } = api::read_message(input)? else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Expected image open",
        ));
    };
    let image = match decode(&mut Broker::new(input, output, source_length)) {
        Ok(image) if image.metadata().validate().is_ok() => image,
        _ => return api::write_message(output, &api::Message::Failure),
    };
    api::write_message(output, &api::Message::Metadata(image.metadata()))?;
    loop {
        let api::Message::Viewport { id, viewport } = api::read_message(input)? else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Expected image viewport",
            ));
        };
        let message = match image.render(viewport) {
            Ok(frame) if frame.width == viewport.width && frame.height == viewport.height => {
                api::Message::Rgba { id, frame }
            }
            _ => return api::write_message(output, &api::Message::Failure),
        };
        api::write_message(output, &message)?;
    }
}

#[cfg(test)]
mod tests;
