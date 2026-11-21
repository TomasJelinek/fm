//! Proposed worker-private MMV2 pipe framing, independent of the public plugin ABI.
//! Any framing/transport error ends the stream; this codec never resynchronizes.
use crate::{
    media::{AudioTrack, MAX_PCM_FRAMES},
    playback_session::{self, Command, Completion, Event, Request},
};
use std::{
    io::{self, Read, Write},
    num::NonZeroU64,
};

pub const MAX_READ_BYTES: usize = 256 * 1024;
const HEADER_BYTES: usize = 12;

pub enum Incoming {
    Open {
        source_length: u64,
        generation: NonZeroU64,
        width: u32,
        height: u32,
    },
    Session(Request),
    ReadReply {
        read_request: NonZeroU64,
        result: io::Result<Vec<u8>>,
    },
}

pub enum Outgoing<'a> {
    Opened {
        generation: NonZeroU64,
        duration_micros: Option<u64>,
        width: u32,
        height: u32,
        audio: AudioTrack,
    },
    Read {
        read_request: NonZeroU64,
        offset: u64,
        length: u32,
    },
    Event(&'a Event),
    /// `codec` and `tried` describe cause 5 (`NoDecoder`) and are zero for other causes.
    Failed {
        request: u64,
        generation: u64,
        cause: u8,
        codec: u8,
        tried: u8,
    },
}

fn malformed() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "Invalid MMV2 pipe message")
}

fn invalid_output() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "Invalid MMV2 output message")
}

fn field<const N: usize>(payload: &[u8], start: usize) -> io::Result<[u8; N]> {
    let end = start.checked_add(N).ok_or_else(malformed)?;
    payload
        .get(start..end)
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(malformed)
}

fn tag(payload: &[u8], start: usize) -> io::Result<NonZeroU64> {
    NonZeroU64::new(u64::from_le_bytes(field(payload, start)?)).ok_or_else(malformed)
}

