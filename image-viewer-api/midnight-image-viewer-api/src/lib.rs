//! Versioned persistent raster protocol. No filenames, URLs or native identities cross it.
#![forbid(unsafe_code)]

use std::io::{self, Read, Write};

/// Maximum physical viewport edge.
pub const MAX_EDGE: u32 = 4096;
/// Maximum physical pixels in one output, independently of the decoded image size.
pub const MAX_PIXELS: u64 = 4 * 1024 * 1024;
/// Maximum decoded edge before or after orientation.
pub const MAX_DECODED_EDGE: u32 = 32768;
/// Maximum retained decoded RGBA8 allocation, also applied to decoder allocations.
pub const MAX_DECODED_BYTES: u64 = 128 * 1024 * 1024;
/// Maximum opaque source read or reply.
pub const MAX_READ_BYTES: usize = 256 * 1024;
/// Smallest supported physical-pixels-per-image-pixel scale.
pub const MIN_SCALE: f64 = 1.0 / MAX_DECODED_EDGE as f64;
/// Largest supported physical-pixels-per-image-pixel scale.
pub const MAX_SCALE: f64 = 64.0;
/// Maximum pages in a paginated document. Metadata is bounded independently of raster pixels.
pub const MAX_DOCUMENT_PAGES: usize = 4096;
/// Longest failure reason in bytes. Reasons are printable ASCII, so also in characters.
pub const MAX_REASON_BYTES: usize = 160;

/// Original dimensions after orientation. Animated sources expose the first still frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub width: u32,
    pub height: u32,
}

impl Metadata {
    /// Reject dimensions that exceed either the edge or decoded-byte budgets.
    pub fn validate(self) -> io::Result<Self> {
        if self.width == 0
            || self.height == 0
            || self.width > MAX_DECODED_EDGE
            || self.height > MAX_DECODED_EDGE
            || u64::from(self.width) * u64::from(self.height) * 4 > MAX_DECODED_BYTES
        {
            return Err(invalid());
        }
        Ok(self)
    }
}

/// Ordered document page sizes in logical units. Raster viewports address one zero-based page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DocumentMetadata {
    pub pages: Vec<Metadata>,
}

impl DocumentMetadata {
    /// Validate the page count before retaining metadata or accepting page requests.
    pub fn validate(&self) -> io::Result<()> {
        if self.pages.is_empty() || self.pages.len() > MAX_DOCUMENT_PAGES {
            return Err(invalid());
        }
        for page in &self.pages {
            page.validate()?;
        }
        Ok(())
    }
}

/// Physical output size, source-to-physical scale, and source-coordinate top-left.
/// Pixel centers are sampled at `origin + (output + 0.5) / scale`. Outside-image pixels
/// are transparent. Scale 1 with integral origins must reproduce exact original pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub origin_x: f64,
    pub origin_y: f64,
}

impl Viewport {
    /// Validate all numeric values before allocation or coordinate conversion.
    pub fn validate(self) -> io::Result<Self> {
        rgba_length(self.width, self.height)?;
        if !self.scale.is_finite()
            || !(MIN_SCALE..=MAX_SCALE).contains(&self.scale)
            || !self.origin_x.is_finite()
            || !self.origin_y.is_finite()
            || self.origin_x.abs() > f64::from(MAX_DECODED_EDGE) * f64::from(MAX_EDGE)
            || self.origin_y.abs() > f64::from(MAX_DECODED_EDGE) * f64::from(MAX_EDGE)
        {
            return Err(invalid());
        }
        Ok(self)
    }
}

/// Straight-alpha sRGB RGBA8 physical viewport pixels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A provider's short explanation of a `Failure`, for the viewer's status line: 1 to
/// `MAX_REASON_BYTES` printable ASCII characters (0x20-0x7E) without `/` or `\`, so it can
/// carry neither control or direction characters nor a path or URL. It comes from
/// untrusted content and is shown as text only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailureReason(String);

impl FailureReason {
    /// Accept text that already has the wire form; anything else is invalid.
    pub fn new(text: &str) -> io::Result<Self> {
        if text.is_empty()
            || text.len() > MAX_REASON_BYTES
            || !text
                .bytes()
                .all(|byte| (0x20..=0x7e).contains(&byte) && byte != b'/' && byte != b'\\')
        {
            return Err(invalid());
        }
        Ok(Self(text.to_owned()))
    }

