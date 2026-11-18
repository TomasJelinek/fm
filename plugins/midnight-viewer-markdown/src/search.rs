//! Streaming KMP search, preserving source positions through case folding.
//!
//! Each step reads one 64 KiB window, parses it once and matches the displayed
//! text of every top-level unit the parser proved complete, then continues at
//! the last proven boundary. Search keeps that boundary privately: its scan
//! never changes the viewport's checkpoints. Without a proven boundary within
//! the replay distance it matches decoded source text, as the viewport shows.
use crate::{
    decode::{self, Decoded},
    fence::{self, OpenBlock},
    markdown,
    paging::REPLAY_BYTES,
    plugin::{self, Permit, Session, lock},
};
use midnight_plugin_sdk::{
    Context, Output,
    api::{abi, messages, viewer::*},
};
use std::{
    collections::VecDeque,
    sync::{Arc, atomic::AtomicUsize},
    time::{Duration, Instant},
};

/// Bytes read per step: the same bounded window a viewport parses.
const STEP_BYTES: u64 = 65536;
/// Displayed scalars between cancellation polls; each poll may call the host.
const POLL_SCALARS: usize = 4096;

struct Scan {
    session: Arc<Session>,
    work: Arc<AtomicUsize>,
    request: messages::Search,
    /// Source offset of the next displayed text to match.
    offset: u64,
    /// The latest proven top-level boundary at or before `offset` found by
    /// this scan.
    boundary: Option<u64>,
    /// Semantic blocks are joined by a line break, also across steps.
    separate: Option<u64>,
    /// A top-level fence, table or HTML block longer than a read, whose end
    /// plain steps look for.
    open_block: Option<OpenBlock>,
    query: Vec<char>,
    prefix: Vec<usize>,
    matched: usize,
    positions: VecDeque<(u64, u64)>,
    last: Option<messages::SearchMatch>,
    progress: Instant,
    scalars: usize,
}

enum Outcome {
    Found(messages::SearchMatch),
    Continue,
    Finished,
}

pub fn start(
    context: Context,
    session: Arc<Session>,
    request: messages::Search,
    work: Arc<AtomicUsize>,
) {
    let query: Vec<char> = request
        .query
        .chars()
        .flat_map(|c| Fold::new(c, request.case_sensitive))
        .collect();
    if query.is_empty() {
        let _ = context.complete();
        return;
    }
    let mut prefix = vec![0; query.len()];
    for i in 1..query.len() {
        let mut j = prefix[i - 1];
        while j > 0 && query[i] != query[j] {
            j = prefix[j - 1];
        }
        if query[i] == query[j] {
            j += 1;
        }
        prefix[i] = j;
    }
    let bom = lock(&session.metadata).bom;
    let (offset, boundary) = if request.backwards {
        (bom, Some(bom))
    } else {
        (request.source_anchor.max(bom), None)
    };
    step(
        context,
        Scan {
            session,
            work,
            request,
            offset,
            boundary,
            separate: None,
            open_block: None,
            query,
            prefix,
            matched: 0,
            positions: VecDeque::new(),
            last: None,
            progress: Instant::now(),
            scalars: 0,
        },
    );
}

/// Case folding of one displayed scalar without allocating.
enum Fold {
    One(Option<char>),
    Lower(std::char::ToLowercase),
}

impl Fold {
    fn new(ch: char, sensitive: bool) -> Self {
        if sensitive || ch.is_ascii() {
            Self::One(Some(if sensitive {
                ch
            } else {
                ch.to_ascii_lowercase()
            }))
        } else {
            Self::Lower(ch.to_lowercase())
        }
    }
}

impl Iterator for Fold {
    type Item = char;
    fn next(&mut self) -> Option<char> {
        match self {
            Self::One(ch) => ch.take(),
            Self::Lower(chars) => chars.next(),
        }
    }
}

