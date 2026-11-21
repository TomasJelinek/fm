use super::{alternate_pieces, piece_starts};
use crate::source_map::SourceMap;
use crate::{
    ColorSpan, LayoutError, StyleSpan, TextOptions, TextPage, TextRenderer, Viewport, is_line_break,
};
use parley::PositionedLayoutItem;
use std::{collections::HashSet, ops::Range};
use unicode_segmentation::UnicodeSegmentation;

#[test]
fn pieces_end_at_line_starts_then_after_whitespace_then_between_grapheme_clusters() {
    assert_eq!(piece_starts("aaa\nbbb\nccc\n", 5), Ok(vec![4, 8]));
    assert_eq!(piece_starts("aaa\nbbb\nccc\n", 8), Ok(vec![8]));
    assert_eq!(piece_starts("aaa\nbbb\nccc\n", 12), Ok(vec![]));
    assert_eq!(piece_starts("ab\u{2028}cd\u{85}ef", 6), Ok(vec![5]));
    assert_eq!(piece_starts("alpha beta gamma", 12), Ok(vec![11]));
    // Byte 4 separates the second e from its accent, inside one grapheme cluster.
    assert_eq!(piece_starts("e\u{301}e\u{301}e\u{301}", 4), Ok(vec![3, 6]));
    assert_eq!(
        piece_starts(&"👩\u{200d}💻".repeat(3), 12),
        Ok(vec![11, 22])
    );
    // One grapheme cluster longer than a piece is cut between its characters.
    assert_eq!(
        piece_starts(&format!("e{}", "\u{301}".repeat(10)), 8),
        Ok(vec![7, 15])
    );
    assert_eq!(piece_starts("éé", 1), Err(LayoutError::LimitExceeded));
    assert_eq!(
        alternate_pieces("aaa\nbbb\nccc\nddd\n", 4),
        Ok(vec![4..8, 12..16])
    );
}

#[test]
fn every_piece_fits_and_ends_between_grapheme_clusters() {
    let text = format!(
        "{}{}",
        "Žluťoučký kůň pěl ódy.\nالعربية עברית 👩\u{200d}💻🇨🇿 e\u{301}\u{2029}".repeat(40),
        "x\u{301}".repeat(200)
    );
    let clusters: HashSet<usize> = text.grapheme_indices(true).map(|(at, _)| at).collect();
    for limit in [11, 16, 61, 256] {
        let mut start = 0;
        for end in piece_starts(&text, limit)
            .unwrap()
            .into_iter()
            .chain([text.len()])
        {
            assert!(
                (1..=limit).contains(&(end - start)),
                "piece {start}..{end} for {limit}"
            );
            assert!(end == text.len() || clusters.contains(&end));
            start = end;
        }
    }
}

const TEXTS: [&str; 6] = [
    "AVATAR Wave To. office affluent\n\"Quoted,\" she said; fi fl ffi.\nLast line, no end",
    "Latin العربية עברית 123 end\nالعربية first, then English 4.5\nעברית (1) [x] 10%\n",
    "e\u{301}\u{302} a\u{308} 👩\u{200d}💻 👍🏽 🇨🇿🇩🇪\nz\u{335}\u{336}a\u{337}lgo\n",
    "中文文本，日本語のテキスト。\n한국어 텍스트\nक्षत्रिय हिन्दी\n",
    "a\tb\tc\r\n\tindented\r\nx\ty\n",
    "one\u{2028}two\u{85}three\u{2029}four\u{b}five\u{c}six",
];

fn option_sets() -> [TextOptions<'static>; 3] {
    [
        TextOptions::default(),
        TextOptions {
            family: "sans-serif",
            size: 15.0,
            scale: 1.25,
            wrap_width: Some(140.0),
            line_height: Some(1.5),
            ..TextOptions::default()
        },
        TextOptions {
            family: "serif",
            italic: true,
            weight: 700,
            tabular_numbers: true,
            tab_width: 8,
            ..TextOptions::default()
        },
    ]
}

/// Paint and typography spans at character boundaries of `text`.
fn spans(text: &str) -> (Vec<ColorSpan>, Vec<StyleSpan>) {
    let boundaries: Vec<usize> = text
        .char_indices()
        .map(|(at, _)| at)
        .chain([text.len()])
        .collect();
    let at = |character: usize| boundaries[character.min(boundaries.len() - 1)];
    let colors = vec![
        ColorSpan {
            range: at(2)..at(7),
            color: [200, 40, 40, 255],
        },
        ColorSpan {
            range: at(12)..at(20),
            color: [30, 120, 30, 255],
        },
    ];
    let style = |range: Range<usize>, size: f32, bold: bool| StyleSpan {
        range,
        weight: if bold { 700 } else { 400 },
        italic: !bold,
        underline: bold,
        strikethrough: !bold,
        monospace: !bold,
        size,
    };
    (
        colors,
        vec![
            style(at(3)..at(9), 22.0, true),
            style(at(15)..at(24), 16.0, false),
        ],
    )
}

/// Everything a layout gives the painter, line by line.
fn glyphs(page: &TextPage) -> Vec<String> {
    page.layout
        .lines()
        .map(|line| {
            let mut description = format!(
                "{:?} {:?} {:?}",
                line.metrics(),
                line.text_range(),
                line.break_reason()
            );
            for item in line.items() {
                let PositionedLayoutItem::GlyphRun(glyph_run) = item else {
                    continue;
                };
                let run = glyph_run.run();
                let font = run.font();
                for glyph in glyph_run.positioned_glyphs() {
                    description += &format!(
                        " {}@{},{}+{} font {}#{} {} {:?} {:?}",
                        glyph.id,
                        glyph.x,
                        glyph.y,
                        glyph.advance,
                        font.data.id(),
                        font.index,
                        run.font_size(),
                        run.normalized_coords(),
                        glyph_run.style()
                    );
                }
            }
            description
        })
        .collect()
}

