//! Bounded logical text/table windows shared by documents and slide contents.
use crate::{OfficeCatalog, OfficeError, OfficeTarget, SlideId, html::Html};

pub(crate) const BLOCKS: u32 = 64;
pub(crate) const MAX_BLOCKS: u32 = 1_048_576;
const TEXT_BYTES: usize = 32 * 1024;
const WINDOW_BYTES: usize = 512 * 1024;
const TRUNCATED: &str = " [truncated]";

#[derive(Clone, Default)]
pub(crate) struct Text {
    value: String,
    truncated: bool,
    enabled: bool,
    limit: usize,
}
impl Text {
    pub fn new(enabled: bool) -> Self {
        Self::with_limit(enabled, TEXT_BYTES)
    }
    pub fn with_limit(enabled: bool, limit: usize) -> Self {
        Self {
            enabled,
            limit: limit.min(TEXT_BYTES),
            ..Self::default()
        }
    }
    pub fn push(&mut self, value: &str) {
        if !self.enabled || self.truncated {
            return;
        }
        let remaining = self
            .limit
            .saturating_sub(TRUNCATED.len())
            .saturating_sub(self.value.len());
        let mut length = remaining.min(value.len());
        while !value.is_char_boundary(length) {
            length -= 1;
        }
        self.value.push_str(&value[..length]);
        self.truncated = length != value.len();
    }
    pub fn clear(&mut self) {
        self.value.clear();
        self.truncated = false;
    }
    pub fn spaces(&mut self, count: u32) {
        if !self.enabled || self.truncated {
            return;
        }
        let remaining = self
            .limit
            .saturating_sub(TRUNCATED.len())
            .saturating_sub(self.value.len());
        let length = remaining.min(count as usize);
        self.value.extend(std::iter::repeat_n(' ', length));
        self.truncated = length != count as usize;
    }
    pub fn finish(mut self) -> String {
        if self.truncated && self.limit >= TRUNCATED.len() {
            self.value.push_str(TRUNCATED);
        }
        self.value
    }
}

/// Unselected physical rows retain structural state but no cell vector.
pub(crate) struct Row {
    cells: Option<Vec<String>>,
    bytes: usize,
    omitted_columns: bool,
}
impl Row {
    pub fn new(selected: bool) -> Self {
        Self {
            cells: selected.then(Vec::new),
            bytes: 0,
            omitted_columns: false,
        }
    }
    pub fn text(&self) -> Text {
        Text::with_limit(
            self.cells.as_ref().is_some_and(|cells| cells.len() < 32),
            TEXT_BYTES.saturating_sub(self.bytes),
        )
    }
    pub fn append(&mut self, value: &str, count: u32) {
        let Some(cells) = &mut self.cells else {
            return;
        };
        let copies = 32usize.saturating_sub(cells.len()).min(count as usize);
        for _ in 0..copies {
            let remaining = TEXT_BYTES.saturating_sub(self.bytes);
            if remaining < TRUNCATED.len() {
                self.omitted_columns = true;
                return;
            }
            let mut text = Text::with_limit(true, remaining);
            text.push(value);
            let text = text.finish();
            self.bytes += text.len();
            cells.push(text);
            #[cfg(test)]
            MATERIALIZED_CELLS.with(|count| count.set(count.get() + 1));
        }
        self.omitted_columns |= copies < count as usize;
    }
    pub fn finish(self) -> Option<Block> {
        self.cells.map(|cells| Block::Row {
            cells,
            omitted_columns: self.omitted_columns,
        })
    }
}

#[cfg(test)]
std::thread_local! { static MATERIALIZED_CELLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }
#[cfg(test)]
pub(crate) fn take_materialized_cells() -> usize {
    MATERIALIZED_CELLS.with(|count| count.replace(0))
}

