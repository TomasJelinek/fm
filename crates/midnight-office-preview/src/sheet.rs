use crate::{
    OfficeError, OfficeTarget,
    model::WINDOW_TEXT_LIMIT,
    package::Package,
    xml::{Namespace, Token},
};
use std::collections::{BTreeMap, BTreeSet};

const CELL_BYTES: usize = 32 * 1024;
const WINDOW_BYTES: usize = 512 * 1024;
const TRUNCATION: &str = " [truncated]";

pub(crate) struct Cell {
    pub row: u32,
    pub column: u16,
    pub text: String,
}
pub(crate) struct Window {
    pub row: u32,
    pub column: u16,
    pub cells: Vec<Cell>,
    pub next: Option<(u32, u16)>,
    /// The sheet holds charts, pictures, comments or other contents not shown here.
    pub omitted: bool,
}
struct Pending {
    row: u32,
    column: u16,
    kind: String,
    value: Text,
    formula: Text,
    inline: Text,
    selected: bool,
    has_value: bool,
    has_formula: bool,
    has_inline: bool,
}
#[derive(Default)]
struct Text {
    text: String,
    truncated: bool,
}
impl Text {
    fn push(&mut self, text: &str) {
        if self.truncated {
            return;
        }
        let available = (CELL_BYTES - TRUNCATION.len()).saturating_sub(self.text.len());
        let mut length = text.len().min(available);
        while !text.is_char_boundary(length) {
            length -= 1;
        }
        self.text.push_str(&text[..length]);
        if length != text.len() {
            self.truncated = true;
        }
    }
    fn finish(mut self) -> String {
        if self.truncated {
            self.text.push_str(TRUNCATION);
        }
        self.text
    }
}

