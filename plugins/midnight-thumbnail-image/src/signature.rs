//! What the shared image decoder takes from the first bytes of the content: it picks
//! a codec by image-rs's signatures and reads anything else that starts with markup
//! as SVG. Such content belongs to the image decoder even when a PDF header occurs
//! further in, because image metadata, SVG text and comments can mention `%PDF-`.

/// Bytes at the start that image-rs's reader matches against its signatures.
const SIGNATURE_BYTES: usize = 16;

/// The signatures of image-rs 0.25.10 (`guess_format`), with their masks: a masked
/// signature compares `byte & mask`, and bytes past the mask compare whole. The tests
/// check this table against `image::guess_format`.
const RASTER: [(&[u8], &[u8]); 22] = [
    (b"\x89PNG\r\n\x1a\n", b""),
    (&[0xff, 0xd8, 0xff], b""),
    (b"GIF89a", b""),
    (b"GIF87a", b""),
    (b"RIFF\0\0\0\0WEBP", b"\xff\xff\xff\xff\0\0\0\0"),
    (b"MM\x00*", b""),
    (b"II*\x00", b""),
    (b"DDS ", b""),
    (b"BM", b""),
    (&[0, 0, 1, 0], b""),
    (b"#?RADIANCE", b""),
    (b"\0\0\0\0ftypavif", b"\xff\xff\0\0"),
    (&[0x76, 0x2f, 0x31, 0x01], b""),
    (b"qoif", b""),
    (b"P1", b""),
    (b"P2", b""),
    (b"P3", b""),
    (b"P4", b""),
    (b"P5", b""),
    (b"P6", b""),
    (b"P7", b""),
    (b"farbfeld", b""),
];

/// Whether the shared image decoder takes content that starts with `prefix`: a raster
/// signature of image-rs, or markup, which it reads as SVG.
pub(crate) fn image(prefix: &[u8]) -> bool {
    raster(prefix) || markup(prefix)
}

/// image-rs's own choice from the first bytes, as the decoder's reader makes it.
fn raster(prefix: &[u8]) -> bool {
    let start = &prefix[..prefix.len().min(SIGNATURE_BYTES)];
    RASTER.iter().any(|(signature, mask)| {
        if mask.is_empty() {
            return start.starts_with(signature);
        }
        start.len() >= signature.len()
            && start
                .iter()
                .zip(signature.iter())
                .zip(mask.iter().chain(std::iter::repeat(&0xff)))
                .all(|((byte, expected), mask)| byte & mask == *expected)
    })
}

/// The decoder's SVG start: after an optional UTF-8 byte order mark and ASCII
/// whitespace, the first byte is `<`.
fn markup(prefix: &[u8]) -> bool {
    let text = prefix.strip_prefix(b"\xef\xbb\xbf").unwrap_or(prefix);
    text.iter().find(|byte| !byte.is_ascii_whitespace()) == Some(&b'<')
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, ImageFormat, RgbaImage};
    use std::io::Cursor;

    /// Deterministic xorshift.
    fn random(state: &mut u64) -> u64 {
        *state ^= *state << 13;
        *state ^= *state >> 7;
        *state ^= *state << 17;
        *state
    }

    /// The table makes image-rs's choice: every signature with its masked bytes filled
    /// in, longer, one byte short or with one byte changed, and prefixes that follow a
    /// signature only in part.
    #[test]
    fn raster_signatures_choose_like_image_rs() {
        let mut prefixes: Vec<Vec<u8>> = vec![Vec::new()];
        for (signature, mask) in RASTER {
            for fill in [0x00, 0x5a, 0xff] {
                let full: Vec<u8> = signature
                    .iter()
                    .zip(mask.iter().chain(std::iter::repeat(&0xff)))
                    .map(|(&byte, &mask)| byte | (fill & !mask))
                    .collect();
                prefixes.push([full.as_slice(), b"%PDF-1.7 and more"].concat());
                prefixes.push(full[..full.len() - 1].to_vec());
                for index in 0..full.len() {
                    for flip in [0x01, 0x20, 0x80] {
                        let mut changed = full.clone();
                        changed[index] ^= flip;
                        prefixes.push(changed);
                    }
                }
                prefixes.push(full);
            }
        }
        let mut state = 0x2545_f491_4f6c_dd1d;
        for _ in 0..20_000 {
            let length = (random(&mut state) % 21) as usize;
            let (signature, _) = RASTER[(random(&mut state) % RASTER.len() as u64) as usize];
            let prefix = (0..length)
                .map(|index| {
                    let value = random(&mut state);
                    match signature.get(index) {
                        Some(byte) if !value.is_multiple_of(4) => *byte,
                        _ => (value >> 8) as u8,
                    }
                })
                .collect();
            prefixes.push(prefix);
        }
        let mut recognized = 0;
        for prefix in &prefixes {
            let expected =
                image::guess_format(&prefix[..prefix.len().min(SIGNATURE_BYTES)]).is_ok();
            assert_eq!(raster(prefix), expected, "{prefix:?}");
            recognized += usize::from(expected);
        }
        assert!(recognized > 1000, "{recognized} of {}", prefixes.len());
    }

    /// Every raster format that the shared decoder reads is recognized from the start of
    /// image-rs's own encoding of it.
    #[test]
    fn encoded_images_of_the_decoder_formats_are_recognized() {
        let image = DynamicImage::ImageRgba8(RgbaImage::from_pixel(
            16,
            8,
            image::Rgba([10, 200, 30, 255]),
        ));
        for format in [
            ImageFormat::Png,
            ImageFormat::Jpeg,
            ImageFormat::Gif,
            ImageFormat::WebP,
            ImageFormat::Bmp,
            ImageFormat::Ico,
            ImageFormat::Tiff,
        ] {
            let image = if format == ImageFormat::Jpeg {
                DynamicImage::ImageRgb8(image.to_rgb8())
            } else {
                image.clone()
            };
            let mut encoded = Cursor::new(Vec::new());
            image.write_to(&mut encoded, format).unwrap();
            assert!(super::image(encoded.get_ref()), "{format:?}");
        }
    }

    #[test]
    fn markup_after_a_byte_order_mark_and_whitespace_is_svg() {
        for svg in [
            &b"<svg/>"[..],
            b"<?xml version=\"1.0\"?>",
            b"\xef\xbb\xbf\n  <svg/>",
            b" \t\r\n<!-- %PDF-1.7 -->",
        ] {
            assert!(image(svg), "{svg:?}");
        }
        for other in [
            &b""[..],
            b"   ",
            b"%PDF-1.7\n<svg/>",
            b"\xef\xbb\xbf%PDF-1.7",
            b"\xef\xbb\xbf\xef\xbb\xbf<svg/>",
            b"# comment\n<svg/>",
        ] {
            assert!(!image(other), "{other:?}");
        }
    }
}
