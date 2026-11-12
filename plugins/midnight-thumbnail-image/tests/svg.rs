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
