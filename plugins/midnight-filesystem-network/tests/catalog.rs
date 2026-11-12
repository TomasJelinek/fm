mod support;
use midnight_filesystem_network::{adapter::*, api::*};
use std::sync::atomic::{AtomicUsize, Ordering};
static ENUMERATIONS: AtomicUsize = AtomicUsize::new(0);
static ACTIVATIONS: AtomicUsize = AtomicUsize::new(0);

struct Fixture;
impl Adapter for Fixture {
    type Cursor = ();
    fn root() -> Record {
        Record {
            target: Target {
                kind: NodeKind::Root,
                key: b"fixture-root".to_vec(),
            },
            display: "Network".into(),
        }
    }
    fn validate(target: &Target) -> Result<(), Failure> {
        if target.key.starts_with(b"fixture-") {
            Ok(())
        } else {
            Err(Failure::new(ErrorCode::Invalid, "Wrong scope"))
        }
    }
    fn manual(input: &str) -> Result<Record, Failure> {
        Ok(Record {
            target: Target {
                kind: NodeKind::Share,
                key: format!("fixture-{input}").into_bytes(),
            },
            display: input.into(),
        })
    }
    fn begin(_: &Target, _: &Context<'_>) -> Result<(), Failure> {
        ENUMERATIONS.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
    fn next(_: &mut (), _: &Context<'_>) -> Result<Option<Record>, Failure> {
        Ok(None)
    }
    fn activate(target: &Target, context: &Context<'_>) -> Result<ActivationTarget, Failure> {
        context.check()?;
        ACTIVATIONS.fetch_add(1, Ordering::Relaxed);
        if target.kind == NodeKind::Share {
            Ok(ActivationTarget::Native(Address {
                encoding: AddressEncoding::UnixBytes,
                bytes: [b"/native/".as_slice(), &target.key].concat(),
            }))
        } else {
            Ok(ActivationTarget::WithinProvider(Address {
                encoding: AddressEncoding::Opaque,
                bytes: target.key.clone(),
            }))
        }
    }
}

#[test]
fn real_abi_keeps_restore_metadata_and_manual_resolution_separate_from_activation() {
    let client = support::Client::new::<Fixture>();
    let (mount, root) = client.root();
    assert!(matches!(
        client.call(Request::Parent {
            mount,
            node: root.id
        }),
        Response::Parent(None)
    ));
    assert!(matches!(
        client.call(Request::Stat {
            mount,
            node: root.id
        }),
        Response::Node(_)
    ));
    let Response::Resolved {
        mount: share_mount,
        node: share,
    } = client.call(Request::ResolveText {
        text: "文档".into(),
    })
    else {
        panic!("manual target must work before discovery")
    };
    assert_eq!(ENUMERATIONS.load(Ordering::Relaxed), 0);
    assert_eq!(ACTIVATIONS.load(Ordering::Relaxed), 0);
    assert!(matches!(
        client.call(Request::Stat {
            mount,
            node: share.id
        }),
        Response::Failed(Failure {
            code: ErrorCode::StaleReference,
            ..
        })
    ));
    assert!(matches!(
        client.cancelled(
            Request::Activate {
                mount: share_mount,
                node: share.id
            },
            true
        ),
        Response::Failed(Failure {
            code: ErrorCode::Cancelled,
            ..
        })
    ));
    assert_eq!(ACTIVATIONS.load(Ordering::Relaxed), 0);
    assert!(matches!(
        client.call(Request::Activate {
            mount: share_mount,
            node: share.id
        }),
        Response::Activated(ActivationTarget::Native(_))
    ));
    assert_eq!(ACTIVATIONS.load(Ordering::Relaxed), 1);
    assert!(matches!(
        client.call(Request::OpenRead {
            mount: share_mount,
            node: share.id
        }),
        Response::Failed(Failure {
            code: ErrorCode::Unsupported,
            ..
        })
    ));
    let Response::Parent(Some(parent)) = client.call(Request::Parent {
        mount: share_mount,
        node: share.id,
    }) else {
        panic!()
    };
    assert!(matches!(
        client.call(Request::Activate {
            mount: share_mount,
            node: parent.id
        }),
        Response::Activated(ActivationTarget::WithinProvider(_))
    ));
    assert_eq!(
        client.cancelled(Request::CloseMount { mount: share_mount }, true),
        Response::Unit
    );
    assert_eq!(client.call(Request::CloseMount { mount }), Response::Unit);
    assert!(matches!(
        client.call(Request::Stat {
            mount,
            node: root.id
        }),
        Response::Failed(Failure {
            code: ErrorCode::StaleReference,
            ..
        })
    ));
}
