//! Streaming KMP search, preserving source positions through case folding.
use crate::{
    paging,
    plugin::{self, Session, lock},
};
use midnight_plugin_sdk::{
    Context, Output,
    api::{abi, messages, viewer::*},
};
use std::{
    collections::VecDeque,
    sync::Arc,
    time::{Duration, Instant},
};

struct Scan {
    session: Arc<Session>,
    request: messages::Search,
    offset: u64,
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
    let bom = lock(&session.metadata).bom;
    let offset = if request.backwards {
        bom
    } else {
        request.source_anchor.max(bom)
    };
    step(
        context,
        Scan {
            session,
            request,
            offset,
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
fn fold(ch: char, sensitive: bool) -> Vec<char> {
    if sensitive {
        vec![ch]
    } else {
        ch.to_lowercase().collect()
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
    let viewport = messages::Viewport {
        source_anchor: scan.offset,
        backwards: false,
        max_bytes: 8192,
        max_lines: 4096,
    };
    let (read_offset, count, _) = paging::plan(&scan.session, &viewport);
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
            let Some(decoded) = paging::decode_window(
                &scan.session,
                &data,
                read_offset,
                scan.offset,
                &viewport,
                eof,
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
            let Some((page, next)) = paging::search_text(
                &scan.session,
                &decoded,
                scan.offset,
                eof && consumed == data.len(),
                || context.is_cancelled(),
            ) else {
                return;
            };
            for (at, ch) in page.text.char_indices() {
                if context.is_cancelled() {
                    return;
                }
                let left = page
                    .anchors
                    .partition_point(|a| a.text_offset as usize <= at)
                    - 1;
                let right = page
                    .anchors
                    .partition_point(|a| (a.text_offset as usize) < at + ch.len_utf8());
                let start = page.anchors[left].source_offset;
                let end = page.anchors[right].source_offset;
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
            if next <= scan.offset {
                finish(context, scan.last);
                return;
            }
            scan.offset = next;
            if !page.has_after {
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
