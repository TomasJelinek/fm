//! AVIF's container stages its coded items, unlike the streaming raster codecs.
//! Bound this format's input and both native frame allocations before decoding.
use image::codecs::avif::AvifDecoder;
use std::io::{self, Cursor, Read};

pub(crate) const MAX_ENCODED_BYTES: u64 = 32 * 1024 * 1024;
// Worst case: 16-bit 4:4:4 color plus 16-bit alpha uses eight bytes per pixel.
// Native references, parser staging and conversion scratch remain process-contained.
const MAX_FRAME_PIXELS: u32 = (super::MAX_DECODED_BYTES / 8) as u32;

pub(crate) fn decoder<R: Read>(reader: R) -> io::Result<AvifDecoder<Cursor<Vec<u8>>>> {
    with_budgets(reader, MAX_ENCODED_BYTES, MAX_FRAME_PIXELS)
}

fn with_budgets<R: Read>(
    mut reader: R,
    encoded_bytes: u64,
    frame_pixels: u32,
) -> io::Result<AvifDecoder<Cursor<Vec<u8>>>> {
    // mp4parse's normal mode tolerates some trailing-box read errors. Enforce
    // this scoped AVIF staging cap before parsing so it cannot be swallowed.
    let mut encoded = Vec::new();
    reader
        .by_ref()
        .take(encoded_bytes)
        .read_to_end(&mut encoded)?;
    let mut probe = [0];
    if reader.read(&mut probe)? != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "AVIF encoded input exceeds budget",
        ));
    }
    let mut settings = dav1d::Settings::new();
    settings.set_n_threads(1);
    settings.set_max_frame_delay(1);
    settings.set_all_layers(false);
    settings.set_strict_std_compliance(true);
    settings.set_frame_size_limit(frame_pixels);
    AvifDecoder::new_with_settings(Cursor::new(encoded), &settings).map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageDecoder;
    use std::io::Cursor;

    const WHITE: &[u8] = include_bytes!("../tests/fixtures/white_1x1.avif");
    const ALPHA: &[u8] = include_bytes!("../tests/fixtures/draw_points_idat.avif");

    #[test]
    fn primary_frame_is_limited_before_native_allocation() {
        assert_eq!(
            with_budgets(Cursor::new(ALPHA), 1024, 363)
                .unwrap()
                .dimensions(),
            (33, 11)
        );
        assert!(with_budgets(Cursor::new(ALPHA), 1024, 362).is_err());
    }

    #[test]
    fn alpha_frame_has_the_same_native_allocation_limit() {
        // Retain the real 33x11 alpha bitstream but replace the primary coded
        // item with the independently valid 1x1 white bitstream. The container
        // remains parseable; the native alpha guard must reject its larger plane.
        let mut input = ALPHA.to_vec();
        let white_coded = &WHITE[282..];
        input.splice(456..498, white_coded.iter().copied());
        let meta_size = (input.len() - 32) as u32;
        input[32..36].copy_from_slice(&meta_size.to_be_bytes());
        input[426..430].copy_from_slice(&(30 + white_coded.len() as u32).to_be_bytes());
        input[119..123].copy_from_slice(&(white_coded.len() as u32).to_be_bytes());
        assert_eq!(
            with_budgets(Cursor::new(&input), 1024, 363)
                .unwrap()
                .dimensions(),
            (1, 1)
        );
        assert!(with_budgets(Cursor::new(&input), 1024, 4).is_err());
    }

    #[test]
    fn trailing_unknown_box_cannot_hide_excess_encoded_input() {
        let mut input = WHITE.to_vec();
        input.extend_from_slice(&100u32.to_be_bytes());
        input.extend_from_slice(b"free");
        input.extend_from_slice(&[0; 92]);
        assert!(with_budgets(Cursor::new(&input), input.len() as u64, 1).is_ok());
        assert!(with_budgets(Cursor::new(&input), WHITE.len() as u64 + 9, 1).is_err());
    }

    #[test]
    fn encoded_limit_is_explicit_and_exact_size_input_is_accepted() {
        assert!(with_budgets(Cursor::new(WHITE), WHITE.len() as u64, 1).is_ok());
        assert!(with_budgets(Cursor::new(WHITE), WHITE.len() as u64 - 1, 1).is_err());
        let error = match with_budgets(Cursor::new([1, 2, 3]), 2, 1) {
            Ok(_) => panic!("excess input was accepted"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
    }
}
