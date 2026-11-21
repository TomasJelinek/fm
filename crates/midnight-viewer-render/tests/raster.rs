use midnight_viewer_render::{
    CellAlignment, ColorSpan, LayoutError, RasterMut, StyleSpan, TableCell, TableRow, TextOptions,
    TextRenderer, Viewport,
};
use std::sync::Arc;

#[test]
fn rasterize_into_a_shared_buffer_matches_rasterize_and_rejects_mismatched_buffers() {
    let mut renderer = TextRenderer::default();
    let text = "alpha العربية e\u{301}\nstruck 👩\u{200d}💻 under\tx";
    let options = TextOptions {
        size: 22.0,
        ..TextOptions::default()
    };
    let styled = renderer
        .layout_with_styles(
            text,
            &options,
            &[ColorSpan {
                range: 0..5,
                color: [250, 35, 20, 255],
            }],
            &[StyleSpan {
                range: 26..text.len(),
                weight: 700,
                italic: true,
                underline: true,
                strikethrough: true,
                monospace: false,
                size: 22.0,
            }],
        )
        .unwrap();
    let table = renderer
        .layout_with_tables(
            text,
            &options,
            &[],
            &[],
            &[TableRow {
                range: 0..text.len(),
                header: true,
                cells: vec![TableCell {
                    range: 0..text.len(),
                    alignment: CellAlignment::Center,
                }],
            }],
        )
        .unwrap();
    let mut view = viewport(300, 90);
    view.origin = [3.25, 1.5];
    view.background = [50, 100, 200, 128];
    let bytes = 300 * 90 * 4;
    for (page, selection) in [
        (&styled, Some(2..8)),
        (&table, Some(0..12)),
        (&styled, None),
    ] {
        let expected = renderer.rasterize(page, view, selection.clone()).unwrap();
        // A frame buffer allocated once and painted while it is still unique.
        let mut shared: Arc<[u8]> = std::iter::repeat_n(0xa5, bytes).collect();
        renderer
            .rasterize_into(page, view, selection, Arc::get_mut(&mut shared).unwrap())
            .unwrap();
        assert!(shared[..] == expected.rgba[..], "rasterize_into differs");
    }
    for length in [0, bytes - 4, bytes - 1, bytes + 4] {
        let mut buffer = vec![0xa5; length];
        assert_eq!(
            renderer.rasterize_into(&styled, view, None, &mut buffer),
            Err(LayoutError::InvalidGeometry)
        );
        assert!(
            buffer.iter().all(|&byte| byte == 0xa5),
            "a rejected buffer stays untouched"
        );
    }
    // The viewport is validated exactly as by `rasterize`.
    assert_eq!(
        renderer.rasterize_into(&styled, viewport(0, 1), None, &mut []),
        Err(LayoutError::LimitExceeded)
    );
    view.origin[0] = f32::NAN;
    assert_eq!(
        renderer.rasterize_into(&styled, view, None, &mut vec![0; bytes]),
        Err(LayoutError::InvalidGeometry)
    );
}

#[test]
fn paint_over_into_blends_over_the_surface_and_rejects_inconsistent_ones() {
    let mut renderer = TextRenderer::default();
    let label = renderer.layout("1234", 18.0, 1.0).unwrap();
    let mut view = viewport(64, 32);
    view.origin = [-4.5, -3.0];
    view.background = [10, 20, 30, 255];
    // Painted over an opaque surface, the label must equal `rasterize` over that
    // background: nothing is cleared or replaced around its glyphs.
    let expected = renderer.rasterize(&label, view, None).unwrap();
    let background = [10, 20, 30, 255].repeat(64 * 32);
    let mut surface = background.clone();
    let mut target = RasterMut::new(64, 32, &mut surface).unwrap();
    assert_eq!((target.width(), target.height()), (64, 32));
    assert_eq!(target.rgba_mut().len(), 64 * 32 * 4);
    renderer
        .paint_over_into(&label, &mut target, view.origin)
        .unwrap();
    assert_ne!(surface, background, "the label is painted");
    assert_eq!(surface, expected.rgba);
    let mut target = RasterMut::new(64, 32, &mut surface).unwrap();
    assert_eq!(
        renderer.paint_over_into(&label, &mut target, [f32::NAN, -3.0]),
        Err(LayoutError::InvalidGeometry)
    );
    assert_eq!(surface, expected.rgba, "a rejected origin paints nothing");
    for length in [0, 64 * 32 * 4 - 1, 64 * 32 * 4 + 4] {
        assert!(matches!(
            RasterMut::new(64, 32, &mut vec![0; length]),
            Err(LayoutError::InvalidGeometry)
        ));
    }
    assert!(RasterMut::new(u32::MAX, u32::MAX, &mut []).is_err());
}

