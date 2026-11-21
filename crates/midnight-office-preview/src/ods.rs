//! Bounded sparse OpenDocument spreadsheets. Logical repetitions never allocate their prefix.
use crate::{
    OfficeCatalog, OfficeError, OfficeFormat, OfficeTarget, SheetDescriptor, SheetId,
    document::Text,
    odt::{repeat, required, spaces},
    package::Package,
    sheet::{Cell, Window},
    xml::{Element, Namespace, Token},
};
use std::borrow::Cow;

const ROWS: u32 = 1_048_576;
const COLUMNS: u32 = 16_384;
const WINDOW_ROWS: u32 = 128;
const WINDOW_COLUMNS: u32 = 32;
const WINDOW_TEXT: usize = 512 * 1024;

type Stack = Vec<(Namespace, String)>;

pub(crate) struct Ods {
    pub catalog: OfficeCatalog,
}
impl Ods {
    pub fn load(package: &mut Package) -> Result<Self, OfficeError> {
        crate::odt::load(package, "spreadsheet")?;
        let work = package.work.clone();
        let mut xml = package.xml("content.xml")?;
        let mut stack = Stack::new();
        let mut document = Document::default();
        let mut sheets = Vec::new();
        loop {
            work.check()?;
            match xml.next()? {
                Token::Start(element) => {
                    document.enter(&element, &stack)?;
                    if direct_sheet(&element, &stack) {
                        if sheets.len() >= 1024 {
                            return Err(OfficeError::LimitExceeded("sheet descriptors"));
                        }
                        let name = required(&element, Namespace::OdfTable, "name")?;
                        if name.len() > 1024 {
                            return Err(OfficeError::LimitExceeded("sheet name"));
                        }
                        sheets.push(SheetDescriptor {
                            id: SheetId(sheets.len() as u16),
                            name: name.to_owned(),
                        });
                    } else if element.is(Namespace::OdfTable, "table")
                        && !stack
                            .iter()
                            .any(|(ns, name)| *ns == Namespace::OdfTable && name == "table")
                    {
                        return Err(OfficeError::Malformed("ODS sheet outside spreadsheet"));
                    }
                    stack.push((element.ns, element.name));
                }
                Token::End(_, _) => {
                    stack.pop();
                }
                Token::Text(_) => {}
                Token::Eof => break,
            }
        }
        if !document.body || !document.spreadsheet {
            return Err(OfficeError::Malformed("missing ODS body"));
        }
        Ok(Self {
            catalog: OfficeCatalog {
                format: OfficeFormat::Ods,
                sheets,
                document: None,
                slides: Vec::new(),
            },
        })
    }
}

#[derive(Default)]
struct Document {
    body: bool,
    spreadsheet: bool,
}
impl Document {
    /// Checks the document, body and spreadsheet structure. Scripts, styles and
    /// spreadsheet-level settings (validations, ranges, forms, tracked changes and
    /// other writers' markup) are never read, so they are not refused either.
    fn enter(&mut self, element: &Element, stack: &Stack) -> Result<(), OfficeError> {
        if stack.is_empty() && !element.is(Namespace::OdfOffice, "document-content") {
            return Err(OfficeError::Malformed("ODF content root"));
        }
        if element.is(Namespace::OdfOffice, "body") {
            if self.body
                || stack.len() != 1
                || !parent(stack, Namespace::OdfOffice, "document-content")
            {
                return Err(OfficeError::Malformed("ODS body structure"));
            }
            self.body = true;
        }
        if element.is(Namespace::OdfOffice, "spreadsheet") {
            if self.spreadsheet || stack.len() != 2 || !parent(stack, Namespace::OdfOffice, "body")
            {
                return Err(OfficeError::Malformed("ODS spreadsheet structure"));
            }
            self.spreadsheet = true;
        }
        Ok(())
    }
}

fn parent(stack: &Stack, ns: Namespace, name: &str) -> bool {
    stack
        .last()
        .is_some_and(|(parent_ns, parent_name)| *parent_ns == ns && parent_name == name)
}
fn direct_sheet(element: &Element, stack: &Stack) -> bool {
    element.is(Namespace::OdfTable, "table") && parent(stack, Namespace::OdfOffice, "spreadsheet")
}
fn row_parent(stack: &Stack) -> bool {
    stack.last().is_some_and(|(ns, name)| {
        *ns == Namespace::OdfTable
            && matches!(
                name.as_str(),
                "table" | "table-header-rows" | "table-rows" | "table-row-group"
            )
    })
}
fn column_parent(stack: &Stack) -> bool {
    stack.last().is_some_and(|(ns, name)| {
        *ns == Namespace::OdfTable
            && matches!(
                name.as_str(),
                "table" | "table-columns" | "table-header-columns" | "table-column-group"
            )
    })
}
fn end(start: u32, count: u32, maximum: u32) -> Result<u32, OfficeError> {
    start
        .checked_add(count)
        .filter(|next| *next <= maximum + 1)
        .ok_or(OfficeError::LimitExceeded("ODS logical coordinates"))
}

