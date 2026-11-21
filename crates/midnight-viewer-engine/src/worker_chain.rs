//! Byte-capped text replies joined to the line budget of their request.
//!
//! The engine asks a Text or Source provider for the viewport's rows plus its retained
//! guards (`page_viewport`). A provider may stop earlier at a byte budget of its own: the
//! basic and source viewers send at most about 8 KiB per page. Rows of a few hundred bytes
//! then give a page barely taller than the viewport, or shorter, and scrolling across it
//! loses travel, jumps and shows cut rows as lines. Such a reply is held back here, the
//! contiguous next range is requested, and the joined page reaches the worker as the one
//! reply to the original request: positions, motion and raster continuity are unchanged.
//!
//! A refill asks for the rows around the camera, and the shown page already holds most of
//! them: the page moves a little each time the camera nears its edge. Such a request reads
//! only the rows past the page's edge, and its reply is joined to the rows the page holds
//! (`Worker::overlap`), as the reply to a byte-capped read is joined to the next range.
//!
//! A joined page is one text layout and stays within what one layout takes (`fit`): its
//! renderer keeps a glyph cluster's offset inside its shaping run in 16 bits (parley 0.11,
//! `ClusterData::text_offset`), and uniformly styled text is one run, so every byte past
//! 64 KiB of a page would map to the wrong glyph. The caret of offset 72,000 of a 73,200-byte
//! page lay on the page's end, a row too low, and refills placed on it lost travel.
use super::*;

/// The most bytes one joined page lays out, tabs expanded: one shaping run's 16-bit offsets.
pub(super) const JOIN_BYTES: usize = crate::viewport::PAGE_BYTES;
/// The most color spans one layout takes (`midnight_viewer_render`'s span limit).
pub(super) const JOIN_SPANS: usize = crate::viewport::PAGE_SPANS;
/// Ranges read for one request at most, however little each one holds.
const JOIN_READS: u8 = 16;
/// First request ID a test worker without a plugin session gives a chained read.
#[cfg(test)]
pub(super) const CHAINED_ID: u64 = 1 << 40;

/// One request whose reply is still being joined. The worker knows only `logical`; the
/// events of `physical`, the range read last, are translated to it.
pub(super) struct Chain {
    logical: u64,
    physical: u64,
    viewport: messages::Viewport,
    page: Option<viewer::TextPage>,
    /// The range read last of a backward chain may start inside a row.
    cut_start: bool,
    /// Ranges read so far; the last one added no byte, or filled the layout, when `stalled`.
    reads: u8,
    stalled: bool,
    /// The request's first part came from the shown page (`Worker::overlap`).
    seeded: bool,
    /// The range read after that part did not continue it: read the whole request.
    whole: bool,
}

#[derive(Default)]
pub(super) struct Chains(Vec<Chain>);

/// Logical lines in `text`: the unit of a request's line budget.
fn lines(text: &str) -> u32 {
    u32::try_from(text.bytes().filter(|byte| *byte == b'\n').count()).unwrap_or(u32::MAX)
}

/// The bytes `text` takes in its layout, which expands a tab to at most a tab width.
pub(super) fn layout_bytes(text: &str) -> usize {
    let tabs = text.bytes().filter(|byte| *byte == b'\t').count();
    let width = usize::from(TextOptions::default().tab_width);
    text.len() + tabs * width.saturating_sub(1)
}

/// Whether a joined page of `lines` rows reads on for a request of `max_lines`: up to its
/// line budget, while its layout has room. A backward page holds one newline less than its
/// line budget: its last row runs up to the anchor. One missing line is enough.
fn reads_on(lines: u32, page: &viewer::TextPage, max_lines: u32) -> bool {
    lines.saturating_add(1) < max_lines
        && layout_bytes(&page.text) < JOIN_BYTES
        && page.spans.len() < JOIN_SPANS
}

/// The bytes a read after `page` may add to its layout.
fn room(page: &viewer::TextPage) -> u32 {
    u32::try_from(JOIN_BYTES.saturating_sub(layout_bytes(&page.text))).unwrap_or(u32::MAX)
}

