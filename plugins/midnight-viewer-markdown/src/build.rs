//! Parser events to inert semantic blocks with exact source anchors.
use crate::{
    decode::{self, Decoded},
    fence::{self, Kind, OpenBlock},
    markdown::link_target,
};
use midnight_plugin_sdk::api::{document::*, viewer::*};
use pulldown_cmark::{Alignment, CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;

/// Parser events of one top-level unit before that unit is shown literally.
const UNIT_EVENTS: usize = 16384;
/// Parser events built per parse. Later units are shown as one literal block,
/// while their complete top-level boundaries are still recorded.
const PARSE_EVENTS: usize = 65536;
/// Nested parser tags (containers and inline styles) within one unit.
const MAX_NESTING: usize = 16;
/// Minimum distance between the checkpoints recorded from one parse.
const CHECKPOINT_SPACING: u64 = 1024;
/// Parser events between cancellation polls; each poll may call the host.
const POLL_EVENTS: usize = 256;

fn parser(text: &str) -> Parser<'_> {
    Parser::new_ext(
        text,
        Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH,
    )
}

/// One parsed read window.
pub(crate) struct Parsed {
    pub page: DocumentPage,
    /// Complete top-level boundaries after the read start, ascending and at
    /// least [`CHECKPOINT_SPACING`] apart, plus the last proven boundary.
    pub checkpoints: Vec<u64>,
    /// Source end of the last top-level unit the parser proved complete in this
    /// read, or `u64::MAX` at EOF. Later blocks may continue past the read
    /// edge, so they are never presented as complete layout units.
    pub proven_end: u64,
    /// A top-level fenced code block, GFM table or HTML block of type 6 or 7
    /// cut by the read edge, whose end later plain reads can find.
    pub open_block: Option<OpenBlock>,
}

enum Container {
    Quote,
    /// `ordinal` is the displayed number and delimiter of an ordered item and
    /// `marker` its source marker; `first` holds until the item's first block.
    Item {
        ordinal: Option<(u64, char)>,
        marker: Range<usize>,
        first: bool,
    },
}

/// A top-level parser unit: a depth-0 tag with all its events, or a rule.
struct Unit {
    start: usize,
    blocks: usize,
    events: usize,
    literal: bool,
    /// The kind of a fenced code block, table or HTML block of type 6 or 7.
    kind: Option<Kind>,
}

