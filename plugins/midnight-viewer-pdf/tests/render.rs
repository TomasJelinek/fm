use midnight_image_viewer_sdk::{
    Document,
    api::{Metadata, Viewport},
};
use std::io::{self, Cursor, Read, Seek, SeekFrom};

fn pdf(pages: &[(&str, &str)]) -> Vec<u8> {
    pdf_with(pages, "")
}

/// A generated PDF whose trailer also holds `trailer` entries.
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

fn viewport() -> Viewport {
    Viewport {
        width: 20,
        height: 20,
        scale: 1.0,
        origin_x: 0.0,
        origin_y: 0.0,
    }
}

#[test]
fn renders_later_pages_with_independent_dimensions_and_colors() {
    let bytes = pdf(&[
        ("[0 0 20 30]", "1 0 0 rg 0 0 20 30 re f"),
        ("[0 0 40 20]", "0 0 1 rg 0 0 40 20 re f"),
    ]);
    let document = midnight_viewer_pdf::decode(&mut Cursor::new(bytes)).unwrap();
    assert_eq!(
        document.metadata().pages,
        vec![
            Metadata {
                width: 20,
                height: 30
            },
            Metadata {
                width: 40,
                height: 20
            }
        ]
    );
    assert_eq!(
        &document.render(0, viewport()).unwrap().rgba[..4],
        &[255, 0, 0, 255]
    );
    assert_eq!(
        &document.render(1, viewport()).unwrap().rgba[..4],
        &[0, 0, 255, 255]
    );
    assert!(document.render(2, viewport()).is_err());
}

#[test]
fn zoom_and_scroll_render_vector_detail_at_the_requested_origin() {
    let bytes = pdf(&[(
        "[0 0 40 20]",
        "1 0 0 rg 0 0 20 20 re f 0 0 1 rg 20 0 20 20 re f",
    )]);
    let document = midnight_viewer_pdf::decode(&mut Cursor::new(bytes)).unwrap();
    let red = document
        .render(
            0,
            Viewport {
                scale: 4.0,
                ..viewport()
            },
        )
        .unwrap();
    assert!(
        red.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [255, 0, 0, 255])
    );
    let blue = document
        .render(
            0,
            Viewport {
                scale: 4.0,
                origin_x: 20.0,
                ..viewport()
            },
        )
        .unwrap();
    assert!(
        blue.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [0, 0, 255, 255])
    );
}

#[test]
fn standard_fonts_are_embedded_and_blank_pages_are_white() {
    let bytes = pdf(&[
        ("[0 0 120 40]", "BT /F1 20 Tf 5 10 Td (Hello PDF) Tj ET"),
        ("[0 0 20 20]", ""),
    ]);
    let document = midnight_viewer_pdf::decode(&mut Cursor::new(bytes)).unwrap();
    let text = document
        .render(
            0,
            Viewport {
                width: 120,
                height: 40,
                ..viewport()
            },
        )
        .unwrap();
    assert!(
        text.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[0] < 100)
            .count()
            > 100
    );
    let blank = document.render(1, viewport()).unwrap();
    assert!(
        blank
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [255; 4])
    );
}

/// Text is drawn with antialiased edges at the exact requested scale: the outline of each
/// glyph has intermediate grey levels between the black stems and the white page.
#[test]
fn glyph_edges_are_antialiased_at_the_requested_physical_scale() {
    let bytes = pdf(&[("[0 0 120 40]", "BT /F1 20 Tf 5 12 Td (Hello PDF) Tj ET")]);
    let document = midnight_viewer_pdf::decode(&mut Cursor::new(bytes)).unwrap();
    for scale in [1.0, 2.5, 4.0] {
        let frame = document
            .render(
                0,
                Viewport {
                    width: (120.0 * scale) as u32,
                    height: (40.0 * scale) as u32,
                    scale,
                    ..viewport()
                },
            )
            .unwrap();
        let pixels = frame.rgba.as_chunks::<4>().0;
        let ink = pixels.iter().filter(|pixel| pixel[0] < 250).count();
        let solid = pixels.iter().filter(|pixel| pixel[0] < 10).count();
        let edge = pixels
            .iter()
            .filter(|pixel| (10..250).contains(&pixel[0]))
            .count();
        assert!(
            pixels.iter().all(|pixel| pixel[3] == 255),
            "page pixels are opaque"
        );
        assert!(solid > 0, "scale {scale}: no solid stems");
        // Aliased text would have no grey edge at all; antialiased text at these sizes
        // has a grey rim on every stroke.
        assert!(
            edge * 5 > ink,
            "scale {scale}: only {edge} of {ink} ink pixels are grey edge pixels"
        );
    }
}

