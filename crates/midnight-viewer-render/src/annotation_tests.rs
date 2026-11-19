//! An annotation must keep its pixel phase when an integer guard is cropped away.
use super::*;
use crate::{CellAlignment, StyleSpan, TableCell, TableRow, TextOptions};

fn paint_labels(renderer: &mut TextRenderer, camera: [f32; 2], size: [u32; 2]) -> Vec<u8> {
    let mut pixels = [11, 23, 47, 101].repeat(size[0] as usize * size[1] as usize);
    let mut target = RasterMut::new(size[0], size[1], &mut pixels).unwrap();
    for (label, y) in [("2009", 131.140_63_f32), ("2010", 147.533_2_f32)] {
        let page = renderer
            .layout_with_options(
                label,
                &TextOptions {
                    size: 14.0,
                    scale: 1.0,
                    foreground: [220; 4],
                    ..TextOptions::default()
                },
                &[],
            )
            .unwrap();
        let position = [(64.0 - page.width() - 8.0).max(0.0), y];
        renderer
            .paint_annotation_at_into(&page, &mut target, camera, position)
            .unwrap();
    }
    pixels
}

#[test]
fn annotation_integer_guard_crop_preserves_fractional_camera_pixels() {
    for x in [0.0, 0.3125] {
        let camera = [x, 55.230_46_f32];
        for guarded_first in [false, true] {
            let mut renderer = TextRenderer::default();
            let guard_camera = [camera[0] - 16.0, camera[1] - 56.0];
            let (cpu, guarded) = if guarded_first {
                let guarded = paint_labels(&mut renderer, guard_camera, [96, 210]);
                (paint_labels(&mut renderer, camera, [64, 98]), guarded)
            } else {
                let cpu = paint_labels(&mut renderer, camera, [64, 98]);
                (cpu, paint_labels(&mut renderer, guard_camera, [96, 210]))
            };
            assert!(
                cpu.as_chunks::<4>()
                    .0
                    .iter()
                    .any(|pixel| *pixel != [11, 23, 47, 101])
            );
            let visible = crop_guard(&guarded);
            assert_eq!(
                cpu.iter().zip(&visible).position(|(a, b)| a != b),
                None,
                "integer guards preserve every annotation channel: camera={camera:?}, guarded_first={guarded_first}"
            );
        }
    }
}

fn crop_guard(pixels: &[u8]) -> Vec<u8> {
    (56..154)
        .flat_map(|row| {
            let start = (row * 96 + 16) * 4;
            pixels[start..start + 64 * 4].iter().copied()
        })
        .collect()
}

fn paint_page(
    renderer: &mut TextRenderer,
    page: &TextPage,
    camera: [f32; 2],
    size: [u32; 2],
) -> Vec<u8> {
    let mut pixels = [11, 23, 47, 101].repeat(size[0] as usize * size[1] as usize);
    renderer
        .paint_annotation_at_into(
            page,
            &mut RasterMut::new(size[0], size[1], &mut pixels).unwrap(),
            camera,
            [17.375, 131.140_63],
        )
        .unwrap();
    pixels
}

#[test]
fn composite_annotation_decorations_and_borders_keep_the_same_guard_pixels() {
    let mut renderer = TextRenderer::default();
    let options = TextOptions {
        size: 14.0,
        foreground: [220; 4],
        ..TextOptions::default()
    };
    let rows = [TableRow {
        range: 0..9,
        cells: vec![
            TableCell {
                range: 0..4,
                alignment: CellAlignment::Left,
            },
            TableCell {
                range: 5..9,
                alignment: CellAlignment::Left,
            },
        ],
        header: false,
    }];
    let styles = [StyleSpan {
        range: 0..9,
        weight: 400,
        italic: false,
        underline: true,
        strikethrough: true,
        monospace: true,
        size: 14.0,
    }];
    let page = renderer
        .layout_with_tables("2009\t2010", &options, &[], &styles, &rows)
        .unwrap();
    assert!(
        page.composite
            .as_ref()
            .is_some_and(|grid| !grid.parts.is_empty() && !grid.borders.is_empty())
    );
    let camera = [0.3125, 55.230_46];
    let cpu = paint_page(&mut renderer, &page, camera, [64, 98]);
    let guarded = paint_page(
        &mut renderer,
        &page,
        [camera[0] - 16.0, camera[1] - 56.0],
        [96, 210],
    );
    assert_eq!(cpu, crop_guard(&guarded));
    let plain = renderer
        .layout_with_tables("2009\t2010", &options, &[], &[], &rows)
        .unwrap();
    assert_ne!(
        cpu,
        paint_page(&mut renderer, &plain, camera, [64, 98]),
        "real decorations must be painted"
    );
}

#[test]
fn annotation_geometry_errors_leave_the_target_untouched() {
    let mut renderer = TextRenderer::default();
    let page = renderer
        .layout_with_options("2009", &TextOptions::default(), &[])
        .unwrap();
    for (camera, position) in [
        ([f32::NAN, 0.0], [0.0, 0.0]),
        ([0.0, f32::INFINITY], [0.0, 0.0]),
        ([0.0, 0.0], [f32::NAN, 0.0]),
        ([0.0, 0.0], [0.0, f32::NEG_INFINITY]),
        ([f32::MAX, 0.0], [-f32::MAX, 0.0]),
    ] {
        let mut pixels = [11, 23, 47, 101].repeat(8 * 8);
        let original = pixels.clone();
        assert_eq!(
            renderer.paint_annotation_at_into(
                &page,
                &mut RasterMut::new(8, 8, &mut pixels).unwrap(),
                camera,
                position
            ),
            Err(LayoutError::InvalidGeometry)
        );
        assert_eq!(pixels, original);
    }
    let mut pixels = [11, 23, 47, 101].repeat(8 * 8);
    let original = pixels.clone();
    let mut invalid_target = RasterMut {
        width: 16,
        height: 8,
        rgba: &mut pixels,
    };
    assert_eq!(
        renderer.paint_annotation_at_into(&page, &mut invalid_target, [0.0; 2], [0.0; 2]),
        Err(LayoutError::InvalidGeometry)
    );
    assert_eq!(pixels, original);
}
