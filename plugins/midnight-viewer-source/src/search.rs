//! Streaming KMP search, preserving source positions through case folding.
use crate::{
    decode,
    plugin::{self, Session, lock},
};
use midnight_plugin_sdk::{
    Context, Output,
    api::{abi, messages, viewer::*},
};
use std::{
    char::ToLowercase,
    collections::VecDeque,
    iter::Once,
    sync::Arc,
    time::{Duration, Instant},
};

/// Source bytes requested per step. The whole read is decoded and scanned, so
/// each byte is requested once (plus at most an incomplete trailing scalar).
const READ_BYTES: u64 = 32768;
/// Decoded text per step; only control/malformed expansion can reach it.
const MAX_TEXT: usize = 64 * 1024;
/// The host cancellation callback is polled once per this many scalars.
const POLL_SCALARS: usize = 4096;

struct Scan {
    session: Arc<Session>,
    request: messages::Search,
    offset: u64,
    known_boundary: bool,
    query: Vec<char>,
    prefix: Vec<usize>,
    matched: usize,
    positions: VecDeque<(u64, u64)>,
    last: Option<messages::SearchMatch>,
    progress: Instant,
    _permit: Box<dyn FnOnce() + Send>,
}
pub fn start(
    context: Context,
    session: Arc<Session>,
    request: messages::Search,
    permit: Box<dyn FnOnce() + Send>,
) {
    let query: Vec<char> = request
        .query
        .chars()
        .flat_map(|c| fold(c, request.case_sensitive))
        .collect();
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
    let (bom, encoding) = {
        let metadata = lock(&session.metadata);
        (metadata.bom, metadata.encoding)
    };
    let offset = if request.backwards {
        bom
    } else {
        decode::lookbehind(request.source_anchor, bom, encoding)
    };
    step(
        context,
        Scan {
            session,
            request,
            offset,
            known_boundary: offset == bom,
            query,
            prefix,
            matched: 0,
            positions: VecDeque::new(),
            last: None,
            progress: Instant::now(),
            _permit: permit,
        },
    );
}

/// Case-folded scalars of `ch`, without allocating.
enum Folded {
    Exact(Once<char>),
    Lower(ToLowercase),
}

impl Iterator for Folded {
    type Item = char;

    fn next(&mut self) -> Option<char> {
        match self {
            Self::Exact(ch) => ch.next(),
            Self::Lower(lower) => lower.next(),
        }
    }
}

fn fold(ch: char, sensitive: bool) -> Folded {
    if sensitive {
        Folded::Exact(std::iter::once(ch))
    } else {
        Folded::Lower(ch.to_lowercase())
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
    let count = (u64::MAX - scan.offset).min(READ_BYTES) as u32;
    if count == 0 {
        finish(context, scan.last);
        return;
    }
    plugin::read(
        context,
        scan.session.clone(),
        scan.offset,
        count,
        Box::new(move |context, data, eof| {
            let encoding = lock(&scan.session.metadata).encoding;
            let Some(decoded) = decode::fragment(
                &data,
                (scan.offset, encoding, scan.known_boundary),
                eof,
                MAX_TEXT,
                None,
                || context.is_cancelled(),
            ) else {
                return;
            };
            let consumed = decoded.consumed;
            if consumed == 0 {
                if eof {
                    finish(context, scan.last);
                } else {
                    let _ = context.fail(abi::INVALID, "Search decoder made no progress");
                }
                return;
            }
            // The decoder records exactly one boundary per displayed scalar.
            let scalars = decoded.text.chars().zip(decoded.anchors.windows(2));
            for (index, (ch, boundary)) in scalars.enumerate() {
                if index % POLL_SCALARS == POLL_SCALARS - 1 && context.is_cancelled() {
                    return;
                }
                let start = boundary[0].source_offset;
                let end = boundary[1].source_offset;
                for value in fold(ch, scan.request.case_sensitive) {
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
                        let start = scan.positions.front().unwrap().0;
                        let end = scan.positions.back().unwrap().1;
                        if end > start
                            && if scan.request.backwards {
                                end <= scan.request.source_anchor
                            } else {
                                start >= scan.request.source_anchor
                            }
                        {
                            let found = messages::SearchMatch {
                                source: SourceRange {
                                    offset: start,
                                    length: end - start,
                                },
                                preview: scan.request.query.clone(),
                            };
                            if !scan.request.backwards {
                                finish(context, Some(found));
                                return;
                            }
                            scan.last = Some(found);
                        }
                        scan.matched = scan.prefix[scan.matched - 1];
                    }
                }
            }
            scan.offset += consumed as u64;
            scan.known_boundary = true;
            if eof && consumed == data.len() {
                finish(context, scan.last);
                return;
            }
            if scan.progress.elapsed() >= Duration::from_millis(250) {
                scan.progress = Instant::now();
                let progress = messages::Progress {
                    completed: scan.offset,
                    total: scan.session.length.filter(|&length| length >= scan.offset),
                    phase: 1,
                };
                match Output::progress(&progress) {
                    Ok(output) => {
                        if let Err(error) =
                            context.send(output, Box::new(move |context| step(context, scan)))
                        {
                            let _ = context.fail(error.status, "Search progress queue full");
                        }
                    }
                    Err(_) => {
                        let _ = context.fail(abi::INVALID, "Invalid search progress");
                    }
                }
            } else {
                step(context, scan);
            }
        }),
    );
}
fn finish(context: Context, found: Option<messages::SearchMatch>) {
    if let Some(found) = found {
        plugin::send(context, Output::search_match(&found));
    } else {
        let _ = context.complete();
    }
}