fn step(context: Context, mut scan: Scan) {
    if context.is_cancelled() {
        return;
    }
    if scan.request.backwards && scan.offset >= scan.request.source_anchor {
        finish(context, scan.last);
        return;
    }
    let (bom, encoding) = {
        let metadata = lock(&scan.session.metadata);
        (metadata.bom, metadata.encoding)
    };
    // Semantic reconstruction needs a proven boundary close before the scan.
    let lower = scan.offset.saturating_sub(REPLAY_BYTES).max(bom);
    let start = {
        let hints = lock(&scan.session.hints);
        if hints.plain {
            None
        } else {
            scan.boundary
                .filter(|boundary| (lower..=scan.offset).contains(boundary))
                .or_else(|| hints.checkpoints.latest(lower..=scan.offset))
        }
    };
    let read_offset = start.unwrap_or_else(|| {
        // A pending block scan continues contiguously from its resume line.
        let near = decode::lookbehind(scan.offset, bom, encoding);
        scan.open_block
            .map(|open| open.resume)
            .filter(|&resume| resume <= near && near - resume <= STEP_BYTES / 2)
            .unwrap_or(near)
    });
    let count = STEP_BYTES.min(u64::MAX - read_offset) as u32;
    if count == 0 {
        finish(context, scan.last);
        return;
    }
    plugin::read(
        context,
        scan.session.clone(),
        read_offset,
        count,
        Box::new(move |context, data, eof| {
            // Only decoding/parsing/matching holds a work slot, not the read.
            let Some(_permit) = Permit::try_acquire(&scan.work) else {
                let _ = context.fail(abi::QUEUE_FULL, "Viewer work budget exhausted");
                return;
            };
            let cancelled = || context.is_cancelled();
            let boundary = start.is_some() || read_offset == bom;
            let location = (read_offset, encoding, boundary);
            let Some(mut decoded) = decode::fragment(&data, location, eof, 65536, cancelled) else {
                return;
            };
            let mut semantic = start.is_some();
            let reaches = decoded
                .anchors
                .last()
                .is_some_and(|last| last.source_offset > scan.offset);
            if !reaches && !(eof && decoded.consumed == data.len()) {
                // Control/invalid substitutions expanded this window's text
                // before the scan position. Match the same read in plain text.
                let near = decode::lookbehind(scan.offset, bom, encoding).max(read_offset);
                let skip = (near - read_offset).min(data.len() as u64) as usize;
                let location = (near, encoding, near == bom);
                let Some(nearer) = decode::fragment(&data[skip..], location, eof, 65536, cancelled)
                else {
                    return;
                };
                decoded = nearer;
                decoded.consumed += skip;
                semantic = false;
            }
            let decoded_eof = eof && decoded.consumed == data.len();
            let outcome = if semantic {
                match_parsed(&context, &mut scan, &decoded, decoded_eof)
            } else {
                match_plain(&context, &mut scan, &decoded, decoded_eof)
            };
            match outcome {
                None => {}
                Some(Outcome::Found(found)) => finish(context, Some(found)),
                Some(Outcome::Finished) => finish(context, scan.last),
                Some(Outcome::Continue) => continue_scan(context, scan),
            }
        }),
    );
}

fn continue_scan(context: Context, mut scan: Scan) {
    if scan.progress.elapsed() < Duration::from_millis(250) {
        step(context, scan);
        return;
    }
    scan.progress = Instant::now();
    let progress = messages::Progress {
        completed: scan.offset,
        total: scan.session.length.filter(|&length| length >= scan.offset),
        phase: 1,
    };
    match Output::progress(&progress) {
        Ok(output) => {
            if let Err(error) = context.send(output, Box::new(move |context| step(context, scan))) {
                let _ = context.fail(error.status, "Search progress queue full");
            }
        }
        Err(_) => {
            let _ = context.fail(abi::INVALID, "Invalid search progress");
        }
    }
}

/// Match the displayed text of every unit proven complete in this read, then
/// continue at the last proven boundary. A unit longer than the read is
/// matched as parsed and the scan continues after the read in plain text.
fn match_parsed(
    context: &Context,
    scan: &mut Scan,
    decoded: &Decoded,
    eof: bool,
) -> Option<Outcome> {
    let parsed = markdown::render_context(decoded, scan.session.revision, eof, || {
        context.is_cancelled()
    })?;
    let read_end = decoded.anchors.last()?.source_offset;
    let proven = parsed.proven_end;
    let progress = proven > scan.offset && proven <= read_end;
    for block in &parsed.page.blocks {
        let source = &block.text.source;
        if progress && source.offset >= proven {
            // Units after the last proven boundary may continue past this
            // read; the next step parses them completely.
            break;
        }
        let end = source.end().ok()?;
        // Skip text already matched; an empty rule at the scan position still
        // separates its neighbours.
        if end < scan.offset || (end == scan.offset && source.length != 0) {
            continue;
        }
        if let Some(previous) = scan.separate
            && let Some(found) = feed(scan, '\n', previous, source.offset)
        {
            return Some(Outcome::Found(found));
        }
        let anchors = &block.text.anchors;
        if let Some(found) = feed_text(context, scan, &block.text.text, anchors, u64::MAX)? {
            return Some(Outcome::Found(found));
        }
        scan.separate = Some(end);
    }
    if eof {
        return Some(Outcome::Finished);
    }
    if progress {
        scan.offset = proven;
        scan.boundary = Some(proven);
    } else {
        scan.offset = read_end;
        scan.separate = None;
        if parsed.open_block.is_some() {
            scan.open_block = parsed.open_block;
        }
    }
    Some(Outcome::Continue)
}

