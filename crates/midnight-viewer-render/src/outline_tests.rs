use crate::{TextOptions, TextRenderer, Viewport};

fn view(x: f32, y: f32) -> Viewport {
    Viewport {
        width: 240,
        height: 130,
        origin: [x, y],
        background: [23, 27, 31, 255],
        selection_color: [41, 72, 98, 255],
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
                    coordinates: run.normalized_coords().to_vec(),
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
                let actual = outlines
                    .get(&key, [50.0, 80.0], [200, 200])
                    .unwrap_or_else(|| {
                        outlines
                            .build(&key, &mut cached_scaler, [50.0, 80.0], [200, 200])
                            .unwrap()
                    })
                    .unwrap();
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
