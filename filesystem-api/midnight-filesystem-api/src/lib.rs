//! Public, path-free filesystem worker contract. See PROTOCOL.md.
mod wire;
use std::io;
pub use wire::{read_message, write_message};

pub const MAX_FRAME: usize = 1024 * 1024;
pub const MAX_READ: u32 = 256 * 1024;
pub const MAX_NAME: usize = 64 * 1024;
pub const MAX_BATCH: usize = 128;
pub const VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Format {
    Zip = 1,
    Tar = 2,
    Gzip = 3,
    TarGzip = 4,
    SevenZip = 5,
    Xz = 6,
    TarXz = 7,
    Bzip2 = 8,
    TarBzip2 = 9,
    Zstd = 10,
    TarZstd = 11,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Directory = 1,
    File = 2,
    Link = 3,
    Other = 4,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ErrorCode {
    Invalid = 1,
    Unsupported = 2,
    Limit = 3,
    Cancelled = 4,
    SourceChanged = 5,
    NotFound = 6,
    ReadOnly = 7,
    Corrupt = 8,
    Io = 9,
}

/// Identity is session-scoped and independent of the raw name and display label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub id: u64,
    pub parent: u64,
    pub name: Vec<u8>,
    pub display: String,
    pub kind: Kind,
    pub size: Option<u64>,
    pub readable: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Message {
    Open {
        request: u64,
        format: Format,
        length: u64,
    },
    Opened {
        request: u64,
        root: u64,
    },
    List {
        request: u64,
        directory: u64,
        after: u64,
    },
    Entries {
        request: u64,
        entries: Vec<Node>,
        more: bool,
    },
    Read {
        request: u64,
        entry: u64,
        offset: u64,
        length: u32,
    },
    Content {
        request: u64,
        bytes: Vec<u8>,
        eof: bool,
    },
    SourceRead {
        request: u64,
        offset: u64,
        length: u32,
    },
    SourceData {
        request: u64,
        bytes: Vec<u8>,
    },
    Failed {
        request: u64,
        code: ErrorCode,
        message: String,
    },
    Close {
        request: u64,
    },
}

impl Message {
    pub fn request(&self) -> u64 {
        match self {
            Self::Open { request, .. }
            | Self::Opened { request, .. }
            | Self::List { request, .. }
            | Self::Entries { request, .. }
            | Self::Read { request, .. }
            | Self::Content { request, .. }
            | Self::SourceRead { request, .. }
            | Self::SourceData { request, .. }
            | Self::Failed { request, .. }
            | Self::Close { request } => *request,
        }
    }
}

pub(crate) fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid filesystem worker message",
    )
}
