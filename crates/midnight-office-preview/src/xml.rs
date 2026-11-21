//! A lexical input gate bounds tokens BEFORE quick-xml grows an event buffer.
use crate::OfficeError;
use quick_xml::{NsReader, events::Event, name::ResolveResult};
use std::io::{self, BufRead, BufReader, Read};

const TOKEN: usize = 64 * 1024;
const UTF16_OR_BINARY: &str =
    "contains XML in UTF-16 or binary form, which the preview cannot read";
const DOCUMENT_TYPE: &str =
    "contains an XML document type definition (DTD), which the preview refuses for safety";
const ENCODING: &str =
    "declares an XML text encoding other than UTF-8, which the preview cannot read";
const INSTRUCTION: &str = "contains XML processing instructions, which the preview does not read";

#[cfg(test)]
#[path = "xml_guard_tests.rs"]
mod guard_tests;

struct Guard<R> {
    input: BufReader<R>,
    token: Vec<u8>,
    position: usize,
}
impl<R: Read> Guard<R> {
    fn new(input: R) -> Self {
        Self {
            input: BufReader::with_capacity(8192, input),
            token: Vec::with_capacity(TOKEN),
            position: 0,
        }
    }
    fn next_token(&mut self) -> io::Result<()> {
        self.token.clear();
        self.position = 0;
        let mut quote = None;
        while let Some(&byte) = self.input.fill_buf()?.first() {
            if !self.token.is_empty() && self.token[0] != b'<' && byte == b'<' {
                break;
            }
            if self.token.len() == TOKEN {
                return Err(io::Error::other(OfficeError::LimitExceeded(
                    crate::model::TOKEN_LIMIT,
                )));
            }
            if byte == 0 || (self.token.is_empty() && (byte == 0xff || byte == 0xfe)) {
                return Err(io::Error::other(OfficeError::Unsupported(UTF16_OR_BINARY)));
            }
            if byte < 0x20 && !b"\t\r\n".contains(&byte) {
                return Err(io::Error::other(OfficeError::Malformed(
                    "XML control character",
                )));
            }
            self.input.consume(1);
            self.token.push(byte);
            if self.token[0] != b'<' {
                continue;
            }
            // quick-xml's PI/declaration parser consumes through `?>`, including
            // unquoted `>` and apparent tags inside the PI. Never expose those
            // fragments separately: they belong to one bounded parser event.
            if self.token.starts_with(b"<?") {
                if self.token.ends_with(b"?>") {
                    break;
                }
                continue;
            }
            if self.token.starts_with(b"<!") {
                let prefix_ok = b"<!--".starts_with(&self.token)
                    || b"<![CDATA[".starts_with(&self.token)
                    || self.token.starts_with(b"<!--")
                    || self.token.starts_with(b"<![CDATA[");
                if !prefix_ok {
                    return Err(io::Error::other(OfficeError::Unsupported(DOCUMENT_TYPE)));
                }
                if (self.token.starts_with(b"<!--") && self.token.ends_with(b"-->"))
                    || (self.token.starts_with(b"<![CDATA[") && self.token.ends_with(b"]]>"))
                {
                    break;
                }
                continue;
            }
            if let Some(delimiter) = quote {
                if byte == delimiter {
                    quote = None;
                }
            } else if byte == b'\'' || byte == b'"' {
                quote = Some(byte);
            } else if byte == b'>' {
                break;
            }
        }
        if self.token.starts_with(b"<?") && !self.token.ends_with(b"?>") {
            return Err(io::Error::other(OfficeError::Malformed(
                "unterminated XML instruction",
            )));
        }
        std::str::from_utf8(&self.token)
            .map_err(|_| io::Error::other(OfficeError::Malformed("XML is not UTF-8")))?;
        if self.token.starts_with(b"<")
            && !self.token.starts_with(b"<!")
            && !self.token.starts_with(b"<?")
        {
            check_tag_names(&self.token)?;
        }
        Ok(())
    }
}

fn check_name(name: &[u8]) -> io::Result<()> {
    if name.len() > 256 {
        return Err(io::Error::other(OfficeError::LimitExceeded(
            "XML qualified name before parser allocation",
        )));
    }
    if name.is_empty()
        || name
            .iter()
            .any(|byte| *byte == b'/' || byte.is_ascii_whitespace())
    {
        return Err(io::Error::other(OfficeError::Malformed(
            "XML qualified name",
        )));
    }
    Ok(())
}