/// The longest part of `page` that fits `bytes` of layout and `spans` spans, cut at one of
/// its anchors: from its start, or up to its end for a backward read.
pub(super) fn fit(
    page: viewer::TextPage,
    bytes: usize,
    spans: usize,
    backwards: bool,
) -> viewer::TextPage {
    if layout_bytes(&page.text) <= bytes && page.spans.len() <= spans {
        return page;
    }
    let anchors = &page.anchors;
    let offset = |index: usize| anchors[index].text_offset as usize;
    let mut used = 0;
    let range = if backwards {
        let mut keep = anchors.len() - 1;
        for index in (0..anchors.len() - 1).rev() {
            used += layout_bytes(&page.text[offset(index)..offset(index + 1)]);
            let kept = page.spans.len()
                - page
                    .spans
                    .partition_point(|span| span.end as usize <= offset(index));
            if used > bytes || kept > spans {
                break;
            }
            keep = index;
        }
        offset(keep)..page.text.len()
    } else {
        let mut keep = 0;
        for index in 1..anchors.len() {
            used += layout_bytes(&page.text[offset(index - 1)..offset(index)]);
            let kept = page
                .spans
                .partition_point(|span| (span.start as usize) < offset(index));
            if used > bytes || kept > spans {
                break;
            }
            keep = index;
        }
        0..offset(keep)
    };
    slice(&page, range)
}

/// The part of `page` between two of its anchors, with its spans clipped to it. An edge cut
/// inside a row makes it partial, as a provider marks a cropped edge.
fn slice(page: &viewer::TextPage, range: std::ops::Range<usize>) -> viewer::TextPage {
    let first = page
        .anchors
        .partition_point(|anchor| (anchor.text_offset as usize) < range.start);
    let last = page
        .anchors
        .partition_point(|anchor| anchor.text_offset as usize <= range.end)
        - 1;
    let (start, end) = (range.start as u32, range.end as u32);
    let text = page.text[range.clone()].to_owned();
    let cut_start = range.start > 0 && !page.text[..range.start].ends_with('\n');
    let cut_end = range.end < page.text.len() && !text.ends_with('\n');
    viewer::TextPage {
        revision: page.revision,
        source: viewer::SourceRange {
            offset: page.anchors[first].source_offset,
            length: page.anchors[last].source_offset - page.anchors[first].source_offset,
        },
        spans: page
            .spans
            .iter()
            .filter(|span| span.start < end && span.end > start)
            .map(|span| viewer::Span {
                start: span.start.max(start) - start,
                end: span.end.min(end) - start,
                ..span.clone()
            })
            .collect(),
        anchors: page.anchors[first..=last]
            .iter()
            .map(|anchor| viewer::Anchor {
                text_offset: anchor.text_offset - start,
                source_offset: anchor.source_offset,
            })
            .collect(),
        has_before: page.has_before || range.start > 0,
        has_after: page.has_after || range.end < page.text.len(),
        partial_line: page.partial_line || cut_start || cut_end,
        text,
    }
}

/// Whether `second` continues `first`: the next source range of the same revision.
pub(super) fn adjacent(first: &viewer::TextPage, second: &viewer::TextPage) -> bool {
    let end = first.source.offset.checked_add(first.source.length);
    end == Some(second.source.offset)
        && first.revision == second.revision
        && second
            .anchors
            .first()
            .is_some_and(|anchor| anchor.text_offset == 0)
        && first.anchors.last().map(|anchor| anchor.source_offset) == end
        && u32::try_from(first.text.len() + second.text.len()).is_ok()
}

