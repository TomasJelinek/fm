//! WordprocessingML paragraph and table-row streaming, without layout evaluation.
use crate::{
    OfficeError,
    document::{Block, Collector, Row, Text, Window},
    package::Package,
    xml::{Namespace, Token},
};

pub(crate) fn load(package: &mut Package) -> Result<String, OfficeError> {
    crate::workbook::office_part(package, crate::workbook::WORDPROCESSING)
}

pub(crate) fn window(package: &mut Package, part: &str, start: u32) -> Result<Window, OfficeError> {
    let mut output = Collector::new(start)?;
    let mut xml = package.xml(part)?;
    let mut stack: Vec<(Namespace, String)> = Vec::new();
    let mut body_seen = false;
    let mut in_body = false;
    let mut table_depth = 0u32;
    let mut row: Option<Row> = None;
    let mut cell: Option<Text> = None;
    let mut paragraph: Option<Text> = None;
    let mut heading = 0;
    let mut list = false;
    let mut skip = None;
    loop {
        match xml.next()? {
            Token::Start(element) => {
                if stack.is_empty() && !element.is(Namespace::Word, "document") {
                    return Err(OfficeError::Malformed("document root"));
                }
                if skip.is_none() && in_body {
                    match (element.ns, element.name.as_str()) {
                        (Namespace::Word, "tbl") => {
                            table_depth += 1;
                            if table_depth > 1 {
                                output.omit();
                                skip = Some(stack.len());
                            }
                        }
                        (Namespace::Word, "tr") => {
                            if table_depth != 1 || row.is_some() {
                                return Err(OfficeError::Malformed("document table row structure"));
                            }
                            row = Some(output.row(1)?);
                        }
                        (Namespace::Word, "tc") => {
                            let cells = row
                                .as_ref()
                                .ok_or(OfficeError::Malformed("document cell outside row"))?;
                            if cell.is_some() {
                                return Err(OfficeError::Malformed("nested document cell"));
                            }
                            cell = Some(cells.text());
                        }
                        (Namespace::Word, "p") => {
                            if table_depth > 0 && cell.is_none() {
                                return Err(OfficeError::Malformed(
                                    "document table paragraph outside cell",
                                ));
                            }
                            if paragraph.is_some() {
                                return Err(OfficeError::Malformed("nested document paragraph"));
                            }
                            paragraph = Some(Text::new(output.selected() && table_depth == 0));
                            heading = 0;
                            list = false;
                        }
                        (Namespace::Word, "pStyle") => {
                            let style = element.attr_ns(Namespace::Word, "val").unwrap_or("");
                            heading = style
                                .strip_prefix("Heading")
                                .and_then(|v| v.parse::<u8>().ok())
                                .filter(|v| (1..=6).contains(v))
                                .unwrap_or(0);
                        }
                        (Namespace::Word, "numPr") => list = true,
                        (Namespace::Word, "tab" | "br" | "cr") => {
                            let value = if element.name == "tab" { "\t" } else { "\n" };
                            if let Some(text) = cell.as_mut().or(paragraph.as_mut()) {
                                text.push(value);
                            }
                        }
                        (
                            Namespace::Word,
                            "drawing" | "pict" | "object" | "altChunk" | "del" | "moveFrom",
                        ) => {
                            output.omit();
                            skip = Some(stack.len());
                        }
                        (Namespace::Word, _) => (),
                        _ => {
                            output.omit();
                            skip = Some(stack.len());
                        }
                    }
                }
                if element.is(Namespace::Word, "body") {
                    if body_seen || stack.len() != 1 {
                        return Err(OfficeError::Malformed("document body structure"));
                    }
                    body_seen = true;
                    in_body = true;
                }
                stack.push((element.ns, element.name));
            }
            Token::Text(value) => {
                if skip.is_none()
                    && stack
                        .last()
                        .is_some_and(|(ns, name)| *ns == Namespace::Word && name == "t")
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
                        if ns == Namespace::Word && name == "tbl" {
                            table_depth -= 1;
                        }
                    }
                    continue;
                }
                if ns != Namespace::Word {
                    continue;
                }
                match name.as_str() {
                    "p" if in_body => {
                        let text = paragraph
                            .take()
                            .ok_or(OfficeError::Malformed("document paragraph end"))?;
                        if let Some(cell) = &mut cell {
                            cell.push("\n");
                        } else if table_depth == 0
                            && !output.push(
                                Block::Paragraph {
                                    text: text.finish(),
                                    heading,
                                    list,
                                },
                                1,
                            )?
                        {
                            break;
                        }
                    }
                    "tc" => {
                        let text = cell
                            .take()
                            .ok_or(OfficeError::Malformed("document cell end"))?
                            .finish();
                        let cells = row
                            .as_mut()
                            .ok_or(OfficeError::Malformed("document row end"))?;
                        cells.append(&text, 1);
                    }
                    "tr" => {
                        let cells = row
                            .take()
                            .ok_or(OfficeError::Malformed("document row end"))?;
                        let keep_reading = match cells.finish() {
                            Some(block) => output.push(block, 1)?,
                            None => output.skip(1)?,
                        };
                        if !keep_reading {
                            break;
                        }
                    }
                    "tbl" => {
                        table_depth = table_depth
                            .checked_sub(1)
                            .ok_or(OfficeError::Malformed("document table nesting"))?;
                    }
                    "body" => in_body = false,
                    _ => (),
                }
            }
            Token::Eof => break,
        }
    }
    if !body_seen {
        return Err(OfficeError::Malformed("missing document body"));
    }
    output.finish()
}
