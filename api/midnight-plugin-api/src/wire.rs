//! Bounded, definite-length CBOR with numeric field tags.
use minicbor::{Decoder, data::Type};

pub const MAX_MESSAGE_BYTES: usize = 1024 * 1024;
pub const MAX_DEPTH: usize = 16;
pub const MAX_ITEMS: usize = 65536;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireError {
    Limit,
    Malformed,
    Unsupported,
}

/// Validate before schema decoding allocates collections or follows unknown fields.
pub fn validate(bytes: &[u8]) -> Result<(), WireError> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(WireError::Limit);
    }
    let mut decoder = Decoder::new(bytes);
    let mut remaining = MAX_ITEMS;
    value(&mut decoder, 0, &mut remaining)?;
    if decoder.position() == bytes.len() {
        Ok(())
    } else {
        Err(WireError::Malformed)
    }
}

fn value(decoder: &mut Decoder<'_>, depth: usize, remaining: &mut usize) -> Result<(), WireError> {
    if depth > MAX_DEPTH {
        return Err(WireError::Limit);
    }
    *remaining = remaining.checked_sub(1).ok_or(WireError::Limit)?;
    match decoder.datatype()? {
        Type::U8
        | Type::U16
        | Type::U32
        | Type::U64
        | Type::I8
        | Type::I16
        | Type::I32
        | Type::I64
        | Type::Int => {
            decoder.int()?;
        }
        Type::Bytes => {
            decoder.bytes()?;
        }
        Type::String => {
            decoder.str()?;
        }
        Type::Bool => {
            decoder.bool()?;
        }
        Type::Null => {
            decoder.null()?;
        }
        Type::Array => {
            let count = decoder.array()?.ok_or(WireError::Unsupported)?;
            if count > *remaining as u64 {
                return Err(WireError::Limit);
            }
            for _ in 0..count {
                value(decoder, depth + 1, remaining)?;
            }
        }
        Type::Map => {
            let count = decoder.map()?.ok_or(WireError::Unsupported)?;
            if count > (*remaining / 2) as u64 {
                return Err(WireError::Limit);
            }
            let mut previous = None;
            for _ in 0..count {
                let tag = decoder.u64()?;
                if previous.is_some_and(|previous| tag <= previous) {
                    return Err(WireError::Malformed);
                }
                previous = Some(tag);
                *remaining = remaining.checked_sub(1).ok_or(WireError::Limit)?;
                value(decoder, depth + 1, remaining)?;
            }
        }
        // No indefinite containers, tags, floating-point or extension simple values.
        _ => return Err(WireError::Unsupported),
    }
    Ok(())
}

impl From<minicbor::decode::Error> for WireError {
    fn from(_: minicbor::decode::Error) -> Self {
        Self::Malformed
    }
}

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Limit => "plugin message exceeds its resource budget",
            Self::Malformed => "invalid or ambiguous plugin message",
            Self::Unsupported => "unsupported CBOR representation",
        })
    }
}

impl std::error::Error for WireError {}
