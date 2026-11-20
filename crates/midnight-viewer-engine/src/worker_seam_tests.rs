//! Scrolling across a table taller than the viewport and the request window. A provider
//! keeps a table whole, so it cannot always fit one page together with the content after
//! (or before) it, and it answers a continuation anchored inside the table with the same
//! window again. The Markdown viewer stopped this way inside
//! `docs/IMPLEMENTATION_BACKLOG.md`: every wheel step asked for that window once more.
use super::*;
use midnight_plugin_host::api::document;

const GEOMETRY: ViewerGeometry = ViewerGeometry {
    width: 480,
    height: 200,
    scale: 1.0,
};

/// One block at `offset`: several cells form a table row, one cell without a row a
/// paragraph. Anchors cover every character, as a provider's exact source map does.
fn block(offset: u64, cells: &[String], row: Option<bool>) -> document::Block {
    let text = cells.join("\t");
    let mut cursor = 0u32;
    let table = row.map(|header| document::TableRow {
        header,
        cells: cells
            .iter()
            .map(|cell| {
                let start = cursor;
                cursor += cell.len() as u32 + 1;
                document::TableCell {
                    start,
                    end: start + cell.len() as u32,
                    alignment: document::TABLE_ALIGN_LEFT,
                }
            })
            .collect(),
    });
    let anchors = text
        .char_indices()
        .map(|(at, _)| at)
        .chain(std::iter::once(text.len()))
        .map(|at| viewer::Anchor {
            text_offset: at as u32,
            source_offset: offset + at as u64,
        })
        .collect();
    document::Block {
        kind: document::BLOCK_PARAGRAPH,
        depth: 0,
        heading_level: 0,
        links: vec![],
        text: viewer::TextPage {
            revision: 1,
            source: viewer::SourceRange {
                offset,
                length: text.len() as u64,
            },
            text,
            spans: vec![],
            anchors,
            has_before: false,
            has_after: false,
            partial_line: false,
        },
        table,
        strikethrough: None,
    }
}

/// Blocks laid out from `start`, one separator byte apart, as one provider page.
fn page(worker: &Worker, start: u64, blocks: &[(Vec<String>, Option<bool>)]) -> Page {
    let mut offset = start;
    let blocks: Vec<_> = blocks
        .iter()
        .map(|(cells, row)| {
            let block = block(offset, cells, *row);
            offset += block.text.source.length + 1;
            block
        })
        .collect();
    Page::document(
        document::DocumentPage {
            revision: 1,
            source: viewer::SourceRange {
                offset: start,
                length: offset - start,
            },
            blocks,
            has_before: true,
            has_after: true,
        },
        &worker.appearance,
    )
}

fn paragraph(text: &str) -> (Vec<String>, Option<bool>) {
    (vec![text.to_owned()], None)
}

/// A paragraph, then a table whose body rows wrap into many lines each, then the
/// paragraph that the provider could still fit after it.
fn table_page(worker: &Worker) -> Page {
    let cell =
        |row: usize| format!("Row {row} evidence that wraps in its narrow column. ").repeat(9);
    let mut blocks = vec![
        paragraph("The table below is one layout unit."),
        (
            vec!["ID".into(), "State".into(), "Evidence".into()],
            Some(true),
        ),
    ];
    blocks.extend((1..=4).map(|row| {
        (
            vec![format!("ROW-{row}"), cell(row), cell(row)],
            Some(false),
        )
    }));
    blocks.push(paragraph("All rows apply to every platform."));
    page(worker, 19_367, &blocks)
}

/// The content after the table page: another provider page starting at its end.
fn following_page(worker: &Worker, start: u64) -> Page {
    let blocks: Vec<_> = (1..=12)
        .map(|index| paragraph(&format!("Following paragraph {index} after the table.")))
        .collect();
    page(worker, start, &blocks)
}

fn markdown_worker() -> (Worker, Mailbox) {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_DOCUMENT;
    worker.modes = vec![viewer::MODE_DOCUMENT];
    worker.geometry = GEOMETRY;
    worker.apply_font();
    (worker, output)
}

