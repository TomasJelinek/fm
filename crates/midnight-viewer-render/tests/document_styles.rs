use midnight_viewer_render::{StyleSpan, TextOptions, TextRenderer};

#[test]
fn underline_is_painted_in_the_same_raster_as_glyphs() {
    let mut renderer = TextRenderer::default();
    let options = TextOptions::default();
    let normal = renderer.layout_with_options("link", &options, &[]).unwrap();
    let styled = renderer
        .layout_with_styles(
            "link",
            &options,
            &[],
            &[StyleSpan {
                range: 0..4,
                weight: 400,
                italic: false,
                underline: true,
                strikethrough: false,
                monospace: false,
                size: options.size,
            }],
        )
        .unwrap();
    let viewport = midnight_viewer_render::Viewport {
        width: 160,
        height: 80,
        origin: [0.0; 2],
        background: [0; 4],
        selection_color: [0; 4],
    };
    let normal = renderer.rasterize(&normal, viewport, None).unwrap();
    let styled = renderer.rasterize(&styled, viewport, None).unwrap();
    assert!(
        normal.rgba != styled.rgba,
        "Underline must alter visible pixels"
    );
}

#[test]
fn heading_geometry_and_hit_testing_share_the_styled_layout() {
    let text = "Heading\nplain";
    let mut renderer = TextRenderer::default();
    let options = TextOptions::default();
    let plain = renderer.layout_with_options(text, &options, &[]).unwrap();
    let styled = renderer
        .layout_with_styles(
            text,
            &options,
            &[],
            &[StyleSpan {
                range: 0..7,
                weight: 700,
                italic: false,
                underline: false,
                strikethrough: false,
                monospace: false,
                size: 30.0,
            }],
        )
        .unwrap();
    assert!(styled.height() > plain.height());
    assert!(styled.width() > plain.width());
    assert_eq!(styled.selected_text(0..text.len()).unwrap(), text);
    let rect = styled.selection_geometry(0..1).unwrap()[0];
    assert_eq!(
        styled
            .hit_test((rect.x0 + 0.1) as f32, ((rect.y0 + rect.y1) / 2.0) as f32)
            .unwrap(),
        0
    );
}

#[test]
fn oversized_and_invalid_semantic_styles_are_rejected_before_shaping() {
    let mut renderer = TextRenderer::default();
    assert!(
        renderer
            .layout_with_styles(
                "文",
                &TextOptions::default(),
                &[],
                &[StyleSpan {
                    range: 1..2,
                    weight: 700,
                    italic: false,
                    underline: false,
                    strikethrough: false,
                    monospace: false,
                    size: 20.0,
                }]
            )
            .is_err()
    );
    assert!(
        renderer
            .layout_with_styles(
                "x",
                &TextOptions::default(),
                &[],
                &[StyleSpan {
                    range: 0..1,
                    weight: 700,
                    italic: false,
                    underline: false,
                    strikethrough: false,
                    monospace: false,
                    size: f32::NAN,
                }]
            )
            .is_err()
    );
}

#[test]
fn code_spans_use_monospace_and_strikethrough_changes_visible_pixels() {
    let mut renderer = TextRenderer::default();
    let options = TextOptions {
        family: "sans-serif",
        ..TextOptions::default()
    };
    let style = StyleSpan {
        range: 0..4,
        weight: 400,
        italic: false,
        underline: false,
        strikethrough: false,
        monospace: true,
        size: options.size,
    };
    let code = renderer
        .layout_with_styles("iWMW", &options, &[], std::slice::from_ref(&style))
        .unwrap();
    let monospace = renderer
        .layout_with_options("iWMW", &TextOptions::default(), &[])
        .unwrap();
    assert!((code.width() - monospace.width()).abs() < 0.01);
    let strike = renderer
        .layout_with_styles(
            "iWMW",
            &options,
            &[],
            &[StyleSpan {
                strikethrough: true,
                ..style
            }],
        )
        .unwrap();
    let viewport = midnight_viewer_render::Viewport {
        width: 120,
        height: 60,
        origin: [0.0; 2],
        background: [0; 4],
        selection_color: [0; 4],
    };
    assert_ne!(
        renderer.rasterize(&code, viewport, None).unwrap().rgba,
        renderer.rasterize(&strike, viewport, None).unwrap().rgba
    );
}
