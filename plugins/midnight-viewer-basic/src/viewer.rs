use crate::{
    decode::{self, Encoding},
    io,
    search::{Matcher, Want},
};
use midnight_plugin_sdk::{
    Context, Output, Request, Viewer,
    api::{abi, messages, viewer},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

/// Bytes per search read: the protocol maximum. Search never encodes pages, so the
/// viewport unit cap does not apply and each byte is requested about once.
const SEARCH_READ_BYTES: usize = viewer::MAX_PAGE_BYTES;
/// Backward search window; aligned to UTF-16 it stays one protocol read.
const BACKWARD_WINDOW: u64 = SEARCH_READ_BYTES as u64 - 2;
// A window must exceed its overlap (4-byte resynchronization plus the span of a
// 4096-byte query whose characters fold to at most three characters each).
const _: () = assert!(BACKWARD_WINDOW > 4 * 3 * 4096 + 4);

pub struct BasicViewer {
    sessions: Mutex<HashMap<u64, Arc<Session>>>,
}
pub(crate) struct Session {
    pub open: messages::Open,
    generation: u64,
    alive: AtomicBool,
    encoding: Mutex<Option<(Encoding, u64)>>,
}
impl Session {
    pub fn live(&self, context: &Context) -> bool {
        if context.is_cancelled() {
            return false;
        }
        if !self.alive.load(Ordering::Acquire) {
            fail(
                context,
                abi::UNKNOWN_HANDLE,
                "Viewer session was closed or replaced",
            );
            return false;
        }
        true
    }
    fn encoding(&self) -> Option<(Encoding, u64)> {
        *self.encoding.lock().unwrap()
    }
}
impl Viewer for BasicViewer {
    const ID: &'static str = "midnight.viewer.basic";
    const VERSION: &'static str = "0.1.0";
    fn create(configuration: &[u8]) -> Result<Self, abi::Status> {
        if !configuration.is_empty() {
            return Err(abi::UNSUPPORTED);
        }
        Ok(Self {
            sessions: Mutex::new(HashMap::new()),
        })
    }
    fn handle(&self, request: Request, context: Context) {
        if context.is_cancelled() {
            return;
        }
        if request.kind() == abi::REQUEST_OPEN {
            let Ok(open) = request.decode::<messages::Open>() else {
                fail(&context, abi::INVALID, "Invalid open request");
                return;
            };
            if !matches!(open.mode, viewer::MODE_TEXT | viewer::MODE_BINARY) {
                fail(
                    &context,
                    abi::UNSUPPORTED,
                    "Basic viewer supports Text and Binary",
                );
                return;
            }
            let session = Arc::new(Session {
                open,
                generation: context.generation(),
                alive: AtomicBool::new(true),
                encoding: Mutex::new(None),
            });
            {
                let mut sessions = self.sessions.lock().unwrap();
                if let Some(old) = sessions.get(&context.session()) {
                    if old.generation >= session.generation {
                        fail(&context, abi::INVALID, "Open generation must increase");
                        return;
                    }
                    old.alive.store(false, Ordering::Release);
                } else if sessions.len() >= 32 {
                    fail(
                        &context,
                        abi::QUEUE_FULL,
                        "Basic viewer session budget exhausted",
                    );
                    return;
                }
                sessions.insert(context.session(), session.clone());
            }
            io::read(
                context,
                session.clone(),
                0,
                4,
                Box::new(move |context, bytes, _| {
                    let (encoding, bom) = if session.open.mode == viewer::MODE_BINARY {
                        (Encoding::Utf8, 0)
                    } else {
                        decode::detect(&bytes)
                    };
                    *session.encoding.lock().unwrap() = Some((encoding, bom));
                    send_finish(
                        &context,
                        Output::opened(&messages::Opened {
                            mode: session.open.mode,
                            length: session.open.length,
                            initial_anchor: bom,
                            estimated_lines: None,
                        }),
                    );
                }),
            );
            return;
        }
        if request.kind() == abi::REQUEST_CLOSE {
            let mut sessions = self.sessions.lock().unwrap();
            if sessions
                .get(&context.session())
                .is_some_and(|s| s.generation == context.generation())
            {
                let session = sessions.remove(&context.session()).unwrap();
                session.alive.store(false, Ordering::Release);
            }
            let _ = context.complete();
            return;
        }
        let session = self
            .sessions
            .lock()
            .unwrap()
            .get(&context.session())
            .cloned();
        let Some(session) = session.filter(|s| s.generation == context.generation()) else {
            fail(
                &context,
                abi::UNKNOWN_HANDLE,
                "Unknown viewer session generation",
            );
            return;
        };
        let Some((encoding, bom)) = session.encoding() else {
            fail(
                &context,
                abi::INVALID,
                "Wait for Opened before requesting content",
            );
            return;
        };
        match request.kind() {
            abi::REQUEST_VIEWPORT => match request.decode::<messages::Viewport>() {
                Ok(viewport) => viewport_page(context, session, encoding, bom, viewport),
                Err(_) => fail(&context, abi::INVALID, "Invalid viewport request"),
            },
            abi::REQUEST_SEARCH => match request.decode::<messages::Search>() {
                Ok(query) => start_search(context, session, encoding, bom, query),
                Err(_) => fail(&context, abi::INVALID, "Invalid search request"),
            },
            _ => fail(&context, abi::UNSUPPORTED, "Unsupported request"),
        }
    }
}

/// Moves `offset` down to a UTF-16 code unit boundary relative to the BOM.
fn align(offset: u64, encoding: Encoding, bom: u64) -> u64 {
    if encoding == Encoding::Utf8 {
        offset
    } else {
        offset - (offset - bom) % 2
    }
}

fn viewport_page(
    context: Context,
    session: Arc<Session>,
    encoding: Encoding,
    bom: u64,
    viewport: messages::Viewport,
) {
    let anchor = session
        .open
        .length
        .map_or(viewport.source_anchor, |len| {
            viewport.source_anchor.min(len)
        })
        .max(bom);
    let binary = session.open.mode == viewer::MODE_BINARY;
    let count = if binary {
        (viewport.max_bytes as usize)
            .min(decode::READ_BYTES)
            .min(viewport.max_lines as usize * 16)
    } else {
        (viewport.max_bytes as usize).min(decode::READ_BYTES - 16)
    };
    // A backward page starts no earlier than `limit`; text reads begin four bytes
    // before it so the decoder resynchronizes on a real scalar boundary first.
    let limit = if viewport.backwards {
        anchor.saturating_sub(count as u64).max(bom)
    } else {
        anchor
    };
    let mut offset = limit;
    if !binary {
        offset = offset.saturating_sub(4).max(bom);
    }
    offset = align(offset, encoding, bom);
    // A read from the BOM needs no resynchronization, so its margin is content.
    let limit = if offset == bom { bom } else { limit };
    let read_bytes = if viewport.backwards {
        (anchor - offset) as usize
    } else {
        count + (anchor - offset) as usize
    };
    let read_bytes = read_bytes + if binary { 0 } else { 4 };
    io::read(
        context,
        session.clone(),
        offset,
        read_bytes,
        Box::new(move |context, bytes, eof| {
            if binary {
                let page = viewer::BinaryPage {
                    revision: session.open.revision,
                    source: viewer::SourceRange {
                        offset,
                        length: bytes.len() as u64,
                    },
                    has_before: offset > 0,
                    has_after: !eof,
                };
                send_finish(&context, Output::binary(&page, bytes.into_boxed_slice()));
                return;
            }
            let mut page = decode::page(&bytes, encoding, offset, eof);
            page.revision = session.open.revision;
            let page = crate::window::select(page, bom, anchor, limit, &viewport);
            // A backward page ending at the start of content is legitimately empty.
            if page.source.length == 0
                && !bytes.is_empty()
                && !(viewport.backwards && page.source.offset == bom)
                && session.open.length.is_none_or(|len| anchor < len)
            {
                fail(
                    &context,
                    abi::INVALID,
                    "Viewport budget cannot contain one decoded unit",
                );
                return;
            }
            if !session.live(&context) {
                return;
            }
            send_finish(&context, Output::text(&page));
        }),
    );
}

struct SearchState {
    matcher: Matcher,
    query: messages::Search,
    session: Arc<Session>,
    encoding: Encoding,
    bom: u64,
    /// The next read of a linear scan, or the end of the next backward window.
    offset: u64,
    last: Option<viewer::SourceRange>,
}

fn start_search(
    context: Context,
    session: Arc<Session>,
    encoding: Encoding,
    bom: u64,
    query: messages::Search,
) {
    let mut state = SearchState {
        matcher: Matcher::new(&query.query, query.case_sensitive),
        session,
        encoding,
        bom,
        offset: bom,
        last: None,
        query,
    };
    if !state.query.backwards {
        // Start four bytes early so the decoder resynchronizes before the anchor.
        let offset = state.query.source_anchor.saturating_sub(4).max(bom);
        state.offset = align(offset, encoding, bom);
        scan_linear(context, state);
    } else if let Some(length) = state.session.open.length {
        state.offset = state.query.source_anchor.min(length).max(bom);
        scan_backward(context, state);
    } else {
        // Without a known length the content end cannot be found backwards.
        scan_linear(context, state);
    }
}

fn cancelled(context: &Context, session: &Session) -> bool {
    context.is_cancelled() || !session.alive.load(Ordering::Acquire)
}

/// Scans forward from `state.offset`: forward searches stop at the first match at or
/// after the anchor, backward searches without a known length keep the last match
/// ending at or before it.
fn scan_linear(context: Context, mut state: SearchState) {
    if !state.session.live(&context) {
        return;
    }
    let anchor = state.query.source_anchor;
    let backwards = state.query.backwards;
    if backwards && state.offset >= anchor {
        finish_search(&context, state);
        return;
    }
    let count = if backwards {
        (anchor - state.offset).min(SEARCH_READ_BYTES as u64) as usize
    } else {
        SEARCH_READ_BYTES
    };
    io::read(
        context,
        state.session.clone(),
        state.offset,
        count,
        Box::new(move |context, bytes, eof| {
            let want = if backwards {
                Want::LastUntil(anchor)
            } else {
                Want::FirstFrom(anchor)
            };
            let session = state.session.clone();
            let Ok(scan) =
                state
                    .matcher
                    .scan(&bytes, state.encoding, state.offset, eof, want, || {
                        cancelled(&context, &session)
                    })
            else {
                return;
            };
            if scan.found.is_some() {
                state.last = scan.found;
                if !backwards {
                    finish_search(&context, state);
                    return;
                }
            }
            let boundary = eof || (backwards && state.offset + bytes.len() as u64 >= anchor);
            if scan.consumed == 0 {
                if boundary {
                    finish_search(&context, state);
                } else {
                    fail(&context, abi::INVALID, "Content made no decoding progress");
                }
                return;
            }
            state.offset += scan.consumed as u64;
            if boundary && scan.consumed == bytes.len() {
                finish_search(&context, state);
            } else {
                scan_linear(context, state);
            }
        }),
    );
}

/// Scans fixed windows backwards from the anchor and stops at the first window
/// that proves its last match is the nearest one, so repeated backward searches
/// cost about one window instead of the whole preceding content.
fn scan_backward(context: Context, mut state: SearchState) {
    if !state.session.live(&context) {
        return;
    }
    let end = state.offset;
    let start = align(
        end.saturating_sub(BACKWARD_WINDOW).max(state.bom),
        state.encoding,
        state.bom,
    );
    if start == end {
        finish_search(&context, state);
        return;
    }
    io::read(
        context,
        state.session.clone(),
        start,
        (end - start) as usize,
        Box::new(move |context, bytes, eof| {
            let session = state.session.clone();
            state.matcher.reset();
            let Ok(scan) = state.matcher.scan(
                &bytes,
                state.encoding,
                start,
                eof,
                Want::LastUntil(end),
                || cancelled(&context, &session),
            ) else {
                return;
            };
            // Decoding resynchronizes on a real boundary within four bytes of the
            // window start, so every match ending at or after `proven` was decoded
            // exactly like a scan from the BOM; earlier matches may be incomplete.
            let proven = start.saturating_add(state.matcher.span_bound() + 4);
            let complete = scan.found.as_ref().is_some_and(|found| {
                found
                    .end()
                    .is_ok_and(|found_end| start == state.bom || found_end >= proven)
            });
            if complete || start == state.bom {
                state.last = scan.found;
                finish_search(&context, state);
                return;
            }
            if proven >= end {
                fail(
                    &context,
                    abi::INVALID,
                    "Search window cannot hold the query",
                );
                return;
            }
            // No match ends in [proven, end]; continue with matches ending earlier.
            state.offset = proven;
            scan_backward(context, state);
        }),
    );
}

fn finish_search(context: &Context, state: SearchState) {
    if !state.session.live(context) {
        return;
    }
    if let Some(source) = state.last {
        send_finish(
            context,
            Output::search_match(&messages::SearchMatch {
                source,
                preview: state.query.query,
            }),
        );
    } else {
        let _ = context.complete();
    }
}
pub(crate) fn fail(context: &Context, code: u32, message: &str) {
    let _ = context.fail(code, message);
}
fn send_finish(
    context: &Context,
    output: Result<Output, midnight_plugin_sdk::api::wire::WireError>,
) {
    match output {
        Ok(output) => {
            if let Err(error) = context.send(
                output,
                Box::new(|context| {
                    let _ = context.complete();
                }),
            ) {
                fail(context, error.status, "Unable to queue viewer output");
            }
        }
        Err(_) => fail(
            context,
            abi::INVALID,
            "Viewer output exceeds protocol budgets",
        ),
    }
}
