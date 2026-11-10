#![allow(dead_code)]
use midnight_filesystem_plugin_api::{abi::*, *};
use std::{
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

pub struct Client {
    pub table: Api,
    instance: u64,
}

impl Client {
    pub fn new() -> Self {
        let mut table = Api::default();
        // SAFETY: the complete aligned output table is live for this synchronous call.
        assert_eq!(
            unsafe {
                midnight_filesystem_native_macos::midnight_filesystem_entry_v1(
                    &mut table,
                    size_of::<Api>() as u32,
                )
            },
            OK
        );
        Self::from_table(table)
    }

    pub fn from_table(table: Api) -> Self {
        table.validate(size_of::<Api>() as u32).unwrap();
        let mut instance = 0;
        // SAFETY: configuration and owned handle output are valid until return.
        assert_eq!(
            unsafe { table.create.unwrap()(&Config::default(), &mut instance) },
            OK
        );
        Self { table, instance }
    }

    pub fn call(&self, request: Request) -> Response {
        self.cancelled(request, false)
    }

    pub fn cancelled(&self, request: Request, cancelled: bool) -> Response {
        unsafe extern "C" fn cancellation(context: u64) -> u32 {
            u32::from(context != 0)
        }
        let bytes = encode_request(&request, DEFAULT_REQUEST_BYTES as usize).unwrap();
        let call = Call {
            struct_size: size_of::<Call>() as u32,
            operation: request.operation() as u32,
            payload: Slice {
                data: bytes.as_ptr(),
                len: bytes.len() as u64,
            },
            cancel: Cancellation {
                context: u64::from(cancelled),
                cancelled: Some(cancellation),
            },
        };
        let mut output = Buffer::default();
        // SAFETY: bytes, callback and output remain live through the synchronous call.
        assert_eq!(
            unsafe { self.table.call.unwrap()(self.instance, &call, &mut output) },
            OK
        );
        output.validate(DEFAULT_REPLY_BYTES).unwrap();
        // SAFETY: the producing instance owns immutable reply bytes until release below.
        let bytes =
            unsafe { std::slice::from_raw_parts(output.bytes.data, output.bytes.len as usize) };
        let response = decode_response(bytes, DEFAULT_REPLY_BYTES as usize);
        // SAFETY: this instance produced the still-live output buffer.
        assert_eq!(
            unsafe { self.table.release_buffer.unwrap()(self.instance, output.id) },
            OK
        );
        response.unwrap()
    }

    pub fn resolve(&self, path: &Path) -> (u64, Node) {
        match self.call(Request::Resolve {
            address: address(path),
        }) {
            Response::Resolved { mount, node } => (mount, node),
            reply => panic!("Cannot resolve {}: {reply:?}", path.display()),
        }
    }

    pub fn open(&self, mount: u64, node: u64) -> u64 {
        match self.call(Request::OpenRead { mount, node }) {
            Response::Opened { reader, .. } => reader,
            reply => panic!("Cannot open captured content: {reply:?}"),
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // SAFETY: this test's synchronous calls and buffers have completed.
        assert_eq!(unsafe { self.table.destroy.unwrap()(self.instance) }, OK);
    }
}

pub fn address(path: &Path) -> Address {
    Address {
        encoding: AddressEncoding::UnixBytes,
        bytes: path.as_os_str().as_bytes().to_vec(),
    }
}

pub fn failure(response: Response, expected: ErrorCode) {
    match response {
        Response::Failed(error) => assert_eq!(error.code, expected),
        other => panic!("Expected {expected:?}, got {other:?}"),
    }
}

pub struct Fixture {
    pub path: PathBuf,
    root: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let root = std::env::var_os("MIDNIGHT_NATIVE_FIXTURE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| repository.join("_build/goal/native-macos/fixtures"));
        assert!(root.is_absolute() && root.components().any(|part| part.as_os_str() == "_build"));
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let path = root.join(format!(
            "macos-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self { path, root }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let path = self.path.canonicalize().unwrap();
        assert_eq!(path.parent(), Some(self.root.as_path()));
        assert!(path.starts_with(&self.root) && path != self.root);
        std::fs::remove_dir_all(path).unwrap();
    }
}
