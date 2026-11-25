//! Travel over long rows: the co-simulation of `worker_frontier_tests.rs`, in which the
//! real worker shows rows of 1200, 2400 and 8000 bytes, or a file mixing them with short
//! rows, from a simulated basic or Source viewer that answers one frame later. One page is
//! one layout of at most 120 KiB (`viewport::PAGE_BYTES`): the 76 rows of 1200 bytes its
//! line budget asks for, 51 of 2400 and 15 of 8000, while a 720-pixel window shows 44 rows
//! (76 with its guards). Every gesture must still move its whole distance, wrapped or not.
//! A check whose fault needs only a window taller than some pages runs in a lower window,
//! which still exceeds them and costs fewer page reads per gesture.
use super::frontier::{Engine, NOTCH, Window, frame, rest, rows, settle, try_gesture};
use super::*;
use std::fmt::Write as _;

/// An arrow key (`scrolling::keyboard`).
const ARROW: f64 = 28.0;
/// The window of the acceptance runs and of `measure_page_reads_by_row_width`.
const SIZE: ViewerGeometry = ViewerGeometry {
    width: 1312,
    height: 720,
    scale: 1.0,
};
/// A window of 30 rows: more than the 27 rows of 2400 bytes a 64 KiB page held, fewer
/// than the 51 a page holds now.
const ROWS_30: ViewerGeometry = ViewerGeometry {
    height: 500,
    ..SIZE
};
/// A window of 22 rows: more than the 15 rows of 8000 bytes a page holds.
const ROWS_22: ViewerGeometry = ViewerGeometry {
    height: 360,
    ..SIZE
};

/// Blocks of rows, `(count, width)` each, repeated until `total` bytes, numbered as
/// [`rows`] numbers them.
fn mixed(blocks: &[(usize, usize)], total: usize) -> String {
    let mut text = String::new();
    let mut row = 0;
    'fill: loop {
        for &(count, width) in blocks {
            for _ in 0..count {
                let prefix = format!("row {row:07} ");
                let _ = writeln!(text, "{prefix}{}", "x".repeat(width - prefix.len() - 1));
                row += 1;
                if text.len() >= total {
                    break 'fill;
                }
            }
        }
    }
    text
}

/// The problems of three scrollbar positions and of the gestures of the measurement after
/// them, a quarter into `source`, in `mode`, wrapped or not: a position that is not
/// acknowledged or, with a `landing` tolerance, shows the thumb farther than that from
/// where it was released; a gesture whose travel differs from its distance by more than a
/// pixel, or that never comes to rest. In a 720-pixel window the gestures move down 3,732
/// pixels, then up 2,796.
fn problems(
    source: String,
    size: ViewerGeometry,
    mode: u16,
    wrap: bool,
    landing: Option<f64>,
) -> Vec<String> {
    let mut engine = Engine::open(source, mode, size, 1);
    if wrap {
        engine = engine.wrapped();
    }
    let mut window = Window::default();
    rest(&mut engine, &mut window);
    let mut problems = Vec::new();
    for position in [0.5, 0.75, 0.25] {
        window.seek(&mut engine, position);
        if !settle(&mut engine, &mut window) {
            problems.push(format!("the scrollbar at {position} never came to rest"));
            return problems;
        }
        if window.acknowledged != window.position_sequence()
            || landing.is_some_and(|landing| (window.position - position).abs() > landing)
        {
            problems.push(format!(
                "the scrollbar at {position} shows {} (request {} of {})",
                window.position,
                window.acknowledged,
                window.position_sequence()
            ));
        }
    }
    let screen = f64::from(size.height);
    for (name, inputs, burst, repeat) in [
        ("PageDown", vec![(screen, true)], false, 3),
        ("5 notches", vec![(NOTCH, false); 5], false, 2),
        ("a burst of 5 notches", vec![(NOTCH, false); 5], true, 1),
        ("12 Down", vec![(ARROW, true); 12], false, 1),
        ("a burst of 12 Down", vec![(ARROW, true); 12], true, 1),
        ("PageUp", vec![(-screen, true)], false, 3),
        ("5 notches up", vec![(-NOTCH, false); 5], false, 1),
        ("a burst of 12 Up", vec![(-ARROW, true); 12], true, 1),
    ] {
        let expected: f64 = inputs.iter().map(|(pixels, _)| pixels).sum();
        for index in 0..repeat {
            let dropped = window.dropped.len();
            match try_gesture(&mut engine, &mut window, &inputs, burst) {
                Some(moved) if (moved - expected).abs() <= 1.0 => {}
                Some(moved) => problems.push(format!(
                    "{name} #{index} moved {moved:.1} of {expected}; dropped {:?}",
                    &window.dropped[dropped..]
                )),
                None => {
                    problems.push(format!(
                        "{name} #{index} never came to rest: {:?}",
                        window.log.last()
                    ));
                    return problems;
                }
            }
        }
    }
    problems
}

