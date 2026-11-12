use midnight_viewer_render::{LayoutError, MAX_TEXT_BYTES, TextOptions, TextRenderer};

#[test]
fn word_wrap_reflows_unicode_selection_without_changing_source_text() {
    let mut renderer = TextRenderer::default();
    let text = "first\tŽluťoučký 世界 café second third fourth fifth";
    let page = renderer
        .layout_with_options(
            text,
            &TextOptions {
                wrap_width: Some(150.0),
                ..Default::default()
            },
            &[],
        )
        .unwrap();
    assert!(page.line_count() > 1);
    assert!(page.width() <= 151.0);
    assert_eq!(page.selected_text(0..text.len()).unwrap(), text);
    let start = text.find("café").unwrap();
    let rectangles = page.selection_geometry(start..text.len()).unwrap();
    assert!(rectangles.len() > 1, "search selection spans wrapped rows");
    let first = page.selection_geometry(start..start + 1).unwrap()[0];
    assert_eq!(
        page.hit_test(
            (first.x0 + 0.1) as f32,
            ((first.y0 + first.y1) / 2.0) as f32
        )
        .unwrap(),
        start
    );
}

#[test]
fn long_unbroken_unicode_text_wraps_without_splitting_source_units() {
    let mut renderer = TextRenderer::default();
    let text = "é".repeat(8191);
    let page = renderer
        .layout_with_options(
            &text,
            &TextOptions {
                wrap_width: Some(120.0),
                ..Default::default()
            },
            &[],
        )
        .unwrap();
    assert!(page.line_count() > 100);
    assert!(page.width() <= 121.0);
    assert_eq!(page.selected_text(0..text.len()).unwrap(), text);
}

#[test]
fn wrap_width_rejects_nonfinite_and_nonpositive_geometry() {
    let mut renderer = TextRenderer::default();
    for width in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(matches!(
            renderer.layout_with_options(
                "text",
                &TextOptions {
                    wrap_width: Some(width),
                    ..Default::default()
                },
                &[]
            ),
            Err(LayoutError::InvalidGeometry)
        ));
    }
    assert!(matches!(
        renderer.layout_with_options(
            &"x".repeat(MAX_TEXT_BYTES + 1),
            &TextOptions {
                wrap_width: Some(120.0),
                ..Default::default()
            },
            &[]
        ),
        Err(LayoutError::LimitExceeded)
    ));
}