    /// Reduce arbitrary decoder text to the wire form. Whitespace and control characters
    /// become single spaces, other non-ASCII characters `?`, and each word that contains
    /// `/` or `\` (a path or URL) `...`. Longer text is cut and ends with `...`. Text
    /// without a letter or digit left gives `None`. Only a bounded prefix is examined.
    pub fn sanitize(text: &str) -> Option<Self> {
        // Each output byte needs at least one input character; a few more let a long
        // replaced word or collapsed whitespace still fill the bound.
        const EXAMINED: usize = MAX_REASON_BYTES * 4;
        let mut characters = text.chars();
        let mut normalized = String::with_capacity(EXAMINED);
        for character in characters.by_ref().take(EXAMINED) {
            normalized.push(match character {
                ' '..='~' => character,
                _ if character.is_whitespace() || character.is_control() => ' ',
                _ => '?',
            });
        }
        let mut cut = characters.next().is_some();
        let mut reason = String::with_capacity(MAX_REASON_BYTES + 4);
        for word in normalized.split(' ').filter(|word| !word.is_empty()) {
            let word = if word.contains(['/', '\\']) {
                "..."
            } else {
                word
            };
            if !reason.is_empty() {
                reason.push(' ');
            }
            reason.push_str(word);
            if reason.len() > MAX_REASON_BYTES {
                cut = true;
                break;
            }
        }
        if cut {
            // Everything is ASCII here, so any byte index is a character boundary. End at
            // a word boundary unless the text is one long word.
            let end = MAX_REASON_BYTES - 3;
            if let Some(&next) = reason.as_bytes().get(end) {
                reason.truncate(end);
                if next != b' '
                    && let Some(space) = reason.rfind(' ')
                {
                    reason.truncate(space);
                }
            }
            reason.truncate(reason.trim_end().len());
            reason.push_str("...");
        }
        if !reason.bytes().any(|byte| byte.is_ascii_alphanumeric()) {
            return None;
        }
        Self::new(&reason).ok()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Raster protocol. Only Open may cause source reads; viewports use retained content.
/// Protocol-2 document packages use the additive document metadata and page viewport messages.
/// `FailureReason` (API 0.3.0) may only follow `Failure`, as the worker's last message.
/// Hosts before 0.3.0 stop reading at `Failure`, so the trailer never reaches them.
#[derive(Clone, Debug, PartialEq)]
pub enum Message {
    Open {
        source_length: u64,
    },
    Metadata(Metadata),
    Viewport {
        id: u64,
        viewport: Viewport,
    },
    Rgba {
        id: u64,
        frame: Frame,
    },
    Read {
        offset: u64,
        length: u32,
    },
    Data(Vec<u8>),
    Failure,
    DocumentMetadata(DocumentMetadata),
    PageViewport {
        id: u64,
        page: u32,
        viewport: Viewport,
    },
    FailureReason(FailureReason),
}

fn invalid() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "Invalid image-viewer protocol message",
    )
}

/// Validate dimensions before allocating or presenting a physical viewport.
pub fn rgba_length(width: u32, height: u32) -> io::Result<usize> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || width > MAX_EDGE || height > MAX_EDGE || pixels > MAX_PIXELS {
        return Err(invalid());
    }
    Ok((pixels * 4) as usize)
}

fn read_length(offset: u64, length: u32) -> io::Result<()> {
    if length == 0
        || length as usize > MAX_READ_BYTES
        || offset.checked_add(u64::from(length)).is_none()
    {
        return Err(invalid());
    }
    Ok(())
}

/// Validate, write and flush one message. Large payloads are not copied into another wire buffer.
pub fn write_message(writer: &mut impl Write, message: &Message) -> io::Result<()> {
    let mut header = Vec::with_capacity(41);
    let mut bytes: &[u8] = &[];
    match message {
        Message::Open { source_length } => {
            header.push(1);
            header.extend_from_slice(&source_length.to_le_bytes());
        }
        Message::Metadata(metadata) => {
            metadata.validate()?;
            header.push(2);
            header.extend_from_slice(&metadata.width.to_le_bytes());
            header.extend_from_slice(&metadata.height.to_le_bytes());
        }
        Message::Viewport { id, viewport } => {
            viewport.validate()?;
            header.push(3);
            header.extend_from_slice(&id.to_le_bytes());
            header.extend_from_slice(&viewport.width.to_le_bytes());
            header.extend_from_slice(&viewport.height.to_le_bytes());
            header.extend_from_slice(&viewport.scale.to_le_bytes());
            header.extend_from_slice(&viewport.origin_x.to_le_bytes());
            header.extend_from_slice(&viewport.origin_y.to_le_bytes());
        }
        Message::Rgba { id, frame } => {
            if rgba_length(frame.width, frame.height)? != frame.rgba.len() {
                return Err(invalid());
            }
            header.push(4);
            header.extend_from_slice(&id.to_le_bytes());
            header.extend_from_slice(&frame.width.to_le_bytes());
            header.extend_from_slice(&frame.height.to_le_bytes());
            bytes = &frame.rgba;
        }
        Message::Read { offset, length } => {
            read_length(*offset, *length)?;
            header.push(5);
            header.extend_from_slice(&offset.to_le_bytes());
            header.extend_from_slice(&length.to_le_bytes());
        }
        Message::Data(data) => {
            if data.len() > MAX_READ_BYTES {
                return Err(invalid());
            }
            header.push(6);
            bytes = data;
        }
        Message::Failure => header.push(7),
        Message::DocumentMetadata(metadata) => {
            metadata.validate()?;
            header.push(8);
            header.extend_from_slice(&(metadata.pages.len() as u32).to_le_bytes());
            for page in &metadata.pages {
                header.extend_from_slice(&page.width.to_le_bytes());
                header.extend_from_slice(&page.height.to_le_bytes());
            }
        }
        Message::PageViewport { id, page, viewport } => {
            viewport.validate()?;
            if *page as usize >= MAX_DOCUMENT_PAGES {
                return Err(invalid());
            }
            header.push(9);
            header.extend_from_slice(&id.to_le_bytes());
            header.extend_from_slice(&page.to_le_bytes());
            header.extend_from_slice(&viewport.width.to_le_bytes());
            header.extend_from_slice(&viewport.height.to_le_bytes());
            header.extend_from_slice(&viewport.scale.to_le_bytes());
            header.extend_from_slice(&viewport.origin_x.to_le_bytes());
            header.extend_from_slice(&viewport.origin_y.to_le_bytes());
        }
        Message::FailureReason(reason) => {
            header.push(10);
            bytes = reason.as_str().as_bytes();
        }
    }
    writer.write_all(b"MIV1")?;
    writer.write_all(&((header.len() + bytes.len()) as u32).to_le_bytes())?;
    writer.write_all(&header)?;
    writer.write_all(bytes)?;
    writer.flush()
}

