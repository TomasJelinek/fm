use super::*;
use crate::Transport;
use midnight_filesystem_plugin_api::{ActivationTarget, ErrorCode, Kind, Metadata, NameEncoding};
use std::{collections::VecDeque, sync::Mutex};

#[derive(Debug, Default)]
struct Fake {
    replies: Mutex<VecDeque<Response>>,
    calls: Mutex<Vec<Request>>,
    cancel_on: Mutex<
        Option<(
            midnight_filesystem_plugin_api::Operation,
            midnight_core::Cancellation,
        )>,
    >,
}
impl Transport for Fake {
    fn call(&self, request: &Request, _: Cancel<'_>) -> Result<Response, Failure> {
        self.calls.lock().unwrap().push(request.clone());
        if matches!(
            request,
            Request::CloseMount { .. } | Request::CloseList { .. } | Request::CloseRead { .. }
        ) {
            return Ok(Response::Unit);
        }
        if let Some((operation, cancel)) = &*self.cancel_on.lock().unwrap()
            && request.operation() == *operation
        {
            cancel.cancel();
        }
        self.replies
            .lock()
            .unwrap()
            .pop_front()
            .ok_or_else(|| invalid("Unexpected request"))
    }
}
fn fixture(replies: Vec<Response>) -> (NetworkProvider, Arc<Fake>) {
    let fake = Arc::new(Fake {
        replies: Mutex::new(replies.into()),
        ..Default::default()
    });
    (
        NetworkProvider {
            id: "network.test".into(),
            client: NativeProvider {
                transport: fake.clone(),
                capabilities: abi::CAP_BROWSE
                    | abi::CAP_STAT
                    | abi::CAP_VOLUMES
                    | abi::CAP_ACTIVATE
                    | abi::CAP_REFRESH
                    | abi::CAP_RESOLVE_TEXT,
            },
        },
        fake,
    )
}
fn node(id: u64) -> Node {
    Node {
        id,
        address: Address {
            encoding: AddressEncoding::Opaque,
            bytes: vec![id as u8],
        },
        name_encoding: NameEncoding::Utf8,
        raw_name: "文档".as_bytes().into(),
        display: "文档".into(),
        kind: Kind::Directory,
        metadata: Metadata::default(),
    }
}
fn address() -> NetworkAddress {
    NetworkAddress {
        provider: "network.test".into(),
        opaque: Arc::from([1]),
        display: "Network".into(),
    }
}
fn resolved() -> Response {
    Response::Resolved {
        mount: 1,
        node: node(1),
    }
}

#[test]
fn browsing_is_streamed_and_never_activates_or_requests_native_content() {
    let (provider, fake) = fixture(vec![
        resolved(),
        Response::Page {
            cursor: 4,
            page: 0,
            done: false,
            entries: vec![node(2)],
        },
    ]);
    let mut names = vec![];
    provider
        .list_inner(&address(), &|| false, &mut |entry| {
            names.push(entry.display);
            false
        })
        .unwrap();
    assert_eq!(names, ["文档"]);
    let calls = fake.calls.lock().unwrap();
    assert!(matches!(calls[0], Request::Resolve { .. }));
    assert!(matches!(
        calls[1],
        Request::ListBegin { max_entries: 1, .. }
    ));
    assert_eq!(
        &calls[2..],
        &[
            Request::CloseList { cursor: 4 },
            Request::CloseMount { mount: 1 }
        ]
    );
}

#[test]
fn activation_preserves_opaque_addresses_and_rejects_native_uri_confusion() {
    let (provider, fake) = fixture(vec![
        resolved(),
        Response::Activated(ActivationTarget::WithinProvider(node(2).address)),
    ]);
    let NetworkDestination::Browse(next) = provider.activate_inner(&address(), &|| false).unwrap()
    else {
        panic!()
    };
    assert_eq!(next.opaque.as_ref(), &[2]);
    assert_eq!(next.provider, "network.test");
    assert_eq!(
        fake.calls.lock().unwrap().last(),
        Some(&Request::CloseMount { mount: 1 })
    );
    let (provider, _) = fixture(vec![
        resolved(),
        Response::Activated(ActivationTarget::Native(Address {
            encoding: AddressEncoding::Opaque,
            bytes: b"smb://host/share".to_vec(),
        })),
    ]);
    assert!(provider.activate_inner(&address(), &|| false).is_err());
}

#[test]
fn foreign_addresses_and_cancelled_requests_do_not_reach_plugin() {
    let (provider, fake) = fixture(vec![]);
    let mut foreign = address();
    foreign.provider = "other".into();
    assert!(provider.activate_inner(&foreign, &|| false).is_err());
    assert_eq!(
        provider
            .activate_inner(&address(), &|| true)
            .unwrap_err()
            .code,
        ErrorCode::Cancelled
    );
    assert!(fake.calls.lock().unwrap().is_empty());
}

