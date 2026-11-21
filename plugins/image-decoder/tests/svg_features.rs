//! Static SVG features that the decoder draws: markers, text, local references, paint
//! servers, clip paths, masks, filters, `switch` fallbacks and embedded raster images.
use image::{ImageFormat, Rgba, RgbaImage};
use std::io::Cursor;

fn decode(svg: &str) -> midnight_image_decoder::Decoded {
    midnight_image_decoder::decode_with_note(&mut Cursor::new(svg), None)
        .unwrap_or_else(|error| panic!("{error}: {svg:.200}"))
}

/// A diagram with the features of a reported file that was refused: a background
/// rectangle, an arrow marker with `orient="auto-start-reverse"` referred to as
/// `url(#arrow)` from both ends of each connector, and text in a font-family list of
/// families that the decoder does not have.
fn diagram(text: bool, orient: &str) -> String {
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="640" height="400" viewBox="0 0 640 400" font-family="Segoe UI, Helvetica, Arial, sans-serif" font-size="12"><rect width="640" height="400" fill="#ffffff"/><defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="{orient}"><path d="M0,0 L10,5 L0,10 z" fill="#202020"/></marker></defs>"##
    );
    for row in 0..12 {
        let y = 20 + row * 30;
        svg += &format!(
            r##"<rect x="20" y="{y}" width="200" height="24" fill="#e8f0fe" stroke="#1a73e8"/>"##
        );
        if text {
            svg += &format!(
                r##"<text x="30" y="{}" fill="#202020">Step {row}: synthetic label</text>"##,
                y + 17
            );
        }
        svg += &format!(
            r##"<path d="M240,{0} L420,{0}" stroke="#202020" fill="none" marker-start="url(#arrow)" marker-end="url(#arrow)"/>"##,
            y + 12
        );
    }
    svg + "</svg>"
}

fn dark(pixel: &Rgba<u8>) -> bool {
    pixel[3] == 255 && u32::from(pixel[0]) + u32::from(pixel[1]) + u32::from(pixel[2]) < 200
}

/// Dark pixels inside the boxes, where only text is dark.
fn text_pixels(image: &RgbaImage) -> usize {
    (0..12)
        .flat_map(|row| {
            let top = 21 + row * 30;
            (top..top + 22).flat_map(|y| (22..218).map(move |x| (x, y)))
        })
        .filter(|&(x, y)| dark(image.get_pixel(x, y)))
        .count()
}

#[test]
fn reported_diagram_features_draw_arrowheads_and_text() {
    let decoded = decode(&diagram(true, "auto-start-reverse"));
    let image = &decoded.pixels;
    assert_eq!(image.dimensions(), (640, 400));
    assert_eq!(decoded.note, None);
    // The fallback family draws every label; without text the boxes have no dark pixel.
    let labels = text_pixels(image);
    assert!(labels > 12 * 60, "{labels} text pixels");
    assert_eq!(
        text_pixels(&decode(&diagram(false, "auto-start-reverse")).pixels),
        0
    );
    for row in 0..12 {
        let y = 20 + row * 30 + 12 - 2;
        // Two pixels off the one-pixel line lie inside the arrowheads only: the end
        // points right at x=420, the reversed start points left at x=240.
        assert!(dark(image.get_pixel(416, y)), "end arrowhead, row {row}");
        assert!(
            dark(image.get_pixel(244, y)),
            "reversed start arrowhead, row {row}"
        );
        assert!(!dark(image.get_pixel(330, y)), "line only, row {row}");
    }
    // Plain `auto` would point the start marker along the path, away from x=244.
    let forward = decode(&diagram(true, "auto")).pixels;
    assert!(!dark(forward.get_pixel(244, 30)));
    assert!(dark(forward.get_pixel(236, 30)));
}

