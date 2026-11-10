//! Trusted dynamic library boundary without any network discovery or activation.
use super::*;
use crate::Catalog;
use midnight_core::{Cancellation, NetworkFailureKind, NetworkNeighborhood};
use std::path::PathBuf;

#[test]
#[ignore = "requires a protected independent network plugin installation; performs no LAN I/O"]
fn installed_network_metadata_loads_and_cleans_up_without_discovery() {
    let installation = PathBuf::from(
        std::env::var_os("MIDNIGHT_NETWORK_TEST_INSTALLATION")
            .expect("Set a protected network plugin test installation"),
    )
    .canonicalize()
    .unwrap();
    let catalog = Catalog::discover(&installation).unwrap();
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
    let package = catalog.network_package().unwrap();
    assert!(NativeProvider::open(package).is_err());
    let provider = NetworkProvider::open(package).unwrap();
    let cancel = Cancellation::default();
    let root = provider.root(&cancel).unwrap();
    assert_eq!(root.provider, package.id);
    assert!(provider.parent(&root, &cancel).unwrap().is_none());

    // More than the plugin's 64-mount allowance proves each pure lookup closes
    // its lease. This reserved name is parsed only; it is never sent to the OS.
    for _ in 0..80 {
        let server = provider.parse("fixture.invalid", &cancel).unwrap();
        assert_eq!(
            provider.parent(&server, &cancel).unwrap(),
            Some(root.clone())
        );
        let (_lease, mount, node) = provider.resolve(&server, &|| false).unwrap();
        let Response::Node(stat) = provider
            .client
            .exchange(
                Request::Stat {
                    mount,
                    node: node.id,
                },
                &|| false,
            )
            .unwrap()
        else {
            panic!("Missing installed provider stat response")
        };
        assert_eq!(stat.id, node.id);
        assert_eq!(stat.address, node.address);
        assert_eq!(stat.kind, Kind::Directory);
    }

    assert!(matches!(
        provider
            .parse("ftp://fixture.invalid", &cancel)
            .unwrap_err()
            .kind,
        NetworkFailureKind::Unsupported | NetworkFailureKind::Invalid
    ));
    let mut foreign = root.clone();
    foreign.provider = "foreign.provider".into();
    assert_eq!(
        provider.parent(&foreign, &cancel).unwrap_err().kind,
        NetworkFailureKind::Invalid
    );
    let mut malformed = root.clone();
    malformed.opaque = vec![0xff].into();
    assert!(provider.parent(&malformed, &cancel).is_err());
    cancel.cancel();
    assert_eq!(
        provider.parse("fixture.invalid", &cancel).unwrap_err().kind,
        NetworkFailureKind::Cancelled
    );

    // Activation shape validation is pure. Never invoke the provider's Activate
    // operation: Linux may initialize a GVfs namespace even for its root.
    for address in [
        Address {
            encoding: AddressEncoding::Opaque,
            bytes: root.opaque.to_vec(),
        },
        Address {
            encoding: AddressEncoding::UnixBytes,
            bytes: b"relative/path".to_vec(),
        },
        Address {
            encoding: AddressEncoding::WindowsUtf16Le,
            bytes: vec![b'C'],
        },
    ] {
        assert!(ActivationTarget::Native(address).validate().is_err());
    }
}
