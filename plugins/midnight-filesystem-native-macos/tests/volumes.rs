#![cfg(target_os = "macos")]
#![allow(unsafe_code)]
mod support;
use midnight_filesystem_plugin_api::*;
use support::*;

#[test]
fn native_volumes_keep_raw_roots_and_report_independent_capacity() {
    let fixture = Fixture::new();
    let client = Client::new();
    let Response::Volumes(volumes) = client.call(Request::Volumes) else {
        panic!("Volume discovery failed")
    };
    assert!(volumes.iter().any(|volume| volume.root.bytes == b"/"));
    let unique: std::collections::HashSet<_> = volumes.iter().map(|volume| &volume.id).collect();
    assert_eq!(unique.len(), volumes.len());
    for volume in &volumes {
        assert_eq!(volume.root.encoding, AddressEncoding::UnixBytes);
        assert!(volume.root.bytes.starts_with(b"/") && !volume.root.bytes.contains(&0));
    }
    let Response::VolumeInformation(info) = client.call(Request::VolumeInformation {
        address: address(&fixture.path),
    }) else {
        panic!("Volume information failed")
    };
    assert!(info.capacity.total.is_some_and(|total| total > 0));
    assert!(info.capacity.free <= info.capacity.total);
    assert!(info.capacity.available <= info.capacity.total);
    assert!(info.filesystem.is_some());
    assert!(info.root.is_some());
    assert!(info.read_only.is_some());
    assert!(matches!(
        info.storage,
        StorageKind::Unknown | StorageKind::Network
    ));
    assert_eq!(
        client.call(Request::VolumeLabel {
            address: address(&fixture.path)
        }),
        Response::VolumeLabel(None)
    );
    let Response::SpaceInfo { available, total } = client.call(Request::SpaceInfo {
        address: address(&fixture.path),
    }) else {
        panic!("Space query failed")
    };
    // Capacity is queried independently; background writes can change available bytes.
    assert!(total > 0 && available <= total);
    failure(
        client.cancelled(Request::Volumes, true),
        ErrorCode::Cancelled,
    );
    failure(
        client.cancelled(
            Request::VolumeInformation {
                address: address(&fixture.path),
            },
            true,
        ),
        ErrorCode::Cancelled,
    );
}
