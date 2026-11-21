use midnight_image_viewer_sdk::{
    Image,
    api::{FailureReason, Viewport},
};
use std::io::Cursor;

/// The features of a reported diagram that was refused: an `auto-start-reverse` arrow
/// marker referred to as `url(#arrow)`, a background rectangle and text in a font-family
/// list of families the worker does not have.
const DIAGRAM: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="60" viewBox="0 0 300 60" font-family="Segoe UI, Helvetica, Arial, sans-serif" font-size="14"><rect width="300" height="60" fill="#ffffff"/><defs><marker id="arrow" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="8" markerHeight="8" orient="auto-start-reverse"><path d="M0,0 L10,5 L0,10 z" fill="#202020"/></marker></defs><text x="10" y="25" fill="#202020">Synthetic step</text><path d="M150,40 L290,40" stroke="#202020" fill="none" marker-start="url(#arrow)" marker-end="url(#arrow)"/></svg>"##;

fn whole(image: &impl Image) -> Vec<[u8; 4]> {
    let metadata = image.metadata();
    image
        .render(Viewport {
            width: metadata.width,
            height: metadata.height,
            scale: 1.0,
            origin_x: 0.0,
            origin_y: 0.0,
        })
        .unwrap()
        .rgba
        .as_chunks::<4>()
        .0
        .to_vec()
}

#[test]
fn reported_diagram_features_show_text_and_both_arrowheads() {
    let image = midnight_viewer_image::decode(&mut Cursor::new(DIAGRAM)).unwrap();
    assert_eq!((image.metadata().width, image.metadata().height), (300, 60));
    let pixels = whole(&image);
    let dark = |x: usize, y: usize| {
        let pixel = pixels[y * 300 + x];
        pixel[3] == 255 && u32::from(pixel[0]) + u32::from(pixel[1]) + u32::from(pixel[2]) < 200
    };
    let text = (0..140)
        .flat_map(|x| (8..32).map(move |y| (x, y)))
        .filter(|&(x, y)| dark(x, y))
        .count();
    assert!(text > 60, "{text} text pixels");
    // Two pixels off the line: inside the end arrowhead and the reversed start arrowhead.
    assert!(dark(286, 38) && dark(154, 38) && !dark(220, 38));
}

#[test]
fn svg_refusals_reach_the_status_line_whole() {
    let reason = |svg: &str| {
        let error = midnight_viewer_image::decode(&mut Cursor::new(svg.as_bytes()))
            .err()
            .expect("refused");
        let text = error.to_string();
        // The SDK sends the decoder's text unchanged when it fits the wire form.
        assert_eq!(
            FailureReason::sanitize(&text).map(|reason| reason.as_str().to_owned()),
            Some(text.clone())
        );
        text
    };
    // A limit is a complete sentence, which the host shows as it is.
    assert_eq!(
        reason(r#"<!DOCTYPE svg [<!ENTITY x "y">]><svg xmlns="http://www.w3.org/2000/svg"/>"#),
        "This SVG declares XML entities (a DOCTYPE internal subset), which are not supported."
    );
    // Damage continues the host's "This image is damaged or incomplete:" sentence.
    assert!(
        reason(r#"<svg xmlns="http://www.w3.org/2000/svg" width="4"><rect"#)
            .starts_with("The SVG is not well-formed XML: ")
    );
}

#[test]
fn svg_shapes_preserve_original_size_and_straight_alpha() {
    let source = br#"<?xml version="1.0"?><svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="20" height="10" fill="red" fill-opacity="0.5"/></svg>"#;
    let image = midnight_viewer_image::decode(&mut Cursor::new(source)).unwrap();
    assert_eq!((image.metadata().width, image.metadata().height), (20, 10));
    let frame = image
        .render(Viewport {
            width: 20,
            height: 10,
            scale: 1.0,
            origin_x: 0.0,
            origin_y: 0.0,
        })
        .unwrap();
    assert!(
        frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == [255, 0, 0, 128])
    );
}

#[test]
fn svg_text_uses_bundled_latin_arabic_and_hebrew_fonts() {
    for text in ["Příliš žluťoučký", "مرحبا", "שלום"] {
        let source = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="50"><text x="8" y="35" font-size="24">{text}</text></svg>"#
        );
        let image = midnight_viewer_image::decode(&mut Cursor::new(source.as_bytes())).unwrap();
        let frame = image
            .render(Viewport {
                width: 200,
                height: 50,
                scale: 1.0,
                origin_x: 0.0,
                origin_y: 0.0,
            })
            .unwrap();
        assert!(
            frame
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| pixel[3] != 0)
                .count()
                > 40,
            "{text}"
        );
    }
}

#[test]
fn bundled_script_fallback_draws_distinct_glyphs_instead_of_missing_character_boxes() {
    for (first, second) in [("مممم", "سسسس"), ("שששש", "אאאא")] {
        let render = |text| {
            let source = format!(
                r#"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="50"><text x="8" y="35" font-family="Unavailable document font" font-size="24">{text}</text></svg>"#
            );
            let image = midnight_viewer_image::decode(&mut Cursor::new(source.as_bytes())).unwrap();
            image
                .render(Viewport {
                    width: 200,
                    height: 50,
                    scale: 1.0,
                    origin_x: 0.0,
                    origin_y: 0.0,
                })
                .unwrap()
                .rgba
        };
        assert_ne!(render(first), render(second));
    }
}
