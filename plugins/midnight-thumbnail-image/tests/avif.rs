use midnight_thumbnail_sdk::api::Request;
use std::io::Cursor;

#[test]
fn avif_thumbnail_preserves_aspect_ratio_and_alpha() {
    let source = include_bytes!("../../image-decoder/tests/fixtures/draw_points_idat.avif");
    let frame = midnight_thumbnail_image::decode(
        &Request {
            id: 42,
            source_length: source.len() as u64,
            width: 15,
            height: 15,
            frame: 0,
        },
        &mut Cursor::new(source),
    )
    .unwrap();
    assert_eq!(
        (frame.id, frame.width, frame.height, frame.frame),
        (42, 15, 5, 0)
    );
    let pixels = frame.rgba.as_chunks::<4>().0;
    assert!(pixels.iter().any(|pixel| pixel[3] == 255));
    assert!(pixels.iter().any(|pixel| pixel[3] > 0 && pixel[3] < 255));
}