#[derive(Clone)]
pub(crate) enum Block {
    Paragraph {
        text: String,
        heading: u8,
        list: bool,
    },
    Row {
        cells: Vec<String>,
        omitted_columns: bool,
    },
}
impl Block {
    fn has_text(&self) -> bool {
        let visible = |text: &str| text.chars().any(|character| !character.is_whitespace());
        match self {
            Self::Paragraph { text, .. } => visible(text),
            Self::Row { cells, .. } => cells.iter().any(|text| visible(text)),
        }
    }
    fn bytes(&self) -> usize {
        match self {
            Self::Paragraph { text, .. } => text.len(),
            Self::Row { cells, .. } => cells.iter().map(String::len).sum(),
        }
    }
}
pub(crate) struct Window {
    pub start: u32,
    pub blocks: Vec<Block>,
    pub next: Option<u32>,
    pub omitted: bool,
}
pub(crate) struct Collector {
    pub window: Window,
    cursor: u32,
    text_bytes: usize,
    html_bytes: usize,
    pending_omitted: bool,
}
impl Collector {
    pub fn new(start: u32) -> Result<Self, OfficeError> {
        if start >= MAX_BLOCKS {
            return Err(OfficeError::InvalidTarget);
        }
        Ok(Self {
            window: Window {
                start,
                blocks: Vec::new(),
                next: None,
                omitted: false,
            },
            cursor: 0,
            text_bytes: 0,
            html_bytes: 16 * 1024,
            pending_omitted: false,
        })
    }
    pub fn selected(&self) -> bool {
        self.cursor >= self.window.start && self.cursor < self.window.start.saturating_add(BLOCKS)
    }
    pub fn selected_repeat(&self, count: u32) -> bool {
        self.cursor < self.window.start.saturating_add(BLOCKS)
            && self.cursor.saturating_add(count) > self.window.start
    }
    fn end(&self, count: u32) -> Result<u32, OfficeError> {
        if count == 0 {
            return Err(OfficeError::Malformed("zero document repetition"));
        }
        self.cursor
            .checked_add(count)
            .filter(|end| *end <= MAX_BLOCKS)
            .ok_or(OfficeError::LimitExceeded("document logical blocks"))
    }
    pub fn row(&self, count: u32) -> Result<Row, OfficeError> {
        self.end(count)?;
        Ok(Row::new(self.selected_repeat(count)))
    }
    /// Advance unselected structural rows without constructing a block or cells.
    pub fn skip(&mut self, count: u32) -> Result<bool, OfficeError> {
        let end = self.end(count)?;
        if self.selected_repeat(count) {
            return Err(OfficeError::Malformed(
                "missing selected row materialization",
            ));
        }
        self.pending_omitted = false;
        if self.cursor >= self.window.start.saturating_add(BLOCKS) {
            self.window.next = Some(self.cursor);
            return Ok(false);
        }
        self.cursor = end;
        Ok(true)
    }
    /// Returns false after a bounded lookahead establishes that another window exists.
    pub fn push(&mut self, block: Block, count: u32) -> Result<bool, OfficeError> {
        let end = self.end(count)?;
        let first = self.cursor.max(self.window.start);
        let last = end.min(self.window.start.saturating_add(BLOCKS));
        let escaped = |text: &str| {
            text.chars()
                .map(|c| match c {
                    '&' => 5,
                    '<' | '>' => 4,
                    '"' => 6,
                    '\'' => 5,
                    _ => c.len_utf8(),
                })
                .sum::<usize>()
        };
        // Conservative per-block markup and fixed-page reserve precede allocation.
        let html_cost = 1024
            + match &block {
                Block::Paragraph { text, .. } => escaped(text),
                Block::Row { cells, .. } => cells.iter().map(|text| escaped(text)).sum(),
            };
        for index in first..last {
            if self.text_bytes + block.bytes() > WINDOW_BYTES
                || self.html_bytes + html_cost > 1024 * 1024
            {
                self.window.next = Some(index);
                return Ok(false);
            }
            self.text_bytes += block.bytes();
            self.html_bytes += html_cost;
            self.window.omitted |= self.pending_omitted;
            self.window.blocks.push(block.clone());
        }
        self.cursor = end;
        self.pending_omitted = false;
        if end > self.window.start.saturating_add(BLOCKS) {
            self.window.next = Some(last);
        }
        Ok(self.window.next.is_none())
    }
    pub fn omit(&mut self) {
        // Belongs to the pending paragraph/row, including every repeated occurrence.
        // A rejected lookahead block must never mutate the admitted window.
        self.pending_omitted = true;
    }
    pub fn finish(mut self) -> Result<Window, OfficeError> {
        // Standalone unsupported objects at EOF have no paragraph/row to admit.
        if self.window.next.is_none() && self.selected() {
            self.window.omitted |= self.pending_omitted;
        }
        // A window of only objects renders a notice; it is never an empty window.
        Ok(self.window)
    }
}