/// `first` followed by `second`, which continues it (`adjacent`).
pub(super) fn join(mut first: viewer::TextPage, second: viewer::TextPage) -> viewer::TextPage {
    let shift = first.text.len() as u32;
    // A row cut at the junction is whole again; either outer edge can still be cut.
    let first_cut_end = first.has_after && !first.text.ends_with('\n');
    let first_cut_start = first.partial_line && !first_cut_end;
    let second_cut_end = second.has_after && !second.text.ends_with('\n');
    // The second page's opening anchor is the first page's closing one.
    first
        .anchors
        .extend(second.anchors.iter().skip(1).map(|anchor| viewer::Anchor {
            text_offset: anchor.text_offset + shift,
            source_offset: anchor.source_offset,
        }));
    first
        .spans
        .extend(second.spans.iter().map(|span| viewer::Span {
            start: span.start + shift,
            end: span.end + shift,
            ..span.clone()
        }));
    first.text.push_str(&second.text);
    first.source.length += second.source.length;
    first.has_after = second.has_after;
    first.partial_line = first_cut_start
        || (first.partial_line && first_cut_end)
        || (second.partial_line && second_cut_end);
    first
}

/// Drop the cut row a backward read starts with, so that it never shows as a line of its
/// own, and tell whether it did. A page without a whole row keeps it: an enormous line is
/// all there is to show.
pub(super) fn trim_cut_start(mut page: viewer::TextPage) -> (viewer::TextPage, bool) {
    let Some(newline) = page.text.find('\n') else {
        return (page, false);
    };
    let start = newline + 1;
    if start >= page.text.len() {
        return (page, false);
    }
    let Some(index) = page
        .anchors
        .iter()
        .position(|anchor| anchor.text_offset as usize == start)
    else {
        return (page, false);
    };
    let source = page.anchors[index].source_offset;
    let Ok(shift) = u32::try_from(start) else {
        return (page, false);
    };
    page.text.drain(..start);
    page.anchors.drain(..index);
    for anchor in &mut page.anchors {
        anchor.text_offset -= shift;
    }
    page.spans.retain(|span| span.end > shift);
    for span in &mut page.spans {
        span.start = span.start.max(shift) - shift;
        span.end -= shift;
    }
    page.source.length -= source - page.source.offset;
    page.source.offset = source;
    page.has_before = true;
    (page, true)
}

impl Chains {
    pub(super) fn clear(&mut self) {
        self.0.clear();
    }
}

impl Worker {
    /// Whether the replies to this worker's viewport requests may need joining.
    fn joins_replies(&self) -> bool {
        !self.binary && matches!(self.mode, viewer::MODE_TEXT | viewer::MODE_SOURCE)
    }

    /// Track a Text or Source viewport request whose reply may need joining.
    pub(super) fn chain_requested(&mut self, id: u64, viewport: &messages::Viewport) {
        self.chain_requested_with(id, viewport, None);
    }

    /// Track a request whose first part `held` came from the shown page (`overlap`).
    pub(super) fn chain_requested_with(
        &mut self,
        id: u64,
        viewport: &messages::Viewport,
        held: Option<viewer::TextPage>,
    ) {
        if !self.joins_replies() {
            return;
        }
        self.retire_stale_chains();
        self.chains.0.push(Chain {
            logical: id,
            physical: id,
            viewport: viewport.clone(),
            seeded: held.is_some(),
            page: held,
            cut_start: false,
            reads: 1,
            stalled: false,
            whole: false,
        });
    }

    /// The part of the request `viewport` that the shown page holds, and the read of the
    /// rest: the rows past the page's end, or before its start for a backward request. A
    /// scrollbar position (`position`) keeps a whole read, which its recovery checks.
    pub(super) fn overlap(
        &self,
        viewport: &messages::Viewport,
        position: Option<u64>,
    ) -> Option<(viewer::TextPage, messages::Viewport)> {
        if position.is_some() || !self.joins_replies() {
            return None;
        }
        #[cfg(test)]
        if unsent::reads_whole() {
            return None;
        }
        let page = self.page.as_ref()?;
        let index = page
            .anchors
            .partition_point(|anchor| anchor.source_offset < viewport.source_anchor);
        let anchor = page.anchors.get(index)?;
        let at = anchor.text_offset as usize;
        if anchor.source_offset != viewport.source_anchor || at == 0 || at >= page.text.len() {
            return None;
        }
        let (held, source_anchor) = if viewport.backwards {
            (page.before.then(|| page.text_range(0..at))??, page.start)
        } else {
            let end = page.text.len();
            (page.after.then(|| page.text_range(at..end))??, page.end)
        };
        let lines = lines(&held.text);
        // The rows read after the held ones end where a whole read would: at the line
        // budget, or where one layout is full (`fit`), however the reads fall. A piece of
        // an enormous row, without a row of its own, is read whole.
        if lines == 0 {
            return None;
        }
        reads_on(lines, &held, viewport.max_lines).then(|| {
            let read = messages::Viewport {
                source_anchor,
                backwards: viewport.backwards,
                max_bytes: viewport.max_bytes.min(room(&held)),
                max_lines: viewport.max_lines - lines,
            };
            (held, read)
        })
    }