/// None means clean EOF before any header byte. Partial headers/payloads are
/// UnexpectedEof. Only a successful ReadReply may allocate, after its announced
/// length, tag and result are validated; empty success represents source EOF.
/// Session ordering and the decoder's native source/seek range remain separate.
pub fn read_message(input: &mut impl Read) -> io::Result<Option<Incoming>> {
    let mut header = [0; HEADER_BYTES];
    loop {
        match input.read(&mut header[..1]) {
            Ok(0) => return Ok(None),
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    input.read_exact(&mut header[1..])?;
    if &header[..4] != b"MMV2" || header[5..8] != [0; 3] {
        return Err(malformed());
    }
    let kind = header[4];
    let length = u32::from_le_bytes(field(&header, 8)?) as usize;
    let valid_length = match kind {
        1 | 3 => length == 24,
        2 => length == 19,
        4 => length == 8,
        5 => (9..=9 + MAX_READ_BYTES).contains(&length),
        _ => false,
    };
    if !valid_length {
        return Err(malformed());
    }
    if kind == 5 {
        let mut prefix = [0; 9];
        input.read_exact(&mut prefix)?;
        let read_request = tag(&prefix, 0)?;
        let result = match prefix[8] {
            0 => {
                let mut bytes = vec![0; length - prefix.len()];
                input.read_exact(&mut bytes)?;
                Ok(bytes)
            }
            code @ 1..=3 if length == prefix.len() => {
                let kind = match code {
                    1 => io::ErrorKind::PermissionDenied,
                    2 => io::ErrorKind::Interrupted,
                    _ => io::ErrorKind::Other,
                };
                Err(io::Error::new(kind, "Captured media read failed"))
            }
            _ => return Err(malformed()),
        };
        return Ok(Some(Incoming::ReadReply {
            read_request,
            result,
        }));
    }
    let mut fields = [0; 24];
    input.read_exact(&mut fields[..length])?;
    let message = match kind {
        1 => {
            let source_length = u64::from_le_bytes(field(&fields, 0)?);
            let generation = tag(&fields, 8)?;
            let width = u32::from_le_bytes(field(&fields, 16)?);
            let height = u32::from_le_bytes(field(&fields, 20)?);
            crate::media::output_length(width, height).map_err(|_| malformed())?;
            Incoming::Open {
                source_length,
                generation,
                width,
                height,
            }
        }
        2 => {
            let id = tag(&fields, 0)?;
            let generation = tag(&fields, 8)?;
            let frames = fields[16];
            let pcm_frames = u16::from_le_bytes(field(&fields, 17)?);
            if (frames == 0 && pcm_frames == 0)
                || frames > playback_session::MAX_PULL_FRAMES
                || pcm_frames > MAX_PCM_FRAMES
            {
                return Err(malformed());
            }
            Incoming::Session(Request {
                id,
                command: Command::Pull {
                    generation,
                    frames,
                    pcm_frames,
                },
            })
        }
        3 => Incoming::Session(Request {
            id: tag(&fields, 0)?,
            command: Command::Seek {
                generation: tag(&fields, 8)?,
                position_micros: u64::from_le_bytes(field(&fields, 16)?),
            },
        }),
        4 => Incoming::Session(Request {
            id: tag(&fields, 0)?,
            command: Command::Close,
        }),
        _ => return Err(malformed()),
    };
    Ok(Some(message))
}

/// Validate before writing any header, then write borrowed pixels directly.
/// A final flush publishes broker reads and terminals before the worker waits;
/// a transport error may leave a partial frame and must terminate the stream.
/// Failed cause bytes are the worker entry's private classification, not strings.
pub fn write_message(output: &mut impl Write, message: Outgoing<'_>) -> io::Result<()> {
    if let Outgoing::Event(Event::Pcm {
        request,
        generation,
        block,
    }) = &message
    {
        let count = u16::try_from(block.samples.len() / 2).map_err(|_| invalid_output())?;
        if count == 0 || count > MAX_PCM_FRAMES || !block.samples.len().is_multiple_of(2) {
            return Err(invalid_output());
        }
        let mut pcm = Vec::with_capacity(26 + block.samples.len() * 2);
        pcm.extend_from_slice(&request.get().to_le_bytes());
        pcm.extend_from_slice(&generation.get().to_le_bytes());
        pcm.extend_from_slice(&block.first_frame.to_le_bytes());
        pcm.extend_from_slice(&count.to_le_bytes());
        for sample in &block.samples {
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
        return write_parts(output, 0x86, &pcm, &[]);
    }
    let mut metadata = [0; 32];
    let (kind, fields, raw): (u8, usize, &[u8]) = match message {
        Outgoing::Opened {
            generation,
            duration_micros,
            width,
            height,
            audio,
        } => {
            // Opened describes natural oriented dimensions, separately from
            // the requested/output frame cap enforced by output_length.
            let pixels = u64::from(width).checked_mul(u64::from(height));
            if width == 0
                || height == 0
                || pixels.is_none_or(|pixels| pixels > 32 * 1024 * 1024)
                || duration_micros == Some(u64::MAX)
            {
                return Err(invalid_output());
            }
            metadata[..8].copy_from_slice(&generation.get().to_le_bytes());
            metadata[8..16].copy_from_slice(&duration_micros.unwrap_or(u64::MAX).to_le_bytes());
            metadata[16..20].copy_from_slice(&width.to_le_bytes());
            metadata[20..24].copy_from_slice(&height.to_le_bytes());
            metadata[24] = audio as u8;
            (0x81, 25, &[])
        }
        Outgoing::Read {
            read_request,
            offset,
            length,
        } => {
            if u64::from(length) > MAX_READ_BYTES as u64
                || offset.checked_add(u64::from(length)).is_none()
            {
                return Err(invalid_output());
            }
            metadata[..8].copy_from_slice(&read_request.get().to_le_bytes());
            metadata[8..16].copy_from_slice(&offset.to_le_bytes());
            metadata[16..20].copy_from_slice(&length.to_le_bytes());
            (0x82, 20, &[])
        }
        Outgoing::Event(Event::Video {
            request,
            generation,
            frame,
        }) => {
            let length = crate::media::output_length(frame.width, frame.height)?;
            if frame.presentation_micros < 0 || frame.rgba.len() != length {
                return Err(invalid_output());
            }
            metadata[..8].copy_from_slice(&request.get().to_le_bytes());
            metadata[8..16].copy_from_slice(&generation.get().to_le_bytes());
            metadata[16..24].copy_from_slice(&frame.presentation_micros.to_le_bytes());
            metadata[24..28].copy_from_slice(&frame.width.to_le_bytes());
            metadata[28..32].copy_from_slice(&frame.height.to_le_bytes());
            (0x83, 32, &frame.rgba)
        }
        Outgoing::Event(Event::Pcm { .. }) => unreachable!("PCM serialized above"),
        Outgoing::Event(Event::Completed {
            request,
            generation,
            completion,
        }) => {
            metadata[..8].copy_from_slice(&request.get().to_le_bytes());
            metadata[8..16].copy_from_slice(&generation.get().to_le_bytes());
            metadata[16] = match completion {
                Completion::Served => 0,
                Completion::EndOfMedia => 1,
                Completion::Superseded => 2,
            };
            (0x84, 17, &[])
        }
        Outgoing::Failed {
            request,
            generation,
            cause,
            codec,
            tried,
        } => {
            metadata[..8].copy_from_slice(&request.to_le_bytes());
            metadata[8..16].copy_from_slice(&generation.to_le_bytes());
            metadata[16..19].copy_from_slice(&[cause, codec, tried]);
            (0x85, 19, &[])
        }
    };
    write_parts(output, kind, &metadata[..fields], raw)
}

fn write_parts(output: &mut impl Write, kind: u8, fields: &[u8], raw: &[u8]) -> io::Result<()> {
    let length = fields
        .len()
        .checked_add(raw.len())
        .and_then(|length| u32::try_from(length).ok())
        .ok_or_else(invalid_output)?;
    let mut header = [0; HEADER_BYTES];
    header[..4].copy_from_slice(b"MMV2");
    header[4] = kind;
    header[8..].copy_from_slice(&length.to_le_bytes());
    output.write_all(&header)?;
    output.write_all(fields)?;
    output.write_all(raw)?;
    output.flush()
}

#[cfg(test)]
#[path = "playback_wire_tests.rs"]
mod tests;
