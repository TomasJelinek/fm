use crate::{
    decode::{self, Decoded},
    plugin::{Session, lock, send},
    syntax::{self, Language, State},
};
use midnight_plugin_sdk::{
    Context, Output,
    api::{messages::Viewport, viewer::*},
};
use std::{collections::BTreeMap, sync::Arc};

// Keep the BOM plus the latest complete-line states, including during insertion.
const MAX_CHECKPOINTS: usize = 256;

// Only the latest 255 staged states can survive the live cache's retention rule.
// Inline ring storage has no geometric allocation growth on newline-dense input.
struct CheckpointBatch {
    entries: [(u64, State); MAX_CHECKPOINTS - 1],
    len: usize,
    next: usize,
}

const _: () = assert!(size_of::<CheckpointBatch>() <= 8192);

impl CheckpointBatch {
    fn new() -> Self {
        Self {
            entries: [(0, State::default()); MAX_CHECKPOINTS - 1],
            len: 0,
            next: 0,
        }
    }

    fn push(&mut self, checkpoint: (u64, State)) {
        self.entries[self.next] = checkpoint;
        self.next = (self.next + 1) % self.entries.len();
        self.len = (self.len + 1).min(self.entries.len());
    }

    fn iter(&self) -> impl Iterator<Item = (u64, State)> + '_ {
        let first = if self.len == self.entries.len() {
            self.next
        } else {
            0
        };
        (0..self.len).map(move |at| self.entries[(first + at) % self.entries.len()])
    }
}

pub struct Hints {
    name: String,
    language: Option<Language>,
    plain: bool,
    checkpoints: BTreeMap<u64, State>,
}
impl Hints {
    pub fn new(name: &str, mode: u16) -> Self {
        Self {
            name: name.into(),
            language: None,
            plain: mode == MODE_TEXT,
            checkpoints: BTreeMap::new(),
        }
    }
    pub fn initialize(&mut self, prefix: &str, bom: u64) {
        self.language = syntax::detect(&self.name, prefix);
        self.insert_checkpoint(bom, State::default());
    }

    fn insert_checkpoint(&mut self, offset: u64, state: State) {
        if let Some(previous) = self.checkpoints.get_mut(&offset) {
            *previous = state;
            return;
        }
        if self.checkpoints.len() >= MAX_CHECKPOINTS {
            let Some(first) = self.checkpoints.keys().nth(1).copied() else {
                return;
            };
            if offset < first {
                return;
            }
            // Evict before inserting, keeping the BOM and latest 255 offsets.
            self.checkpoints.remove(&first);
        }
        self.checkpoints.insert(offset, state);
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
        .map(|(&at, _)| at);
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
    let known = lock(&session.hints).checkpoints.contains_key(&offset);
    let Some(mut decoded) = decode::fragment(&data, (offset, encoding, known), eof, 65536, || {
        context.is_cancelled()
    }) else {
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
            || context.is_cancelled(),
        ) else {
            return;
        };
        decoded = fallback;
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
    let (language, state, plain) = {
        let hints = lock(&session.hints);
        (
            hints.language,
            hints
                .checkpoints
                .get(&decoded.anchors[0].source_offset)
                .copied(),
            hints.plain,
        )
    };
    if !plain && let (Some(language), Some(mut state)) = (language, state) {
        // A checkpoint is stored only immediately after a complete newline.
        // This prevents tokens or comment/string delimiters being split.
        let mut spans = Vec::new();
        let mut cursor = 0;
        let mut checkpoints = CheckpointBatch::new();
        let mut valid = true;
        for line in decoded.text[..end].split_inclusive('\n') {
            let Some(part) =
                syntax::highlight(line, language, &mut state, || context.is_cancelled())
            else {
                valid = false;
                break;
            };
            for span in part {
                let left = cursor + span.start as usize;
                let right = cursor + span.end as usize;
                if right > start && left < end {
                    spans.push(Span {
                        start: left.max(start).saturating_sub(start) as u32,
                        end: (right.min(end) - start) as u32,
                        role: span.role,
                        flags: span.flags,
                    });
                    if spans.len() > MAX_SPANS {
                        valid = false;
                        break;
                    }
                }
            }
            cursor += line.len();
            if line.ends_with('\n') {
                checkpoints.push((decoded.source_at(cursor).unwrap(), state));
            }
            if !valid {
                break;
            }
        }
        if valid {
            page.spans = spans;
            let mut hints = lock(&session.hints);
            for (offset, state) in checkpoints.iter() {
                hints.insert_checkpoint(offset, state);
            }
        }
    }
    if context.is_cancelled() {
        return;
    }
    let output = match Output::text(&page) {
        Err(midnight_plugin_sdk::api::wire::WireError::Limit) => {
            // Source anchors and styles share the CBOR item budget. Keep exact
            // selection boundaries and discard optional colouring first.
            page.spans.clear();
            Output::text(&page)
        }
        output => output,
    };
    send(context, output);
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

pub fn search_text(decoded: &Decoded, revision: u64, eof: bool) -> TextPage {
    decoded.clone().page(revision, eof)
}

#[cfg(test)]
#[path = "paging_tests.rs"]
mod tests;
