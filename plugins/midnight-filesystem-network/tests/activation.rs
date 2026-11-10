#[path = "network_support/mod.rs"]
mod support;
use midnight_filesystem_network::{adapter::*, api::*};
use std::sync::atomic::{AtomicUsize, Ordering};
static MODE: AtomicUsize = AtomicUsize::new(0);
struct Fixture;
impl Adapter for Fixture {
    type Cursor = ();
    fn root() -> Record {
        Record {
            target: Target {
                kind: NodeKind::Root,
                key: b"root".to_vec(),
            },
            display: "Network".into(),
        }
    }
    fn validate(_: &Target) -> Result<(), Failure> {
        Ok(())
    }
    fn manual(input: &str) -> Result<Record, Failure> {
        Ok(Record {
            target: Target {
                kind: NodeKind::Server,
                key: input.as_bytes().to_vec(),
            },
            display: "Caption differs from identity".into(),
        })
    }
    fn begin(_: &Target, _: &Context<'_>) -> Result<(), Failure> {
        panic!("activation fixture must not enumerate")
    }
    fn next(_: &mut (), _: &Context<'_>) -> Result<Option<Record>, Failure> {
        panic!("activation fixture must not enumerate")
    }
    fn activate(target: &Target, _: &Context<'_>) -> Result<ActivationTarget, Failure> {
        Ok(match MODE.load(Ordering::SeqCst) {
            0 => ActivationTarget::WithinProvider(Address {
                encoding: AddressEncoding::Opaque,
                bytes: target.key.clone(),
            }),
            1 => ActivationTarget::WithinProvider(Address {
                encoding: AddressEncoding::Opaque,
                bytes: b"different-target".to_vec(),
            }),
            _ => ActivationTarget::Native(Address {
                encoding: AddressEncoding::UnixBytes,
                bytes: b"/native-but-not-a-share".to_vec(),
            }),
        })
    }
}
#[test]
fn namespace_activation_preserves_full_trail_and_rejects_adapter_retargeting() {
    let client = support::Client::new::<Fixture>();
    let Response::Resolved { mount, node } = client.call(Request::ResolveText {
        text: "server-identity".into(),
    }) else {
        panic!()
    };
    let Response::Activated(ActivationTarget::WithinProvider(address)) =
        client.call(Request::Activate {
            mount,
            node: node.id,
        })
    else {
        panic!()
    };
    assert_eq!(address, node.address);
    assert_ne!(address.bytes, b"server-identity");
    let Response::Resolved {
        mount: restored,
        node: server,
    } = client.call(Request::Resolve { address })
    else {
        panic!()
    };
    assert_eq!(server.raw_name, b"server-identity");
    let Response::Parent(Some(parent)) = client.call(Request::Parent {
        mount: restored,
        node: server.id,
    }) else {
        panic!()
    };
    assert_eq!(parent.raw_name, b"root");
    for mode in [1, 2] {
        MODE.store(mode, Ordering::SeqCst);
        assert!(matches!(
            client.call(Request::Activate {
                mount,
                node: node.id
            }),
            Response::Failed(Failure {
                code: ErrorCode::Invalid,
                ..
            })
        ));
    }
}
