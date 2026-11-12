//! Public, path-free filesystem worker contract. See PROTOCOL.md.
mod streaming;
mod timestamp;
mod wire;
use std::io;
pub use timestamp::Timestamp;
pub use wire::{
    read_message, read_message_with_protocol, write_message, write_message_with_protocol,
};

pub const MAX_FRAME: usize = 1024 * 1024;
pub const MAX_READ: u32 = 256 * 1024;
pub const MAX_NAME: usize = 64 * 1024;
pub const MAX_BATCH: usize = 128;
pub const VERSION: u16 = 1;

/// Select the installed worker's protocol explicitly; legacy helpers always use V1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u16)]
pub enum Protocol {
    #[default]
    V1 = 1,
    V2 = 2,
    /// Streaming with acknowledged request-local member cancellation.
    V3 = 3,
}

impl Protocol {
    /// Protocols with host-owned staging and policy-ready containment.
    pub fn supports_staging(self) -> bool {
        matches!(self, Self::V2 | Self::V3)
    }
}

impl TryFrom<u16> for Protocol {
    type Error = io::Error;
    fn try_from(value: u16) -> io::Result<Self> {
        match value {
            1 => Ok(Self::V1),
            2 => Ok(Self::V2),
            3 => Ok(Self::V3),
            _ => Err(invalid()),
        }
    }
}

/// Host-owned staging authority. IDs are session scoped and never filesystem paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StagePurpose {
    Member { entry: u64 },
    Scratch,
}

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
    /// UTC when the archive records an offset, otherwise a timezone-free civil time.
    pub modified: Option<Timestamp>,
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
    StageBegin {
        request: u64,
        purpose: StagePurpose,
        expected_length: Option<u64>,
        max_length: u64,
    },
    StageCreated {
        request: u64,
        stage: u64,
    },
    StageAppend {
        request: u64,
        stage: u64,
        offset: u64,
        bytes: Vec<u8>,
    },
    StageAck {
        request: u64,
        stage: u64,
        next_offset: u64,
    },
    StageSeal {
        request: u64,
        stage: u64,
        length: u64,
    },
    StageSealed {
        request: u64,
        stage: u64,
        length: u64,
    },
    SpoolRead {
        request: u64,
        stage: u64,
        offset: u64,
        length: u32,
    },
    SpoolData {
        request: u64,
        stage: u64,
        bytes: Vec<u8>,
    },
    ContentReady {
        request: u64,
        entry: u64,
        stage: u64,
        offset: u64,
        length: u64,
    },
    /// Actual decoded bytes consumed while validating unpublished staged content.
    StageProgress {
        request: u64,
        stage: u64,
        completed: u64,
    },
    StageProgressAck {
        request: u64,
        stage: u64,
        completed: u64,
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
            | Self::Close { request }
            | Self::StageBegin { request, .. }
            | Self::StageCreated { request, .. }
            | Self::StageAppend { request, .. }
            | Self::StageAck { request, .. }
            | Self::StageSeal { request, .. }
            | Self::StageSealed { request, .. }
            | Self::SpoolRead { request, .. }
            | Self::SpoolData { request, .. }
            | Self::ContentReady { request, .. } => *request,
            Self::StageProgress { request, .. } | Self::StageProgressAck { request, .. } => {
                *request
            }
        }
    }
}

pub(crate) fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid filesystem worker message",
    )
}
