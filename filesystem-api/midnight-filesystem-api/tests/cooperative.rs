use midnight_filesystem_api::*;

#[test]
fn cooperative_version_is_explicit_and_keeps_streaming_records() {
    let protocol = Protocol::try_from(3).expect("negotiated cooperative protocol");
    let messages = [
        Message::StageBegin {
            request: 7,
            purpose: StagePurpose::Member { entry: 3 },
            expected_length: Some(4),
            max_length: 4,
        },
        Message::StageAppend {
            request: 7,
            stage: 1,
            offset: 0,
            bytes: b"data".to_vec(),
        },
        Message::StageProgress {
            request: 7,
            stage: 1,
            completed: 4,
        },
        Message::StageSeal {
            request: 7,
            stage: 1,
            length: 4,
        },
        Message::Failed {
            request: 7,
            code: ErrorCode::Cancelled,
            message: "cancelled".into(),
        },
        Message::ContentReady {
            request: 7,
            entry: 3,
            stage: 1,
            offset: 0,
            length: 4,
        },
    ];
    for message in messages {
        let mut current = Vec::new();
        write_message_with_protocol(&mut current, protocol, &message).unwrap();
        assert_eq!(
            read_message_with_protocol(&mut current.as_slice(), protocol).unwrap(),
            message
        );
        let mut legacy = Vec::new();
        write_message_with_protocol(&mut legacy, Protocol::V2, &message).unwrap();
        assert_eq!(&current[..4], &legacy[..4]);
        assert_eq!(
            &current[6..],
            &legacy[6..],
            "only the negotiated version changes bytes"
        );
        assert!(read_message_with_protocol(&mut current.as_slice(), Protocol::V2).is_err());
        assert!(read_message_with_protocol(&mut legacy.as_slice(), protocol).is_err());
    }
}

#[test]
fn cooperative_version_preserves_timestamps_and_strict_frame_limits() {
    let protocol = Protocol::try_from(3).expect("negotiated cooperative protocol");
    let message = Message::Entries {
        request: 8,
        more: false,
        entries: vec![Node {
            id: 2,
            parent: 1,
            name: b"a".to_vec(),
            display: "a".into(),
            kind: Kind::File,
            size: Some(4),
            readable: true,
            modified: Some(Timestamp::Utc {
                seconds: 123,
                nanoseconds: 456,
            }),
        }],
    };
    let mut bytes = Vec::new();
    write_message_with_protocol(&mut bytes, protocol, &message).unwrap();
    assert_eq!(
        read_message_with_protocol(&mut bytes.as_slice(), protocol).unwrap(),
        message
    );
    bytes.push(0);
    let size = (bytes.len() as u32 - 4).to_le_bytes();
    bytes[..4].copy_from_slice(&size);
    assert!(read_message_with_protocol(&mut bytes.as_slice(), protocol).is_err());
    assert!(
        write_message_with_protocol(
            &mut Vec::new(),
            protocol,
            &Message::StageAppend {
                request: 1,
                stage: 1,
                offset: 0,
                bytes: vec![1; MAX_READ as usize + 1],
            }
        )
        .is_err()
    );
    assert!(Protocol::try_from(4).is_err());
}