/// What a session does when it admits a request (the test worker has no session).
fn admit(worker: &mut Worker, viewport: &messages::Viewport) {
    worker.land_at_bottom = viewport.backwards;
    worker.anchor = viewport.source_anchor;
}

/// The provider's complete reply to the pending request: its page, then completion.
fn reply(worker: &mut Worker, output: &Mailbox, page: Page) {
    let id = 7;
    worker.viewport_request = Some(id);
    worker.received_page(id, page, output);
    worker.event(id, EventKind::Completed, output);
}

fn height(worker: &Worker) -> f32 {
    worker.shaped.as_ref().unwrap().height()
}

fn paint(worker: &mut Worker, output: &Mailbox) -> ViewerFrame {
    worker.paint(output).unwrap();
    output
        .take()
        .into_iter()
        .find_map(|message| match message {
            ViewerOutput::Frame(frame) => Some(*frame),
            _ => None,
        })
        .expect("a frame")
}

#[test]
fn a_reply_that_cannot_pass_a_tall_table_continues_at_the_page_boundary() {
    let (mut worker, output) = markdown_worker();
    let table = table_page(&worker);
    assert!(worker.set_page(table.clone(), &output));
    let window = worker.page_viewport(table.start, false);
    let line = worker.appearance.font_size * 1.2;
    assert!(
        height(&worker) > 4.0 * GEOMETRY.height as f32
            && height(&worker) > window.max_lines as f32 * line,
        "the table must be taller than the viewport and the request window: {} px, {} lines",
        height(&worker),
        window.max_lines
    );
    unsent::take();
    // Wheel down until a step needs content after this page.
    let mut steps = 0;
    while worker.continuation.is_none() {
        worker.input(ViewerInput::Scroll { x: 0.0, y: 40.0 }, &output);
        steps += 1;
        assert!(steps < 1000, "the wheel never reached the page bottom");
    }
    let first = unsent::take();
    assert_eq!(first.len(), 1);
    assert!(!first[0].backwards);
    assert!(
        first[0].source_anchor > table.start && first[0].source_anchor < table.end,
        "the request keeps the visible part of the table"
    );
    // The provider cannot add the next unit to the whole table: the same window again.
    reply(&mut worker, &output, table.clone());
    let seam = unsent::take();
    assert_eq!(
        seam.len(),
        1,
        "the viewer must ask for what follows the page instead: {seam:?}"
    );
    assert_eq!(seam[0].source_anchor, table.end);
    assert!(!seam[0].backwards);
    // That request carries no continuation: if it cannot advance either, it ends there.
    admit(&mut worker, &seam[0]);
    reply(&mut worker, &output, table.clone());
    assert!(unsent::take().is_empty(), "a repeated reply must not loop");
    // The page from that boundary shows its first line at the viewport top: nothing
    // between the old bottom edge and the new top edge is skipped.
    let following = following_page(&worker, table.end);
    reply(&mut worker, &output, following);
    assert_eq!(worker.page.as_ref().unwrap().start, table.end);
    assert_eq!(worker.origin[1], 0.0);
    assert!(unsent::take().is_empty());
    // Scrolling continues from there.
    worker.input(ViewerInput::Scroll { x: 0.0, y: 40.0 }, &output);
    assert_eq!(worker.origin[1], 40.0);
}

#[test]
fn a_reply_that_cannot_reach_back_over_a_tall_table_continues_at_the_page_boundary() {
    let (mut worker, output) = markdown_worker();
    let table = table_page(&worker);
    let following = following_page(&worker, table.end);
    assert!(worker.set_page(following.clone(), &output));
    assert_eq!(worker.origin[1], 0.0);
    unsent::take();
    worker.input(ViewerInput::Scroll { x: 0.0, y: -40.0 }, &output);
    let first = unsent::take();
    assert_eq!(first.len(), 1);
    assert!(first[0].backwards);
    // The provider cannot add the whole table before this page: the same window again.
    reply(&mut worker, &output, following.clone());
    let seam = unsent::take();
    assert_eq!(seam.len(), 1, "{seam:?}");
    assert_eq!(seam[0].source_anchor, following.start);
    assert!(seam[0].backwards);
    // The page ending at that boundary shows its last line at the viewport bottom.
    admit(&mut worker, &seam[0]);
    reply(&mut worker, &output, table);
    let bottom = height(&worker) - GEOMETRY.height as f32;
    assert!(
        (worker.origin[1] - bottom).abs() < 0.5,
        "{} != {bottom}",
        worker.origin[1]
    );
    assert!(unsent::take().is_empty());
}

