use crate::{
    decode::{self, Decoded, Encoding, Tail},
    plugin::{Session, lock, send},
    syntax::{self, Language, State},
};
use midnight_plugin_sdk::{
    Context, Output,
    api::{messages::Viewport, viewer::*},
};
use std::{collections::BTreeMap, sync::Arc};

// At most 256 complete-line states per session: the dense line starts of the
// latest highlighted page (continuation in either direction) plus a sparse,
// evenly spaced cover of every visited region including the BOM (revisits).
const MAX_RECENT: usize = 64;
const MAX_COVERAGE: usize = 192;
const _: () = assert!(MAX_RECENT + MAX_COVERAGE == 256);
// Coverage states keep at least this distance; it doubles whenever the cover
// is full, so it stays bounded while spanning arbitrarily large files.
const INITIAL_SPACING: u64 = 2048;
// One read is at most 64 KiB, so a pass offers at most this many spaced states.
const MAX_SPACED: usize = (65536 / INITIAL_SPACING) as usize + 1;
// A page replays at most this much source before its anchor.
const MAX_REPLAY: u64 = 32768;
// The host cancellation callback is polled once per this many replayed bytes.
const POLL_BYTES: usize = 4096;

/// States produced by one highlight pass, in fixed inline storage (no growth on
/// newline-dense input). They are committed after the pass, never partially.
struct Staged {
    recent: [(u64, State); MAX_RECENT],
    recent_len: usize,
    recent_next: usize,
    spaced: [(u64, State); MAX_SPACED],
    spaced_len: usize,
    spacing: u64,
}

const _: () = assert!(size_of::<Staged>() <= 4096);

impl Staged {
    fn new(spacing: u64) -> Self {
        Self {
            recent: [(0, State::default()); MAX_RECENT],
            recent_len: 0,
            recent_next: 0,
            spaced: [(0, State::default()); MAX_SPACED],
            spaced_len: 0,
            spacing,
        }
    }

    fn push(&mut self, offset: u64, state: State) {
        self.recent[self.recent_next] = (offset, state);
        self.recent_next = (self.recent_next + 1) % MAX_RECENT;
        self.recent_len = (self.recent_len + 1).min(MAX_RECENT);
        let due = self.spaced[..self.spaced_len]
            .last()
            .is_none_or(|&(last, _)| offset >= last.saturating_add(self.spacing));
        if due && self.spaced_len < MAX_SPACED {
            self.spaced[self.spaced_len] = (offset, state);
            self.spaced_len += 1;
        }
    }

    fn recent(&self) -> impl Iterator<Item = (u64, State)> + '_ {
        let first = if self.recent_len == MAX_RECENT {
            self.recent_next
        } else {
            0
        };
        (0..self.recent_len).map(move |at| self.recent[(first + at) % MAX_RECENT])
    }
}

/// Known lexical states at complete-line source offsets.
struct Checkpoints {
    recent: BTreeMap<u64, State>,
    coverage: BTreeMap<u64, State>,
    spacing: u64,
}

impl Checkpoints {
    fn new() -> Self {
        Self {
            recent: BTreeMap::new(),
            coverage: BTreeMap::new(),
            spacing: INITIAL_SPACING,
        }
    }

    /// The nearest known state at or before `offset`.
    fn latest(&self, offset: u64) -> Option<(u64, State)> {
        let recent = self.recent.range(..=offset).next_back();
        let coverage = self.coverage.range(..=offset).next_back();
        recent
            .into_iter()
            .chain(coverage)
            .max_by_key(|(at, _)| **at)
            .map(|(&at, &state)| (at, state))
    }

    fn commit(&mut self, staged: &Staged) {
        for &(offset, state) in &staged.spaced[..staged.spaced_len] {
            self.offer(offset, state);
        }
        if staged.recent_len > 0 {
            self.recent.clear();
            self.recent.extend(staged.recent());
        }
    }

    /// Adds a coverage state unless one is already within the current spacing.
    /// A full cover doubles the spacing and keeps an evenly thinned subset, so
    /// earlier regions stay covered instead of being displaced by later ones.
    fn offer(&mut self, offset: u64, state: State) {
        if let Some(previous) = self.coverage.get_mut(&offset) {
            *previous = state;
            return;
        }
        while !self.crowded(offset) {
            if self.coverage.len() < MAX_COVERAGE {
                self.coverage.insert(offset, state);
                return;
            }
            self.spacing = self.spacing.saturating_mul(2);
            let spacing = self.spacing;
            let mut kept: Option<u64> = None;
            // The lowest entry is the BOM, which is always kept.
            self.coverage.retain(|&at, _| {
                let keep = kept.is_none_or(|previous| at - previous >= spacing);
                if keep {
                    kept = Some(at);
                }
                keep
            });
        }
    }

    fn crowded(&self, offset: u64) -> bool {
        let before = self.coverage.range(..offset).next_back();
        let after = self.coverage.range(offset..).next();
        before.is_some_and(|(&at, _)| offset - at < self.spacing)
            || after.is_some_and(|(&at, _)| at - offset < self.spacing)
    }
}

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
        self.checkpoints.coverage.insert(bom, State::default());
    }

    fn highlights(&self) -> bool {
        !self.plain && self.language.is_some()
    }
}

/// The bounded read for one viewport. `state` is the lexical state at `offset`
/// when the read starts at a checkpoint; it travels with the read so a
/// concurrent eviction cannot turn a valid replay into a plain page.
pub struct Plan {
    pub offset: u64,
    pub count: u32,
    pub anchor: u64,
    state: Option<State>,
}

