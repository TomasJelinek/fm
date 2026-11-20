use image::{DynamicImage, ImageFormat, RgbaImage};
use midnight_thumbnail_sdk::api::{Frame, Request};
use std::io::{self, Cursor, Read, Seek, SeekFrom};

/// A PDF with one page per `(MediaBox and other page entries, content stream)` and an
/// optional extra trailer entry.
fn pdf_with(pages: &[(&str, &str)], trailer: &str) -> Vec<u8> {
    let ids = (0..pages.len())
        .map(|index| format!("{} 0 R", 3 + 2 * index))
        .collect::<Vec<_>>()
        .join(" ");
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        format!("<< /Type /Pages /Kids [{ids}] /Count {} >>", pages.len()),
    ];
    for (index, (dimensions, content)) in pages.iter().enumerate() {
        objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox {dimensions} /Resources << /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >> >> /Contents {} 0 R >>", 4 + index * 2));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = vec![0];
    for (index, object) in objects.iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len()).as_bytes());
    for offset in offsets.iter().skip(1) {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R {trailer}>>\nstartxref\n{xref}\n%%EOF\n",
            offsets.len()
        )
        .as_bytes(),
    );
    bytes
}

fn pdf(pages: &[(&str, &str)]) -> Vec<u8> {
    pdf_with(pages, "")
}

fn request(width: u32, height: u32) -> Request {
    Request {
        id: 11,
        source_length: 0,
        width,
        height,
        frame: 0,
    }
}

fn thumbnail(bytes: &[u8], width: u32, height: u32) -> io::Result<Frame> {
    midnight_thumbnail_image::decode(&request(width, height), &mut Cursor::new(bytes))
}

fn pixels(frame: &Frame) -> &[[u8; 4]] {
    frame.rgba.as_chunks::<4>().0
}

#[test]
fn the_first_page_fills_the_request_with_its_aspect_ratio() {
    let red = "1 0 0 rg 0 0 400 400 re f";
    // Portrait, landscape and square pages in a square request; a tiny request.
    for (page, (width, height), expected) in [
        ("[0 0 200 300]", (96, 96), (64, 96)),
        ("[0 0 400 200]", (96, 96), (96, 48)),
        ("[0 0 300 300]", (128, 128), (128, 128)),
        ("[0 0 200 300]", (1, 1), (1, 1)),
        // A small page is drawn at the requested size: it is vector content.
        ("[0 0 20 30]", (512, 512), (341, 512)),
    ] {
        let frame = thumbnail(&pdf(&[(page, red)]), width, height).unwrap();
        assert_eq!((frame.id, frame.frame), (11, 0));
        assert_eq!((frame.width, frame.height), expected, "{page}");
        assert_eq!(frame.rgba.len(), (expected.0 * expected.1 * 4) as usize);
        assert!(
            pixels(&frame)
                .iter()
                .all(|pixel| *pixel == [255, 0, 0, 255]),
            "{page}: {:?}",
            &pixels(&frame)[..4]
        );
    }
}

#[test]
fn only_page_one_is_drawn() {
    let bytes = pdf(&[
        ("[0 0 20 30]", "0 0 1 rg 0 0 20 30 re f"),
        ("[0 0 40 20]", "1 0 0 rg 0 0 40 20 re f"),
        ("[0 0 40 20]", "0 1 0 rg 0 0 40 20 re f"),
    ]);
    let frame = thumbnail(&bytes, 20, 20).unwrap();
    assert_eq!((frame.width, frame.height), (13, 20));
    assert!(
        pixels(&frame)
            .iter()
            .all(|pixel| *pixel == [0, 0, 255, 255])
    );
}

#[test]
fn text_is_dark_on_an_opaque_white_page() {
    let bytes = pdf(&[("[0 0 120 160]", "BT /F1 24 Tf 10 120 Td (Hello PDF) Tj ET")]);
    let frame = thumbnail(&bytes, 120, 160).unwrap();
    assert_eq!((frame.width, frame.height), (120, 160));
    let all = pixels(&frame);
    assert!(
        all.iter().all(|pixel| pixel[3] == 255),
        "the page is opaque"
    );
    let white = all.iter().filter(|pixel| **pixel == [255; 4]).count();
    let dark = all
        .iter()
        .filter(|pixel| pixel[0] < 100 && pixel[1] < 100 && pixel[2] < 100)
        .count();
    assert!(white > all.len() / 2, "{white} white pixels");
    assert!(dark > 50, "{dark} dark text pixels");
    // A blank page is white paper, not transparency.
    let blank = thumbnail(&pdf(&[("[0 0 50 50]", "")]), 64, 64).unwrap();
    assert!(pixels(&blank).iter().all(|pixel| *pixel == [255; 4]));
}

