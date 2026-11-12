use midnight_filesystem_plugin_api::{
    decode_request, decode_response, encode_request, encode_response,
};

const LIMIT: usize = 1024 * 1024;
fn packet(body: &[u8]) -> Vec<u8> {
    [b"FSBR\x01\0\0\0".as_slice(), body].concat()
}
fn number(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn blob(bytes: &mut Vec<u8>, value: &[u8]) {
    number(bytes, value.len() as u64);
    bytes.extend_from_slice(value);
}
fn name(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.push(3); // Lossless Unix basename, independent of the test platform.
    blob(bytes, value);
}
fn object(bytes: &mut Vec<u8>) {
    blob(bytes, b"source-id");
    blob(bytes, b"binding-revision");
    name(bytes, b"source");
    bytes.push(2); // File.
    bytes.extend_from_slice(&1u16.to_le_bytes());
    number(bytes, 7);
    bytes.push(0); // Unknown compression.
}
fn failure(bytes: &mut Vec<u8>) {
    bytes.extend_from_slice(&5u16.to_le_bytes());
    bytes.push(0); // No OS error.
    blob(bytes, b"Binding changed.");
}
fn request_cases() -> Vec<(u32, Vec<u8>)> {
    let mut prepare = vec![];
    number(&mut prepare, 1);
    number(&mut prepare, 2);
    let mut rename = vec![];
    number(&mut rename, 3);
    number(&mut rename, 1);
    name(&mut rename, b"renamed");
    rename.push(0);
    let mut replace = rename.clone();
    *replace.last_mut().unwrap() = 1;
    number(&mut replace, 4);
    vec![
        (20, packet(&prepare)),
        (21, packet(&rename)),
        (21, packet(&replace)),
        (22, packet(&3u64.to_le_bytes())),
        (23, packet(&3u64.to_le_bytes())),
    ]
}
fn response_cases() -> Vec<Vec<u8>> {
    let mut prepared = 15u16.to_le_bytes().to_vec();
    number(&mut prepared, 3);
    object(&mut prepared);
    blob(&mut prepared, b"parent-id");
    prepared.push(1);
    let mut cases = vec![packet(&prepared)];
    for outcome in 0..=5 {
        let mut bytes = 16u16.to_le_bytes().to_vec();
        number(&mut bytes, 3);
        number(&mut bytes, u64::from(outcome != 0));
        bytes.push(outcome);
        match outcome {
            0 | 1 => {}
            2 => {
                number(&mut bytes, 4);
                object(&mut bytes);
                bytes.push(1);
            }
            3 => failure(&mut bytes),
            4 => {
                name(&mut bytes, b"renamed");
                bytes.push(0);
                bytes.extend_from_slice(&1u32.to_le_bytes());
                bytes.push(1); // Displaced target.
                bytes.push(2); // Unix address.
                blob(&mut bytes, b"/parent/.recovery/target-\xff");
                blob(&mut bytes, b"target-id");
                bytes.push(0); // Durability unconfirmed.
            }
            5 => {
                bytes.push(3); // Unknown source location.
                bytes.push(0); // No confirmed destination name.
                bytes.extend_from_slice(&0u32.to_le_bytes());
                failure(&mut bytes);
            }
            _ => unreachable!(),
        }
        cases.push(packet(&bytes));
    }
    cases
}

#[test]
fn prepared_rename_requests_preserve_the_assigned_wire_bytes() {
    for (operation, bytes) in request_cases() {
        let request = decode_request(operation, &bytes, LIMIT)
            .unwrap_or_else(|error| panic!("operation {operation} must decode: {error}"));
        assert_eq!(request.operation() as u32, operation);
        assert_eq!(encode_request(&request, LIMIT).unwrap(), bytes);
    }
}

#[test]
fn prepared_rename_receipts_preserve_all_outcome_tags_and_raw_bytes() {
    for bytes in response_cases() {
        let response = decode_response(&bytes, LIMIT)
            .unwrap_or_else(|error| panic!("receipt must decode: {error}; bytes={bytes:?}"));
        assert_eq!(encode_response(&response, LIMIT).unwrap(), bytes);
    }
}

#[test]
fn prepared_rename_transport_rejects_truncation_trailing_bytes_and_zero_handles() {
    for (operation, bytes) in request_cases() {
        assert!(decode_request(operation, &bytes, LIMIT).is_ok());
        for length in 0..bytes.len() {
            assert!(decode_request(operation, &bytes[..length], LIMIT).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode_request(operation, &trailing, LIMIT).is_err());
        let mut zero = bytes;
        zero[8..16].fill(0);
        assert!(decode_request(operation, &zero, LIMIT).is_err());
    }
    for bytes in response_cases() {
        assert!(decode_response(&bytes, LIMIT).is_ok());
        for length in 0..bytes.len() {
            assert!(decode_response(&bytes[..length], LIMIT).is_err());
        }
        let mut trailing = bytes.clone();
        trailing.push(0);
        assert!(decode_response(&trailing, LIMIT).is_err());
        let mut zero = bytes;
        zero[10..18].fill(0);
        assert!(decode_response(&zero, LIMIT).is_err());
    }
}

fn rename_name_packet(encoding: u8, value: &[u8]) -> Vec<u8> {
    let mut bytes = vec![];
    number(&mut bytes, 3);
    number(&mut bytes, 1);
    bytes.push(encoding);
    blob(&mut bytes, value);
    bytes.push(0);
    packet(&bytes)
}
fn prepared_packet(identity: &[u8], revision: &[u8]) -> Vec<u8> {
    let mut bytes = 15u16.to_le_bytes().to_vec();
    number(&mut bytes, 3);
    blob(&mut bytes, identity);
    blob(&mut bytes, revision);
    name(&mut bytes, b"source");
    bytes.push(2);
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.push(0);
    blob(&mut bytes, b"parent");
    bytes.push(1);
    packet(&bytes)
}
#[test]
fn prepared_rename_names_preserve_raw_bytes_and_apply_only_their_namespace_policy() {
    let wide = |value: &str| {
        value
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>()
    };
    for (encoding, value) in [
        (1, "文档 renamed".as_bytes().to_vec()),
        (2, wide("文档 renamed")),
        (2, vec![0x00, 0xd8]), // An unpaired UTF-16 unit is a lossless native name.
        (3, b"raw-\xff\\name".to_vec()), // Backslash is a Unix basename byte.
        (3, vec![b'a'; 64 * 1024]),
    ] {
        let bytes = rename_name_packet(encoding, &value);
        let request = decode_request(21, &bytes, LIMIT).unwrap();
        assert_eq!(encode_request(&request, LIMIT).unwrap(), bytes);
    }
    for encoding in [1, 2, 3] {
        for value in ["", ".", "..", "a/b", "a\0b"] {
            let value = if encoding == 2 {
                wide(value)
            } else {
                value.as_bytes().to_vec()
            };
            assert!(decode_request(21, &rename_name_packet(encoding, &value), LIMIT).is_err());
        }
    }
    for value in [
        "a\\b",
        "C:name",
        "name.",
        "name ",
        "NUL",
        "con.txt",
        "LPT¹.log",
        "COM9",
        "CONIN$",
        "a?b",
    ] {
        assert!(
            decode_request(21, &rename_name_packet(2, &wide(value)), LIMIT).is_err(),
            "{value}"
        );
    }
    for (encoding, value) in [
        (0, b"opaque".as_slice()),
        (4, b"unknown"),
        (1, b"\xff"),
        (2, b"x"),
    ] {
        assert!(decode_request(21, &rename_name_packet(encoding, value), LIMIT).is_err());
    }
    assert!(
        decode_request(
            21,
            &rename_name_packet(3, &vec![b'a'; 64 * 1024 + 1]),
            LIMIT
        )
        .is_err()
    );
}
#[test]
fn prepared_rename_identity_revision_and_nested_enums_are_bounded() {
    assert!(decode_response(&prepared_packet(b"id", b"revision"), LIMIT).is_ok());
    for identity in [vec![], vec![1; 257]] {
        assert!(decode_response(&prepared_packet(&identity, b"revision"), LIMIT).is_err());
    }
    for revision in [vec![], vec![1; 4097]] {
        assert!(decode_response(&prepared_packet(b"id", &revision), LIMIT).is_err());
    }
    assert!(decode_response(&prepared_packet(&[1; 256], &[2; 4096]), LIMIT).is_ok());
    let (_, valid) = request_cases().remove(1);
    for offset in [8usize, 16] {
        let mut invalid = valid.clone();
        invalid[offset..offset + 8].fill(0);
        assert!(decode_request(21, &invalid, LIMIT).is_err());
    }
    let mut invalid = valid.clone();
    *invalid.last_mut().unwrap() = 2;
    assert!(decode_request(21, &invalid, LIMIT).is_err());
    let mut replace = valid;
    *replace.last_mut().unwrap() = 1;
    number(&mut replace, 0);
    assert!(decode_request(21, &replace, LIMIT).is_err());
    for mut bytes in response_cases().into_iter().skip(1) {
        bytes[26] = 6; // Unknown RenameOutcome.
        assert!(decode_response(&bytes, LIMIT).is_err());
    }
}
#[test]
fn prepared_rename_recovery_roles_locations_and_aggregate_locators_are_bounded() {
    for location in 0..=3 {
        let mut bytes = 16u16.to_le_bytes().to_vec();
        number(&mut bytes, 3);
        number(&mut bytes, 1);
        bytes.push(5);
        bytes.push(location);
        bytes.push(1);
        name(&mut bytes, b"renamed");
        bytes.extend_from_slice(&3u32.to_le_bytes());
        for role in 0..=2 {
            bytes.push(role);
            bytes.push(2);
            blob(&mut bytes, b"/recovery/\xff");
            blob(&mut bytes, if role == 2 { b"" } else { b"id" });
        }
        failure(&mut bytes);
        let bytes = packet(&bytes);
        let response = decode_response(&bytes, LIMIT).unwrap();
        assert_eq!(encode_response(&response, LIMIT).unwrap(), bytes);
    }
    let receipt = |count: u32, size: usize, role: u8, location: u8| {
        let mut bytes = 16u16.to_le_bytes().to_vec();
        number(&mut bytes, 3);
        number(&mut bytes, 1);
        bytes.extend_from_slice(&[5, location, 0]);
        bytes.extend_from_slice(&count.to_le_bytes());
        for _ in 0..count {
            bytes.extend_from_slice(&[role, 2]);
            blob(&mut bytes, &vec![b'a'; size]);
            blob(&mut bytes, b"id");
        }
        failure(&mut bytes);
        packet(&bytes)
    };
    assert!(decode_response(&receipt(4, 64 * 1024, 0, 2), LIMIT).is_ok());
    for bytes in [
        receipt(5, 1, 0, 2),
        receipt(4, 64 * 1024 + 1, 0, 2),
        receipt(1, 1, 3, 2),
        receipt(1, 1, 0, 4),
    ] {
        assert!(decode_response(&bytes, LIMIT).is_err());
    }
}

#[test]
fn existing_windows_spellings_are_observed_losslessly_but_not_proposed_as_destinations() {
    use midnight_filesystem_plugin_api::*;
    for units in [
        "name.".encode_utf16().collect::<Vec<_>>(),
        "name ".encode_utf16().collect(),
        "NUL".encode_utf16().collect(),
        vec![0xd800],
    ] {
        let name = NativeText {
            encoding: NameEncoding::WindowsUtf16Le,
            bytes: units.iter().flat_map(|value| value.to_le_bytes()).collect(),
        };
        let object = RenameObject {
            identity: vec![1],
            revision: vec![2],
            name: name.clone(),
            kind: Kind::File,
            metadata: Metadata::default(),
        };
        for response in [
            Response::RenamePrepared(PreparedRename {
                handle: 1,
                source: object.clone(),
                parent_identity: vec![3],
                recoverable_replace: false,
            }),
            Response::RenameState(RenameState {
                prepared: 1,
                attempt: 1,
                outcome: RenameOutcome::Conflict(RenameConflict {
                    token: 4,
                    target: object,
                    can_replace: false,
                }),
            }),
        ] {
            let bytes = encode_response(&response, LIMIT).unwrap();
            assert_eq!(decode_response(&bytes, LIMIT).unwrap(), response);
        }
        if units != [0xd800] {
            assert!(
                encode_request(
                    &Request::Rename {
                        prepared: 1,
                        attempt: 1,
                        name,
                        policy: RenamePolicy::NoReplace
                    },
                    LIMIT
                )
                .is_err()
            );
        }
    }
    for units in [
        vec![],
        vec![0],
        vec![46],
        vec![46, 46],
        vec![97, 47, 98],
        vec![97, 92, 98],
    ] {
        let name = NativeText {
            encoding: NameEncoding::WindowsUtf16Le,
            bytes: units.into_iter().flat_map(u16::to_le_bytes).collect(),
        };
        assert!(name.validate_rename_observed_basename().is_err());
    }
}
