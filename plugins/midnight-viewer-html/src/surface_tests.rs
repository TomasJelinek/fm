use super::{Layers, Surface};
use std::hint::black_box;
use std::time::Instant;

#[test]
#[ignore = "manual timing: cargo test --lib paint_frame_copy_timing -- --ignored --nocapture"]
fn paint_frame_copy_timing() {
    let (width, height) = (3840, 2160);
    let bgra = vec![0x80; width as usize * height as usize * 4];
    let mut surface = Surface::new(1, width, height).unwrap();
    let runs = 20;
    // Previous OnPaint path: retain the converted base, then clone it into the frame.
    let mut base = Vec::new();
    let started = Instant::now();
    for _ in 0..runs {
        base = surface.paint_cropped(1, width, height, &bgra).unwrap();
        black_box(base.clone());
    }
    let retained = started.elapsed() / runs;
    drop(base);
    // Current path without a popup: the converted paint moves into the frame.
    let mut layers = Layers::default();
    let started = Instant::now();
    for _ in 0..runs {
        let rgba = surface.paint_cropped(1, width, height, &bgra).unwrap();
        black_box(layers.view(width, height, rgba).unwrap());
    }
    let moved = started.elapsed() / runs;
    eprintln!("3840x2160 paint: retain+clone {retained:?}, move {moved:?} per paint");
}

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
fn surface_bounds_and_resize_reject_stale_paints() {
    assert!(Surface::new(0, 1, 1).is_err());
    assert!(Surface::new(1, 8193, 1).is_err());
    assert!(Surface::new(1, 8192, 8192).is_err());
    let mut surface = Surface::new(7, 1, 1).unwrap();
    let mut bgra = vec![10, 20, 30, 128];
    let frame = surface.paint_cropped(7, 1, 1, &bgra).unwrap();
    bgra.fill(255);
    assert_eq!(frame, vec![30, 20, 10, 128], "owned RGBA, not a CEF borrow");
    surface.resize(8, 2, 1).unwrap();
    assert!(surface.paint_cropped(7, 2, 1, &[0; 8]).is_none());
    assert!(surface.paint_cropped(8, 1, 1, &bgra).is_none());
    assert!(surface.resize(9, 8193, 1).is_err());
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
