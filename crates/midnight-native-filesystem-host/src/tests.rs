use super::*;
use midnight_filesystem_plugin_api::*;
use std::sync::{
    Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Debug, Default)]
struct Fake {
    requests: Mutex<Vec<Request>>,
    replies: Mutex<std::collections::VecDeque<Response>>,
    cancel_after_resolve: AtomicBool,
    cancel_after_open: AtomicBool,
    cancelled: AtomicBool,
}
impl Transport for Fake {
    fn call(&self, request: &Request, _: Cancel<'_>) -> Result<Response, Failure> {
        self.requests.lock().unwrap().push(request.clone());
        if matches!(request, Request::Resolve { .. })
            && self.cancel_after_resolve.load(Ordering::Acquire)
        {
            self.cancelled.store(true, Ordering::Release);
        }
        if matches!(request, Request::OpenRead { .. })
            && self.cancel_after_open.load(Ordering::Acquire)
        {
            self.cancelled.store(true, Ordering::Release);
        }
        if matches!(
            request,
            Request::CloseMount { .. }
                | Request::CloseList { .. }
                | Request::CloseRead { .. }
                | Request::ValidateRead { .. }
        ) {
            return Ok(Response::Unit);
        }
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| Failure::new(ErrorCode::Invalid, "Unexpected request"))
    }
}
fn fixture(replies: Vec<Response>) -> (NativeProvider, Arc<Fake>) {
    let fake = Arc::new(Fake {
        replies: Mutex::new(replies.into()),
        ..Default::default()
    });
    (
        NativeProvider {
            transport: fake.clone(),
            capabilities: abi::CAP_NATIVE_ADDRESS
                | abi::CAP_BROWSE
                | abi::CAP_READ
                | abi::CAP_STAT
                | abi::CAP_VOLUMES
                | abi::CAP_STORAGE
                | abi::CAP_VOLUME_DETAILS,
        },
        fake,
    )
}
fn node() -> Node {
    Node {
        id: 2,
        address: test_path::address(
            &std::env::temp_dir()
                .join("host-native-tests")
                .join("žluťoučký.txt"),
        ),
        name_encoding: test_path::encoding(),
        raw_name: test_path::name("žluťoučký.txt"),
        display: "žluťoučký.txt".into(),
        kind: Kind::File,
        metadata: Metadata {
            size: Some(5),
            ..Metadata::default()
        },
    }
}
fn resolve() -> Response {
    Response::Resolved {
        mount: 1,
        node: node(),
    }
}
fn resolve_directory() -> Response {
    let mut root = node();
    root.id = 1;
    root.address = test_path::address(&path());
    root.kind = Kind::Directory;
    Response::Resolved {
        mount: 1,
        node: root,
    }
}
fn path() -> std::path::PathBuf {
    std::env::temp_dir().join("host-native-tests")
}

#[test]
fn stopping_listing_closes_cursor_before_mount_and_preserves_native_name() {
    let (host, fake) = fixture(vec![
        resolve_directory(),
        Response::Page {
            cursor: 7,
            page: 0,
            done: false,
            entries: vec![node()],
        },
    ]);
    let mut names = Vec::new();
    host.read_directory(&path(), &|| false, &mut |entry| {
        names.push(entry.name);
        false
    })
    .unwrap();
    assert_eq!(names, [std::ffi::OsString::from("žluťoučký.txt")]);
    let calls = fake.requests.lock().unwrap();
    assert!(matches!(
        calls.as_slice(),
        [
            Request::Resolve { .. },
            Request::ListBegin { .. },
            Request::CloseList { cursor: 7 },
            Request::CloseMount { mount: 1 }
        ]
    ));
}

#[test]
fn cancelled_owned_resolve_is_closed_without_starting_enumeration() {
    let (host, fake) = fixture(vec![resolve_directory()]);
    fake.cancel_after_resolve.store(true, Ordering::Release);
    let error = host
        .read_directory(
            &path(),
            &|| fake.cancelled.load(Ordering::Acquire),
            &mut |_| panic!("cancelled entry"),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Cancelled);
    assert!(matches!(
        fake.requests.lock().unwrap().as_slice(),
        [Request::Resolve { .. }, Request::CloseMount { mount: 1 }]
    ));
}

#[test]
fn wrong_page_number_fails_and_still_closes_cursor_and_mount() {
    let (host, fake) = fixture(vec![
        resolve_directory(),
        Response::Page {
            cursor: 7,
            page: 9,
            done: true,
            entries: vec![],
        },
    ]);
    assert!(
        host.read_directory(&path(), &|| false, &mut |_| true)
            .is_err()
    );
    assert!(matches!(
        &fake.requests.lock().unwrap()[2..],
        [
            Request::CloseList { cursor: 7 },
            Request::CloseMount { mount: 1 }
        ]
    ));
}

