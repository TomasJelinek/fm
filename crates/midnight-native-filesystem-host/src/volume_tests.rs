use super::*;
use midnight_core::{Cancellation, StorageService, VolumeService};

fn text(value: &str) -> NativeText {
    NativeText {
        encoding: test_path::encoding(),
        bytes: test_path::name(value),
    }
}

#[test]
fn rich_volume_fields_preserve_independent_values_and_native_text() {
    assert_rich_volume_fields(text);
}

#[test]
fn rich_volume_fields_accept_portable_utf8_text() {
    assert_rich_volume_fields(|value| NativeText {
        encoding: NameEncoding::Utf8,
        bytes: value.as_bytes().to_vec(),
    });
}

#[test]
fn malformed_portable_volume_text_is_rejected() {
    for bytes in [vec![0xff], b"label\0suffix".to_vec()] {
        let (host, _) = fixture(vec![Response::VolumeLabel(Some(NativeText {
            encoding: NameEncoding::Utf8,
            bytes,
        }))]);
        assert!(host.volume_label(&path()).is_err());
    }
}

fn assert_rich_volume_fields(text: fn(&str) -> NativeText) {
    let information = VolumeInformation {
        root: Some(test_path::address(&path())),
        label: Some(text("Data 文档")),
        filesystem: Some(text("native")),
        source: Some(text("source")),
        capacity: VolumeCapacity {
            total: Some(100),
            free: Some(60),
            available: Some(20),
        },
        storage: StorageKind::LocalSsd,
        read_only: Some(false),
        properties: vec![VolumeProperty::FileCompression, VolumeProperty::HardLinks],
        unavailable: vec!["Partial diagnostic".into()],
    };
    let (host, fake) = fixture(vec![
        Response::VolumeInformation(information),
        Response::VolumeLabel(Some(text("Data 文档"))),
    ]);
    let value = host
        .volume_information(&path(), &Cancellation::default())
        .unwrap();
    assert_eq!(value.root, Some(path()));
    assert_eq!(value.label, Some("Data 文档".into()));
    assert_eq!(value.filesystem, Some("native".into()));
    assert_eq!(value.source, Some("source".into()));
    assert_eq!(value.capacity.used(), Some(40));
    assert_eq!(value.capacity.available, Some(20));
    assert_eq!(value.storage, midnight_core::StorageKind::LocalSsd);
    assert_eq!(
        value.properties,
        vec![
            midnight_core::VolumeProperty::FileCompression,
            midnight_core::VolumeProperty::HardLinks
        ]
    );
    assert_eq!(value.unavailable, ["Partial diagnostic"]);
    assert_eq!(
        host.volume_label(&path()).unwrap(),
        Some("Data 文档".into())
    );
    assert_eq!(fake.requests.lock().unwrap().len(), 2);
}

#[test]
fn cancelled_or_unsupported_rich_queries_never_open_a_native_path() {
    let (mut host, fake) = fixture(vec![]);
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(host.volume_information(&path(), &cancel).is_err());
    host.capabilities &= !abi::CAP_VOLUME_DETAILS;
    assert!(host.volume_label(&path()).is_err());
    assert!(
        host.volume_information(&path(), &Cancellation::default())
            .is_err()
    );
    assert!(fake.requests.lock().unwrap().is_empty());
}
