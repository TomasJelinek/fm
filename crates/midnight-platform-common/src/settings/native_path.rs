//! Per-OS settings never reinterpret another system's native path encoding.
//! Unicode paths stay readable JSON strings; malformed Unicode uses exact hex.
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

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
pub(super) enum NativePath {
    Unicode(String),
    Encoded(EncodedPath),
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct EncodedPath {
    encoding: String,
    hex: String,
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
