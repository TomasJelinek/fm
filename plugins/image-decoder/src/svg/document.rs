//! SVG source text: its size, encoding, document type declaration and XML syntax.
//!
//! Damage is reported as a fragment ("The SVG is not well-formed XML: ...") that the
//! viewer host places after "This image is damaged or incomplete:". A limit or an
//! unsupported construct is reported as a complete sentence starting with "This ",
//! which the host shows as it is.
use resvg::usvg::roxmltree;
use std::io::{self, Read};

/// Largest SVG source.
pub(super) const MAX_BYTES: u64 = 4 * 1024 * 1024;
/// Most XML nodes: elements, text runs (also whitespace between elements), comments and
/// processing instructions. A 4 MiB file of small elements stays below it.
pub(super) const MAX_NODES: u32 = 262_144;
pub(super) const SVG_NAMESPACE: &str = "http://www.w3.org/2000/svg";

/// Read the whole source as UTF-8 text, refusing empty, non-markup and oversized input
/// before it is fully read.
pub(super) fn read(reader: impl Read) -> io::Result<String> {
    let mut reader = reader.take(MAX_BYTES + 1);
    let mut bytes = Vec::new();
    // No raster signature matched. Say so for empty or non-markup content before
    // reading up to 4 MiB of it as SVG text: the reason reaches the viewer's status.
    (&mut reader).take(64).read_to_end(&mut bytes)?;
    if bytes.is_empty() {
        return Err(invalid("The file is empty".into()));
    }
    if !markup_prefix(&bytes) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "The content is not a recognized image format",
        ));
    }
    reader.read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(limit(
            "This SVG is larger than 4 MiB, the limit for SVG images.".into(),
        ));
    }
    String::from_utf8(bytes).map_err(|error| {
        let valid = &error.as_bytes()[..error.utf8_error().valid_up_to()];
        let line = 1 + valid.iter().filter(|byte| **byte == b'\n').count();
        invalid(format!("The SVG text is not valid UTF-8 (line {line})"))
    })
}

/// Whether a content prefix can start SVG text: after an optional UTF-8 byte order
/// mark and whitespace, the first byte is `<`. A prefix of only whitespace is undecided.
fn markup_prefix(prefix: &[u8]) -> bool {
    let text = prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix);
    text.iter()
        .find(|byte| !byte.is_ascii_whitespace())
        .is_none_or(|byte| *byte == b'<')
}

/// Parse bounded XML whose root is an SVG `svg` element. A document type declaration
/// without an internal subset (`<!DOCTYPE svg PUBLIC "..." "...">`, as many editors
/// write) is blanked out first, so the parser never processes a DTD; an internal
/// subset, which is where XML entities are declared, is refused.
pub(super) fn parse(text: &mut String) -> io::Result<roxmltree::Document<'_>> {
    blank_doctype(text)?;
    let text = text.trim_start_matches('\u{feff}');
    let document = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: MAX_NODES,
            entity_resolver: None,
        },
    )
    .map_err(|error| xml_error(text, &error))?;
    let root = document.root_element();
    if root.tag_name().name() != "svg" {
        return Err(statement(
            "This XML document is not an SVG image: its root element is not svg.".into(),
        ));
    }
    if root.tag_name().namespace() != Some(SVG_NAMESPACE) {
        return Err(statement(
            "This SVG does not declare the SVG namespace on its root element, so it is not \
             a standalone SVG image."
                .into(),
        ));
    }
    Ok(document)
}

/// Replace a document type declaration that has no internal subset by spaces, keeping
/// line breaks so that later error positions stay right. The prolog grammar matches the
/// XML parser's: an optional declaration, then comments, processing instructions and
/// whitespace. If this scanner and the parser ever disagree, the parser still refuses
/// the DTD (it runs with DTDs disallowed), so entities are never expanded.
fn blank_doctype(text: &mut String) -> io::Result<()> {
    let Some((start, end)) = doctype(text)? else {
        return Ok(());
    };
    // Each character becomes one space, so later lines and columns keep their numbers.
    let blanked: String = text[start..end]
        .chars()
        .map(|character| {
            if matches!(character, '\n' | '\r') {
                character
            } else {
                ' '
            }
        })
        .collect();
    text.replace_range(start..end, &blanked);
    Ok(())
}

