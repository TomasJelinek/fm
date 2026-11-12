//! Paginated retained-document service over the same opaque broker and native sandbox.
use crate::{Broker, ReadSeek, api};
use std::io::{self, Read, Write};

/// Provider-owned parsed document. Rendering cannot access the original source or external assets.
pub trait Document {
    fn metadata(&self) -> api::DocumentMetadata;
    fn render(&self, page: u32, viewport: api::Viewport) -> io::Result<api::Frame>;
}

/// Enter the mandatory sandbox, parse an opaque source once, then serve bounded page viewports.
pub fn serve_document<D: Document>(
    decode: impl FnOnce(&mut dyn ReadSeek) -> io::Result<D>,
) -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    midnight_worker_sdk::enter(&mut input, &mut output)?;
    session(&mut input, &mut output, decode)
}

fn session<R: Read, W: Write, D: Document>(
    input: &mut R,
    output: &mut W,
    decode: impl FnOnce(&mut dyn ReadSeek) -> io::Result<D>,
) -> io::Result<()> {
    let api::Message::Open { source_length } = api::read_message(input)? else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Expected document open",
        ));
    };
    let document = match decode(&mut Broker::new(input, output, source_length)) {
        Ok(document) => document,
        Err(_) => return api::write_message(output, &api::Message::Failure),
    };
    let metadata = document.metadata();
    if metadata.validate().is_err() {
        return api::write_message(output, &api::Message::Failure);
    }
    let count = metadata.pages.len();
    api::write_message(output, &api::Message::DocumentMetadata(metadata))?;
    loop {
        let api::Message::PageViewport { id, page, viewport } = api::read_message(input)? else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Expected document viewport",
            ));
        };
        if page as usize >= count {
            return api::write_message(output, &api::Message::Failure);
        }
        let message = match document.render(page, viewport) {
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
