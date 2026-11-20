//! Bound the wire allocation to the requested tile before the public decoder allocates it.
use midnight_image_viewer_api::{self as api, Message};
use std::io::{self, Read};

pub(crate) fn read(reader: &mut impl Read, width: u32, height: u32) -> io::Result<Message> {
    let maximum = api::rgba_length(width, height)? + 17;
    let mut header = [0u8; 8];
    reader.read_exact(&mut header)?;
    let length = u32::from_le_bytes(header[4..].try_into().map_err(io::Error::other)?) as usize;
    if &header[..4] != b"MIV1" || length == 0 || length > maximum {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "Image reply exceeds requested raster",
        ));
    }
    api::read_message(&mut std::io::Cursor::new(header).chain(reader))
}

/// The optional reason a worker sends right after `Failure` (API 0.3.0), read under the
/// caller's deadline. Anything but one well-formed reason, including the end of stream of
/// a worker before SDK 0.4.0 that sends none, gives `None`; the declared length is checked
/// before the body is read.
pub(crate) fn failure_reason(reader: &mut impl Read) -> Option<api::FailureReason> {
    let mut header = [0u8; 9];
    reader.read_exact(&mut header).ok()?;
    let length = u32::from_le_bytes(header[4..8].try_into().ok()?) as usize;
    if &header[..4] != b"MIV1" || header[8] != 10 || length > api::MAX_REASON_BYTES + 1 {
        return None;
    }
    match api::read_message(&mut std::io::Cursor::new(header).chain(reader)).ok()? {
        Message::FailureReason(reason) => Some(reason),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct HeaderOnly(std::io::Cursor<Vec<u8>>);
    impl Read for HeaderOnly {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            assert!(
                self.0.position() < 8,
                "oversized reply body was read before rejection"
            );
            self.0.read(buffer)
        }
    }

    #[test]
    fn tile_reply_rejects_a_larger_surface_before_reading_or_allocating_its_body() {
        let mut header = b"MIV1".to_vec();
        header.extend_from_slice(&(1024u32 * 1024 * 4 + 17).to_le_bytes());
        let error = read(&mut HeaderOnly(std::io::Cursor::new(header)), 256, 256).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn exact_rgba_and_failure_keep_the_public_wire_contract() {
        for message in [
            Message::Failure,
            Message::Rgba {
                id: 13,
                frame: api::Frame {
                    width: 2,
                    height: 1,
                    rgba: vec![255; 8],
                },
            },
        ] {
            let mut wire = Vec::new();
            api::write_message(&mut wire, &message).unwrap();
            assert_eq!(read(&mut wire.as_slice(), 2, 1).unwrap(), message);
        }
    }

    fn wire(messages: &[Message]) -> Vec<u8> {
        let mut wire = Vec::new();
        for message in messages {
            api::write_message(&mut wire, message).unwrap();
        }
        wire
    }

    #[test]
    fn a_reason_follows_failure_in_the_same_stream_and_older_workers_send_none() {
        let reason = api::FailureReason::new("Format error decoding Png").unwrap();
        let mut stream = std::io::Cursor::new(wire(&[
            Message::Failure,
            Message::FailureReason(reason.clone()),
        ]));
        assert_eq!(read(&mut stream, 2, 1).unwrap(), Message::Failure);
        assert_eq!(failure_reason(&mut stream), Some(reason));
        // A worker built with SDK 0.3.0 ends its stream after Failure.
        let mut stream = std::io::Cursor::new(wire(&[Message::Failure]));
        assert_eq!(read(&mut stream, 2, 1).unwrap(), Message::Failure);
        assert_eq!(failure_reason(&mut stream), None);
    }

    #[test]
    fn anything_but_one_valid_reason_is_ignored_without_reading_a_large_body() {
        let frame = Message::Rgba {
            id: 1,
            frame: api::Frame {
                width: 2,
                height: 1,
                rgba: vec![0; 8],
            },
        };
        assert_eq!(failure_reason(&mut wire(&[frame]).as_slice()), None);
        assert_eq!(
            failure_reason(&mut wire(&[Message::Failure]).as_slice()),
            None
        );
        let mut invalid = b"MIV1\x06\0\0\0\x0ahi/ab".to_vec();
        assert_eq!(failure_reason(&mut invalid.as_slice()), None);
        invalid.truncate(10);
        assert_eq!(failure_reason(&mut invalid.as_slice()), None);
        let mut header = b"MIV1".to_vec();
        header.extend_from_slice(&(16u32 * 1024 * 1024).to_le_bytes());
        header.push(10);
        assert_eq!(
            failure_reason(&mut HeaderOnly(std::io::Cursor::new(header.clone()))),
            None
        );
        header[8] = 4;
        assert_eq!(
            failure_reason(&mut HeaderOnly(std::io::Cursor::new(header))),
            None
        );
    }
}
