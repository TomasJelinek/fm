//! Standalone ABI test plugin; never ship this as a user-facing viewer.
use midnight_plugin_sdk::{
    Context, Output, Request, Viewer,
    api::{abi, messages, viewer},
};

struct Fixture;
midnight_plugin_sdk::export_viewer!(Fixture);

impl Viewer for Fixture {
    const ID: &'static str = "midnight.test.viewer";
    const VERSION: &'static str = "0.1.0";

    fn create(_: &[u8]) -> Result<Self, abi::Status> {
        Ok(Self)
    }

    fn handle(&self, request: Request, context: Context) {
        if request.kind() == abi::REQUEST_CLOSE {
            context.complete().unwrap();
            return;
        }
        let viewport: messages::Viewport = match request.decode() {
            Ok(viewport) => viewport,
            Err(_) => {
                context
                    .fail(abi::INVALID, "Invalid fixture request")
                    .unwrap();
                return;
            }
        };
        match viewport.source_anchor {
            0 => read(context),
            1 => progress(context, 0),
            2 => context.complete().unwrap(),
            3 => panic!("intentional ABI conformance panic"),
            _ => context
                .fail(abi::UNSUPPORTED, "Unknown fixture scenario")
                .unwrap(),
        }
    }
}

fn read(context: Context) {
    context
        .read(
            messages::ReadAt {
                reader: 7,
                revision: 9,
                offset: 1u64 << 34,
                length: 4,
            },
            Box::new(|context, reply, content| {
                if reply.status != abi::OK {
                    context.fail(reply.status, "Fixture read failed").unwrap();
                    return;
                }
                let page = viewer::BinaryPage {
                    revision: reply.revision,
                    source: viewer::SourceRange {
                        offset: reply.offset,
                        length: reply.bytes.into(),
                    },
                    has_before: true,
                    has_after: !reply.eof,
                };
                let output = Output::binary(&page, content.as_ref().into()).unwrap();
                context
                    .send(
                        output,
                        Box::new(move |context| {
                            // Retain the input through another worker, beyond the read callback.
                            assert_eq!(content.as_ref(), &[0, 127, 128, 255]);
                            context.complete().unwrap();
                        }),
                    )
                    .unwrap();
            }),
        )
        .unwrap();
}

fn progress(context: Context, completed: u64) {
    if completed == 100 {
        context.complete().unwrap();
        return;
    }
    let output = Output::progress(&messages::Progress {
        completed,
        total: Some(100),
        phase: 0,
    })
    .unwrap();
    context
        .send(
            output,
            Box::new(move |context| progress(context, completed + 1)),
        )
        .unwrap();
}
