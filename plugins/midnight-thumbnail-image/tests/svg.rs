use midnight_thumbnail_sdk::api::Request;
use std::io::Cursor;

#[test]
fn svg_thumbnail_rasterizes_at_requested_aspect_ratio_with_alpha() {
    let source = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2000 1000"><rect width="2000" height="1000" fill="blue" fill-opacity="0.5"/></svg>"#;
    let frame = midnight_thumbnail_image::decode(
        &Request {
            id: 7,
            source_length: source.len() as u64,
            width: 20,
            height: 20,
            frame: 0,
        },
        &mut Cursor::new(source),
    )
    .unwrap();
    assert_eq!(
        (frame.id, frame.width, frame.height, frame.frame),
        (7, 20, 10, 0)
    );
    assert!(
        frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [0, 0, 255, 128])
    );
}

/// The features of a reported diagram that was refused: an `auto-start-reverse` arrow
/// marker referred to as `url(#arrow)`, a background rectangle and text in a font-family
/// list of families the worker does not have.
fn diagram(text: bool) -> String {
    let label = if text {
        r##"<text x="10" y="60" fill="#202020" font-size="40">Synthetic step</text>"##
    } else {
        ""
    };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="600" height="200" viewBox="0 0 600 200" font-family="Segoe UI, Helvetica, Arial, sans-serif"><rect width="600" height="200" fill="#ffffff"/><defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#202020"/></marker></defs>{label}<path d="M300,150 L580,150" stroke="#202020" stroke-width="5" fill="none" marker-start="url(#arrow)" marker-end="url(#arrow)"/></svg>"##
    )
}

fn thumbnail(svg: &str) -> Vec<[u8; 4]> {
    let frame = midnight_thumbnail_image::decode(
        &Request {
            id: 3,
            source_length: svg.len() as u64,
            width: 150,
            height: 150,
            frame: 0,
        },
        &mut Cursor::new(svg.as_bytes()),
    )
    .unwrap();
    assert_eq!((frame.width, frame.height), (150, 50));
    frame.rgba.as_chunks::<4>().0.to_vec()
}

#[test]
fn svg_thumbnail_of_the_reported_diagram_features_shows_text_and_arrowheads() {
    let dark = |pixel: &[u8; 4]| {
        pixel[3] == 255 && u32::from(pixel[0]) + u32::from(pixel[1]) + u32::from(pixel[2]) < 300
    };
    let label = |pixels: &[[u8; 4]]| {
        (0..20 * 150)
            .filter(|&index| index % 150 < 75 && dark(&pixels[index]))
            .count()
    };
    let with_text = thumbnail(&diagram(true));
    assert!(label(&with_text) > 30, "{} text pixels", label(&with_text));
    assert_eq!(label(&thumbnail(&diagram(false))), 0);
    // At a quarter scale the 5-unit line is about one pixel high at y 37.5; arrowheads,
    // eight stroke widths long and tall, cover the row two pixels above it near the
    // ends: the end one points right at x 146, the reversed start one left at x 74.
    let row = 150 * 35;
    assert!(dark(&with_text[row + 140]), "end arrowhead");
    assert!(dark(&with_text[row + 80]), "reversed start arrowhead");
    assert!(!dark(&with_text[row + 110]), "line only");
}

#[test]
fn fractional_svg_scale_does_not_create_an_oversized_thumbnail_frame() {
    let source = br#"<svg xmlns="http://www.w3.org/2000/svg" width="13" height="13"><rect width="13" height="13" fill="blue"/></svg>"#;
    let frame = midnight_thumbnail_image::decode(
        &Request {
            id: 9,
            source_length: source.len() as u64,
            width: 7,
            height: 7,
            frame: 0,
        },
        &mut Cursor::new(source),
    )
    .unwrap();
    assert_eq!((frame.id, frame.width, frame.height), (9, 7, 7));
    assert_eq!(frame.rgba.len(), 7 * 7 * 4);
}