/// The host renders a page in bands of tiles. Adjacent bands meet without a seam: they
/// equal one render of the whole area, also when the second band reuses the page's caches.
#[test]
fn page_bands_join_without_seams_and_reuse_bounded_caches() {
    let bytes = pdf(&[
        (
            "[0 0 200 120]",
            "0.5 w 10 10 m 190 110 l S BT /F1 14 Tf 12 60 Td (Band seams 0123) Tj ET",
        ),
        ("[0 0 200 120]", "0 0 1 rg 20 20 160 80 re f"),
        ("[0 0 200 120]", ""),
    ]);
    let document = midnight_viewer_pdf::decode(&mut Cursor::new(bytes)).unwrap();
    let scale = 2.75;
    let whole = document
        .render(
            0,
            Viewport {
                width: 550,
                height: 330,
                scale,
                ..viewport()
            },
        )
        .unwrap();
    let mut joined = Vec::new();
    for (top, height) in [(0u32, 256u32), (256, 74)] {
        let band = document
            .render(
                0,
                Viewport {
                    width: 550,
                    height,
                    scale,
                    origin_x: 0.0,
                    origin_y: f64::from(top) / scale,
                },
            )
            .unwrap();
        joined.extend_from_slice(&band.rgba);
    }
    assert_eq!(joined.len(), whole.rgba.len());
    let differing = joined
        .iter()
        .zip(&whole.rgba)
        .filter(|(band, whole)| band.abs_diff(**whole) > 2)
        .count();
    assert!(
        differing * 1000 < whole.rgba.len(),
        "{differing} channel values differ between bands and one render"
    );
    assert_eq!(document.retained_caches(), vec![(0, scale)]);
    for page in [1, 2, 0] {
        document.render(page, viewport()).unwrap();
    }
    // Two pages are retained, least recently used first; the third evicted the oldest.
    assert_eq!(document.retained_caches(), vec![(2, 1.0), (0, 1.0)]);
}

#[test]
fn malformed_documents_and_invalid_geometry_fail() {
    for bytes in [
        vec![],
        b"not a PDF".to_vec(),
        b"%PDF-1.7\ntruncated".to_vec(),
    ] {
        assert!(midnight_viewer_pdf::decode(&mut Cursor::new(bytes)).is_err());
    }
    assert!(
        midnight_viewer_pdf::decode(&mut Cursor::new(pdf(&[("[0 0 999999 20]", "")]))).is_err()
    );
    let document =
        midnight_viewer_pdf::decode(&mut Cursor::new(pdf(&[("[0 0 20 20]", "")]))).unwrap();
    assert!(
        document
            .render(
                0,
                Viewport {
                    width: 4097,
                    ..viewport()
                }
            )
            .is_err()
    );
    assert!(
        document
            .render(
                0,
                Viewport {
                    scale: f64::NAN,
                    ..viewport()
                }
            )
            .is_err()
    );
}

struct Oversized;
impl Read for Oversized {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("Oversized source must not be read")
    }
}
impl Seek for Oversized {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        Ok(128 * 1024 * 1024 + 1)
    }
}

#[test]
fn rejects_encoded_source_budget_before_allocation_or_read() {
    assert!(midnight_viewer_pdf::decode(&mut Oversized).is_err());
}

/// Decode errors reach the viewer's status line as the failure's reason.
#[test]
fn rejections_say_why_in_short_path_free_words() {
    let reason = |error: io::Error| (error.kind(), error.to_string());
    let encrypted = pdf_with(
        &[("[0 0 20 20]", "")],
        &format!(
            "/Encrypt << /Filter /Standard /V 2 /R 3 /Length 128 /O <{}> /U <{}> /P -4 >> \
             /ID [<{}> <{}>] ",
            "11".repeat(32),
            "22".repeat(32),
            "33".repeat(16),
            "33".repeat(16)
        ),
    );
    for (bytes, kind, text) in [
        (Vec::new(), io::ErrorKind::InvalidData, "The file is empty"),
        (
            b"not a PDF".to_vec(),
            io::ErrorKind::InvalidData,
            "The PDF structure is invalid or incomplete",
        ),
        (
            encrypted,
            io::ErrorKind::Unsupported,
            "The document needs a password or uses unsupported encryption",
        ),
        (
            pdf(&[("[0 0 999999 20]", "")]),
            io::ErrorKind::Unsupported,
            "A page is larger than this viewer supports",
        ),
    ] {
        let error = midnight_viewer_pdf::decode(&mut Cursor::new(bytes))
            .err()
            .expect("a rejected document");
        assert_eq!(reason(error), (kind, text.to_owned()));
    }
    let error = midnight_viewer_pdf::decode(&mut Oversized)
        .err()
        .expect("an oversized document");
    assert_eq!(
        reason(error),
        (
            io::ErrorKind::Unsupported,
            "The document is larger than the 128 MiB this viewer reads".to_owned()
        )
    );
}

#[test]
fn rotated_cropped_pages_and_outside_viewport_keep_geometry_and_alpha() {
    let bytes = pdf(&[(
        "[0 0 40 20] /CropBox [10 0 40 20] /Rotate 90",
        "0 1 0 rg 0 0 40 20 re f",
    )]);
    let document = midnight_viewer_pdf::decode(&mut Cursor::new(bytes)).unwrap();
    assert_eq!(
        document.metadata().pages,
        vec![Metadata {
            width: 20,
            height: 30
        }]
    );
    let frame = document
        .render(
            0,
            Viewport {
                width: 40,
                height: 40,
                origin_x: -10.0,
                origin_y: -5.0,
                ..viewport()
            },
        )
        .unwrap();
    assert_eq!(&frame.rgba[..4], &[0, 0, 0, 0]);
    let interior = (10 * 40 + 15) * 4;
    assert_eq!(&frame.rgba[interior..interior + 4], &[0, 255, 0, 255]);
    let outside = (39 * 40 + 39) * 4;
    assert_eq!(&frame.rgba[outside..outside + 4], &[0, 0, 0, 0]);
}