    /// Whether `logical` is still a request the worker waits for.
    fn chain_current(&self, logical: u64) -> bool {
        self.viewport_request == Some(logical) || self.source_refresh_request() == Some(logical)
    }

    /// Superseded requests leave their chains: cancel a range still being read for one.
    fn retire_stale_chains(&mut self) {
        let mut index = 0;
        while index < self.chains.0.len() {
            let chain = &self.chains.0[index];
            if self.chain_current(chain.logical) {
                index += 1;
                continue;
            }
            let chain = self.chains.0.remove(index);
            if chain.physical != chain.logical
                && let Some(session) = &self.session
            {
                session.cancel(chain.physical);
            }
        }
    }

    /// The events that `id`'s `kind` stands for once joined: none while the range after a
    /// byte-capped reply is still being read, then its joined page and its completion
    /// under the original request.
    pub(super) fn chain_event(&mut self, id: u64, kind: EventKind) -> Vec<(u64, EventKind)> {
        let Some(index) = self.chains.0.iter().position(|chain| chain.physical == id) else {
            return vec![(id, kind)];
        };
        if !self.chain_current(self.chains.0[index].logical) {
            let chain = self.chains.0.remove(index);
            if chain.physical == chain.logical {
                return vec![(id, kind)];
            }
            if let Some(session) = &self.session {
                session.cancel(chain.physical);
            }
            return Vec::new();
        }
        match kind {
            EventKind::TextPage(_) if self.chains.0[index].whole => Vec::new(),
            EventKind::TextPage(page) => {
                let chain = &mut self.chains.0[index];
                // A reply joins the pages read before it within one layout.
                let (bytes, spans) = chain
                    .page
                    .as_ref()
                    .map_or((JOIN_BYTES, JOIN_SPANS), |held| {
                        (
                            JOIN_BYTES.saturating_sub(layout_bytes(&held.text)),
                            JOIN_SPANS.saturating_sub(held.spans.len()),
                        )
                    });
                let length = page.text.len();
                let page = fit(page, bytes, spans, chain.viewport.backwards);
                chain.stalled = page.source.length == 0 || page.text.len() < length;
                // A backward read starts where its line budget or its byte budget ended:
                // at a row, or inside one. Its edge flag tells them apart only when it
                // also ends at a row; a newline count cannot, as a byte budget can end
                // just where the line budget would have.
                let cut_start =
                    page.has_before && !(page.text.ends_with('\n') && !page.partial_line);
                let joined = match chain.page.take() {
                    None => page,
                    Some(previous) => {
                        let (first, second) = if chain.viewport.backwards {
                            (page, previous)
                        } else {
                            (previous, page)
                        };
                        if !adjacent(&first, &second) && chain.seeded && chain.reads == 1 {
                            // The rows read did not continue the shown page's: the
                            // request is read whole once this range completes.
                            chain.whole = true;
                            return Vec::new();
                        }
                        if !adjacent(&first, &second) {
                            // Not adjacent after all: the pages joined so far are the reply.
                            chain.page = Some(if chain.viewport.backwards {
                                second
                            } else {
                                first
                            });
                            chain.physical = chain.logical;
                            return self.finish_chain(index);
                        }
                        join(first, second)
                    }
                };
                if chain.viewport.backwards {
                    chain.cut_start = cut_start;
                }
                chain.page = Some(joined);
                Vec::new()
            }
            EventKind::Completed if self.chains.0[index].whole => {
                let whole = self.chains.0[index].viewport.clone();
                if let Some(physical) = self.chain_request(whole) {
                    let chain = &mut self.chains.0[index];
                    chain.physical = physical;
                    chain.reads += 1;
                    chain.seeded = false;
                    chain.whole = false;
                    chain.stalled = false;
                    chain.page = None;
                    return Vec::new();
                }
                self.chains.0[index].page = None;
                self.finish_chain(index)
            }
            EventKind::Completed => {
                let next = {
                    let chain = &self.chains.0[index];
                    let more = !chain.stalled && chain.reads < JOIN_READS;
                    chain.page.as_ref().filter(|_| more).and_then(|page| {
                        let held = lines(&page.text);
                        let anchor = if chain.viewport.backwards {
                            page.has_before.then_some(page.source.offset)
                        } else {
                            page.has_after
                                .then(|| page.source.offset + page.source.length)
                        }?;
                        reads_on(held, page, chain.viewport.max_lines).then(|| messages::Viewport {
                            source_anchor: anchor,
                            backwards: chain.viewport.backwards,
                            max_bytes: chain.viewport.max_bytes.min(room(page)),
                            max_lines: chain.viewport.max_lines - held,
                        })
                    })
                };
                if let Some(next) = next
                    && let Some(physical) = self.chain_request(next)
                {
                    let chain = &mut self.chains.0[index];
                    chain.physical = physical;
                    chain.reads += 1;
                    return Vec::new();
                }
                self.finish_chain(index)
            }
            EventKind::Cancelled | EventKind::Failed(_)
                if self.chains.0[index].physical != self.chains.0[index].logical =>
            {
                // The range after a reply ended without a page: deliver what was joined.
                self.finish_chain(index)
            }
            other => {
                let chain = &self.chains.0[index];
                if chain.physical == chain.logical
                    && matches!(other, EventKind::Cancelled | EventKind::Failed(_))
                {
                    self.chains.0.remove(index);
                }
                vec![(id, other)]
            }
        }
    }