#[test]
fn rotated_and_cropped_pages_keep_their_visible_geometry() {
    // Crop to 30 x 20 points, then rotate a quarter turn: a 20 x 30 portrait page.
    let bytes = pdf(&[(
        "[0 0 40 20] /CropBox [10 0 40 20] /Rotate 90",
        "0 1 0 rg 0 0 40 20 re f",
    )]);
    let frame = thumbnail(&bytes, 60, 60).unwrap();
    assert_eq!((frame.width, frame.height), (40, 60));
    assert!(
        pixels(&frame)
            .iter()
            .all(|pixel| *pixel == [0, 255, 0, 255])
    );
}

#[test]
fn a_header_after_leading_bytes_is_still_a_pdf() {
    let document = pdf(&[("[0 0 20 20]", "1 0 0 rg 0 0 20 20 re f")]);
    let mut shifted = vec![b'#'; 100];
    shifted.push(b'\n');
    shifted.extend_from_slice(&document);
    // The parser repairs the shifted cross-reference offsets.
    let frame = thumbnail(&shifted, 20, 20).unwrap();
    assert!(
        pixels(&frame)
            .iter()
            .all(|pixel| *pixel == [255, 0, 0, 255])
    );
    // A header beyond the first kilobyte is not a PDF header: the image path refuses it.
    let mut late = vec![b'#'; 1100];
    late.extend_from_slice(&document);
    let error = thumbnail(&late, 20, 20).unwrap_err();
    assert_eq!(
        error.to_string(),
        "The content is not a recognized image format"
    );
}

#[test]
fn damaged_password_protected_and_unsupported_documents_fail() {
    let encrypted = pdf_with(
        &[("[0 0 20 20]", "1 0 0 rg 0 0 20 20 re f")],
        &format!(
            "/Encrypt << /Filter /Standard /V 2 /R 3 /Length 128 /O <{}> /U <{}> /P -4 >> /ID [<{}> <{}>] ",
            "11".repeat(32),
            "22".repeat(32),
            "33".repeat(16),
            "33".repeat(16)
        ),
    );
    let mut truncated = pdf(&[("[0 0 20 20]", "1 0 0 rg 0 0 20 20 re f")]);
    truncated.truncate(40);
    let damaged = (
        io::ErrorKind::InvalidData,
        "The PDF structure is invalid or incomplete",
    );
    for (name, bytes, (kind, reason)) in [
        (
            "password",
            encrypted,
            (
                io::ErrorKind::Unsupported,
                "The document needs a password or uses unsupported encryption",
            ),
        ),
        ("truncated", truncated, damaged),
        ("header only", b"%PDF-".to_vec(), damaged),
        (
            "garbage",
            b"%PDF-1.7\n\x00\xff garbage \x01\x02".to_vec(),
            damaged,
        ),
        (
            "no pages",
            pdf(&[]),
            (io::ErrorKind::InvalidData, "PDF has no pages"),
        ),
        (
            "huge page",
            pdf(&[("[0 0 999999 20]", "")]),
            (io::ErrorKind::InvalidData, "Invalid PDF page dimensions"),
        ),
        // The viewer refuses a page whose image cannot be decoded; so does its preview.
        (
            "undecodable image",
            pdf(&[(
                "[0 0 20 20]",
                "q 20 0 0 20 0 0 cm BI /W 4 /H 4 /BPC 8 /CS /RGB /F /DCT ID notajpeg EI Q",
            )]),
            (
                io::ErrorKind::Unsupported,
                "PDF page contains unsupported content",
            ),
        ),
    ] {
        let error = thumbnail(&bytes, 64, 64).expect_err(name);
        assert_eq!(
            (error.kind(), error.to_string()),
            (kind, reason.to_owned()),
            "{name}"
        );
    }
}

/// CRC-32 (IEEE) of a PNG chunk's type and data.
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

/// A small encoded image of `format`.
fn encoded(format: ImageFormat) -> Vec<u8> {
    let image = RgbaImage::from_fn(40, 30, |x, y| {
        image::Rgba([x as u8 * 6, y as u8 * 8, 90, 255])
    });
    let image = if format == ImageFormat::Jpeg {
        DynamicImage::ImageRgb8(DynamicImage::ImageRgba8(image).to_rgb8())
    } else {
        DynamicImage::ImageRgba8(image)
    };
    let mut encoded = Cursor::new(Vec::new());
    image.write_to(&mut encoded, format).unwrap();
    encoded.into_inner()
}

