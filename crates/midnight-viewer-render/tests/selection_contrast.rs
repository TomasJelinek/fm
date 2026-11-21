//! Selected glyphs stay readable on the selection color, judged on painted pixels.
use midnight_viewer_render::{
    ColorSpan, DocumentBlock, DocumentKind, DocumentStyle, Raster, Rect, StyleSpan, TextOptions,
    TextPage, TextRenderer, Viewport,
};
use std::ops::Range;

const DARK_THEME: [u8; 4] = [24, 27, 34, 255];
const DARK_SELECTION: [u8; 4] = [38, 79, 120, 255];
/// About 1.3:1 on the dark selection.
const DARK_BLUE: [u8; 4] = [30, 50, 150, 255];
/// About 8:1 on the dark selection.
const LIGHT_TOKEN: [u8; 4] = [230, 230, 200, 255];
const WIDTH: u32 = 420;
const HEIGHT: u32 = 200;

fn luminance(color: [u8; 4]) -> f64 {
    color[..3]
        .iter()
        .zip([0.2126, 0.7152, 0.0722])
        .map(|(&channel, weight)| {
            let value = f64::from(channel) / 255.0;
            weight
                * if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
        })
        .sum()
}

fn contrast(a: [u8; 4], b: [u8; 4]) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

fn options() -> TextOptions<'static> {
    // Large monospace stems leave fully covered pixels: the brush itself.
    TextOptions {
        size: 36.0,
        ..TextOptions::default()
    }
}

fn layout(renderer: &mut TextRenderer, text: &str, colors: &[ColorSpan]) -> TextPage {
    renderer
        .layout_with_options(text, &options(), colors)
        .unwrap()
}

/// Selected glyphs keep their own colors: no selection text color.
fn viewport(background: [u8; 4], selection_color: [u8; 4]) -> Viewport {
    Viewport {
        width: WIDTH,
        height: HEIGHT,
        origin: [0.0; 2],
        background,
        selection_color,
        selection_text: [0; 4],
    }
}

/// The glyph cells of `bytes` on their line, inset by a pixel so that a
/// neighbor's antialiased edge stays outside. Works for both directions.
fn cells(page: &TextPage, bytes: Range<usize>) -> Rect {
    let start = page.caret_geometry(bytes.start).unwrap();
    let end = page.caret_geometry(bytes.end).unwrap();
    Rect {
        x0: start.x0.min(end.x0) + 1.0,
        x1: start.x0.max(end.x0) - 1.0,
        y0: start.y0,
        y1: start.y1,
    }
}

/// Whole pixels inside `rect`; the viewport origin is zero and themes opaque.
fn pixels(raster: &Raster, rect: Rect) -> Vec<[u8; 4]> {
    let columns = rect.x0.ceil() as usize..rect.x1.floor() as usize;
    (rect.y0.ceil() as usize..rect.y1.floor() as usize)
        .flat_map(|y| columns.clone().map(move |x| pixel(raster, x, y)))
        .collect()
}

fn pixel(raster: &Raster, x: usize, y: usize) -> [u8; 4] {
    let at = (y * raster.width as usize + x) * 4;
    raster.rgba[at..at + 4].try_into().unwrap()
}

/// The pixel that contrasts most with `background`: fully covered glyph ink.
fn strongest(pixels: &[[u8; 4]], background: [u8; 4]) -> [u8; 4] {
    *pixels
        .iter()
        .max_by(|a, b| contrast(**a, background).total_cmp(&contrast(**b, background)))
        .unwrap()
}

/// The ink of a selected token: about 4.5:1 on `selection`, not much more.
fn assert_just_readable(raster: &Raster, cell: Rect, selection: [u8; 4]) -> [u8; 4] {
    let ink = strongest(&pixels(raster, cell), selection);
    let ratio = contrast(ink, selection);
    assert!((4.49..4.65).contains(&ratio), "{ink:?}: {ratio}");
    ink
}

#[test]
fn a_dark_token_turns_readable_on_a_dark_selection_and_a_light_token_keeps_its_pixels() {
    let mut renderer = TextRenderer::default();
    let text = "MMMMMM MMMMMM";
    let colors = [
        ColorSpan {
            range: 0..6,
            color: DARK_BLUE,
        },
        ColorSpan {
            range: 7..13,
            color: LIGHT_TOKEN,
        },
    ];
    let page = layout(&mut renderer, text, &colors);
    let selected = renderer
        .rasterize(
            &page,
            viewport(DARK_THEME, DARK_SELECTION),
            Some(0..text.len()),
        )
        .unwrap();
    // The selected line as painted before: the selection color behind unchanged glyphs.
    let unchanged = renderer
        .rasterize(&page, viewport(DARK_SELECTION, DARK_SELECTION), None)
        .unwrap();

    let light = cells(&page, 7..13);
    assert!(pixels(&selected, light) == pixels(&unchanged, light));
    assert!(pixels(&selected, light).contains(&LIGHT_TOKEN));

    let dark = cells(&page, 0..6);
    assert!(!pixels(&selected, dark).contains(&DARK_BLUE));
    let ink = assert_just_readable(&selected, dark, DARK_SELECTION);
    assert!(luminance(ink) > luminance(DARK_BLUE), "lightened: {ink:?}");
    // Still blue, with the original channel order (red < green < blue).
    assert!(ink[0] < ink[1] && ink[1] < ink[2], "{ink:?}");
}

