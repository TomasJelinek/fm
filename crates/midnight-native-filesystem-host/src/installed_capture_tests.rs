use super::*;
use crate::{Request, Response, Transport, test_path};
use midnight_filesystem_plugin_api::{ErrorCode, Kind, Metadata, Node, abi};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

#[cfg(windows)]
#[path = "installed_capture_windows_tests.rs"]
mod platform;
#[cfg(target_os = "linux")]
#[path = "installed_capture_linux_tests.rs"]
mod platform;

#[derive(Debug)]
struct ContentTransport {
    path: PathBuf,
    revision: Vec<u8>,
    requests: Mutex<Vec<Request>>,
    cancel_on_validate: bool,
    cancelled: AtomicBool,
}
impl Transport for ContentTransport {
    fn call(&self, request: &Request, _: Cancel<'_>) -> Result<Response, Failure> {
        self.requests.lock().unwrap().push(request.clone());
        match request {
            Request::Resolve { address } => {
                assert_eq!(address, &test_path::address(&self.path));
                Ok(Response::Resolved {
                    mount: 1,
                    node: Node {
                        id: 2,
                        address: address.clone(),
                        name_encoding: test_path::encoding(),
                        raw_name: test_path::name("文档.zip"),
                        display: "文档.zip".into(),
                        kind: Kind::File,
                        metadata: Metadata {
                            size: Some(4),
                            ..Default::default()
                        },
                    },
                })
            }
            Request::OpenRead { mount: 1, node: 2 } => Ok(Response::Opened {
                reader: 3,
                length: 4,
                revision: self.revision.clone(),
            }),
            Request::ValidateRead { reader: 3 } => {
                if self.cancel_on_validate {
                    self.cancelled.store(true, Ordering::Release);
                }
                Ok(Response::Unit)
            }
            Request::CloseRead { reader: 3 } | Request::CloseMount { mount: 1 } => {
                Ok(Response::Unit)
            }
            Request::ReadAt {
                reader: 3,
                offset: 0,
                length: 4,
            } => Ok(Response::Content {
                bytes: b"data".to_vec(),
                eof: true,
            }),
            _ => panic!("Unexpected request: {request:?}"),
        }
    }
}

fn fixture(cancel_on_validate: bool) -> (InstalledProvider, Arc<ContentTransport>, PathBuf) {
    let path = std::env::temp_dir().join("absent-native-capture-文档.zip");
    let transport = Arc::new(ContentTransport {
        path: path.clone(),
        revision: platform::current(),
        requests: Mutex::default(),
        cancel_on_validate,
        cancelled: AtomicBool::new(false),
    });
    let installed = InstalledProvider::default();
    installed
        .provider
        .set(Ok(NativeProvider {
            transport: transport.clone(),
            capabilities: abi::CAP_NATIVE_ADDRESS | abi::CAP_READ,
        }))
        .unwrap();
    (installed, transport, path)
}

#[test]
fn matching_capture_returns_provider_bytes_and_current_revision() {
    for expected in [None, Some(platform::current()), Some(platform::legacy())] {
        let (installed, _, path) = fixture(false);
        let file = installed
            .capture_matching(&path, expected.as_deref(), &|| false)
            .unwrap();
        assert_eq!(file.stable_version(), platform::current());
        assert_eq!(file.read_at(0, 4).unwrap(), b"data");
    }
}

#[test]
fn mismatched_capture_releases_provider_resources_and_returns_source_changed() {
    let (installed, transport, path) = fixture(false);
    let mut changed = platform::legacy();
    *changed.last_mut().unwrap() ^= 1;
    let failure = installed
        .capture_matching(&path, Some(&changed), &|| false)
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::SourceChanged);
    assert!(transport.requests.lock().unwrap().ends_with(&[
        Request::CloseRead { reader: 3 },
        Request::CloseMount { mount: 1 },
    ]));
}

#[test]
fn cancellation_during_capture_rejects_matching_content_after_capture_returns() {
    let (installed, transport, path) = fixture(true);
    let failure = installed
        .capture_matching(&path, Some(&platform::current()), &|| {
            transport.cancelled.load(Ordering::Acquire)
        })
        .unwrap_err();
    assert_eq!(failure.code, ErrorCode::Cancelled);
    assert!(transport.requests.lock().unwrap().ends_with(&[
        Request::CloseRead { reader: 3 },
        Request::CloseMount { mount: 1 },
    ]));
}

#[test]
fn cancelled_matching_capture_does_not_discover_or_load_a_provider() {
    let installed = InstalledProvider::default();
    let path = std::env::temp_dir().join("cancelled.zip");
    assert_eq!(
        installed
            .capture_matching(&path, None, &|| true)
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert!(installed.provider.get().is_none());
}

#[test]
fn missing_provider_never_uses_an_existing_native_file() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .join("_build/native-capture-tests");
    std::fs::create_dir_all(&root).unwrap();
    let fixture = tempfile::tempdir_in(root).unwrap();
    let path = fixture.path().join("文档.zip");
    std::fs::write(&path, b"native bytes must not be read").unwrap();
    let revision = midnight_plugin_host::CapturedFile::capture(&path)
        .unwrap()
        .stable_version();
    let installed =
        InstalledProvider::from_installation(fixture.path().join("absent-provider"), None);
    assert!(
        installed
            .capture_matching(&path, Some(&revision), &|| false)
            .is_err()
    );
}
