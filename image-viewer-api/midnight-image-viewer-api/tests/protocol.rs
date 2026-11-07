use midnight_image_viewer_api::{self as api, Frame, Message, Metadata, Viewport};
use std::io::Cursor;

fn viewport() -> Viewport {
    Viewport {
        width: 4,
        height: 3,
        scale: 1.0,
        origin_x: 0.0,
        origin_y: 0.0,
    }
}

#[test]
fn all_messages_round_trip_without_paths_or_unbounded_strings() {
    for message in [
        Message::Open {
            source_length: u64::MAX,
        },
        Message::Metadata(Metadata {
            width: 4,
            height: 3,
        }),
        Message::Viewport {
            id: 5,
            viewport: viewport(),
        },
        Message::Rgba {
            id: 5,
            frame: Frame {
                width: 4,
                height: 3,
                rgba: vec![127; 48],
            },
        },
        Message::Read {
            offset: u64::MAX - 4,
            length: 4,
        },
        Message::Data(vec![1, 2, 3, 4]),
        Message::Failure,
    ] {
        let mut bytes = Vec::new();
        api::write_message(&mut bytes, &message).unwrap();
        assert_eq!(api::read_message(&mut Cursor::new(bytes)).unwrap(), message);
    }
}

#[test]
fn malformed_truncated_and_unknown_messages_fail() {
    let mut valid = Vec::new();
    api::write_message(
        &mut valid,
        &Message::Viewport {
            id: 2,
            viewport: viewport(),
        },
    )
    .unwrap();
    for end in 0..valid.len() {
        assert!(api::read_message(&mut Cursor::new(&valid[..end])).is_err());
    }
    for bytes in [
        b"MIV2\x01\0\0\0\x07".to_vec(),
        b"MIV1\xff\xff\xff\xff".to_vec(),
        b"MIV1\x01\0\0\0\xff".to_vec(),
        b"MIV1\x02\0\0\0\x07\0".to_vec(),
    ] {
        assert!(api::read_message(&mut Cursor::new(bytes)).is_err());
    }
}

#[test]
fn output_and_decode_budgets_are_independent() {
    assert!(api::rgba_length(2048, 2048).is_ok());
    assert!(api::rgba_length(4096, 1024).is_ok());
    assert!(api::rgba_length(4096, 1025).is_err());
    assert!(api::rgba_length(4097, 1).is_err());
    assert!(api::rgba_length(0, 1).is_err());
    assert!(
        Metadata {
            width: 32768,
            height: 1024
        }
        .validate()
        .is_ok()
    );
    assert!(
        Metadata {
            width: 32768,
            height: 1025
        }
        .validate()
        .is_err()
    );
    assert!(
        Metadata {
            width: 32769,
            height: 1
        }
        .validate()
        .is_err()
    );
}

#[test]
fn invalid_viewports_reads_and_frames_are_rejected_before_writing() {
    for scale in [0.0, -1.0, f64::INFINITY, f64::NAN, 65.0] {
        assert!(
            Viewport {
                scale,
                ..viewport()
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        Viewport {
            origin_x: f64::NAN,
            ..viewport()
        }
        .validate()
        .is_err()
    );
    for message in [
        Message::Read {
            offset: u64::MAX,
            length: 1,
        },
        Message::Read {
            offset: 0,
            length: 0,
        },
        Message::Read {
            offset: 0,
            length: 262145,
        },
        Message::Data(vec![0; 262145]),
        Message::Rgba {
            id: 0,
            frame: Frame {
                width: 2,
                height: 2,
                rgba: vec![0; 15],
            },
        },
    ] {
        let mut written = Vec::new();
        assert!(api::write_message(&mut written, &message).is_err());
        assert!(written.is_empty());
    }
}
