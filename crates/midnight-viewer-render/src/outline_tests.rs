use crate::{
    CellAlignment, ColorSpan, TableCell, TableRow, TextOptions, TextPage, TextRenderer, Viewport,
};

#[test]
fn culling_far_lines_and_glyphs_never_changes_raster_pixels() {
    // Stacked marks reach far above their line box, the long line and wide
    // ligature far beside their origins. Culling must still keep all their ink.
    let text = format!(
        "top line\n{}\nfiller\na{} base\nfiller\n{} \u{fdfd} wide 👩\u{200d}💻\n{}",
        "abcdefgh ".repeat(300),
        "\u{301}".repeat(24),
        "e\u{302}\u{303}".repeat(8),
        "tail line\n".repeat(8)
    );
    let mut culled = TextRenderer::default();
    let mut reference = TextRenderer::default();
    reference.glyphs.unculled = true;
    for size in [12.0, 31.5] {
        let options = TextOptions {
            family: "sans-serif",
            size,
            ..TextOptions::default()
        };
        let a = culled.layout_with_options(&text, &options, &[]).unwrap();
        let b = reference.layout_with_options(&text, &options, &[]).unwrap();
        let mut origins = vec![(-200.0, 20.0), (1500.25, 10.75), (4000.5, size * 1.3)];
        let mut y = -160.0;
        while y < a.height() + 40.0 {
            origins.push((0.0, y));
            y += 19.25;
        }
        for (x, y) in origins {
            assert_eq!(
                culled.rasterize(&a, view(x, y), None).unwrap().rgba,
                reference.rasterize(&b, view(x, y), None).unwrap().rgba,
                "size={size} origin={x},{y}"
            );
        }
    }
}

/// A page whose decorations sit far from their lines, as a malformed font's
/// underline/strikeout metrics or style-supplied offsets can place them.
fn far_decorations(renderer: &mut TextRenderer, text: &str) -> TextPage {
    use parley::StyleProperty;
    let (shaped, source_map) = crate::source_map::SourceMap::prepare(text, 4).unwrap();
    let mut builder = renderer
        .layouts
        .ranged_builder(&mut renderer.fonts, &shaped, 1.0, false);
    builder.push_default(StyleProperty::FontSize(16.0));
    builder.push_default(StyleProperty::Brush([200, 200, 200, 255]));
    builder.push(StyleProperty::Underline(true), 0..5);
    builder.push(StyleProperty::UnderlineOffset(Some(-300.0)), 0..5);
    builder.push(StyleProperty::UnderlineSize(Some(4.0)), 0..5);
    builder.push(
        StyleProperty::UnderlineBrush(Some([250, 40, 40, 255])),
        0..5,
    );
    builder.push(StyleProperty::Strikethrough(true), 6..12);
    builder.push(StyleProperty::StrikethroughOffset(Some(260.0)), 6..12);
    builder.push(
        StyleProperty::StrikethroughBrush(Some([40, 250, 40, 255])),
        6..12,
    );
    let mut layout = builder.build(&shaped);
    layout.break_all_lines(None);
    TextPage {
        text: text.into(),
        line_bounds: super::line_bounds(&layout),
        layout,
        source_map,
        composite: None,
    }
}

#[test]
fn culling_keeps_decorations_that_lie_far_from_their_line() {
    let text = "first\nsecond\nthird";
    let mut culled = TextRenderer::default();
    let mut reference = TextRenderer::default();
    reference.glyphs.unculled = true;
    let a = far_decorations(&mut culled, text);
    let b = far_decorations(&mut reference, text);
    let mut seen = [false; 2];
    let mut y = -330.0;
    while y < 420.0 {
        let expected = reference.rasterize(&b, view(0.0, y), None).unwrap().rgba;
        for (index, color) in [[250, 40, 40, 255], [40, 250, 40, 255]].iter().enumerate() {
            seen[index] |= expected.as_chunks::<4>().0.contains(color);
        }
        let actual = culled.rasterize(&a, view(0.0, y), None).unwrap().rgba;
        assert!(actual == expected, "culling changed pixels at origin y={y}");
        y += 19.25;
    }
    assert_eq!(seen, [true; 2], "both far decorations must be painted");
}