pub(crate) fn render_raw(
    decoded: &Decoded,
    revision: u64,
    eof: bool,
    cancelled: impl Fn() -> bool,
) -> Option<Parsed> {
    if cancelled() {
        return None;
    }
    // Every decoder caps its text at 64 KiB, which also bounds parser work.
    debug_assert!(decoded.text.len() <= 65536);
    let start = decoded.anchors.first()?.source_offset;
    let end = decoded.anchors.last()?.source_offset;
    let mut builder = Builder::new(decoded, revision, eof, start, end);
    let mut checkpoints: Vec<u64> = Vec::new();
    let mut skipped = None;
    let mut proven = start;
    let mut depth = 0usize;
    let mut unit: Option<Unit> = None;
    let mut unbuilt: Option<usize> = None;
    let mut open_block = None;
    for (count, (event, range)) in parser(&decoded.text).into_offset_iter().enumerate() {
        if count.is_multiple_of(POLL_EVENTS) && cancelled() {
            return None;
        }
        if depth == 0 {
            if unbuilt.is_none() && count >= PARSE_EVENTS {
                unbuilt = Some(range.start);
            }
            let kind = match &event {
                Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(_))) => {
                    fence::opening(decoded.text.get(range.start..)?)
                        .map(|(marker, length)| Kind::Fence { marker, length })
                }
                Event::Start(Tag::Table(_)) => Some(Kind::Table),
                Event::Start(Tag::HtmlBlock) => {
                    fence::html_block_kind(decoded.text.get(range.start..)?)
                }
                _ => None,
            };
            unit = Some(Unit {
                start: range.start,
                blocks: builder.page.blocks.len(),
                events: 0,
                literal: false,
                kind,
            });
        }
        match event {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth = depth.saturating_sub(1),
            _ => {}
        }
        let current = unit.as_mut()?;
        current.events += 1;
        if unbuilt.is_none() && !current.literal {
            // An over-budget unit is shown literally on its own; completed
            // units before it and later units keep their semantic blocks.
            let fits = current.events <= UNIT_EVENTS
                && depth <= MAX_NESTING
                && builder.event(event, range.clone())?;
            if !fits {
                current.literal = true;
                builder.discard(current.blocks);
            }
        }
        if depth != 0 {
            continue;
        }
        let finished = unit.take()?;
        if unbuilt.is_none() {
            if finished.literal {
                builder.literal(finished.start..range.end)?;
            }
            builder.flush();
            builder.reset();
        }
        // A unit is complete only when the line after it is complete too, or at
        // EOF: the parser takes a partial last line, such as the indentation of
        // the next row, as if it ended the input. A unit ending at the read edge
        // or before such a line may continue in the next read.
        let complete = eof
            || decoded
                .text
                .get(range.end..)
                .is_some_and(|rest| rest.contains('\n'));
        if !complete {
            if let Some(kind) = finished.kind {
                open_block = continued_block(decoded, finished.start, kind);
            }
        } else if range.end < decoded.text.len() {
            // Rendering already traverses the exact parser events. Record complete
            // top-level boundaries here rather than parsing the same window again.
            let offset = decoded.source_at(range.end)?;
            if unbuilt.is_none() {
                proven = offset;
            }
            let last = checkpoints.last().copied().unwrap_or(start);
            if offset >= last.saturating_add(CHECKPOINT_SPACING) {
                checkpoints.push(offset);
                skipped = None;
            } else if offset > last {
                skipped = Some(offset);
            }
        }
    }
    checkpoints.extend(skipped);
    if let Some(open) = unit.take()
        && unbuilt.is_none()
        && open.literal
    {
        builder.literal(open.start..decoded.text.len())?;
    }
    if let Some(at) = unbuilt {
        builder.literal(at..decoded.text.len())?;
    }
    builder.flush();
    let mut page = builder.page;
    for block in &mut page.blocks {
        block.text.has_before = block.text.source.offset != 0;
        block.text.has_after = !eof || block.text.source.end().ok()? < end;
    }
    Some(Parsed {
        page,
        checkpoints,
        proven_end: if eof { u64::MAX } else { proven },
        open_block,
    })
}

/// A fence, table or HTML block unit cut by the read edge: every complete line
/// after its opening line belongs to it, except possibly the last complete
/// line, which may end it exactly at the edge (a closing fence or a blank line)
/// or was judged before the next line was complete; the scan resumes there.
fn continued_block(decoded: &Decoded, start: usize, kind: Kind) -> Option<OpenBlock> {
    let text = &decoded.text;
    let content = start + text.get(start..)?.find('\n')? + 1;
    let complete = &text[..text.rfind('\n')?];
    let last = complete.rfind('\n').map_or(0, |at| at + 1).max(content);
    Some(OpenBlock {
        start: decoded.source_at(start)?,
        kind,
        resume: decoded.source_at(last)?,
    })
}

struct Builder<'a> {
    decoded: &'a Decoded,
    revision: u64,
    eof: bool,
    /// Provisional source of a block until its first displayed text.
    origin: u64,
    page: DocumentPage,
    active: Option<Block>,
    containers: Vec<Container>,
    lists: Vec<Option<u64>>,
    strong: u8,
    emphasis: u8,
    strike: u8,
    images: u8,
    columns: Vec<u8>,
    table_start: usize,
    pending_link: Option<(u32, String)>,
}

impl<'a> Builder<'a> {
    fn new(decoded: &'a Decoded, revision: u64, eof: bool, start: u64, end: u64) -> Self {
        Self {
            decoded,
            revision,
            eof,
            origin: start,
            page: DocumentPage {
                revision,
                source: SourceRange {
                    offset: start,
                    length: end - start,
                },
                blocks: Vec::new(),
                has_before: start != 0,
                has_after: !eof,
            },
            active: None,
            containers: Vec::new(),
            lists: Vec::new(),
            strong: 0,
            emphasis: 0,
            strike: 0,
            images: 0,
            columns: Vec::new(),
            table_start: 0,
            pending_link: None,
        }
    }

