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
struct Permit(Arc<AtomicUsize>);
impl Drop for Permit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Release);
    }
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
            if let Some(session) = lock(&self.sessions).remove(&context.session()) {
                session.closed.store(true, Ordering::Release);
            }
            let _ = context.complete();
            return;
        }
        if context.is_cancelled() {
            return;
        }
        if self
            .work
            .try_update(Ordering::Acquire, Ordering::Relaxed, |n| {
                (n < 4).then_some(n + 1)
            })
            .is_err()
        {
            let _ = context.fail(abi::QUEUE_FULL, "Viewer work budget exhausted");
            return;
        }
        let permit = Arc::new(Permit(self.work.clone()));
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
                reader: open.reader,
                revision: open.revision,
                length: open.length,
                closed: AtomicBool::new(false),
                metadata: Mutex::new(Metadata {
                    encoding: Encoding::Utf8,
                    bom: 0,
                    ready: false,
                }),
                hints: Mutex::new(paging::Hints::new(&open.display_name, open.mode)),
            });
            {
                let mut sessions = lock(&self.sessions);
                if sessions.len() >= 16 || sessions.contains_key(&context.session()) {
                    let _ = context.fail(abi::QUEUE_FULL, "Viewer session unavailable");
                    return;
                }
                sessions.insert(context.session(), session.clone());
            }
            let next = session.clone();
            read(
                context,
                session,
                0,
                512,
                Box::new(move |context, data, _| {
                    let _permit = permit;
                    let (encoding, bom) = decode::encoding(&data);
                    *lock(&next.metadata) = Metadata {
                        encoding,
                        bom: bom as u64,
                        ready: true,
                    };
                    if let Some(prefix) =
                        decode::decode(&data[bom..], bom as u64, encoding, true, 512, 4, || {
                            context.is_cancelled()
                        })
                    {
                        lock(&next.hints).initialize(&prefix.text, bom as u64);
                    }
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
                let (offset, count, anchor) = paging::plan(&session, &viewport);
                let next = session.clone();
                read(
                    context,
                    session,
                    offset,
                    count,
                    Box::new(move |context, data, eof| {
                        let _permit = permit;
                        paging::viewport(context, next, data, offset, anchor, viewport, eof);
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
                search::start(context, session, query, Box::new(move || drop(permit)));
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