#[test]
fn cached_glyphs_never_leak_font_size_color_or_synthesis_between_pages() {
    let mut cached = TextRenderer::default();
    for (size, scale, weight, italic, foreground) in [
        (18.0, 1.0, 400, false, [230, 10, 30, 255]),
        (24.0, 1.5, 700, true, [30, 230, 30, 128]),
        (18.0, 1.0, 400, false, [230, 10, 30, 255]),
    ] {
        let options = TextOptions {
            size,
            scale,
            weight,
            italic,
            foreground,
            ..TextOptions::default()
        };
        let text = "office العربية 👩\u{200d}💻";
        let page = cached.layout_with_options(text, &options, &[]).unwrap();
        let expected_page = TextRenderer::default()
            .layout_with_options(text, &options, &[])
            .unwrap();
        let actual = cached.rasterize(&page, viewport(600, 100), None).unwrap();
        let expected = TextRenderer::default()
            .rasterize(&expected_page, viewport(600, 100), None)
            .unwrap();
        assert_eq!(actual.rgba, expected.rgba);
    }
}

fn viewport(width: u32, height: u32) -> Viewport {
    Viewport {
        width,
        height,
        origin: [0.0; 2],
        background: [12, 16, 20, 255],
        selection_color: [45, 80, 140, 255],
        selection_text: [0; 4],
    }
}

#[test]
fn semantic_colors_paint_real_glyphs_without_changing_source() {
    let text = "Source 文档 العربية e\u{301} 👩\u{200d}💻";
    let mut renderer = TextRenderer::default();
    let page = renderer
        .layout_styled(
            text,
            28.0,
            1.0,
            &[ColorSpan {
                range: 0..6,
                color: [250, 35, 20, 255],
            }],
        )
        .unwrap();
    let pixels = renderer.rasterize(&page, viewport(700, 100), None).unwrap();
    assert_eq!(pixels.rgba.len(), 700 * 100 * 4);
    assert!(
        pixels
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 200 && p[1] < 70)
            .count()
            > 20
    );
    assert_eq!(page.selected_text(0..text.len()).unwrap(), text);
}

#[test]
fn scrolling_crops_the_same_pixels_as_the_full_layout() {
    let mut renderer = TextRenderer::default();
    let page = renderer
        .layout("alpha العربية\r\nsecond line e\u{301}", 24.0, 1.5)
        .unwrap();
    let full = renderer.rasterize(&page, viewport(600, 150), None).unwrap();
    let mut view = viewport(150, 65);
    view.origin = [23.0, 7.0];
    let crop = renderer.rasterize(&page, view, None).unwrap();
    for y in 0..65 {
        let start = ((y + 7) * 600 + 23) * 4;
        assert_eq!(
            &crop.rgba[y * 600..(y + 1) * 600],
            &full.rgba[start..start + 600]
        );
    }
}

#[test]
fn selection_background_and_transparency_use_bounded_premultiplied_pixels() {
    let mut renderer = TextRenderer::default();
    let page = renderer.layout("alpha\r\nβeta", 26.0, 1.0).unwrap();
    let mut view = viewport(200, 100);
    view.background = [50, 100, 200, 128];
    let plain = renderer.rasterize(&page, view, None).unwrap();
    let selected = renderer.rasterize(&page, view, Some(0..5)).unwrap();
    assert_ne!(plain.rgba, selected.rgba);
    assert!(
        selected
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|p| p[..3].iter().all(|&channel| channel <= p[3]))
    );
    for (width, height) in [(0, 1), (u32::MAX, u32::MAX), (8192, 8192)] {
        assert!(
            renderer
                .rasterize(&page, viewport(width, height), None)
                .is_err()
        );
    }
    view.origin[0] = f32::NAN;
    assert!(renderer.rasterize(&page, view, None).is_err());
}
