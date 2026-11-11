use crate::*;
use std::io::{Cursor, Read, Write};

pub fn write_message(writer: &mut impl Write, message: &Message) -> io::Result<()> {
    write_message_with_protocol(writer, Protocol::V1, message)
}

pub fn write_message_with_protocol(
    writer: &mut impl Write,
    protocol: Protocol,
    message: &Message,
) -> io::Result<()> {
    if protocol.supports_staging() {
        streaming::validate(message)?;
    } else if streaming::tag(message).is_some() {
        return Err(invalid());
    }
    let mut data = Vec::new();
    data.extend((protocol as u16).to_le_bytes());
    let tag: u8 = match message {
        Message::Open { .. } => 1,
        Message::Opened { .. } => 2,
        Message::List { .. } => 3,
        Message::Entries { .. } => 4,
        Message::Read { .. } => 5,
        Message::Content { .. } => 6,
        Message::SourceRead { .. } => 7,
        Message::SourceData { .. } => 8,
        Message::Failed { .. } => 9,
        Message::Close { .. } => 10,
        other => streaming::tag(other).ok_or_else(invalid)?,
    };
    data.push(tag);
    put64(&mut data, message.request());
    match message {
        Message::Open { format, length, .. } => {
            data.push(*format as u8);
            put64(&mut data, *length);
        }
        Message::Opened { root, .. } => put64(&mut data, *root),
        Message::List {
            directory, after, ..
        } => {
            put64(&mut data, *directory);
            put64(&mut data, *after);
        }
        Message::Entries { entries, more, .. } => {
            if entries.len() > MAX_BATCH {
                return Err(invalid());
            }
            data.extend((entries.len() as u32).to_le_bytes());
            for node in entries {
                put64(&mut data, node.id);
                put64(&mut data, node.parent);
                bytes(&mut data, &node.name, MAX_NAME)?;
                bytes(&mut data, node.display.as_bytes(), MAX_NAME)?;
                data.push(node.kind as u8);
                data.push(u8::from(node.size.is_some()));
                if let Some(size) = node.size {
                    put64(&mut data, size);
                }
                data.push(u8::from(node.readable));
                if protocol.supports_staging() {
                    timestamp::write(&mut data, node.modified)?;
                }
            }
            data.push(u8::from(*more));
        }
        Message::Read {
            entry,
            offset,
            length,
            ..
        } => {
            put64(&mut data, *entry);
            put64(&mut data, *offset);
            read_len(&mut data, *length)?;
        }
        Message::Content {
            bytes: value, eof, ..
        } => {
            bytes(&mut data, value, MAX_READ as usize)?;
            data.push(u8::from(*eof));
        }
        Message::SourceRead { offset, length, .. } => {
            put64(&mut data, *offset);
            read_len(&mut data, *length)?;
        }
        Message::SourceData { bytes: value, .. } => bytes(&mut data, value, MAX_READ as usize)?,
        Message::Failed { code, message, .. } => {
            data.push(*code as u8);
            bytes(&mut data, message.as_bytes(), 4096)?;
        }
        Message::Close { .. } => {}
        other => streaming::write(&mut data, other)?,
    }
    if data.len() > MAX_FRAME {
        return Err(invalid());
    }
    writer.write_all(&(data.len() as u32).to_le_bytes())?;
    writer.write_all(&data)?;
    writer.flush()
}

pub fn read_message(reader: &mut impl Read) -> io::Result<Message> {
    read_message_with_protocol(reader, Protocol::V1)
}