pub(crate) fn render(
    catalog: &OfficeCatalog,
    target: OfficeTarget,
    window: &Window,
) -> Result<Vec<u8>, OfficeError> {
    let title = match target {
        OfficeTarget::DocumentWindow { document, .. } => catalog
            .document
            .as_ref()
            .filter(|entry| entry.id == document)
            .map(|entry| entry.name.as_str()),
        OfficeTarget::SlideWindow { slide, .. } => catalog
            .slides
            .get(usize::from(slide.0))
            .map(|entry| entry.name.as_str()),
        _ => None,
    }
    .ok_or(OfficeError::InvalidTarget)?;
    let mut html = Html::new();
    html.raw("<h1>")?;
    html.text(title)?;
    html.raw("</h1><nav aria-label=\"Contents\">")?;
    html.link(OfficeTarget::Catalog, "All contents")?;
    if let OfficeTarget::SlideWindow { slide, .. } = target {
        if slide.0 > 0 {
            html.link(
                OfficeTarget::SlideWindow {
                    slide: SlideId(slide.0 - 1),
                    block: 0,
                },
                "Previous slide",
            )?;
        }
        if usize::from(slide.0) + 1 < catalog.slides.len() {
            html.link(
                OfficeTarget::SlideWindow {
                    slide: SlideId(slide.0 + 1),
                    block: 0,
                },
                "Next slide",
            )?;
        }
    }
    let with_block = |block| match target {
        OfficeTarget::DocumentWindow { document, .. } => {
            OfficeTarget::DocumentWindow { document, block }
        }
        OfficeTarget::SlideWindow { slide, .. } => OfficeTarget::SlideWindow { slide, block },
        _ => target,
    };
    if window.start > 0 {
        html.link(
            with_block(window.start.saturating_sub(BLOCKS)),
            "Previous contents",
        )?;
    }
    if let Some(next) = window.next {
        html.link(with_block(next), "Next contents")?;
    }
    html.raw("</nav>")?;
    let text = window.blocks.iter().any(Block::has_text);
    match (window.omitted, text, target) {
        (true, false, OfficeTarget::SlideWindow { .. }) => html.raw(
            "<p>This slide contains only pictures, charts or other objects that the preview \
             does not show.</p>",
        )?,
        (true, false, _) => html.raw(
            "<p>This part of the document contains only pictures, charts or other objects \
             that the preview does not show.</p>",
        )?,
        (true, true, _) => {
            html.raw("<p>Some drawings, objects or unsupported layout contents are omitted.</p>")?
        }
        (false, false, _) => html.raw("<p>Empty content window.</p>")?,
        (false, true, _) => (),
    }
    let mut table = false;
    for block in &window.blocks {
        match block {
            Block::Paragraph {
                text,
                heading,
                list,
            } => {
                if table {
                    html.raw("</tbody></table>")?;
                    table = false;
                }
                let tag = if *heading > 0 {
                    format!("h{}", heading.min(&6))
                } else {
                    "p".into()
                };
                html.raw(&format!("<{tag} style=\"white-space:pre-wrap\">"))?;
                if *list {
                    html.raw("&#8226; ")?;
                }
                html.text(text)?;
                html.raw(&format!("</{tag}>"))?;
            }
            Block::Row {
                cells,
                omitted_columns,
            } => {
                if !table {
                    html.raw("<table><tbody>")?;
                    table = true;
                }
                html.raw("<tr>")?;
                for cell in cells {
                    html.raw("<td>")?;
                    html.text(cell)?;
                    html.raw("</td>")?;
                }
                if *omitted_columns {
                    html.raw("<td>[additional columns omitted]</td>")?;
                }
                html.raw("</tr>")?;
            }
        }
    }
    if table {
        html.raw("</tbody></table>")?;
    }
    html.finish()
}
