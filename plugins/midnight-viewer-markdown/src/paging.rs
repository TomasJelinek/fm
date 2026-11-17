use crate::{
    checkpoints::Checkpoints,
    decode::{self, Decoded},
    fence::{self, OpenFence},
    markdown, page_cache,
    plugin::{Session, lock, send},
    retained_context::Retained,
    trace,
};
use midnight_plugin_sdk::{
    Context, Output,
    api::{document::DocumentPage, messages::Viewport, viewer::*},
};
use std::sync::Arc;

/// Viewport replay starts at a proven boundary at most this far before it.
pub(crate) const REPLAY_BYTES: u64 = 32760;

pub struct Hints {
    pub(crate) plain: bool,
    pub(crate) checkpoints: Checkpoints,
    cache: page_cache::Cache,
    context: Option<Retained>,
    /// The latest top-level fence a parse left open at its read edge.
    fence: Option<OpenFence>,
}
impl Hints {
    pub fn new(mode: u16) -> Self {
        Self {
            plain: mode == MODE_TEXT,
            checkpoints: Checkpoints::default(),
            cache: page_cache::Cache::default(),
            context: None,
            fence: None,
        }
    }
    pub fn initialize(&mut self, bom: u64) {
        self.checkpoints.initialize(bom);
    }
}

/// One bounded viewport read.
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    pub offset: u64,
    pub count: u32,
    /// Forward anchor, or the start of a backward request's look-behind.
    pub anchor: u64,
    /// `offset` is a proven top-level boundary, decided when the read was
    /// planned; later checkpoint eviction cannot revoke it.
    pub known: bool,
}

pub fn cached(session: &Session, request: &Viewport) -> Option<(Arc<DocumentPage>, bool)> {
    let plan = plan(session, request);
    let bom = lock(&session.metadata).bom;
    let mut hints = lock(&session.hints);
    if let Some(page) = hints.cache.get(request) {
        trace::request("page_hit", request);
        return Some((page, hints.plain));
    }
    let context = hints.context.as_ref()?;
    let end = context.source.offset.checked_add(context.source.length)?;
    if hints.plain
        || plan.offset < context.source.offset
        || (end < plan.offset.saturating_add(u64::from(plan.count)) && context.has_after)
        || request.source_anchor > end
    {
        return None;
    }
    let mut page = context.crop(&window(request))?;
    trace::request("context_hit", request);
    page.has_before = page.source.offset > bom;
    hints.cache.insert(request, &page);
    let page = hints.cache.get(request)?;
    Some((page, hints.plain))
}

fn window(request: &Viewport) -> markdown::Window {
    markdown::Window {
        anchor: request.source_anchor,
        backwards: request.backwards,
        max_bytes: request.max_bytes.min(65536) as usize,
        max_lines: request.max_lines as usize,
    }
}

/// Plan a viewport read; the request anchor becomes the checkpoint focus.
pub fn plan(session: &Session, viewport: &Viewport) -> Plan {
    let (bom, encoding) = {
        let metadata = lock(&session.metadata);
        (metadata.bom, metadata.encoding)
    };
    let limit = viewport.max_bytes.min(65536) as u64;
    let upper = viewport.source_anchor.max(bom);
    let anchor = if viewport.backwards {
        upper.saturating_sub((limit * 4 + 8).min(32776)).max(bom)
    } else {
        upper
    };
    let target = if viewport.backwards { upper } else { anchor };
    let lower = target.saturating_sub(REPLAY_BYTES).max(bom);
    let upper_checkpoint = if viewport.backwards {
        target.saturating_sub(1).max(bom)
    } else {
        target
    };
    let checkpoint = {
        let mut hints = lock(&session.hints);
        hints.checkpoints.focus(viewport.source_anchor);
        hints.checkpoints.earliest(lower..=upper_checkpoint)
    };
    let offset = checkpoint.unwrap_or_else(|| decode::lookbehind(anchor, bom, encoding));
    // A backward anchor can lie inside a table/paragraph. Include its following
    // context too, otherwise layout would measure a different partial block.
    let end = target.saturating_add(limit * 4 + 8);
    let count = end.saturating_sub(offset).min(65536).min(u64::MAX - offset) as u32;
    Plan {
        offset,
        count: count.max(1),
        anchor,
        known: checkpoint.is_some(),
    }
}