pub fn read_message_with_protocol(
    reader: &mut impl Read,
    protocol: Protocol,
) -> io::Result<Message> {
    let length = u32le(reader)? as usize;
    if length > MAX_FRAME {
        return Err(invalid());
    }
    let mut bytes = vec![0; length];
    reader.read_exact(&mut bytes)?;
    let mut data = Cursor::new(bytes.as_slice());
    let mut version = [0; 2];
    data.read_exact(&mut version)?;
    if u16::from_le_bytes(version) != protocol as u16 {
        return Err(invalid());
    }
    let tag = byte(&mut data)?;
    let request = u64le(&mut data)?;
    let message = match tag {
        1 => Message::Open {
            request,
            format: match byte(&mut data)? {
                1 => Format::Zip,
                2 => Format::Tar,
                3 => Format::Gzip,
                4 => Format::TarGzip,
                5 => Format::SevenZip,
                6 => Format::Xz,
                7 => Format::TarXz,
                8 => Format::Bzip2,
                9 => Format::TarBzip2,
                10 => Format::Zstd,
                11 => Format::TarZstd,
                _ => return Err(invalid()),
            },
            length: u64le(&mut data)?,
        },
        2 => Message::Opened {
            request,
            root: u64le(&mut data)?,
        },
        3 => Message::List {
            request,
            directory: u64le(&mut data)?,
            after: u64le(&mut data)?,
        },
        4 => {
            let count = u32le(&mut data)? as usize;
            if count > MAX_BATCH {
                return Err(invalid());
            }
            let mut entries = Vec::with_capacity(count);
            for _ in 0..count {
                entries.push(Node {
                    id: u64le(&mut data)?,
                    parent: u64le(&mut data)?,
                    name: blob(&mut data, MAX_NAME)?,
                    display: string(&mut data, MAX_NAME)?,
                    kind: match byte(&mut data)? {
                        1 => Kind::Directory,
                        2 => Kind::File,
                        3 => Kind::Link,
                        4 => Kind::Other,
                        _ => return Err(invalid()),
                    },
                    size: if boolean(&mut data)? {
                        Some(u64le(&mut data)?)
                    } else {
                        None
                    },
                    readable: boolean(&mut data)?,
                    modified: if protocol.supports_staging() {
                        timestamp::read(&mut data)?
                    } else {
                        None
                    },
                });
            }
            Message::Entries {
                request,
                entries,
                more: boolean(&mut data)?,
            }
        }
        5 => Message::Read {
            request,
            entry: u64le(&mut data)?,
            offset: u64le(&mut data)?,
            length: bounded_len(&mut data)?,
        },
        6 => Message::Content {
            request,
            bytes: blob(&mut data, MAX_READ as usize)?,
            eof: boolean(&mut data)?,
        },
        7 => Message::SourceRead {
            request,
            offset: u64le(&mut data)?,
            length: bounded_len(&mut data)?,
        },
        8 => Message::SourceData {
            request,
            bytes: blob(&mut data, MAX_READ as usize)?,
        },
        9 => Message::Failed {
            request,
            code: match byte(&mut data)? {
                1 => ErrorCode::Invalid,
                2 => ErrorCode::Unsupported,
                3 => ErrorCode::Limit,
                4 => ErrorCode::Cancelled,
                5 => ErrorCode::SourceChanged,
                6 => ErrorCode::NotFound,
                7 => ErrorCode::ReadOnly,
                8 => ErrorCode::Corrupt,
                9 => ErrorCode::Io,
                _ => return Err(invalid()),
            },
            message: string(&mut data, 4096)?,
        },
        10 => Message::Close { request },
        11..=21 if protocol.supports_staging() => streaming::read(tag, request, &mut data)?,
        _ => return Err(invalid()),
    };
    if data.position() != length as u64 {
        return Err(invalid());
    }
    if protocol.supports_staging() {
        streaming::validate(&message)?;
    }
    Ok(message)
}

pub(crate) fn put64(out: &mut Vec<u8>, n: u64) {
    out.extend(n.to_le_bytes());
}
pub(crate) fn read_len(out: &mut Vec<u8>, n: u32) -> io::Result<()> {
    if n == 0 || n > MAX_READ {
        return Err(invalid());
    }
    out.extend(n.to_le_bytes());
    Ok(())
}
pub(crate) fn bytes(out: &mut Vec<u8>, value: &[u8], limit: usize) -> io::Result<()> {
    if value.len() > limit
        || out
            .len()
            .checked_add(4)
            .and_then(|n| n.checked_add(value.len()))
            .is_none_or(|n| n > MAX_FRAME)
    {
        return Err(invalid());
    }
    out.extend((value.len() as u32).to_le_bytes());
    out.extend(value);
    Ok(())
}
pub(crate) fn byte(input: &mut impl Read) -> io::Result<u8> {
    let mut b = [0];
    input.read_exact(&mut b)?;
    Ok(b[0])
}
pub(crate) fn boolean(input: &mut impl Read) -> io::Result<bool> {
    match byte(input)? {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(invalid()),
    }
}
fn u32le(input: &mut impl Read) -> io::Result<u32> {
    let mut b = [0; 4];
    input.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
pub(crate) fn u64le(input: &mut impl Read) -> io::Result<u64> {
    let mut b = [0; 8];
    input.read_exact(&mut b)?;
    Ok(u64::from_le_bytes(b))
}
pub(crate) fn bounded_len(input: &mut impl Read) -> io::Result<u32> {
    let n = u32le(input)?;
    if n == 0 || n > MAX_READ {
        return Err(invalid());
    }
    Ok(n)
}
pub(crate) fn blob(input: &mut Cursor<&[u8]>, max: usize) -> io::Result<Vec<u8>> {
    let n = u32le(input)? as usize;
    if n > max
        || n > input
            .get_ref()
            .len()
            .saturating_sub(input.position() as usize)
    {
        return Err(invalid());
    }
    let mut data = vec![0; n];
    input.read_exact(&mut data)?;
    Ok(data)
}
fn string(input: &mut Cursor<&[u8]>, max: usize) -> io::Result<String> {
    String::from_utf8(blob(input, max)?).map_err(|_| invalid())
}