fn array<const N: usize>(reader: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Read a message with type-specific allocation bounds. Unknown versions, tags, numeric
/// overflows, nonfinite geometry, truncated data and trailing fields are errors.
pub fn read_message(reader: &mut impl Read) -> io::Result<Message> {
    if &array::<4>(reader)? != b"MIV1" {
        return Err(invalid());
    }
    let length = u32::from_le_bytes(array(reader)?) as usize;
    if length == 0 || length > MAX_PIXELS as usize * 4 + 17 {
        return Err(invalid());
    }
    match (array::<1>(reader)?[0], length) {
        (1, 9) => Ok(Message::Open {
            source_length: u64::from_le_bytes(array(reader)?),
        }),
        (2, 9) => Ok(Message::Metadata(
            Metadata {
                width: u32::from_le_bytes(array(reader)?),
                height: u32::from_le_bytes(array(reader)?),
            }
            .validate()?,
        )),
        (3, 41) => Ok(Message::Viewport {
            id: u64::from_le_bytes(array(reader)?),
            viewport: Viewport {
                width: u32::from_le_bytes(array(reader)?),
                height: u32::from_le_bytes(array(reader)?),
                scale: f64::from_le_bytes(array(reader)?),
                origin_x: f64::from_le_bytes(array(reader)?),
                origin_y: f64::from_le_bytes(array(reader)?),
            }
            .validate()?,
        }),
        (4, length) if length >= 17 => {
            let id = u64::from_le_bytes(array(reader)?);
            let width = u32::from_le_bytes(array(reader)?);
            let height = u32::from_le_bytes(array(reader)?);
            let size = rgba_length(width, height)?;
            if length != size + 17 {
                return Err(invalid());
            }
            let mut rgba = vec![0; size];
            reader.read_exact(&mut rgba)?;
            Ok(Message::Rgba {
                id,
                frame: Frame {
                    width,
                    height,
                    rgba,
                },
            })
        }
        (5, 13) => {
            let offset = u64::from_le_bytes(array(reader)?);
            let length = u32::from_le_bytes(array(reader)?);
            read_length(offset, length)?;
            Ok(Message::Read { offset, length })
        }
        (6, length) if length <= MAX_READ_BYTES + 1 => {
            let mut bytes = vec![0; length - 1];
            reader.read_exact(&mut bytes)?;
            Ok(Message::Data(bytes))
        }
        (7, 1) => Ok(Message::Failure),
        (8, length) if (13..=5 + 8 * MAX_DOCUMENT_PAGES).contains(&length) => {
            let count = u32::from_le_bytes(array(reader)?) as usize;
            if count == 0 || count > MAX_DOCUMENT_PAGES || length != 5 + count * 8 {
                return Err(invalid());
            }
            let mut pages = Vec::with_capacity(count);
            for _ in 0..count {
                pages.push(
                    Metadata {
                        width: u32::from_le_bytes(array(reader)?),
                        height: u32::from_le_bytes(array(reader)?),
                    }
                    .validate()?,
                );
            }
            Ok(Message::DocumentMetadata(DocumentMetadata { pages }))
        }
        (9, 45) => {
            let id = u64::from_le_bytes(array(reader)?);
            let page = u32::from_le_bytes(array(reader)?);
            if page as usize >= MAX_DOCUMENT_PAGES {
                return Err(invalid());
            }
            let viewport = Viewport {
                width: u32::from_le_bytes(array(reader)?),
                height: u32::from_le_bytes(array(reader)?),
                scale: f64::from_le_bytes(array(reader)?),
                origin_x: f64::from_le_bytes(array(reader)?),
                origin_y: f64::from_le_bytes(array(reader)?),
            }
            .validate()?;
            Ok(Message::PageViewport { id, page, viewport })
        }
        (10, length) if (2..=MAX_REASON_BYTES + 1).contains(&length) => {
            let mut bytes = vec![0; length - 1];
            reader.read_exact(&mut bytes)?;
            let text = std::str::from_utf8(&bytes).map_err(|_| invalid())?;
            Ok(Message::FailureReason(FailureReason::new(text)?))
        }
        _ => Err(invalid()),
    }
}
