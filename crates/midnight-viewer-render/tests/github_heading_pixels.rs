use midnight_viewer_render::{
    DocumentBlock, DocumentKind, DocumentStyle, TextOptions, TextRenderer, Viewport,
};

#[test]
fn heading_separator_preserves_its_pixel_phase_after_fractional_caret_restoration() {
    let mut renderer = TextRenderer::default();
    let text = "A heading with a fractional underline offset";
    let page = renderer
        .layout_document(
            text,
            &TextOptions {
                family: "system-ui",
                size: 16.0,
                foreground: [31, 35, 40, 255],
                wrap_width: Some(800.0),
                ..Default::default()
            },
            &DocumentStyle {
                colors: &[],
                styles: &[],
                tables: &[],
                blocks: &[DocumentBlock {
                    range: 0..text.len(),
                    kind: DocumentKind::Heading(2),
                    depth: 0,
                }],
                background: [255; 4],
                start_of_document: false,
            },
        )
        .unwrap();
    // An H2 underline is placed 0.3em below the line. A caret restored through
    // another fragment can cancel that fractional origin with tiny f32 error.
    let origin = page.height() - 51.0;
    let paint = |renderer: &mut TextRenderer, y| {
        renderer
            .rasterize(
                &page,
                Viewport {
                    width: 800,
                    height: 100,
                    origin: [0.0, y],
                    background: [255; 4],
                    selection_color: [0; 4],
                },
                None,
            )
            .unwrap()
            .rgba
    };
    let reference = paint(&mut renderer, origin);
    for error in [-0.0001, 0.0001] {
        let pixels = paint(&mut renderer, origin + error);
        assert!(
            pixels == reference,
            "root separator changed with error {error}"
        );
    }
    for fraction in [0.25, 0.5] {
        assert!(paint(&mut renderer, origin + fraction) != reference);
    }
}
