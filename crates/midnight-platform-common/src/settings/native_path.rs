//! Per-OS settings never reinterpret another system's native path encoding.
//! Unicode paths stay readable JSON strings; malformed Unicode uses exact hex.
//! A path in the other system's syntax or encoding is recognized as foreign and
//! only described; it is never converted into a native path.
use midnight_core::PathSystem;
use serde::{Deserialize, Serialize};
use std::{io, path::PathBuf};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;
#[cfg(unix)]
use unix as platform;
#[cfg(windows)]
use windows as platform;

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum NativePath {
    Unicode(String),
    Encoded(EncodedPath),
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EncodedPath {
    encoding: String,
    hex: String,
}

/// A stored path of another system and its display text.
pub(super) type Foreign = (PathSystem, String);

impl NativePath {
    /// The other system and display text when this path belongs to it.
    /// Malformed foreign hex is invalid data, as it would be on its own system.
    pub(super) fn foreign(&self) -> io::Result<Option<Foreign>> {
        match self {
            Self::Unicode(text) => {
                Ok(PathSystem::foreign(text).map(|system| (system, text.clone())))
            }
            Self::Encoded(encoded) => encoded.foreign(),
        }
    }

    /// A native absolute stand-in, used only to validate the rest of a record that
    /// keeps a foreign path. The converted record is always discarded.
    pub(super) fn stand_in() -> Self {
        Self::Unicode(match PathSystem::CURRENT {
            PathSystem::Windows => "C:\\".into(),
            PathSystem::Unix => "/".into(),
        })
    }

    /// Replace a foreign path by [`Self::stand_in`] for validation.
    pub(super) fn validation_copy(&self) -> io::Result<Self> {
        Ok(match self.foreign()? {
            Some(_) => Self::stand_in(),
            None => self.clone(),
        })
    }
}

impl EncodedPath {
    fn foreign(&self) -> io::Result<Option<Foreign>> {
        let system = match self.encoding.as_str() {
            "windows-utf16" => PathSystem::Windows,
            "unix-bytes" => PathSystem::Unix,
            _ => return Ok(None),
        };
        if system == PathSystem::CURRENT {
            return Ok(None);
        }
        let digits = self.hex.as_bytes();
        // Display only: replacement characters mark what the text cannot show.
        let display = match system {
            PathSystem::Windows if digits.len().is_multiple_of(4) => {
                let units = digits
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .map(|unit| {
                        unit.iter()
                            .try_fold(0u16, |value, digit| Ok(value << 4 | nibble(*digit)?))
                    })
                    .collect::<io::Result<Vec<u16>>>()?;
                String::from_utf16_lossy(&units)
            }
            PathSystem::Unix if digits.len().is_multiple_of(2) => {
                let bytes = digits
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|byte| Ok((nibble(byte[0])? << 4 | nibble(byte[1])?) as u8))
                    .collect::<io::Result<Vec<u8>>>()?;
                String::from_utf8_lossy(&bytes).into_owned()
            }
            _ => {
                return Err(invalid(
                    "foreign native path has incomplete hexadecimal units",
                ));
            }
        };
        if display.is_empty() || display.contains('\0') {
            return Err(invalid("foreign native path is empty or contains NUL"));
        }
        Ok(Some((system, display)))
    }
}

impl NativePath {
    pub(super) fn from_path(path: PathBuf) -> Self {
        match path.to_str() {
            Some(path) => Self::Unicode(path.to_owned()),
            None => Self::Encoded(platform::encode(path)),
        }
    }

    pub(super) fn into_path(self) -> io::Result<PathBuf> {
        match self {
            Self::Unicode(path) => Ok(path.into()),
            Self::Encoded(encoded) => platform::decode(encoded),
        }
    }
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn nibble(byte: u8) -> io::Result<u16> {
    match byte {
        b'0'..=b'9' => Ok(u16::from(byte - b'0')),
        b'a'..=b'f' => Ok(u16::from(byte - b'a' + 10)),
        b'A'..=b'F' => Ok(u16::from(byte - b'A' + 10)),
        _ => Err(invalid("native path contains invalid hexadecimal digits")),
    }
}
