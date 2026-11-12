//! Semantic viewer data; source bytes, decoded UTF-8 offsets and pixels are distinct.
use crate::wire::WireError;
use minicbor::{Decode, Encode};

pub const MAX_PAGE_BYTES: usize = 256 * 1024;
pub const MAX_SPANS: usize = 4096;
pub const MAX_ANCHORS: usize = 16384;

pub const MODE_TEXT: u16 = 1;
pub const MODE_SOURCE: u16 = 2;
pub const MODE_BINARY: u16 = 3;
pub const MODE_DOCUMENT: u16 = 4;

pub const STYLE_PLAIN: u16 = 0;
pub const STYLE_KEYWORD: u16 = 1;
pub const STYLE_STRING: u16 = 2;
pub const STYLE_COMMENT: u16 = 3;
pub const STYLE_NUMBER: u16 = 4;
pub const STYLE_TYPE: u16 = 5;
pub const STYLE_FUNCTION: u16 = 6;
pub const STYLE_OPERATOR: u16 = 7;
pub const STYLE_HEADING: u16 = 8;
pub const STYLE_LINK: u16 = 9;
pub const STYLE_CODE: u16 = 10;
pub const STYLE_INVALID: u16 = 11;

/// Raw bytes are carried separately in Event.content, exactly source.length bytes.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct BinaryPage {
    #[n(0)]
    pub revision: u64,
    #[n(1)]
    pub source: SourceRange,
    #[n(2)]
    pub has_before: bool,
    #[n(3)]
    pub has_after: bool,
}

impl crate::codec::Model for BinaryPage {
    fn validate(&self) -> Result<(), WireError> {
        self.source.end()?;
        crate::messages::limit(self.source.length <= MAX_PAGE_BYTES as u64)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct SourceRange {
    #[n(0)]
    pub offset: u64,
    #[n(1)]
    pub length: u64,
}

impl SourceRange {
    pub fn end(&self) -> Result<u64, WireError> {
        self.offset
            .checked_add(self.length)
            .ok_or(WireError::Malformed)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Span {
    #[n(0)]
    pub start: u32,
    #[n(1)]
    pub end: u32,
    #[n(2)]
    pub role: u16,
    /// Bit 0: bold; bit 1: italic; bit 2: underline. Other bits require a new model.
    #[n(3)]
    pub flags: u8,
}

/// Boundary mapping supplied by the decoder. Invalid bytes/UTF-16 do not share
/// UTF-8 offsets; every page has explicit first/last source anchors.
/// Only listed boundaries are exact; consumers must not interpolate between them.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Anchor {
    #[n(0)]
    pub text_offset: u32,
    #[n(1)]
    pub source_offset: u64,
}

/// Exact logical coordinates at a decoded source boundary, scoped to a revision.
/// Lines/columns are one-based. Columns count displayed Unicode scalar values,
/// including tabs and combining scalars individually, not glyphs, UTF-16 words,
/// screen cells or bytes. CR/CRLF normalize to LF; LF, U+2028 and U+2029 start
/// lines. BOM is excluded. Invalid/control substitutions count as one scalar.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct TextPosition {
    #[n(0)]
    pub revision: u64,
    /// First complete decoded boundary at or after the requested raw anchor.
    /// A request within a scalar or CRLF may therefore return a later offset.
    #[n(1)]
    pub source_offset: u64,
    #[n(2)]
    pub line: u64,
    #[n(3)]
    pub column: u64,
}

impl crate::codec::Model for TextPosition {
    fn validate(&self) -> Result<(), WireError> {
        crate::messages::check(self.line != 0 && self.column != 0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct TextPage {
    #[n(0)]
    pub revision: u64,
    #[n(1)]
    pub source: SourceRange,
    #[n(2)]
    pub text: String,
    #[n(3)]
    pub spans: Vec<Span>,
    #[n(4)]
    pub anchors: Vec<Anchor>,
    #[n(5)]
    pub has_before: bool,
    #[n(6)]
    pub has_after: bool,
    /// True when an enormous logical line continues outside this fragment.
    #[n(7)]
    pub partial_line: bool,
}

impl TextPage {
    pub fn validate(&self) -> Result<(), WireError> {
        if self.text.len() > MAX_PAGE_BYTES
            || self.spans.len() > MAX_SPANS
            || self.anchors.len() > MAX_ANCHORS
        {
            return Err(WireError::Limit);
        }
        let source_end = self.source.end()?;
        let mut end = 0;
        for span in &self.spans {
            if span.start < end
                || span.start >= span.end
                || span.flags & !7 != 0
                || self
                    .text
                    .get(span.start as usize..span.end as usize)
                    .is_none()
            {
                return Err(WireError::Malformed);
            }
            end = span.end;
        }
        let first = self.anchors.first().ok_or(WireError::Malformed)?;
        let last = self.anchors.last().ok_or(WireError::Malformed)?;
        if first.text_offset != 0
            || first.source_offset != self.source.offset
            || last.text_offset as usize != self.text.len()
            || last.source_offset != source_end
        {
            return Err(WireError::Malformed);
        }
        let mut previous: Option<&Anchor> = None;
        for anchor in &self.anchors {
            if !self.text.is_char_boundary(anchor.text_offset as usize)
                || !(self.source.offset..=source_end).contains(&anchor.source_offset)
                || previous.is_some_and(|previous| {
                    anchor.text_offset <= previous.text_offset
                        || anchor.source_offset < previous.source_offset
                })
            {
                return Err(WireError::Malformed);
            }
            previous = Some(anchor);
        }
        Ok(())
    }
}

impl crate::codec::Model for TextPage {
    fn validate(&self) -> Result<(), WireError> {
        self.validate()
    }
}