#[test]
fn reader_is_retained_for_captured_content_and_released_before_mount() {
    let (host, fake) = fixture(vec![
        resolve(),
        Response::Opened {
            reader: 8,
            length: 5,
            revision: vec![1],
        },
        Response::Content {
            bytes: b"hello".to_vec(),
            eof: true,
        },
    ]);
    let captured = host.capture(&path(), &|| false).unwrap();
    assert_eq!(captured.read_at(0, 5).unwrap(), b"hello");
    assert_eq!(captured.display_name(), "žluťoučký.txt");
    assert_eq!(captured.length(), 5);
    assert!(
        !fake
            .requests
            .lock()
            .unwrap()
            .iter()
            .any(|r| matches!(r, Request::CloseRead { .. }))
    );
    drop(captured);
    assert!(fake.requests.lock().unwrap().ends_with(&[
        Request::CloseRead { reader: 8 },
        Request::CloseMount { mount: 1 }
    ]));
}

#[test]
fn short_provider_read_is_an_error_not_fabricated_eof() {
    let (host, _) = fixture(vec![
        resolve(),
        Response::Opened {
            reader: 8,
            length: 5,
            revision: vec![1],
        },
        Response::Content {
            bytes: b"he".to_vec(),
            eof: true,
        },
    ]);
    let captured = host.capture(&path(), &|| false).unwrap();
    assert!(captured.read_at(0, 5).is_err());
}

#[test]
fn cancelled_owned_reader_closes_before_mount() {
    let (host, fake) = fixture(vec![
        resolve(),
        Response::Opened {
            reader: 8,
            length: 5,
            revision: vec![1],
        },
    ]);
    fake.cancel_after_open.store(true, Ordering::Release);
    assert_eq!(
        host.capture(&path(), &|| fake.cancelled.load(Ordering::Acquire))
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert!(fake.requests.lock().unwrap().ends_with(&[
        Request::CloseRead { reader: 8 },
        Request::CloseMount { mount: 1 }
    ]));
}

#[test]
fn repeated_node_and_unexpected_owned_response_are_rejected_and_closed() {
    let (host, fake) = fixture(vec![
        resolve_directory(),
        Response::Page {
            cursor: 7,
            page: 0,
            done: false,
            entries: vec![node()],
        },
        Response::Page {
            cursor: 7,
            page: 1,
            done: true,
            entries: vec![node()],
        },
    ]);
    let mut emitted = 0;
    assert!(
        host.read_directory(&path(), &|| false, &mut |_| {
            emitted += 1;
            true
        })
        .is_err()
    );
    assert_eq!(emitted, 1);
    assert!(fake.requests.lock().unwrap().ends_with(&[
        Request::CloseList { cursor: 7 },
        Request::CloseMount { mount: 1 }
    ]));
    let (host, fake) = fixture(vec![Response::Opened {
        reader: 8,
        length: 5,
        revision: vec![1],
    }]);
    assert!(host.capture(&path(), &|| false).is_err());
    assert!(
        fake.requests
            .lock()
            .unwrap()
            .ends_with(&[Request::CloseRead { reader: 8 }])
    );
}

#[test]
fn entry_address_cannot_escape_parent_or_disagree_with_raw_name() {
    for change in [false, true] {
        let mut entry = node();
        if change {
            entry.raw_name = test_path::name("different.txt");
        } else {
            entry.address =
                test_path::address(&std::env::temp_dir().join("elsewhere").join("žluťoučký.txt"));
        }
        let (host, _) = fixture(vec![
            resolve_directory(),
            Response::Page {
                cursor: 7,
                page: 0,
                done: true,
                entries: vec![entry],
            },
        ]);
        assert!(
            host.read_directory(&path(), &|| false, &mut |_| panic!("invalid row published"))
                .is_err()
        );
    }
}

#[test]
fn missing_capability_and_relative_path_never_invoke_plugin() {
    let (mut host, fake) = fixture(vec![]);
    assert!(
        host.capture(std::path::Path::new("relative.txt"), &|| false)
            .is_err()
    );
    host.capabilities = abi::CAP_NATIVE_ADDRESS;
    assert!(
        host.read_directory(&path(), &|| false, &mut |_| true)
            .is_err()
    );
    assert!(host.capture(&path(), &|| false).is_err());
    assert!(fake.requests.lock().unwrap().is_empty());
}

use crate::test_path;
#[path = "volume_tests.rs"]
mod volume;
