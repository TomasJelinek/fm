//! A page of row groups answers and paints as one layout of the same rows, across every group
//! edge, and a page that repeats rows of the page before it shapes only the rows it adds.
use super::*;
use crate::{Rect, Viewport};

/// Rows of every kind a group edge can fall between: short and long rows, an empty row,
/// tabs, CRLF, combining marks, wide scripts and fallback fonts (whose rows reach above and
/// below their line), other line separators inside a group, and a last row without a line
/// break.
const TEXTS: [&str; 3] = [
    "row 000 abc\nrow 001 café 東京\n\n\tindented\ttab\nCRLF row\r\nx\ny\nlast row, no end",
    "e\u{301}\u{302} a\u{308} 👩\u{200d}💻 👍🏽 🇨🇿🇩🇪\nz\u{335}\u{336}a\u{337}lgo\none\u{2028}two\u{85}three\n",
    "中文文本，日本語のテキスト。\n한국어 텍스트\nक्षत्रिय हिन्दी\n\n\n",
];

/// Rows with right-to-left letters and Arabic numbers, first or later in their row, between
/// rows without them.
const BIDI_TEXT: &str = "first plain row\nsecond plain\nLatin العربية עברית 123 end\n\
    العربية first, then English 4.5\n(1) after\nעברית (1) [x] 10%\n١٢٣ then Latin\nlast plain\n";

fn options() -> [TextOptions<'static>; 2] {
    [
        TextOptions::default(),
        TextOptions {
            family: "serif",
            size: 15.0,
            scale: 1.25,
            italic: true,
            weight: 700,
            tabular_numbers: true,
            tab_width: 8,
            line_height: Some(1.5),
            ..TextOptions::default()
        },
    ]
}

