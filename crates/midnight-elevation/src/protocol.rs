//! A single bounded request and response. Names retain native code units, never display text.
use std::io::{self, Cursor, Read, Write};

pub(crate) const MAX_FRAME: usize = 64 * 1024 * 1024;
pub(crate) const MAX_ENTRIES: usize = 250_000;
const MAX_PATH: usize = 256 * 1024;
const MAX_ERROR: usize = 16 * 1024;
const MAGIC: &[u8; 8] = b"MELIST01";

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Request {
    pub path: Vec<u8>,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WireEntry {
    pub name: Vec<u8>,
    pub kind: u8,
    pub size: Option<u64>,
    pub modified: Option<(i64, u32)>,
    pub native_attributes: Option<u32>,
    pub unix_mode: Option<u32>,
}
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    pub denied: bool,
    pub message: String,
}
pub(crate) type Reply = Result<Vec<WireEntry>, Failure>;

pub(crate) fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn bytes(out: &mut Vec<u8>, value: &[u8], limit: usize) -> io::Result<()> {
    if value.len() > limit || out.len().saturating_add(value.len() + 4) > MAX_FRAME {
        return Err(invalid("Elevated listing exceeds its protocol limit"));
    }
    out.extend_from_slice(&(value.len() as u32).to_le_bytes());
    out.extend_from_slice(value);
    Ok(())
}
fn read_u32(input: &mut impl Read) -> io::Result<u32> {
    let mut value = [0; 4];
    input.read_exact(&mut value)?;
    Ok(u32::from_le_bytes(value))
}
fn read_bytes(input: &mut Cursor<Vec<u8>>, limit: usize) -> io::Result<Vec<u8>> {
    let length = read_u32(input)? as usize;
    let remaining = input
        .get_ref()
        .len()
        .saturating_sub(input.position() as usize);
    if length > limit || length > remaining {
        return Err(invalid("Invalid elevated listing field length"));
    }
    let mut value = vec![0; length];
    input.read_exact(&mut value)?;
    Ok(value)
}
fn frame(out: &mut impl Write, payload: &[u8]) -> io::Result<()> {
    if payload.len() > MAX_FRAME {
        return Err(invalid("Elevated listing exceeds 64 MiB"));
    }
    out.write_all(MAGIC)?;
    out.write_all(&(payload.len() as u32).to_le_bytes())?;
    out.write_all(payload)?;
    out.flush()
}
fn read_frame(input: &mut impl Read) -> io::Result<Cursor<Vec<u8>>> {
    let mut magic = [0; 8];
    input.read_exact(&mut magic)?;
    if &magic != MAGIC {
        return Err(invalid("Unknown elevated listing protocol"));
    }
    let length = read_u32(input)? as usize;
    if length > MAX_FRAME {
        return Err(invalid("Elevated listing exceeds 64 MiB"));
    }
    let mut data = vec![0; length];
    input.read_exact(&mut data)?;
    Ok(Cursor::new(data))
}
fn done(input: &Cursor<Vec<u8>>) -> io::Result<()> {
    if input.position() as usize != input.get_ref().len() {
        return Err(invalid("Trailing elevated listing data"));
    }
    Ok(())
}
fn byte(input: &mut impl Read) -> io::Result<u8> {
    let mut value = [0];
    input.read_exact(&mut value)?;
    Ok(value[0])
}
fn optional_u64(out: &mut Vec<u8>, value: Option<u64>) {
    out.push(u8::from(value.is_some()));
    if let Some(value) = value {
        out.extend_from_slice(&value.to_le_bytes());
    }
}
fn read_optional_u64(input: &mut impl Read) -> io::Result<Option<u64>> {
    match byte(input)? {
        0 => Ok(None),
        1 => {
            let mut value = [0; 8];
            input.read_exact(&mut value)?;
            Ok(Some(u64::from_le_bytes(value)))
        }
        _ => Err(invalid("Invalid elevated listing option")),
    }
}
pub(crate) fn write_request(out: &mut impl Write, request: &Request) -> io::Result<()> {
    let mut data = Vec::new();
    bytes(&mut data, &request.path, MAX_PATH)?;
    frame(out, &data)
}
pub(crate) fn read_request(input: &mut impl Read) -> io::Result<Request> {
    let mut data = read_frame(input)?;
    let path = read_bytes(&mut data, MAX_PATH)?;
    done(&data)?;
    Ok(Request { path })
}
pub(crate) fn write_reply(out: &mut impl Write, reply: &Reply) -> io::Result<()> {
    let mut data = Vec::new();
    match reply {
        Err(error) => {
            data.push(0);
            data.push(u8::from(error.denied));
            bytes(&mut data, error.message.as_bytes(), MAX_ERROR)?;
        }
        Ok(entries) => {
            if entries.len() > MAX_ENTRIES {
                return Err(invalid("Too many directory entries"));
            }
            data.push(1);
            data.extend_from_slice(&(entries.len() as u32).to_le_bytes());
            for entry in entries {
                bytes(&mut data, &entry.name, MAX_PATH)?;
                data.push(entry.kind);
                optional_u64(&mut data, entry.size);
                optional_u64(&mut data, entry.modified.map(|v| v.0 as u64));
                if let Some((_, nanos)) = entry.modified {
                    data.extend_from_slice(&nanos.to_le_bytes());
                }
                optional_u64(&mut data, entry.native_attributes.map(u64::from));
                optional_u64(&mut data, entry.unix_mode.map(u64::from));
            }
        }
    }
    frame(out, &data)
}
pub(crate) fn read_reply(input: &mut impl Read) -> io::Result<Reply> {
    let mut data = read_frame(input)?;
    let reply = match byte(&mut data)? {
        0 => {
            let denied = match byte(&mut data)? {
                0 => false,
                1 => true,
                _ => return Err(invalid("Invalid error kind")),
            };
            let message = String::from_utf8(read_bytes(&mut data, MAX_ERROR)?)
                .map_err(|_| invalid("Invalid error text"))?;
            Err(Failure { denied, message })
        }
        1 => {
            let length = read_u32(&mut data)? as usize;
            if length > MAX_ENTRIES {
                return Err(invalid("Too many directory entries"));
            }
            let mut entries = Vec::with_capacity(length);
            for _ in 0..length {
                let name = read_bytes(&mut data, MAX_PATH)?;
                let kind = byte(&mut data)?;
                if kind > 4 {
                    return Err(invalid("Unknown entry kind"));
                }
                let size = read_optional_u64(&mut data)?;
                let modified = read_optional_u64(&mut data)?
                    .map(|seconds| {
                        let nanos = read_u32(&mut data)?;
                        if nanos >= 1_000_000_000 {
                            return Err(invalid("Invalid timestamp"));
                        }
                        Ok((seconds as i64, nanos))
                    })
                    .transpose()?;
                let native_attributes = read_optional_u64(&mut data)?
                    .map(u32::try_from)
                    .transpose()
                    .map_err(|_| invalid("Invalid native attributes"))?;
                let unix_mode = read_optional_u64(&mut data)?
                    .map(u32::try_from)
                    .transpose()
                    .map_err(|_| invalid("Invalid Unix mode"))?;
                entries.push(WireEntry {
                    name,
                    kind,
                    size,
                    modified,
                    native_attributes,
                    unix_mode,
                });
            }
            Ok(entries)
        }
        _ => return Err(invalid("Invalid listing response")),
    };
    done(&data)?;
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arbitrary_native_code_units_round_trip() {
        let request = Request {
            path: vec![0xff, 0, 0xd8, 1],
        };
        let mut wire = Vec::new();
        write_request(&mut wire, &request).unwrap();
        assert_eq!(read_request(&mut wire.as_slice()).unwrap(), request);
        let reply = Ok(vec![WireEntry {
            name: vec![0xff, 0x80],
            kind: 2,
            size: Some(u64::MAX),
            modified: Some((-1, 123)),
            native_attributes: Some(2),
            unix_mode: Some(0o100600),
        }]);
        wire.clear();
        write_reply(&mut wire, &reply).unwrap();
        assert_eq!(read_reply(&mut wire.as_slice()).unwrap(), reply);
    }
    #[test]
    fn oversized_and_truncated_frames_are_rejected() {
        let mut wire = MAGIC.to_vec();
        wire.extend_from_slice(&((MAX_FRAME + 1) as u32).to_le_bytes());
        assert!(read_request(&mut wire.as_slice()).is_err());
        wire.clear();
        write_request(&mut wire, &Request { path: vec![1, 2] }).unwrap();
        wire.pop();
        assert!(read_request(&mut wire.as_slice()).is_err());
    }
}