#[test]
fn a_selection_starting_inside_a_run_recolors_only_the_selected_glyphs() {
    let mut renderer = TextRenderer::default();
    let text = "MMMMMMMMMM";
    let colors = [ColorSpan {
        range: 0..text.len(),
        color: DARK_BLUE,
    }];
    let page = layout(&mut renderer, text, &colors);
    let view = viewport(DARK_THEME, DARK_SELECTION);
    let plain = renderer.rasterize(&page, view, None).unwrap();
    let selected = renderer.rasterize(&page, view, Some(4..10)).unwrap();
    let unchanged = renderer
        .rasterize(&page, viewport(DARK_SELECTION, DARK_SELECTION), None)
        .unwrap();
    for glyph in 0..text.len() {
        let cell = cells(&page, glyph..glyph + 1);
        if glyph < 4 {
            assert!(
                pixels(&selected, cell) == pixels(&plain, cell),
                "glyph {glyph} is not selected"
            );
        } else {
            assert!(
                pixels(&selected, cell) != pixels(&unchanged, cell),
                "glyph {glyph} is selected"
            );
            assert_just_readable(&selected, cell, DARK_SELECTION);
        }
    }
    // Everything away from the selection is byte-identical to the plain paint.
    let [rect] = page.selection_geometry(4..10).unwrap()[..] else {
        panic!("one line");
    };
    let rows = rect.y0.floor() as usize..rect.y1.ceil() as usize;
    let columns = rect.x0.floor() as usize - 2..rect.x1.ceil() as usize + 2;
    for y in 0..HEIGHT as usize {
        for x in 0..WIDTH as usize {
            if !rows.contains(&y) || !columns.contains(&x) {
                assert_eq!(pixel(&selected, x, y), pixel(&plain, x, y), "({x}, {y})");
            }
        }
    }
}

#[test]
fn selected_clusters_follow_shaped_tabs_and_right_to_left_order() {
    let mut renderer = TextRenderer::default();
    let view = viewport(DARK_THEME, DARK_SELECTION);
    let unchanged_view = viewport(DARK_SELECTION, DARK_SELECTION);
    // The tab expands to two spaces before the shaped clusters it is selected with.
    // Hebrew letters are two bytes each and run right to left.
    for (text, selection, selected_glyphs) in [
        ("MM\tMMMM", 2..7, vec![3..4, 4..5, 5..6, 6..7]),
        ("MM\tMMMM", 4..7, vec![4..5, 5..6, 6..7]),
        ("אבגדהו", 0..6, vec![0..2, 2..4, 4..6]),
    ] {
        let colors = [ColorSpan {
            range: 0..text.len(),
            color: DARK_BLUE,
        }];
        let page = layout(&mut renderer, text, &colors);
        let plain = renderer.rasterize(&page, view, None).unwrap();
        let selected = renderer
            .rasterize(&page, view, Some(selection.clone()))
            .unwrap();
        let unchanged = renderer.rasterize(&page, unchanged_view, None).unwrap();
        let glyphs = text
            .char_indices()
            .filter(|(_, character)| *character != '\t');
        for (start, character) in glyphs {
            let bytes = start..start + character.len_utf8();
            let cell = cells(&page, bytes.clone());
            if selected_glyphs.contains(&bytes) {
                assert!(
                    pixels(&selected, cell) != pixels(&unchanged, cell),
                    "{text:?} {bytes:?} is selected"
                );
                assert_just_readable(&selected, cell, DARK_SELECTION);
            } else if bytes.end <= selection.start || bytes.start >= selection.end {
                assert!(
                    pixels(&selected, cell) == pixels(&plain, cell),
                    "{text:?} {bytes:?} is not selected"
                );
            }
        }
    }
}