    /// Build one event. `Some(false)` means the current unit exceeds a
    /// semantic budget and must be shown literally; `None` is an invariant
    /// failure of the parser's source ranges.
    fn event(&mut self, event: Event<'_>, range: Range<usize>) -> Option<bool> {
        match event {
            Event::Start(tag) => return self.start(tag, range),
            Event::End(tag) => self.end(tag, range)?,
            Event::Rule => {
                self.flush();
                self.child_block()?;
                let source = self.decoded.source_at(range.start)?;
                // Empty semantic rule, represented by one exact source point.
                let rule = self.block(BLOCK_RULE, 0, source);
                self.page.blocks.push(rule);
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                let decoded = self.decoded;
                let linked = self.pending_link.is_some();
                let flags = self.flags();
                let current = self.current()?;
                let role = if current.kind == BLOCK_CODE {
                    STYLE_CODE
                } else if current.kind == BLOCK_HEADING {
                    STYLE_HEADING
                } else if linked {
                    STYLE_LINK
                } else {
                    STYLE_PLAIN
                };
                append(current, decoded, &text, range, role, flags)?;
            }
            Event::Code(text) => {
                let decoded = self.decoded;
                let flags = u8::from(self.strong != 0) | (u8::from(self.strike != 0) << 3);
                let current = self.current()?;
                append(current, decoded, &text, range, STYLE_CODE, flags)?;
            }
            Event::TaskListMarker(checked) => {
                let decoded = self.decoded;
                let current = self.current()?;
                // The checkbox is the marker; legacy hosts must not add a
                // second bullet. Keep the nesting depth and exact source map.
                current.kind = BLOCK_PARAGRAPH;
                append(
                    current,
                    decoded,
                    // Plain markers stay readable in user-selected text fonts;
                    // Unicode ballot boxes can become emoji or missing glyphs.
                    if checked { "[x] " } else { "[ ] " },
                    range,
                    STYLE_PLAIN,
                    0,
                )?;
            }
            Event::SoftBreak | Event::HardBreak => {
                let decoded = self.decoded;
                if let Some(current) = self.active.as_mut() {
                    let separator = if matches!(event, Event::SoftBreak) {
                        " "
                    } else {
                        "\n"
                    };
                    append(current, decoded, separator, range, STYLE_PLAIN, 0)?;
                }
            }
            _ => {}
        }
        Some(true)
    }

    fn start(&mut self, tag: Tag<'_>, range: Range<usize>) -> Option<bool> {
        match tag {
            Tag::Table(alignments) => {
                self.flush();
                self.child_block()?;
                if alignments.len() > MAX_TABLE_COLUMNS {
                    return Some(false);
                }
                self.table_start = self.page.blocks.len();
                self.columns = alignments
                    .into_iter()
                    .map(|alignment| match alignment {
                        Alignment::Center => TABLE_ALIGN_CENTER,
                        Alignment::Right => TABLE_ALIGN_RIGHT,
                        _ => TABLE_ALIGN_LEFT,
                    })
                    .collect();
            }
            Tag::TableHead | Tag::TableRow => {
                self.flush();
                let source = self.decoded.source_at(range.start)?;
                let mut row = self.block(BLOCK_PARAGRAPH, 0, source);
                row.table = Some(TableRow {
                    cells: Vec::new(),
                    header: tag == Tag::TableHead,
                });
                self.active = Some(row);
            }
            Tag::TableCell => {
                let decoded = self.decoded;
                let row = self.active.as_mut()?;
                let column = row.table.as_ref()?.cells.len();
                if column >= self.columns.len() {
                    return Some(false);
                }
                if column != 0 {
                    append(row, decoded, "\t", range.start..range.start, STYLE_PLAIN, 0)?;
                } else if row.text.text.is_empty() {
                    row.text.source.offset = decoded.source_at(range.start)?;
                    row.text.anchors[0].source_offset = row.text.source.offset;
                }
                let start = row.text.text.len() as u32;
                row.table.as_mut()?.cells.push(TableCell {
                    start,
                    end: start,
                    alignment: self.columns[column],
                });
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.child_block()?;
                self.containers.push(Container::Quote);
            }
            Tag::List(start) => {
                self.flush();
                self.child_block()?;
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let (marker, delimiter) = item_marker(&self.decoded.text, range.start);
                let ordinal = match self.lists.last_mut() {
                    Some(Some(next)) => {
                        let number = *next;
                        *next = next.saturating_add(1);
                        Some((number, delimiter))
                    }
                    _ => None,
                };
                self.containers.push(Container::Item {
                    ordinal,
                    marker,
                    first: true,
                });
            }
            Tag::Paragraph => {
                self.flush();
                let block = self.text_block()?;
                self.active = Some(block);
            }
            Tag::Heading { level, .. } => {
                self.flush();
                self.child_block()?;
                self.active = Some(self.block(BLOCK_HEADING, level as u8, self.origin));
            }
            Tag::CodeBlock(_) | Tag::HtmlBlock => {
                self.flush();
                self.child_block()?;
                self.active = Some(self.block(BLOCK_CODE, 0, self.origin));
            }
            Tag::Strong => self.strong = self.strong.saturating_add(1),
            Tag::Emphasis => self.emphasis = self.emphasis.saturating_add(1),
            Tag::Strikethrough => self.strike = self.strike.saturating_add(1),
            Tag::Image { .. } => self.images = self.images.saturating_add(1),
            Tag::Link { dest_url, .. } if self.images == 0 && link_target(&dest_url) => {
                // The block that receives the link's text exists first: an ordered item's
                // first block starts with its number, which the link must not cover.
                let at = self.current()?.text.text.len() as u32;
                self.pending_link = Some((at, dest_url.into_string()));
            }
            _ => {}
        }
        Some(true)
    }