/// Wheel down until a step needs content after the page; a bounded number of steps, so a
/// wheel that never gets there fails instead of hanging.
fn wheel_to_page_bottom(worker: &mut Worker, output: &Mailbox) {
    let mut steps = 0;
    while worker.continuation.is_none() {
        worker.input(ViewerInput::Scroll { x: 0.0, y: 40.0 }, output);
        steps += 1;
        assert!(steps < 1000, "the wheel never reached the page bottom");
    }
}

#[test]
fn a_reply_that_extends_the_page_needs_no_further_request() {
    let (mut worker, output) = markdown_worker();
    let table = table_page(&worker);
    assert!(worker.set_page(table.clone(), &output));
    wheel_to_page_bottom(&mut worker, &output);
    unsent::take();
    // A provider that could fit more answers with a longer page from the table start.
    let mut longer = table.clone();
    longer.end += 50;
    reply(&mut worker, &output, longer);
    assert!(unsent::take().is_empty());
}

/// Wheel steps that come while the request past the page waits: further down, the viewer
/// still continues at the page boundary; back up, it stays on the page and scrolls back.
#[test]
fn motion_that_turned_back_meanwhile_stays_on_the_page_instead_of_the_seam() {
    for back in [false, true] {
        let (mut worker, output) = markdown_worker();
        let table = table_page(&worker);
        assert!(worker.set_page(table.clone(), &output));
        unsent::take();
        wheel_to_page_bottom(&mut worker, &output);
        assert_eq!(unsent::take().len(), 1);
        // A session admitted the request; the next wheel step waits for its reply.
        let id = 7;
        worker.viewport_request = Some(id);
        let y = if back { -40.0 } else { 40.0 };
        worker.input(ViewerInput::Scroll { x: 0.0, y }, &output);
        assert!(unsent::take().is_empty(), "the step waits for the reply");
        // The same window again: the reply cannot pass the table.
        worker.received_page(id, table.clone(), &output);
        worker.event(id, EventKind::Completed, &output);
        let requests = unsent::take();
        assert_eq!(worker.page.as_ref().unwrap().start, table.start);
        if back {
            assert!(requests.is_empty(), "no jump past the table: {requests:?}");
            // The step back was replayed on the table.
            let bottom = height(&worker) - GEOMETRY.height as f32;
            assert!(
                worker.origin[1] <= bottom - 40.0 + 0.5,
                "{} > {bottom} - 40",
                worker.origin[1]
            );
        } else {
            assert_eq!(requests.len(), 1, "{requests:?}");
            assert_eq!(requests[0].source_anchor, table.end);
            assert!(!requests[0].backwards);
        }
    }
}

