use crate::{
    decode::{self, Decoded},
    markdown,
    plugin::{Session, lock, send},
};
use midnight_plugin_sdk::{
    Context, Output,
    api::{messages::Viewport, viewer::*},
};
use std::{collections::BTreeSet, sync::Arc};

pub struct Hints {
    plain: bool,
    checkpoints: BTreeSet<u64>,
}
impl Hints {
    pub fn new(_: &str, mode: u16) -> Self {
        Self {
            plain: mode == MODE_TEXT,
            checkpoints: BTreeSet::new(),
        }
    }
    pub fn initialize(&mut self, _: &str, bom: u64) {
        self.checkpoints.insert(bom);
    }
}
pub fn plan(session: &Session, viewport: &Viewport) -> (u64, u32, u64) {
    let metadata = lock(&session.metadata);
    let limit = viewport.max_bytes.min(8192) as u64;
    let upper = viewport.source_anchor.max(metadata.bom);
    let anchor = if viewport.backwards {
        upper.saturating_sub(limit * 4 + 8).max(metadata.bom)
    } else {
        upper
    };
    let checkpoint = lock(&session.hints)
        .checkpoints
        .range(..=anchor)
        .next_back()
        .copied();
    let target = if viewport.backwards { upper } else { anchor };
    let offset = checkpoint
        .filter(|&at| anchor - at <= 32768 && target - at <= 65528)
        .unwrap_or_else(|| decode::lookbehind(anchor, metadata.bom, metadata.encoding));
    let end = if viewport.backwards {
        upper.saturating_add(8)
    } else {
        anchor.saturating_add(limit * 4 + 8)
    };
    let count = end.saturating_sub(offset).min(65536).min(u64::MAX - offset) as u32;
    (offset, count.max(1), anchor)
}

pub fn viewport(
    context: Context,
    session: Arc<Session>,
    data: Vec<u8>,
    offset: u64,
    anchor: u64,
    request: Viewport,
    eof: bool,
) {
    let Some(decoded) = decode_window(&session, &data, offset, anchor, &request, eof, || {
        context.is_cancelled()
    }) else {
        return;
    };
    let page_eof =
        eof && decoded.anchors.last().unwrap().source_offset == offset + data.len() as u64;
    let anchor = decoded
        .anchors
        .iter()
        .find(|a| a.source_offset >= anchor)
        .unwrap_or_else(|| decoded.anchors.last().unwrap())
        .source_offset;
    let upper = decoded
        .anchors
        .iter()
        .take_while(|a| a.source_offset <= request.source_anchor)
        .last()
        .map_or(request.source_anchor, |a| a.source_offset);
    let (plain, known) = {
        let hints = lock(&session.hints);
        (
            hints.plain,
            hints
                .checkpoints
                .contains(&decoded.anchors[0].source_offset),
        )
    };
    let mut page = if plain || !known {
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
        let rendered = if request.backwards {
            markdown::render_backward(
                &decoded,
                session.revision,
                page_eof,
                upper,
                request.max_bytes.min(8192) as usize,
                request.max_lines as usize,
                || context.is_cancelled(),
            )
        } else {
            markdown::render_window(
                &decoded,
                session.revision,
                page_eof,
                anchor,
                request.max_bytes.min(8192) as usize,
                request.max_lines as usize,
                || context.is_cancelled(),
            )
        };
        let Some(page) = rendered else {
            return;
        };
        let checkpoints = markdown::checkpoints(&decoded, || context.is_cancelled());
        let mut hints = lock(&session.hints);
        hints.checkpoints.extend(checkpoints);
        while hints.checkpoints.len() > 256 {
            let second = *hints.checkpoints.iter().nth(1).unwrap();
            hints.checkpoints.remove(&second);
        }
        page
    };
    if context.is_cancelled() {
        return;
    }
    page.has_before = page.source.offset > lock(&session.metadata).bom;
    if plain {
        let text = page.blocks.first_mut().map(|b| &mut b.text);
        if let Some(text) = text {
            text.has_before = page.has_before;
            send(context, Output::text(text));
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
        send(context, Output::document(&page));
    }
}

pub fn search_text(
    session: &Session,
    decoded: &Decoded,
    anchor: u64,
    eof: bool,
    cancelled: impl Fn() -> bool,
) -> Option<(TextPage, u64)> {
    let revision = session.revision;
    let anchor = decoded
        .anchors
        .iter()
        .find(|a| a.source_offset >= anchor)
        .unwrap_or_else(|| decoded.anchors.last().unwrap())
        .source_offset;
    let known = {
        let hints = lock(&session.hints);
        !hints.plain
            && hints
                .checkpoints
                .contains(&decoded.anchors[0].source_offset)
    };
    let document = if known {
        let document =
            markdown::render_window(decoded, revision, eof, anchor, 8192, 4096, &cancelled)?;
        let mut hints = lock(&session.hints);
        hints
            .checkpoints
            .extend(markdown::checkpoints(decoded, &cancelled));
        while hints.checkpoints.len() > 256 {
            let second = *hints.checkpoints.iter().nth(1).unwrap();
            hints.checkpoints.remove(&second);
        }
        document
    } else {
        markdown::crop(markdown::plain(decoded, revision, eof), anchor, 8192, 4096)
    };
    let next = document.source.end().ok()?;
    let has_after = document.has_after;
    let mut text = String::new();
    let mut anchors = Vec::new();
    for block in document.blocks {
        if !text.is_empty() {
            text.push('\n');
            anchors.push(Anchor {
                text_offset: text.len() as u32,
                source_offset: block.text.source.offset,
            });
        }
        let base = text.len() as u32;
        if anchors.is_empty() {
            anchors.push(Anchor {
                text_offset: 0,
                source_offset: block.text.source.offset,
            });
        }
        anchors.extend(block.text.anchors.into_iter().skip(1).map(|a| Anchor {
            text_offset: base + a.text_offset,
            source_offset: a.source_offset,
        }));
        text.push_str(&block.text.text);
    }
    if anchors.is_empty() {
        anchors.push(Anchor {
            text_offset: 0,
            source_offset: next,
        });
    }
    let source = SourceRange {
        offset: anchors[0].source_offset,
        length: anchors.last()?.source_offset - anchors[0].source_offset,
    };
    Some((
        TextPage {
            revision,
            has_before: source.offset != 0,
            source,
            text,
            spans: Vec::new(),
            anchors,
            has_after,
            partial_line: false,
        },
        next,
    ))
}

pub fn decode_window(
    session: &Session,
    data: &[u8],
    offset: u64,
    anchor: u64,
    request: &Viewport,
    eof: bool,
    cancelled: impl Fn() -> bool,
) -> Option<Decoded> {
    let (encoding, bom) = {
        let metadata = lock(&session.metadata);
        (metadata.encoding, metadata.bom)
    };
    let known = lock(&session.hints).checkpoints.contains(&offset);
    let mut decoded = decode::fragment(data, (offset, encoding, known), eof, 65536, &cancelled)?;
    let target = if request.backwards {
        request.source_anchor
    } else {
        anchor
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