    fn end(&mut self, tag: TagEnd, range: Range<usize>) -> Option<()> {
        match tag {
            TagEnd::TableCell => {
                let row = self.active.as_mut()?;
                row.table.as_mut()?.cells.last_mut()?.end = row.text.text.len() as u32;
            }
            TagEnd::TableHead | TagEnd::TableRow => self.flush(),
            TagEnd::Table => {
                self.flush();
                // The parser closes an unfinished table at the raw read
                // boundary too. Its missing rows could change every column.
                if !self.eof && range.end == self.decoded.text.len() {
                    for block in &mut self.page.blocks[self.table_start..] {
                        block.table = None;
                    }
                }
                self.columns.clear();
            }
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::CodeBlock | TagEnd::HtmlBlock => {
                self.flush();
            }
            TagEnd::Item | TagEnd::BlockQuote(_) => {
                self.flush();
                self.containers.pop();
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
            }
            TagEnd::Strong => self.strong = self.strong.saturating_sub(1),
            TagEnd::Emphasis => self.emphasis = self.emphasis.saturating_sub(1),
            TagEnd::Strikethrough => self.strike = self.strike.saturating_sub(1),
            TagEnd::Image => self.images = self.images.saturating_sub(1),
            TagEnd::Link => {
                if let (Some((start, target)), Some(block)) =
                    (self.pending_link.take(), self.active.as_mut())
                {
                    let end = block.text.text.len() as u32;
                    if start < end {
                        block.links.push(Link { start, end, target });
                    }
                }
            }
            _ => {}
        }
        Some(())
    }

    fn depth(&self) -> u8 {
        u8::try_from(self.containers.len()).unwrap_or(u8::MAX)
    }

    fn flags(&self) -> u8 {
        u8::from(self.strong != 0)
            | (u8::from(self.emphasis != 0) << 1)
            | (u8::from(self.strike != 0) << 3)
    }

    fn block(&self, kind: u16, level: u8, source: u64) -> Block {
        block(kind, self.depth(), level, self.revision, source)
    }

    /// The paragraph-like block receiving inline text; created on demand.
    fn current(&mut self) -> Option<&mut Block> {
        if self.active.is_none() {
            let block = self.text_block()?;
            self.active = Some(block);
        }
        self.active.as_mut()
    }