#[test]
fn a_light_selection_on_a_light_theme_darkens_a_light_token() {
    const LIGHT_THEME: [u8; 4] = [250, 250, 250, 255];
    const LIGHT_SELECTION: [u8; 4] = [173, 214, 255, 255];
    /// About 1.6:1 on the light selection.
    const SKY: [u8; 4] = [120, 170, 230, 255];
    let mut renderer = TextRenderer::default();
    let text = "MMMMMM";
    let colors = [ColorSpan {
        range: 0..text.len(),
        color: SKY,
    }];
    let page = layout(&mut renderer, text, &colors);
    let selected = renderer
        .rasterize(
            &page,
            viewport(LIGHT_THEME, LIGHT_SELECTION),
            Some(0..text.len()),
        )
        .unwrap();
    let ink = assert_just_readable(&selected, cells(&page, 0..text.len()), LIGHT_SELECTION);
    assert!(luminance(ink) < luminance(SKY), "darkened: {ink:?}");
    assert!(
        ink[0] < ink[1] && ink[1] < ink[2],
        "still sky blue: {ink:?}"
    );
}

#[test]
fn selected_words_of_a_document_recolor_only_inside_their_own_block() {
    let mut renderer = TextRenderer::default();
    let text = "MMMM MMMM\n\nMMMM MMMM";
    let blocks = [
        DocumentBlock {
            range: 0..9,
            kind: DocumentKind::Paragraph,
            depth: 0,
        },
        DocumentBlock {
            range: 11..20,
            kind: DocumentKind::Paragraph,
            depth: 0,
        },
    ];
    let colors = [ColorSpan {
        range: 0..text.len(),
        color: DARK_BLUE,
    }];
    let page = renderer
        .layout_document(
            text,
            &options(),
            &DocumentStyle {
                colors: &colors,
                styles: &[],
                tables: &[],
                blocks: &blocks,
                background: DARK_THEME,
                start_of_document: false,
            },
        )
        .unwrap();
    let view = viewport(DARK_THEME, DARK_SELECTION);
    let plain = renderer.rasterize(&page, view, None).unwrap();
    let selected = renderer.rasterize(&page, view, Some(16..20)).unwrap();
    for word in [0..4, 5..9, 11..15] {
        let cell = cells(&page, word.clone());
        assert!(
            pixels(&selected, cell) == pixels(&plain, cell),
            "{word:?} is not selected"
        );
    }
    let ink = assert_just_readable(&selected, cells(&page, 16..20), DARK_SELECTION);
    assert!(luminance(ink) > luminance(DARK_BLUE), "lightened: {ink:?}");
}

#[test]
fn underlines_and_strikethroughs_of_selected_text_turn_readable_too() {
    let mut renderer = TextRenderer::default();
    let text = "MMMMMMMMMM";
    let colors = [ColorSpan {
        range: 0..text.len(),
        color: DARK_BLUE,
    }];
    let style = |underline, strikethrough| StyleSpan {
        range: 0..text.len(),
        weight: 400,
        italic: false,
        underline,
        strikethrough,
        monospace: false,
        size: options().size,
    };
    let view = viewport(DARK_THEME, DARK_SELECTION);
    let plain = renderer
        .layout_with_styles(text, &options(), &colors, &[style(false, false)])
        .unwrap();
    let plain = renderer.rasterize(&plain, view, Some(4..10)).unwrap();
    for (underline, strikethrough) in [(true, false), (false, true)] {
        let page = renderer
            .layout_with_styles(
                text,
                &options(),
                &colors,
                &[style(underline, strikethrough)],
            )
            .unwrap();
        let selected = renderer.rasterize(&page, view, Some(4..10)).unwrap();
        for glyph in 0..text.len() {
            // The line's own pixels: where the decorated paint differs from the plain one.
            let cell = cells(&page, glyph..glyph + 1);
            let line: Vec<[u8; 4]> = pixels(&selected, cell)
                .into_iter()
                .zip(pixels(&plain, cell))
                .filter(|(decorated, plain)| decorated != plain)
                .map(|(decorated, _)| decorated)
                .collect();
            assert!(!line.is_empty(), "glyph {glyph} has its line");
            if glyph < 4 {
                assert!(line.contains(&DARK_BLUE), "unselected {glyph}: {line:?}");
            } else {
                assert!(!line.contains(&DARK_BLUE), "selected {glyph}: {line:?}");
                let ink = strongest(&line, DARK_SELECTION);
                assert!(contrast(ink, DARK_SELECTION) >= 4.49, "{glyph}: {ink:?}");
            }
        }
    }
}

