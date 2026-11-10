use crate::fields::{Decoder, Encoder};
use crate::validation::{check, validate_raw};
use crate::*;
use std::io::{self, Read, Write};

/// Read one MWV1 message. Prefix, control lengths and geometry are checked before raw allocation.
/// A protocol error is terminal for the stream; callers must not try to resynchronize it.
pub fn read_message(reader: &mut impl Read) -> io::Result<Message> {
    let mut prefix = [0; 16];
    reader.read_exact(&mut prefix)?;
    check(&prefix[..4] == b"MWV1")?;
    let mut b = Decoder(&prefix[4..]);
    check(b.array::<2>()? == PROTOCOL_VERSION.to_le_bytes())?;
    let direction = b.u8()?;
    let tag = b.u8()?;
    let control_len = b.u32()? as usize;
    let raw_len = b.u32()? as usize;
    check((24..=MAX_CONTROL_BYTES).contains(&control_len))?;
    check(match direction {
        0 if (1..=15).contains(&tag) => {
            if tag == 13 {
                raw_len <= MAX_READ_BYTES
            } else {
                raw_len == 0
            }
        }
        1 if (1..=11).contains(&tag) => {
            if tag == 2 {
                raw_len <= MAX_FRAME_BYTES
            } else {
                raw_len == 0
            }
        }
        _ => false,
    })?;
    let mut control = vec![0; control_len];
    reader.read_exact(&mut control)?;
    let mut b = Decoder(&control);
    let header = b.header()?;
    let mut message = if direction == 0 {
        Message::Command(Command {
            header,
            kind: crate::decode::command(&mut b, tag)?,
        })
    } else {
        Message::Event(Event {
            header,
            kind: crate::decode::event(&mut b, tag)?,
        })
    };
    check(b.0.is_empty())?;
    validate_raw(&message, raw_len)?;
    if raw_len != 0 {
        let mut raw = vec![0; raw_len];
        reader.read_exact(&mut raw)?;
        match &mut message {
            Message::Command(Command {
                kind: CommandKind::ResourceReply(ResourceReply::Data { bytes, .. }),
                ..
            }) => *bytes = raw,
            Message::Event(Event {
                kind: EventKind::Frame(frame),
                ..
            }) => frame.rgba = raw,
            _ => unreachable!("raw data validated before allocation"),
        }
    }
    Ok(message)
}

/// Validate, write and flush one message. Raw data is borrowed, never copied into control storage.
/// Validation errors write no bytes; I/O errors may leave a partial message and are terminal.
pub fn write_message(writer: &mut impl Write, message: &Message) -> io::Result<()> {
    let (direction, tag, control) = encode_control(message)?;
    writer.write_all(b"MWV1")?;
    writer.write_all(&PROTOCOL_VERSION.to_le_bytes())?;
    writer.write_all(&[direction, tag])?;
    writer.write_all(&(control.len() as u32).to_le_bytes())?;
    writer.write_all(&(message.raw().len() as u32).to_le_bytes())?;
    writer.write_all(&control)?;
    writer.write_all(message.raw())?;
    writer.flush()
}

pub(crate) fn encode_control(message: &Message) -> io::Result<(u8, u8, Vec<u8>)> {
    validate_raw(message, message.raw().len())?;
    let mut b = Encoder::new(message.header());
    let (direction, tag) = match message {
        Message::Command(command) => (0, crate::encode::command(&mut b, &command.kind)?),
        Message::Event(event) => (1, crate::encode::event(&mut b, &event.kind)?),
    };
    check(b.0.len() <= MAX_CONTROL_BYTES)?;
    Ok((direction, tag, b.0))
}