/// Cells of one worksheet window. Only worksheet/sheetData/row/c and the cell's
/// value, formula and inline text are read; views, formatting, print settings and
/// extension lists are skipped whole. Objects anchored on the sheet and unknown
/// markup in the cell path are skipped too, and the page says so.
pub(crate) fn load(
    package: &mut Package,
    part: &str,
    shared: Option<&str>,
    target: OfficeTarget,
) -> Result<Window, OfficeError> {
    let mut anchor = match target {
        OfficeTarget::SheetFirst { .. } => None,
        OfficeTarget::SheetWindow { row, column, .. } => Some((row, column)),
        _ => return Err(OfficeError::InvalidTarget),
    };
    let mut xml = package.xml(part)?;
    let mut cells = Vec::new();
    let mut selected_ids = BTreeMap::new();
    let mut next = None;
    let mut pending: Option<Pending> = None;
    let mut stack: Vec<String> = Vec::new();
    let mut skipped = 0usize;
    let mut omitted = false;
    let mut previous = None;
    let mut row_number = None;
    let mut text_bytes = 0usize;
    let mut sheet_data = false;
    loop {
        match xml.next()? {
            Token::Start(element) => {
                if skipped > 0 {
                    skipped += 1;
                    continue;
                }
                let Some(parent) = stack.last().map(String::as_str) else {
                    if !element.is(Namespace::Sheet, "worksheet") {
                        return Err(OfficeError::Malformed("worksheet root"));
                    }
                    stack.push(element.name);
                    continue;
                };
                let name = (element.ns == Namespace::Sheet).then_some(element.name.as_str());
                match (parent, name) {
                    ("worksheet", Some("sheetData")) => {
                        if sheet_data {
                            return Err(OfficeError::Malformed("worksheet sheetData structure"));
                        }
                        sheet_data = true;
                    }
                    ("sheetData", Some("row")) => {
                        row_number = element
                            .attr("r")
                            .map(|r| {
                                r.parse::<u32>()
                                    .map_err(|_| OfficeError::Malformed("row coordinate"))
                            })
                            .transpose()?;
                        if row_number.is_some_and(|r| r == 0 || r > 1_048_576) {
                            return Err(OfficeError::Malformed("row outside worksheet"));
                        }
                    }
                    ("row", Some("c")) => {
                        if pending.is_some() {
                            return Err(OfficeError::Malformed("worksheet cell structure"));
                        }
                        let (row, column) = coordinate(element.required("r")?)?;
                        if row_number.is_some_and(|r| r != row)
                            || previous.is_some_and(|value| value >= (row, column))
                        {
                            return Err(OfficeError::Malformed(
                                "duplicate/unordered worksheet coordinates",
                            ));
                        }
                        previous = Some((row, column));
                        let kind = element.attr("t").unwrap_or("n");
                        if !["n", "s", "str", "inlineStr", "b", "e", "d"].contains(&kind) {
                            return Err(OfficeError::Malformed("cell value type"));
                        }
                        let selected = anchor.is_none_or(|(r, c)| inside(row, column, r, c));
                        pending = Some(Pending {
                            row,
                            column,
                            kind: kind.to_owned(),
                            value: Text::default(),
                            formula: Text::default(),
                            inline: Text::default(),
                            selected,
                            has_value: false,
                            has_formula: false,
                            has_inline: false,
                        });
                    }
                    ("c", Some("v" | "f" | "is")) => {
                        let cell = pending
                            .as_mut()
                            .ok_or(OfficeError::Malformed("cell value outside cell"))?;
                        let seen = match element.name.as_str() {
                            "v" => &mut cell.has_value,
                            "f" => &mut cell.has_formula,
                            _ => &mut cell.has_inline,
                        };
                        if *seen {
                            return Err(OfficeError::Malformed("duplicate cell value element"));
                        }
                        *seen = true;
                    }
                    ("is" | "r", Some("t")) | ("is", Some("r")) => (),
                    // Run formatting, phonetic runs (never cell text) and extensions.
                    ("row" | "c" | "is" | "r", Some("extLst"))
                    | ("is" | "r", Some("rPr" | "rPh" | "phoneticPr")) => {
                        skipped = 1;
                        continue;
                    }
                    // Charts, pictures, comments, form controls and OLE objects.
                    (
                        "worksheet",
                        Some(
                            "drawing" | "legacyDrawing" | "legacyDrawingHF" | "picture"
                            | "oleObjects" | "controls",
                        ),
                    ) => {
                        omitted = true;
                        skipped = 1;
                        continue;
                    }
                    // Excel wraps form controls and OLE objects in compatibility markup.
                    ("worksheet", None) if element.name == "AlternateContent" => {
                        omitted = true;
                        skipped = 1;
                        continue;
                    }
                    // Views, formatting, protection, filters, print settings, tables and
                    // extension lists hold no cell contents.
                    ("worksheet", _) => {
                        skipped = 1;
                        continue;
                    }
                    // Unknown markup in the cell path is never read as cell text.
                    _ => {
                        omitted = true;
                        skipped = 1;
                        continue;
                    }
                }
                stack.push(element.name);
            }
            Token::Text(text) => {
                if skipped > 0 {
                    continue;
                }
                if let Some(cell) = pending.as_mut() {
                    // Outside-window values only need a bounded populated marker.
                    let value = if cell.selected {
                        text.as_str()
                    } else if text.is_empty() {
                        ""
                    } else {
                        "x"
                    };
                    match stack.last().map(String::as_str) {
                        Some("v") => cell.value.push(value),
                        Some("f") => cell.formula.push(value),
                        Some("t") => cell.inline.push(value),
                        _ => (),
                    }
                }
            }
            Token::End(ns, name) => {
                if skipped > 0 {
                    skipped -= 1;
                    continue;
                }
                if ns == Namespace::Sheet && name == "c" {
                    let cell = pending.take().ok_or(OfficeError::Malformed("cell end"))?;
                    let populated = !cell.value.text.is_empty()
                        || !cell.inline.text.is_empty()
                        || cell.has_formula;
                    if populated {
                        let (start_row, start_column) =
                            *anchor.get_or_insert((cell.row, cell.column));
                        if inside(cell.row, cell.column, start_row, start_column) {
                            if cells.len() >= 4096 {
                                return Err(OfficeError::LimitExceeded("materialized cells"));
                            }
                            let text = if cell.kind == "s" && !cell.value.text.is_empty() {
                                if cell.value.truncated {
                                    return Err(OfficeError::Malformed("shared-string ID"));
                                }
                                let id = cell
                                    .value
                                    .text
                                    .parse::<u32>()
                                    .map_err(|_| OfficeError::Malformed("shared-string ID"))?;
                                selected_ids.insert(cells.len(), id);
                                String::new()
                            } else if cell.kind == "inlineStr" {
                                cell.inline.finish()
                            } else if !cell.value.text.is_empty() {
                                if cell.kind == "b" {
                                    match cell.value.text.as_str() {
                                        "0" => "FALSE".to_owned(),
                                        "1" => "TRUE".to_owned(),
                                        _ => {
                                            return Err(OfficeError::Malformed(
                                                "boolean cell value",
                                            ));
                                        }
                                    }
                                } else {
                                    cell.value.finish()
                                }
                            } else {
                                let mut text = Text::default();
                                text.push("Formula (not evaluated): ");
                                if cell.formula.text.is_empty() {
                                    text.push("stored formula unavailable");
                                } else {
                                    text.push(&cell.formula.finish());
                                }
                                text.finish()
                            };
                            text_bytes += text.len();
                            if text_bytes > WINDOW_BYTES {
                                return Err(OfficeError::LimitExceeded(WINDOW_TEXT_LIMIT));
                            }
                            cells.push(Cell {
                                row: cell.row,
                                column: cell.column,
                                text,
                            });
                        } else if (cell.row, cell.column) > (start_row, start_column)
                            && next.is_none()
                        {
                            next = Some((cell.row, cell.column));
                        }
                    }
                }
                if name == "row" {
                    row_number = None;
                }
                stack.pop();
            }
            Token::Eof => break,
        }
    }
    if !sheet_data {
        return Err(OfficeError::Malformed("worksheet has no sheetData"));
    }
    drop(xml);
    if !selected_ids.is_empty() {
        let part = shared.ok_or(OfficeError::Malformed("missing shared-string table"))?;
        let needed: BTreeSet<u32> = selected_ids.values().copied().collect();
        let strings = shared_strings(package, part, &needed)?;
        for (cell, id) in selected_ids {
            let value = strings
                .get(&id)
                .ok_or(OfficeError::Malformed("shared-string ID outside table"))?;
            text_bytes += value.len();
            if text_bytes > WINDOW_BYTES {
                return Err(OfficeError::LimitExceeded(WINDOW_TEXT_LIMIT));
            }
            cells[cell].text = value.clone();
        }
    }
    let (row, column) = anchor.unwrap_or((1, 1));
    Ok(Window {
        row,
        column,
        cells,
        next,
        omitted,
    })
}

