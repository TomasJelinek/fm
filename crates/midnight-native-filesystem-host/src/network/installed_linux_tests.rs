//! Opt-in read-only discovery through the installed ABI and the desktop's GIO session.
use super::*;
use crate::Catalog;
use midnight_core::{Cancellation, NetworkNeighborhood};
use std::path::PathBuf;

#[test]
#[ignore = "requires installed network plugin, GIO/GVfs/wsdd and explicit LAN permission"]
fn installed_wsdd_namespace_and_network_root_browse_through_the_desktop() {
    assert_eq!(
        std::env::var("MIDNIGHT_NETWORK_TEST_ALLOW_LAN").as_deref(),
        Ok("1"),
        "Set MIDNIGHT_NETWORK_TEST_ALLOW_LAN=1 to permit local network discovery"
    );
    let installation = PathBuf::from(
        std::env::var_os("MIDNIGHT_NETWORK_TEST_INSTALLATION")
            .expect("Set MIDNIGHT_NETWORK_TEST_INSTALLATION to the application directory"),
    )
    .canonicalize()
    .unwrap();
    let catalog = Catalog::discover(&installation).unwrap();
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
    let provider = NetworkProvider::open(catalog.network_package().unwrap()).unwrap();
    let cancel = Cancellation::default();
    let root = provider.root(&cancel).unwrap();
    let wsdd = provider.parse("wsdd:///", &cancel).unwrap();
    assert_eq!(wsdd.provider, root.provider);
    assert_eq!(provider.parent(&wsdd, &cancel).unwrap(), Some(root.clone()));
    for address in [&wsdd, &root] {
        let NetworkDestination::Browse(ready) = provider.activate(address, &cancel).unwrap() else {
            panic!("Discovery namespaces must not activate a native filesystem path")
        };
        assert_eq!(&ready, address);
        provider.refresh(address, &cancel).unwrap();
        let mut count = 0;
        provider
            .list(address, &cancel, &mut |node| {
                node.address.validate().unwrap();
                assert_eq!(node.address.provider, root.provider);
                count += 1;
                true
            })
            .unwrap();
        // Zero is a valid result on a segment with no responding computers.
        // Do not print private computer names or infer SMB/FUSE interoperability.
        println!("{}: {count} discovery entries", address.display);
    }
}