/// Match decoded source text after the scan position, as the plain viewport
/// fallback displays it.
fn match_plain(
    context: &Context,
    scan: &mut Scan,
    decoded: &Decoded,
    eof: bool,
) -> Option<Outcome> {
    scan.separate = None;
    // The end of a long fence, table or HTML block proves the next checkpoint:
    // match plain text only up to it, then continue semantically from there.
    // An abandoned scan leaves the rest plain; the session checkpoints are
    // never changed here.
    let mut until = u64::MAX;
    if let Some(mut open) = scan.open_block.take() {
        match open.advance(decoded) {
            fence::Scan::Closed(boundary) => {
                scan.boundary = Some(boundary);
                if boundary <= scan.offset {
                    return Some(Outcome::Continue);
                }
                until = boundary;
            }
            fence::Scan::Open => scan.open_block = Some(open),
            fence::Scan::Gap | fence::Scan::Abandoned => {}
        }
    }
    let anchors = &decoded.anchors;
    if let Some(found) = feed_text(context, scan, &decoded.text, anchors, until)? {
        return Some(Outcome::Found(found));
    }
    if until != u64::MAX {
        scan.offset = until;
        return Some(Outcome::Continue);
    }
    let end = anchors.last()?.source_offset;
    if eof {
        return Some(Outcome::Finished);
    }
    if end <= scan.offset {
        let _ = context.fail(abi::INVALID, "Search decoder made no progress");
        return None;
    }
    scan.offset = end;
    Some(Outcome::Continue)
}

/// Feed displayed text from the scan position up to `until`; `anchors` are its
/// exact, ascending source boundaries (first at 0, last at the text length).
fn feed_text(
    context: &Context,
    scan: &mut Scan,
    text: &str,
    anchors: &[Anchor],
    until: u64,
) -> Option<Option<messages::SearchMatch>> {
    let Some(begin) = anchors.iter().position(|a| a.source_offset >= scan.offset) else {
        return Some(None);
    };
    let mut left = begin;
    let mut right = begin;
    let begin = anchors[begin].text_offset as usize;
    for (at, ch) in text.get(begin..)?.char_indices() {
        let at = begin + at;
        let end = at + ch.len_utf8();
        while anchors.get(left + 1)?.text_offset as usize <= at {
            left += 1;
        }
        right = right.max(left);
        while (anchors.get(right)?.text_offset as usize) < end {
            right += 1;
        }
        scan.scalars += 1;
        if scan.scalars.is_multiple_of(POLL_SCALARS) && context.is_cancelled() {
            return None;
        }
        let (start, end) = (anchors[left].source_offset, anchors[right].source_offset);
        if start >= until {
            break;
        }
        if let Some(found) = feed(scan, ch, start, end) {
            return Some(Some(found));
        }
    }
    Some(None)
}

/// Advance the KMP automaton by one displayed scalar spanning `start..end`.
/// Forward searches return their first match; backward searches retain the
/// latest match ending at or before the anchor.
fn feed(scan: &mut Scan, ch: char, start: u64, end: u64) -> Option<messages::SearchMatch> {
    for value in Fold::new(ch, scan.request.case_sensitive) {
        scan.positions.push_back((start, end));
        if scan.positions.len() > scan.query.len() {
            scan.positions.pop_front();
        }
        while scan.matched > 0 && value != scan.query[scan.matched] {
            scan.matched = scan.prefix[scan.matched - 1];
        }
        if value == scan.query[scan.matched] {
            scan.matched += 1;
        }
        if scan.matched == scan.query.len() {
            let start = scan.positions.front().map_or(start, |position| position.0);
            let end = scan.positions.back().map_or(end, |position| position.1);
            let anchor = scan.request.source_anchor;
            let wanted = if scan.request.backwards {
                end <= anchor
            } else {
                start >= anchor
            };
            if end > start && wanted {
                let found = messages::SearchMatch {
                    source: SourceRange {
                        offset: start,
                        length: end - start,
                    },
                    preview: scan.request.query.clone(),
                };
                if !scan.request.backwards {
                    return Some(found);
                }
                scan.last = Some(found);
            }
            scan.matched = scan.prefix[scan.matched - 1];
        }
    }
    None
}

fn finish(context: Context, found: Option<messages::SearchMatch>) {
    if let Some(found) = found {
        plugin::send(context, Output::search_match(&found));
    } else {
        let _ = context.complete();
    }
}
