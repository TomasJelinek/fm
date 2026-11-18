use crate::{
    OfficeError, OfficeTarget,
    package::Package,
    workbook::Workbook,
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

pub(crate) fn load(
    package: &mut Package,
    workbook: &Workbook,
    target: OfficeTarget,
) -> Result<Window, OfficeError> {
    let (sheet, mut anchor) = match target {
        OfficeTarget::SheetFirst { sheet } => (sheet, None),
        OfficeTarget::SheetWindow { sheet, row, column } => (sheet, Some((row, column))),
        _ => return Err(OfficeError::InvalidTarget),
    };
    let mut xml = package.xml(&workbook.parts[usize::from(sheet.0)])?;
    let mut cells = Vec::new();
    let mut selected_ids = BTreeMap::new();
    let mut next = None;
    let mut pending: Option<Pending> = None;
    let mut stack: Vec<String> = Vec::new();
    let mut previous = None;
    let mut row_number = None;
    let mut text_bytes = 0usize;
    let mut sheet_data = false;
    loop {
        match xml.next()? {
            Token::Start(element) => {
                if stack.is_empty() && !element.is(Namespace::Sheet, "worksheet") {
                    return Err(OfficeError::Unsupported("not a worksheet"));
                }
                if element.ns != Namespace::Sheet || !supported_element(&element.name) {
                    return Err(OfficeError::Unsupported(
                        "worksheet element outside contents subset",
                    ));
                }
                match element.name.as_str() {
                    "sheetData" => {
                        if sheet_data || stack.as_slice() != ["worksheet"] {
                            return Err(OfficeError::Malformed("worksheet sheetData structure"));
                        }
                        sheet_data = true;
                    }
                    "row" => {
                        if stack.last().map(String::as_str) != Some("sheetData") {
                            return Err(OfficeError::Malformed("worksheet row structure"));
                        }
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
                    "c" => {
                        if pending.is_some() || stack.last().map(String::as_str) != Some("row") {
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
                            return Err(OfficeError::Unsupported("cell value type"));
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
                    "v" | "f" | "is" => {
                        let cell = pending
                            .as_mut()
                            .ok_or(OfficeError::Malformed("cell value outside cell"))?;
                        if stack.last().map(String::as_str) != Some("c") {
                            return Err(OfficeError::Malformed("nested cell value"));
                        }
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
                    "t" if pending.is_none()
                        || !matches!(
                            stack.last().map(String::as_str),
                            Some("is" | "r" | "rPh")
                        ) =>
                    {
                        return Err(OfficeError::Malformed("inline text structure"));
                    }
                    _ => (),
                }
                stack.push(element.name);
            }
            Token::Text(text) => {
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
                        Some("t") if !stack.iter().any(|name| name == "rPh") => {
                            cell.inline.push(value)
                        }
                        _ => (),
                    }
                }
            }
            Token::End(ns, name) => {
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
                                return Err(OfficeError::LimitExceeded("window text"));
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
        let part = workbook
            .shared_strings
            .as_deref()
            .ok_or(OfficeError::Malformed("missing shared-string table"))?;
        let needed: BTreeSet<u32> = selected_ids.values().copied().collect();
        let strings = shared_strings(package, part, &needed)?;
        for (cell, id) in selected_ids {
            let value = strings
                .get(&id)
                .ok_or(OfficeError::Malformed("shared-string ID outside table"))?;
            text_bytes += value.len();
            if text_bytes > WINDOW_BYTES {
                return Err(OfficeError::LimitExceeded("window text"));
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
    })
}

fn shared_strings(
    package: &mut Package,
    part: &str,
    needed: &BTreeSet<u32>,
) -> Result<BTreeMap<u32, String>, OfficeError> {
    let mut xml = package.xml(part)?;
    let mut stack: Vec<String> = Vec::new();
    let mut index = 0u32;
    let mut current = Text::default();
    let mut strings = BTreeMap::new();
    let mut retained = 0;
    loop {
        match xml.next()? {
            Token::Start(element) => {
                if stack.is_empty() && !element.is(Namespace::Sheet, "sst") {
                    return Err(OfficeError::Malformed("shared-string table root"));
                }
                if element.ns != Namespace::Sheet
                    || ![
                        "sst",
                        "si",
                        "t",
                        "r",
                        "rPr",
                        "rFont",
                        "charset",
                        "family",
                        "b",
                        "i",
                        "strike",
                        "outline",
                        "shadow",
                        "condense",
                        "extend",
                        "color",
                        "sz",
                        "u",
                        "vertAlign",
                        "scheme",
                        "rPh",
                        "phoneticPr",
                    ]
                    .contains(&element.name.as_str())
                {
                    return Err(OfficeError::Unsupported("shared-string element"));
                }
                if element.name == "si" {
                    if stack.as_slice() != ["sst"] {
                        return Err(OfficeError::Malformed("shared-string structure"));
                    }
                    current = Text::default();
                }
                stack.push(element.name);
            }
            Token::Text(text) => {
                if needed.contains(&index)
                    && stack.last().map(String::as_str) == Some("t")
                    && !stack.iter().any(|name| name == "rPh")
                {
                    current.push(&text);
                }
            }
            Token::End(_, name) => {
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

fn supported_element(name: &str) -> bool {
    [
        "worksheet",
        "sheetPr",
        "tabColor",
        "outlinePr",
        "pageSetUpPr",
        "dimension",
        "sheetViews",
        "sheetView",
        "pane",
        "selection",
        "sheetFormatPr",
        "cols",
        "col",
        "sheetData",
        "row",
        "c",
        "v",
        "f",
        "is",
        "t",
        "r",
        "rPr",
        "rFont",
        "charset",
        "family",
        "b",
        "i",
        "strike",
        "outline",
        "shadow",
        "condense",
        "extend",
        "color",
        "sz",
        "u",
        "vertAlign",
        "scheme",
        "rPh",
        "phoneticPr",
        "sheetCalcPr",
        "sheetProtection",
        "protectedRanges",
        "protectedRange",
        "scenarios",
        "scenario",
        "inputCells",
        "autoFilter",
        "filterColumn",
        "filters",
        "filter",
        "dateGroupItem",
        "customFilters",
        "customFilter",
        "dynamicFilter",
        "colorFilter",
        "iconFilter",
        "top10",
        "sortState",
        "sortCondition",
        "dataConsolidate",
        "dataRefs",
        "dataRef",
        "mergeCells",
        "mergeCell",
        "conditionalFormatting",
        "cfRule",
        "formula",
        "colorScale",
        "cfvo",
        "dataBar",
        "iconSet",
        "dataValidations",
        "dataValidation",
        "formula1",
        "formula2",
        "hyperlinks",
        "hyperlink",
        "printOptions",
        "pageMargins",
        "pageSetup",
        "headerFooter",
        "oddHeader",
        "oddFooter",
        "evenHeader",
        "evenFooter",
        "firstHeader",
        "firstFooter",
        "rowBreaks",
        "colBreaks",
        "brk",
        "ignoredErrors",
        "ignoredError",
        "tableParts",
        "tablePart",
    ]
    .contains(&name)
}
