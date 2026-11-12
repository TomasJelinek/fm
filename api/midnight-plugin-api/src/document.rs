//! Safe semantic document data. Links are metadata, never an automatic action.
use crate::{
    codec::Model,
    messages::{check, limit},
    viewer::{self, SourceRange, TextPage},
    wire::WireError,
};
use minicbor::{Decode, Encode};

pub const BLOCK_PARAGRAPH: u16 = 1;
pub const BLOCK_HEADING: u16 = 2;
pub const BLOCK_CODE: u16 = 3;
pub const BLOCK_QUOTE: u16 = 4;
pub const BLOCK_RULE: u16 = 5;
pub const BLOCK_LIST_ITEM: u16 = 6;
pub const MAX_BLOCKS: usize = 512;
pub const MAX_TABLE_COLUMNS: usize = 64;
pub const MAX_TABLE_CELLS: usize = 4096;
pub const TABLE_ALIGN_LEFT: u8 = 0;
pub const TABLE_ALIGN_CENTER: u8 = 1;
pub const TABLE_ALIGN_RIGHT: u8 = 2;

/// A UTF-8 range in the containing block's decoded text.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct TextRange {
    #[n(0)]
    pub start: u32,
    #[n(1)]
    pub end: u32,
}

/// Cell ranges exclude the row's plain-text separators and may be empty.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct TableCell {
    #[n(0)]
    pub start: u32,
    #[n(1)]
    pub end: u32,
    #[n(2)]
    pub alignment: u8,
}

/// Adjacent annotated paragraph blocks form a table. Each remains independently
/// bounded and anchored, allowing tables to continue across document pages.
#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct TableRow {
    #[n(0)]
    pub cells: Vec<TableCell>,
    #[n(1)]
    pub header: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Link {
    #[n(0)]
    pub start: u32,
    #[n(1)]
    pub end: u32,
    /// The host separately checks scheme/intent before an explicit user action.
    #[n(2)]
    pub target: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct Block {
    #[n(0)]
    pub kind: u16,
    #[n(1)]
    pub depth: u8,
    #[n(2)]
    pub heading_level: u8,
    #[n(3)]
    pub text: TextPage,
    #[n(4)]
    pub links: Vec<Link>,
    /// Additive metadata: older hosts display the paragraph's plain-text row.
    #[n(5)]
    pub table: Option<TableRow>,
    /// Additive decoration ranges; legacy hosts safely omit the decoration.
    #[n(6)]
    pub strikethrough: Option<Vec<TextRange>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Encode, Decode)]
#[cbor(map)]
pub struct DocumentPage {
    #[n(0)]
    pub revision: u64,
    #[n(1)]
    pub source: SourceRange,
    #[n(2)]
    pub blocks: Vec<Block>,
    #[n(3)]
    pub has_before: bool,
    #[n(4)]
    pub has_after: bool,
}

impl Model for DocumentPage {
    fn validate(&self) -> Result<(), WireError> {
        limit(self.blocks.len() <= MAX_BLOCKS)?;
        let end = self.source.end()?;
        let mut previous_end = self.source.offset;
        let mut totals = Totals::default();
        for block in &self.blocks {
            block.text.validate()?;
            check(
                block.text.revision == self.revision && block.text.source.offset >= previous_end,
            )?;
            previous_end = block.text.source.end()?;
            check(previous_end <= end)?;
            block.validate()?;
            totals.add(block)?;
        }
        Ok(())
    }
}

impl Block {
    fn validate(&self) -> Result<(), WireError> {
        check(
            (BLOCK_PARAGRAPH..=BLOCK_LIST_ITEM).contains(&self.kind)
                && self.depth <= 16
                && if self.kind == BLOCK_HEADING {
                    (1..=6).contains(&self.heading_level)
                } else {
                    self.heading_level == 0
                },
        )?;
        limit(self.links.len() <= viewer::MAX_SPANS)?;
        if let Some(row) = &self.table {
            check(self.kind == BLOCK_PARAGRAPH)?;
            limit((1..=MAX_TABLE_COLUMNS).contains(&row.cells.len()))?;
            let mut previous_end = 0;
            for cell in &row.cells {
                check(
                    cell.start >= previous_end
                        && cell.alignment <= TABLE_ALIGN_RIGHT
                        && self
                            .text
                            .text
                            .get(cell.start as usize..cell.end as usize)
                            .is_some(),
                )?;
                previous_end = cell.end;
            }
        }
        if let Some(ranges) = &self.strikethrough {
            limit(ranges.len() <= viewer::MAX_SPANS)?;
            let mut previous_end = 0;
            for range in ranges {
                check(
                    range.start >= previous_end
                        && range.start < range.end
                        && self
                            .text
                            .text
                            .get(range.start as usize..range.end as usize)
                            .is_some(),
                )?;
                previous_end = range.end;
            }
        }
        let mut end = 0;
        for link in &self.links {
            check(
                link.start >= end
                    && link.start < link.end
                    && self
                        .text
                        .text
                        .get(link.start as usize..link.end as usize)
                        .is_some()
                    && !link.target.chars().any(char::is_control),
            )?;
            limit((1..=8192).contains(&link.target.len()))?;
            end = link.end;
        }
        Ok(())
    }
}

#[derive(Default)]
struct Totals {
    text: usize,
    spans: usize,
    anchors: usize,
    links: usize,
    targets: usize,
    cells: usize,
    decorations: usize,
}

impl Totals {
    fn add(&mut self, block: &Block) -> Result<(), WireError> {
        // Each contribution was bounded first, and there are at most 512 blocks.
        self.text += block.text.text.len();
        self.spans += block.text.spans.len();
        self.anchors += block.text.anchors.len();
        self.links += block.links.len();
        self.cells += block.table.as_ref().map_or(0, |row| row.cells.len());
        self.decorations += block.strikethrough.as_ref().map_or(0, Vec::len);
        self.targets += block
            .links
            .iter()
            .map(|link| link.target.len())
            .sum::<usize>();
        limit(
            self.text <= viewer::MAX_PAGE_BYTES
                && self.spans <= viewer::MAX_SPANS
                && self.anchors <= viewer::MAX_ANCHORS
                && self.links <= viewer::MAX_SPANS
                && self.cells <= MAX_TABLE_CELLS
                && self.decorations <= viewer::MAX_SPANS
                && self.targets <= 64 * 1024,
        )
    }
}
