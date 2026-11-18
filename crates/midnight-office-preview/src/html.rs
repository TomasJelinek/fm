use crate::{OfficeCatalog, OfficeError, OfficeFormat, OfficeTarget, SheetId, sheet::Window};

const HTML_BYTES: usize = 1024 * 1024;
const FOOTER: &str = "</body></html>";
const LINK_OPEN: &str = "<a href=\"";
const LINK_TITLE: &str = "\">";
const LINK_CLOSE: &str = "</a>";
const CATALOG_END: &str = "</nav>";
const ELLIPSIS: &str = "…";

pub(crate) struct Html(String);
impl Html {
    pub(crate) fn new() -> Self {
        Self("<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width\"><title>Office preview</title><link rel=\"icon\" href=\"data:,\"><style>body{font:16px system-ui;margin:1.2rem;color:#20252b;background:#fff}nav{display:flex;flex-wrap:wrap;gap:1rem;margin:1rem 0}table{border-collapse:collapse}th,td{border:1px solid #ccd2da;padding:.3rem;vertical-align:top;white-space:pre-wrap;overflow-wrap:anywhere;max-width:32rem}td{min-width:6rem}th{background:#edf1f5;white-space:nowrap}a{color:#145ba6}</style></head><body><p>Office contents preview — text and stored values; layout and formatting may differ.</p>".to_owned())
    }
    pub(crate) fn raw(&mut self, text: &str) -> Result<(), OfficeError> {
        if self
            .0
            .len()
            .checked_add(text.len())
            .is_none_or(|length| length > HTML_BYTES)
        {
            return Err(OfficeError::LimitExceeded("HTML resource"));
        }
        self.0.push_str(text);
        Ok(())
    }
    pub(crate) fn text(&mut self, text: &str) -> Result<(), OfficeError> {
        for character in text.chars() {
            if let Some(escaped) = escape(character) {
                self.raw(escaped)?;
            } else {
                let mut bytes = [0; 4];
                self.raw(character.encode_utf8(&mut bytes))?;
            }
        }
        Ok(())
    }
    fn link_open(&mut self, target: OfficeTarget) -> Result<(), OfficeError> {
        self.raw(LINK_OPEN)?;
        self.text(&target.path())?;
        self.raw(LINK_TITLE)
    }
    pub(crate) fn link(&mut self, target: OfficeTarget, title: &str) -> Result<(), OfficeError> {
        self.link_open(target)?;
        self.text(title)?;
        self.raw(LINK_CLOSE)
    }
    fn preview(&mut self, text: &str, budget: usize) -> Result<(), OfficeError> {
        let mut end = 0;
        let mut cost = 0usize;
        for (offset, character) in text.char_indices() {
            let next = checked_add(cost, escaped_character_len(character))?;
            if next > budget {
                break;
            }
            cost = next;
            end = offset + character.len_utf8();
        }
        self.text(&text[..end])?;
        if end < text.len() {
            self.raw(ELLIPSIS)?;
        }
        Ok(())
    }
    pub(crate) fn finish(mut self) -> Result<Vec<u8>, OfficeError> {
        self.raw(FOOTER)?;
        Ok(self.0.into_bytes())
    }
}

fn escape(character: char) -> Option<&'static str> {
    match character {
        '&' => Some("&amp;"),
        '<' => Some("&lt;"),
        '>' => Some("&gt;"),
        '"' => Some("&quot;"),
        '\'' => Some("&#39;"),
        _ => None,
    }
}

fn escaped_character_len(character: char) -> usize {
    escape(character).map_or_else(|| character.len_utf8(), str::len)
}

fn checked_add(left: usize, right: usize) -> Result<usize, OfficeError> {
    left.checked_add(right)
        .ok_or(OfficeError::LimitExceeded("HTML resource"))
}

fn escaped_len(text: &str) -> Result<usize, OfficeError> {
    text.chars().try_fold(0, |length, character| {
        checked_add(length, escaped_character_len(character))
    })
}

fn catalog_links<'a>(
    html: &mut Html,
    entries: impl Clone + ExactSizeIterator<Item = (OfficeTarget, &'a str)>,
) -> Result<(), OfficeError> {
    let count = entries.len();
    if count == 0 {
        return Ok(());
    }
    // Reserve every anchor, ordinal, possible ellipsis and closing tag before
    // sharing the remaining escaped-name bytes. The prefix is the actual page,
    // including its CSS/copy; no descriptor or canonical target is discarded.
    let mut fixed = checked_add(html.0.len(), CATALOG_END.len())?;
    fixed = checked_add(fixed, FOOTER.len())?;
    for (index, (target, _)) in entries.clone().enumerate() {
        for length in [
            LINK_OPEN.len(),
            escaped_len(&target.path())?,
            LINK_TITLE.len(),
            escaped_len(&format!("{}: ", checked_add(index, 1)?))?,
            ELLIPSIS.len(),
            LINK_CLOSE.len(),
        ] {
            fixed = checked_add(fixed, length)?;
        }
    }
    let budget = HTML_BYTES
        .checked_sub(fixed)
        .and_then(|remaining| remaining.checked_div(count))
        .ok_or(OfficeError::LimitExceeded("HTML resource"))?;
    for (index, (target, name)) in entries.enumerate() {
        html.link_open(target)?;
        html.text(&format!("{}: ", checked_add(index, 1)?))?;
        html.preview(name, budget)?;
        html.raw(LINK_CLOSE)?;
    }
    Ok(())
}