#[test]
fn paint_servers_clip_paths_masks_and_filters_are_drawn() {
    let decoded = decode(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="20"><defs>
<pattern id="checks" width="4" height="4" patternUnits="userSpaceOnUse"><rect width="2" height="2" fill="#ff0000"/><rect x="2" y="2" width="2" height="2" fill="#ff0000"/></pattern>
<clipPath id="left" clipPathUnits="objectBoundingBox"><rect width="0.5" height="1"/></clipPath>
<mask id="top"><rect width="100" height="10" fill="white"/></mask>
<filter id="flood" x="0" y="0" width="1" height="1"><feFlood flood-color="#00ff00"/></filter>
<radialGradient id="glow"><stop offset="0" stop-color="#ffff00"/><stop offset="1" stop-color="#ffff00" stop-opacity="0"/></radialGradient>
</defs>
<rect width="20" height="20" fill="url(#checks)"/>
<rect x="20" width="20" height="20" fill="#0000ff" clip-path="url(#left)"/>
<rect x="40" width="20" height="20" fill="#0000ff" mask="url(#top)"/>
<rect x="60" width="20" height="20" fill="#000000" filter="url(#flood)"/>
<circle cx="90" cy="10" r="10" fill="url(#glow)"/>
</svg>"##,
    );
    let image = &decoded.pixels;
    assert_eq!(decoded.note, None);
    assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
    assert_eq!(image.get_pixel(2, 0).0[3], 0);
    assert_eq!(image.get_pixel(2, 2).0, [255, 0, 0, 255]);
    assert_eq!(image.get_pixel(24, 10).0, [0, 0, 255, 255]);
    assert_eq!(image.get_pixel(35, 10).0[3], 0);
    assert_eq!(image.get_pixel(50, 4).0, [0, 0, 255, 255]);
    assert_eq!(image.get_pixel(50, 15).0[3], 0);
    assert_eq!(image.get_pixel(70, 10).0, [0, 255, 0, 255]);
    assert!(image.get_pixel(90, 10).0[3] > 200);
    assert!(image.get_pixel(98, 10).0[3] < 100);
}

#[test]
fn a_switch_draws_its_svg_fallback_instead_of_html() {
    let drawn = decode(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><switch><foreignObject requiredFeatures="http://www.w3.org/TR/SVG11/feature#Extensibility" width="20" height="20"><div xmlns="http://www.w3.org/1999/xhtml">HTML</div></foreignObject><rect width="20" height="20" fill="#00ff00"/></switch></svg>"##,
    );
    assert_eq!(drawn.pixels.get_pixel(10, 10).0, [0, 255, 0, 255]);
    assert_eq!(drawn.note, None);
    let alone = decode(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><foreignObject width="20" height="20"><div xmlns="http://www.w3.org/1999/xhtml">HTML</div></foreignObject><rect width="5" height="5" fill="#00ff00"/></svg>"##,
    );
    assert_eq!(alone.pixels.get_pixel(10, 10).0[3], 0);
    assert_eq!(alone.pixels.get_pixel(2, 2).0, [0, 255, 0, 255]);
    assert_eq!(
        alone.note.as_deref(),
        Some("Not shown: HTML content (foreignObject).")
    );
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let value = chunk.iter().enumerate().fold(0u32, |value, (index, byte)| {
            value | u32::from(*byte) << (16 - 8 * index)
        });
        for index in 0..4 {
            text.push(if index <= chunk.len() {
                TABLE[(value >> (18 - 6 * index) & 63) as usize] as char
            } else {
                '='
            });
        }
    }
    text
}

fn encoded(format: ImageFormat, color: [u8; 3]) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(8, 8, image::Rgb(color)))
        .write_to(&mut bytes, format)
        .unwrap();
    bytes.into_inner()
}

fn embedding(mime: &str, data: &[u8]) -> String {
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><image width="10" height="10" href="data:{mime};base64,{}"/><rect x="10" width="10" height="10" fill="#0000ff"/></svg>"##,
        base64(data)
    )
}

