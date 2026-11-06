use midnight_viewer_render::{ColorSpan, TextRenderer, Viewport};

#[test]
fn cached_glyphs_never_leak_font_size_color_or_synthesis_between_pages() {
    use midnight_viewer_render::TextOptions;
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
