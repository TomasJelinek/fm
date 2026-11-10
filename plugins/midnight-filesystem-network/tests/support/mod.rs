#![allow(dead_code)]
use midnight_filesystem_network::{NetworkFilesystem, adapter::Adapter, api::*};
use midnight_filesystem_plugin_sdk::entry;

pub struct Client {
    table: abi::Api,
    instance: u64,
}
impl Client {
    pub fn new<A: Adapter>() -> Self {
        let mut table = abi::Api::default();
        // SAFETY: live aligned table with its exact writable capacity.
        assert_eq!(
            unsafe {
                entry::<NetworkFilesystem<A>>(
                    &mut table,
                    size_of::<abi::Api>() as u32,
                    "network.test",
                    "0.1.0",
                    abi::CAP_BROWSE
                        | abi::CAP_STAT
                        | abi::CAP_VOLUMES
                        | abi::CAP_ACTIVATE
                        | abi::CAP_REFRESH
                        | abi::CAP_RESOLVE_TEXT,
                )
            },
            abi::OK
        );
        table.validate(size_of::<abi::Api>() as u32).unwrap();
        let mut instance = 0;
        // SAFETY: config and writable output remain live throughout create.
        assert_eq!(
            unsafe { table.create.unwrap()(&abi::Config::default(), &mut instance) },
            abi::OK
        );
        Self { table, instance }
    }
    pub fn call(&self, request: Request) -> Response {
        self.cancelled(request, false)
    }
    pub fn cancelled(&self, request: Request, cancelled: bool) -> Response {
        unsafe extern "C" fn callback(value: u64) -> u32 {
            u32::from(value != 0)
        }
        let bytes = encode_request(&request, DEFAULT_REQUEST_BYTES as usize).unwrap();
        let input = abi::Call {
            struct_size: size_of::<abi::Call>() as u32,
            operation: request.operation() as u32,
            payload: abi::Slice {
                data: bytes.as_ptr(),
                len: bytes.len() as u64,
            },
            cancel: abi::Cancellation {
                context: u64::from(cancelled),
                cancelled: Some(callback),
            },
        };
        let mut output = abi::Buffer::default();
        // SAFETY: all borrowed storage and callback remain live for this synchronous call.
        assert_eq!(
            unsafe { self.table.call.unwrap()(self.instance, &input, &mut output) },
            abi::OK
        );
        output.validate(DEFAULT_REPLY_BYTES).unwrap();
        // SAFETY: immutable response belongs to this instance until the matching release.
        let bytes =
            unsafe { std::slice::from_raw_parts(output.bytes.data, output.bytes.len as usize) };
        let response = decode_response(bytes, DEFAULT_REPLY_BYTES as usize);
        // SAFETY: buffer was returned by this still-live instance and is released once.
        assert_eq!(
            unsafe { self.table.release_buffer.unwrap()(self.instance, output.id) },
            abi::OK
        );
        response.unwrap()
    }
    pub fn root(&self) -> (u64, Node) {
        let Response::Volumes(volumes) = self.call(Request::Volumes) else {
            panic!("missing network root")
        };
        assert_eq!(volumes.len(), 1);
        let Response::Resolved { mount, node } = self.call(Request::Resolve {
            address: volumes[0].root.clone(),
        }) else {
            panic!("failed to resolve network root")
        };
        (mount, node)
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: this client has no live calls or reply buffers when it drops.
        assert_eq!(
            unsafe { self.table.destroy.unwrap()(self.instance) },
            abi::OK
        );
    }
}
