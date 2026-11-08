//! Version-one viewer requests and host-service payloads. IDs are instance-scoped.
use crate::{
    codec::Model,
    viewer::{self, SourceRange},
    wire::WireError,
};
use minicbor::{Decode, Encode};

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Open {
    #[n(0)]
    pub reader: u64,
    #[n(1)]
    pub length: Option<u64>,
    #[n(2)]
    pub revision: u64,
    #[n(3)]
    pub mode: u16,
    /// Display only, never an authority to open an OS path.
    #[n(4)]
    pub display_name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Viewport {
    #[n(0)]
    pub source_anchor: u64,
    #[n(1)]
    pub backwards: bool,
    #[n(2)]
    pub max_bytes: u32,
    #[n(3)]
    pub max_lines: u32,
}

/// Optional lookup independent of viewport delivery. Cancellation stops indexing;
/// old plugins may reject the request as UNSUPPORTED without affecting viewing.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Locate {
    /// A raw source byte offset, not a decoded UTF-8 or display-column offset.
    #[n(0)]
    pub source_anchor: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Search {
    #[n(0)]
    pub query: String,
    #[n(1)]
    pub source_anchor: u64,
    #[n(2)]
    pub backwards: bool,
    #[n(3)]
    pub case_sensitive: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct ReadAt {
    #[n(0)]
    pub reader: u64,
    #[n(1)]
    pub revision: u64,
    #[n(2)]
    pub offset: u64,
    #[n(3)]
    pub length: u32,
}

/// Host-owned raw bytes arrive separately in Request.content. The SDK must also
/// check its length equals `bytes`; a short read is EOF only when `eof` is true.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct ReadReply {
    #[n(0)]
    pub revision: u64,
    #[n(1)]
    pub offset: u64,
    #[n(2)]
    pub bytes: u32,
    #[n(3)]
    pub eof: bool,
    #[n(4)]
    pub status: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct CloseReader {
    #[n(0)]
    pub reader: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Opened {
    #[n(0)]
    pub mode: u16,
    #[n(1)]
    pub length: Option<u64>,
    #[n(2)]
    pub initial_anchor: u64,
    #[n(3)]
    pub estimated_lines: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct SearchMatch {
    #[n(0)]
    pub source: SourceRange,
    #[n(1)]
    pub preview: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Progress {
    #[n(0)]
    pub completed: u64,
    #[n(1)]
    pub total: Option<u64>,
    #[n(2)]
    pub phase: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Failure {
    #[n(0)]
    pub code: u32,
    #[n(1)]
    pub message: String,
}

impl Model for Open {
    fn validate(&self) -> Result<(), WireError> {
        check(self.reader != 0 && valid_mode(self.mode))?;
        limit(self.display_name.len() <= viewer::MAX_PAGE_BYTES)
    }
}

impl Model for Viewport {
    fn validate(&self) -> Result<(), WireError> {
        limit(
            (1..=viewer::MAX_PAGE_BYTES as u32).contains(&self.max_bytes)
                && (1..=4096).contains(&self.max_lines),
        )
    }
}

impl Model for Locate {
    fn validate(&self) -> Result<(), WireError> {
        Ok(()) // The source-owning plugin checks its captured source length.
    }
}

impl Model for Search {
    fn validate(&self) -> Result<(), WireError> {
        limit((1..=4096).contains(&self.query.len()))
    }
}

impl Model for ReadAt {
    fn validate(&self) -> Result<(), WireError> {
        check(self.reader != 0 && self.length != 0)?;
        check_read(self.offset, self.length)
    }
}

impl Model for ReadReply {
    fn validate(&self) -> Result<(), WireError> {
        check(self.status == 0 || (self.bytes == 0 && !self.eof))?;
        check_read(self.offset, self.bytes)
    }
}

impl Model for CloseReader {
    fn validate(&self) -> Result<(), WireError> {
        check(self.reader != 0)
    }
}

impl Model for Opened {
    fn validate(&self) -> Result<(), WireError> {
        check(
            valid_mode(self.mode)
                && self
                    .length
                    .is_none_or(|length| self.initial_anchor <= length),
        )
    }
}

impl Model for SearchMatch {
    fn validate(&self) -> Result<(), WireError> {
        self.source.end()?;
        check(self.source.length != 0)?;
        limit(self.preview.len() <= 4096)
    }
}

impl Model for Progress {
    fn validate(&self) -> Result<(), WireError> {
        check(self.total.is_none_or(|total| self.completed <= total))
    }
}

impl Model for Failure {
    fn validate(&self) -> Result<(), WireError> {
        limit(self.message.len() <= 4096)
    }
}

fn check_read(offset: u64, bytes: u32) -> Result<(), WireError> {
    limit(bytes as usize <= viewer::MAX_PAGE_BYTES)?;
    SourceRange {
        offset,
        length: u64::from(bytes),
    }
    .end()?;
    Ok(())
}

pub(crate) fn valid_mode(mode: u16) -> bool {
    (viewer::MODE_TEXT..=viewer::MODE_DOCUMENT).contains(&mode)
}

pub(crate) fn check(valid: bool) -> Result<(), WireError> {
    if valid {
        Ok(())
    } else {
        Err(WireError::Malformed)
    }
}

pub(crate) fn limit(valid: bool) -> Result<(), WireError> {
    if valid { Ok(()) } else { Err(WireError::Limit) }
}