/// The byte span of a document type declaration without an internal subset.
fn doctype(text: &str) -> io::Result<Option<(usize, usize)>> {
    let bytes = text.as_bytes();
    let mut position = if text.starts_with('\u{feff}') { 3 } else { 0 };
    let skip_space = |position: &mut usize| {
        while bytes.get(*position).is_some_and(u8::is_ascii_whitespace) {
            *position += 1;
        }
    };
    let skip_past = |position: &mut usize, marker: &str| -> bool {
        match text[*position..].find(marker) {
            Some(offset) => {
                *position += offset + marker.len();
                true
            }
            None => false,
        }
    };
    loop {
        skip_space(&mut position);
        let rest = &text[position..];
        if rest.starts_with("<?") {
            if !skip_past(&mut position, "?>") {
                return Ok(None);
            }
        } else if rest.starts_with("<!--") {
            if !skip_past(&mut position, "-->") {
                return Ok(None);
            }
        } else if rest.starts_with("<!DOCTYPE") {
            break;
        } else {
            return Ok(None);
        }
    }
    let start = position;
    position += "<!DOCTYPE".len();
    // Name, external identifier and their literals, up to `[` or `>`.
    let mut quote = None;
    while let Some(&byte) = bytes.get(position) {
        match (quote, byte) {
            (Some(open), _) if byte == open => quote = None,
            (Some(_), _) => {}
            (None, b'"' | b'\'') => quote = Some(byte),
            (None, b'[') => {
                return Err(statement(
                    "This SVG declares XML entities (a DOCTYPE internal subset), which are not \
                     supported."
                        .into(),
                ));
            }
            (None, b'>') => return Ok(Some((start, position + 1))),
            (None, _) => {}
        }
        position += 1;
    }
    // Unterminated: leave it to the parser, which refuses it.
    Ok(None)
}

/// A not well-formed document, named by its cause and position. Causes never quote the
/// document: names and values from it could be long or misleading in a status line.
fn xml_error(text: &str, error: &roxmltree::Error) -> io::Error {
    use roxmltree::Error as E;
    let cause = match error {
        E::NodesLimitReached => {
            return limit(format!(
                "This SVG has more than {MAX_NODES} XML nodes (elements, text runs and \
                 comments), the limit for SVG images."
            ));
        }
        E::AttributesLimitReached | E::NamespacesLimitReached => {
            return limit("This SVG has more attributes or namespaces than XML allows.".into());
        }
        E::DtdDetected => "its document type declaration cannot be read",
        E::UnexpectedEndOfStream | E::UnclosedRootNode => "the document ends too early",
        E::UnexpectedCloseTag(..) | E::UnexpectedEntityCloseTag(_) => {
            "a closing tag does not match the open element"
        }
        E::InvalidAttributeValue(_) => "an attribute value contains '<'",
        E::DuplicatedAttribute(..) => "an element repeats an attribute",
        E::UnknownEntityReference(..) => "it refers to an undefined entity",
        E::MalformedEntityReference(_) => "an '&' does not start a valid reference",
        E::EntityReferenceLoop(_) => "its entities refer to each other",
        E::UnknownNamespace(..) => "it uses an undeclared namespace prefix",
        E::DuplicatedNamespace(..) => "an element declares a namespace twice",
        E::InvalidXmlPrefixUri(_)
        | E::UnexpectedXmlUri(_)
        | E::UnexpectedXmlnsUri(_)
        | E::InvalidElementNamePrefix(_) => "it misuses a reserved namespace",
        E::NonXmlChar(..) => "it contains a character that XML does not allow",
        E::InvalidComment(_) => "a comment contains '--'",
        E::InvalidCharacterData(_) => "character data contains ']]>'",
        E::UnexpectedDeclaration(_) => "the XML declaration is not at the very start",
        E::NoRootNode => "it has no root element",
        E::InvalidName(_) => "a tag or attribute name is invalid",
        _ => "the markup is invalid",
    };
    let position = match error {
        E::UnexpectedEndOfStream | E::UnclosedRootNode => Some(end_position(text)),
        E::NoRootNode | E::DtdDetected => None,
        _ => {
            let position = error.pos();
            Some((position.row, position.col))
        }
    };
    invalid(match position {
        Some((line, column)) => {
            format!("The SVG is not well-formed XML: {cause} (line {line}, column {column})")
        }
        None => format!("The SVG is not well-formed XML: {cause}"),
    })
}

fn end_position(text: &str) -> (u32, u32) {
    let line = 1 + text.bytes().filter(|byte| *byte == b'\n').count();
    let column = 1 + text
        .rsplit('\n')
        .next()
        .map_or(0, |last| last.chars().count());
    (line as u32, column as u32)
}

/// Damaged or incomplete content: the viewer host prefixes its own sentence.
pub(super) fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// A refusal that names a limit and its bound, as a complete sentence ("This SVG ...").
pub(super) fn limit(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::FileTooLarge, message)
}

/// A refusal of content this decoder does not support, as a complete sentence.
pub(super) fn statement(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, message)
}
