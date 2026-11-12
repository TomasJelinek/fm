use midnight_image_viewer_sdk::{Image, api::Viewport};
use std::io::Cursor;

#[test]
fn avif_viewer_retains_original_alpha_pixels_for_zoom_and_pan() {
    let source = include_bytes!("../../image-decoder/tests/fixtures/draw_points_idat.avif");
    let image = midnight_viewer_image::decode(&mut Cursor::new(source)).unwrap();
    assert_eq!((image.metadata().width, image.metadata().height), (33, 11));
    let frame = image
        .render(Viewport {
            width: 33,
            height: 11,
            scale: 1.0,
            origin_x: 0.0,
            origin_y: 0.0,
        })
        .unwrap();
    let pixels = frame.rgba.as_chunks::<4>().0;
    assert!(pixels.iter().any(|pixel| pixel[3] == 255));
    assert!(pixels.iter().any(|pixel| pixel[3] > 0 && pixel[3] < 255));
    let panned = image
        .render(Viewport {
            width: 2,
            height: 2,
            scale: 2.0,
            origin_x: 16.0,
            origin_y: 5.0,
        })
        .unwrap();
    assert_eq!(panned.rgba, pixels[5 * 33 + 16].repeat(4));
}
