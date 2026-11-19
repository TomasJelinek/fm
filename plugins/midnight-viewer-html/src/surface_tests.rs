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

fn damage(x: i32, y: i32, width: i32, height: i32) -> super::DamageRect {
    super::DamageRect {
        x,
        y,
        width,
        height,
    }
}

fn changed_pixel(bytes: &mut [u8], width: u32, x: u32, y: u32) {
    let at = ((y * width + x) * 4) as usize;
    let alpha = ((x * 31 + y * 17 + 73) % 256) as u8;
    bytes[at..at + 4].copy_from_slice(&[alpha & 0x35, alpha & 0x6a, alpha & 0x97, alpha]);
}

#[test]
fn dirty_patch_matches_whole_conversion_for_overlap_edges_and_dpi_crop() {
    let (width, height, native_width, native_height) = (20, 16, 24, 19);
    let mut surface = Surface::new(7, width, height).unwrap();
    let preceding = input(native_width, native_height);
    let mut base = surface
        .paint_cropped(7, native_width, native_height, &preceding)
        .unwrap();
    let original = base.clone();
    let mut source = preceding;
    // Authored changed pixels, independently of the helper's clipping logic.
    for (left, top, right, bottom) in [
        (0, 0, 3, 3),
        (1, 1, 6, 4),
        (18, 14, 24, 19),
        (20, 2, 24, 4),
        (10, 4, 13, 9),
    ] {
        for y in top..bottom {
            for x in left..right {
                changed_pixel(&mut source, native_width, x, y);
            }
        }
    }
    let borrowed = source.clone();
    let expected = surface
        .paint_cropped(7, native_width, native_height, &source)
        .unwrap();
    let rectangles = [
        damage(-2, -1, 5, 4),
        damage(1, 1, 5, 3),
        damage(18, 14, 10, 10),
        damage(20, 2, 4, 2),
        damage(10, 4, 3, 5),
    ];
    assert!(surface.patch_cropped(
        7,
        native_width,
        native_height,
        &source,
        &rectangles,
        &mut base
    ));
    assert_eq!(
        base, expected,
        "damage application must equal the real whole conversion"
    );
    assert_eq!(source, borrowed, "borrowed CEF buffer remains untouched");
    let untouched = ((12 * width + 8) * 4) as usize;
    assert_eq!(
        &base[untouched..untouched + 4],
        &original[untouched..untouched + 4]
    );
    for y in 0..height {
        for x in 0..width {
            let target = ((y * width + x) * 4) as usize;
            let native = ((y * native_width + x) * 4) as usize;
            assert_eq!(base[target + 3], source[native + 3], "alpha at {x},{y}");
        }
    }
}

#[test]
fn dirty_patch_malformed_final_rectangle_preserves_every_base_byte() {
    let surface = Surface::new(7, 20, 16).unwrap();
    let source = input(24, 19);
    let original = vec![0xa5; 20 * 16 * 4];
    for invalid in [
        damage(2, 1, 0, 1),
        damage(2, 1, 1, 0),
        damage(2, 1, -1, 1),
        damage(2, 1, 1, -1),
    ] {
        let mut base = original.clone();
        assert!(!surface.patch_cropped(
            7,
            24,
            19,
            &source,
            &[damage(1, 1, 3, 2), invalid],
            &mut base
        ));
        assert_eq!(
            base, original,
            "all rectangles must be validated before the first write"
        );
    }
}

