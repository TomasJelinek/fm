use super::*;
use crate::rename_test_support::{Client, Temp};
use midnight_filesystem_plugin_api::abi::*;
use midnight_filesystem_plugin_sdk::{Filesystem, Reply, native::NativeAdapter};
use std::{ffi::CString, io::Read, os::unix::ffi::OsStrExt};

struct Race;
impl Filesystem for Race {
    fn create(_: Config) -> Result<Self, Failure> {
        Ok(Self)
    }
    fn handle(&self, request: Request, context: &CallContext<'_>) -> Result<Reply, Failure> {
        let Request::Resolve { address } = request else {
            panic!("test request");
        };
        let path = crate::adapter::Linux::decode(&address)?;
        let mut file = open_with_hook(&path, context, || {
            std::fs::rename(&path, path.with_file_name("held")).unwrap();
            let name = CString::new(path.as_os_str().as_bytes()).unwrap();
            // SAFETY: fresh FIFO in this test's exclusively owned fixture directory.
            assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        })?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        Reply::stateless(Response::Content { bytes, eof: true })
    }
}

#[test]
fn regular_probe_reopens_the_held_inode_after_path_becomes_fifo() {
    // The owned fixture root lies inside the repository's _build output.
    let base = Temp::new();
    let path = base.path.join("source");
    std::fs::write(&path, b"held regular content").unwrap();
    let mut api = Api::default();
    // SAFETY: complete writable descriptor with lifetime covering all calls below.
    assert_eq!(
        unsafe {
            midnight_filesystem_plugin_sdk::entry::<Race>(
                &mut api,
                size_of::<Api>() as u32,
                "test.race",
                "0.1.0",
                CAP_READ,
            )
        },
        OK
    );
    let mut instance = 0;
    // SAFETY: valid Config and writable output until return.
    assert_eq!(
        unsafe { api.create.unwrap()(&Config::default(), &mut instance) },
        OK
    );
    let request = Request::Resolve {
        address: crate::adapter::Linux::encode(&path),
    };
    let bytes = encode_request(&request, DEFAULT_REQUEST_BYTES as usize).unwrap();
    let input = Call {
        struct_size: size_of::<Call>() as u32,
        operation: request.operation() as u32,
        payload: Slice {
            data: bytes.as_ptr(),
            len: bytes.len() as u64,
        },
        cancel: Cancellation::default(),
    };
    let mut output = Buffer::default();
    // SAFETY: borrowed request payload, callback descriptor and writable output remain mapped.
    assert_eq!(
        unsafe { api.call.unwrap()(instance, &input, &mut output) },
        OK
    );
    // SAFETY: live response buffer is retained by this same descriptor until release.
    let bytes = unsafe { std::slice::from_raw_parts(output.bytes.data, output.bytes.len as usize) };
    let response = decode_response(bytes, DEFAULT_REPLY_BYTES as usize).unwrap();
    // SAFETY: release and destruction use the producing descriptor, after the call quiesced.
    assert_eq!(
        unsafe { api.release_buffer.unwrap()(instance, output.id) },
        OK
    );
    assert_eq!(unsafe { api.destroy.unwrap()(instance) }, OK);
    assert_eq!(
        response,
        Response::Content {
            bytes: b"held regular content".to_vec(),
            eof: true
        }
    );
}

#[test]
fn validation_checks_the_current_name_without_opening_content_again() {
    let temp = Temp::new();
    let path = temp.path.join("selected");
    std::fs::write(&path, vec![7u8; 4096]).unwrap();
    let client = Client::new();
    let (mount, node) = client.resolve(&path);
    let opens = || DATA_OPENS.with(std::cell::Cell::get);
    let before = opens();
    let Response::Opened { reader, .. } = client.call(Request::OpenRead {
        mount,
        node: node.id,
    }) else {
        panic!("regular file must open")
    };
    for _ in 0..3 {
        assert_eq!(
            client.call(Request::ValidateRead { reader }),
            Response::Unit
        );
    }
    for offset in [0, 2048] {
        assert_eq!(
            client.call(Request::ReadAt {
                reader,
                offset,
                length: 2048,
            }),
            Response::Content {
                bytes: vec![7u8; 2048],
                eof: offset == 2048,
            }
        );
    }
    assert_eq!(opens() - before, 1, "only OpenRead opens file content");
    // The metadata-only check still detects a replaced name.
    std::fs::rename(&path, temp.path.join("held")).unwrap();
    std::fs::write(&path, vec![7u8; 4096]).unwrap();
    assert!(matches!(
        client.call(Request::ValidateRead { reader }),
        Response::Failed(Failure {
            code: ErrorCode::SourceChanged,
            ..
        })
    ));
    assert_eq!(opens() - before, 1);
    assert_eq!(client.call(Request::CloseRead { reader }), Response::Unit);
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
}
