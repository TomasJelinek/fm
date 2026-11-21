use std::io::{self, Cursor, Read, Seek, SeekFrom};

fn decode(body: &str) -> io::Result<image::RgbaImage> {
    let svg =
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">{body}</svg>"#);
    midnight_image_decoder::decode(&mut Cursor::new(svg), Some((20, 20)))
}

fn with_note(svg: &str) -> io::Result<midnight_image_decoder::Decoded> {
    midnight_image_decoder::decode_with_note(&mut Cursor::new(svg), None)
}

fn refusal(svg: &str) -> io::Error {
    with_note(svg).expect_err("the SVG must be refused")
}

#[test]
fn local_reuse_and_gradients_render_real_pixels() {
    let image = decode(r##"<defs><linearGradient id="g"><stop stop-color="red"/><stop offset="1" stop-color="blue"/></linearGradient><rect id="r" width="40" height="20" fill="url(#g)"/></defs><use href="#r"/>"##).unwrap();
    assert_eq!(image.dimensions(), (20, 10));
    assert!(image.get_pixel(0, 5)[0] > 200);
    assert!(image.get_pixel(19, 5)[2] > 200);
}

#[test]
fn scripts_html_and_links_are_skipped_and_named_while_the_rest_is_drawn() {
    // usvg ignores elements outside the SVG namespace (here an explicitly empty one) with
    // their content, so nothing in them is drawn, opened or named.
    let ignored = with_note(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><g xmlns=""><image href="file:///private/image.png" width="40" height="20"/><rect width="40" height="20" fill="red"/></g></svg>"#,
    )
    .unwrap();
    assert_eq!(ignored.note, None);
    assert_eq!(ignored.pixels.get_pixel(10, 10).0[3], 0);
    for (body, note) in [
        (r#"<script>alert(1)</script>"#, "Not shown: scripts."),
        (
            r#"<foreignObject width="40" height="20"><div/></foreignObject>"#,
            "Not shown: HTML content (foreignObject).",
        ),
        (
            r#"<image href="file:///private/image.png" width="40" height="20"/>"#,
            "Not shown: linked images.",
        ),
        (
            r#"<image href="https://example.invalid/image.png" width="40" height="20"/>"#,
            "Not shown: linked images.",
        ),
        (
            r#"<image href="data:image/svg+xml,%3Csvg/%3E" width="40" height="20"/>"#,
            "Not shown: embedded images that cannot be decoded.",
        ),
        (
            r#"<use href="file:///private/image.svg#x"/>"#,
            "Not shown: references to other files.",
        ),
        (
            r#"<rect width="10" height="20" fill="url(other.svg#paint) red"/>"#,
            "Not shown: references to other files.",
        ),
        (
            r#"<rect width="10" height="20" fill="red" onload="alert(1)"/>"#,
            "Not shown: scripts.",
        ),
    ] {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">{body}<rect x="30" width="10" height="20" fill="blue"/></svg>"#
        );
        let decoded = with_note(&svg).unwrap_or_else(|error| panic!("{body}: {error}"));
        assert_eq!(decoded.note.as_deref(), Some(note), "{body}");
        assert_eq!(
            decoded.pixels.get_pixel(35, 10).0,
            [0, 0, 255, 255],
            "{body}"
        );
    }
}

#[test]
fn a_huge_blur_is_drawn_at_a_cost_independent_of_its_radius() {
    let image = decode(r##"<filter id="f"><feGaussianBlur stdDeviation="100000"/></filter><rect width="40" height="20" fill="red" filter="url(#f)"/>"##).unwrap();
    assert_eq!(image.dimensions(), (20, 10));
}

#[test]
fn entity_declarations_cycles_deep_xml_and_exponential_reuse_fail_before_render() {
    let dtd = r#"<!DOCTYPE svg [<!ENTITY x "boom">]><svg xmlns="http://www.w3.org/2000/svg"><text>&x;</text></svg>"#;
    assert_eq!(
        refusal(dtd).to_string(),
        "This SVG declares XML entities (a DOCTYPE internal subset), which are not supported."
    );
    let cycle = refusal(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20">
<g id="a"><use href="#a"/></g></svg>"##,
    );
    assert_eq!(cycle.kind(), io::ErrorKind::InvalidData);
    assert_eq!(
        cycle.to_string(),
        "This SVG is invalid: the reference on line 2 leads back to itself."
    );
    let paint_cycle = refusal(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><defs><pattern id="p" width="4" height="4"><rect width="4" height="4" fill="url(#q)"/></pattern><pattern id="q" width="4" height="4"><rect width="4" height="4" fill="url(#p)"/></pattern></defs><rect width="40" height="20" fill="url(#p)"/></svg>"##,
    );
    assert!(paint_cycle.to_string().contains("leads back to itself"));
    let deep = refusal(&format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg">{}{}</svg>"#,
        "<g>".repeat(66),
        "</g>".repeat(66)
    ));
    assert_eq!(
        deep.to_string(),
        "This SVG nests elements and references more than 64 levels deep (line 1), the limit \
         for SVG images."
    );
    let mut body = String::from(r#"<defs><rect id="n0" width="1" height="1"/>"#);
    for level in 1..20 {
        body.push_str(&format!(
            r##"<g id="n{level}"><use href="#n{}"/><use href="#n{}"/></g>"##,
            level - 1,
            level - 1
        ));
    }
    body.push_str(r##"</defs><use href="#n19"/>"##);
    let error = decode(&body).expect_err("exponential reuse");
    assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);
    assert_eq!(
        error.to_string(),
        "This SVG needs more than 256 MiB for its elements and path data once its references \
         are expanded, the limit for SVG images."
    );
}

#[test]
fn a_repeated_id_and_a_missing_reference_do_not_refuse_the_drawing() {
    let image = decode(r##"<g id="a"/><g id="a"/><use href="#missing"/><rect width="40" height="20" fill="url(#none) red"/>"##).unwrap();
    assert_eq!(image.get_pixel(10, 5).0, [255, 0, 0, 255]);
}

#[test]
fn markers_patterns_masks_clip_paths_and_filters_count_every_instance() {
    // A two-node marker on every vertex of a long path.
    let path = "l1 0".repeat(150_000);
    let markers = refusal(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><marker id="m"><path d="M0 0h1"/></marker><path d="M0 0{path}" marker-mid="url(#m)"/></svg>"##
    ));
    assert!(
        markers
            .to_string()
            .starts_with("This SVG needs more than 256 MiB for its elements")
    );
    // Each level of patterns, clip paths and masks doubles the content of the previous one.
    let kind = |level: usize| ["pattern", "clipPath", "mask"][level % 3];
    let attribute = |level: usize| ["fill", "clip-path", "mask"][level % 3];
    let mut defs = String::from(
        r#"<pattern id="r0" width="1" height="1"><rect width="1" height="1"/></pattern>"#,
    );
    for level in 1..20 {
        let (previous, reference) = (level - 1, attribute(level - 1));
        defs.push_str(&format!(
            r##"<{0} id="r{level}" width="1" height="1"><rect width="1" height="1" {reference}="url(#r{previous})"/><rect width="1" height="1" {reference}="url(#r{previous})"/></{0}>"##,
            kind(level)
        ));
    }
    let error = refusal(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><defs>{defs}</defs><rect width="40" height="20" {}="url(#r19)"/></svg>"##,
        attribute(19)
    ));
    assert!(
        error
            .to_string()
            .starts_with("This SVG needs more than 256 MiB for its elements")
    );
    // Style sheets apply references too.
    let styled = refusal(&format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><style>path {{ marker-mid: url(#m) }}</style><marker id="m"><path d="M0 0h1"/></marker><path d="M0 0{path}"/></svg>"##
    ));
    assert!(
        styled
            .to_string()
            .starts_with("This SVG needs more than 256 MiB for its elements")
    );
}

#[test]
fn repeated_attribute_data_is_bounded() {
    let path = format!("M0 0{}", "l1 0".repeat(700_000));
    let mut svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><defs><path id="p" d="{path}"/></defs>"##
    );
    for _ in 0..30 {
        svg.push_str(r##"<use href="#p"/>"##);
    }
    svg.push_str("</svg>");
    assert_eq!(
        refusal(&svg).to_string(),
        "This SVG needs more than 256 MiB for its elements and path data once its references \
         are expanded, the limit for SVG images."
    );
}

#[test]
fn expansion_budget_follows_the_href_that_usvg_follows() {
    // usvg instantiates the unprefixed `href`; a leading `xlink:href` to a leaf must
    // not hide 10^6 expanded references from the budget.
    let mut body = String::from(
        r#"<defs><rect id="s" width="1" height="1"/><rect id="n0" width="40" height="20"/>"#,
    );
    for level in 1..=6 {
        body.push_str(&format!(r#"<g id="n{level}">"#));
        for _ in 0..10 {
            body.push_str(&format!(
                r##"<use xlink:href="#s" href="#n{}"/>"##,
                level - 1
            ));
        }
        body.push_str("</g>");
    }
    body.push_str(r##"</defs><use xlink:href="#s" href="#n6"/>"##);
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="40" height="20">{body}</svg>"#
    );
    let error = midnight_image_decoder::decode(&mut Cursor::new(svg), Some((20, 20)))
        .expect_err("hidden reference expansion must be refused");
    assert_eq!(error.kind(), io::ErrorKind::FileTooLarge);
    // The xlink form alone still works when no unprefixed `href` competes with it.
    let xlink = r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="4" height="2"><defs><rect id="r" width="4" height="2" fill="red"/></defs><use xlink:href="#r"/></svg>"##;
    let image = midnight_image_decoder::decode(&mut Cursor::new(xlink), None).unwrap();
    assert_eq!(image.get_pixel(1, 1).0, [255, 0, 0, 255]);
}

#[test]
fn excessive_dimensions_and_nodes_are_refused_even_for_tiny_thumbnails() {
    for (dimensions, message) in [
        (
            r#"width="32769" height="1""#,
            "This SVG is 32769 x 1 pixels; the limit is 32768 pixels per side.",
        ),
        (
            r#"width="20000" height="20000""#,
            "This SVG is 20000 x 20000 pixels, more than the limit of 128 MiB of pixels.",
        ),
        (
            r#"width="0" height="20""#,
            "This SVG has no usable size: its width, height or viewBox is zero, negative or \
             missing.",
        ),
    ] {
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" {dimensions}><rect width="100%" height="100%"/></svg>"#
        );
        let error = midnight_image_decoder::decode(&mut Cursor::new(svg), Some((1, 1)))
            .expect_err(dimensions);
        assert_eq!(error.to_string(), message);
    }
    assert_eq!(
        decode(&"<g/>".repeat(270_000)).unwrap_err().to_string(),
        "This SVG has more than 262144 XML nodes (elements, text runs and comments), the \
         limit for SVG images."
    );
}

#[test]
fn drawing_buffers_and_work_are_bounded_before_rendering() {
    let refused = |body: &str| {
        refusal(&format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="1000">{body}</svg>"#
        ))
        .to_string()
    };
    let memory = "This SVG needs more than 448 MiB of memory to draw";
    let work = "This SVG needs more than 8.6 billion pixel operations";
    // A 20000 x 20000 pattern tile.
    assert!(refused(r##"<pattern id="p" width="20000" height="20000" patternUnits="userSpaceOnUse"><rect width="20000" height="20000"/></pattern><rect width="1000" height="1000" fill="url(#p)"/>"##).starts_with(memory));
    // Nested translucent layers of content far larger than the canvas, each clamped by
    // resvg to five times the canvas per side.
    // resvg shifts each layer to its own origin, so deeper layers are clamped smaller: here
    // 100, 100, 64 and then 36 MB each (measured), so twelve levels need about 590 MB.
    let layers = format!(
        r#"{}<rect x="-5000" y="-5000" width="20000" height="20000"/>{}"#,
        r#"<g opacity="0.5">"#.repeat(12),
        "</g>".repeat(12)
    );
    assert!(refused(&layers).starts_with(memory));
    // A morphology kernel as large as the region, and a billion turbulence octaves.
    assert!(refused(r##"<filter id="f"><feMorphology radius="1000"/></filter><rect width="1000" height="1000" filter="url(#f)"/>"##).starts_with(work));
    assert!(refused(r##"<filter id="f"><feTurbulence baseFrequency="0.1" numOctaves="1000000000"/></filter><rect width="1000" height="1000" filter="url(#f)"/>"##).starts_with(work));
    // The same file is cheap at thumbnail size.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="1000" height="1000"><filter id="f"><feMorphology radius="1000"/></filter><rect width="1000" height="1000" filter="url(#f)"/></svg>"##;
    assert!(midnight_image_decoder::decode(&mut Cursor::new(svg), Some((64, 64))).is_ok());
}

#[test]
fn malformed_documents_say_why_and_where() {
    for (svg, message) in [
        (
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\"><rect x=\"",
            "The SVG is not well-formed XML: the document ends too early (line 1, column 75)",
        ),
        (
            "<svg xmlns=\"http://www.w3.org/2000/svg\">\n<g></rect></svg>",
            "The SVG is not well-formed XML: a closing tag does not match the open element \
             (line 2, column 4)",
        ),
        (
            "<svg xmlns=\"http://www.w3.org/2000/svg\"><text>&nbsp;</text></svg>",
            "The SVG is not well-formed XML: it refers to an undefined entity (line 1, column 47)",
        ),
        (
            "<html xmlns=\"http://www.w3.org/1999/xhtml\"/>",
            "This XML document is not an SVG image: its root element is not svg.",
        ),
        (
            "<svg width=\"4\" height=\"4\"/>",
            "This SVG does not declare the SVG namespace on its root element, so it is not a \
             standalone SVG image.",
        ),
    ] {
        assert_eq!(refusal(svg).to_string(), message, "{svg}");
    }
    let mut latin1 = b"<svg xmlns=\"http://www.w3.org/2000/svg\">\n<text>".to_vec();
    latin1.extend(b"caf\xe9</text></svg>");
    let error = midnight_image_decoder::decode(&mut Cursor::new(latin1), None).unwrap_err();
    assert_eq!(
        error.to_string(),
        "The SVG text is not valid UTF-8 (line 2)"
    );
    let large = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg"><!--{}--></svg>"#,
        " ".repeat(4 * 1024 * 1024)
    );
    assert_eq!(
        refusal(&large).to_string(),
        "This SVG is larger than 4 MiB, the limit for SVG images."
    );
}

#[test]
fn a_document_type_declaration_without_entities_is_accepted() {
    let svg = "<?xml version=\"1.0\"?>\n<!-- exported -->\n<!DOCTYPE svg PUBLIC \"-//W3C//DTD \
               SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">\n<svg \
               xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"2\"><rect width=\"4\" \
               height=\"2\" fill=\"red\"/></svg>";
    let decoded = with_note(svg).unwrap();
    assert_eq!(decoded.pixels.get_pixel(1, 1).0, [255, 0, 0, 255]);
    // Positions after a blanked declaration keep their line numbers.
    let broken = svg.replace("<rect", "<rect <");
    assert!(
        refusal(&broken)
            .to_string()
            .ends_with("(line 4, column 68)")
    );
}

#[test]
fn every_reason_fits_a_status_line() {
    let mut cases = vec![
        "This SVG declares XML entities (a DOCTYPE internal subset), which are not supported."
            .to_owned(),
    ];
    cases.push(refusal(&"<g/>".repeat(270_000)).to_string());
    for reason in cases {
        assert!(reason.len() <= 160, "{reason}");
        assert!(
            reason
                .bytes()
                .all(|byte| (0x20..0x7f).contains(&byte) && byte != b'/' && byte != b'\\'),
            "{reason}"
        );
    }
}

#[test]
fn empty_and_non_markup_content_say_what_they_are() {
    let empty = midnight_image_decoder::decode(&mut Cursor::new(Vec::<u8>::new()), None)
        .expect_err("empty content");
    assert_eq!(empty.to_string(), "The file is empty");
    for content in [
        b"plain text named like an image\n".as_slice(),
        b"\xef\xbb\xbf  # heading",
        &[0; 300],
    ] {
        let error = midnight_image_decoder::decode(&mut Cursor::new(content), None)
            .expect_err("not an image");
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert_eq!(
            error.to_string(),
            "The content is not a recognized image format"
        );
    }
    // Markup after a byte order mark and whitespace still reaches the SVG parser.
    let svg = "\u{feff}\n <svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"2\"/>";
    let image = midnight_image_decoder::decode(&mut Cursor::new(svg), None).unwrap();
    assert_eq!(image.dimensions(), (4, 2));
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
