use midnight_filesystem_api::*;
use std::io::Cursor;

#[test]
fn source_requests_and_opaque_directory_members_round_trip() {
    let messages = [
        Message::Open {
            request: 9,
            format: Format::Zip,
            length: 1024,
        },
        Message::SourceRead {
            request: 9,
            offset: 100,
            length: 7,
        },
        Message::SourceData {
            request: 9,
            bytes: vec![0, 255, 7],
        },
        Message::Entries {
            request: 10,
            entries: vec![Node {
                id: 7,
                parent: 1,
                name: b"\xff.txt".to_vec(),
                display: "�.txt".into(),
                kind: Kind::File,
                size: Some(7),
                readable: true,
            }],
            more: false,
        },
        Message::Failed {
            request: 11,
            code: ErrorCode::SourceChanged,
            message: "source replaced".into(),
        },
    ];
    for message in messages {
        let mut bytes = Vec::new();
        write_message(&mut bytes, &message).unwrap();
        assert_eq!(read_message(&mut Cursor::new(bytes)).unwrap(), message);
    }
}

#[test]
fn rejects_oversized_frames_before_allocation_and_truncated_payloads() {
    let mut data = Cursor::new(((MAX_FRAME + 1) as u32).to_le_bytes());
    assert!(read_message(&mut data).is_err());
    let message = Message::Read {
        request: 3,
        entry: 8,
        offset: u64::MAX,
        length: MAX_READ,
    };
    let mut bytes = Vec::new();
    write_message(&mut bytes, &message).unwrap();
    bytes.pop();
    assert!(read_message(&mut Cursor::new(bytes)).is_err());
}

#[test]
fn rejects_out_of_budget_read_and_noncanonical_trailing_data() {
    let mut bytes = Vec::new();
    assert!(
        write_message(
            &mut bytes,
            &Message::Read {
                request: 1,
                entry: 2,
                offset: 0,
                length: MAX_READ + 1,
            }
        )
        .is_err()
    );
    write_message(
        &mut bytes,
        &Message::Opened {
            request: 1,
            root: 1,
        },
    )
    .unwrap();
    bytes.push(0);
    let n = u32::from_le_bytes(bytes[..4].try_into().unwrap()) + 1;
    bytes[..4].copy_from_slice(&n.to_le_bytes());
    assert!(read_message(&mut Cursor::new(bytes)).is_err());
}