struct Row {
    first: u32,
    end: u32,
    column: u32,
    cells: Vec<(u16, String)>,
    text_bytes: usize,
}
struct Pending {
    first: u32,
    end: u32,
    text: Text,
    fallback: Text,
    has_text: bool,
    cached_text: bool,
    formula: bool,
    paragraph: Option<usize>,
    paragraphs_seen: bool,
}
impl Pending {
    fn new(element: &Element, row: &Row, anchor: Option<(u32, u16)>) -> Result<Self, OfficeError> {
        let count = repeat(element, Namespace::OdfTable, "number-columns-repeated")?;
        let end = end(row.column, count, COLUMNS)?;
        let selected = anchor.is_none_or(|(top, left)| {
            row.first < top + WINDOW_ROWS
                && row.end > top
                && row.column < u32::from(left) + WINDOW_COLUMNS
                && end > u32::from(left)
        });
        let cached = cached_value(element)?;
        let formula = element.attr_ns(Namespace::OdfTable, "formula");
        let mut fallback = Text::new(selected);
        if let Some(value) = &cached {
            fallback.push(value);
        } else if let Some(formula) = formula {
            fallback.push("Formula (not evaluated): ");
            fallback.push(if formula.is_empty() {
                "stored formula unavailable"
            } else {
                formula
            });
        }
        Ok(Self {
            first: row.column,
            end,
            text: Text::new(selected),
            fallback,
            has_text: false,
            cached_text: cached.as_ref().is_some_and(|value| !value.is_empty()),
            formula: formula.is_some(),
            paragraph: None,
            paragraphs_seen: false,
        })
    }
    fn finish(self) -> (String, bool) {
        let populated = self.has_text || self.cached_text || self.formula;
        (
            if self.has_text {
                self.text.finish()
            } else {
                self.fallback.finish()
            },
            populated,
        )
    }
}

fn cached_value(element: &Element) -> Result<Option<Cow<'_, str>>, OfficeError> {
    let office = |name| element.attr_ns(Namespace::OdfOffice, name);
    let boolean = |value: &str| match value {
        "true" | "1" => Ok(Cow::Borrowed("TRUE")),
        "false" | "0" => Ok(Cow::Borrowed("FALSE")),
        _ => Err(OfficeError::Malformed("ODS boolean stored value")),
    };
    let attribute = match office("value-type") {
        Some("float" | "currency" | "percentage") => "value",
        Some("string") => "string-value",
        Some("date") => "date-value",
        Some("time") => "time-value",
        Some("boolean") => return office("boolean-value").map(boolean).transpose(),
        // Another type has no stored value the preview knows; its paragraphs show it.
        Some(_) => return Ok(None),
        None => {
            if let Some(value) = office("boolean-value") {
                return boolean(value).map(Some);
            }
            return Ok(["string-value", "value", "date-value", "time-value"]
                .into_iter()
                .find_map(office)
                .map(Cow::Borrowed));
        }
    };
    Ok(office(attribute).map(Cow::Borrowed))
}