#[test]
fn without_a_visible_selection_glyphs_keep_their_colors() {
    let mut renderer = TextRenderer::default();
    let text = "MMMM MMMM";
    let colors = [ColorSpan {
        range: 0..4,
        color: DARK_BLUE,
    }];
    let page = layout(&mut renderer, text, &colors);
    let view = viewport(DARK_THEME, DARK_SELECTION);
    let before = renderer.rasterize(&page, view, None).unwrap();
    let selected = renderer
        .rasterize(&page, view, Some(0..text.len()))
        .unwrap();
    assert!(selected.rgba != before.rgba);
    let after = renderer.rasterize(&page, view, None).unwrap();
    assert!(
        after.rgba == before.rgba,
        "selected brushes stay with the selection"
    );
    let invisible = viewport(DARK_THEME, [255, 255, 255, 0]);
    let hidden = renderer
        .rasterize(&page, invisible, Some(0..text.len()))
        .unwrap();
    assert!(
        hidden.rgba == before.rgba,
        "a transparent selection changes nothing"
    );
}

#[test]
fn a_selection_text_color_paints_every_selected_glyph_and_its_underline() {
    const WHITE: [u8; 4] = [255, 255, 255, 255];
    let mut renderer = TextRenderer::default();
    // Three colored words; the last stays unselected.
    let text = "MMMM MMMM MMMM";
    let colors = [
        ColorSpan {
            range: 0..4,
            color: DARK_BLUE,
        },
        ColorSpan {
            range: 5..9,
            color: LIGHT_TOKEN,
        },
        ColorSpan {
            range: 10..14,
            color: DARK_BLUE,
        },
    ];
    let underline = StyleSpan {
        range: 5..9,
        weight: 400,
        italic: false,
        underline: true,
        strikethrough: false,
        monospace: false,
        size: options().size,
    };
    let page = renderer
        .layout_with_styles(text, &options(), &colors, &[underline])
        .unwrap();
    let view = Viewport {
        selection_text: WHITE,
        ..viewport(DARK_THEME, DARK_SELECTION)
    };
    let plain = renderer.rasterize(&page, view, None).unwrap();
    let selected = renderer.rasterize(&page, view, Some(0..9)).unwrap();
    for word in [0..4, 5..9] {
        let cell = cells(&page, word.clone());
        let painted = pixels(&selected, cell);
        // Fully covered stems show the text color itself; no pixel keeps a glyph color.
        assert_eq!(strongest(&painted, DARK_SELECTION), WHITE, "{word:?}");
        assert!(
            !painted.contains(&DARK_BLUE) && !painted.contains(&LIGHT_TOKEN),
            "{word:?}"
        );
        // Every other pixel is the fill or a blend of the fill towards the text color.
        for pixel in painted {
            assert!(
                (0..3).all(|channel| pixel[channel] >= DARK_SELECTION[channel]),
                "{word:?}: {pixel:?}"
            );
        }
    }
    let unselected = cells(&page, 10..14);
    assert!(pixels(&selected, unselected) == pixels(&plain, unselected));
    assert!(pixels(&selected, unselected).contains(&DARK_BLUE));
}

#[test]
fn a_selected_wrapped_paragraph_has_no_gaps_between_its_lines() {
    let mut renderer = TextRenderer::default();
    let text = "MMMM MMMM MMMM MMMM MMMM MMMM MMMM MMMM MMMM MMMM MMMM MMMM";
    let blocks = [DocumentBlock {
        range: 0..text.len(),
        kind: DocumentKind::Paragraph,
        depth: 0,
    }];
    // Generous line spacing, as the Markdown presets use.
    let page = renderer
        .layout_document(
            text,
            &TextOptions {
                size: 18.0,
                wrap_width: Some(200.0),
                line_height: Some(1.6),
                ..TextOptions::default()
            },
            &DocumentStyle {
                colors: &[],
                styles: &[],
                tables: &[],
                blocks: &blocks,
                background: DARK_THEME,
                start_of_document: false,
            },
        )
        .unwrap();
    let lines = page.selection_geometry(0..text.len()).unwrap();
    assert!(lines.len() >= 3, "the paragraph wraps: {lines:?}");
    let view = Viewport {
        selection_text: [255; 4],
        ..viewport(DARK_THEME, DARK_SELECTION)
    };
    let selected = renderer
        .rasterize(&page, view, Some(0..text.len()))
        .unwrap();
    // Down the paragraph's left edge, every row from the first line's top to the last
    // line's bottom is the selection (or selected ink on it), never the page.
    let left = lines
        .iter()
        .map(|rect| rect.x0)
        .fold(f64::INFINITY, f64::min);
    let top = lines
        .iter()
        .map(|rect| rect.y0)
        .fold(f64::INFINITY, f64::min);
    let bottom = lines.iter().map(|rect| rect.y1).fold(0.0, f64::max);
    let x = left.ceil() as usize;
    for y in top.ceil() as usize..bottom.floor() as usize {
        assert_ne!(pixel(&selected, x, y), DARK_THEME, "row {y} between lines");
    }
}
