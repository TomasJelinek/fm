use crate::{
    checkpoints::{Batch as CheckpointBatch, Checkpoints},
    decode::{self, Decoded, Encoding, Tail},
    plugin::{Session, lock, send},
    preparation,
    resume::Snapshot,
    syntax::{self, Language, State},
};
use midnight_plugin_sdk::{
    Context, Output,
    api::{messages::Viewport, viewer::*},
};
use std::sync::Arc;

pub struct Hints {
    name: String,
    language: Option<Language>,
    plain: bool,
    checkpoints: Checkpoints,
}
impl Hints {
    pub fn new(name: &str, mode: u16) -> Self {
        Self {
            name: name.into(),
            language: None,
            plain: mode == MODE_TEXT,
            checkpoints: Checkpoints::new(),
        }
    }
    pub fn initialize(&mut self, prefix: &str, bom: u64) {
        self.language = syntax::detect(&self.name, prefix);
        self.insert_checkpoint(bom, State::default());
    }

    fn insert_checkpoint(&mut self, offset: u64, state: State) {
        self.checkpoints.insert(offset, state);
    }

    fn highlights(&self) -> bool {
        !self.plain && self.language.is_some()
    }
}

pub(crate) struct ReadPlan {
    pub(crate) offset: u64,
    pub(crate) count: u32,
    pub(crate) anchor: u64,
    pub(crate) snapshot: Option<Snapshot>,
}

pub(crate) fn preparation_origin(session: &Session, lower: u64) -> Option<(Language, Snapshot)> {
    let hints = lock(&session.hints);
    if hints.plain {
        return None;
    }
    let language = hints.language?;
    let (offset, state) = hints.checkpoints.nearest(lower)?;
    Some((language, Snapshot::new(offset, state, true)))
}

pub(crate) fn commit_checkpoints(session: &Session, checkpoints: &CheckpointBatch) {
    let mut hints = lock(&session.hints);
    for (offset, state) in checkpoints.iter() {
        hints.insert_checkpoint(offset, state);
    }
}

/// Raw bytes that can hold the requested UTF-8 text and normalized line breaks.
fn page_span(encoding: Encoding, limit: u64, lines: u64) -> u64 {
    match encoding {
        Encoding::Utf8 => limit + lines + 8,
        Encoding::Le | Encoding::Be => 2 * (limit + lines) + 8,
    }
}

pub fn plan(session: &Session, viewport: &Viewport) -> ReadPlan {
    let metadata = lock(&session.metadata);
    if let Some(snapshot) = lock(&session.preparation).take_ready(session.revision, viewport)
        && snapshot.offset < u64::MAX
    {
        let end = preparation::end(viewport, session.length, metadata.bom);
        let count = end
            .saturating_sub(snapshot.offset)
            .min(65536)
            .min(u64::MAX - snapshot.offset) as u32;
        return ReadPlan {
            offset: snapshot.offset,
            count: count.max(1),
            anchor: if viewport.backwards {
                preparation::lower(viewport, metadata.bom)
            } else {
                viewport.source_anchor.max(metadata.bom)
            },
            snapshot: Some(snapshot),
        };
    }
    let limit = viewport.max_bytes.min(8192) as u64;
    let lines = u64::from(viewport.max_lines);
    let upper = viewport.source_anchor.max(metadata.bom);
    let anchor = if viewport.backwards {
        upper
            .saturating_sub(page_span(metadata.encoding, limit, lines + 1))
            .max(metadata.bom)
    } else {
        upper
    };
    let checkpoint = {
        let hints = lock(&session.hints);
        if hints.highlights() {
            hints.checkpoints.nearest(anchor)
        } else {
            None
        }
    };
    let target = if viewport.backwards { upper } else { anchor };
    let snapshot = checkpoint
        .filter(|&(at, _)| anchor - at <= 32768 && target - at <= 65528)
        .map(|(at, state)| Snapshot::new(at, state, true));
    let offset = snapshot.map_or_else(
        || decode::lookbehind(anchor, metadata.bom, metadata.encoding),
        |snapshot| snapshot.offset,
    );
    let end = if viewport.backwards {
        upper.saturating_add(8)
    } else {
        anchor
            .saturating_add(page_span(metadata.encoding, limit, lines))
            .saturating_add(if snapshot.is_some() {
                preparation::LOOKAHEAD
            } else {
                0
            })
    };
    let count = end.saturating_sub(offset).min(65536).min(u64::MAX - offset) as u32;
    ReadPlan {
        offset,
        count: count.max(1),
        anchor,
        snapshot,
    }
}

