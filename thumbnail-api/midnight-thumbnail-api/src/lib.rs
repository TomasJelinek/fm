//! Versioned, allocation-bounded thumbnail helper messages. No host paths cross this interface.
#![forbid(unsafe_code)]

use std::io::{self, Cursor, Read, Write};

/// Maximum physical output edge, independent of the input dimensions.
pub const MAX_EDGE: u32 = 512;
/// Maximum straight-alpha sRGB RGBA8 output allocation per frame.
pub const MAX_FRAME_BYTES: usize = (MAX_EDGE * MAX_EDGE * 4) as usize;
/// A broker read can never exceed this allocation.
pub const MAX_READ_BYTES: usize = 256 * 1024;
/// Frame zero is the static representative; frames one through five animate.
pub const MAX_ANIMATION_FRAMES: u8 = 6;

/// An opaque request over an already captured read-only source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub id: u64,
    pub source_length: u64,
    pub width: u32,
    pub height: u32,
    pub frame: u8,
}

/// Oriented, aspect-preserving straight-alpha sRGB RGBA8 pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub id: u64,
    pub width: u32,
    pub height: u32,
    pub frame: u8,
    pub rgba: Vec<u8>,
}

/// The complete v1 protocol. Strings, filesystem names and URLs are deliberately absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Request(Request),
    Read { offset: u64, length: u32 },
    Data(Vec<u8>),
    Frame(Frame),
    Failed,
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid thumbnail protocol message",
    )
}

/// Validate output dimensions before any allocation or presentation.
pub fn rgba_length(width: u32, height: u32) -> io::Result<usize> {
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE {
        return Err(invalid());
    }
    Ok((width * height * 4) as usize)
}

fn validate(message: &Message) -> io::Result<()> {
    match message {
        Message::Request(request) => {
            rgba_length(request.width, request.height)?;
            if request.frame >= MAX_ANIMATION_FRAMES {
                return Err(invalid());
            }
        }
        Message::Read { offset, length } => {
            if *length == 0
                || *length as usize > MAX_READ_BYTES
                || offset.checked_add(u64::from(*length)).is_none()
            {
                return Err(invalid());
            }
        }
        Message::Data(bytes) if bytes.len() > MAX_READ_BYTES => return Err(invalid()),
        Message::Frame(frame)
            if frame.frame >= MAX_ANIMATION_FRAMES
                || rgba_length(frame.width, frame.height)? != frame.rgba.len() =>
        {
            return Err(invalid());
        }
        _ => {}
    }
    Ok(())
}

/// Write and flush one bounded message. Invalid dimensions/lengths are rejected before writing.
pub fn write_message(writer: &mut impl Write, message: &Message) -> io::Result<()> {
    validate(message)?;
    let mut payload = Vec::new();
    match message {
        Message::Request(request) => {
            payload.push(1);
            payload.extend_from_slice(&request.id.to_le_bytes());
            payload.extend_from_slice(&request.source_length.to_le_bytes());
            payload.extend_from_slice(&request.width.to_le_bytes());
            payload.extend_from_slice(&request.height.to_le_bytes());
            payload.push(request.frame);
        }
        Message::Read { offset, length } => {
            payload.push(2);
            payload.extend_from_slice(&offset.to_le_bytes());
            payload.extend_from_slice(&length.to_le_bytes());
        }
        Message::Data(bytes) => {
            payload.push(3);
            payload.extend_from_slice(bytes);
        }
        Message::Frame(frame) => {
            payload.push(4);
            payload.extend_from_slice(&frame.id.to_le_bytes());
            payload.extend_from_slice(&frame.width.to_le_bytes());
            payload.extend_from_slice(&frame.height.to_le_bytes());
            payload.push(frame.frame);
            payload.extend_from_slice(&frame.rgba);
        }
        Message::Failed => payload.push(5),
    }
    writer.write_all(b"MTH1")?;
    writer.write_all(&(payload.len() as u32).to_le_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()
}

fn array<const N: usize>(reader: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Read one message with a strict allocation ceiling; reject unknown versions and trailing fields.
pub fn read_message(reader: &mut impl Read) -> io::Result<Message> {
    if &array::<4>(reader)? != b"MTH1" {
        return Err(invalid());
    }
    let length = u32::from_le_bytes(array(reader)?) as usize;
    if length == 0 || length > MAX_FRAME_BYTES + 18 {
        return Err(invalid());
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    let mut payload = Cursor::new(&bytes);
    let message = match array::<1>(&mut payload)?[0] {
        1 => Message::Request(Request {
            id: u64::from_le_bytes(array(&mut payload)?),
            source_length: u64::from_le_bytes(array(&mut payload)?),
            width: u32::from_le_bytes(array(&mut payload)?),
            height: u32::from_le_bytes(array(&mut payload)?),
            frame: array::<1>(&mut payload)?[0],
        }),
        2 => Message::Read {
            offset: u64::from_le_bytes(array(&mut payload)?),
            length: u32::from_le_bytes(array(&mut payload)?),
        },
        3 => {
            if length > MAX_READ_BYTES + 1 {
                return Err(invalid());
            }
            payload.set_position(length as u64);
            Message::Data(bytes[1..].to_vec())
        }
        4 => {
            let id = u64::from_le_bytes(array(&mut payload)?);
            let width = u32::from_le_bytes(array(&mut payload)?);
            let height = u32::from_le_bytes(array(&mut payload)?);
            let frame = array::<1>(&mut payload)?[0];
            if rgba_length(width, height)? + 18 != length {
                return Err(invalid());
            }
            payload.set_position(length as u64);
            Message::Frame(Frame {
                id,
                width,
                height,
                frame,
                rgba: bytes[18..].to_vec(),
            })
        }
        5 => Message::Failed,
        _ => return Err(invalid()),
    };
    if payload.position() != length as u64 {
        return Err(invalid());
    }
    validate(&message)?;
    Ok(message)
}
