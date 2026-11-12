use super::*;
use crate::{Context, Request, Viewer, host::tests::fixture};
use std::sync::atomic::Ordering;

struct TestViewer;

impl Viewer for TestViewer {
    const ID: &'static str = "test.viewer";
    const VERSION: &'static str = "0.1.0";
    fn create(_: &[u8]) -> Result<Self, abi::Status> {
        Ok(Self)
    }
    fn handle(&self, request: Request, context: Context) {
        assert_eq!(request.kind(), abi::REQUEST_VIEWPORT);
        let viewport: midnight_plugin_api::messages::Viewport = request.decode().unwrap();
        assert_eq!(viewport.source_anchor, 1u64 << 35);
        context.complete().unwrap();
    }
}

fn empty() -> abi::ByteSlice {
    abi::ByteSlice {
        data: std::ptr::null(),
        len: 0,
        reserved: 0,
    }
}

#[test]
fn entry_rejects_short_tables_without_writing_and_exports_valid_descriptor() {
    let mut untouched = [0xa5u64; 16];
    assert_eq!(
        unsafe { entry::<TestViewer>(untouched.as_mut_ptr().cast(), 16) },
        abi::INVALID
    );
    assert_eq!(untouched, [0xa5u64; 16]);
    let mut table = std::mem::MaybeUninit::<abi::PluginApi>::uninit();
    assert_eq!(
        unsafe { entry::<TestViewer>(table.as_mut_ptr(), size_of::<abi::PluginApi>() as u32) },
        abi::OK
    );
    let table = unsafe { table.assume_init() };
    assert!(
        table
            .validate(size_of::<abi::PluginApi>() as u32, abi::KIND_VIEWER)
            .is_ok()
    );
}

#[test]
fn ffi_lifecycle_preserves_callbacks_buffers_and_requires_quiescent_shutdown() {
    let (host, state, _) = fixture();
    state.accept_work.store(true, Ordering::Relaxed);
    let mut instance = 0;
    assert_eq!(
        unsafe { create::<TestViewer>(&host.api(), empty(), &mut instance) },
        abi::OK
    );
    assert_ne!(instance, 0);
    let payload = midnight_plugin_api::codec::encode(&midnight_plugin_api::messages::Viewport {
        source_anchor: 1u64 << 35,
        backwards: false,
        max_bytes: 256,
        max_lines: 32,
    })
    .unwrap();
    let request = abi::Request {
        id: 17,
        session: 1,
        generation: 3,
        kind: abi::REQUEST_VIEWPORT,
        flags: 0,
        payload: abi::ByteSlice {
            data: payload.as_ptr(),
            len: payload.len() as u32,
            reserved: 0,
        },
        content: abi::Buffer {
            bytes: empty(),
            id: 0,
        },
    };
    assert_eq!(unsafe { submit(instance, &request) }, abi::OK);
    assert_eq!(destroy(instance), abi::NOT_QUIESCENT);
    assert!(state.run_one());
    assert_eq!(begin_shutdown(instance), abi::OK);
    assert_eq!(destroy(instance), abi::NOT_QUIESCENT);
    let mut event = std::mem::MaybeUninit::<abi::Event>::uninit();
    let mut count = 999;
    assert_eq!(
        unsafe { drain(instance, 1, event.as_mut_ptr(), &mut count) },
        abi::OK
    );
    assert_eq!(count, 1);
    let event = unsafe { event.assume_init() };
    assert_eq!(event.kind, abi::EVENT_COMPLETED);
    assert_eq!(release_buffer(instance, event.payload.id), abi::OK);
    assert_eq!(
        release_buffer(instance, event.payload.id),
        abi::UNKNOWN_HANDLE
    );
    assert_eq!(destroy(instance), abi::OK);
    assert_eq!(begin_shutdown(instance), abi::UNKNOWN_HANDLE);
    assert_eq!(destroy(instance), abi::UNKNOWN_HANDLE);
}

#[test]
fn invalid_host_and_request_metadata_fail_before_plugin_work() {
    let (host, state, _) = fixture();
    let mut api = host.api();
    api.header.abi_major = 999;
    let mut instance = 44;
    assert_eq!(
        unsafe { create::<TestViewer>(&api, empty(), &mut instance) },
        abi::INCOMPATIBLE
    );
    assert_eq!(instance, 0);
    assert_eq!(
        unsafe { create::<TestViewer>(&host.api(), empty(), &mut instance) },
        abi::OK
    );
    assert_eq!(unsafe { submit(instance, std::ptr::null()) }, abi::INVALID);
    let request = abi::Request {
        id: 1,
        session: 1,
        generation: 1,
        kind: abi::REQUEST_CLOSE,
        flags: 0,
        payload: empty(),
        content: abi::Buffer {
            bytes: empty(),
            id: 88,
        },
    };
    assert_eq!(unsafe { submit(instance, &request) }, abi::INVALID);
    assert!(!state.run_one());
    assert_eq!(begin_shutdown(instance), abi::OK);
    assert_eq!(destroy(instance), abi::OK);
}
