//! Decoded-size budgets: 256 MiB of RGBA for a full decode, 128 MiB for a thumbnail's
//! original. The PNGs are written here: an all-black grayscale image compresses to a
//! fixed-Huffman stream of 13-bit repeat codes, so a 61-megapixel source is small.
use std::io::Cursor;

/// A grayscale 8-bit PNG of `width` x `height` black pixels.
fn black_png(width: u32, height: u32) -> Vec<u8> {
    let raw = (u64::from(width) + 1) * u64::from(height);
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 0, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &header);
    chunk(&mut png, b"IDAT", &zeros_zlib(raw));
    chunk(&mut png, b"IEND", &[]);
    png
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = png.len();
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let crc = crc32(&png[start..]);
    png.extend_from_slice(&crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
        }
    }
    !crc
}

/// A zlib stream of `length` (at least 4) zero bytes in one fixed-Huffman block.
fn zeros_zlib(length: u64) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.push(0b011, 3); // final block, fixed Huffman
    bits.huffman(0b0011_0000, 8); // literal 0
    // Copies of the previous byte (distance 1): 258 bytes as symbol 285, the rest of
    // the tail as 3..=10 bytes (symbols 257..=264), all without extra bits.
    let mut left = length - 1;
    assert!(left >= 3);
    while left > 0 {
        let run = match left {
            261.. => 258,
            14.. => 10,
            11..=13 => left - 3,
            _ => left,
        };
        if run == 258 {
            bits.huffman(0b1100_0101, 8);
        } else {
            bits.huffman(run as u32 - 2, 7);
        }
        bits.huffman(0, 5); // distance 1
        left -= run;
    }
    bits.huffman(0, 7); // end of block
    let mut zlib = vec![0x78, 0x01];
    zlib.extend(bits.finish());
    // Adler-32 of zeros: a stays 1 and b grows by 1 per byte.
    let adler = ((length % 65_521) << 16 | 1) as u32;
    zlib.extend_from_slice(&adler.to_be_bytes());
    zlib
}

#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    current: u32,
    count: u32,
}

impl Bits {
    /// Data bits, least significant first.
    fn push(&mut self, value: u32, count: u32) {
        for bit in 0..count {
            self.current |= ((value >> bit) & 1) << self.count;
            self.count += 1;
            if self.count == 8 {
                self.bytes.push(self.current as u8);
                self.current = 0;
                self.count = 0;
            }
        }
    }

    /// Huffman codes, most significant first.
    fn huffman(&mut self, code: u32, count: u32) {
        for bit in (0..count).rev() {
            self.push((code >> bit) & 1, 1);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.count > 0 {
            self.bytes.push(self.current as u8);
        }
        self.bytes
    }
}

#[test]
fn generated_png_decodes_exactly() {
    let image = midnight_image_decoder::decode(&mut Cursor::new(black_png(300, 7)), None)
        .expect("small generated PNG");
    assert_eq!(image.dimensions(), (300, 7));
    assert!(image.pixels().all(|pixel| pixel.0 == [0, 0, 0, 255]));
}

#[test]
fn originals_up_to_256_mib_of_rgba_decode_and_larger_ones_are_refused() {
    // 8000 x 8000 is 244 MiB of RGBA, above the former 128 MiB budget.
    let png = black_png(8000, 8000);
    assert!(png.len() < 1024 * 1024, "{} bytes", png.len());
    let image = midnight_image_decoder::decode(&mut Cursor::new(&png), None)
        .expect("244 MiB of RGBA is within the budget");
    assert_eq!(image.dimensions(), (8000, 8000));
    assert_eq!(image.get_pixel(7999, 7999).0, [0, 0, 0, 255]);
    drop(image);
    // A thumbnail's original keeps the 128 MiB budget of the 512 MiB thumbnail workers.
    let error = midnight_image_decoder::decode(&mut Cursor::new(&png), Some((64, 64)))
        .expect_err("a thumbnail's original has at most 128 MiB");
    assert_eq!(error.to_string(), "Image dimensions exceed budget");
    let small =
        midnight_image_decoder::decode(&mut Cursor::new(black_png(5792, 5792)), Some((64, 64)))
            .expect("128 MiB original for a thumbnail");
    assert_eq!(small.dimensions(), (64, 64));
    // One row more than 8192 x 8192 (exactly 256 MiB) is refused before decoding.
    let error = midnight_image_decoder::decode(&mut Cursor::new(black_png(8192, 8193)), None)
        .expect_err("over 256 MiB of RGBA");
    assert_eq!(error.to_string(), "Image dimensions exceed budget");
}

#[test]
fn jpeg_files_over_96_mib_are_refused_for_full_decodes_only() {
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
        .encode(&[90; 8 * 8 * 3], 8, 8, image::ExtendedColorType::Rgb8)
        .unwrap();
    // The codec ignores bytes after the end-of-image marker; they still count.
    jpeg.resize(96 << 20, 0);
    let image = midnight_image_decoder::decode(&mut Cursor::new(&jpeg), None)
        .expect("a 96 MiB JPEG file is within the limit");
    assert_eq!(image.dimensions(), (8, 8));
    jpeg.push(0);
    let error = midnight_image_decoder::decode(&mut Cursor::new(&jpeg), None)
        .expect_err("one byte over 96 MiB");
    assert_eq!(error.kind(), std::io::ErrorKind::FileTooLarge);
    assert_eq!(
        error.to_string(),
        "This JPEG file is larger than 96 MiB, the limit for JPEG images."
    );
    let thumbnail = midnight_image_decoder::decode(&mut Cursor::new(&jpeg), Some((4, 4)))
        .expect("thumbnails keep their own bounds");
    assert_eq!(thumbnail.dimensions(), (4, 4));
}
