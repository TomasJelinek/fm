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
}