/// Every gesture over `source` moves its whole distance in `mode`, wrapped or not.
fn keeps_travel(
    name: &str,
    source: &str,
    size: ViewerGeometry,
    mode: u16,
    wrap: bool,
    landing: Option<f64>,
) {
    let report = problems(source.to_owned(), size, mode, wrap, landing);
    assert!(
        report.is_empty(),
        "{name}, mode {mode} wrap {wrap}: {report:#?}"
    );
}

/// The tests of a travel check in Text and Source, wrapped or not: four tests, so that the
/// views run in parallel; a module-name filter still selects all four.
macro_rules! travel_in_every_view {
    ($name:literal, $source:expr, $landing:expr) => {
        travel_in_every_view!($name, $source, SIZE, $landing);
    };
    ($name:literal, $source:expr, $size:expr, $landing:expr) => {
        use super::*;

        #[test]
        fn text() {
            keeps_travel($name, &$source, $size, viewer::MODE_TEXT, false, $landing);
        }

        #[test]
        fn text_wrapped() {
            keeps_travel($name, &$source, $size, viewer::MODE_TEXT, true, $landing);
        }

        #[test]
        fn source() {
            keeps_travel($name, &$source, $size, viewer::MODE_SOURCE, false, $landing);
        }

        #[test]
        fn source_wrapped() {
            keeps_travel($name, &$source, $size, viewer::MODE_SOURCE, true, $landing);
        }
    };
}

/// Text pages of the basic viewer's 8 KiB ranges reached 73,584 bytes, past what one layout
/// then placed right: the caret of each backward refill's last row fell a row low, and
/// every PageUp stopped three rows (49 px) short.
mod rows_of_1200_bytes_keep_every_gesture_s_travel {
    travel_in_every_view!("1200-byte rows", rows(1_500_000 / 1200, 1200), Some(0.002));
}

/// 27 rows filled a page of 64 KiB, fewer than the window shows: the page left the
/// retained camera, whose relative scrolling clamped each step to the next page's top.
/// PageDown moved 27 rows (443 px) in Source and 3 in Text, and an arrow key one row.
/// A window of 30 rows still shows more than such a page.
mod rows_of_2400_bytes_keep_every_gesture_s_travel {
    travel_in_every_view!(
        "2400-byte rows",
        rows(1_500_000 / 2400, 2400),
        ROWS_30,
        Some(0.002)
    );
}

/// A page of 120 KiB holds 51 rows of 2400 bytes, more than the window's 44. With pages of
/// 64 KiB (27 rows), 245 to 489 of the window's 720 pixels stayed blank below the page's
/// rows at rest. The camera may still pass a page's rows until the next refill
/// (`viewport::camera_span`): at rest, by less than a tenth of the window. One test per
/// mode, so that they run in parallel.
mod rows_of_2400_bytes_fill_the_window_at_rest {
    use super::*;

    #[test]
    fn text() {
        fills_the_window_at_rest(viewer::MODE_TEXT, rows(1_500_000 / 2400, 2400));
    }

    #[test]
    fn source() {
        fills_the_window_at_rest(viewer::MODE_SOURCE, rows(1_500_000 / 2400, 2400));
    }
}

/// One request reads fifteen rows of 8000 bytes (`viewport::PAGE_BYTES`), a third of the
/// window's 44: after an opening or a scrollbar position they covered 261 of its 720 pixels,
/// and 459 stayed blank until a gesture grew the page (row groups take a request's bytes per
/// refill, `worker_chain::Budget`). Such a page grows at rest too, until its rows fill the
/// window (`Worker::rows_end_in_window`).
mod rows_of_8000_bytes_fill_the_window_at_rest {
    use super::*;

    #[test]
    fn text() {
        fills_the_window_at_rest(viewer::MODE_TEXT, rows(8_000_000 / 8000, 8000));
    }

    #[test]
    fn source() {
        fills_the_window_at_rest(viewer::MODE_SOURCE, rows(8_000_000 / 8000, 8000));
    }
}

fn fills_the_window_at_rest(mode: u16, source: String) {
    let screen = f64::from(SIZE.height);
    let mut engine = Engine::open(source, mode, SIZE, 1);
    let mut window = Window::default();
    rest(&mut engine, &mut window);
    let mut blank = vec![blank_below_rows(&engine)];
    window.seek(&mut engine, 0.5);
    assert!(
        settle(&mut engine, &mut window) && window.acknowledged == window.position_sequence(),
        "mode {mode}: the position rests"
    );
    blank.push(blank_below_rows(&engine));
    for (inputs, repeat) in [
        (vec![(screen, true)], 4),
        (vec![(NOTCH, false); 5], 4),
        (vec![(ARROW, true); 20], 1),
        (vec![(-screen, true)], 4),
    ] {
        for _ in 0..repeat {
            let moved = try_gesture(&mut engine, &mut window, &inputs, false);
            assert!(moved.is_some(), "mode {mode}: a gesture rests");
            blank.push(blank_below_rows(&engine));
        }
    }
    assert!(
        blank.iter().all(|pixels| *pixels < screen / 10.0),
        "mode {mode}: blank pixels below the rows at rest {blank:?}"
    );
}