fn selections(text: &str) -> Vec<Range<usize>> {
    let boundaries: Vec<usize> = (0..=text.len())
        .filter(|&at| text.is_char_boundary(at))
        .collect();
    let at = |index: usize| boundaries[index];
    let count = boundaries.len() - 1;
    let mut ranges = vec![
        0..text.len(),
        at(1)..at(count - 1),
        at(count / 4)..at(count * 3 / 4),
    ];
    let mut start = 0;
    for line in text.split_inclusive(is_line_break) {
        ranges.push(start..start + line.len());
        start += line.len();
    }
    ranges
}

/// Asserts that `pieces` lays out, answers and paints exactly as `full`.
fn assert_same(renderer: &mut TextRenderer, full: &TextPage, pieces: &TextPage, what: &str) {
    assert_eq!(glyphs(full), glyphs(pieces), "{what}: glyphs");
    let text = full.text();
    for index in (0..=text.len()).filter(|&at| text.is_char_boundary(at)) {
        let caret = full.caret_geometry(index);
        assert_eq!(caret, pieces.caret_geometry(index), "{what}: caret {index}");
    }
    let (width, height) = (full.width(), full.height());
    for row in 0..(height / 5.7) as usize + 2 {
        for column in 0..(width / 4.3) as usize + 2 {
            let (x, y) = (column as f32 * 4.3 - 3.0, row as f32 * 5.7 - 2.0);
            let index = full.hit_test(x, y);
            assert_eq!(index, pieces.hit_test(x, y), "{what}: hit test {x}, {y}");
        }
    }
    let ranges = selections(text);
    for range in &ranges {
        let boxes = full.selection_geometry(range.clone());
        assert_eq!(
            boxes,
            pieces.selection_geometry(range.clone()),
            "{what}: {range:?}"
        );
    }
    let viewport = Viewport {
        width: width.ceil() as u32 + 4,
        height: height.ceil() as u32 + 4,
        origin: [-2.0, -2.0],
        background: [250, 250, 245, 255],
        selection_color: [40, 80, 120, 255],
    };
    for selection in [None, Some(ranges[2].clone())] {
        let expected = renderer.rasterize(full, viewport, selection.clone());
        let painted = renderer.rasterize(pieces, viewport, selection);
        assert!(
            expected.unwrap().rgba == painted.unwrap().rgba,
            "{what}: pixels"
        );
    }
}

#[test]
fn runs_cut_at_line_starts_keep_every_glyph_caret_hit_selection_and_pixel() {
    let mut renderer = TextRenderer::default();
    for text in TEXTS {
        for (set, options) in option_sets().iter().enumerate() {
            let (shaped, _) = SourceMap::prepare(text, options.tab_width).unwrap();
            // Every line fits a piece, so every piece ends at a line start.
            let limit = shaped
                .split_inclusive(is_line_break)
                .map(str::len)
                .max()
                .unwrap();
            let starts = piece_starts(&shaped, limit).unwrap();
            assert!(!starts.is_empty(), "{text:?} is cut");
            assert!(
                starts
                    .iter()
                    .all(|&at| shaped[..at].ends_with(is_line_break))
            );
            let (colors, styles) = if set == 0 {
                spans(text)
            } else {
                (Vec::new(), Vec::new())
            };
            let full = renderer
                .layout_with_styles(text, options, &colors, &styles)
                .unwrap();
            let pieces = renderer
                .layout_with_runs(text, options, &colors, &styles, limit)
                .unwrap();
            assert_same(&mut renderer, &full, &pieces, &format!("{text:?} {set}"));
        }
    }
}

/// A grapheme cluster longer than a run is cut between its characters; the rows after it
/// keep their carets.
#[test]
fn a_cluster_longer_than_a_run_is_cut_and_later_rows_stay_in_place() {
    let text = format!("e{}\nnext row", "\u{301}".repeat(40));
    let mut renderer = TextRenderer::default();
    let options = TextOptions::default();
    let full = renderer
        .layout_with_styles(&text, &options, &[], &[])
        .unwrap();
    let pieces = renderer
        .layout_with_runs(&text, &options, &[], &[], 16)
        .unwrap();
    let next = text.find('\n').unwrap() + 1;
    for index in next..=text.len() {
        let caret = full.caret_geometry(index);
        assert_eq!(caret, pieces.caret_geometry(index), "caret {index}");
    }
}

#[test]
fn runs_cut_after_whitespace_keep_monospace_glyphs_and_geometry() {
    let text = "The quick brown fox jumps over the lazy dog; 0123456789.\n".repeat(3)
        + "Žluťoučký kůň úpěl ďábelské ódy, naposledy.";
    let mut renderer = TextRenderer::default();
    let options = TextOptions::default();
    let full = renderer
        .layout_with_styles(&text, &options, &[], &[])
        .unwrap();
    let pieces = renderer
        .layout_with_runs(&text, &options, &[], &[], 20)
        .unwrap();
    let runs =
        |page: &TextPage| -> usize { page.layout.lines().map(|line| line.runs().count()).sum() };
    assert!(runs(&pieces) > runs(&full), "pieces end inside lines");
    assert_same(&mut renderer, &full, &pieces, "whitespace");
}
