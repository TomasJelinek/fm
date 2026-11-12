use midnight_filesystem_api::*;
use std::io::Cursor;

fn encode(protocol: Protocol, message: &Message) -> Vec<u8> {
    let mut bytes = Vec::new();
    write_message_with_protocol(&mut bytes, protocol, message).unwrap();
    bytes
}

#[test]
fn legacy_open_bytes_and_default_protocol_remain_exact() {
    let message = Message::Open {
        request: 1,
        format: Format::Zip,
        length: 7,
    };
    let expected = vec![
        20, 0, 0, 0, 1, 0, 1, 1, 0, 0, 0, 0, 0, 0, 0, 1, 7, 0, 0, 0, 0, 0, 0, 0,
    ];
    assert_eq!(encode(Protocol::V1, &message), expected);
    let mut default = Vec::new();
    write_message(&mut default, &message).unwrap();
    assert_eq!(default, expected);
    assert_eq!(read_message(&mut Cursor::new(expected)).unwrap(), message);
}

#[test]
fn streaming_messages_round_trip_and_are_never_accepted_as_legacy() {
    let messages = [
        Message::StageProgress {
            request: 7,
            stage: 1,
            completed: 256 * 1024,
        },
        Message::StageProgressAck {
            request: 7,
            stage: 1,
            completed: 256 * 1024,
        },
        Message::StageBegin {
            request: 2,
            purpose: StagePurpose::Member { entry: 9 },
            expected_length: Some(3),
            max_length: 3,
        },
        Message::StageBegin {
            request: 2,
            purpose: StagePurpose::Scratch,
            expected_length: None,
            max_length: u64::MAX,
        },
        Message::StageCreated {
            request: 2,
            stage: 8,
        },
        Message::StageAppend {
            request: 2,
            stage: 8,
            offset: 0,
            bytes: vec![1, 2, 3],
        },
        Message::StageAck {
            request: 2,
            stage: 8,
            next_offset: 3,
        },
        Message::StageSeal {
            request: 2,
            stage: 8,
            length: 3,
        },
        Message::StageSealed {
            request: 2,
            stage: 8,
            length: 3,
        },
        Message::SpoolRead {
            request: 2,
            stage: 8,
            offset: 1,
            length: 2,
        },
        Message::SpoolData {
            request: 2,
            stage: 8,
            bytes: vec![2, 3],
        },
        Message::ContentReady {
            request: 2,
            entry: 9,
            stage: 8,
            offset: 1,
            length: 2,
        },
    ];
    for message in messages {
        let bytes = encode(Protocol::V2, &message);
        assert_eq!(
            read_message_with_protocol(&mut Cursor::new(&bytes), Protocol::V2).unwrap(),
            message
        );
        assert!(read_message(&mut Cursor::new(&bytes)).is_err());
        assert!(write_message(&mut Vec::new(), &message).is_err());
        for length in 0..bytes.len() {
            assert!(
                read_message_with_protocol(&mut Cursor::new(&bytes[..length]), Protocol::V2)
                    .is_err()
            );
        }
    }
}

#[test]
fn streaming_rejects_invalid_ids_lengths_and_overflow_before_writing() {
    let messages = [
        Message::SourceRead {
            request: 0,
            offset: 0,
            length: 1,
        },
        Message::SourceRead {
            request: 1,
            offset: u64::MAX,
            length: 1,
        },
        Message::Read {
            request: 1,
            entry: 1,
            offset: 0,
            length: 1,
        },
        Message::SpoolRead {
            request: 1,
            stage: 1,
            offset: u64::MAX,
            length: 1,
        },
        Message::SpoolRead {
            request: 1,
            stage: 1,
            offset: 0,
            length: MAX_READ + 1,
        },
        Message::SpoolRead {
            request: 1,
            stage: 1,
            offset: 0,
            length: 0,
        },
        Message::StageCreated {
            request: 1,
            stage: 0,
        },
        Message::StageAppend {
            request: 1,
            stage: 1,
            offset: 0,
            bytes: vec![],
        },
        Message::StageAppend {
            request: 1,
            stage: 1,
            offset: u64::MAX,
            bytes: vec![1],
        },
        Message::StageAppend {
            request: 1,
            stage: 1,
            offset: 0,
            bytes: vec![1; MAX_READ as usize + 1],
        },
        Message::StageBegin {
            request: 1,
            purpose: StagePurpose::Member { entry: 1 },
            expected_length: Some(0),
            max_length: 0,
        },
        Message::StageBegin {
            request: 1,
            purpose: StagePurpose::Scratch,
            expected_length: Some(4),
            max_length: 3,
        },
        Message::ContentReady {
            request: 1,
            entry: 2,
            stage: 1,
            offset: u64::MAX,
            length: 2,
        },
    ];
    for message in messages {
        let mut output = Vec::new();
        assert!(
            write_message_with_protocol(&mut output, Protocol::V2, &message).is_err(),
            "{message:?}"
        );
        assert!(output.is_empty());
    }
}

#[test]
fn streaming_decoder_rejects_mutated_version_tag_boolean_and_extra_fields() {
    let message = Message::StageBegin {
        request: 1,
        purpose: StagePurpose::Scratch,
        expected_length: None,
        max_length: 7,
    };
    let encoded = encode(Protocol::V2, &message);
    for (offset, value) in [(4, 3), (6, 255), (15, 255), (16, 2)] {
        let mut bytes = encoded.clone();
        bytes[offset] = value;
        assert!(read_message_with_protocol(&mut Cursor::new(bytes), Protocol::V2).is_err());
    }
    let mut extra = encoded;
    extra.push(0);
    let length = (extra.len() - 4) as u32;
    extra[..4].copy_from_slice(&length.to_le_bytes());
    assert!(read_message_with_protocol(&mut Cursor::new(extra), Protocol::V2).is_err());
}