pub(crate) fn catalog(catalog: &OfficeCatalog) -> Result<Vec<u8>, OfficeError> {
    let mut html = Html::new();
    let (heading, label, empty) = match catalog.format {
        OfficeFormat::Xlsx | OfficeFormat::Ods => ("Workbook sheets", "Sheets", "Empty workbook."),
        OfficeFormat::Docx | OfficeFormat::Odt => {
            ("Document contents", "Contents", "Empty document.")
        }
        OfficeFormat::Pptx | OfficeFormat::Odp => ("Slides", "Slides", "Empty presentation."),
    };
    html.raw("<h1>")?;
    html.text(heading)?;
    html.raw("</h1><p>Long names are shortened; open an item to see its full name.</p><nav aria-label=\"")?;
    html.text(label)?;
    html.raw("\">")?;
    let is_empty = match catalog.format {
        OfficeFormat::Xlsx | OfficeFormat::Ods => {
            catalog_links(
                &mut html,
                catalog.sheets.iter().map(|entry| {
                    (
                        OfficeTarget::SheetFirst { sheet: entry.id },
                        entry.name.as_str(),
                    )
                }),
            )?;
            catalog.sheets.is_empty()
        }
        OfficeFormat::Docx | OfficeFormat::Odt => {
            catalog_links(
                &mut html,
                catalog.document.iter().map(|entry| {
                    (
                        OfficeTarget::DocumentWindow {
                            document: entry.id,
                            block: 0,
                        },
                        entry.name.as_str(),
                    )
                }),
            )?;
            catalog.document.is_none()
        }
        OfficeFormat::Pptx | OfficeFormat::Odp => {
            catalog_links(
                &mut html,
                catalog.slides.iter().map(|entry| {
                    (
                        OfficeTarget::SlideWindow {
                            slide: entry.id,
                            block: 0,
                        },
                        entry.name.as_str(),
                    )
                }),
            )?;
            catalog.slides.is_empty()
        }
    };
    html.raw(CATALOG_END)?;
    if is_empty {
        html.raw("<p>")?;
        html.text(empty)?;
        html.raw("</p>")?;
    }
    html.finish()
}

pub(crate) fn sheet(
    catalog: &OfficeCatalog,
    sheet: SheetId,
    window: &Window,
) -> Result<Vec<u8>, OfficeError> {
    let position = usize::from(sheet.0);
    let descriptor = catalog
        .sheets
        .get(position)
        .filter(|entry| entry.id == sheet)
        .ok_or(OfficeError::InvalidTarget)?;
    let mut html = Html::new();
    html.raw("<h1>")?;
    html.text(&descriptor.name)?;
    html.raw("</h1><nav aria-label=\"Sheets\">")?;
    html.link(OfficeTarget::Catalog, "All sheets")?;
    html.link(OfficeTarget::SheetFirst { sheet }, "First cells")?;
    if let Some(previous) = position
        .checked_sub(1)
        .and_then(|index| catalog.sheets.get(index))
    {
        html.link(
            OfficeTarget::SheetFirst { sheet: previous.id },
            "Previous sheet",
        )?;
    }
    if let Some(next) = catalog.sheets.get(position + 1) {
        html.link(OfficeTarget::SheetFirst { sheet: next.id }, "Next sheet")?;
    }
    html.raw("</nav><nav aria-label=\"Cell windows\">")?;
    let row = window.row;
    let column = window.column;
    if row > 1 {
        html.link(
            OfficeTarget::SheetWindow {
                sheet,
                row: row.saturating_sub(128).max(1),
                column,
            },
            "Previous rows",
        )?;
    }
    if row <= 1_048_576 - 128 {
        html.link(
            OfficeTarget::SheetWindow {
                sheet,
                row: row + 128,
                column,
            },
            "Next rows",
        )?;
    }
    if column > 1 {
        html.link(
            OfficeTarget::SheetWindow {
                sheet,
                row,
                column: column.saturating_sub(32).max(1),
            },
            "Previous columns",
        )?;
    }
    if column <= 16_384 - 32 {
        html.link(
            OfficeTarget::SheetWindow {
                sheet,
                row,
                column: column + 32,
            },
            "Next columns",
        )?;
    }
    if let Some((row, column)) = window.next {
        html.link(
            OfficeTarget::SheetWindow { sheet, row, column },
            "Next populated region",
        )?;
    }
    html.raw("</nav><p>")?;
    html.text(&format!(
        "Rows {}–{}; columns {}–{}",
        row,
        (row + 127).min(1_048_576),
        column_label(column),
        column_label((column + 31).min(16_384))
    ))?;
    html.raw(". Empty rows are omitted.</p>")?;
    if window.cells.is_empty() {
        html.raw("<p>Empty cell range.</p>")?;
        return html.finish();
    }
    html.raw("<table><thead><tr><th scope=\"col\">Row</th>")?;
    for col in column..=(column + 31).min(16_384) {
        html.raw("<th scope=\"col\">")?;
        html.text(&column_label(col))?;
        html.raw("</th>")?;
    }
    html.raw("</tr></thead><tbody>")?;
    let mut position = 0;
    while position < window.cells.len() {
        let row = window.cells[position].row;
        html.raw("<tr><th scope=\"row\">")?;
        html.text(&row.to_string())?;
        html.raw("</th>")?;
        for col in column..=(column + 31).min(16_384) {
            html.raw("<td>")?;
            if let Some(cell) = window.cells.get(position)
                && cell.row == row
                && cell.column == col
            {
                html.text(&cell.text)?;
                position += 1;
            }
            html.raw("</td>")?;
        }
        html.raw("</tr>")?;
    }
    html.raw("</tbody></table>")?;
    html.finish()
}

fn column_label(mut column: u16) -> String {
    let mut result = Vec::with_capacity(3);
    while column > 0 {
        column -= 1;
        result.push(b'A' + (column % 26) as u8);
        column /= 26;
    }
    result.reverse();
    result.into_iter().map(char::from).collect()
}

#[cfg(test)]
#[path = "html_tests.rs"]
mod tests;