#[test]
fn embedded_png_jpeg_gif_and_webp_are_drawn_through_the_raster_path() {
    for (format, mime) in [
        (ImageFormat::Png, "image/png"),
        (ImageFormat::Jpeg, "image/jpeg"),
        (ImageFormat::Gif, "image/gif"),
        (ImageFormat::WebP, "image/webp"),
    ] {
        let decoded = decode(&embedding(mime, &encoded(format, [250, 20, 10])));
        let pixel = decoded.pixels.get_pixel(5, 5).0;
        assert!(
            pixel[0] > 230 && pixel[1] < 40 && pixel[2] < 40 && pixel[3] == 255,
            "{format:?}: {pixel:?}"
        );
        assert_eq!(decoded.note, None, "{format:?}");
    }
    // The content decides, not the declared type.
    let mislabeled = decode(&embedding(
        "image/jpeg",
        &encoded(ImageFormat::Png, [0, 200, 0]),
    ));
    assert_eq!(mislabeled.pixels.get_pixel(5, 5).0, [0, 200, 0, 255]);
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                0xedb8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A PNG that declares its size and holds no pixel data.
fn png_header(width: u32, height: u32) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = width.to_be_bytes().to_vec();
    header.extend(height.to_be_bytes());
    header.extend([8, 6, 0, 0, 0]);
    for (kind, data) in [(b"IHDR", header), (b"IEND", Vec::new())] {
        let body: Vec<u8> = kind.iter().chain(&data).copied().collect();
        png.extend((data.len() as u32).to_be_bytes());
        png.extend(&body);
        png.extend(crc32(&body).to_be_bytes());
    }
    png
}

#[test]
fn embedded_images_that_cannot_be_decoded_are_skipped_and_named() {
    let png = encoded(ImageFormat::Png, [250, 20, 10]);
    for (case, mime, data) in [
        ("BMP", "image/bmp", encoded(ImageFormat::Bmp, [250, 20, 10])),
        ("damaged PNG", "image/png", png[..png.len() / 2].to_vec()),
        (
            "PNG over the edge limit",
            "image/png",
            png_header(40_000, 1),
        ),
        (
            "nested SVG",
            "image/svg+xml",
            br#"<svg xmlns="http://www.w3.org/2000/svg"/>"#.to_vec(),
        ),
    ] {
        let decoded = decode(&embedding(mime, &data));
        assert_eq!(decoded.pixels.get_pixel(5, 5).0[3], 0, "{case}");
        assert_eq!(
            decoded.pixels.get_pixel(15, 5).0,
            [0, 0, 255, 255],
            "{case}"
        );
        assert_eq!(
            decoded.note.as_deref(),
            Some("Not shown: embedded images that cannot be decoded."),
            "{case}"
        );
    }
}

#[test]
fn linked_images_are_never_opened() {
    // An opaque PNG on disk, linked by absolute path and file URL: a resolver that read
    // files would draw it.
    let path = std::env::temp_dir().join(format!("midnight-svg-linked-{}.png", std::process::id()));
    image::RgbImage::from_pixel(10, 10, image::Rgb([250, 20, 10]))
        .save(&path)
        .unwrap();
    let absolute = path
        .to_string_lossy()
        .replace('&', "&amp;")
        .replace('"', "&quot;");
    let url = format!("file:///{}", absolute.replace('\\', "/"));
    for href in [
        absolute.as_str(),
        url.as_str(),
        "https://example.invalid/image.png",
        "//example.invalid/image.png",
    ] {
        let decoded = decode(&format!(
            r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><image width="10" height="10" href="{href}"/><rect x="10" width="10" height="10" fill="#0000ff"/></svg>"##
        ));
        let drawn = (0..10)
            .flat_map(|y| (0..10).map(move |x| (x, y)))
            .filter(|&(x, y)| decoded.pixels.get_pixel(x, y).0[3] != 0)
            .count();
        assert_eq!(drawn, 0, "{href}");
        assert_eq!(decoded.pixels.get_pixel(15, 5).0, [0, 0, 255, 255]);
        assert_eq!(
            decoded.note.as_deref(),
            Some("Not shown: linked images."),
            "{href}"
        );
    }
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn identical_embedded_images_are_decoded_once_and_drawn_everywhere() {
    let png = encoded(ImageFormat::Png, [250, 20, 10]);
    let data = base64(&png);
    let mut svg =
        String::from(r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="10">"#);
    for x in 0..8 {
        svg += &format!(
            r#"<image x="{}" width="10" height="10" href="data:image/png;base64,{data}"/>"#,
            x * 10
        );
    }
    svg += "</svg>";
    let decoded = decode(&svg);
    for x in 0..8 {
        assert_eq!(
            decoded.pixels.get_pixel(x * 10 + 5, 5).0,
            [250, 20, 10, 255]
        );
    }
}
