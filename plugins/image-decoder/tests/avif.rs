use std::io::Cursor;

#[test]
fn real_avif_opaque_white_pixel_decodes_as_srgb() {
    let image = midnight_image_decoder::decode(
        &mut Cursor::new(include_bytes!("fixtures/white_1x1.avif")),
        None,
    )
    .unwrap();
    assert_eq!(image.dimensions(), (1, 1));
    // Upstream encoded this white pixel at default (lossy) quality.
    let pixel = image.get_pixel(0, 0).0;
    assert_eq!(pixel[3], 255);
    assert!(pixel[..3].iter().all(|channel| channel.abs_diff(255) <= 3));
}

#[test]
fn real_avif_color_and_alpha_match_independent_png_reference() {
    let image = midnight_image_decoder::decode(
        &mut Cursor::new(include_bytes!("fixtures/draw_points_idat.avif")),
        None,
    )
    .unwrap();
    let reference = image::load_from_memory(include_bytes!("fixtures/draw_points.png"))
        .unwrap()
        .to_rgba8();
    assert_eq!(image.dimensions(), (33, 11));
    assert_eq!(reference.dimensions(), (33, 11));
    for x in [5, 16, 27] {
        let actual = image.get_pixel(x, 5).0;
        let expected = reference.get_pixel(x, 5).0;
        assert_eq!(actual[3], expected[3], "alpha at {x}");
        for channel in 0..3 {
            assert!(
                actual[channel].abs_diff(expected[channel]) <= 3,
                "color at {x}: {actual:?} vs {expected:?}"
            );
        }
    }
    assert!(image.pixels().any(|pixel| pixel[3] > 0 && pixel[3] < 255));
}

#[test]
fn truncated_avif_returns_an_error_without_pixels() {
    let source = include_bytes!("fixtures/draw_points_idat.avif");
    for length in [32, 139, 430, source.len() - 1] {
        assert!(midnight_image_decoder::decode(&mut Cursor::new(&source[..length]), None).is_err());
    }
}
