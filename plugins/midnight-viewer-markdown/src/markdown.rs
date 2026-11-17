//! CommonMark to inert semantic blocks. This module has no I/O capability.
use crate::{
    build::{self, Parsed},
    decode::{self, Decoded},
};
use midnight_plugin_sdk::api::{codec, document::*, viewer::*};

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

    /// The bounded scalar fragment for an oversized unit or an anchor after
    /// the last proven unit. A forward fragment that starts in proven content
    /// stops before units that may continue past the read.
    fn fragment(&self, raw: &DocumentPage, proven_end: u64) -> DocumentPage {
        let complete = raw
            .blocks
            .iter()
            .take_while(|block| block.text.source.offset < proven_end)
            .count();
        let proven_after_anchor = raw.blocks[..complete]
            .last()
            .is_some_and(|block| block.text.source.offset + block.text.source.length > self.anchor);
        if !self.backwards && proven_after_anchor && complete < raw.blocks.len() {
            self.crop(crate::semantic_window::slice_ref(raw, 0..complete))
        } else {
            self.crop(raw.clone())
        }
    }
}

/// The production viewport selection over one decoded read: semantic units,
/// their bounded fallbacks, and finally the scalar plain-text fragment.
pub(crate) fn render_slice(
    decoded: &Decoded,
    revision: u64,
    eof: bool,
    window: Window,
    cancelled: impl Fn() -> bool,
) -> Option<(DocumentPage, Vec<u64>)> {
    let parsed = render_context(decoded, revision, eof, &cancelled)?;
    let page = crop_context(&parsed.page, parsed.proven_end, &window)
        .unwrap_or_else(|| window.crop(plain(decoded, revision, eof)));
    Some((page, parsed.checkpoints))
}

pub(crate) fn render_context(
    decoded: &Decoded,
    revision: u64,
    eof: bool,
    cancelled: impl Fn() -> bool,
) -> Option<Parsed> {
    build::render_raw(decoded, revision, eof, cancelled)
}

/// Select complete layout units around the anchor, or a bounded fragment.
/// `proven_end` is the parse's [`Parsed::proven_end`].
pub(crate) fn crop_context(
    raw: &DocumentPage,
    proven_end: u64,
    window: &Window,
) -> Option<DocumentPage> {
    let selected = crate::semantic_window::crop_ref(raw, window, proven_end)
        .unwrap_or_else(|| window.fragment(raw, proven_end));
    if fits_wire(&selected) {
        return Some(selected);
    }
    // A dense unit can fit its text and exact anchors only without optional
    // styles and link metadata; keep the complete unit then.
    let mut stripped = selected.clone();
    strip_decoration(&mut stripped);
    if fits_wire(&stripped) {
        return Some(stripped);
    }
    drop(stripped);
    // Otherwise use the same bounded prose fragment as an oversized raw unit,
    // cropped from the still-decorated selection: the smaller fragment keeps
    // its styles and links whenever they fit.
    let mut page = window.crop(selected);
    if fits_wire(&page) {
        return Some(page);
    }
    strip_decoration(&mut page);
    fits_wire(&page).then_some(page)
}

/// Crop a page whose blocks carry no table geometry (plain pages and scalar
/// fragments) to the displayed text after `anchor`.
pub fn crop(
    mut page: DocumentPage,
    anchor: u64,
    max_bytes: usize,
    max_lines: usize,
) -> DocumentPage {
    debug_assert!(page.blocks.iter().all(|block| block.table.is_none()));
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
            if start == end && block.kind != BLOCK_RULE {
                return None;
            }
            let text = &mut block.text;
            text.anchors
                .retain(|a| (start..=end).contains(&(a.text_offset as usize)));
            let first = text.anchors.first()?.source_offset;
            let last = text.anchors.last()?.source_offset;
            for a in &mut text.anchors {
                a.text_offset -= start as u32;
            }
            text.source.offset = first;
            text.source.length = last - first;
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
            budget -= block.text.text.len();
            // Paragraphs, headings, list items and table rows have no literal
            // trailing newline, but each consumes at least one displayed line.
            // Counting only SoftBreak/HardBreak allowed hundreds of blocks into
            // a viewport requesting only a handful of lines.
            if !block.text.text.ends_with('\n') {
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

fn fits_wire(page: &DocumentPage) -> bool {
    codec::encode(page).is_ok()
}

/// Anchors, spans and links share one CBOR item quota. Optional decoration is
/// discarded before displayed text or exact anchors, keeping document search
/// consistent with the visible fallback.
fn strip_decoration(page: &mut DocumentPage) {
    for block in &mut page.blocks {
        block.text.spans.clear();
        block.links.clear();
        block.strikethrough = None;
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

pub fn safe_link(target: &str) -> bool {
    if target.is_empty()
        || target.len() > 2048
        || target
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || decode::visible(c) != c)
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

#[cfg(test)]
#[path = "crop_context_tests.rs"]
mod crop_context_tests;