/// A long page: 300 numbered rows of 3 to 200 characters, some coloured and styled.
fn long_text() -> (String, Vec<ColorSpan>, Vec<StyleSpan>) {
    let mut text = String::new();
    let mut colors = Vec::new();
    let mut styles = Vec::new();
    for row in 0..300 {
        let start = text.len();
        let width = 3 + (row * 37) % 198;
        text += &format!("{row:03} ");
        text += &"abcdefghij ".repeat(width / 11);
        text += "\n";
        if row % 7 == 3 {
            colors.push(ColorSpan {
                range: start..start + 3,
                color: [200, 40, 40, 255],
            });
        }
        if row % 11 == 5 {
            styles.push(StyleSpan {
                range: start + 4..text.len() - 1,
                weight: 700,
                italic: false,
                underline: true,
                strikethrough: false,
                monospace: false,
                size: 18.0,
            });
        }
    }
    (text, colors, styles)
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

fn same_rect(a: &Rect, b: &Rect) -> bool {
    close(a.x0, b.x0) && close(a.y0, b.y0) && close(a.x1, b.x1) && close(a.y1, b.y1)
}

/// The start of every group of a page of row groups.
fn group_starts(page: &TextPage) -> Vec<usize> {
    page.composite
        .as_ref()
        .expect("a page of row groups")
        .parts
        .iter()
        .map(|part| part.range.start)
        .collect()
}

/// Whether every line of `page`, a page or a composite of pages, advances by its clusters'
/// advances. Parley 0.11.1 counts a cluster at a wrong index in a line's advance when a
/// ligature, combining sequence or emoji sequence starts a cluster outside the layout's
/// first run: its line breaker passes an absolute cluster index to `Run::get`, which takes
/// one relative to the run (`line_break.rs`, `run.get(self.state.cluster_idx + 1)`). Such a
/// layout's width and the highlight of a line selected whole then depend on other rows of
/// the same layout; carets, hit tests, glyphs and copies do not.
fn exact_advances(page: &TextPage) -> bool {
    parts(page).iter().all(|page| {
        page.layout.lines().all(|line| {
            let clusters: f32 = line
                .runs()
                .map(|run| run.clusters().map(|cluster| cluster.advance()).sum::<f32>())
                .sum();
            (clusters - line.metrics().advance).abs() < 1e-3
        })
    })
}

/// The layouts of `page`: its parts', or its own.
fn parts(page: &TextPage) -> Vec<&TextPage> {
    match &page.composite {
        Some(grid) => grid.parts.iter().map(|part| &part.page).collect(),
        None => vec![page],
    }
}

/// The top and bottom of every line of `page`, a page or a composite of pages.
fn line_edges(page: &TextPage) -> Vec<f32> {
    parts(page)
        .iter()
        .flat_map(|page| page.layout.lines())
        .flat_map(|line| {
            [
                line.metrics().block_min_coord,
                line.metrics().block_max_coord,
            ]
        })
        .collect()
}

/// Offsets around every group edge: the edge, the last characters before it (the last column
/// and the line break of the group's last row) and the first ones after it.
fn edge_offsets(text: &str, starts: &[usize]) -> Vec<usize> {
    let mut offsets = Vec::new();
    for &start in starts {
        for delta in -3_isize..=3 {
            if let Some(at) = start.checked_add_signed(delta)
                && at <= text.len()
                && text.is_char_boundary(at)
            {
                offsets.push(at);
            }
        }
    }
    offsets.push(text.len());
    offsets.sort_unstable();
    offsets.dedup();
    offsets
}

/// Asserts that `rows` answers and paints as `one`, one layout of the same text.
fn assert_like_one_layout(
    renderer: &mut TextRenderer,
    one: &TextPage,
    rows: &TextPage,
    what: &str,
) {
    let text = one.text();
    assert_eq!(text, rows.text(), "{what}: text");
    // Widths, and the highlight of a line selected whole, follow parley's line advances.
    let exact = exact_advances(one) && exact_advances(rows);
    assert!(
        !exact || close(f64::from(one.width()), f64::from(rows.width())),
        "{what}: width"
    );
    assert!(
        close(f64::from(one.height()), f64::from(rows.height())),
        "{what}: height"
    );
    assert_eq!(one.line_count(), rows.line_count(), "{what}: lines");
    let (anchors, grouped) = (one.scroll_anchors(), rows.scroll_anchors());
    assert_eq!(
        anchors.len(),
        grouped.len(),
        "{what}: scroll anchors {anchors:?} {grouped:?}"
    );
    for (a, b) in anchors.iter().zip(&grouped) {
        assert!(
            a.0 == b.0 && close(f64::from(a.1), f64::from(b.1)),
            "{what}: anchor {a:?} {b:?}"
        );
    }
    for index in (0..=text.len()).filter(|&at| text.is_char_boundary(at)) {
        match (one.caret_geometry(index), rows.caret_geometry(index)) {
            (Ok(a), Ok(b)) => assert!(same_rect(&a, &b), "{what}: caret {index} {a:?} {b:?}"),
            (a, b) => assert_eq!(a, b, "{what}: caret {index}"),
        }
    }
    let (width, height) = (one.width(), one.height());
    let mut ys: Vec<f32> = (0..(height / 5.7) as usize + 2)
        .map(|row| row as f32 * 5.7 - 2.0)
        .collect();
    // On and around every line's top and bottom: where one group ends and the next begins,
    // and where a row's box reaches past its line (a fallback font's taller row).
    for edge in line_edges(one).into_iter().chain(line_edges(rows)) {
        ys.extend([edge, edge - 0.01, edge + 0.01]);
    }
    for &y in &ys {
        for column in 0..(width / 4.3) as usize + 2 {
            let x = column as f32 * 4.3 - 3.0;
            assert_eq!(
                one.hit_test(x, y),
                rows.hit_test(x, y),
                "{what}: hit test {x}, {y}"
            );
        }
        match (one.line_at_y(y), rows.line_at_y(y)) {
            (Ok((a, top_a)), Ok((b, top_b))) => assert!(
                a == b && close(f64::from(top_a), f64::from(top_b)),
                "{what}: line at {y}: {a:?} {top_a} {b:?} {top_b}"
            ),
            (a, b) => assert_eq!(a, b, "{what}: line at {y}"),
        }
    }
    let offsets = edge_offsets(text, &group_starts(rows));
    let between = offsets
        .iter()
        .enumerate()
        .flat_map(|(i, &start)| offsets[i..].iter().map(move |&end| start..end));
    let ranges: Vec<Range<usize>> = std::iter::once(0..text.len()).chain(between).collect();
    for range in &ranges {
        let (a, b) = (
            one.selection_geometry(range.clone()).unwrap(),
            rows.selection_geometry(range.clone()).unwrap(),
        );
        // Lines between the first and the last are highlighted by their advance.
        let whole_lines = anchors
            .iter()
            .filter(|(at, _)| range.start < *at && *at < range.end)
            .count()
            > 1;
        assert!(
            (whole_lines && !exact)
                || a.len() == b.len() && a.iter().zip(&b).all(|(a, b)| same_rect(a, b)),
            "{what}: selection {range:?} {a:?} {b:?}"
        );
        assert_eq!(
            one.selected_text(range.clone()),
            rows.selected_text(range.clone()),
            "{what}: copy {range:?}"
        );
    }
    let viewport = Viewport {
        width: width.ceil() as u32 + 4,
        height: height.ceil() as u32 + 4,
        origin: [-2.0, -2.0],
        background: [250, 250, 245, 255],
        selection_color: [40, 80, 120, 255],
        selection_text: [0; 4],
    };
    // A selection that spans group edges (and a Find match, painted the same way).
    let spanning = offsets.first().copied().unwrap_or(0)..offsets.last().copied().unwrap_or(0);
    for selection in [None, exact.then_some(spanning)] {
        let expected = renderer
            .rasterize(one, viewport, selection.clone())
            .unwrap();
        let painted = renderer
            .rasterize(rows, viewport, selection.clone())
            .unwrap();
        assert!(
            expected.rgba == painted.rgba,
            "{what}: pixels with {selection:?}"
        );
    }
}

/// Every group size, from a group per row to one group, answers and paints as one layout.
#[test]
fn row_groups_answer_and_paint_as_one_layout() {
    let mut renderer = TextRenderer::default();
    for (set, options) in options().iter().enumerate() {
        for text in TEXTS {
            let one = renderer
                .layout_with_styles(text, options, &[], &[])
                .unwrap();
            for group in [1, 7, 40, GROUP_BYTES] {
                let (rows, layout) = renderer
                    .layout_rows_with(text, options, &[], &[], None, group)
                    .unwrap();
                assert_eq!(layout.shaped_bytes, text.len());
                assert_like_one_layout(
                    &mut renderer,
                    &one,
                    &rows,
                    &format!("{text:?} {set} {group}"),
                );
            }
        }
    }
}

/// A row with right-to-left letters or Arabic numbers is a paragraph of its own: a group of
/// its own, ordered by its own first strong character, whatever the group size or the row
/// the page starts with. Each group size answers and paints as a group per row.
#[test]
fn a_row_with_right_to_left_text_is_a_paragraph_of_its_own() {
    let mut renderer = TextRenderer::default();
    let mut start = 0;
    let rows: Vec<Range<usize>> = BIDI_TEXT
        .split_inclusive('\n')
        .map(|row| {
            start += row.len();
            start - row.len()..start
        })
        .collect();
    let bidi = |row: &Range<usize>| bidirectional(&BIDI_TEXT[row.clone()]);
    assert_eq!(
        rows.iter().map(bidi).collect::<Vec<_>>(),
        [false, false, true, true, false, true, true, false]
    );
    for (set, options) in options().iter().enumerate() {
        // A page that starts with a right-to-left row orders each row as one that starts
        // earlier does.
        let later = rows[3].start;
        let (whole, _) = renderer
            .layout_rows(BIDI_TEXT, options, &[], &[], None)
            .unwrap();
        let (tail, _) = renderer
            .layout_rows(&BIDI_TEXT[later..], options, &[], &[], None)
            .unwrap();
        let top = whole.caret_geometry(later).unwrap().y0 - tail.caret_geometry(0).unwrap().y0;
        for index in (later..=BIDI_TEXT.len()).filter(|&at| BIDI_TEXT.is_char_boundary(at)) {
            let (a, b) = (
                whole.caret_geometry(index).unwrap(),
                tail.caret_geometry(index - later).unwrap(),
            );
            assert!(
                close(a.x0, b.x0) && close(a.x1, b.x1) && close(a.y0 - top, b.y0),
                "{set}: caret {index} {a:?} {b:?}"
            );
        }
        let (each, _) = renderer
            .layout_rows_with(BIDI_TEXT, options, &[], &[], None, 1)
            .unwrap();
        for group in [7, 40, GROUP_BYTES] {
            let (grouped, _) = renderer
                .layout_rows_with(BIDI_TEXT, options, &[], &[], None, group)
                .unwrap();
            let groups = grouped.row_groups().unwrap();
            for row in rows.iter().filter(|row| bidi(row)) {
                assert!(groups.contains(row), "{set} {group}: {row:?} in {groups:?}");
            }
            assert_like_one_layout(
                &mut renderer,
                &each,
                &grouped,
                &format!("bidi {set} {group}"),
            );
        }
    }
}

/// Groups stay within one layout's colour spans and lines: rows that one layout cannot
/// take, densely coloured or empty, are laid out in more, smaller groups.
#[test]
fn groups_stay_within_one_layouts_limits() {
    let mut renderer = TextRenderer::default();
    let options = TextOptions::default();
    let coloured = "abcdefghijklmnopqrstuvwxyz0123\n".repeat(400);
    let spans: Vec<ColorSpan> = (0..coloured.len())
        .filter(|&at| coloured.as_bytes()[at] != b'\n')
        .map(|at| ColorSpan {
            range: at..at + 1,
            color: [(at % 251) as u8, 40, 40, 255],
        })
        .collect();
    let empty = "\n".repeat(5000);
    for (text, spans) in [(&coloured, &spans[..]), (&empty, &[][..])] {
        assert_eq!(
            renderer
                .layout_with_styles(text, &options, spans, &[])
                .err(),
            Some(LayoutError::LimitExceeded)
        );
        let (rows, layout) = renderer
            .layout_rows(text, &options, spans, &[], None)
            .unwrap();
        let groups = rows.row_groups().unwrap();
        assert!(layout.groups > 1, "{layout:?}");
        for group in &groups {
            assert!(colors_in(spans, group) <= crate::MAX_SPANS, "{group:?}");
            assert!(crate::hard_line_count(&text[group.clone()]) <= crate::MAX_LINES);
        }
        assert_eq!(rows.line_count(), text.lines().count() + 1);
    }
}

/// Coloured and styled long rows in many groups.
#[test]
fn coloured_long_rows_in_groups_answer_and_paint_as_one_layout() {
    let mut renderer = TextRenderer::default();
    let (text, colors, styles) = long_text();
    let options = TextOptions::default();
    let one = renderer
        .layout_with_styles(&text, &options, &colors, &styles)
        .unwrap();
    let (rows, layout) = renderer
        .layout_rows_with(&text, &options, &colors, &styles, None, 1000)
        .unwrap();
    assert!(layout.groups > 10, "{layout:?}");
    assert_like_one_layout(&mut renderer, &one, &rows, "long rows");
}

/// The offset of row `row` of `text`.
fn row_start(text: &str, row: usize) -> usize {
    if row == 0 {
        return 0;
    }
    text.match_indices('\n')
        .nth(row - 1)
        .map_or(text.len(), |(at, _)| at + 1)
}

/// A refill forwards keeps the rows it shares with the page before it: it shapes the rows it
/// adds and the group its new first row cuts, and it answers and paints as one layout.
#[test]
fn a_refill_shapes_only_the_rows_it_adds() {
    let mut renderer = TextRenderer::default();
    let (text, colors, styles) = long_text();
    let options = TextOptions::default();
    let piece = |from: usize, to: usize| -> (Range<usize>, Vec<ColorSpan>, Vec<StyleSpan>) {
        let range = row_start(&text, from)..row_start(&text, to);
        (
            range.clone(),
            clipped_colors(&colors, &range),
            Styles::new(&styles).clip(&range),
        )
    };
    let group = 1000;
    let (first, first_colors, first_styles) = piece(20, 120);
    let (old, _) = renderer
        .layout_rows_with(
            &text[first.clone()],
            &options,
            &first_colors,
            &first_styles,
            None,
            group,
        )
        .unwrap();
    for (from, to, backwards) in [(35, 150, false), (5, 110, true)] {
        let (range, range_colors, range_styles) = piece(from, to);
        let shift = first.start as isize - range.start as isize;
        let (new, layout) = renderer
            .layout_rows_with(
                &text[range.clone()],
                &options,
                &range_colors,
                &range_styles,
                Some((&old, shift)),
                group,
            )
            .unwrap();
        // The groups of the old page that the new one holds whole, at the same place.
        let page = &text[range.clone()];
        let kept: usize = old
            .composite
            .as_ref()
            .unwrap()
            .parts
            .iter()
            .filter(|part| {
                let start = part.range.start as isize + shift;
                start >= 0
                    && start as usize + part.range.len() <= page.len()
                    && (start == 0 || page.as_bytes()[start as usize - 1] == b'\n')
            })
            .map(|part| part.range.len())
            .sum();
        assert_eq!(layout.reused_bytes, kept, "{from}..{to}: {layout:?}");
        assert_eq!(layout.reused_bytes + layout.shaped_bytes, range.len());
        // Only the added rows and the one group that the new edge cuts are shaped.
        let added = if backwards {
            row_start(&text, 20) - row_start(&text, from)
        } else {
            row_start(&text, to) - row_start(&text, 120)
        };
        assert!(
            layout.shaped_bytes <= added + group,
            "{from}..{to}: {layout:?}, {added} added"
        );
        let one = renderer
            .layout_with_styles(&text[range.clone()], &options, &range_colors, &range_styles)
            .unwrap();
        assert_like_one_layout(&mut renderer, &one, &new, &format!("refill {from}..{to}"));
    }
}

/// A group whose colours, styles or options differ is shaped again; a last row that the old
/// page cut is shaped again when the new page continues it.
#[test]
fn changed_groups_are_shaped_again() {
    let mut renderer = TextRenderer::default();
    let (text, colors, styles) = long_text();
    let options = TextOptions::default();
    let (old, _) = renderer
        .layout_rows_with(&text, &options, &colors, &styles, None, 1000)
        .unwrap();
    // One colour more, in the third group: that group alone is shaped again.
    let third = group_starts(&old)[2];
    let mut recoloured = colors.clone();
    recoloured.push(ColorSpan {
        range: third + 4..third + 5,
        color: [1, 2, 3, 255],
    });
    recoloured.sort_by_key(|span| span.range.start);
    let (_, layout) = renderer
        .layout_rows_with(&text, &options, &recoloured, &styles, Some((&old, 0)), 1000)
        .unwrap();
    let parts = &old.composite.as_ref().unwrap().parts;
    assert_eq!(layout.shaped_bytes, parts[2].range.len(), "{layout:?}");
    // Other options: nothing is reused.
    let larger = TextOptions {
        size: 19.0,
        ..options
    };
    let (_, layout) = renderer
        .layout_rows_with(&text, &larger, &colors, &styles, Some((&old, 0)), 1000)
        .unwrap();
    assert_eq!(layout.reused_groups, 0);
    // A page cut inside a row, then continued: its last group is shaped again.
    let cut = row_start(&text, 50) + 2;
    let (short, _) = renderer
        .layout_rows_with(&text[..cut], &options, &[], &[], None, 1000)
        .unwrap();
    let (_, layout) = renderer
        .layout_rows_with(
            &text[..row_start(&text, 60)],
            &options,
            &[],
            &[],
            Some((&short, 0)),
            1000,
        )
        .unwrap();
    let last = short
        .composite
        .as_ref()
        .unwrap()
        .parts
        .last()
        .unwrap()
        .range
        .clone();
    assert_eq!(layout.reused_bytes, last.start, "{layout:?}");
}
