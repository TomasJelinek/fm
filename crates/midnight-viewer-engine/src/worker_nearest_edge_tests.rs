//! Scrollbar positions that no bounded read can show exactly. Byte-capped Text and Source
//! windows of long rows are barely taller than the viewport, so each window's cameras
//! cover only a sliver of the scrollbar (`nearest_edge` in `worker_position.rs`).
use super::super::position::LoadedPosition;
use super::position::{recovery_integration_admit, rendered};
use super::text_position::{edge_page, edge_worker, settled_frame};
use super::*;

/// Rows like those of `third-party-inventory.csv`: 100 to 260 bytes, 180 on average.
fn csv_source() -> String {
    (0..640)
        .map(|row| {
            let prefix = format!("component-{row:03},MIT,");
            let width = 100 + row * 37 % 161;
            format!("{prefix}{}\n", "x".repeat(width - prefix.len() - 1))
        })
        .collect()
}

/// About 8 KiB of these rows is one byte-capped Text window of the basic viewer: barely
/// taller than a 700-pixel viewport, so its cameras cover only a sliver of the scrollbar.
/// A drag to a position just past that sliver, on either side, missed it; the correction
/// asked for twice the lines, which the byte cap answered with the same window, and the
/// drag failed with "The requested viewport was not completed.". It now settles at the
/// window's nearer edge and acknowledges the view it shows, without a third read.
#[test]
fn text_byte_capped_window_settles_a_missed_drag_at_its_nearer_edge() {
    let source = csv_source();
    let start = source.len() * 2 / 5;
    let end = start + 8180;
    let window = edge_page(&source, start, end);
    for after in [true, false] {
        let (mut worker, output) = edge_worker(window.clone(), source.len() as u64);
        worker.geometry = ViewerGeometry {
            width: 1000,
            height: 700,
            scale: 1.0,
        };
        let height = {
            let page = worker.page.as_ref().unwrap();
            let shaped = worker.shaped.as_ref().unwrap();
            crate::viewport::available_height(page, shaped, 700.0) - 0.25
        };
        worker.geometry.height = height.floor() as u32;
        assert!(worker.set_page(Page::text(window.clone(), &worker.appearance), &output));
        assert!(worker.scroll_map.is_some());
        let maximum = {
            let page = worker.page.as_ref().unwrap();
            let shaped = worker.shaped.as_ref().unwrap();
            let height = worker.geometry.height as f32;
            crate::viewport::available_height(page, shaped, height) - height
        };
        assert!(maximum > 0.0 && maximum < 2.0, "{maximum} px of travel");
        worker.origin[1] = maximum;
        let (high, _) = worker.position_metrics().unwrap();
        worker.origin[1] = 0.0;
        let (low, _) = worker.position_metrics().unwrap();
        assert!(high - low < 0.0005, "a sliver: {low}..{high}");
        let first = rendered(&mut worker, &output);
        let target = if after { high + 0.0002 } else { low - 0.0002 };
        assert!(worker.positions.accept(first.generation, 4, false, target));
        worker.positions.target = Some(crate::position_ack::Target {
            sequence: 4,
            position: target,
            corrected: false,
        });
        worker.positions.requested(91, 4);
        worker.positions.loaded(91);
        worker.viewport_request = Some(91);
        worker.position_page = Some((91, worker.page.as_ref().unwrap().clone()));
        let LoadedPosition::ReloadViewport { sequence, viewport } = worker.refine_position() else {
            panic!("the first miss grows the window once");
        };
        assert_eq!(sequence, 4);
        assert_eq!(
            (viewport.source_anchor, viewport.backwards),
            if after {
                (start as u64, false)
            } else {
                (end as u64, true)
            }
        );
        // The byte cap answers the larger line budget with the same window.
        worker.positions.requested(92, 4);
        worker.viewport_request = Some(92);
        unsent::take();
        worker.event(92, EventKind::TextPage(window.clone()), &output);
        worker.event(92, EventKind::Completed, &output);
        let (settled, unavailable) = settled_frame(&mut worker, &output);
        assert!(
            !unavailable && worker.status.is_empty(),
            "after={after}: {}",
            worker.status
        );
        assert_eq!(settled.position_sequence, [4, 0], "after={after}");
        assert_eq!(settled.generation, first.generation);
        assert_eq!(worker.origin[1], if after { maximum } else { 0.0 });
        assert_eq!(settled.position, if after { high } else { low });
        assert!(unsent::take().is_empty(), "no third read");
        assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
    }
}