pub fn viewport(
    context: Context,
    session: Arc<Session>,
    data: Vec<u8>,
    plan: ReadPlan,
    request: Viewport,
    eof: bool,
) {
    let ReadPlan {
        offset,
        anchor,
        mut snapshot,
        ..
    } = plan;
    let (encoding, bom) = {
        let metadata = lock(&session.metadata);
        (metadata.encoding, metadata.bom)
    };
    if request.backwards && request.source_anchor < bom {
        let Some(empty) = decode::decode(&[], 0, encoding, true, 0, 0, || context.is_cancelled())
        else {
            return;
        };
        send(
            context,
            Output::text(&empty.page(session.revision, eof && data.is_empty())),
        );
        return;
    }
    let limit = request.max_bytes.min(8192) as usize;
    let tail = (!request.backwards).then_some(Tail {
        source: anchor,
        bytes: limit,
        lines: request.max_lines as usize,
    });
    let lookahead = if snapshot.is_some() { 128 } else { 0 };
    let known = snapshot.is_some() || offset == bom;
    let Some(mut decoded) = decode::fragment_with_lookahead(
        &data,
        (offset, encoding, known),
        eof,
        65536,
        tail,
        lookahead,
        || context.is_cancelled(),
    ) else {
        return;
    };
    let target = if request.backwards {
        request.source_anchor
    } else {
        anchor
    };
    let decoded_end = decoded.anchors.last().unwrap().source_offset;
    let read_end = offset + data.len() as u64;
    if decoded_end < target.min(read_end)
        || (!request.backwards && decoded_end == target && target < read_end)
    {
        // Expanding malformed/control prefixes can exhaust the decoded context
        // budget before the viewport. Reuse the already bounded raw read and
        // decode only a near-anchor fragment; syntax then falls back to plain.
        let lower = if request.backwards {
            target.saturating_sub(u64::from(request.max_bytes.min(8192)) * 2 + 8)
        } else {
            target
        };
        let near = decode::lookbehind(lower, bom, encoding).max(offset);
        let skip = (near - offset).min(data.len() as u64) as usize;
        let Some(fallback) = decode::fragment(
            &data[skip..],
            (offset + skip as u64, encoding, near == bom),
            eof,
            65536,
            tail,
            || context.is_cancelled(),
        ) else {
            return;
        };
        decoded = fallback;
        snapshot = None;
    }
    let begin = decoded
        .anchors
        .iter()
        .position(|a| a.source_offset >= anchor)
        .unwrap_or(decoded.anchors.len() - 1);
    let (start, end) = if request.backwards {
        let end = decoded
            .anchors
            .iter()
            .take_while(|a| a.source_offset <= request.source_anchor)
            .last()
            .map_or(0, |a| a.text_offset as usize);
        (
            decode::suffix_start(
                &decoded.text,
                end,
                request.max_bytes.min(8192) as usize,
                request.max_lines as usize,
            ),
            end,
        )
    } else {
        let start = decoded.anchors[begin].text_offset as usize;
        let mut end = start;
        let mut lines = 0;
        for ch in decoded.text[start..].chars() {
            if end - start + ch.len_utf8() > request.max_bytes.min(8192) as usize
                || lines >= request.max_lines
            {
                break;
            }
            end += ch.len_utf8();
            if ch == '\n' {
                lines += 1;
            }
        }
        (start, end)
    };
    let last_source = decoded.source_at(end).unwrap();
    let page_eof = eof && last_source == offset + data.len() as u64;
    let selected = subset(&decoded, start, end);
    let mut page = selected.page(session.revision, page_eof);
    page.has_before = page.source.offset > bom;
    page.partial_line |= if start == 0 {
        snapshot.is_some_and(|snapshot| !snapshot.line_start)
    } else {
        !decoded.text[..start].ends_with('\n')
    };
    let (language, plain) = {
        let hints = lock(&session.hints);
        (hints.language, hints.plain)
    };
    if !plain && let (Some(language), Some(snapshot)) = (language, snapshot) {
        let mut state = snapshot.state;
        let mut styles = crate::styles::Styles::new(start..end, request.backwards);
        let mut cursor = 0;
        let mut next_poll = 0;
        let mut checkpoints = CheckpointBatch::new();
        let mut valid = true;
        let mut lookahead = end;
        for ch in decoded.text[end..].chars() {
            if lookahead - end + ch.len_utf8() > 128 {
                break;
            }
            lookahead += ch.len_utf8();
        }
        for line in decoded.text[..lookahead].split_inclusive('\n') {
            if cursor >= next_poll {
                next_poll = cursor + 4096;
                if context.is_cancelled()
                    || session.closed.load(std::sync::atomic::Ordering::Acquire)
                {
                    return;
                }
            }
            styles.origin(cursor);
            let final_chunk = eof
                && cursor + line.len() == decoded.text.len()
                && decoded
                    .anchors
                    .last()
                    .is_some_and(|a| a.source_offset == offset + data.len() as u64);
            // As on main, poll host cancellation between bounded lexical chunks,
            // rather than crossing the plugin ABI for each line or token.
            let scanned =
                syntax::stream(line, language, &mut state, &mut styles, final_chunk, || {
                    false
                });
            let Some(consumed) = scanned else {
                valid = false;
                break;
            };
            cursor += consumed;
            if consumed == line.len() && line.ends_with('\n') {
                checkpoints.push((decoded.source_at(cursor).unwrap(), state));
            }
            if consumed < line.len() {
                break;
            }
        }
        if valid {
            let (mut spans, window, clipped) = styles.finish_with_hints(snapshot.hints);
            if clipped
                && crate::page_budget::crop(&mut page, window.start - start, window.end - start)
                    .is_err()
            {
                let _ = context.fail(
                    midnight_plugin_sdk::api::abi::INVALID,
                    "Invalid style fragment boundary",
                );
                return;
            }
            for span in &mut spans {
                span.start -= window.start as u32;
                span.end -= window.start as u32;
            }
            page.spans = spans;
            if !context.is_cancelled() && !session.closed.load(std::sync::atomic::Ordering::Acquire)
            {
                commit_checkpoints(&session, &checkpoints);
            }
        } else if !context.is_cancelled()
            && !session.closed.load(std::sync::atomic::Ordering::Acquire)
        {
            let _ = context.fail(
                midnight_plugin_sdk::api::abi::INVALID,
                "Syntax continuation exceeded its state budget",
            );
            return;
        }
    }
    if context.is_cancelled() || session.closed.load(std::sync::atomic::Ordering::Acquire) {
        return;
    }
    send(context, crate::page_budget::output(page, request.backwards));
}

fn subset(decoded: &Decoded, start: usize, end: usize) -> Decoded {
    let anchors: Vec<_> = decoded
        .anchors
        .iter()
        .filter(|a| (start..=end).contains(&(a.text_offset as usize)))
        .map(|a| Anchor {
            text_offset: a.text_offset - start as u32,
            source_offset: a.source_offset,
        })
        .collect();
    let consumed = (anchors.last().unwrap().source_offset - anchors[0].source_offset) as usize;
    Decoded {
        text: decoded.text[start..end].into(),
        anchors,
        consumed,
    }
}

#[cfg(test)]
#[path = "paging_tests.rs"]
mod tests;