/// Bound the same complete names retained by quick-xml 0.41 before exposing a
/// token. Only the terminal `/>` is syntax; an internal slash is part of its
/// candidate name. Closing names include all text except trailing XML whitespace.
fn check_tag_names(token: &[u8]) -> io::Result<()> {
    if let Some(closing) = token.strip_prefix(b"</") {
        let name = closing.strip_suffix(b">").unwrap_or(closing);
        let length = name
            .iter()
            .rposition(|byte| !byte.is_ascii_whitespace())
            .map_or(0, |last| last + 1);
        return check_name(&name[..length]);
    }
    let content = token.strip_prefix(b"<").unwrap_or(token);
    let content = content
        .strip_suffix(b"/>")
        .or_else(|| content.strip_suffix(b">"))
        .unwrap_or(content);
    let length = content
        .iter()
        .position(u8::is_ascii_whitespace)
        .unwrap_or(content.len());
    check_name(&content[..length])?;
    let mut attributes = &content[length..];
    loop {
        let whitespace = attributes
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        attributes = &attributes[whitespace..];
        if attributes.is_empty() {
            return Ok(());
        }
        let length = attributes
            .iter()
            .position(|byte| byte.is_ascii_whitespace() || *byte == b'=')
            .unwrap_or(attributes.len());
        check_name(&attributes[..length])?;
        attributes = &attributes[length..];
        let whitespace = attributes
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        attributes = &attributes[whitespace..];
        attributes = attributes
            .strip_prefix(b"=")
            .ok_or_else(|| io::Error::other(OfficeError::Malformed("XML attribute assignment")))?;
        let whitespace = attributes
            .iter()
            .take_while(|byte| byte.is_ascii_whitespace())
            .count();
        attributes = &attributes[whitespace..];
        let (&quote, value) = attributes
            .split_first()
            .filter(|(quote, _)| matches!(quote, b'\'' | b'"'))
            .ok_or_else(|| io::Error::other(OfficeError::Malformed("XML attribute quoting")))?;
        let end = value
            .iter()
            .position(|byte| *byte == quote)
            .ok_or_else(|| io::Error::other(OfficeError::Malformed("XML attribute quoting")))?;
        attributes = &value[end + 1..];
    }
}
impl<R: Read> BufRead for Guard<R> {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        if self.position == self.token.len() {
            self.next_token()?;
        }
        Ok(&self.token[self.position..])
    }
    fn consume(&mut self, size: usize) {
        self.position = (self.position + size).min(self.token.len());
    }
}
impl<R: Read> Read for Guard<R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let input = self.fill_buf()?;
        let size = input.len().min(output.len());
        output[..size].copy_from_slice(&input[..size]);
        self.consume(size);
        Ok(size)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Namespace {
    None,
    Sheet,
    Word,
    Presentation,
    Drawing,
    OdfOffice,
    OdfText,
    OdfTable,
    OdfDraw,
    OdfPresentation,
    OdfManifest,
    PackageRelationships,
    Relationships,
    ContentTypes,
    Other,
}
fn namespace(value: ResolveResult<'_>) -> Result<Namespace, OfficeError> {
    Ok(match value {
        ResolveResult::Unbound => Namespace::None,
        ResolveResult::Unknown(_) => {
            return Err(OfficeError::Malformed("unbound XML namespace prefix"));
        }
        ResolveResult::Bound(value) => match value.as_ref() {
            b"http://schemas.openxmlformats.org/spreadsheetml/2006/main"
            | b"http://purl.oclc.org/ooxml/spreadsheetml/main" => Namespace::Sheet,
            b"http://schemas.openxmlformats.org/wordprocessingml/2006/main"
            | b"http://purl.oclc.org/ooxml/wordprocessingml/main" => Namespace::Word,
            b"http://schemas.openxmlformats.org/presentationml/2006/main"
            | b"http://purl.oclc.org/ooxml/presentationml/main" => Namespace::Presentation,
            b"http://schemas.openxmlformats.org/drawingml/2006/main"
            | b"http://purl.oclc.org/ooxml/drawingml/main" => Namespace::Drawing,
            b"urn:oasis:names:tc:opendocument:xmlns:office:1.0" => Namespace::OdfOffice,
            b"urn:oasis:names:tc:opendocument:xmlns:text:1.0" => Namespace::OdfText,
            b"urn:oasis:names:tc:opendocument:xmlns:table:1.0" => Namespace::OdfTable,
            b"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" => Namespace::OdfDraw,
            b"urn:oasis:names:tc:opendocument:xmlns:presentation:1.0" => Namespace::OdfPresentation,
            b"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" => Namespace::OdfManifest,
            b"http://schemas.openxmlformats.org/package/2006/relationships" => {
                Namespace::PackageRelationships
            }
            b"http://schemas.openxmlformats.org/officeDocument/2006/relationships"
            | b"http://purl.oclc.org/ooxml/officeDocument/relationships" => {
                Namespace::Relationships
            }
            b"http://schemas.openxmlformats.org/package/2006/content-types" => {
                Namespace::ContentTypes
            }
            _ => Namespace::Other,
        },
    })
}

