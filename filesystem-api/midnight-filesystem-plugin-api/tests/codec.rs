use midnight_filesystem_plugin_api::*;

fn address() -> Address {
    Address {
        encoding: AddressEncoding::UnixBytes,
        bytes: b"/raw/\xff".to_vec(),
    }
}
fn node() -> Node {
    Node {
        id: 7,
        address: address(),
        name_encoding: NameEncoding::UnixBytes,
        raw_name: b"\xff".to_vec(),
        display: "caption 文档".into(),
        kind: Kind::File,
        metadata: Metadata {
            size: Some(0),
            modified: Some(Timestamp {
                seconds: -1,
                nanos: 500_000_000,
            }),
            native_attributes: None,
            unix_mode: Some(0o100600),
            compression: Compression::Unknown,
        },
    }
}

#[test]
fn resolve_has_a_literal_portable_little_endian_fixture() {
    let request = Request::Resolve {
        address: Address {
            encoding: AddressEncoding::UnixBytes,
            bytes: b"/a".to_vec(),
        },
    };
    let expected = [
        b'F', b'S', b'B', b'R', 1, 0, 0, 0, 2, 2, 0, 0, 0, 0, 0, 0, 0, b'/', b'a',
    ];
    assert_eq!(encode_request(&request, 1024).unwrap(), expected);
    assert_eq!(decode_request(1, &expected, 1024).unwrap(), request);
}

#[test]
fn every_request_roundtrips_without_normalizing_paths_or_scoping_ids_by_labels() {
    for request in [
        Request::Resolve { address: address() },
        Request::Stat { mount: 1, node: 2 },
        Request::ListBegin {
            mount: 1,
            directory: 2,
            max_entries: 128,
        },
        Request::ListNext {
            cursor: 3,
            expected_page: 1,
            max_entries: 17,
        },
        Request::CloseList { cursor: 3 },
        Request::Parent { mount: 1, node: 2 },
        Request::OpenRead { mount: 1, node: 2 },
        Request::ValidateRead { reader: 4 },
        Request::ReadAt {
            reader: 4,
            offset: u64::MAX - 256,
            length: 256,
        },
        Request::CloseRead { reader: 4 },
        Request::CloseMount { mount: 1 },
        Request::Volumes,
        Request::StorageKind { address: address() },
        Request::SpaceInfo { address: address() },
    ] {
        let bytes = encode_request(&request, DEFAULT_REQUEST_BYTES as usize).unwrap();
        assert_eq!(
            decode_request(
                request.operation() as u32,
                &bytes,
                DEFAULT_REQUEST_BYTES as usize
            )
            .unwrap(),
            request
        );
    }
}

#[test]
fn every_response_roundtrips_missing_metadata_negative_time_and_exact_revision_bytes() {
    let node = node();
    for response in [
        Response::Unit,
        Response::Resolved {
            mount: 1,
            node: node.clone(),
        },
        Response::Node(node.clone()),
        Response::Page {
            cursor: 3,
            page: 0,
            done: false,
            entries: vec![node.clone()],
        },
        Response::Parent(Some(node)),
        Response::Parent(None),
        Response::Opened {
            reader: 4,
            length: u64::MAX,
            revision: vec![0, 255, 1],
        },
        Response::Content {
            bytes: vec![],
            eof: true,
        },
        Response::Volumes(vec![Volume {
            id: "root".into(),
            display: "Root".into(),
            root: address(),
        }]),
        Response::StorageKind(StorageKind::Unknown),
        Response::SpaceInfo {
            available: 0,
            total: u64::MAX,
        },
        Response::Failed(Failure {
            code: ErrorCode::PermissionDenied,
            os_code: Some(5),
            message: "Denied 文档".into(),
        }),
    ] {
        let bytes = encode_response(&response, DEFAULT_REPLY_BYTES as usize).unwrap();
        assert_eq!(
            decode_response(&bytes, DEFAULT_REPLY_BYTES as usize).unwrap(),
            response
        );
    }
}

#[test]
fn windows_surrogates_and_paths_far_beyond_max_path_remain_exact() {
    let units: Vec<u16> = [b'C' as u16, b':' as u16, b'\\' as u16]
        .into_iter()
        .chain(std::iter::repeat_n(b'x' as u16, 600))
        .chain([0xD800])
        .collect();
    let bytes: Vec<_> = units.into_iter().flat_map(u16::to_le_bytes).collect();
    let request = Request::Resolve {
        address: Address {
            encoding: AddressEncoding::WindowsUtf16Le,
            bytes,
        },
    };
    let encoded = encode_request(&request, 4096).unwrap();
    assert_eq!(decode_request(1, &encoded, 4096).unwrap(), request);
}

#[test]
fn truncated_lengths_and_bad_schema_are_rejected_before_model_allocation() {
    let valid = encode_request(&Request::Resolve { address: address() }, 1024).unwrap();
    for cut in 0..valid.len() {
        assert!(decode_request(1, &valid[..cut], 1024).is_err(), "{cut}");
    }
    for (offset, byte) in [(0, b'X'), (4, 2), (6, 1), (8, 99)] {
        let mut invalid = valid.clone();
        invalid[offset] = byte;
        assert!(decode_request(1, &invalid, 1024).is_err());
    }
    let mut huge = valid.clone();
    huge[9..17].copy_from_slice(&u64::MAX.to_le_bytes());
    assert!(decode_request(1, &huge, 1024).is_err());
    let mut trailing = valid.clone();
    trailing.push(0);
    assert!(decode_request(1, &trailing, 1024).is_err());
    assert!(decode_request(99, &valid, 1024).is_err());
    assert!(decode_request(1, &valid, valid.len() - 1).is_err());
}

#[test]
fn invalid_models_and_resource_overflows_are_rejected_without_truncation() {
    for request in [
        Request::Stat { mount: 0, node: 2 },
        Request::ListBegin {
            mount: 1,
            directory: 2,
            max_entries: 129,
        },
        Request::ReadAt {
            reader: 1,
            offset: u64::MAX,
            length: 1,
        },
        Request::ReadAt {
            reader: 1,
            offset: 0,
            length: 0,
        },
        Request::Resolve {
            address: Address {
                encoding: AddressEncoding::UnixBytes,
                bytes: b"/a\0b".to_vec(),
            },
        },
        Request::Resolve {
            address: Address {
                encoding: AddressEncoding::WindowsUtf16Le,
                bytes: vec![1],
            },
        },
    ] {
        assert!(encode_request(&request, 4096).is_err(), "{request:?}");
    }
    assert!(encode_request(&Request::Resolve { address: address() }, 8).is_err());
    let mut bad = node();
    bad.metadata.modified.as_mut().unwrap().nanos = 1_000_000_000;
    assert!(encode_response(&Response::Node(bad), 4096).is_err());
    assert!(
        encode_response(
            &Response::Page {
                cursor: 1,
                page: 0,
                done: true,
                entries: vec![node(); 129]
            },
            1_000_000
        )
        .is_err()
    );
}