#[test]
fn culling_keeps_ink_that_leaves_its_table_cell_box() {
    // A tight line height puts stacked capital accents above the cell's box.
    // Part culling must use the same ink reach as line culling.
    let cell = "\u{1ed6}\u{302}\u{303}\u{301} tall";
    let text = format!("intro\n\n{cell}\tx\n\noutro");
    let start = text.find('\u{1ed6}').unwrap();
    let end = start + cell.len() + 2;
    let rows = [TableRow {
        range: start..end,
        header: false,
        cells: vec![
            TableCell {
                range: start..start + cell.len(),
                alignment: CellAlignment::Left,
            },
            TableCell {
                range: end - 1..end,
                alignment: CellAlignment::Left,
            },
        ],
    }];
    let spans = [ColorSpan {
        range: start..start + cell.len(),
        color: [255, 0, 0, 255],
    }];
    let options = TextOptions {
        family: "sans-serif",
        size: 24.0,
        line_height: Some(1.0),
        ..TextOptions::default()
    };
    let mut culled = TextRenderer::default();
    let mut reference = TextRenderer::default();
    reference.glyphs.unculled = true;
    let a = culled
        .layout_with_tables(&text, &options, &spans, &[], &rows)
        .unwrap();
    let b = reference
        .layout_with_tables(&text, &options, &spans, &[], &rows)
        .unwrap();
    let cell_top = a
        .composite
        .as_ref()
        .unwrap()
        .parts
        .iter()
        .find(|part| part.range.start == start)
        .unwrap()
        .origin[1];
    let red = |rgba: &[u8]| {
        rgba.as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] > 100 && pixel[1] < 40 && pixel[2] < 40)
    };
    let mut visible = false;
    // Each raster ends just above the cell box, inside the accents' ink.
    for gap in [0.5, 1.5, 2.5, 4.0] {
        let origin = view(0.0, cell_top - gap - 130.0);
        let expected = reference.rasterize(&b, origin, None).unwrap().rgba;
        visible |= red(&expected);
        let actual = culled.rasterize(&a, origin, None).unwrap().rgba;
        assert!(actual == expected, "culling changed pixels at gap={gap}");
    }
    assert!(visible, "the fixture must put ink above the cell box");
}

#[test]
fn repainting_a_large_proportional_raster_reuses_every_cached_glyph_image() {
    // Fractional advances give every glyph instance its own phase. Syntax colors
    // must not multiply coverage masks, and the working set must fit the cache.
    // Indent each line differently so that lines do not repeat glyph phases.
    let text: String = (0..100)
        .map(|line| {
            format!(
                "{}Proportional prose keeps fractional advances, so each glyph has its phase.\n",
                "i".repeat(line % 17)
            )
        })
        .collect();
    let palette = [
        [86, 156, 214, 255],
        [206, 145, 120, 255],
        [181, 206, 168, 255],
    ];
    let mut spans = Vec::new();
    let mut start = 0;
    for (index, word) in text.split_inclusive(' ').enumerate() {
        spans.push(ColorSpan {
            range: start..start + word.len(),
            color: palette[index % palette.len()],
        });
        start += word.len();
    }
    let mut renderer = TextRenderer::default();
    let page = renderer
        .layout_with_options(
            &text,
            &TextOptions {
                family: "sans-serif",
                size: 15.0,
                ..TextOptions::default()
            },
            &spans,
        )
        .unwrap();
    let view = Viewport {
        width: 1000,
        height: 2400,
        origin: [0.0, 0.25],
        background: [23, 27, 31, 255],
        selection_color: [41, 72, 98, 255],
        selection_text: [0; 4],
    };
    let first = renderer.rasterize(&page, view, None).unwrap();
    let misses = renderer.glyphs.misses;
    assert!(
        misses > 4096,
        "the fixture exceeds the former 4096-entry bound"
    );
    let repaint = renderer.rasterize(&page, view, None).unwrap();
    assert_eq!(first.rgba, repaint.rgba);
    assert_eq!(
        renderer.glyphs.misses, misses,
        "an unchanged repaint must reuse every cached glyph image"
    );
}

fn view(x: f32, y: f32) -> Viewport {
    Viewport {
        width: 240,
        height: 130,
        origin: [x, y],
        background: [23, 27, 31, 255],
        selection_color: [41, 72, 98, 255],
        selection_text: [0; 4],
    }
}

#[test]
fn fractional_motion_reuses_hinted_outlines_across_new_raster_phases() {
    let mut renderer = TextRenderer::default();
    let page = renderer.layout("abcd0123\nabcd0123", 19.0, 1.0).unwrap();
    let first = renderer.rasterize(&page, view(0.123, 0.234), None).unwrap();
    let scales = renderer.glyphs.scales;
    assert!(scales > 0);
    let next = renderer.rasterize(&page, view(0.321, 0.432), None).unwrap();
    assert_ne!(
        first.rgba, next.rgba,
        "the new fraction must change actual pixels"
    );
    assert_eq!(
        renderer.glyphs.scales, scales,
        "a new phase rebuilt hinted glyph outlines"
    );
}