pub(crate) struct Element {
    pub ns: Namespace,
    pub name: String,
    attrs: Vec<(Namespace, String, String)>,
}
impl Element {
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attr_ns(Namespace::None, name)
    }
    pub fn attr_ns(&self, ns: Namespace, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(a, b, _)| *a == ns && b == name)
            .map(|(_, _, v)| v.as_str())
    }
    pub fn required(&self, name: &str) -> Result<&str, OfficeError> {
        self.attr(name)
            .ok_or(OfficeError::Malformed("missing XML attribute"))
    }
    pub fn is(&self, ns: Namespace, name: &str) -> bool {
        self.ns == ns && self.name == name
    }
}
pub(crate) enum Token {
    Start(Element),
    End(Namespace, String),
    Text(String),
    Eof,
}
pub(crate) struct Xml<R> {
    reader: NsReader<Guard<R>>,
    buffer: Vec<u8>,
    depth: usize,
    roots: usize,
    namespace_bytes: usize,
    declaration_seen: bool,
}
impl<R: Read> Xml<R> {
    pub fn new(input: R) -> Self {
        let mut reader = NsReader::from_reader(Guard::new(input));
        reader.config_mut().expand_empty_elements = true;
        reader.config_mut().check_end_names = true;
        Self {
            reader,
            buffer: Vec::with_capacity(TOKEN),
            depth: 0,
            roots: 0,
            namespace_bytes: 0,
            declaration_seen: false,
        }
    }
    pub fn next(&mut self) -> Result<Token, OfficeError> {
        loop {
            self.buffer.clear();
            let event =
                self.reader
                    .read_event_into(&mut self.buffer)
                    .map_err(|error| match error {
                        quick_xml::Error::Io(error) => match ArcError::into_owned(error) {
                            Ok(error) => OfficeError::from(error),
                            Err(_) => OfficeError::Malformed("XML input failure"),
                        },
                        _ => OfficeError::Malformed("XML event"),
                    })?;
            match event {
                Event::Start(element) => {
                    self.depth += 1;
                    if self.depth > 128 {
                        return Err(OfficeError::LimitExceeded("XML depth"));
                    }
                    if self.depth == 1 {
                        self.roots += 1;
                    }
                    if self.roots != 1 {
                        return Err(OfficeError::Malformed("multiple XML roots"));
                    }
                    let (ns, local) = self.reader.resolver().resolve_element(element.name());
                    let ns = namespace(ns)?;
                    let name = utf8(local.as_ref())?.to_owned();
                    let mut attrs = Vec::new();
                    for attr in element.attributes() {
                        let attr = attr.map_err(|_| {
                            OfficeError::Malformed("duplicate or invalid XML attribute")
                        })?;
                        if attrs.len() >= 128 {
                            return Err(OfficeError::LimitExceeded("XML attributes"));
                        }
                        if attr.key.as_ref() == b"xmlns" || attr.key.as_ref().starts_with(b"xmlns:")
                        {
                            self.namespace_bytes += attr.key.as_ref().len() + attr.value.len();
                            if self.namespace_bytes > 256 * 1024 {
                                return Err(OfficeError::LimitExceeded(
                                    "XML namespace declarations",
                                ));
                            }
                            continue;
                        }
                        let (ns, local) = self.reader.resolver().resolve_attribute(attr.key);
                        let ns = namespace(ns)?;
                        if attr.key.as_ref().len() > 256 {
                            return Err(OfficeError::LimitExceeded("XML attribute name"));
                        }
                        let name = utf8(local.as_ref())?.to_owned();
                        if attrs.iter().any(|(a, b, _)| *a == ns && b == &name) {
                            return Err(OfficeError::Malformed("duplicate expanded XML attribute"));
                        }
                        let value = quick_xml::escape::unescape(utf8(&attr.value)?)
                            .map_err(|_| OfficeError::Malformed("XML attribute entity"))?
                            .into_owned();
                        attrs.push((ns, name, value));
                    }
                    return Ok(Token::Start(Element { ns, name, attrs }));
                }
                Event::End(element) => {
                    let (ns, local) = self.reader.resolver().resolve_element(element.name());
                    let ns = namespace(ns)?;
                    let name = utf8(local.as_ref())?.to_owned();
                    self.depth = self
                        .depth
                        .checked_sub(1)
                        .ok_or(OfficeError::Malformed("XML depth underflow"))?;
                    return Ok(Token::End(ns, name));
                }
                Event::Text(text) => {
                    let text = utf8(text.as_ref())?.to_owned();
                    if self.depth == 0 && !text.trim_matches('\u{feff}').trim().is_empty() {
                        return Err(OfficeError::Malformed("text outside XML root"));
                    }
                    return Ok(Token::Text(text));
                }
                Event::CData(text) => {
                    if self.depth == 0 {
                        return Err(OfficeError::Malformed("CDATA outside XML root"));
                    }
                    return Ok(Token::Text(utf8(text.as_ref())?.to_owned()));
                }
                Event::GeneralRef(reference) => {
                    let encoded = format!("&{};", utf8(reference.as_ref())?);
                    let value = quick_xml::escape::unescape(&encoded)
                        .map_err(|_| OfficeError::Malformed("XML entity reference"))?
                        .into_owned();
                    if self.depth == 0
                        || value
                            .chars()
                            .any(|c| c < ' ' && !['\t', '\r', '\n'].contains(&c))
                    {
                        return Err(OfficeError::Malformed("XML character reference"));
                    }
                    return Ok(Token::Text(value));
                }
                Event::Decl(declaration) => {
                    if self.declaration_seen || self.depth != 0 || self.roots != 0 {
                        return Err(OfficeError::Malformed("XML declaration placement"));
                    }
                    self.declaration_seen = true;
                    if let Some(encoding) = declaration.encoding() {
                        let encoding = encoding
                            .map_err(|_| OfficeError::Malformed("XML encoding declaration"))?;
                        if !encoding.eq_ignore_ascii_case(b"utf-8")
                            && !encoding.eq_ignore_ascii_case(b"us-ascii")
                        {
                            return Err(OfficeError::Unsupported(ENCODING));
                        }
                    }
                }
                Event::DocType(_) => return Err(OfficeError::Unsupported(DOCUMENT_TYPE)),
                Event::PI(_) => return Err(OfficeError::Unsupported(INSTRUCTION)),
                Event::Eof => {
                    if self.depth != 0 || self.roots != 1 {
                        return Err(OfficeError::Malformed("incomplete XML"));
                    }
                    return Ok(Token::Eof);
                }
                Event::Comment(_) => (),
                Event::Empty(_) => return Err(OfficeError::Malformed("unexpected unexpanded XML")),
            }
        }
    }
}
// quick-xml wraps I/O errors in Arc. No clone of opaque source errors is invented.
struct ArcError;
impl ArcError {
    fn into_owned(
        error: std::sync::Arc<io::Error>,
    ) -> Result<io::Error, std::sync::Arc<io::Error>> {
        std::sync::Arc::try_unwrap(error)
    }
}
fn utf8(value: &[u8]) -> Result<&str, OfficeError> {
    std::str::from_utf8(value).map_err(|_| OfficeError::Malformed("XML UTF-8"))
}
