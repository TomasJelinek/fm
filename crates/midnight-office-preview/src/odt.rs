//! ODF manifest validation and bounded paragraph/table-row document preview.
use crate::{
    OfficeError,
    document::{Block, Collector, Row, Text, Window},
    package::Package,
    xml::{Element, Namespace, Token},
};

pub(crate) fn load(package: &mut Package, kind: &str) -> Result<(), OfficeError> {
    let expected = format!("application/vnd.oasis.opendocument.{kind}");
    if package.read_small("mimetype", 256)? != expected.as_bytes() {
        return Err(OfficeError::Unsupported("ODF media type"));
    }
    let mut xml = package.xml("META-INF/manifest.xml")?;
    let mut root = false;
    let mut content = false;
    let mut depth = 0;
    let mut entries = std::collections::BTreeSet::new();
    let mut retained = 0usize;
    loop {
        match xml.next()? {
            Token::Start(element) => {
                depth += 1;
                if depth == 1 && !element.is(Namespace::OdfManifest, "manifest") {
                    return Err(OfficeError::Malformed("ODF manifest root"));
                }
                if element.is(Namespace::OdfManifest, "encryption-data") {
                    return Err(OfficeError::Encrypted);
                }
                if element.is(Namespace::OdfManifest, "file-entry") {
                    if depth != 2 {
                        return Err(OfficeError::Malformed("ODF manifest entry structure"));
                    }
                    let path = required(&element, Namespace::OdfManifest, "full-path")?;
                    let media = required(&element, Namespace::OdfManifest, "media-type")?;
                    retained = retained
                        .checked_add(path.len())
                        .filter(|n| *n <= 1024 * 1024)
                        .ok_or(OfficeError::LimitExceeded("ODF manifest descriptors"))?;
                    if entries.len() >= 10_000 || path.len() > 4096 {
                        return Err(OfficeError::LimitExceeded("ODF manifest entries"));
                    }
                    if !entries.insert(path.to_owned()) {
                        return Err(OfficeError::Malformed("duplicate ODF manifest entry"));
                    }
                    if path == "/" {
                        if media != expected {
                            return Err(OfficeError::Unsupported("ODF root media type"));
                        }
                        root = true;
                    }
                    if path == "content.xml" {
                        if media != "text/xml" {
                            return Err(OfficeError::Unsupported("ODF content media type"));
                        }
                        content = true;
                    }
                    if path.starts_with("Basic/") || path.starts_with("Scripts/") {
                        return Err(OfficeError::Unsupported("ODF scripts"));
                    }
                }
            }
            Token::End(_, _) => depth -= 1,
            Token::Eof => break,
            Token::Text(_) => (),
        }
    }
    if !root || !content {
        return Err(OfficeError::Malformed("ODF manifest membership"));
    }
    drop(xml);
    if !package.contains("content.xml") {
        return Err(OfficeError::Malformed("missing ODF content"));
    }
    Ok(())
}

pub(crate) fn required<'a>(
    element: &'a Element,
    ns: Namespace,
    name: &str,
) -> Result<&'a str, OfficeError> {
    element
        .attr_ns(ns, name)
        .ok_or(OfficeError::Malformed("missing ODF attribute"))
}
pub(crate) fn repeat(element: &Element, ns: Namespace, name: &str) -> Result<u32, OfficeError> {
    let Some(value) = element.attr_ns(ns, name) else {
        return Ok(1);
    };
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(OfficeError::Malformed("ODF repeat count"));
    }
    value
        .parse::<u32>()
        .ok()
        .filter(|n| *n > 0)
        .ok_or(OfficeError::LimitExceeded("ODF repeat count"))
}

/// ODF text:s count defaults to one but explicitly permits zero.
pub(crate) fn spaces(element: &Element) -> Result<u32, OfficeError> {
    let Some(value) = element.attr_ns(Namespace::OdfText, "c") else {
        return Ok(1);
    };
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(OfficeError::Malformed("ODF space count"));
    }
    value
        .parse::<u32>()
        .map_err(|_| OfficeError::LimitExceeded("ODF space count"))
}

pub(crate) fn scalar(element: &Element, text: &mut Text) {
    let value = [
        "string-value",
        "value",
        "boolean-value",
        "date-value",
        "time-value",
    ]
    .into_iter()
    .find_map(|name| element.attr_ns(Namespace::OdfOffice, name));
    if let Some(value) = value {
        text.push(value);
    } else if let Some(formula) = element.attr_ns(Namespace::OdfTable, "formula") {
        text.push("[formula] ");
        text.push(formula);
    }
}

