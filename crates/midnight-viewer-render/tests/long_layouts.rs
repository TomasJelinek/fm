//! Layouts longer than 64 KiB. Parley keeps a glyph cluster's byte offset inside
//! its shaping run in 16 bits, and uniformly styled text is one shaping run. Each
//! test compares the caret, a hit test and a selection of bytes past 65,536 with
//! the same row laid out alone, a layout far below that limit.
use midnight_viewer_render::{Rect, TextOptions, TextPage, TextRenderer};

/// 16-pixel text with a 1.25 line height: visual rows exactly 20 pixels apart.
const ROW: f64 = 20.0;

fn options(wrap_width: Option<f32>) -> TextOptions<'static> {
    TextOptions {
        family: "monospace",
        size: 16.0,
        line_height: Some(1.25),
        wrap_width,
        ..TextOptions::default()
    }
}

/// A 99-byte row, distinct by its number, every character a cluster of its own.
fn row(number: usize) -> String {
    let mut row = format!("{number:05}:");
    while row.len() < 99 {
        row.push(char::from(b'a' + (row.len() % 26) as u8));
    }
    row
}

fn assert_close(actual: Rect, alone: Rect, top: f64, what: &str) {
    let actual = [actual.x0, actual.y0, actual.x1, actual.y1];
    let expected = [alone.x0, alone.y0 + top, alone.x1, alone.y1 + top];
    assert!(
        actual
            .iter()
            .zip(expected)
            .all(|(actual, expected)| (actual - expected).abs() < 0.01),
        "{what} is at {actual:?}, the row alone puts it at {expected:?}"
    );
}

/// `alone` is the row starting at byte `start` of `page`, laid out by itself; the
/// row's first visual line is `top` pixels down the page.
fn assert_row(page: &TextPage, start: usize, top: f64, alone: &TextPage, columns: &[usize]) {
    for &column in columns {
        let index = start + column;
        let caret = alone.caret_geometry(column).unwrap();
        let what = format!("the caret of byte {index}");
        assert_close(page.caret_geometry(index).unwrap(), caret, top, &what);
        // One pixel into the character's glyph (or a tab's first space).
        let x = caret.x0 as f32 + 1.0;
        let y = ((caret.y0 + caret.y1) / 2.0) as f32;
        assert_eq!(
            page.hit_test(x, y + top as f32).unwrap(),
            start + alone.hit_test(x, y).unwrap(),
            "hit test at the caret of byte {index}"
        );
        let selected = page.selection_geometry(index..index + 3).unwrap();
        let expected = alone.selection_geometry(column..column + 3).unwrap();
        assert_eq!(
            selected.len(),
            expected.len(),
            "selection of bytes {index}..{}: {selected:?}",
            index + 3
        );
        for (selected, expected) in selected.into_iter().zip(expected) {
            let what = format!("the selection of bytes {index}..{}", index + 3);
            assert_close(selected, expected, top, &what);
        }
    }
}

#[test]
fn carets_hit_tests_and_selections_hold_past_64_kib_of_unwrapped_rows() {
    const ROWS: usize = 1000;
    let mut renderer = TextRenderer::default();
    let text: String = (0..ROWS).map(|number| row(number) + "\n").collect();
    assert_eq!(text.len(), 100_000);
    let page = renderer
        .layout_with_options(&text, &options(None), &[])
        .unwrap();
    assert_eq!(page.line_count(), ROWS + 1);
    // Byte 65,536 is column 36 of row 655.
    for number in [3, 655, 700, 999] {
        let alone = renderer
            .layout_with_options(&row(number), &options(None), &[])
            .unwrap();
        let top = number as f64 * ROW;
        assert_row(&page, number * 100, top, &alone, &[0, 1, 30, 40, 95]);
    }
}

#[test]
fn carets_hit_tests_and_selections_hold_past_64_kib_of_wrapped_rows() {
    const ROWS: usize = 1000;
    let mut renderer = TextRenderer::default();
    let text: String = (0..ROWS).map(|number| row(number) + "\n").collect();
    let wrapped = options(Some(300.0));
    let page = renderer.layout_with_options(&text, &wrapped, &[]).unwrap();
    for number in [3, 655, 700, 999] {
        let alone = renderer
            .layout_with_options(&row(number), &wrapped, &[])
            .unwrap();
        let lines = alone.line_count();
        assert!(lines > 1, "a row wraps");
        assert_eq!(page.line_count(), ROWS * lines + 1);
        let top = (number * lines) as f64 * ROW;
        assert_row(&page, number * 100, top, &alone, &[0, 1, 30, 40, 95]);
    }
}

#[test]
fn carets_hit_tests_and_selections_hold_where_tab_expansion_passes_64_kib() {
    const ROWS: usize = 4000;
    // 13 source bytes, 22 laid out: three tabs widen to four spaces each.
    let row = |number: usize| format!("\t{number:04}\t\tv{number:04}");
    let mut renderer = TextRenderer::default();
    let text: String = (0..ROWS).map(|number| row(number) + "\n").collect();
    assert_eq!(text.len(), 52_000, "the source stays below 64 KiB");
    let page = renderer
        .layout_with_options(&text, &options(None), &[])
        .unwrap();
    assert_eq!(page.line_count(), ROWS + 1);
    // Row 2979 is the first to start past 65,536 laid-out bytes.
    for number in [3, 2979, 3500, 3999] {
        let alone = renderer
            .layout_with_options(&row(number), &options(None), &[])
            .unwrap();
        let top = number as f64 * ROW;
        assert_row(&page, number * 13, top, &alone, &[0, 1, 5, 6, 7, 9]);
    }
}
