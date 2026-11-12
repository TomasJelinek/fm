use midnight_filesystem_plugin_api::*;

#[test]
fn optional_volume_queries_decode_without_changing_legacy_space_wire() {
    let payload = b"FSBR\x01\0\0\0\x02\x01\0\0\0\0\0\0\0/";
    for operation in [15, 16] {
        assert!(
            decode_request(operation, payload, 1024).is_ok(),
            "operation {operation}"
        );
    }
    let legacy = Response::SpaceInfo {
        available: 7,
        total: 19,
    };
    assert_eq!(
        encode_response(&legacy, 1024).unwrap(),
        b"FSBR\x01\0\0\0\x0a\0\x07\0\0\0\0\0\0\0\x13\0\0\0\0\0\0\0"
    );
}

#[test]
fn optional_empty_label_has_an_additive_response_tag() {
    assert!(decode_response(b"FSBR\x01\0\0\0\x0c\0\0", 1024).is_ok());
}

#[test]
fn raw_details_roundtrip_preserves_independent_capacity_and_partial_failures() {
    let value = Response::VolumeInformation(VolumeInformation {
        root: Some(Address {
            encoding: AddressEncoding::UnixBytes,
            bytes: b"/media/raw\xff".to_vec(),
        }),
        label: Some(NativeText {
            encoding: NameEncoding::WindowsUtf16Le,
            bytes: vec![0, 0xd8],
        }),
        filesystem: Some(NativeText {
            encoding: NameEncoding::Utf8,
            bytes: b"ext4".to_vec(),
        }),
        source: Some(NativeText {
            encoding: NameEncoding::UnixBytes,
            bytes: b"/dev/raw\xff".to_vec(),
        }),
        capacity: VolumeCapacity {
            total: Some(100),
            free: Some(40),
            available: Some(0),
        },
        storage: StorageKind::Unknown,
        read_only: Some(false),
        properties: vec![VolumeProperty::NoExecute, VolumeProperty::SparseFiles],
        unavailable: vec!["Optional device identity unavailable".into()],
    });
    let bytes = encode_response(&value, 4096).unwrap();
    assert_eq!(&bytes[8..10], &[13, 0]);
    assert_eq!(decode_response(&bytes, 4096).unwrap(), value);
    assert_eq!(
        encode_response(
            &Response::Failed(Failure::new(ErrorCode::Unsupported, "old")),
            4096
        )
        .unwrap()[8..10],
        [11, 0]
    );
}

#[test]
fn details_reject_impossible_counters_duplicate_properties_and_bad_native_text() {
    let encode = |value| {
        encode_response(
            &Response::VolumeInformation(value),
            DEFAULT_REPLY_BYTES as usize,
        )
    };
    for capacity in [
        VolumeCapacity {
            total: Some(100),
            free: Some(101),
            available: None,
        },
        VolumeCapacity {
            total: Some(100),
            free: None,
            available: Some(101),
        },
    ] {
        assert!(
            encode(VolumeInformation {
                capacity,
                ..Default::default()
            })
            .is_err()
        );
    }
    assert!(
        encode(VolumeInformation {
            properties: vec![VolumeProperty::Quotas; 2],
            ..Default::default()
        })
        .is_err()
    );
    for (encoding, bytes) in [
        (NameEncoding::WindowsUtf16Le, vec![1]),
        (NameEncoding::WindowsUtf16Le, vec![0, 0]),
        (NameEncoding::UnixBytes, vec![0]),
        (NameEncoding::Utf8, vec![0xff]),
        (NameEncoding::Opaque, vec![1]),
        (NameEncoding::UnixBytes, vec![1; MAX_VOLUME_TEXT + 1]),
    ] {
        assert!(
            encode(VolumeInformation {
                label: Some(NativeText { encoding, bytes }),
                ..Default::default()
            })
            .is_err()
        );
    }
    assert!(
        encode(VolumeInformation {
            unavailable: vec!["x".into(); MAX_VOLUME_DIAGNOSTICS + 1],
            ..Default::default()
        })
        .is_err()
    );
    assert!(
        encode(VolumeInformation {
            capacity: VolumeCapacity {
                total: None,
                free: Some(100),
                available: None
            },
            ..Default::default()
        })
        .is_ok()
    );
}

#[test]
fn decoder_rejects_duplicate_wire_properties_and_unknown_tags() {
    let mut bytes = b"FSBR\x01\0\0\0\x0d\0".to_vec();
    // No root/text/counters, Unknown storage and absent read-only.
    bytes.extend([0; 9]);
    bytes.extend(2_u32.to_le_bytes());
    bytes.extend([11, 11]);
    bytes.extend(0_u32.to_le_bytes());
    assert_eq!(decode_response(&bytes, 1024), Err(WireError::Malformed));
    bytes[23] = 255;
    assert_eq!(decode_response(&bytes, 1024), Err(WireError::Unsupported));
}

#[test]
fn decoder_rejects_capacity_fields_exceeding_physical_total() {
    let value = Response::VolumeInformation(VolumeInformation {
        capacity: VolumeCapacity {
            total: Some(100),
            free: Some(40),
            available: Some(0),
        },
        ..Default::default()
    });
    let valid = encode_response(&value, 1024).unwrap();
    for offset in [24, 33] {
        let mut invalid = valid.clone();
        invalid[offset..offset + 8].copy_from_slice(&101_u64.to_le_bytes());
        assert_eq!(decode_response(&invalid, 1024), Err(WireError::Malformed));
    }
}

#[test]
fn volume_root_uses_transport_budget_instead_of_native_text_field_limit() {
    let mut bytes = vec![b'/'];
    bytes.extend(vec![b'x'; MAX_VOLUME_TEXT + 32]);
    let value = Response::VolumeInformation(VolumeInformation {
        root: Some(Address {
            encoding: AddressEncoding::UnixBytes,
            bytes,
        }),
        ..Default::default()
    });
    let encoded = encode_response(&value, DEFAULT_REPLY_BYTES as usize)
        .expect("native addresses are bounded by the transport, not the text-field budget");
    assert_eq!(
        decode_response(&encoded, DEFAULT_REPLY_BYTES as usize).unwrap(),
        value
    );
}
