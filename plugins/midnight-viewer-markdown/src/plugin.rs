use crate::{
    decode::{self, Encoding},
    paging, search,
};
use midnight_plugin_sdk::{
    Context, Output, Request, Viewer,
    api::{abi, messages, wire},
};
use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

/// Requests of one plugin instance that may decode/parse/match at once.
/// Waiting for content reads holds no slot; the SDK bounds pending reads.
const WORK_SLOTS: usize = 4;
const MAX_SESSIONS: usize = 16;

pub struct Plugin {
    sessions: Mutex<HashMap<u64, Arc<Session>>>,
    work: Arc<AtomicUsize>,
}
pub struct Metadata {
    pub encoding: Encoding,
    pub bom: u64,
    pub ready: bool,
}
pub struct Session {
    /// The envelope generation of the Open that created this session; other
    /// generations of the same session ID are stale.
    pub generation: u64,
    pub reader: u64,
    pub revision: u64,
    pub length: Option<u64>,
    pub closed: AtomicBool,
    pub metadata: Mutex<Metadata>,
    pub hints: Mutex<paging::Hints>,
}
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// One of the instance's CPU work slots, released on drop.
pub(crate) struct Permit(Arc<AtomicUsize>);
impl Permit {
    pub fn try_acquire(work: &Arc<AtomicUsize>) -> Option<Self> {
        work.try_update(Ordering::Acquire, Ordering::Relaxed, |n| {
            (n < WORK_SLOTS).then_some(n + 1)
        })
        .ok()?;
        Some(Self(work.clone()))
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
}

/// Run a read continuation's CPU phase under a work slot.
fn with_permit(
    work: Arc<AtomicUsize>,
    done: impl FnOnce(Context, Vec<u8>, bool) + Send + 'static,
) -> ReadDone {
    Box::new(move |context, data, eof| {
        let Some(_permit) = Permit::try_acquire(&work) else {
            let _ = context.fail(abi::QUEUE_FULL, "Viewer work budget exhausted");
            return;
        };
        done(context, data, eof);
    })
}

impl Viewer for Plugin {
    const ID: &'static str = "midnight.viewer.markdown";
    const VERSION: &'static str = "0.1.0";
    fn create(_: &[u8]) -> Result<Self, abi::Status> {
        Ok(Self {
            sessions: Mutex::new(HashMap::new()),
            work: Arc::new(AtomicUsize::new(0)),
        })
    }
    fn handle(&self, request: Request, context: Context) {
        if request.kind() == abi::REQUEST_CLOSE {
            let mut sessions = lock(&self.sessions);
            // A stale generation's Close must not close the live session.
            if sessions
                .get(&context.session())
                .is_some_and(|session| session.generation == context.generation())
                && let Some(session) = sessions.remove(&context.session())
            {
                session.closed.store(true, Ordering::Release);
            }
            drop(sessions);
            let _ = context.complete();
            return;
        }
        if context.is_cancelled() {
            return;
        }
        // The synchronous part (decoding requests, cache hits) is CPU work too;
        // the slot is released before any content read is awaited.
        let Some(permit) = Permit::try_acquire(&self.work) else {
            let _ = context.fail(abi::QUEUE_FULL, "Viewer work budget exhausted");
            return;
        };
        if request.kind() == abi::REQUEST_OPEN {
            let open: messages::Open = match request.decode() {
                Ok(open) => open,
                Err(_) => {
                    let _ = context.fail(abi::INVALID, "Invalid Open");
                    return;
                }
            };
            if open.mode != midnight_plugin_sdk::api::viewer::MODE_DOCUMENT
                && open.mode != midnight_plugin_sdk::api::viewer::MODE_TEXT
            {
                let _ = context.fail(abi::UNSUPPORTED, "Unsupported viewer mode");
                return;
            }
            if open.display_name.len() > 4096 {
                let _ = context.fail(abi::INVALID, "Display name exceeds plugin budget");
                return;
            }
            let session = Arc::new(Session {
                generation: context.generation(),
                reader: open.reader,
                revision: open.revision,
                length: open.length,
                closed: AtomicBool::new(false),
                metadata: Mutex::new(Metadata {
                    encoding: Encoding::Utf8,
                    bom: 0,
                    ready: false,
                }),
                hints: Mutex::new(paging::Hints::new(open.mode)),
            });
            {
                let mut sessions = lock(&self.sessions);
                if sessions.len() >= MAX_SESSIONS || sessions.contains_key(&context.session()) {
                    let _ = context.fail(abi::QUEUE_FULL, "Viewer session unavailable");
                    return;
                }
                sessions.insert(context.session(), session.clone());
            }
            let next = session.clone();
            drop(permit);
            read(
                context,
                session,
                0,
                4,
                with_permit(self.work.clone(), move |context, data, _| {
                    let (encoding, bom) = decode::encoding(&data);
                    *lock(&next.metadata) = Metadata {
                        encoding,
                        bom: bom as u64,
                        ready: true,
                    };
                    lock(&next.hints).initialize(bom as u64);
                    send(
                        context,
                        Output::opened(&messages::Opened {
                            mode: open.mode,
                            length: open.length,
                            initial_anchor: bom as u64,
                            estimated_lines: None,
                        }),
                    );
                }),
            );
            return;
        }
        let Some(session) = lock(&self.sessions).get(&context.session()).cloned() else {
            let _ = context.fail(abi::UNKNOWN_HANDLE, "Viewer session is closed");
            return;
        };
        if session.generation != context.generation() {
            let _ = context.fail(abi::UNKNOWN_HANDLE, "Viewer session generation is stale");
            return;
        }
        if !lock(&session.metadata).ready {
            let _ = context.fail(abi::INVALID, "Viewer is not opened yet");
            return;
        }
        match request.kind() {
            abi::REQUEST_VIEWPORT => {
                let viewport: messages::Viewport = match request.decode() {
                    Ok(value) => value,
                    Err(_) => {
                        let _ = context.fail(abi::INVALID, "Invalid Viewport");
                        return;
                    }
                };
                if let Some((page, plain)) = paging::cached(&session, &viewport) {
                    paging::send_page(context, &page, plain);
                    return;
                }
                let plan = paging::plan(&session, &viewport);
                let next = session.clone();
                drop(permit);
                read(
                    context,
                    session,
                    plan.offset,
                    plan.count,
                    with_permit(self.work.clone(), move |context, data, eof| {
                        paging::viewport(context, next, data, plan, viewport, eof);
                    }),
                );
            }
            abi::REQUEST_SEARCH => {
                let query: messages::Search = match request.decode() {
                    Ok(value) => value,
                    Err(_) => {
                        let _ = context.fail(abi::INVALID, "Invalid Search");
                        return;
                    }
                };
                drop(permit);
                search::start(context, session, query, self.work.clone());
            }
            _ => {
                let _ = context.fail(abi::UNSUPPORTED, "Unsupported request");
            }
        }
    }
}

pub fn send(context: Context, output: Result<Output, wire::WireError>) {
    match output {
        Ok(output) => {
            if let Err(error) = context.send(
                output,
                Box::new(|context| {
                    let _ = context.complete();
                }),
            ) {
                let _ = context.fail(error.status, "Output queue rejected page");
            }
        }
        Err(_) => {
            let _ = context.fail(abi::INVALID, "Page exceeded semantic model budget");
        }
    }
}

type ReadDone = Box<dyn FnOnce(Context, Vec<u8>, bool) + Send>;
pub fn read(context: Context, session: Arc<Session>, offset: u64, length: u32, done: ReadDone) {
    if length == 0 || length > 65536 || offset.checked_add(length as u64).is_none() {
        let _ = context.fail(abi::INVALID, "Read range exceeds plugin budget");
        return;
    }
    read_more(context, session, offset, length, Vec::new(), done);
}
fn read_more(
    context: Context,
    session: Arc<Session>,
    offset: u64,
    length: u32,
    mut data: Vec<u8>,
    done: ReadDone,
) {
    if context.is_cancelled() || session.closed.load(Ordering::Acquire) {
        return;
    }
    let request = messages::ReadAt {
        reader: session.reader,
        revision: session.revision,
        offset: offset + data.len() as u64,
        length: length - data.len() as u32,
    };
    let result = context.read(
        request,
        Box::new(move |context, reply, content| {
            if context.is_cancelled() || session.closed.load(Ordering::Acquire) {
                return;
            }
            if reply.status != abi::OK {
                let _ = context.fail(reply.status, "Content read failed");
                return;
            }
            if reply.bytes == 0 && !reply.eof {
                let _ = context.fail(abi::INVALID, "Content read made no progress");
                return;
            }
            data.extend_from_slice(content.as_ref());
            drop(content);
            if reply.eof || data.len() >= length as usize {
                done(context, data, reply.eof);
            } else {
                read_more(context, session, offset, length, data, done);
            }
        }),
    );
    if let Err(status) = result {
        let _ = context.fail(status, "Content service rejected read");
    }
}
