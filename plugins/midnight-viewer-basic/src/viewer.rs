use crate::{
    decode::{self, Encoding},
    io,
    search::Matcher,
};
use midnight_plugin_sdk::{
    Context, Output, Request, Viewer,
    api::{abi, messages, viewer},
};
use std::{
    collections::{BTreeSet, HashMap},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct BasicViewer {
    sessions: Mutex<HashMap<u64, Arc<Session>>>,
}
pub(crate) struct Session {
    pub open: messages::Open,
    generation: u64,
    alive: AtomicBool,
    encoding: Mutex<Option<(Encoding, u64)>>,
    checkpoints: Mutex<BTreeSet<u64>>,
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
    fn record(&self, page: &viewer::TextPage) {
        let mut checkpoints = self.checkpoints.lock().unwrap();
        checkpoints.insert(page.source.offset);
        for a in &page.anchors {
            if a.text_offset > 0 && page.text.as_bytes()[a.text_offset as usize - 1] == b'\n' {
                checkpoints.insert(a.source_offset);
            }
        }
        while checkpoints.len() > 1024 {
            let remove: Vec<_> = checkpoints.iter().copied().skip(1).step_by(2).collect();
            for offset in remove {
                checkpoints.remove(&offset);
            }
        }
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
                checkpoints: Mutex::new(BTreeSet::new()),
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
                Ok(query) => {
                    let mut offset = if query.backwards {
                        bom
                    } else {
                        query.source_anchor.saturating_sub(4).max(bom)
                    };
                    if encoding != Encoding::Utf8 {
                        offset -= (offset - bom) % 2;
                    }
                    let state = SearchState {
                        matcher: Matcher::new(&query.query, query.case_sensitive),
                        query,
                        session,
                        encoding,
                        offset,
                        last: None,
                    };
                    search_next(context, state);
                }
                Err(_) => fail(&context, abi::INVALID, "Invalid search request"),
            },
            _ => fail(&context, abi::UNSUPPORTED, "Unsupported request"),
        }
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
    let mut offset = if viewport.backwards {
        anchor.saturating_sub(count as u64)
    } else {
        anchor
    };
    offset = offset.max(bom);
    if !binary {
        offset = offset.saturating_sub(4).max(bom);
    }
    if encoding != Encoding::Utf8 {
        offset -= (offset - bom) % 2;
    }
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
            let mut page = decode::page(
                &bytes,
                encoding,
                offset,
                eof,
                viewer::MAX_PAGE_BYTES,
                usize::MAX,
            );
            page.revision = session.open.revision;
            let page = crate::window::select(page, bom, anchor, &viewport);
            if page.source.length == 0
                && !bytes.is_empty()
                && !(viewport.backwards && anchor == bom)
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
            session.record(&page);
            send_finish(&context, Output::text(&page));
        }),
    );
}

struct SearchState {
    matcher: Matcher,
    query: messages::Search,
    session: Arc<Session>,
    encoding: Encoding,
    offset: u64,
    last: Option<viewer::SourceRange>,
}
fn search_next(context: Context, mut state: SearchState) {
    if !state.session.live(&context) {
        return;
    }
    if state.query.backwards && state.offset >= state.query.source_anchor {
        finish_search(&context, state);
        return;
    }
    let count = if state.query.backwards {
        (state.query.source_anchor - state.offset).min(decode::READ_BYTES as u64) as usize
    } else {
        decode::READ_BYTES
    };
    io::read(
        context,
        state.session.clone(),
        state.offset,
        count,
        Box::new(move |context, bytes, eof| {
            let boundary = eof
                || (state.query.backwards
                    && state.offset + bytes.len() as u64 >= state.query.source_anchor);
            let page = decode::page(
                &bytes,
                state.encoding,
                state.offset,
                eof,
                viewer::MAX_PAGE_BYTES,
                usize::MAX,
            );
            let Ok(matches) = state.matcher.feed(&page, || {
                context.is_cancelled() || !state.session.alive.load(Ordering::Acquire)
            }) else {
                return;
            };
            if state.query.backwards {
                if let Some(found) = matches
                    .into_iter()
                    .rfind(|m| m.offset + m.length <= state.query.source_anchor)
                {
                    state.last = Some(found);
                }
            } else if let Some(found) = matches
                .into_iter()
                .find(|m| m.offset >= state.query.source_anchor)
            {
                state.last = Some(found);
                finish_search(&context, state);
                return;
            }
            if page.source.length == 0 {
                if boundary {
                    finish_search(&context, state);
                } else {
                    fail(&context, abi::INVALID, "Content made no decoding progress");
                }
                return;
            }
            state.session.record(&page);
            state.offset = page.source.end().unwrap();
            if boundary && page.source.length == bytes.len() as u64 {
                finish_search(&context, state);
            } else {
                search_next(context, state);
            }
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
