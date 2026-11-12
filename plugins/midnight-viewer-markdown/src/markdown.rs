//! CommonMark to inert semantic blocks. This module has no I/O capability.
use crate::decode::{self, Decoded};
use midnight_plugin_sdk::api::{codec, document::*, viewer::*, wire::WireError};
use pulldown_cmark::{Alignment, Event, Options, Parser, Tag, TagEnd};
use std::ops::Range;

fn parser(text: &str) -> Parser<'_> {
    Parser::new_ext(
        text,
        Options::ENABLE_TABLES | Options::ENABLE_TASKLISTS | Options::ENABLE_STRIKETHROUGH,
    )
}

pub fn render(
    decoded: &Decoded,
    revision: u64,
    eof: bool,
    cancelled: impl Fn() -> bool,
) -> Option<DocumentPage> {
    render_window(
        decoded,
        revision,
        eof,
        decoded.anchors.first()?.source_offset,
        8192,
        usize::MAX,
        cancelled,
    )
}

pub fn render_window(
    decoded: &Decoded,
    revision: u64,
    eof: bool,
    anchor: u64,
    max_bytes: usize,
    max_lines: usize,
    cancelled: impl Fn() -> bool,
) -> Option<DocumentPage> {
    render_slice(
        decoded,
        revision,
        eof,
        Window {
            anchor,
            backwards: false,
            max_bytes,
            max_lines,
        },
        cancelled,
    )
    .map(|(page, _)| page)
}

pub(crate) struct Window {
    pub anchor: u64,
    pub backwards: bool,
    pub max_bytes: usize,
    pub max_lines: usize,
}

impl Window {
    fn crop(&self, mut page: DocumentPage) -> DocumentPage {
        // Scalar paging cannot preserve a table's column measurements. Render
        // oversized units as wrapped row text rather than unstable partial tables.
        for block in &mut page.blocks {
            block.table = None;
        }
        if self.backwards {
            crop_backward(page, self.anchor, self.max_bytes.min(8192), self.max_lines)
        } else {
            crop(page, self.anchor, self.max_bytes.min(8192), self.max_lines)
        }
    }
}

pub(crate) fn render_slice(
    decoded: &Decoded,
    revision: u64,
    eof: bool,
    window: Window,
    cancelled: impl Fn() -> bool,
) -> Option<(DocumentPage, Vec<u64>)> {
    let (raw, checkpoints) = render_context(decoded, revision, eof, &cancelled)?;
    let mut page =
        crate::semantic_window::crop(raw, &window).unwrap_or_else(|raw| window.crop(raw));
    if !fits_wire(&mut page) {
        page = window.crop(plain(decoded, revision, eof));
    }
    Some((page, checkpoints))
}

pub(crate) fn render_context(
    decoded: &Decoded,
    revision: u64,
    eof: bool,
    cancelled: impl Fn() -> bool,
) -> Option<(DocumentPage, Vec<u64>)> {
    let mut checkpoints = Vec::new();
    let raw = render_raw(decoded, revision, eof, &mut checkpoints, &cancelled)?;
    Some((raw, checkpoints))
}

pub(crate) fn crop_context(raw: &DocumentPage, window: &Window) -> Option<DocumentPage> {
    let mut page =
        crate::semantic_window::crop_ref(raw, window).unwrap_or_else(|| window.crop(raw.clone()));
    if !fits_wire(&mut page) {
        // A complete layout unit can fit the read budget while exceeding the
        // public exact-anchor/item budget. Preserve exact selection boundaries
        // and use the same bounded prose fallback as an oversized raw unit.
        page = window.crop(page);
    }
    fits_wire(&mut page).then_some(page)
}

