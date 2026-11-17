//! Public provider-side broker stream. Plugins depend only on public APIs and their decoder.
#![forbid(unsafe_code)]
pub use midnight_thumbnail_api as api;
use std::io::{self, Read, Seek, SeekFrom, Write};

pub trait ReadSeek: Read + Seek {}
impl<T: Read + Seek> ReadSeek for T {}

pub struct Broker<'a, R, W> {
    input: &'a mut R,
    output: &'a mut W,
    length: u64,
    position: u64,
}

impl<'a, R: Read, W: Write> Broker<'a, R, W> {
    pub fn new(input: &'a mut R, output: &'a mut W, length: u64) -> Self {
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
                "Invalid broker read reply",
            ));
        };
        if data.is_empty() || data.len() > length {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Invalid broker read length",
            ));
        }
        bytes[..data.len()].copy_from_slice(&data);
        self.position += data.len() as u64;
        Ok(data.len())
    }
}
impl<R: Read, W: Write> Seek for Broker<'_, R, W> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(delta) => self.length.checked_add_signed(delta),
        }
        .filter(|position| *position <= self.length)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Seek outside captured source")
        })?;
        self.position = position;
        Ok(position)
    }
}

pub fn serve(
    decode: impl FnOnce(&api::Request, &mut dyn ReadSeek) -> io::Result<api::Frame>,
) -> io::Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    midnight_worker_sdk::enter(&mut input, &mut output)?;
    let api::Message::Request(request) = api::read_message(&mut input)? else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Expected thumbnail request",
        ));
    };
    let result = decode(
        &request,
        &mut Broker::new(&mut input, &mut output, request.source_length),
    );
    let message = match result {
        Ok(frame)
            if frame.id == request.id
                && frame.frame == request.frame
                && frame.width <= request.width
                && frame.height <= request.height =>
        {
            api::Message::Frame(frame)
        }
        _ => api::Message::Failed,
    };
    api::write_message(&mut output, &message)
}