#[test]
fn cached_outlines_match_reference_pixels_with_overhang_clipping_and_fallback() {
    let text = "fj Åg e\u{301}\u{302} العربية עברית 世界 👩‍💻 \u{10ffff}\n".repeat(24);
    let mut cached = TextRenderer::default();
    let mut reference = TextRenderer::default();
    reference.glyphs.uncached_outlines = true;
    for (size, weight, italic) in [(9.25, 400, false), (19.5, 700, true), (38.75, 350, false)] {
        let options = TextOptions {
            size,
            weight,
            italic,
            ..TextOptions::default()
        };
        let a = cached.layout_with_options(&text, &options, &[]).unwrap();
        let b = reference.layout_with_options(&text, &options, &[]).unwrap();
        for (x, y) in [
            (1000.25, 1000.75),
            (0.25, 0.75),
            (0.0, 0.0),
            (-10.75, -19.9),
            (0.001, 0.999),
            (25.35, 81.72),
            (100.91, 320.13),
            (1000.25, 1000.75),
        ] {
            let a = cached.rasterize(&a, view(x, y), None).unwrap();
            let b = reference.rasterize(&b, view(x, y), None).unwrap();
            assert_eq!(
                a.rgba, b.rgba,
                "size={size} weight={weight} italic={italic} origin={x},{y}"
            );
        }
    }
}

#[test]
fn synthesized_bold_and_skew_match_swash_mask_data_and_placement() {
    use crate::glyph_cache::GlyphKey;
    use parley::PositionedLayoutItem;
    use swash::{
        FontRef,
        scale::{Render, ScaleContext, Source},
        zeno::{Transform, Vector},
    };
    let mut renderer = TextRenderer::default();
    let page = renderer.layout("fj", 19.0, 1.0).unwrap();
    let glyph_run = page
        .layout
        .lines()
        .next()
        .unwrap()
        .items()
        .find_map(|item| match item {
            PositionedLayoutItem::GlyphRun(run) => Some(run),
            _ => None,
        })
        .unwrap();
    let run = glyph_run.run();
    let data = run.font();
    let font = FontRef::from_index(data.data.as_ref(), data.index as usize).unwrap();
    let mut outlines = super::outlines::OutlineCache::default();
    for glyph in glyph_run.positioned_glyphs() {
        for (size, bold, skew) in [
            (11.25_f32, true, 0.0_f32),
            (19.5, false, 12.0),
            (38.75, true, -11.0),
        ] {
            for fraction in [[0.0_f32, 0.0_f32], [0.001, 0.999], [0.75, 0.25]] {
                let key = GlyphKey {
                    font: (data.data.id(), data.index),
                    size: size.to_bits(),
                    // Variation coordinates reach the scaler directly; the key
                    // only needs a stable interned id for them.
                    coordinates: 1,
                    glyph: glyph.id as u16,
                    fraction: fraction.map(f32::to_bits),
                    color: [133, 177, 209, 255],
                    embolden: bold,
                    skew: skew.to_bits(),
                };
                let mut reference_context = ScaleContext::default();
                let mut reference_scaler = reference_context
                    .builder(font)
                    .size(size)
                    .hint(true)
                    .normalized_coords(run.normalized_coords())
                    .build();
                let mut render = Render::new(&[Source::Outline]);
                render.offset(Vector::new(fraction[0], -fraction[1]));
                if bold {
                    render.embolden(size * 0.02);
                }
                if skew != 0.0 {
                    render.transform(Some(Transform::new(
                        1.0,
                        0.0,
                        skew.to_radians().tan(),
                        1.0,
                        0.0,
                        0.0,
                    )));
                }
                let expected = render.render(&mut reference_scaler, key.glyph).unwrap();
                let mut cached_context = ScaleContext::default();
                let mut cached_scaler = cached_context
                    .builder(font)
                    .size(size)
                    .hint(true)
                    .normalized_coords(run.normalized_coords())
                    .build();
                let rendered = outlines
                    .get(&key, [50.0, 80.0], [200, 200])
                    .unwrap_or_else(|| {
                        outlines
                            .build(&key, &mut cached_scaler, [50.0, 80.0], [200, 200])
                            .unwrap()
                    });
                let super::outlines::Outline::Ink(actual) = rendered else {
                    panic!("visible glyph without ink");
                };
                assert_eq!(
                    (
                        actual.placement.left,
                        actual.placement.top,
                        actual.placement.width,
                        actual.placement.height
                    ),
                    (
                        expected.placement.left,
                        expected.placement.top,
                        expected.placement.width,
                        expected.placement.height
                    )
                );
                assert_eq!(
                    actual.data, expected.data,
                    "size={size} bold={bold} skew={skew} fraction={fraction:?}"
                );
            }
        }
    }
}
