//! Protocol2 additions. Stateful grants, source identity and integrity stay with the host.
use crate::{wire::*, *};
use std::io::Cursor;

pub(crate) fn tag(message: &Message) -> Option<u8> {
    Some(match message {
        Message::StageBegin { .. } => 11,
        Message::StageCreated { .. } => 12,
        Message::StageAppend { .. } => 13,
        Message::StageAck { .. } => 14,
        Message::StageSeal { .. } => 15,
        Message::StageSealed { .. } => 16,
        Message::SpoolRead { .. } => 17,
        Message::SpoolData { .. } => 18,
        Message::ContentReady { .. } => 19,
        Message::StageProgress { .. } => 20,
        Message::StageProgressAck { .. } => 21,
        _ => return None,
    })
}

pub(crate) fn write(out: &mut Vec<u8>, message: &Message) -> io::Result<()> {
    match message {
        Message::StageBegin {
            purpose,
            expected_length,
            max_length,
            ..
        } => {
            match purpose {
                StagePurpose::Scratch => out.push(0),
                StagePurpose::Member { entry } => {
                    out.push(1);
                    put64(out, *entry);
                }
            }
            out.push(u8::from(expected_length.is_some()));
            if let Some(length) = expected_length {
                put64(out, *length);
            }
            put64(out, *max_length);
        }
        Message::StageCreated { stage, .. } => put64(out, *stage),
        Message::StageProgress {
            stage, completed, ..
        }
        | Message::StageProgressAck {
            stage, completed, ..
        } => {
            put64(out, *stage);
            put64(out, *completed);
        }
        Message::StageAppend {
            stage,
            offset,
            bytes: value,
            ..
        } => {
            put64(out, *stage);
            put64(out, *offset);
            bytes(out, value, MAX_READ as usize)?;
        }
        Message::StageAck {
            stage, next_offset, ..
        } => {
            put64(out, *stage);
            put64(out, *next_offset);
        }
        Message::StageSeal { stage, length, .. } | Message::StageSealed { stage, length, .. } => {
            put64(out, *stage);
            put64(out, *length);
        }
        Message::SpoolRead {
            stage,
            offset,
            length,
            ..
        } => {
            put64(out, *stage);
            put64(out, *offset);
            read_len(out, *length)?;
        }
        Message::SpoolData {
            stage,
            bytes: value,
            ..
        } => {
            put64(out, *stage);
            bytes(out, value, MAX_READ as usize)?;
        }
        Message::ContentReady {
            entry,
            stage,
            offset,
            length,
            ..
        } => {
            put64(out, *entry);
            put64(out, *stage);
            put64(out, *offset);
            put64(out, *length);
        }
        _ => return Err(invalid()),
    }
    Ok(())
}

pub(crate) fn read(tag: u8, request: u64, input: &mut Cursor<&[u8]>) -> io::Result<Message> {
    Ok(match tag {
        20 => Message::StageProgress {
            request,
            stage: u64le(input)?,
            completed: u64le(input)?,
        },
        21 => Message::StageProgressAck {
            request,
            stage: u64le(input)?,
            completed: u64le(input)?,
        },
        11 => Message::StageBegin {
            request,
            purpose: match byte(input)? {
                0 => StagePurpose::Scratch,
                1 => StagePurpose::Member {
                    entry: u64le(input)?,
                },
                _ => return Err(invalid()),
            },
            expected_length: if boolean(input)? {
                Some(u64le(input)?)
            } else {
                None
            },
            max_length: u64le(input)?,
        },
        12 => Message::StageCreated {
            request,
            stage: u64le(input)?,
        },
        13 => Message::StageAppend {
            request,
            stage: u64le(input)?,
            offset: u64le(input)?,
            bytes: blob(input, MAX_READ as usize)?,
        },
        14 => Message::StageAck {
            request,
            stage: u64le(input)?,
            next_offset: u64le(input)?,
        },
        15 => Message::StageSeal {
            request,
            stage: u64le(input)?,
            length: u64le(input)?,
        },
        16 => Message::StageSealed {
            request,
            stage: u64le(input)?,
            length: u64le(input)?,
        },
        17 => Message::SpoolRead {
            request,
            stage: u64le(input)?,
            offset: u64le(input)?,
            length: bounded_len(input)?,
        },
        18 => Message::SpoolData {
            request,
            stage: u64le(input)?,
            bytes: blob(input, MAX_READ as usize)?,
        },
        19 => Message::ContentReady {
            request,
            entry: u64le(input)?,
            stage: u64le(input)?,
            offset: u64le(input)?,
            length: u64le(input)?,
        },
        _ => return Err(invalid()),
    })
}

pub(crate) fn validate(message: &Message) -> io::Result<()> {
    if message.request() == 0 {
        return Err(invalid());
    }
    match message {
        Message::Entries { entries, .. } => {
            for node in entries {
                member(node.id)?;
                nonzero(node.parent)?;
                if node.id == node.parent || (node.readable && node.kind != Kind::File) {
                    return Err(invalid());
                }
            }
        }
        Message::SourceRead { offset, length, .. } => range(*offset, u64::from(*length))?,
        Message::Read {
            entry,
            offset,
            length,
            ..
        } => {
            member(*entry)?;
            range(*offset, u64::from(*length))?;
        }
        Message::Opened { root, .. } if *root != 1 => return Err(invalid()),
        Message::List { directory: 0, .. } => return Err(invalid()),
        Message::StageBegin {
            purpose,
            expected_length,
            max_length,
            ..
        } => {
            if let StagePurpose::Member { entry } = purpose {
                member(*entry)?;
            }
            if expected_length.is_some_and(|length| length > *max_length) {
                return Err(invalid());
            }
        }
        Message::StageCreated { stage, .. }
        | Message::StageProgress { stage, .. }
        | Message::StageProgressAck { stage, .. }
        | Message::StageAck { stage, .. }
        | Message::StageSeal { stage, .. }
        | Message::StageSealed { stage, .. }
        | Message::SpoolData { stage, .. } => nonzero(*stage)?,
        Message::StageAppend {
            stage,
            offset,
            bytes,
            ..
        } => {
            nonzero(*stage)?;
            if bytes.is_empty() || bytes.len() > MAX_READ as usize {
                return Err(invalid());
            }
            range(*offset, bytes.len() as u64)?;
        }
        Message::SpoolRead {
            stage,
            offset,
            length,
            ..
        } => {
            nonzero(*stage)?;
            range(*offset, u64::from(*length))?;
        }
        Message::ContentReady {
            entry,
            stage,
            offset,
            length,
            ..
        } => {
            member(*entry)?;
            nonzero(*stage)?;
            range(*offset, *length)?;
        }
        _ => {}
    }
    Ok(())
}
fn member(id: u64) -> io::Result<()> {
    if id < 2 { Err(invalid()) } else { Ok(()) }
}
fn nonzero(id: u64) -> io::Result<()> {
    if id == 0 { Err(invalid()) } else { Ok(()) }
}
fn range(offset: u64, length: u64) -> io::Result<()> {
    offset.checked_add(length).ok_or_else(invalid).map(|_| ())
}
