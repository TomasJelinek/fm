use super::*;
use std::io::Cursor;

#[test]
fn broker_caps_reads_and_rejects_out_of_range_seeks() {
    let mut input = Vec::new();
    api::write_message(
        &mut input,
        &api::Message::Data(vec![3; api::MAX_READ_BYTES]),
    )
    .unwrap();
    let mut input = Cursor::new(input);
    let mut output = Vec::new();
    let mut broker = Broker::new(&mut input, &mut output, u64::MAX);
    let mut bytes = vec![0; api::MAX_READ_BYTES + 1];
    assert_eq!(broker.read(&mut bytes).unwrap(), api::MAX_READ_BYTES);
    assert!(broker.seek(SeekFrom::End(1)).is_err());
    broker.seek(SeekFrom::Start(0)).unwrap();
    assert!(broker.seek(SeekFrom::Current(-1)).is_err());
    assert_eq!(
        api::read_message(&mut Cursor::new(output)).unwrap(),
        api::Message::Read {
            offset: 0,
            length: api::MAX_READ_BYTES as u32,
        }
    );
}

#[test]
fn broker_rejects_short_oversized_and_wrong_replies() {
    for reply in [
        api::Message::Data(vec![]),
        api::Message::Data(vec![1; 3]),
        api::Message::Failure,
    ] {
        let mut input = Vec::new();
        api::write_message(&mut input, &reply).unwrap();
        let mut input = Cursor::new(input);
        let mut output = Vec::new();
        assert!(
            Broker::new(&mut input, &mut output, 2)
                .read(&mut [0; 2])
                .is_err()
        );
    }
}

struct Pixel;
impl Image for Pixel {
    fn metadata(&self) -> api::Metadata {
        api::Metadata {
            width: 1,
            height: 1,
        }
    }
    fn render(&self, viewport: api::Viewport) -> io::Result<api::Frame> {
        Ok(api::Frame {
            width: viewport.width,
            height: viewport.height,
            rgba: vec![9; api::rgba_length(viewport.width, viewport.height)?],
        })
    }
}

#[test]
fn session_decodes_once_and_serves_multiple_viewports() {
    let mut bytes = Vec::new();
    api::write_message(&mut bytes, &api::Message::Open { source_length: 1 }).unwrap();
    api::write_message(&mut bytes, &api::Message::Data(vec![1])).unwrap();
    for id in [1, 2] {
        api::write_message(
            &mut bytes,
            &api::Message::Viewport {
                id,
                viewport: api::Viewport {
                    width: 1,
                    height: 1,
                    scale: 1.0,
                    origin_x: 0.0,
                    origin_y: 0.0,
                },
            },
        )
        .unwrap();
    }
    let mut output = Vec::new();
    let result = session(&mut Cursor::new(bytes), &mut output, |stream| {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        assert_eq!(byte, [1]);
        Ok(Pixel)
    });
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::UnexpectedEof);
    let mut output = Cursor::new(output);
    assert!(matches!(
        api::read_message(&mut output).unwrap(),
        api::Message::Read { .. }
    ));
    assert_eq!(
        api::read_message(&mut output).unwrap(),
        api::Message::Metadata(Pixel.metadata())
    );
    for expected in [1, 2] {
        assert!(
            matches!(api::read_message(&mut output).unwrap(), api::Message::Rgba { id, .. } if id == expected)
        );
    }
}

fn open(source_length: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    api::write_message(&mut bytes, &api::Message::Open { source_length }).unwrap();
    bytes
}

fn reason(text: &str) -> api::Message {
    api::Message::FailureReason(api::FailureReason::new(text).unwrap())
}

#[test]
fn a_failed_decode_ends_with_failure_and_its_sanitized_reason() {
    let mut output = Vec::new();
    session(&mut Cursor::new(open(0)), &mut output, |_| {
        Err::<Pixel, _>(io::Error::new(
            io::ErrorKind::InvalidData,
            "Format error decoding Png:\n CRC mismatch in /private/source.png",
        ))
    })
    .unwrap();
    let mut written = Cursor::new(output);
    assert_eq!(
        api::read_message(&mut written).unwrap(),
        api::Message::Failure
    );
    assert_eq!(
        api::read_message(&mut written).unwrap(),
        reason("Format error decoding Png: CRC mismatch in ...")
    );
    assert!(api::read_message(&mut written).is_err());
}

#[test]
fn a_reason_without_printable_words_leaves_only_failure() {
    let mut output = Vec::new();
    session(&mut Cursor::new(open(0)), &mut output, |_| {
        Err::<Pixel, _>(io::Error::other("/a/b \u{202e}\u{1b}"))
    })
    .unwrap();
    let mut written = Cursor::new(output);
    assert_eq!(
        api::read_message(&mut written).unwrap(),
        api::Message::Failure
    );
    assert_eq!(written.position(), written.get_ref().len() as u64);
}

struct Oversized;
impl Image for Oversized {
    fn metadata(&self) -> api::Metadata {
        api::Metadata {
            width: api::MAX_DECODED_EDGE + 1,
            height: 1,
        }
    }
    fn render(&self, _: api::Viewport) -> io::Result<api::Frame> {
        Err(io::Error::other("Never rendered"))
    }
}

#[test]
fn invalid_metadata_and_render_errors_explain_themselves() {
    let mut output = Vec::new();
    session(&mut Cursor::new(open(0)), &mut output, |_| Ok(Oversized)).unwrap();
    let mut written = Cursor::new(output);
    assert_eq!(
        api::read_message(&mut written).unwrap(),
        api::Message::Failure
    );
    assert_eq!(
        api::read_message(&mut written).unwrap(),
        reason("The image dimensions exceed the viewer's limits")
    );

    struct Broken;
    impl Image for Broken {
        fn metadata(&self) -> api::Metadata {
            Pixel.metadata()
        }
        fn render(&self, _: api::Viewport) -> io::Result<api::Frame> {
            Err(io::Error::other("Out of tiles"))
        }
    }
    let mut input = open(0);
    api::write_message(
        &mut input,
        &api::Message::Viewport {
            id: 4,
            viewport: api::Viewport {
                width: 1,
                height: 1,
                scale: 1.0,
                origin_x: 0.0,
                origin_y: 0.0,
            },
        },
    )
    .unwrap();
    let mut output = Vec::new();
    session(&mut Cursor::new(input), &mut output, |_| Ok(Broken)).unwrap();
    let mut written = Cursor::new(output);
    assert_eq!(
        api::read_message(&mut written).unwrap(),
        api::Message::Metadata(Pixel.metadata())
    );
    assert_eq!(
        api::read_message(&mut written).unwrap(),
        api::Message::Failure
    );
    assert_eq!(
        api::read_message(&mut written).unwrap(),
        reason("Out of tiles")
    );
}
