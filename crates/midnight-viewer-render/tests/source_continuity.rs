use midnight_viewer_render::{TextOptions, TextRenderer, Viewport};

#[test]
fn source_glyph_pixels_survive_rebasing_through_a_fractional_line_prefix() {
    let prefix = "first line\nsecond line\nthird line\nfourth line\n";
    let body: String = (0..90)
        .map(|i| format!("  \"value_{i}\": \"stable coloured JSON scalar words 0123456789\",\n"))
        .collect();
    for scale in [1.0, 1.25] {
        let mut renderer = TextRenderer::default();
        let options = TextOptions {
            family: "monospace",
            size: 14.0,
            scale,
            foreground: [31, 90, 132, 255],
            ..Default::default()
        };
        let complete = renderer
            .layout_with_options(&format!("{prefix}{body}"), &options, &[])
            .unwrap();
        let suffix = renderer.layout_with_options(&body, &options, &[]).unwrap();
        let prefix_top = complete.caret_geometry(prefix.len()).unwrap().y0 as f32;
        let suffix_top = suffix.caret_geometry(0).unwrap().y0 as f32;
        eprintln!("scale {scale}, prefix caret {prefix_top}, suffix caret {suffix_top}");
        for offset in [7.0, 32.25] {
            let origin = prefix_top.ceil() + offset;
            let paint = |renderer: &mut TextRenderer, page, y| {
                renderer
                    .rasterize(
                        page,
                        Viewport {
                            width: 800,
                            height: 400,
                            origin: [0.0, y],
                            background: [255; 4],
                            selection_color: [40, 80, 120, 255],
                        },
                        None,
                    )
                    .unwrap()
            };
            let expected = paint(&mut renderer, &complete, origin);
            let actual = paint(&mut renderer, &suffix, suffix_top + (origin - prefix_top));
            let changed = expected
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .zip(actual.rgba.as_chunks::<4>().0)
                .filter(|(a, b)| a.iter().zip(*b).any(|(a, b)| a.abs_diff(*b) > 3))
                .count();
            assert_eq!(
                changed, 0,
                "same source pixels changed after rebase at scale {scale}, origin {origin}"
            );
        }
    }
}