/// A successful result contains only the requested intersection, never a repeated prefix.
pub(crate) fn window(package: &mut Package, target: OfficeTarget) -> Result<Window, OfficeError> {
    target.validate()?;
    let (sheet, mut anchor) = match target {
        OfficeTarget::SheetFirst { sheet } => (sheet, None),
        OfficeTarget::SheetWindow { sheet, row, column } => (sheet, Some((row, column))),
        _ => return Err(OfficeError::InvalidTarget),
    };
    if usize::from(sheet.0) >= 1024 {
        return Err(OfficeError::InvalidTarget);
    }
    let work = package.work.clone();
    let mut xml = package.xml("content.xml")?;
    let mut stack = Stack::new();
    let mut document = Document::default();
    let mut table_index = 0usize;
    let mut active = None;
    let mut skipped = None;
    let mut row_cursor = 1;
    let mut declared_columns = 0;
    let mut row: Option<Row> = None;
    let mut pending: Option<Pending> = None;
    let mut cells = Vec::new();
    let mut next = None;
    let mut text_bytes = 0usize;
    let mut omitted = false;
    loop {
        work.check()?;
        match xml.next()? {
            Token::Start(element) => {
                document.enter(&element, &stack)?;
                if direct_sheet(&element, &stack) {
                    if table_index == usize::from(sheet.0) {
                        active = Some(stack.len());
                    }
                    table_index += 1;
                } else if active.is_some() && skipped.is_none() {
                    match (element.ns, element.name.as_str()) {
                        // A table inside a cell or a shape is not part of this grid.
                        (Namespace::OdfTable, "table") => {
                            omitted = true;
                            skipped = Some(stack.len());
                        }
                        (Namespace::OdfTable, "table-row") => {
                            if row.is_some() || !row_parent(&stack) {
                                return Err(OfficeError::Malformed("ODS row structure"));
                            }
                            let count =
                                repeat(&element, Namespace::OdfTable, "number-rows-repeated")?;
                            row = Some(Row {
                                first: row_cursor,
                                end: end(row_cursor, count, ROWS)?,
                                column: 1,
                                cells: Vec::new(),
                                text_bytes: 0,
                            });
                        }
                        (Namespace::OdfTable, "table-cell" | "covered-table-cell") => {
                            if pending.is_some()
                                || !parent(&stack, Namespace::OdfTable, "table-row")
                            {
                                return Err(OfficeError::Malformed("ODS cell structure"));
                            }
                            pending = Some(Pending::new(
                                &element,
                                row.as_ref()
                                    .ok_or(OfficeError::Malformed("ODS cell outside row"))?,
                                anchor,
                            )?);
                        }
                        (Namespace::OdfText, "p") => {
                            let cell = pending
                                .as_mut()
                                .ok_or(OfficeError::Malformed("ODS paragraph outside cell"))?;
                            if cell.paragraph.is_some() {
                                return Err(OfficeError::Malformed("nested ODS paragraph"));
                            }
                            if cell.paragraphs_seen {
                                cell.text.push("\n");
                                cell.has_text = true;
                            }
                            cell.paragraphs_seen = true;
                            cell.paragraph = Some(stack.len());
                        }
                        (Namespace::OdfText, "s") => {
                            let cell = paragraph(&mut pending)?;
                            let count = spaces(&element)?;
                            cell.text.spaces(count);
                            cell.has_text |= count > 0;
                            work.check()?;
                        }
                        (Namespace::OdfText, "tab" | "line-break") => {
                            let cell = paragraph(&mut pending)?;
                            cell.text
                                .push(if element.name == "tab" { "\t" } else { "\n" });
                            cell.has_text = true;
                        }
                        (
                            Namespace::OdfText,
                            "span" | "a" | "bookmark" | "bookmark-start" | "bookmark-end"
                            | "soft-page-break",
                        ) => {
                            paragraph(&mut pending)?;
                        }
                        (
                            Namespace::OdfTable,
                            "table-header-rows" | "table-rows" | "table-row-group",
                        ) => {
                            if row.is_some() || !row_parent(&stack) {
                                return Err(OfficeError::Malformed("ODS row container"));
                            }
                        }
                        (
                            Namespace::OdfTable,
                            "table-columns" | "table-header-columns" | "table-column-group",
                        ) => {
                            if row.is_some() || !column_parent(&stack) {
                                return Err(OfficeError::Malformed("ODS column container"));
                            }
                        }
                        (Namespace::OdfTable, "table-column") => {
                            if row.is_some() || !column_parent(&stack) {
                                return Err(OfficeError::Malformed("ODS column declaration"));
                            }
                            let count =
                                repeat(&element, Namespace::OdfTable, "number-columns-repeated")?;
                            declared_columns = end(declared_columns + 1, count, COLUMNS)? - 1;
                        }
                        (
                            Namespace::OdfTable,
                            "table-source" | "named-expressions" | "scenario",
                        ) if row.is_none() && parent(&stack, Namespace::OdfTable, "table") => {
                            // Stored values remain readable; data sources are never activated.
                            skipped = Some(stack.len());
                        }
                        // Shapes, charts, pictures and comments are not shown.
                        (Namespace::OdfDraw, _)
                        | (Namespace::OdfTable, "shapes")
                        | (Namespace::OdfOffice, "annotation") => {
                            omitted = true;
                            skipped = Some(stack.len());
                        }
                        // Other markup in a cell could hold text: never cell text, and
                        // the page says that something is not shown.
                        _ if pending.is_some() => {
                            omitted = true;
                            skipped = Some(stack.len());
                        }
                        // Forms, conditional formats, protection and other sheet-level
                        // markup hold no cell contents.
                        _ => skipped = Some(stack.len()),
                    }
                }
                stack.push((element.ns, element.name));
            }
            Token::Text(value) => {
                if active.is_some()
                    && skipped.is_none()
                    && let Some(cell) = &mut pending
                {
                    if cell.paragraph.is_some() {
                        cell.has_text |= !value.is_empty();
                        cell.text.push(&value);
                    } else if !value.trim().is_empty() {
                        return Err(OfficeError::Malformed("ODS cell text outside paragraph"));
                    }
                }
            }
            Token::End(ns, name) => {
                stack.pop();
                if let Some(depth) = skipped {
                    if stack.len() == depth {
                        skipped = None;
                    }
                    continue;
                }
                if active.is_none() {
                    continue;
                }
                match (ns, name.as_str()) {
                    (Namespace::OdfText, "p") => {
                        let cell = pending
                            .as_mut()
                            .ok_or(OfficeError::Malformed("ODS paragraph end"))?;
                        if cell.paragraph != Some(stack.len()) {
                            return Err(OfficeError::Malformed("ODS paragraph structure"));
                        }
                        cell.paragraph = None;
                    }
                    (Namespace::OdfTable, "table-cell" | "covered-table-cell") => {
                        let cell = pending
                            .take()
                            .ok_or(OfficeError::Malformed("ODS cell end"))?;
                        let first = cell.first;
                        let last = cell.end;
                        let (text, populated) = cell.finish();
                        let row = row.as_mut().ok_or(OfficeError::Malformed("ODS row"))?;
                        row.column = last;
                        if !populated {
                            continue;
                        }
                        let (top, left) = *anchor.get_or_insert((row.first, first as u16));
                        next_region(
                            &mut next,
                            row.first,
                            row.end,
                            first,
                            last,
                            top,
                            u32::from(left),
                        );
                        if row.end > top && row.first < top + WINDOW_ROWS {
                            for column in first.max(u32::from(left))
                                ..last.min(u32::from(left) + WINDOW_COLUMNS)
                            {
                                work.check()?;
                                row.text_bytes = row
                                    .text_bytes
                                    .checked_add(text.len())
                                    .filter(|bytes| *bytes <= WINDOW_TEXT)
                                    .ok_or(OfficeError::LimitExceeded("ODS row text"))?;
                                row.cells.push((column as u16, text.clone()));
                            }
                        }
                    }
                    (Namespace::OdfTable, "table-row") => {
                        if pending.is_some() {
                            return Err(OfficeError::Malformed("unfinished ODS cell"));
                        }
                        let row = row.take().ok_or(OfficeError::Malformed("ODS row end"))?;
                        if let Some((top, _)) = anchor {
                            for number in row.first.max(top)..row.end.min(top + WINDOW_ROWS) {
                                for (column, text) in &row.cells {
                                    work.check()?;
                                    text_bytes = text_bytes
                                        .checked_add(text.len())
                                        .filter(|bytes| *bytes <= WINDOW_TEXT)
                                        .ok_or(OfficeError::LimitExceeded("ODS window text"))?;
                                    if cells.len() >= 4096 {
                                        return Err(OfficeError::LimitExceeded("ODS window cells"));
                                    }
                                    cells.push(Cell {
                                        row: number,
                                        column: *column,
                                        text: text.clone(),
                                    });
                                }
                            }
                        }
                        row_cursor = row.end;
                    }
                    (Namespace::OdfTable, "table") if active == Some(stack.len()) => break,
                    _ => {}
                }
            }
            Token::Eof => break,
        }
    }
    if active.is_none() {
        return Err(OfficeError::InvalidTarget);
    }
    work.check()?;
    let (row, column) = anchor.unwrap_or((1, 1));
    Ok(Window {
        row,
        column,
        cells,
        next,
        omitted,
    })
}

fn paragraph(pending: &mut Option<Pending>) -> Result<&mut Pending, OfficeError> {
    pending
        .as_mut()
        .filter(|cell| cell.paragraph.is_some())
        .ok_or(OfficeError::Malformed("ODS inline text outside paragraph"))
}

fn next_region(
    next: &mut Option<(u32, u16)>,
    first_row: u32,
    end_row: u32,
    first_column: u32,
    end_column: u32,
    top: u32,
    left: u32,
) {
    let bottom = (top + WINDOW_ROWS).min(ROWS + 1);
    let right = (left + WINDOW_COLUMNS).min(COLUMNS + 1);
    let mut candidate = |row: u32, column: u32| {
        let point = (row, column as u16);
        if next.is_none_or(|previous| point < previous) {
            *next = Some(point);
        }
    };
    let row = first_row.max(top);
    if row < end_row && row < bottom && end_column > right {
        candidate(row, first_column.max(right));
    }
    let row = first_row.max(bottom);
    if row < end_row {
        candidate(row, first_column);
    }
}

#[cfg(test)]
#[path = "ods_tests.rs"]
mod tests;