    /// Request the next range of a chain. A test worker without a plugin session records
    /// it as unsent and answers it under a request ID of its own.
    fn chain_request(&self, next: messages::Viewport) -> Option<u64> {
        if let Some(session) = &self.session {
            return session.viewport(next).ok();
        }
        #[cfg(test)]
        {
            unsent::record(&next);
            if let Some(id) = unsent::admitted(&next) {
                return Some(id);
            }
            let reads: u64 = self
                .chains
                .0
                .iter()
                .map(|chain| u64::from(chain.reads))
                .sum();
            Some(CHAINED_ID + reads)
        }
        #[cfg(not(test))]
        None
    }

    /// The request whose events a chain waits for now, for tests that answer it.
    #[cfg(test)]
    pub(super) fn chained_request(&self, logical: u64) -> Option<u64> {
        self.chains
            .0
            .iter()
            .find(|chain| chain.logical == logical)
            .map(|chain| chain.physical)
    }

    fn finish_chain(&mut self, index: usize) -> Vec<(u64, EventKind)> {
        let chain = self.chains.0.remove(index);
        let Some(page) = chain.page else {
            return vec![(chain.logical, EventKind::Completed)];
        };
        // A backward chain ends with the range read last: it starts at the file's start or
        // at a row, or its first row, which may be cut, is dropped here. A whole one is
        // dropped too when that cannot be told: the page above shows it. The other edge is
        // the request's anchor. A forward chain's first row starts at the anchor, which the
        // provider's flag does not tell apart from a cut end.
        let (mut page, whole_start) = if !chain.viewport.backwards {
            (page, false)
        } else if chain.cut_start {
            trim_cut_start(page)
        } else {
            (page, true)
        };
        if whole_start {
            page.partial_line = page.has_after && !page.text.ends_with('\n');
        }
        vec![
            (chain.logical, EventKind::TextPage(page)),
            (chain.logical, EventKind::Completed),
        ]
    }
}
