use midnight_image_viewer_sdk::{Image, api::Viewport};
use std::io::Cursor;

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