pub fn viewport(
    context: Context,
    session: Arc<Session>,
    data: Vec<u8>,
    plan: Plan,
    request: Viewport,
    eof: bool,
) {
    let Some(decoded) = decode_window(&session, &data, plan, &request, eof, || {
        context.is_cancelled()
    }) else {
        return;
    };
    let (Some(first), Some(last)) = (decoded.anchors.first(), decoded.anchors.last()) else {
        return;
    };
    let page_eof = eof && last.source_offset == plan.offset + data.len() as u64;
    let known = plan.known && first.source_offset == plan.offset;
    let anchor = decoded
        .anchors
        .iter()
        .find(|a| a.source_offset >= plan.anchor)
        .unwrap_or(last)
        .source_offset;
    let upper = decoded
        .anchors
        .iter()
        .take_while(|a| a.source_offset <= request.source_anchor)
        .last()
        .map_or(request.source_anchor, |a| a.source_offset);
    let plain = lock(&session.hints).plain;
    let mut page = if plain || !known {
        if !plain {
            resynchronize(&session, &decoded);
        }
        let page = markdown::plain(&decoded, session.revision, page_eof);
        if request.backwards {
            markdown::crop_backward(
                page,
                upper,
                request.max_bytes.min(8192) as usize,
                request.max_lines as usize,
            )
        } else {
            markdown::crop(
                page,
                anchor,
                request.max_bytes.min(8192) as usize,
                request.max_lines as usize,
            )
        }
    } else {
        trace::request("parse", &request);
        let Some(parsed) = markdown::render_context(&decoded, session.revision, page_eof, || {
            context.is_cancelled()
        }) else {
            return;
        };
        let mut window = window(&request);
        window.anchor = if request.backwards { upper } else { anchor };
        let page =
            markdown::crop_context(&parsed.page, parsed.proven_end, &window).unwrap_or_else(|| {
                let plain = markdown::plain(&decoded, session.revision, page_eof);
                if request.backwards {
                    markdown::crop_backward(
                        plain,
                        upper,
                        window.max_bytes.min(8192),
                        window.max_lines,
                    )
                } else {
                    markdown::crop(plain, anchor, window.max_bytes.min(8192), window.max_lines)
                }
            });
        let retained = Retained::new(
            &parsed.page,
            parsed.proven_end,
            &window,
            &decoded.anchors,
            || context.is_cancelled(),
        );
        let mut hints = lock(&session.hints);
        // One bounded parsed context, separate from the small final-page LRU.
        // It is revision-local and was already read on demand. Nearby anchors
        // can now reuse both bytes and parse structure while scrolling back.
        if !context.is_cancelled()
            && let Some(retained) = retained
        {
            hints.context = Some(retained);
        }
        trace::retained(&parsed.page, hints.context.as_ref());
        hints.checkpoints.extend(parsed.checkpoints);
        if parsed.open_fence.is_some() {
            hints.fence = parsed.open_fence;
        }
        page
    };
    if context.is_cancelled() {
        return;
    }
    page.has_before = page.source.offset > lock(&session.metadata).bom;
    let cached = {
        let mut hints = lock(&session.hints);
        hints.cache.insert(&request, &page);
        hints.cache.get(&request)
    };
    send_page(context, cached.as_deref().unwrap_or(&page), plain);
}

/// A plain read without a nearby checkpoint may continue the scan for the end
/// of a long top-level fence; the line after it is a proven checkpoint.
fn resynchronize(session: &Session, decoded: &Decoded) {
    let Some(mut open) = lock(&session.hints).fence else {
        return;
    };
    let scan = open.advance(decoded);
    let mut hints = lock(&session.hints);
    if hints
        .fence
        .is_some_and(|current| current.start != open.start)
    {
        return;
    }
    match scan {
        fence::Scan::Closed(boundary) => {
            hints.checkpoints.extend([boundary]);
            hints.fence = None;
        }
        fence::Scan::Open => hints.fence = Some(open),
        fence::Scan::Gap => {}
    }
}

pub fn send_page(context: Context, page: &DocumentPage, plain: bool) {
    if context.is_cancelled() {
        return;
    }
    if plain {
        let text = page.blocks.first().map(|b| b.text.clone());
        if let Some(mut text) = text {
            // The block keeps the flags of the whole decoded read; the crop
            // decides whether the page itself has content before/after it.
            text.has_before = page.has_before;
            text.has_after = page.has_after;
            text.partial_line = page.has_after && !text.text.ends_with('\n');
            send(context, Output::text(&text));
        } else {
            // Cropping removes empty semantic blocks, but a plain viewport
            // still needs a page to replace the host's opening or prior frame.
            let text = TextPage {
                revision: page.revision,
                anchors: vec![Anchor {
                    text_offset: 0,
                    source_offset: page.source.offset,
                }],
                source: SourceRange {
                    offset: page.source.offset,
                    length: 0,
                },
                text: String::new(),
                spans: Vec::new(),
                has_before: page.has_before,
                has_after: page.has_after,
                partial_line: false,
            };
            send(context, Output::text(&text));
        }
    } else {
        send(context, Output::document(page));
    }
}

fn decode_window(
    session: &Session,
    data: &[u8],
    plan: Plan,
    request: &Viewport,
    eof: bool,
    cancelled: impl Fn() -> bool,
) -> Option<Decoded> {
    let (encoding, bom) = {
        let metadata = lock(&session.metadata);
        (metadata.encoding, metadata.bom)
    };
    let offset = plan.offset;
    let boundary = plan.known || offset == bom;
    let mut decoded = decode::fragment(data, (offset, encoding, boundary), eof, 65536, &cancelled)?;
    let target = if request.backwards {
        request.source_anchor
    } else {
        plan.anchor
    };
    let end = decoded.anchors.last()?.source_offset;
    let read_end = offset + data.len() as u64;
    if end < target.min(read_end) || (!request.backwards && end == target && target < read_end) {
        // Rendered control/invalid prefixes can expand threefold. Drop distant
        // context, never the viewport, and reuse this same bounded raw read.
        let lower = if request.backwards {
            target.saturating_sub(u64::from(request.max_bytes.min(8192)) * 2 + 8)
        } else {
            target
        };
        let near = decode::lookbehind(lower, bom, encoding).max(offset);
        let skip = (near - offset).min(data.len() as u64) as usize;
        decoded = decode::fragment(
            &data[skip..],
            (offset + skip as u64, encoding, near == bom),
            eof,
            65536,
            &cancelled,
        )?;
        decoded.consumed += skip;
    }
    Some(decoded)
}
