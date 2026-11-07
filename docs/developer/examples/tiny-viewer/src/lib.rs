//! Teaching example, not a replacement for the production Basic viewer.
//! One session per instance, forward UTF-8 pages only; no search or normalization.
use midnight_plugin_sdk::{
    Context, Output, Request, Viewer,
    api::{abi, messages, viewer, wire::WireError},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

struct Session {
    open: messages::Open,
    id: u64,
    generation: u64,
    alive: AtomicBool,
}

struct TinyViewer(Mutex<Option<Arc<Session>>>);
midnight_plugin_sdk::export_viewer!(TinyViewer);

impl Viewer for TinyViewer {
    const ID: &'static str = "example.viewer.tiny";
    const VERSION: &'static str = "0.1.0";

    fn create(configuration: &[u8]) -> Result<Self, abi::Status> {
        if !configuration.is_empty() {
            return Err(abi::UNSUPPORTED);
        }
        Ok(Self(Mutex::new(None)))
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
            if open.mode != viewer::MODE_TEXT {
                fail(&context, abi::UNSUPPORTED, "Example supports Text only");
                return;
            }
            let output = Output::opened(&messages::Opened {
                mode: open.mode,
                length: open.length,
                initial_anchor: 0,
                estimated_lines: None,
            });
            {
                let mut slot = self.0.lock().unwrap();
                if slot.is_some() {
                    fail(
                        &context,
                        abi::UNSUPPORTED,
                        "One session per example instance",
                    );
                    return;
                }
                *slot = Some(Arc::new(Session {
                    open,
                    id: context.session(),
                    generation: context.generation(),
                    alive: AtomicBool::new(true),
                }));
            }
            send_and_finish(&context, output);
            return;
        }
        let session = self.0.lock().unwrap().clone();
        let Some(session) = session.filter(|session| {
            session.id == context.session() && session.generation == context.generation()
        }) else {
            fail(&context, abi::UNKNOWN_HANDLE, "Unknown or stale session");
            return;
        };
        match request.kind() {
            abi::REQUEST_VIEWPORT => match request.decode::<messages::Viewport>() {
                Ok(viewport) => read_page(context, session, viewport),
                Err(_) => fail(&context, abi::INVALID, "Invalid viewport"),
            },
            abi::REQUEST_CLOSE => {
                session.alive.store(false, Ordering::Release);
                self.0.lock().unwrap().take();
                let _ = context.complete();
            }
            _ => fail(
                &context,
                abi::UNSUPPORTED,
                "Example has no search implementation",
            ),
        }
    }
}

fn read_page(context: Context, session: Arc<Session>, viewport: messages::Viewport) {
    if viewport.backwards {
        fail(
            &context,
            abi::UNSUPPORTED,
            "Example supports forward pages only",
        );
        return;
    }
    if session
        .open
        .length
        .is_some_and(|length| viewport.source_anchor > length)
    {
        fail(&context, abi::INVALID, "Anchor is beyond the source");
        return;
    }
    let read = messages::ReadAt {
        reader: session.open.reader,
        revision: session.open.revision,
        offset: viewport.source_anchor,
        length: viewport.max_bytes.min(4096),
    };
    let status = context.read(
        read,
        Box::new(move |context, reply, content| {
            if context.is_cancelled() {
                return;
            }
            if !session.alive.load(Ordering::Acquire) {
                fail(&context, abi::UNKNOWN_HANDLE, "Session was closed");
                return;
            }
            if reply.status != abi::OK {
                fail(&context, reply.status, "Host read failed");
                return;
            }
            let bytes = content.as_ref();
            // A partial final scalar can be reread on the next page. An invalid
            // start/interior or malformed final source is unsupported here.
            let text = match std::str::from_utf8(bytes) {
                Ok(text) => text,
                Err(error) if error.error_len().is_none() && !reply.eof => {
                    std::str::from_utf8(&bytes[..error.valid_up_to()]).unwrap()
                }
                Err(_) => {
                    fail(
                        &context,
                        abi::UNSUPPORTED,
                        "Expected UTF-8 at a scalar boundary",
                    );
                    return;
                }
            };
            if text.is_empty() && !reply.eof {
                fail(
                    &context,
                    abi::UNSUPPORTED,
                    "Read made no complete-character progress",
                );
                return;
            }
            if text
                .chars()
                .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
            {
                fail(
                    &context,
                    abi::UNSUPPORTED,
                    "Example does not escape control characters",
                );
                return;
            }
            let end = text
                .match_indices('\n')
                .nth(viewport.max_lines as usize - 1)
                .map_or(text.len(), |(index, _)| index + 1);
            let text = &text[..end];
            let mut anchors: Vec<_> = text
                .char_indices()
                .map(|(index, _)| viewer::Anchor {
                    text_offset: index as u32,
                    source_offset: reply.offset + index as u64,
                })
                .collect();
            anchors.push(viewer::Anchor {
                text_offset: text.len() as u32,
                source_offset: reply.offset + text.len() as u64,
            });
            let has_after = !reply.eof || text.len() < bytes.len();
            let page = viewer::TextPage {
                revision: reply.revision,
                source: viewer::SourceRange {
                    offset: reply.offset,
                    length: text.len() as u64,
                },
                text: text.to_owned(),
                spans: Vec::new(),
                anchors,
                has_before: reply.offset != 0,
                has_after,
                // Conservatively mark a seek fragment when preceding text is unknown.
                partial_line: reply.offset != 0 || (has_after && !text.ends_with('\n')),
            };
            send_and_finish(&context, Output::text(&page));
            // Content is released when this callback's last lease is dropped.
        }),
    );
    if let Err(status) = status {
        fail(&context, status, "Read admission failed");
    }
}

fn send_and_finish(context: &Context, output: Result<Output, WireError>) {
    let Ok(output) = output else {
        fail(
            context,
            abi::INVALID,
            "Output exceeds the model or wire limits",
        );
        return;
    };
    if let Err(error) = context.send(
        output,
        Box::new(|context| {
            let _ = context.complete();
        }),
    ) {
        fail(context, error.status, "Output admission failed");
        // Rejected output and continuation remain ours and are dropped here.
    }
}

fn fail(context: &Context, status: abi::Status, message: &str) {
    // A racing cancellation may already have produced this request's terminal.
    let _ = context.fail(status, message);
}