/// The application's retained raster: wheel and keys pan the retained pixels and send
/// the new camera, which the worker refills around once it leaves the page.
#[test]
fn a_retained_pan_past_a_tall_table_continues_at_the_page_boundary() {
    let (mut worker, output) = markdown_worker();
    worker.input(ViewerInput::RetainedRaster(true), &output);
    let table = table_page(&worker);
    assert!(worker.set_page(table.clone(), &output));
    let info = paint(&mut worker, &output).raster.expect("retained pixels");
    unsent::take();
    // Pan down in wheel steps, as the window does, until the camera leaves the page.
    let mut camera = info.origin;
    let mut first = Vec::new();
    for sequence in 1..1000 {
        camera[1] += 40.0;
        worker.input(
            ViewerInput::Raster {
                surface: info.surface,
                input: midnight_core::ViewerRasterInput::PanTo {
                    sequence,
                    origin: camera,
                    settled: true,
                },
            },
            &output,
        );
        first = unsent::take();
        if !first.is_empty() && worker.continuation.is_some() {
            break;
        }
        paint(&mut worker, &output);
    }
    assert_eq!(
        first.len(),
        1,
        "a camera past the page needs a refill: {first:?}"
    );
    assert!(!first[0].backwards);
    reply(&mut worker, &output, table.clone());
    let seam = unsent::take();
    assert_eq!(seam.len(), 1, "{seam:?}");
    assert_eq!(seam[0].source_anchor, table.end);
    assert!(!seam[0].backwards);
    admit(&mut worker, &seam[0]);
    let following = following_page(&worker, table.end);
    reply(&mut worker, &output, following);
    assert_eq!(worker.page.as_ref().unwrap().start, table.end);
    assert_eq!(worker.origin[1], 0.0);
}

/// A scrollbar fraction just past the last camera of a window that ends at a whole table
/// cannot be shown by that window, and the next window starts below it: no window shows it
/// at that camera. After its one correction read, the position settles at the nearer edge
/// of the loaded window, where the document continues, instead of failing.
#[test]
fn a_scrollbar_position_no_window_can_show_settles_at_the_nearest_edge() {
    let (mut worker, output) = markdown_worker();
    let table = table_page(&worker);
    // The table page lies in the middle of a longer document.
    worker.length = table.end + 40_000;
    assert!(worker.set_page(table.clone(), &output));
    let maximum = height(&worker) - GEOMETRY.height as f32;
    worker.origin[1] = maximum;
    let (bottom, _) = worker.position_metrics().unwrap();
    worker.origin[1] = 0.0;
    let (top, _) = worker.position_metrics().unwrap();
    assert!(top > 0.01 && bottom < 0.99, "{top}..{bottom}");
    for (position, edge) in [(bottom + 0.004, maximum), (top - 0.004, 0.0)] {
        worker.origin[1] = maximum / 2.0;
        worker.positions.target = Some(crate::position_ack::Target {
            sequence: 1,
            position,
            corrected: true,
        });
        assert_eq!(
            worker.refine_position(),
            super::super::position::LoadedPosition::Ready,
            "{position} past {top}..{bottom}"
        );
        assert_eq!(worker.origin[1], edge);
    }
}

/// The window beside a whole table can be much smaller than the band no window shows:
/// the table's window ends with its last rows, and the small window after it starts below
/// the band. A position in that band settles at the small window's nearer edge even when
/// it lies farther away than the window's own source span, which plain text never allows.
#[test]
fn a_small_window_beside_a_table_settles_a_position_farther_away_than_its_span() {
    let (mut worker, output) = markdown_worker();
    let table = table_page(&worker);
    let following = following_page(&worker, table.end);
    worker.length = following.end + 40_000;
    assert!(worker.set_page(following.clone(), &output));
    let span = (following.end - following.start) as f64 / worker.length as f64;
    let maximum = height(&worker) - GEOMETRY.height as f32;
    assert!(maximum > 0.0);
    worker.origin[1] = maximum;
    let (bottom, _) = worker.position_metrics().unwrap();
    worker.origin[1] = 0.0;
    let (top, _) = worker.position_metrics().unwrap();
    assert!(
        top > 3.0 * span && bottom + 3.0 * span < 1.0,
        "{top}..{bottom}"
    );
    for (position, edge) in [(top - 2.0 * span, 0.0), (bottom + 2.0 * span, maximum)] {
        worker.origin[1] = maximum / 2.0;
        worker.positions.target = Some(crate::position_ack::Target {
            sequence: 1,
            position,
            corrected: true,
        });
        assert_eq!(
            worker.refine_position(),
            super::super::position::LoadedPosition::Ready,
            "{position} beside {top}..{bottom}, span {span}"
        );
        assert_eq!(worker.origin[1], edge);
    }
}