/// Pixels of the window below the shown page's rows while the file goes on.
fn blank_below_rows(engine: &Engine) -> f64 {
    let worker = &engine.worker;
    let (Some(page), Some(shaped)) = (&worker.page, &worker.shaped) else {
        return f64::INFINITY;
    };
    if !page.after {
        return 0.0;
    }
    let window = f64::from(worker.geometry.height) * f64::from(worker.geometry.scale);
    (f64::from(worker.origin[1]) + window - f64::from(shaped.height())).max(0.0)
}

/// Eight rows filled a page of 64 KiB, and PageDown moved at most those eight (131 px), in
/// Text one row or none; a page of 120 KiB holds fifteen. A Source position lands on a
/// whole row (`source_minimum_origin`): two rows are 0.4 % of this file. A window of 22
/// rows still shows more rows than either page holds.
mod rows_of_8000_bytes_keep_every_gesture_s_travel {
    travel_in_every_view!(
        "8000-byte rows",
        rows(4_000_000 / 8000, 8000),
        ROWS_22,
        Some(0.004)
    );
}

/// One PageDown grows a page of 8000-byte rows past the 720-pixel window (row groups). In
/// Source every scrollbar position after it then failed with "The requested viewport was not
/// completed.": its read was estimated from the grown page's extent (44 rows) and started a
/// whole read's bytes ahead of the named row, while a page of one read, shorter than the
/// window, landed only where it showed the thumb by its own bytes (`byte_position`). Two rows
/// are 0.2 % of this file. The position's page then grows at rest until it fills the window,
/// and the thumb stays where the position left it: a growth that moved it by 0.007 to 0.015
/// was not adopted.
mod rows_of_8000_bytes_take_scrollbar_positions_after_page_down {
    use super::*;

    #[test]
    fn text() {
        positions_after_page_down(viewer::MODE_TEXT, 0.002);
    }

    #[test]
    fn source() {
        positions_after_page_down(viewer::MODE_SOURCE, 0.002);
    }
}

/// Every scrollbar position after one PageDown over 8000-byte rows is acknowledged with the
/// thumb within `landing` of where it was released, and the thumb stays there at rest, within
/// a fifth of a row (rounding to a whole pixel moves the camera half a pixel at most).
fn positions_after_page_down(mode: u16, landing: f64) {
    const STAYS: f64 = 0.0002;
    let screen = f64::from(SIZE.height);
    let mut engine = Engine::open(rows(8_000_000 / 8000, 8000), mode, SIZE, 1);
    let mut window = Window::default();
    rest(&mut engine, &mut window);
    let moved = try_gesture(&mut engine, &mut window, &[(screen, true)], false);
    assert!(
        moved.is_some_and(|moved| (moved - screen).abs() <= 1.0),
        "mode {mode}: PageDown moved {moved:?}"
    );
    for position in [0.5, 0.75, 0.25] {
        window.seek(&mut engine, position);
        let mut acknowledged = None;
        for _ in 0..2000 {
            frame(&mut engine, &mut window);
            if window.acknowledged == window.position_sequence() {
                acknowledged = Some(window.position);
                break;
            }
        }
        let rested = settle(&mut engine, &mut window);
        assert!(
            rested
                && window.acknowledged == window.position_sequence()
                && acknowledged.is_some_and(|shown| {
                    (shown - position).abs() <= landing && (window.position - shown).abs() < STAYS
                }),
            "mode {mode}: the scrollbar at {position} showed {acknowledged:?} when acknowledged \
             and {} at rest (request {} of {}, at rest: {rested}), status {:?}",
            window.position,
            window.acknowledged,
            window.position_sequence(),
            engine.worker.status
        );
    }
}

/// Short rows whose pages end by their line budget meet rows that no page holds a window
/// of, both ways: gestures dropped their travel with a failed handoff or never came to rest.
/// A scrollbar position here lands within the one correction read its estimate of the
/// rows' density allows (`refine_position`), up to 1.6 % from the released thumb, as it
/// did before; only its acknowledgement is checked.
mod rows_mixing_short_and_very_long_ones_keep_every_gesture_s_travel {
    travel_in_every_view!("mixed rows", mixed_rows(), None);
}

/// Rows of 60 to 8000 bytes in blocks, 1.5 MB.
fn mixed_rows() -> String {
    mixed(
        &[
            (120, 60),
            (30, 2400),
            (90, 120),
            (12, 8000),
            (60, 830),
            (24, 1200),
            (80, 250),
        ],
        1_500_000,
    )
}
