use super::*;
use crate::{
    adapter::{NodeKind, Target},
    api::AddressEncoding,
};

fn record(kind: NodeKind, key: &[u8], display: &str) -> Record {
    Record {
        target: Target {
            kind,
            key: key.to_vec(),
        },
        display: display.into(),
    }
}
fn location() -> Location {
    Location {
        trail: vec![
            record(NodeKind::Root, b"root", "Network"),
            record(NodeKind::Container, b"scope-A", "Same caption"),
            record(NodeKind::Server, b"server-\xff\0", "文档 👩🏽‍💻"),
            record(NodeKind::Share, b"share", "déjà vu"),
        ],
    }
}

#[test]
fn exact_scoped_identity_unicode_and_parent_survive_storage() {
    let value = location();
    let address = value.encode().unwrap();
    assert_eq!(address.encoding, AddressEncoding::Opaque);
    assert_eq!(Location::decode(&address).unwrap(), value);
    let mut other_scope = value.clone();
    other_scope.trail[1].target.key = b"scope-B".to_vec();
    assert_ne!(other_scope.encode().unwrap(), address);
    let parent = value.parent().unwrap();
    assert_eq!(parent.trail, value.trail[..3]);
    assert!(
        parent
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .is_none()
    );
}

#[test]
fn long_resource_keys_are_not_truncated_or_limited_to_max_path() {
    let mut value = location();
    value.trail[3].target.key = "長い名前/".repeat(3000).into_bytes();
    let address = value.encode().unwrap();
    assert_eq!(Location::decode(&address).unwrap(), value);
}

#[test]
fn malformed_locations_cannot_become_native_paths_or_partial_trails() {
    let address = location().encode().unwrap();
    for end in 0..address.bytes.len() {
        let short = Address {
            encoding: AddressEncoding::Opaque,
            bytes: address.bytes[..end].to_vec(),
        };
        assert!(Location::decode(&short).is_err());
    }
    let mut trailing = address.clone();
    trailing.bytes.push(0);
    assert!(Location::decode(&trailing).is_err());
    let wrong = Address {
        encoding: AddressEncoding::UnixBytes,
        bytes: address.bytes,
    };
    assert!(Location::decode(&wrong).is_err());
    assert!(Location { trail: vec![] }.encode().is_err());
    let mut cycle = location();
    cycle.trail.push(cycle.trail[1].clone());
    assert!(cycle.encode().is_err());
    let mut bad_root = location();
    bad_root.trail[0].target.kind = NodeKind::Share;
    assert!(bad_root.encode().is_err());
}

#[test]
fn excessive_depth_and_bytes_are_explicit_resource_errors() {
    let mut value = location();
    value.trail = vec![value.trail[0].clone()];
    for index in 0..MAX_DEPTH {
        value
            .trail
            .push(record(NodeKind::Container, &index.to_le_bytes(), "group"));
    }
    assert_eq!(value.encode().unwrap_err().code, ErrorCode::ResourceLimit);
    let mut value = location();
    value.trail[3].target.key.resize(MAX_ADDRESS_BYTES, b'x');
    assert_eq!(value.encode().unwrap_err().code, ErrorCode::ResourceLimit);
}
