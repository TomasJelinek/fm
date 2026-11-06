use midnight_viewer_render::{TextOptions, TextRenderer, Viewport};

#[test]
fn tabs_align_unicode_columns_without_changing_copied_text_or_hit_offsets() {
    let mut renderer = TextRenderer::default();
    let options = TextOptions::default();
    let source = "a\tb\r\n文\tc\r\ne\u{301}\td\r\n👩\u{200d}💻\te";
    let expanded = "a   b\n文  c\ne\u{301}   d\n👩\u{200d}💻  e";
    let page = renderer.layout_with_options(source, &options, &[]).unwrap();
    let reference = renderer
        .layout_with_options(expanded, &options, &[])
        .unwrap();
    assert_eq!(page.selected_text(0..source.len()).unwrap(), source);
    assert_eq!(page.line_count(), 4);
    for character in ['b', 'c', 'd', 'e'] {
        let at = source.rfind(character).unwrap();
        let reference_at = expanded.rfind(character).unwrap();
        let boxes = page.selection_geometry(at..at + 1).unwrap();
        assert_eq!(
            boxes,
            reference
                .selection_geometry(reference_at..reference_at + 1)
                .unwrap()
        );
        for rect in boxes {
            let index = page
                .hit_test((rect.x0 + 0.1) as f32, ((rect.y0 + rect.y1) / 2.0) as f32)
                .unwrap();
            assert_eq!(
                index, at,
                "hit returns original source offset after tab/CRLF"
            );
        }
    }
}

#[test]
fn font_family_color_and_dpi_are_host_configuration() {
    let mut renderer = TextRenderer::default();
    let options = TextOptions {
        foreground: [250, 20, 20, 255],
        ..TextOptions::default()
    };
    let page = renderer.layout_with_options("iW", &options, &[]).unwrap();
    let narrow = page.selection_geometry(0..1).unwrap()[0];
    let wide = page.selection_geometry(1..2).unwrap()[0];
    assert!(((narrow.x1 - narrow.x0) - (wide.x1 - wide.x0)).abs() < 0.1);
    let pixels = renderer
        .rasterize(
            &page,
            Viewport {
                width: 150,
                height: 100,
                origin: [0.0; 2],
                background: [0; 4],
                selection_color: [0, 240, 0, 255],
            },
            Some(0..1),
        )
        .unwrap();
    assert!(
        pixels
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .any(|p| p[0] > 200 && p[1] < 40)
    );
    assert!(pixels.rgba.as_chunks::<4>().0.contains(&[0, 240, 0, 255]));
    let double = renderer
        .layout_with_options(
            "iW",
            &TextOptions {
                scale: 2.0,
                ..options
            },
            &[],
        )
        .unwrap();
    assert!((double.width() / page.width() - 2.0).abs() < 0.15);
}

#[test]
fn expansion_and_physical_font_size_are_bounded_before_layout() {
    let mut renderer = TextRenderer::default();
    assert!(
        renderer
            .layout_with_options(&"\t".repeat(256 * 1024), &TextOptions::default(), &[])
            .is_err()
    );
    for options in [
        TextOptions {
            size: 512.0,
            scale: 8.0,
            ..TextOptions::default()
        },
        TextOptions {
            tab_width: 0,
            ..TextOptions::default()
        },
        TextOptions {
            tab_width: 65,
            ..TextOptions::default()
        },
        TextOptions {
            weight: 1001,
            ..TextOptions::default()
        },
    ] {
        assert!(renderer.layout_with_options("abc", &options, &[]).is_err());
    }
}