    /// The first text block of a list item carries its marker: the host's
    /// bullet for unordered items, or the displayed number of an ordered item
    /// (a paragraph, so no bullet is added). Later blocks of the same item are
    /// paragraphs at the item's depth.
    fn text_block(&mut self) -> Option<Block> {
        let mut number = None;
        let kind = match self.containers.last_mut() {
            Some(Container::Item {
                ordinal,
                marker,
                first,
            }) if *first => {
                *first = false;
                number = ordinal.map(|ordinal| (ordinal, marker.clone()));
                if number.is_some() {
                    BLOCK_PARAGRAPH
                } else {
                    BLOCK_LIST_ITEM
                }
            }
            Some(Container::Quote) => BLOCK_QUOTE,
            Some(Container::Item { .. }) | None => BLOCK_PARAGRAPH,
        };
        let mut block = self.block(kind, 0, self.origin);
        if let Some(((value, delimiter), marker)) = number {
            let text = format!("{value}{delimiter} ");
            append(&mut block, self.decoded, &text, marker, STYLE_PLAIN, 0)?;
        }
        Some(block)
    }

    /// A non-text block opening an ordered item keeps the item's number on a
    /// short line of its own; an unordered item then shows no bullet.
    fn child_block(&mut self) -> Option<()> {
        let mut number = None;
        if let Some(Container::Item {
            ordinal,
            marker,
            first,
        }) = self.containers.last_mut()
            && *first
        {
            *first = false;
            number = ordinal.map(|ordinal| (ordinal, marker.clone()));
        }
        if let Some(((value, delimiter), marker)) = number {
            let mut block = self.block(BLOCK_PARAGRAPH, 0, self.origin);
            let text = format!("{value}{delimiter} ");
            append(&mut block, self.decoded, &text, marker, STYLE_PLAIN, 0)?;
            self.page.blocks.push(block);
        }
        Some(())
    }

    fn flush(&mut self) {
        if let Some(block) = self
            .active
            .take()
            .filter(|b| !b.text.text.is_empty() || b.table.is_some())
        {
            self.page.blocks.push(block);
        }
    }

    fn reset(&mut self) {
        self.active = None;
        self.containers.clear();
        self.lists.clear();
        self.strong = 0;
        self.emphasis = 0;
        self.strike = 0;
        self.images = 0;
        self.columns.clear();
        self.pending_link = None;
    }

    /// Drop a unit's partial blocks before it is shown literally.
    fn discard(&mut self, blocks: usize) {
        self.page.blocks.truncate(blocks);
        self.reset();
    }

    /// The unit's source text as one readable paragraph with exact anchors.
    fn literal(&mut self, range: Range<usize>) -> Option<()> {
        let text = self.decoded.text.get(range.clone())?;
        let end = range.start + text.trim_end_matches('\n').len();
        if end > range.start {
            let block = fragment(self.decoded, self.revision, range.start..end)?;
            self.page.blocks.push(block);
        }
        Some(())
    }
}

/// The ordered-list marker (`12.` or `3)`) at the start of an item, with its
/// delimiter; an empty range when the item has no numeric marker there.
fn item_marker(text: &str, start: usize) -> (Range<usize>, char) {
    let rest = text.get(start..).unwrap_or_default();
    let body = rest.trim_start_matches([' ', '\t']);
    let at = start + (rest.len() - body.len());
    let digits = body.bytes().take_while(u8::is_ascii_digit).count();
    match body.as_bytes().get(digits) {
        Some(&delimiter @ (b'.' | b')')) if digits > 0 => {
            (at..at + digits + 1, char::from(delimiter))
        }
        _ => (start..start, '.'),
    }
}

/// A decoded text range as one plain paragraph with every exact anchor.
pub(crate) fn fragment(decoded: &Decoded, revision: u64, range: Range<usize>) -> Option<Block> {
    let first = decoded
        .anchors
        .partition_point(|a| (a.text_offset as usize) < range.start);
    let last = decoded
        .anchors
        .partition_point(|a| (a.text_offset as usize) <= range.end);
    let anchors: Vec<Anchor> = decoded
        .anchors
        .get(first..last)?
        .iter()
        .map(|a| Anchor {
            text_offset: a.text_offset - range.start as u32,
            source_offset: a.source_offset,
        })
        .collect();
    let offset = anchors.first()?.source_offset;
    let length = anchors.last()?.source_offset.checked_sub(offset)?;
    Some(Block {
        kind: BLOCK_PARAGRAPH,
        depth: 0,
        heading_level: 0,
        links: Vec::new(),
        table: None,
        strikethrough: None,
        text: TextPage {
            revision,
            source: SourceRange { offset, length },
            text: decoded.text.get(range)?.to_owned(),
            spans: Vec::new(),
            anchors,
            has_before: offset != 0,
            has_after: false,
            partial_line: false,
        },
    })
}

