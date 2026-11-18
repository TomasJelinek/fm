//! Text/table extraction from one slide. No drawing geometry or external resources.
use crate::{
    OfficeError, SlideId,
    document::{Block, Collector, Row, Text, Window},
    odt::{repeat, scalar, spaces},
    xml::{Namespace, Token, Xml},
};
use std::io::Read;
#[derive(Clone, Copy)]
pub(crate) enum Dialect {
    Drawing,
    Odf(SlideId),
}

pub(crate) fn scan<R: Read>(
    mut xml: Xml<R>,
    dialect: Dialect,
    start: u32,
) -> Result<Window, OfficeError> {
    let mut output = Collector::new(start)?;
    let mut stack: Vec<(Namespace, String)> = Vec::new();
    let odf = matches!(dialect, Dialect::Odf(_));
    let text_ns = if odf {
        Namespace::OdfText
    } else {
        Namespace::Drawing
    };
    let table_ns = if odf {
        Namespace::OdfTable
    } else {
        Namespace::Drawing
    };
    let mut active = None;
    let mut pages = 0usize;
    let mut table = false;
    let mut row: Option<Row> = None;
    let mut row_repeat = 1;
    let mut cell_repeat = 1;
    let mut logical_columns = 0u32;
    let mut cell: Option<Text> = None;
    let mut cell_paragraph = false;
    let mut paragraph: Option<Text> = None;
    let mut skip = None;
    loop {
        match xml.next()? {
            Token::Start(element) => {
                if stack.is_empty() {
                    let valid = if odf {
                        element.is(Namespace::OdfOffice, "document-content")
                    } else {
                        element.is(Namespace::Presentation, "sld")
                    };
                    if !valid {
                        return Err(OfficeError::Unsupported("slide document root"));
                    }
                    if !odf {
                        active = Some(0);
                    }
                }
                if let Dialect::Odf(slide) = dialect
                    && element.is(Namespace::OdfDraw, "page")
                    && stack
                        .last()
                        .is_some_and(|(ns, n)| *ns == Namespace::OdfOffice && n == "presentation")
                {
                    if pages == usize::from(slide.0) {
                        active = Some(stack.len());
                    }
                    pages += 1;
                }
                if active.is_some() && skip.is_none() {
                    if element.ns == text_ns && element.name == "p" {
                        if paragraph.is_some() {
                            return Err(OfficeError::Malformed("nested slide paragraph"));
                        }
                        if table && cell.is_none() {
                            return Err(OfficeError::Malformed(
                                "slide table paragraph outside cell",
                            ));
                        }
                        if !cell_paragraph && let Some(text) = &mut cell {
                            text.clear();
                            cell_paragraph = true;
                        }
                        paragraph = Some(Text::new(output.selected() && !table));
                    } else if element.ns == table_ns
                        && element.name == if odf { "table" } else { "tbl" }
                    {
                        if table {
                            output.omit();
                            skip = Some(stack.len());
                        } else {
                            table = true;
                        }
                    } else if element.ns == table_ns
                        && element.name == if odf { "table-row" } else { "tr" }
                    {
                        if !table || row.is_some() {
                            return Err(OfficeError::Malformed("slide row structure"));
                        }
                        logical_columns = 0;
                        row_repeat = if odf {
                            repeat(&element, Namespace::OdfTable, "number-rows-repeated")?
                        } else {
                            1
                        };
                        row = Some(output.row(row_repeat)?);
                    } else if element.ns == table_ns
                        && (element.name == if odf { "table-cell" } else { "tc" }
                            || odf && element.name == "covered-table-cell")
                    {
                        let cells = row
                            .as_ref()
                            .ok_or(OfficeError::Malformed("slide cell outside row"))?;
                        if cell.is_some() {
                            return Err(OfficeError::Malformed("nested slide cell"));
                        }
                        cell_repeat = if odf {
                            repeat(&element, Namespace::OdfTable, "number-columns-repeated")?
                        } else {
                            1
                        };
                        logical_columns = logical_columns
                            .checked_add(cell_repeat)
                            .filter(|count| *count <= 16_384)
                            .ok_or(OfficeError::LimitExceeded("slide table logical columns"))?;
                        cell = Some(cells.text());
                        if odf && let Some(text) = &mut cell {
                            scalar(&element, text);
                        }
                        cell_paragraph = false;
                    } else if odf && element.is(Namespace::OdfText, "s") {
                        let count = spaces(&element)?;
                        if let Some(text) = cell.as_mut().or(paragraph.as_mut()) {
                            text.spaces(count);
                        }
                    } else if (odf
                        && element.ns == Namespace::OdfText
                        && ["tab", "line-break"].contains(&element.name.as_str()))
                        || (!odf && element.is(Namespace::Drawing, "br"))
                    {
                        if let Some(text) = cell.as_mut().or(paragraph.as_mut()) {
                            text.push(if element.name == "tab" { "\t" } else { "\n" });
                        }
                    } else if unsupported(element.ns, &element.name, odf)
                        || (!odf
                            && element.is(Namespace::Drawing, "graphicData")
                            && !matches!(
                                element.attr("uri"),
                                Some(
                                    "http://schemas.openxmlformats.org/drawingml/2006/table"
                                        | "http://purl.oclc.org/ooxml/drawingml/table"
                                )
                            ))
                    {
                        output.omit();
                        skip = Some(stack.len());
                    }
                }
                stack.push((element.ns, element.name));
            }
            Token::Text(value) => {
                let drawing_text = !odf
                    && stack
                        .last()
                        .is_some_and(|(ns, n)| *ns == Namespace::Drawing && n == "t");
                if active.is_some()
                    && skip.is_none()
                    && paragraph.is_some()
                    && (odf || drawing_text)
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
                if active.is_none() {
                    continue;
                }
                if ns == text_ns && name == "p" {
                    let text = paragraph
                        .take()
                        .ok_or(OfficeError::Malformed("slide paragraph end"))?;
                    if let Some(cell) = &mut cell {
                        cell.push("\n");
                    } else if !table
                        && !output.push(
                            Block::Paragraph {
                                text: text.finish(),
                                heading: 0,
                                list: false,
                            },
                            1,
                        )?
                    {
                        break;
                    }
                } else if ns == table_ns
                    && (name == if odf { "table-cell" } else { "tc" }
                        || odf && name == "covered-table-cell")
                {
                    let value = cell
                        .take()
                        .ok_or(OfficeError::Malformed("slide cell end"))?
                        .finish();
                    let cells = row.as_mut().ok_or(OfficeError::Malformed("slide row"))?;
                    cells.append(&value, cell_repeat);
                } else if ns == table_ns && name == if odf { "table-row" } else { "tr" } {
                    let cells = row.take().ok_or(OfficeError::Malformed("slide row end"))?;
                    let keep_reading = match cells.finish() {
                        Some(block) => output.push(block, row_repeat)?,
                        None => output.skip(row_repeat)?,
                    };
                    if !keep_reading {
                        break;
                    }
                } else if ns == table_ns && name == if odf { "table" } else { "tbl" } {
                    table = false;
                }
                if active == Some(stack.len()) {
                    break;
                }
            }
            Token::Eof => break,
        }
    }
    if active.is_none() {
        return Err(OfficeError::InvalidTarget);
    }
    output.finish()
}
fn unsupported(ns: Namespace, name: &str, odf: bool) -> bool {
    if odf {
        (ns == Namespace::OdfDraw
            && ["image", "object", "object-ole", "plugin", "applet"].contains(&name))
            || (ns == Namespace::OdfOffice && name == "scripts")
            || ns == Namespace::Other
    } else {
        (ns == Namespace::Presentation && ["pic", "oleObj", "contentPart"].contains(&name))
            || (ns == Namespace::Drawing && ["blip", "videoFile", "audioFile"].contains(&name))
            || ns == Namespace::Other
    }
}
