use super::Surface;

fn input(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    for y in 0..height {
        for x in 0..width {
            let alpha = ((x + y * width) % 256) as u8;
            bytes.extend_from_slice(&[
                ((x * 13 + y * 5) % 256) as u8 & alpha,
                ((x * 7 + y * 17) % 256) as u8 & alpha,
                ((x * 23 + y * 3) % 256) as u8 & alpha,
                alpha,
            ]);
        }
    }
    bytes
}

#[test]
fn cropped_conversion_matches_scalar_pixels_for_all_alpha_values_and_rounded_edges() {
    let (width, height) = (19, 17);
    let mut surface = Surface::new(7, width, height).unwrap();
    for extra_x in [0, 1, 16] {
        for extra_y in [0, 1, 16] {
            let native_width = width + extra_x;
            let native_height = height + extra_y;
            let source = input(native_width, native_height);
            let original = source.clone();
            let actual = surface
                .paint_cropped(7, native_width, native_height, &source)
                .unwrap();
            let mut expected = Vec::new();
            for y in 0..height as usize {
                for x in 0..width as usize {
                    let pixel = (y * native_width as usize + x) * 4;
                    expected.push(source[pixel + 2]);
                    expected.push(source[pixel + 1]);
                    expected.push(source[pixel]);
                    expected.push(source[pixel + 3]);
                }
            }
            assert_eq!(actual, expected, "crop {extra_x}x{extra_y}");
            assert_eq!(
                source, original,
                "borrowed CEF pixels must remain untouched"
            );
        }
    }
}

#[test]
fn cropped_conversion_keeps_revision_dimension_and_buffer_length_fences() {
    let mut surface = Surface::new(7, 19, 17).unwrap();
    for (revision, width, height) in [
        (6, 19, 17),
        (7, 18, 17),
        (7, 19, 16),
        (7, 36, 17),
        (7, 19, 34),
    ] {
        assert!(
            surface
                .paint_cropped(revision, width, height, &input(width, height))
                .is_none()
        );
    }
    let bytes = input(20, 18);
    assert!(
        surface
            .paint_cropped(7, 20, 18, &bytes[..bytes.len() - 1])
            .is_none()
    );
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(surface.paint_cropped(7, 20, 18, &extra).is_none());
    let actual = surface.paint_cropped(7, 20, 18, &bytes).unwrap();
    assert_eq!(actual.len(), 19 * 17 * 4);
}