/// Source bytes that can hold a page of `limit` text bytes and `lines` line
/// breaks. Decoded UTF-8 never shrinks except a CRLF (two bytes, one `\n`);
/// UTF-16 takes two bytes per text byte plus two more per CRLF. Eight more
/// bytes complete a split scalar or CRLF at either end of the page.
fn page_span(encoding: Encoding, limit: u64, lines: u64) -> u64 {
    match encoding {
        Encoding::Utf8 => limit + lines + 8,
        Encoding::Le | Encoding::Be => 2 * (limit + lines) + 8,
    }
}

pub fn plan(session: &Session, viewport: &Viewport) -> Plan {
    let metadata = lock(&session.metadata);
    let limit = viewport.max_bytes.min(8192) as u64;
    let lines = u64::from(viewport.max_lines);
    let upper = viewport.source_anchor.max(metadata.bom);
    let anchor = if viewport.backwards {
        // A backward page can also end with its own trailing line break.
        let span = page_span(metadata.encoding, limit, lines + 1);
        upper.saturating_sub(span).max(metadata.bom)
    } else {
        upper
    };
    let target = if viewport.backwards { upper } else { anchor };
    let checkpoint = {
        let hints = lock(&session.hints);
        // Without highlighting a checkpoint would only lengthen the read.
        if hints.highlights() {
            hints.checkpoints.latest(anchor)
        } else {
            None
        }
    }
    .filter(|&(at, _)| anchor - at <= MAX_REPLAY && target - at <= 65528);
    let (offset, state) = match checkpoint {
        Some((at, state)) => (at, Some(state)),
        None => (
            decode::lookbehind(anchor, metadata.bom, metadata.encoding),
            None,
        ),
    };
    let end = if viewport.backwards {
        upper.saturating_add(8)
    } else {
        anchor.saturating_add(page_span(metadata.encoding, limit, lines))
    };
    let count = end.saturating_sub(offset).min(65536).min(u64::MAX - offset) as u32;
    Plan {
        offset,
        count: count.max(1),
        anchor,
        state,
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
    let Plan {
        offset,
        anchor,
        state,
        ..
    } = plan;
    let limit = request.max_bytes.min(8192) as usize;
    // A forward page needs no decoded text after its own end.
    let tail = (!request.backwards).then_some(Tail {
        source: anchor,
        bytes: limit,
        lines: request.max_lines as usize,
    });
    let known = state.is_some() || offset == bom;
    let Some(mut decoded) =
        decode::fragment(&data, (offset, encoding, known), eof, 65536, tail, || {
            context.is_cancelled()
        })
    else {
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
            decode::suffix_start(&decoded.text, end, limit, request.max_lines as usize),
            end,
        )
    } else {
        let start = decoded.anchors[begin].text_offset as usize;
        let mut end = start;
        let mut lines = 0;
        for ch in decoded.text[start..].chars() {
            if end - start + ch.len_utf8() > limit || lines >= request.max_lines {
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
    let (language, plain) = {
        let hints = lock(&session.hints);
        (hints.language, hints.plain)
    };
    // The planned state describes the read offset; the near-anchor fallback
    // decodes from elsewhere and keeps only the BOM's known initial state.
    let first = decoded.anchors[0].source_offset;
    let state = if first == offset {
        state
    } else if first == bom {
        Some(State::default())
    } else {
        None
    };
    if !plain
        && let (Some(language), Some(state)) = (language, state)
        && !highlight(
            &context,
            &session,
            &decoded,
            (start, end),
            language,
            state,
            &mut page,
        )
    {
        return;
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

/// Lexes `decoded.text[..end]` line by line from the state at the decoded
/// start, styles the page range `[start, end)` and commits the states of all
/// completed lines. An exhausted span budget or a nesting limit only leaves
/// the page plain; states reached before it are still exact. Returns false
/// when the request was cancelled.
fn highlight(
    context: &Context,
    session: &Session,
    decoded: &Decoded,
    (start, end): (usize, usize),
    language: Language,
    mut state: State,
    page: &mut TextPage,
) -> bool {
    let mut staged = Staged::new(lock(&session.hints).checkpoints.spacing);
    let mut spans = Vec::new();
    let mut styled = true;
    let mut cursor = 0;
    let mut next_poll = 0;
    for line in decoded.text[..end].split_inclusive('\n') {
        if cursor >= next_poll {
            next_poll = cursor + POLL_BYTES;
            if context.is_cancelled() {
                return false;
            }
        }
        // A checkpoint is stored only immediately after a complete newline.
        // This prevents tokens or comment/string delimiters being split.
        let Some(lexed) = syntax::highlight(line, language, &mut state, || false) else {
            styled = false;
            break;
        };
        let line_end = cursor + line.len();
        if styled && line_end > start {
            for span in &lexed.spans {
                let left = cursor + span.start as usize;
                let right = cursor + span.end as usize;
                if right > start && left < end {
                    spans.push(Span {
                        start: left.max(start).saturating_sub(start) as u32,
                        end: (right.min(end) - start) as u32,
                        role: span.role,
                        flags: span.flags,
                    });
                }
            }
            // A partially styled page would misrepresent the omitted tokens.
            styled = !lexed.truncated && spans.len() <= MAX_SPANS;
        }
        cursor = line_end;
        if line.ends_with('\n') {
            staged.push(decoded.source_at(cursor).unwrap(), state);
        }
    }
    if styled {
        page.spans = spans;
    }
    lock(&session.hints).checkpoints.commit(&staged);
    true
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
