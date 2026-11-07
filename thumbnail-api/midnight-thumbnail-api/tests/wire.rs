use midnight_thumbnail_api::*;
use std::io::Cursor;

#[test]
fn bounded_request_and_rgba_frame_round_trip() {
    let messages = [
        Message::Request(Request {
            id: 42,
            source_length: u64::MAX,
            width: 128,
            height: 96,
            frame: 3,
        }),
        Message::Read {
            offset: 700,
            length: MAX_READ_BYTES as u32,
        },
        Message::Data(vec![1, 2, 3]),
        Message::Frame(Frame {
            id: 42,
            width: 2,
            height: 1,
            frame: 3,
            rgba: vec![1; 8],
        }),
        Message::Failed,
    ];
    for message in messages {
        let mut bytes = Vec::new();
        write_message(&mut bytes, &message).unwrap();
        assert_eq!(read_message(&mut Cursor::new(bytes)).unwrap(), message);
    }
}

#[test]
fn malformed_frame_and_excessive_allocations_are_rejected() {
    for message in [
        Message::Frame(Frame {
            id: 1,
            width: 2,
            height: 2,
            frame: 0,
            rgba: vec![0; 15],
        }),
        Message::Request(Request {
            id: 1,
            source_length: 5,
            width: MAX_EDGE + 1,
            height: 1,
            frame: 0,
        }),
        Message::Request(Request {
            id: 1,
            source_length: 5,
            width: 1,
            height: 1,
            frame: MAX_ANIMATION_FRAMES,
        }),
        Message::Read {
            offset: u64::MAX,
            length: 1,
        },
        Message::Data(vec![0; MAX_READ_BYTES + 1]),
    ] {
        assert!(write_message(&mut Vec::new(), &message).is_err());
    }
    let mut corrupt = b"MTH1".to_vec();
    corrupt.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(read_message(&mut Cursor::new(corrupt)).is_err());
}

#[test]
fn every_truncation_is_rejected() {
    let mut bytes = Vec::new();
    write_message(
        &mut bytes,
        &Message::Request(Request {
            id: 9,
            source_length: 12,
            width: 4,
            height: 5,
            frame: 0,
        }),
    )
    .unwrap();
    for length in 0..bytes.len() {
        assert!(read_message(&mut Cursor::new(&bytes[..length])).is_err());
    }
}