/// Byte-capped Source windows of the long rows of `third-party-inventory.csv`: about 8 KiB
/// of them is barely taller than the viewport, so a window's cameras cover only a sliver
/// of the scrollbar and no reread the cap allows brings another camera. A window that
/// starts inside a row, as the source viewer's reads do, has no camera below its first
/// complete line at all, and one shorter than the viewport has a single camera. Each of
/// these drags was refused ("The requested viewport was not completed."); the position is
/// now resolved in the candidate's camera range, or settles at its nearer edge, and
/// acknowledged.
#[test]
fn scrollbar_source_byte_capped_rereads_complete_a_drag_in_the_candidate() {
    use crate::position_ack::{MissingEdge, SourceRecovery, SourceWindow};

    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Case {
        Before,
        Inside,
        After,
    }
    let source = csv_source();
    let rows = source[..source.len() * 2 / 5].rfind('\n').unwrap() + 1;
    let rows_end = rows
        + source[rows..]
            .split_inclusive('\n')
            .map(str::len)
            .scan(0, |length, row| {
                *length += row;
                (*length <= 8180).then_some(*length)
            })
            .last()
            .unwrap();
    let inside = rows + 57;
    assert!(!source[rows..inside].contains('\n'));
    for (start, end, short, case) in [
        (rows, rows_end, false, Case::After),
        (rows, rows_end, false, Case::Before),
        (inside, inside + 8192, false, Case::Inside),
        (inside, inside + 8192, false, Case::After),
        (inside, inside + 8192, true, Case::Before),
        (inside, inside + 8192, true, Case::After),
    ] {
        let window = edge_page(&source, start, end);
        let (mut worker, output) = worker();
        worker.mode = viewer::MODE_SOURCE;
        worker.generation = 1;
        worker.wrap = false;
        worker.raster.enabled = false;
        worker.positions.reset().unwrap();
        worker.length = source.len() as u64;
        worker.geometry = ViewerGeometry {
            width: 1000,
            height: 700,
            scale: 1.0,
        };
        // Size the viewport to the candidate, or just above it, and read its sliver of
        // the scrollbar.
        assert!(worker.set_page(Page::text(window.clone(), &worker.appearance), &output));
        let height = {
            let page = worker.page.as_ref().unwrap();
            let shaped = worker.shaped.as_ref().unwrap();
            if short {
                shaped.height() + 10.0
            } else {
                crate::viewport::available_height(page, shaped, 700.0) - 0.25
            }
        };
        worker.geometry.height = height.floor() as u32;
        assert!(worker.set_page(Page::text(window.clone(), &worker.appearance), &output));
        let (maximum, minimum) = {
            let page = worker.page.as_ref().unwrap();
            let shaped = worker.shaped.as_ref().unwrap();
            let height = worker.geometry.height as f32;
            (
                crate::viewport::available_height(page, shaped, height) - height,
                crate::viewport::source_minimum_origin(page, shaped),
            )
        };
        let maximum = if short {
            assert!(worker.scroll_map.is_none() && maximum < 0.0);
            0.0
        } else {
            assert!(maximum > 0.0 && maximum < 2.0, "{maximum} px of travel");
            maximum
        };
        let whole = !window.partial_line;
        assert!(
            if whole {
                minimum == Some(0.0)
            } else {
                minimum.is_some_and(|minimum| minimum > maximum.max(1.0))
            },
            "{case:?}: first complete line at {minimum:?}, cameras up to {maximum}"
        );
        let mut at = |y: f32| {
            worker.origin[1] = y;
            worker.position_metrics().unwrap().0
        };
        let (low, high, middle) = (at(0.0), at(maximum), at(maximum / 2.0));
        assert!(high - low < 0.0005, "a sliver: {low}..{high}");
        let source_length = worker.length as f64;
        let target = match case {
            Case::Before => low - 0.0002,
            Case::Inside => middle,
            Case::After => high + 0.0002,
        };
        // The drag starts from the file's first rows.
        let first_rows = edge_page(&source, 0, source[..8000].rfind('\n').unwrap() + 1);
        assert!(worker.set_page(Page::text(first_rows, &worker.appearance), &output));
        let previous = rendered(&mut worker, &output);
        // The completed first reply missed and was reread once: for more lines toward
        // the position, or for the complete first line of a window that starts inside one.
        let mut recovery = SourceRecovery::new(messages::Viewport {
            source_anchor: start as u64,
            backwards: false,
            max_bytes: 12_288,
            max_lines: 48,
        });
        let reread = recovery
            .reread(
                SourceWindow {
                    start: start as u64,
                    end: end as u64,
                    before: true,
                    after: true,
                },
                // As `resolve_source_position` chooses for this candidate.
                if short {
                    if target * source_length < start as f64 {
                        MissingEdge::Before
                    } else {
                        MissingEdge::After
                    }
                } else if whole && case == Case::After {
                    MissingEdge::After
                } else {
                    MissingEdge::Before
                },
            )
            .expect("the first miss rereads once");
        recovery_integration_admit(
            &mut worker,
            92,
            4,
            target,
            reread.source_anchor,
            reread.max_lines,
        );
        worker.positions.source = Some(recovery);
        // The byte cap answers the larger line budget with the same window.
        worker.event(92, EventKind::TextPage(window.clone()), &output);
        worker.event(92, EventKind::Completed, &output);
        let (settled, unavailable) = settled_frame(&mut worker, &output);
        assert!(!unavailable, "{case:?}: {}", worker.status);
        assert!(
            !worker.status.contains("not completed"),
            "{}",
            worker.status
        );
        assert_eq!(settled.position_sequence, [4, 0], "{case:?}");
        assert_eq!(settled.generation, previous.generation);
        assert_eq!(
            worker.page.as_ref().map(|page| (page.start, page.end)),
            Some((start as u64, end as u64))
        );
        match case {
            Case::Before => {
                assert_eq!(worker.origin[1], 0.0);
                assert_eq!(settled.position, low);
            }
            Case::Inside => {
                assert!(!short && worker.origin[1] > 0.0 && worker.origin[1] < maximum);
                assert!(
                    (settled.position - target).abs() < 1e-9,
                    "{}",
                    settled.position
                );
            }
            Case::After => {
                assert_eq!(worker.origin[1], maximum);
                assert_eq!(settled.position, high);
            }
        }
        assert!(settled.pixels != previous.pixels);
        assert!(worker.positions.source.is_none());
        assert!(!worker.positions.has_pending() && worker.viewport_request.is_none());
    }
}