#[test]
fn dirty_patch_rejects_stale_geometry_revision_and_exact_length_mismatches() {
    let mut surface = Surface::new(7, 20, 16).unwrap();
    let source = input(24, 19);
    let original = vec![0x5a; 20 * 16 * 4];
    let rectangles = [damage(1, 1, 3, 2)];
    for (revision, width, height) in [
        (6, 24, 19),
        (7, 19, 19),
        (7, 24, 15),
        (7, 37, 19),
        (7, 24, 33),
        (7, 0, 19),
        (7, u32::MAX, u32::MAX),
    ] {
        let mut base = original.clone();
        assert!(!surface.patch_cropped(revision, width, height, &source, &rectangles, &mut base));
        assert_eq!(base, original);
    }
    let mut extra_source = source.clone();
    extra_source.push(0);
    for invalid in [&source[..source.len() - 1], extra_source.as_slice()] {
        let mut base = original.clone();
        assert!(!surface.patch_cropped(7, 24, 19, invalid, &rectangles, &mut base));
        assert_eq!(base, original);
    }
    for length in [original.len() - 1, original.len() + 1] {
        let mut base = vec![0x73; length];
        let before = base.clone();
        assert!(!surface.patch_cropped(7, 24, 19, &source, &rectangles, &mut base));
        assert_eq!(base, before);
    }
    surface.resize(8, 16, 20).unwrap();
    let mut base = original.clone();
    assert!(!surface.patch_cropped(7, 24, 19, &source, &rectangles, &mut base));
    assert_eq!(base, original);
    assert!(!surface.patch_cropped(8, 24, 19, &source, &rectangles, &mut base));
    assert_eq!(
        base, original,
        "same byte area cannot authorize old native geometry"
    );
    for invalid in [
        Surface {
            revision: 0,
            width: 20,
            height: 16,
            view_paint: 0,
        },
        Surface {
            revision: 7,
            width: 0,
            height: 16,
            view_paint: 0,
        },
        Surface {
            revision: 7,
            width: u32::MAX,
            height: u32::MAX,
            view_paint: 0,
        },
        Surface {
            revision: 7,
            width: 8192,
            height: 8192,
            view_paint: 0,
        },
    ] {
        let mut base = original.clone();
        assert!(!invalid.patch_cropped(7, 24, 19, &source, &rectangles, &mut base));
        assert_eq!(base, original);
    }
}

#[test]
fn dirty_patch_empty_or_fully_clipped_damage_falls_back_without_mutation() {
    let surface = Surface::new(7, 20, 16).unwrap();
    let source = input(24, 19);
    let original = vec![0x6d; 20 * 16 * 4];
    for rectangles in [
        vec![],
        vec![damage(20, 0, 4, 16)],
        vec![damage(0, 16, 20, 3)],
        vec![damage(-10, -10, 2, 2)],
        vec![damage(i32::MAX, i32::MAX, i32::MAX, i32::MAX)],
        vec![damage(i32::MIN, 0, i32::MAX, 1)],
    ] {
        let mut base = original.clone();
        assert!(!surface.patch_cropped(7, 24, 19, &source, &rectangles, &mut base));
        assert_eq!(base, original);
    }
}

#[test]
fn dirty_patch_count_and_summed_area_budget_fallback_is_atomic() {
    let surface = Surface::new(7, 20, 16).unwrap();
    let source = input(24, 19);
    let original = vec![0x6d; 20 * 16 * 4];
    let mut accepted = original.clone();
    assert!(surface.patch_cropped(7, 24, 19, &source, &[damage(0, 0, 1, 1); 64], &mut accepted));
    assert_eq!(
        &accepted[..4],
        &[source[2], source[1], source[0], source[3]]
    );
    for rectangles in [
        vec![damage(0, 0, 1, 1); 65],
        vec![damage(0, 0, 11, 16)],
        vec![damage(0, 0, 10, 16); 2],
        vec![damage(-1, -1, i32::MAX, i32::MAX)],
    ] {
        let mut base = original.clone();
        assert!(!surface.patch_cropped(7, 24, 19, &source, &rectangles, &mut base));
        assert_eq!(base, original);
    }
}

#[test]
fn dirty_patch_accepts_exact_half_area_and_wide_signed_clipping_without_recoloring() {
    let (width, height) = (20, 16);
    let mut surface = Surface::new(7, width, height).unwrap();
    for wide_signed in [false, true] {
        let preceding = input(width, height);
        let mut base = surface.paint_cropped(7, width, height, &preceding).unwrap();
        let mut source = preceding;
        let (right, bottom) = if wide_signed { (1, 1) } else { (10, 16) };
        for y in 0..bottom {
            for x in 0..right {
                changed_pixel(&mut source, width, x, y);
            }
        }
        let rect = if wide_signed {
            damage(-2147483646, 0, i32::MAX, 1)
        } else {
            damage(0, 0, 10, 16)
        };
        let expected = surface.paint_cropped(7, width, height, &source).unwrap();
        assert!(surface.patch_cropped(7, width, height, &source, &[rect], &mut base));
        assert_eq!(base, expected);
    }
}
