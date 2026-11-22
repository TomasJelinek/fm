//! Unwrapped Text and Source pages are laid out as row groups
//! (`TextRenderer::layout_rows`): a refill shapes only the rows it adds, a page of long rows
//! grows to hold the prepared screens, and the page answers, copies and finds as one layout
//! of its rows would. Wrapped pages keep one layout.
use super::frontier::{Engine, Window, frame, gesture, rest, rows};
use super::*;

/// The window of the long-row runs.
const SIZE: ViewerGeometry = ViewerGeometry {
    width: 1312,
    height: 720,
    scale: 1.0,
};
/// A group's most bytes (`midnight_viewer_render`'s row groups).
const GROUP: u64 = 8 * 1024;

/// A view of 2400-byte rows at rest at 30 % of the file.
fn long_rows(mode: u16, wrap: bool) -> (Engine, Window) {
    let mut engine = Engine::open(rows(4000, 2400), mode, SIZE, 1);
    if wrap {
        engine = engine.wrapped();
    }
    let mut window = Window::default();
    rest(&mut engine, &mut window);
    window.seek(&mut engine, 0.3);
    rest(&mut engine, &mut window);
    (engine, window)
}

/// The shown page's rows laid out afresh, as row groups that reuse nothing, with the
/// worker's typography: it answers as one layout of its rows (`rows_tests.rs` in the
/// renderer), which a page past one layout's bytes cannot be.
fn fresh_layout(worker: &Worker) -> TextPage {
    let page = worker.page.as_ref().expect("a page");
    let family = worker.text_family().to_owned();
    let options = worker.text_options(&family);
    TextRenderer::default()
        .layout_rows(
            &page.text,
            &options,
            &page.spans,
            &page.scaled_styles(&options),
            None,
        )
        .expect("a fresh layout")
        .0
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

/// Three PageDowns over 2400-byte rows: the refills shape the rows they read and at most the
/// group that a new first row cuts, never the rows they share with the shown page.
#[test]
fn a_refill_shapes_only_the_rows_it_adds() {
    for mode in [viewer::MODE_TEXT, viewer::MODE_SOURCE] {
        let (mut engine, mut window) = long_rows(mode, false);
        let read = engine.bytes;
        prepare_counters::take();
        let screen = f64::from(SIZE.height);
        for _ in 0..3 {
            let moved = gesture(&mut engine, &mut window, &[(screen, true)], false);
            assert!((moved - screen).abs() < 1.0, "mode {mode}: moved {moved}");
        }
        let counts = prepare_counters::take();
        let read = (engine.bytes - read) as u64;
        assert!(counts.layouts > 0, "mode {mode}: no refill");
        assert!(
            counts.layout_bytes <= read + counts.layouts * GROUP,
            "mode {mode}: {} bytes shaped in {} layouts for {read} bytes read",
            counts.layout_bytes,
            counts.layouts
        );
    }
}

/// The screens a page of `engine`'s rows holds above and below the window.
fn screens_around(engine: &Engine) -> (f64, f64) {
    let worker = &engine.worker;
    let shaped = worker.shaped.as_ref().expect("a layout");
    let screen = f64::from(SIZE.height);
    let above = f64::from(worker.origin[1]) / screen;
    (above, f64::from(shaped.height()) / screen - above - 1.0)
}

/// After a PageDown and a rest, a page of 2400-byte rows holds the prepared screens ahead of
/// the window, two, and the two a page holds beyond them (`worker_bands.rs`), less a row of
/// the line budget, and more than the quarter screen behind it that starts a refill against
/// the travel: far more than one layout of 120 KiB (51 rows, 1.2 screens) could.
#[test]
fn a_page_of_long_rows_grows_to_hold_the_prepared_screens() {
    let (mut engine, mut window) = long_rows(viewer::MODE_TEXT, false);
    let screen = f64::from(SIZE.height);
    gesture(&mut engine, &mut window, &[(screen, true)], false);
    rest(&mut engine, &mut window);
    let (above, below) = screens_around(&engine);
    assert!(
        above >= 0.25 && below >= 3.9,
        "the page holds {above:.2} screens above the window and {below:.2} below"
    );
    let shaped = engine.worker.shaped.as_ref().expect("a layout");
    assert!(
        shaped.row_groups().is_some_and(|groups| groups.len() > 1),
        "not laid out in row groups"
    );
}

/// A refilled page of row groups, which reuses the groups it shares with the page before
/// it, gives the rows, carets, hit tests, selections (a Find match among them) and copies
/// of its rows laid out afresh, at every group edge.
#[test]
fn a_refilled_page_answers_copies_and_finds_as_a_fresh_layout() {
    let (mut engine, mut window) = long_rows(viewer::MODE_TEXT, false);
    let screen = f64::from(SIZE.height);
    for _ in 0..2 {
        gesture(&mut engine, &mut window, &[(screen, true)], false);
    }
    let one = fresh_layout(&engine.worker);
    let shaped = engine.worker.shaped.as_ref().expect("a layout");
    let groups = shaped.row_groups().expect("row groups");
    assert!(groups.len() > 2, "{} groups", groups.len());
    let (anchors, grouped) = (one.scroll_anchors(), shaped.scroll_anchors());
    assert_eq!(anchors.len(), grouped.len(), "scroll anchors");
    for (a, b) in anchors.iter().zip(&grouped) {
        assert!(
            a.0 == b.0 && close(f64::from(a.1), f64::from(b.1)),
            "anchor {a:?} {b:?}"
        );
    }
    let text = engine.worker.page.as_ref().unwrap().text.clone();
    for group in &groups[1..] {
        let edge = group.start;
        // The last column of the group before, its line break, and the next row's start.
        for at in [edge - 2, edge - 1, edge, edge + 1] {
            let (a, b) = (
                one.caret_geometry(at).unwrap(),
                shaped.caret_geometry(at).unwrap(),
            );
            assert!(
                close(a.x0, b.x0) && close(a.y0, b.y0) && close(a.y1, b.y1),
                "caret {at}: {a:?} {b:?}"
            );
            let (x, y) = (a.x0 as f32 + 0.5, a.y0 as f32 + 0.5);
            assert_eq!(one.hit_test(x, y), shaped.hit_test(x, y), "hit at {x}, {y}");
        }
        // A match at the end of the group's last row, one at the next row's start, and a
        // selection across the edge: highlighted and copied as one layout.
        for range in [edge - 5..edge - 1, edge..edge + 4, edge - 5..edge + 4] {
            let (a, b) = (
                one.selection_geometry(range.clone()).unwrap(),
                shaped.selection_geometry(range.clone()).unwrap(),
            );
            assert!(
                a.len() == b.len()
                    && a.iter().zip(&b).all(|(a, b)| close(a.x0, b.x0)
                        && close(a.x1, b.x1)
                        && close(a.y0, b.y0)
                        && close(a.y1, b.y1)),
                "selection {range:?}: {a:?} {b:?}"
            );
            engine.worker.selection = Some((range.start, range.end));
            engine.worker.copy_page_selection(&engine.output);
            let copied = engine
                .output
                .take()
                .into_iter()
                .find_map(|output| match output {
                    ViewerOutput::CopyText(copied) => Some(copied),
                    _ => None,
                });
            assert_eq!(
                copied.as_deref(),
                Some(&text[range.clone()]),
                "copy {range:?}"
            );
        }
    }
}

/// While a refill of a page of long rows reads its ranges, one plugin reply a frame, the
/// camera moves on over the rows the page holds: the worker takes each camera request and
/// publishes its frame, instead of holding the camera where the refill started.
#[test]
fn the_camera_moves_on_while_a_refill_reads() {
    // At rest after a seek the page holds 51 rows and the window lies near its end: the
    // first step starts a refill of 16 ranges, and the steps during it stay on the page.
    let (mut engine, mut window) = long_rows(viewer::MODE_TEXT, false);
    let step = 2.0;
    let mut frames = 0;
    while engine.worker.raster.continuing.is_none() {
        window.push(&mut engine, step, true);
        frame(&mut engine, &mut window);
        frames += 1;
        assert!(frames < 400, "no refill started");
    }
    let mut followed = 0;
    while engine.worker.raster.continuing.is_some() {
        let start = window.shown();
        window.push(&mut engine, step, true);
        frame(&mut engine, &mut window);
        if engine.worker.raster.continuing.is_none() {
            break;
        }
        let worker = &engine.worker;
        assert!(
            worker.raster.follows(worker.origin),
            "the camera waits during the refill"
        );
        assert!(window.shown() >= start, "moved back");
        followed += 1;
        frames += 1;
        assert!(frames < 800, "the refill never completed");
    }
    assert!(followed >= 4, "{followed} frames of the refill");
    rest(&mut engine, &mut window);
}

/// Wrapped rows keep one layout: their pages and suites are those of before.
#[test]
fn wrapped_rows_keep_one_layout() {
    let (engine, _) = long_rows(viewer::MODE_TEXT, true);
    let shaped = engine.worker.shaped.as_ref().expect("a layout");
    assert!(shaped.row_groups().is_none(), "wrapped rows in row groups");
}