#[test]
fn later_listing_failure_preserves_first_page_and_releases_leases() {
    let (provider, fake) = fixture(vec![
        resolved(),
        Response::Page {
            cursor: 4,
            page: 0,
            done: false,
            entries: vec![node(2)],
        },
        Response::Failed(Failure::new(ErrorCode::Offline, "Server disappeared")),
    ]);
    let mut received = vec![];
    let error = provider
        .list_inner(&address(), &|| false, &mut |entry| {
            received.push(entry);
            true
        })
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Offline);
    assert_eq!(received.len(), 1);
    let calls = fake.calls.lock().unwrap();
    assert_eq!(
        &calls[calls.len() - 2..],
        &[
            Request::CloseList { cursor: 4 },
            Request::CloseMount { mount: 1 }
        ]
    );
}

#[test]
fn successful_replies_after_cancellation_are_rejected_and_leases_are_closed() {
    use midnight_core::{Cancellation, NetworkNeighborhood};
    use midnight_filesystem_plugin_api::{Operation, Volume};
    for operation in [
        Operation::Volumes,
        Operation::ResolveText,
        Operation::Parent,
        Operation::Refresh,
        Operation::Activate,
        Operation::ListBegin,
    ] {
        let replies = match operation {
            Operation::Volumes => vec![Response::Volumes(vec![Volume {
                id: "root".into(),
                display: "Network".into(),
                root: node(1).address,
            }])],
            Operation::ResolveText => vec![resolved()],
            Operation::Parent => vec![resolved(), Response::Parent(None)],
            Operation::Refresh => vec![resolved(), Response::Unit],
            Operation::Activate => vec![
                resolved(),
                Response::Activated(ActivationTarget::WithinProvider(node(2).address)),
            ],
            Operation::ListBegin => vec![
                resolved(),
                Response::Page {
                    cursor: 4,
                    page: 0,
                    done: true,
                    entries: vec![],
                },
            ],
            _ => unreachable!(),
        };
        let (provider, fake) = fixture(replies);
        let cancel = Cancellation::default();
        *fake.cancel_on.lock().unwrap() = Some((operation, cancel.clone()));
        let result = match operation {
            Operation::Volumes => provider.root(&cancel).map(|_| ()),
            Operation::ResolveText => provider.parse("server", &cancel).map(|_| ()),
            Operation::Parent => provider.parent(&address(), &cancel).map(|_| ()),
            Operation::Refresh => provider.refresh(&address(), &cancel),
            Operation::Activate => provider.activate(&address(), &cancel).map(|_| ()),
            Operation::ListBegin => provider.list(&address(), &cancel, &mut |_| {
                panic!("cancelled page published")
            }),
            _ => unreachable!(),
        };
        assert_eq!(
            result.unwrap_err().kind,
            midnight_core::NetworkFailureKind::Cancelled,
            "late successful {operation:?}"
        );
        let calls = fake.calls.lock().unwrap();
        if operation != Operation::Volumes {
            assert_eq!(calls.last(), Some(&Request::CloseMount { mount: 1 }));
        }
        if operation == Operation::ListBegin {
            assert_eq!(
                &calls[calls.len() - 2..],
                &[
                    Request::CloseList { cursor: 4 },
                    Request::CloseMount { mount: 1 }
                ]
            );
        }
    }
}

#[cfg(windows)]
#[path = "tests_windows.rs"]
mod windows;

#[test]
fn public_network_errors_preserve_distinct_states_with_identical_messages() {
    use midnight_core::{Cancellation, NetworkNeighborhood};
    let mut kinds = std::collections::HashSet::new();
    for code in [
        ErrorCode::Unavailable,
        ErrorCode::Offline,
        ErrorCode::AuthenticationRequired,
        ErrorCode::PermissionDenied,
        ErrorCode::MountRequired,
        ErrorCode::Unsupported,
        ErrorCode::TimedOut,
        ErrorCode::Cancelled,
        ErrorCode::NotFound,
        ErrorCode::Busy,
        ErrorCode::Invalid,
        ErrorCode::Io,
        ErrorCode::SourceChanged,
        ErrorCode::StaleReference,
        ErrorCode::ResourceLimit,
    ] {
        let (provider, _) = fixture(vec![Response::Failed(Failure {
            code,
            message: "same message".into(),
            os_code: Some(1234),
        })]);
        let error = provider.root(&Cancellation::default()).unwrap_err();
        assert!(kinds.insert(format!("{:?}", error.kind)), "lost {code:?}");
        assert_eq!(error.message, "same message");
        assert_eq!(error.os_code, Some(1234));
    }
}

#[test]
fn malformed_captured_addresses_are_rejected_before_transport() {
    let mut oversized = address();
    oversized.opaque = vec![1; 16 * 1024 * 1024 + 1].into();
    let mut bad_display = address();
    bad_display.display.push('\0');
    let mut empty = address();
    empty.opaque = Arc::from([]);
    for address in [oversized, bad_display, empty] {
        let (provider, fake) = fixture(vec![]);
        assert_eq!(
            provider
                .activate_inner(&address, &|| false)
                .unwrap_err()
                .code,
            ErrorCode::Invalid
        );
        assert!(fake.calls.lock().unwrap().is_empty());
    }
}