fn block(kind: u16, depth: u8, level: u8, revision: u64, source: u64) -> Block {
    Block {
        kind,
        depth,
        heading_level: level,
        links: Vec::new(),
        table: None,
        strikethrough: None,
        text: TextPage {
            revision,
            source: SourceRange {
                offset: source,
                length: 0,
            },
            text: String::new(),
            spans: Vec::new(),
            anchors: vec![Anchor {
                text_offset: 0,
                source_offset: source,
            }],
            has_before: source != 0,
            has_after: false,
            partial_line: false,
        },
    }
}

fn append(
    block: &mut Block,
    decoded: &Decoded,
    rendered: &str,
    range: Range<usize>,
    role: u16,
    flags: u8,
) -> Option<()> {
    if rendered.is_empty() {
        return Some(());
    }
    // Numeric character references are decoded after the source substitution:
    // display controls and bidirectional overrides like raw source ones.
    let rendered = decode::visible_text(rendered);
    let rendered = rendered.as_ref();
    let raw = decoded.text.get(range.clone())?;
    let exact = if raw == rendered {
        Some(range.start)
    } else if role == STYLE_CODE {
        // Code spans display each line ending as a space; both are one byte,
        // so the displayed scalars still map exactly onto source scalars.
        raw.find(rendered)
            .or_else(|| {
                raw.contains('\n')
                    .then(|| raw.replace('\n', " ").find(rendered))
                    .flatten()
            })
            .map(|at| range.start + at)
    } else if raw.strip_prefix('\\') == Some(rendered) {
        Some(range.start + 1)
    } else {
        None
    };
    let start = exact.unwrap_or(range.start);
    let end = exact.map_or(range.end, |start| start + rendered.len());
    let source_start = decoded.source_at(start)?;
    let source_end = decoded.source_at(end)?;
    let text = &mut block.text;
    let base = text.text.len() as u32;
    if base == 0 {
        text.source.offset = source_start;
        text.anchors[0].source_offset = source_start;
    }
    // A removed marker creates two source choices at one displayed boundary.
    // Keep the previous exact boundary; never fabricate/interpolate an offset.
    text.text.push_str(rendered);
    if let Some(exact) = exact {
        // An exact rendering repeats decoded scalars one to one, so the decoded
        // anchors after `exact` are its boundaries in order: step through them
        // instead of searching once per scalar.
        let mut index = decoded
            .anchors
            .binary_search_by_key(&(exact as u32), |a| a.text_offset)
            .ok()?;
        for (at, ch) in rendered.char_indices() {
            let end = exact + at + ch.len_utf8();
            index += 1;
            let anchor = decoded
                .anchors
                .get(index)
                .filter(|anchor| anchor.text_offset as usize == end)?;
            text.anchors.push(Anchor {
                text_offset: base + (end - exact) as u32,
                source_offset: anchor.source_offset,
            });
        }
    } else {
        // Entities and normalized code can map multiple source scalars to one
        // displayed span. Only event endpoints are exact; no guessed interior.
        text.anchors.push(Anchor {
            text_offset: text.text.len() as u32,
            source_offset: source_end,
        });
    }
    text.source.length = source_end.checked_sub(text.source.offset)?;
    if flags & 8 != 0 {
        let ranges = block.strikethrough.get_or_insert_with(Vec::new);
        if let Some(last) = ranges.last_mut().filter(|last| last.end == base) {
            last.end = text.text.len() as u32;
        } else {
            ranges.push(TextRange {
                start: base,
                end: text.text.len() as u32,
            });
        }
    }
    if role != STYLE_PLAIN || flags & 7 != 0 {
        let end = text.text.len() as u32;
        // Adjacent events with one style (each fenced-code line, say) share a
        // span, keeping dense documents within the public span/item budgets.
        if let Some(last) = text
            .spans
            .last_mut()
            .filter(|last| last.end == base && last.role == role && last.flags == flags & 7)
        {
            last.end = end;
        } else {
            text.spans.push(Span {
                start: base,
                end,
                role,
                flags: flags & 7,
            });
        }
    }
    Some(())
}