fn shared_strings(
    package: &mut Package,
    part: &str,
    needed: &BTreeSet<u32>,
) -> Result<BTreeMap<u32, String>, OfficeError> {
    let mut xml = package.xml(part)?;
    // Only sst/si with its text and runs is read; formatting, phonetic runs,
    // extension lists and other writers' markup are skipped whole.
    let mut stack: Vec<String> = Vec::new();
    let mut skipped = 0usize;
    let mut index = 0u32;
    let mut current = Text::default();
    let mut strings = BTreeMap::new();
    let mut retained = 0;
    loop {
        match xml.next()? {
            Token::Start(element) => {
                if skipped > 0 {
                    skipped += 1;
                    continue;
                }
                let name = (element.ns == Namespace::Sheet).then_some(element.name.as_str());
                match (stack.last().map(String::as_str), name) {
                    (None, Some("sst")) => (),
                    (None, _) => return Err(OfficeError::Malformed("shared-string table root")),
                    (Some("sst"), Some("si")) => current = Text::default(),
                    (Some("si"), Some("t" | "r")) | (Some("r"), Some("t")) => (),
                    _ => {
                        skipped = 1;
                        continue;
                    }
                }
                stack.push(element.name);
            }
            Token::Text(text) => {
                if skipped == 0
                    && needed.contains(&index)
                    && stack.last().map(String::as_str) == Some("t")
                {
                    current.push(&text);
                }
            }
            Token::End(_, name) => {
                if skipped > 0 {
                    skipped -= 1;
                    continue;
                }
                stack.pop();
                if name == "si" {
                    if needed.contains(&index) {
                        let text = std::mem::take(&mut current).finish();
                        retained += text.capacity();
                        if retained > 8 * 1024 * 1024 {
                            return Err(OfficeError::LimitExceeded("selected shared strings"));
                        }
                        strings.insert(index, text);
                        // A selected prefix is a preview, not whole-member CRC certification.
                        if strings.len() == needed.len() {
                            return Ok(strings);
                        }
                    }
                    index = index
                        .checked_add(1)
                        .ok_or(OfficeError::LimitExceeded("shared-string index"))?;
                }
            }
            Token::Eof => break,
        }
    }
    Err(OfficeError::Malformed("shared-string ID outside table"))
}

fn inside(row: u32, column: u16, first_row: u32, first_column: u16) -> bool {
    row >= first_row
        && row - first_row < 128
        && column >= first_column
        && column - first_column < 32
}

fn coordinate(value: &str) -> Result<(u32, u16), OfficeError> {
    if value.len() > 10 {
        return Err(OfficeError::Malformed("cell coordinate"));
    }
    let mut column = 0u16;
    let mut split = 0;
    for byte in value.bytes().take_while(u8::is_ascii_uppercase) {
        column = column
            .checked_mul(26)
            .and_then(|c| c.checked_add(u16::from(byte - b'A') + 1))
            .ok_or(OfficeError::Malformed("column coordinate overflow"))?;
        split += 1;
    }
    let row: u32 = value[split..]
        .parse()
        .map_err(|_| OfficeError::Malformed("row coordinate"))?;
    if !(1..=1_048_576).contains(&row)
        || !(1..=16_384).contains(&column)
        || !value[split..].bytes().all(|c| c.is_ascii_digit())
    {
        return Err(OfficeError::Malformed("cell outside XLSX coordinates"));
    }
    Ok((row, column))
}
