use std::io::{self, Cursor, Read, Seek, SeekFrom};

fn decode(body: &str) -> io::Result<image::RgbaImage> {
    let svg =
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">{body}</svg>"#);
    midnight_image_decoder::decode(&mut Cursor::new(svg), Some((20, 20)))
}

#[test]
fn local_reuse_and_gradients_render_real_pixels() {
    let image = decode(r##"<defs><linearGradient id="g"><stop stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient><rect id="r" width="40" height="20" fill="url(#g)"/></defs><use href="#r"/>"##).unwrap();
    assert_eq!(image.dimensions(), (20, 10));
    assert!(image.get_pixel(0, 5)[0] > 200);
    assert!(image.get_pixel(19, 5)[2] > 200);
}

#[test]
fn scripts_foreign_content_images_and_external_references_are_explicitly_refused() {
    for body in [
        r#"<g xmlns=""><image href="file:///private/image.png" width="40" height="20"/></g>"#,
        r#"<script>alert(1)</script>"#,
        r#"<foreignObject width="40" height="20"><div/></foreignObject>"#,
        r#"<image href="file:///private/image.png" width="40" height="20"/>"#,
        r#"<image href="https://example.invalid/image.png" width="40" height="20"/>"#,
        r#"<image href="data:image/svg+xml,%3Csvg/%3E" width="40" height="20"/>"#,
        r#"<use href="file:///private/image.svg#x"/>"#,
        r#"<rect width="40" height="20" onload="alert(1)"/>"#,
        r#"<filter id="f"><feGaussianBlur stdDeviation="100000"/></filter>"#,
    ] {
        assert_eq!(
            decode(body)
                .err()
                .unwrap_or_else(|| panic!("unexpected SVG acceptance: {body}"))
                .kind(),
            io::ErrorKind::Unsupported,
            "{body}"
        );
    }
}

#[test]
fn dtd_cycles_duplicate_ids_deep_xml_and_exponential_reuse_fail_before_render() {
    let dtd = br#"<!DOCTYPE svg [<!ENTITY x "boom">]><svg xmlns="http://www.w3.org/2000/svg"><text>&x;</text></svg>"#;
    assert!(midnight_image_decoder::decode(&mut Cursor::new(dtd), None).is_err());
    assert!(decode(r##"<g id="a"><use href="#a"/></g>"##).is_err());
    assert!(decode(r#"<g id="a"/><g id="a"/>"#).is_err());
    assert!(decode(&format!("{}{}", "<g>".repeat(66), "</g>".repeat(66))).is_err());
    let mut body = String::from(r#"<defs><rect id="n0" width="1" height="1"/>"#);
    for level in 1..20 {
        body.push_str(&format!(
            r##"<g id="n{level}"><use href="#n{}"/><use href="#n{}"/></g>"##,
            level - 1,
            level - 1
        ));
    }
    body.push_str(r##"</defs><use href="#n19"/>"##);
    assert!(decode(&body).is_err());
}

#[test]
fn excessive_dimensions_and_nodes_are_refused_even_for_tiny_thumbnails() {
    for dimensions in [
        r#"width="32769" height="1""#,
        r#"width="20000" height="20000""#,
        r#"width="0" height="20""#,
    ] {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" {dimensions}><rect width="100%" height="100%"/></svg>"#
        );
        assert!(midnight_image_decoder::decode(&mut Cursor::new(svg), Some((1, 1))).is_err());
    }
    assert!(decode(&"<g/>".repeat(33000)).is_err());
}

#[test]
fn unknown_input_is_bounded_without_reading_an_entire_large_source() {
    struct Large {
        position: u64,
        read: usize,
    }
    impl Read for Large {
        fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
            bytes.fill(b' ');
            self.position += bytes.len() as u64;
            self.read += bytes.len();
            Ok(bytes.len())
        }
    }
    impl Seek for Large {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            self.position = match from {
                SeekFrom::Start(n) => n,
                SeekFrom::Current(n) => self.position.checked_add_signed(n).unwrap(),
                SeekFrom::End(_) => panic!("must not measure the entire source"),
            };
            Ok(self.position)
        }
    }
    let mut source = Large {
        position: 0,
        read: 0,
    };
    assert!(midnight_image_decoder::decode(&mut source, None).is_err());
    assert!(
        source.read <= 4 * 1024 * 1024 + 2 * 256 * 1024,
        "{}",
        source.read
    );
}

#[test]
fn byte_order_mark_and_requested_bounds_are_handled_without_upscaling() {
    let svg = concat!(
        "\u{feff}",
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="3" height="2"><rect width="3" height="2" fill="red"/></svg>"#
    );
    let image = midnight_image_decoder::decode(&mut Cursor::new(svg), Some((20, 20))).unwrap();
    assert_eq!(image.dimensions(), (3, 2));
    assert_eq!(image.get_pixel(1, 1).0, [255, 0, 0, 255]);
}

#[test]
fn fractional_svg_scale_stays_inside_requested_dimensions() {
    for (original, target, expected) in [
        ((13, 13), (7, 7), (7, 7)),
        ((13, 26), (7, 7), (4, 7)),
        ((13, 13), (20, 20), (13, 13)),
    ] {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{}" height="{}"><rect width="100%" height="100%" fill="red"/></svg>"#,
            original.0, original.1
        );
        let image = midnight_image_decoder::decode(&mut Cursor::new(svg), Some(target)).unwrap();
        assert_eq!(image.dimensions(), expected);
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }
}
