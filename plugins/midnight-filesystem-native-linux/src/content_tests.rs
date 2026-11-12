use super::*;
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
    let base = std::env::temp_dir().join(format!("native-open-race-{}", std::process::id()));
    // The native runner's TMPDIR is inside its dedicated _build fixture root.
    assert!(base.components().any(|part| part.as_os_str() == "_build"));
    std::fs::create_dir(&base).unwrap();
    struct Cleanup(PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).unwrap();
        }
    }
    let _cleanup = Cleanup(base.clone());
    let path = base.join("source");
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
