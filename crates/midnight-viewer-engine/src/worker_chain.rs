//! Byte-capped text replies joined to the line budget of their request.
//!
//! The engine asks a Text or Source provider for the viewport's rows plus its retained
//! guards (`page_viewport`). A provider may stop earlier at a byte budget of its own: the
//! basic and source viewers send at most about 8 KiB per page. Rows of a few hundred bytes
//! then give a page barely taller than the viewport, or shorter, and scrolling across it
//! loses travel, jumps and shows cut rows as lines. Such a reply is held back here, the
//! contiguous next range is requested, and the joined page reaches the worker as the one
//! reply to the original request: positions, motion and raster continuity are unchanged.
use super::*;

/// The most text one joined page holds, as much as one semantic document page.
pub(super) const JOIN_BYTES: usize = 64 * 1024;
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
    /// Ranges read so far; the last one added no byte when `stalled`.
    reads: u8,
    stalled: bool,
}

#[derive(Default)]
pub(super) struct Chains(Vec<Chain>);

/// Logical lines in `text`: the unit of a request's line budget.
fn lines(text: &str) -> u32 {
    u32::try_from(text.bytes().filter(|byte| *byte == b'\n').count()).unwrap_or(u32::MAX)
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
    /// Track a Text or Source viewport request whose reply may need joining.
    pub(super) fn chain_requested(&mut self, id: u64, viewport: &messages::Viewport) {
        if self.binary || !matches!(self.mode, viewer::MODE_TEXT | viewer::MODE_SOURCE) {
            return;
        }
        self.retire_stale_chains();
        self.chains.0.push(Chain {
            logical: id,
            physical: id,
            viewport: viewport.clone(),
            page: None,
            cut_start: false,
            reads: 1,
            stalled: false,
        });
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
            EventKind::TextPage(page) => {
                let chain = &mut self.chains.0[index];
                chain.stalled = page.source.length == 0;
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
                        // A backward page holds one newline less than its line budget:
                        // its last row runs up to the anchor. One missing line is enough.
                        (held.saturating_add(1) < chain.viewport.max_lines
                            && page.text.len() < JOIN_BYTES)
                            .then(|| messages::Viewport {
                                source_anchor: anchor,
                                backwards: chain.viewport.backwards,
                                max_bytes: chain.viewport.max_bytes,
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
