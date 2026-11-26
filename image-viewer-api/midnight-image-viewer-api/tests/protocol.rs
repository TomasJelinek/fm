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
            height: 2048
        }
        .validate()
        .is_ok()
    );
    assert!(
        Metadata {
            width: 32768,
            height: 2049
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
fn decoded_images_up_to_256_mib_of_rgba_are_accepted() {
    // API 0.4.0 raised the decoded-byte budget from 128 MiB to 256 MiB.
    assert_eq!(api::MAX_DECODED_BYTES, 256 << 20);
    // 244 MiB of RGBA, above the former 128 MiB, is accepted on the wire too.
    let large = Metadata {
        width: 8000,
        height: 8000,
    };
    assert!(large.validate().is_ok());
    let mut bytes = Vec::new();
    api::write_message(&mut bytes, &Message::Metadata(large)).unwrap();
    assert_eq!(
        api::read_message(&mut bytes.as_slice()).unwrap(),
        Message::Metadata(large)
    );
    // Exactly 256 MiB is the largest; one more row is refused.
    let exact = Metadata {
        width: 8192,
        height: 8192,
    };
    assert!(exact.validate().is_ok());
    let over = Metadata {
        width: 8192,
        height: 8193,
    };
    assert!(over.validate().is_err());
    assert!(api::write_message(&mut Vec::new(), &Message::Metadata(over)).is_err());
    let mut bytes = b"MIV1".to_vec();
    bytes.extend_from_slice(&9u32.to_le_bytes());
    bytes.push(2);
    bytes.extend_from_slice(&8192u32.to_le_bytes());
    bytes.extend_from_slice(&8193u32.to_le_bytes());
    assert!(api::read_message(&mut bytes.as_slice()).is_err());
    bytes[13..].copy_from_slice(&8192u32.to_le_bytes());
    assert_eq!(
        api::read_message(&mut bytes.as_slice()).unwrap(),
        Message::Metadata(exact)
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

fn reason(text: &str) -> api::FailureReason {
    api::FailureReason::new(text).unwrap()
}

#[test]
fn failure_reason_has_a_bounded_printable_wire_form() {
    let message = Message::FailureReason(reason("Format error decoding Png: CRC error"));
    let mut bytes = Vec::new();
    api::write_message(&mut bytes, &message).unwrap();
    assert_eq!(&bytes[..9], b"MIV1\x25\0\0\0\x0a");
    assert_eq!(
        api::read_message(&mut Cursor::new(&bytes)).unwrap(),
        message
    );
    for end in 0..bytes.len() {
        assert!(api::read_message(&mut Cursor::new(&bytes[..end])).is_err());
    }
    let longest = "x".repeat(api::MAX_REASON_BYTES);
    let mut bytes = Vec::new();
    api::write_message(&mut bytes, &Message::FailureReason(reason(&longest))).unwrap();
    assert!(api::read_message(&mut Cursor::new(bytes)).is_ok());
    let wire = |payload: &[u8]| {
        let mut bytes = b"MIV1".to_vec();
        bytes.extend_from_slice(&(payload.len() as u32 + 1).to_le_bytes());
        bytes.push(10);
        bytes.extend_from_slice(payload);
        bytes
    };
    for payload in [
        b"".as_slice(),
        &[b'x'; api::MAX_REASON_BYTES + 1],
        b"line\nbreak",
        b"tab\there",
        b"escape \x1b[31m",
        b"delete \x7f",
        b"see /home/user/file.png",
        b"C:\\Users\\file.png",
        b"\xc3\xa9t\xc3\xa9",
        b"\xff\xfe",
        "right-to-left \u{202e}override".as_bytes(),
    ] {
        assert!(
            api::read_message(&mut Cursor::new(wire(payload))).is_err(),
            "{payload:?}"
        );
        if let Ok(text) = std::str::from_utf8(payload) {
            assert!(api::FailureReason::new(text).is_err(), "{text:?}");
        }
    }
    // The declared length is checked before the body is read or allocated.
    let mut oversized = b"MIV1".to_vec();
    oversized.extend_from_slice(&(api::MAX_REASON_BYTES as u32 + 2).to_le_bytes());
    oversized.push(10);
    assert!(api::read_message(&mut Cursor::new(oversized)).is_err());
}

#[test]
fn sanitized_reasons_keep_words_and_drop_controls_paths_and_excess() {
    let sanitize =
        |text: &str| api::FailureReason::sanitize(text).map(|reason| reason.as_str().to_owned());
    assert_eq!(
        sanitize("  Format error\tdecoding Png:\r\n  CRC error  ").as_deref(),
        Some("Format error decoding Png: CRC error")
    );
    assert_eq!(
        sanitize(
            "Bad font \"\u{164}est\u{202e}\" in /home/user/a.pdf or C:\\x\\a.pdf, see https://example.invalid/help"
        )
        .as_deref(),
        Some("Bad font \"?est?\" in ... or ... see ...")
    );
    assert_eq!(sanitize("\u{1b}[31mred\u{7}").as_deref(), Some("[31mred"));
    let long = sanitize(&"word ".repeat(1000)).unwrap();
    assert!(long.len() <= api::MAX_REASON_BYTES, "{long}");
    assert!(long.ends_with("word..."), "{long}");
    let unbroken = sanitize(&"x".repeat(100_000)).unwrap();
    assert_eq!(unbroken.len(), api::MAX_REASON_BYTES);
    assert!(unbroken.ends_with("x..."));
    for nothing in ["", " \n\t ", "/a/b \\c", "???", "\u{202e}\u{200b}"] {
        assert_eq!(sanitize(nothing), None, "{nothing:?}");
    }
    // Every sanitized reason is accepted on the wire.
    for text in ["\u{e9}", "a/b c", &"\u{fc}".repeat(500), "ok"] {
        if let Some(reason) = api::FailureReason::sanitize(text) {
            let mut bytes = Vec::new();
            api::write_message(&mut bytes, &Message::FailureReason(reason.clone())).unwrap();
            assert_eq!(
                api::read_message(&mut Cursor::new(bytes)).unwrap(),
                Message::FailureReason(reason)
            );
        }
    }
}