/// `png` with a `tEXt` chunk right after its header, where exporters write metadata.
fn png_with_text(png: &[u8], text: &[u8]) -> Vec<u8> {
    // The signature and the IHDR chunk: length, type, 13 data bytes and CRC.
    let header = 8 + 4 + 4 + 13 + 4;
    let mut chunk = [b"tEXt".as_slice(), text].concat();
    let crc = crc32(&chunk);
    chunk.extend_from_slice(&crc.to_be_bytes());
    let length = u32::try_from(text.len()).unwrap().to_be_bytes();
    [
        &png[..header],
        length.as_slice(),
        chunk.as_slice(),
        &png[header..],
    ]
    .concat()
}

/// `jpeg` with a comment segment right after its start-of-image marker.
fn jpeg_with_comment(jpeg: &[u8], text: &[u8]) -> Vec<u8> {
    let length = u16::try_from(text.len() + 2).unwrap().to_be_bytes();
    [
        &jpeg[..2],
        b"\xff\xfe".as_slice(),
        length.as_slice(),
        text,
        &jpeg[2..],
    ]
    .concat()
}

/// Image metadata and SVG text can mention a PDF header in their first kilobyte: such
/// content still belongs to the image decoder, and its thumbnail is the decoder's.
#[test]
fn a_pdf_header_in_image_metadata_or_svg_text_leaves_the_image_to_the_image_decoder() {
    let mention = b"Comment\0Exported from report.pdf (%PDF-1.7)";
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"30\">\
               <title>%PDF-1.7 export</title><!-- %PDF-1.4 -->\
               <rect width=\"40\" height=\"30\" fill=\"#0a0\"/></svg>";
    for (name, bytes) in [
        (
            "PNG with a tEXt chunk",
            png_with_text(&encoded(ImageFormat::Png), mention),
        ),
        (
            "JPEG with a comment",
            jpeg_with_comment(&encoded(ImageFormat::Jpeg), mention),
        ),
        ("SVG with a title and a comment", svg.as_bytes().to_vec()),
        (
            "SVG after a byte order mark and blank lines",
            format!("\u{feff}\n\n{svg}").into_bytes(),
        ),
    ] {
        assert!(
            bytes[..bytes.len().min(1024)]
                .windows(5)
                .any(|window| window == b"%PDF-"),
            "{name} mentions a PDF header in its first kilobyte"
        );
        let frame = thumbnail(&bytes, 64, 64).unwrap_or_else(|error| panic!("{name}: {error}"));
        let direct =
            midnight_image_decoder::decode(&mut Cursor::new(&bytes), Some((64, 64))).unwrap();
        assert_eq!((frame.width, frame.height), direct.dimensions(), "{name}");
        assert_eq!(frame.rgba, direct.into_raw(), "{name}");
    }
}

/// A source larger than the PDF budget whose bytes past the sniffed prefix must
/// never be read.
struct Oversized {
    position: u64,
}

impl Read for Oversized {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        assert!(
            self.position < 256 * 1024,
            "bytes past the prefix were read"
        );
        let header = b"%PDF-1.7\n";
        for (offset, byte) in output.iter_mut().enumerate() {
            let at = self.position as usize + offset;
            *byte = header.get(at).copied().unwrap_or(b' ');
        }
        self.position += output.len() as u64;
        Ok(output.len())
    }
}

impl Seek for Oversized {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.position = match from {
            SeekFrom::Start(position) => position,
            SeekFrom::End(delta) => {
                (midnight_thumbnail_image::MAX_PDF_SOURCE_BYTES + 1).saturating_add_signed(delta)
            }
            SeekFrom::Current(delta) => self.position.saturating_add_signed(delta),
        };
        Ok(self.position)
    }
}

#[test]
fn a_document_over_the_budget_fails_before_it_is_read() {
    let error = midnight_thumbnail_image::decode(&request(64, 64), &mut Oversized { position: 0 })
        .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::InvalidData);
}

#[test]
fn images_still_decode_through_the_sniffed_prefix() {
    // Larger than the prefix, so decoding also reads past it.
    let noise = RgbaImage::from_fn(400, 300, |x, y| {
        let value = (x.wrapping_mul(7919) ^ y.wrapping_mul(104_729)).wrapping_mul(2_654_435_761);
        image::Rgba([value as u8, (value >> 8) as u8, (value >> 16) as u8, 255])
    });
    let mut encoded = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(noise)
        .write_to(&mut encoded, ImageFormat::Png)
        .unwrap();
    let encoded = encoded.into_inner();
    assert!(encoded.len() > 256 * 1024, "{} bytes", encoded.len());
    let frame = thumbnail(&encoded, 64, 64).unwrap();
    assert_eq!((frame.width, frame.height), (64, 48));
    let direct =
        midnight_image_decoder::decode(&mut Cursor::new(&encoded), Some((64, 64))).unwrap();
    assert_eq!(frame.rgba, direct.into_raw());
}