fn render_raw(
    decoded: &Decoded,
    revision: u64,
    eof: bool,
    checkpoints: &mut Vec<u64>,
    cancelled: impl Fn() -> bool,
) -> Option<DocumentPage> {
    if cancelled() {
        return None;
    }
    let start = decoded.anchors.first()?.source_offset;
    let end = decoded.anchors.last()?.source_offset;
    let mut page = DocumentPage {
        revision,
        source: SourceRange {
            offset: start,
            length: end - start,
        },
        blocks: Vec::new(),
        has_before: start != 0,
        has_after: !eof,
    };
    if decoded.text.len() > 65536 {
        return Some(plain(decoded, revision, eof));
    }
    let mut active: Option<Block> = None;
    let mut depth = 0usize;
    let mut quote = 0u8;
    let mut item = 0u8;
    let mut strong = 0u8;
    let mut emphasis = 0u8;
    let mut strike = 0u8;
    let mut images = 0u8;
    let mut columns = Vec::new();
    let mut table_start = 0;
    let mut pending_link: Option<(u32, String)> = None;
    for (count, (event, range)) in parser(&decoded.text).into_offset_iter().enumerate() {
        if cancelled() {
            return None;
        }
        if count >= 16384 {
            return Some(plain(decoded, revision, eof));
        }
        match event {
            Event::Start(tag) => {
                depth += 1;
                if depth > 16 {
                    return Some(plain(decoded, revision, eof));
                }
                match tag {
                    Tag::Table(alignments) => {
                        flush(&mut active, &mut page);
                        table_start = page.blocks.len();
                        if alignments.len() > MAX_TABLE_COLUMNS {
                            return Some(plain(decoded, revision, eof));
                        }
                        columns = alignments
                            .into_iter()
                            .map(|alignment| match alignment {
                                Alignment::Center => TABLE_ALIGN_CENTER,
                                Alignment::Right => TABLE_ALIGN_RIGHT,
                                _ => TABLE_ALIGN_LEFT,
                            })
                            .collect();
                    }
                    Tag::TableHead | Tag::TableRow => {
                        flush(&mut active, &mut page);
                        let mut row = block(
                            BLOCK_PARAGRAPH,
                            item + quote,
                            0,
                            revision,
                            decoded.source_at(range.start)?,
                        );
                        row.table = Some(TableRow {
                            cells: Vec::new(),
                            header: tag == Tag::TableHead,
                        });
                        active = Some(row);
                    }
                    Tag::TableCell => {
                        let row = active.as_mut()?;
                        let column = row.table.as_ref()?.cells.len();
                        if column >= columns.len() {
                            return Some(plain(decoded, revision, eof));
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
                            alignment: columns[column],
                        });
                    }
                    Tag::BlockQuote(_) => {
                        flush(&mut active, &mut page);
                        quote += 1;
                    }
                    Tag::List(_) => flush(&mut active, &mut page),
                    Tag::Item => {
                        flush(&mut active, &mut page);
                        item += 1;
                        active = Some(block(BLOCK_LIST_ITEM, item + quote, 0, revision, start));
                    }
                    Tag::Paragraph => {
                        flush(&mut active, &mut page);
                        let kind = if item != 0 {
                            BLOCK_LIST_ITEM
                        } else if quote != 0 {
                            BLOCK_QUOTE
                        } else {
                            BLOCK_PARAGRAPH
                        };
                        active = Some(block(kind, item + quote, 0, revision, start));
                    }
                    Tag::Heading { level, .. } => {
                        flush(&mut active, &mut page);
                        active = Some(block(
                            BLOCK_HEADING,
                            item + quote,
                            level as u8,
                            revision,
                            start,
                        ));
                    }
                    Tag::CodeBlock(_) | Tag::HtmlBlock => {
                        flush(&mut active, &mut page);
                        active = Some(block(BLOCK_CODE, item + quote, 0, revision, start));
                    }
                    Tag::Strong => strong += 1,
                    Tag::Emphasis => emphasis += 1,
                    Tag::Strikethrough => strike += 1,
                    Tag::Image { .. } => images += 1,
                    Tag::Link { dest_url, .. } if images == 0 && safe_link(&dest_url) => {
                        let at = active.as_ref().map_or(0, |b| b.text.text.len() as u32);
                        pending_link = Some((at, dest_url.into_string()));
                    }
                    _ => {}
                }
            }
            Event::End(tag) => {
                depth = depth.saturating_sub(1);
                // Rendering already traverses the exact parser events. Record
                // complete top-level boundaries here rather than parsing the
                // same 64 KiB window again after every viewport.
                if depth == 0
                    && checkpoints.len() < 256
                    && range.end < decoded.text.len()
                    && let Some(offset) = decoded.source_at(range.end)
                {
                    checkpoints.push(offset);
                }
                match tag {
                    TagEnd::TableCell => {
                        let row = active.as_mut()?;
                        row.table.as_mut()?.cells.last_mut()?.end = row.text.text.len() as u32;
                    }
                    TagEnd::TableHead | TagEnd::TableRow => flush(&mut active, &mut page),
                    TagEnd::Table => {
                        flush(&mut active, &mut page);
                        // The parser closes an unfinished table at the raw read
                        // boundary too. Its missing rows could change every column.
                        if !eof && range.end == decoded.text.len() {
                            for block in &mut page.blocks[table_start..] {
                                block.table = None;
                            }
                        }
                        columns.clear();
                    }
                    TagEnd::Paragraph
                    | TagEnd::Heading(_)
                    | TagEnd::CodeBlock
                    | TagEnd::HtmlBlock => flush(&mut active, &mut page),
                    TagEnd::Item => {
                        flush(&mut active, &mut page);
                        item = item.saturating_sub(1);
                    }
                    TagEnd::BlockQuote(_) => {
                        flush(&mut active, &mut page);
                        quote = quote.saturating_sub(1);
                    }
                    TagEnd::Strong => strong = strong.saturating_sub(1),
                    TagEnd::Emphasis => emphasis = emphasis.saturating_sub(1),
                    TagEnd::Strikethrough => strike = strike.saturating_sub(1),
                    TagEnd::Image => images = images.saturating_sub(1),
                    TagEnd::Link => {
                        if let (Some((start, target)), Some(block)) =
                            (pending_link.take(), active.as_mut())
                        {
                            let end = block.text.text.len() as u32;
                            if start < end {
                                block.links.push(Link { start, end, target });
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::Rule => {
                flush(&mut active, &mut page);
                let mut rule = block(
                    BLOCK_RULE,
                    item + quote,
                    0,
                    revision,
                    decoded.source_at(range.start)?,
                );
                // Empty semantic rule, represented by one exact source point.
                rule.text.has_after = !eof;
                page.blocks.push(rule);
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                let current = active.get_or_insert_with(|| {
                    block(BLOCK_PARAGRAPH, item + quote, 0, revision, start)
                });
                let role = if current.kind == BLOCK_CODE {
                    STYLE_CODE
                } else if current.kind == BLOCK_HEADING {
                    STYLE_HEADING
                } else if pending_link.is_some() {
                    STYLE_LINK
                } else {
                    STYLE_PLAIN
                };
                append(
                    current,
                    decoded,
                    &text,
                    range,
                    role,
                    u8::from(strong != 0)
                        | (u8::from(emphasis != 0) << 1)
                        | (u8::from(strike != 0) << 3),
                )?;
            }
            Event::Code(text) => {
                let current = active.get_or_insert_with(|| {
                    block(BLOCK_PARAGRAPH, item + quote, 0, revision, start)
                });
                append(
                    current,
                    decoded,
                    &text,
                    range,
                    STYLE_CODE,
                    u8::from(strong != 0) | (u8::from(strike != 0) << 3),
                )?;
            }
            Event::TaskListMarker(checked) => {
                let current = active.get_or_insert_with(|| {
                    block(BLOCK_LIST_ITEM, item + quote, 0, revision, start)
                });
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
                if let Some(current) = active.as_mut() {
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
    }
    flush(&mut active, &mut page);
    for block in &mut page.blocks {
        block.text.has_before = block.text.source.offset != 0;
        block.text.has_after = !eof || block.text.source.end().ok()? < end;
    }
    Some(page)
}

pub fn crop(
    mut page: DocumentPage,
    anchor: u64,
    max_bytes: usize,
    max_lines: usize,
) -> DocumentPage {
    let old_start = page.source.offset;
    let old_end = page.source.offset + page.source.length;
    let mut budget = max_bytes;
    let mut lines = max_lines.min(MAX_BLOCKS);
    let mut clipped = false;
    page.blocks = page
        .blocks
        .into_iter()
        .filter_map(|mut block| {
            if clipped || budget == 0 || lines == 0 {
                clipped = true;
                return None;
            }
            if block.text.source.offset + block.text.source.length < anchor {
                return None;
            }
            let start = block
                .text
                .anchors
                .iter()
                .find(|a| a.source_offset >= anchor)
                .map_or(block.text.text.len(), |a| a.text_offset as usize);
            let mut end = start;
            for ch in block.text.text[start..].chars() {
                if end - start + ch.len_utf8() > budget || lines == 0 {
                    break;
                }
                end += ch.len_utf8();
                if ch == '\n' {
                    lines -= 1;
                }
            }
            // Only a listed source boundary is selectable; transformed entities
            // may intentionally expose fewer boundaries than displayed scalars.
            end = block
                .text
                .anchors
                .iter()
                .take_while(|a| a.text_offset as usize <= end)
                .last()
                .map_or(start, |a| a.text_offset as usize);
            if end < block.text.text.len() {
                clipped = true;
            }
            if start == end && block.kind != BLOCK_RULE && block.table.is_none() {
                return None;
            }
            let text = &mut block.text;
            text.anchors
                .retain(|a| (start..=end).contains(&(a.text_offset as usize)));
            if text.anchors.is_empty() {
                return None;
            }
            for a in &mut text.anchors {
                a.text_offset -= start as u32;
            }
            text.source.offset = text.anchors[0].source_offset;
            text.source.length = text.anchors.last().unwrap().source_offset - text.source.offset;
            text.text = text.text[start..end].into();
            text.spans = text
                .spans
                .drain(..)
                .filter_map(|span| {
                    let left = (span.start as usize).max(start);
                    let right = (span.end as usize).min(end);
                    (left < right).then_some(Span {
                        start: left.saturating_sub(start) as u32,
                        end: right.saturating_sub(start) as u32,
                        role: span.role,
                        flags: span.flags,
                    })
                })
                .collect();
            block.links = block
                .links
                .into_iter()
                .filter_map(|link| {
                    let left = (link.start as usize).max(start);
                    let right = (link.end as usize).min(end);
                    (left < right).then_some(Link {
                        start: left.saturating_sub(start) as u32,
                        end: right.saturating_sub(start) as u32,
                        target: link.target,
                    })
                })
                .collect();
            if let Some(table) = &mut block.table {
                for cell in &mut table.cells {
                    cell.start = (cell.start as usize)
                        .clamp(start, end)
                        .saturating_sub(start) as u32;
                    cell.end = (cell.end as usize).clamp(start, end).saturating_sub(start) as u32;
                }
            }
            if let Some(ranges) = &mut block.strikethrough {
                *ranges = ranges
                    .iter()
                    .filter_map(|range| {
                        let left = (range.start as usize).max(start);
                        let right = (range.end as usize).min(end);
                        (left < right).then_some(TextRange {
                            start: left.saturating_sub(start) as u32,
                            end: right.saturating_sub(start) as u32,
                        })
                    })
                    .collect();
            }
            budget -= text.text.len();
            // Paragraphs, headings, list items and table rows have no literal
            // trailing newline, but each consumes at least one displayed line.
            // Counting only SoftBreak/HardBreak allowed hundreds of blocks into
            // a viewport requesting only a handful of lines.
            if !text.text.ends_with('\n') {
                lines = lines.saturating_sub(1);
            }
            Some(block)
        })
        .collect();
    // The document range describes consumed raw input, including invisible
    // Markdown syntax. Each block retains its own exact visible-text anchors.
    let start = anchor.max(old_start).min(old_end);
    let end = if clipped {
        page.blocks
            .last()
            .map_or(start, |b| b.text.source.offset + b.text.source.length)
    } else {
        old_end
    };
    page.source = SourceRange {
        offset: start,
        length: end.saturating_sub(start),
    };
    page.has_before |= start > old_start;
    page.has_after |= clipped;
    page
}

pub fn render_backward(
    decoded: &Decoded,
    revision: u64,
    eof: bool,
    upper: u64,
    max_bytes: usize,
    max_lines: usize,
    cancelled: impl Fn() -> bool,
) -> Option<DocumentPage> {
    render_slice(
        decoded,
        revision,
        eof,
        Window {
            anchor: upper,
            backwards: true,
            max_bytes,
            max_lines,
        },
        cancelled,
    )
    .map(|(page, _)| page)
}

fn fits_wire(page: &mut DocumentPage) -> bool {
    match codec::encode(page) {
        Ok(_) => true,
        Err(WireError::Limit) => {
            // Anchors, spans and links share one CBOR item quota. Optional
            // decoration is discarded before displayed text or exact anchors,
            // keeping document search consistent with the visible fallback.
            for block in &mut page.blocks {
                block.text.spans.clear();
                block.links.clear();
                block.strikethrough = None;
            }
            codec::encode(page).is_ok()
        }
        Err(_) => false,
    }
}

/// Select the nearest preceding displayed text, retaining the caller's source
/// upper bound through Markdown transformations and sparse entity anchors.
pub fn crop_backward(
    mut page: DocumentPage,
    upper: u64,
    max_bytes: usize,
    max_lines: usize,
) -> DocumentPage {
    let old_start = page.source.offset;
    let old_end = page.source.offset + page.source.length;
    let mut budget = max_bytes;
    let mut lines = max_lines.min(MAX_BLOCKS);
    let mut selected = Vec::new();
    let mut source_start = old_start.min(upper);
    for (index, block) in page.blocks.into_iter().enumerate().rev() {
        if budget == 0 || lines == 0 {
            break;
        }
        if block.text.source.offset >= upper {
            continue;
        }
        if block.kind == BLOCK_RULE {
            source_start = if index == 0 {
                old_start
            } else {
                block.text.source.offset
            };
            selected.push(block);
            lines -= 1;
            continue;
        }
        let Some(end_anchor) = block
            .text
            .anchors
            .iter()
            .take_while(|a| a.source_offset <= upper)
            .last()
        else {
            continue;
        };
        let end = end_anchor.text_offset as usize;
        let wanted = decode::suffix_start(&block.text.text, end, budget, lines);
        let Some(start_anchor) = block
            .text
            .anchors
            .iter()
            .find(|a| a.text_offset as usize >= wanted)
        else {
            continue;
        };
        let start = start_anchor.text_offset as usize;
        let block_start = start_anchor.source_offset;
        if start >= end {
            if end != 0 {
                // No complete displayed scalar fits the remaining budget.
                // Keep that content before the page instead of consuming it.
                source_start = end_anchor.source_offset;
                break;
            }
            continue;
        }
        let source = block.text.source.clone();
        let mut part = crop(
            DocumentPage {
                revision: page.revision,
                source,
                blocks: vec![block],
                has_before: true,
                has_after: true,
            },
            block_start,
            end - start,
            usize::MAX,
        );
        if let Some(block) = part.blocks.pop() {
            source_start = if index == 0 && start == 0 {
                old_start
            } else {
                block_start
            };
            budget -= block.text.text.len();
            lines = lines.saturating_sub(block.text.text.lines().count().max(1));
            selected.push(block);
        }
        if start != 0 {
            break;
        }
    }
    selected.reverse();
    let start = source_start;
    let end = upper.min(old_end).max(start);
    page.blocks = selected;
    page.source = SourceRange {
        offset: start,
        length: end - start,
    };
    page.has_before |= start > old_start;
    page.has_after |= end < old_end;
    page
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

fn flush(active: &mut Option<Block>, page: &mut DocumentPage) {
    if let Some(block) = active
        .take()
        .filter(|b| !b.text.text.is_empty() || b.table.is_some())
    {
        page.blocks.push(block);
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
    let raw = decoded.text.get(range.clone())?;
    let exact = if raw == rendered {
        Some(range.start)
    } else if role == STYLE_CODE {
        raw.find(rendered).map(|at| range.start + at)
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
        for (at, ch) in rendered.char_indices() {
            let end = at + ch.len_utf8();
            text.anchors.push(Anchor {
                text_offset: base + end as u32,
                source_offset: decoded.source_at(exact + end)?,
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
        text.spans.push(Span {
            start: base,
            end: text.text.len() as u32,
            role,
            flags: flags & 7,
        });
    }
    Some(())
}

pub fn safe_link(target: &str) -> bool {
    if target.is_empty()
        || target.len() > 2048
        || target.chars().any(|c| c.is_control() || c.is_whitespace())
        || target.contains('\\')
    {
        return false;
    }
    if target.starts_with('#') {
        return true;
    }
    let Some((scheme, rest)) = target.split_once(':') else {
        return false;
    };
    if scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http") {
        rest.starts_with("//") && rest.len() > 2 && !rest[2..].starts_with('/')
    } else {
        scheme.eq_ignore_ascii_case("mailto") && !rest.is_empty()
    }
}

pub fn plain(decoded: &Decoded, revision: u64, eof: bool) -> DocumentPage {
    let text = decoded.clone().page(revision, eof);
    DocumentPage {
        revision,
        source: text.source.clone(),
        has_before: text.has_before,
        has_after: text.has_after,
        blocks: vec![Block {
            kind: BLOCK_PARAGRAPH,
            depth: 0,
            heading_level: 0,
            text,
            links: Vec::new(),
            table: None,
            strikethrough: None,
        }],
    }
}