pub(crate) fn window(package: &mut Package, start: u32) -> Result<Window, OfficeError> {
    let mut output = Collector::new(start)?;
    let mut xml = package.xml("content.xml")?;
    let mut stack: Vec<(Namespace, String)> = Vec::new();
    let mut body_seen = false;
    let mut in_text = false;
    let mut table = false;
    let mut row: Option<Row> = None;
    let mut row_repeat = 1;
    let mut cell: Option<Text> = None;
    let mut cell_paragraph = false;
    let mut cell_repeat = 1;
    let mut logical_columns = 0u32;
    let mut paragraph: Option<Text> = None;
    let mut heading = 0;
    let mut skip = None;
    loop {
        match xml.next()? {
            Token::Start(element) => {
                if stack.is_empty() && !element.is(Namespace::OdfOffice, "document-content") {
                    return Err(OfficeError::Unsupported("not an ODF contents document"));
                }
                if element.is(Namespace::OdfOffice, "text") {
                    if body_seen
                        || stack
                            .last()
                            .is_none_or(|(ns, n)| *ns != Namespace::OdfOffice || n != "body")
                    {
                        return Err(OfficeError::Malformed("ODT text body"));
                    }
                    body_seen = true;
                    in_text = true;
                } else if in_text && skip.is_none() {
                    match (element.ns, element.name.as_str()) {
                        (Namespace::OdfText, "p" | "h") => {
                            if paragraph.is_some() {
                                return Err(OfficeError::Malformed("nested ODF paragraph"));
                            }
                            if table && cell.is_none() {
                                return Err(OfficeError::Malformed(
                                    "ODF table paragraph outside cell",
                                ));
                            }
                            if !cell_paragraph && let Some(text) = &mut cell {
                                text.clear();
                                cell_paragraph = true;
                            }
                            paragraph = Some(Text::new(output.selected() && !table));
                            heading = if element.name == "h" {
                                element
                                    .attr_ns(Namespace::OdfText, "outline-level")
                                    .and_then(|s| s.parse::<u8>().ok())
                                    .unwrap_or(1)
                                    .clamp(1, 6)
                            } else {
                                0
                            };
                        }
                        (Namespace::OdfText, "s") => {
                            let count = spaces(&element)?;
                            if let Some(text) = cell.as_mut().or(paragraph.as_mut()) {
                                text.spaces(count);
                            }
                        }
                        (Namespace::OdfText, "tab" | "line-break") => {
                            if let Some(text) = cell.as_mut().or(paragraph.as_mut()) {
                                text.push(if element.name == "tab" { "\t" } else { "\n" });
                            }
                        }
                        (Namespace::OdfTable, "table") => {
                            if table {
                                output.omit();
                                skip = Some(stack.len());
                            } else {
                                table = true;
                            }
                        }
                        (Namespace::OdfTable, "table-row") => {
                            if !table || row.is_some() {
                                return Err(OfficeError::Malformed("ODF row structure"));
                            }
                            logical_columns = 0;
                            row_repeat =
                                repeat(&element, Namespace::OdfTable, "number-rows-repeated")?;
                            row = Some(output.row(row_repeat)?);
                        }
                        (Namespace::OdfTable, "table-cell" | "covered-table-cell") => {
                            let cells = row
                                .as_ref()
                                .ok_or(OfficeError::Malformed("ODF cell outside row"))?;
                            if cell.is_some() {
                                return Err(OfficeError::Malformed("nested ODF cell"));
                            }
                            cell_repeat =
                                repeat(&element, Namespace::OdfTable, "number-columns-repeated")?;
                            logical_columns = logical_columns
                                .checked_add(cell_repeat)
                                .filter(|count| *count <= 16_384)
                                .ok_or(OfficeError::LimitExceeded("ODF table logical columns"))?;
                            cell = Some(cells.text());
                            if let Some(text) = &mut cell {
                                scalar(&element, text);
                            }
                            cell_paragraph = false;
                        }
                        (
                            Namespace::OdfText,
                            "list" | "list-item" | "list-header" | "span" | "a" | "bookmark"
                            | "bookmark-start" | "bookmark-end" | "soft-page-break" | "section",
                        ) => (),
                        (
                            Namespace::OdfTable,
                            "table-column"
                            | "table-header-rows"
                            | "table-rows"
                            | "table-row-group"
                            | "table-columns"
                            | "table-header-columns",
                        ) => (),
                        _ => {
                            output.omit();
                            skip = Some(stack.len());
                        }
                    }
                }
                stack.push((element.ns, element.name));
            }
            Token::Text(value) => {
                if in_text
                    && skip.is_none()
                    && paragraph.is_some()
                    && let Some(text) = cell.as_mut().or(paragraph.as_mut())
                {
                    text.push(&value);
                }
            }
            Token::End(ns, name) => {
                stack.pop();
                if let Some(depth) = skip {
                    if stack.len() == depth {
                        skip = None;
                    }
                    continue;
                }
                match (ns, name.as_str()) {
                    (Namespace::OdfText, "p" | "h") if in_text => {
                        let text = paragraph
                            .take()
                            .ok_or(OfficeError::Malformed("ODF paragraph end"))?;
                        if let Some(cell) = &mut cell {
                            cell.push("\n");
                        } else if !table {
                            let list = stack
                                .iter()
                                .any(|(ns, n)| *ns == Namespace::OdfText && n == "list-item");
                            if !output.push(
                                Block::Paragraph {
                                    text: text.finish(),
                                    heading,
                                    list,
                                },
                                1,
                            )? {
                                break;
                            }
                        }
                    }
                    (Namespace::OdfTable, "table-cell" | "covered-table-cell") if in_text => {
                        let value = cell
                            .take()
                            .ok_or(OfficeError::Malformed("ODF cell end"))?
                            .finish();
                        let cells = row.as_mut().ok_or(OfficeError::Malformed("ODF row"))?;
                        cells.append(&value, cell_repeat);
                    }
                    (Namespace::OdfTable, "table-row") if in_text => {
                        let cells = row.take().ok_or(OfficeError::Malformed("ODF row end"))?;
                        let keep_reading = match cells.finish() {
                            Some(block) => output.push(block, row_repeat)?,
                            None => output.skip(row_repeat)?,
                        };
                        if !keep_reading {
                            break;
                        }
                    }
                    (Namespace::OdfTable, "table") if in_text => table = false,
                    (Namespace::OdfOffice, "text") => in_text = false,
                    _ => (),
                }
            }
            Token::Eof => break,
        }
    }
    if !body_seen {
        return Err(OfficeError::Malformed("missing ODT body"));
    }
    output.finish()
}
