//! Retain a bounded selection when a smaller styled fragment covers its view.
use super::*;

fn source_text(text: String) -> viewer::TextPage {
    viewer::TextPage {
        revision: 9,
        source: viewer::SourceRange {
            offset: 8192,
            length: text.len() as u64,
        },
        anchors: text
            .char_indices()
            .map(|(at, _)| viewer::Anchor {
                text_offset: at as u32,
                source_offset: 8192 + at as u64,
            })
            .chain(std::iter::once(viewer::Anchor {
                text_offset: text.len() as u32,
                source_offset: 8192 + text.len() as u64,
            }))
            .collect(),
        text,
        spans: Vec::new(),
        has_before: true,
        has_after: true,
        partial_line: false,
    }
}

fn fragment(original: &viewer::TextPage, range: std::ops::Range<usize>) -> viewer::TextPage {
    let mut page = original.clone();
    page.text = original.text[range.clone()].to_owned();
    page.source.offset += range.start as u64;
    page.source.length = range.len() as u64;
    page.anchors = original
        .anchors
        .iter()
        .filter(|anchor| {
            range.start <= anchor.text_offset as usize && anchor.text_offset as usize <= range.end
        })
        .map(|anchor| viewer::Anchor {
            text_offset: anchor.text_offset - range.start as u32,
            source_offset: anchor.source_offset,
        })
        .collect();
    page.spans = vec![viewer::Span {
        start: 0,
        end: page.text.len() as u32,
        role: viewer::STYLE_STRING,
        flags: 0,
    }];
    page.validate().unwrap();
    page
}

fn visual_source(worker: &Worker) -> (u64, [f32; 2]) {
    let page = worker.page.as_ref().unwrap();
    let shaped = worker.shaped.as_ref().unwrap();
    let index = shaped
        .hit_test(worker.origin[0], worker.origin[1] + 0.5)
        .unwrap();
    let source = page
        .anchors
        .iter()
        .find(|anchor| anchor.text_offset as usize == index)
        .unwrap()
        .source_offset;
    let caret = shaped.caret_geometry(index).unwrap();
    (
        source,
        [
            worker.origin[0] - caret.x0 as f32,
            worker.origin[1] - caret.y0 as f32,
        ],
    )
}

fn painted(worker: &mut Worker, output: &Mailbox) -> Box<ViewerFrame> {
    worker.paint(output).unwrap();
    output
        .take()
        .into_iter()
        .find_map(|event| match event {
            ViewerOutput::Frame(frame) => Some(frame),
            _ => None,
        })
        .expect("a usable bounded page must still paint")
}

fn copied(worker: &mut Worker, output: &Mailbox, frame: u64) -> String {
    worker.input(ViewerInput::Copy { frame }, output);
    output
        .take()
        .into_iter()
        .find_map(|event| match event {
            ViewerOutput::CopyText(text) => Some(text),
            _ => None,
        })
        .expect("the actual Copy input must publish the retained selection")
}

fn long_selection_worker(retained: bool) -> (Worker, Mailbox, viewer::TextPage, usize) {
    let (mut worker, output) = source_worker();
    let row = "int vybrané_value = \"東京 🦀\"; // generated\n";
    let source = source_text(row.repeat(24));
    assert!(worker.set_page(Page::text(source.clone(), &worker.appearance), &output));
    let y = worker
        .shaped
        .as_ref()
        .unwrap()
        .caret_geometry(row.len() * 10)
        .unwrap()
        .y0 as f32;
    worker.origin = [0.0, y + 3.0];
    worker.selection = Some((1, source.text.len() - 1));
    worker.raster.enabled = retained;
    worker.reset_raster_context();
    (worker, output, source, row.len())
}

#[test]
fn visible_overlap_restyles_and_copies_the_full_selection_with_both_endpoints_missing() {
    for retained in [false, true] {
        let (mut worker, output, source, row) = long_selection_worker(retained);
        let incoming = fragment(&source, row * 5..row * 20);
        let first = painted(&mut worker, &output);
        let before_copy = copied(&mut worker, &output, first.revision);
        assert_eq!(before_copy, source.text[1..source.text.len() - 1]);
        let selected = worker.selection;
        let origin = worker.origin;
        let visible = visual_source(&worker);
        let acknowledged = worker.positions.acknowledged;
        let generation = worker.positions.generation;
        assert!(incoming.source.offset > source.source.offset + 1);
        assert!(incoming.source.end().unwrap() < source.source.end().unwrap() - 1);
        ready(&mut worker, &output, 61);
        worker.text.syntax.refresh_admitted(62);
        worker.event(62, EventKind::TextPage(incoming), &output);
        worker.event(62, EventKind::Completed, &output);
        let second = painted(&mut worker, &output);
        // Retained frames carry their pixels as prepared strips.
        let shown = |frame: &ViewerFrame| {
            if frame.raster.is_some() {
                super::super::raster::stacked(frame).1
            } else {
                frame.pixels.to_vec()
            }
        };
        assert_ne!(
            shown(second.as_ref()),
            shown(first.as_ref()),
            "styles must reach visible pixels even during a long selection"
        );
        assert_eq!(
            worker.page.as_ref().unwrap().text.as_ref(),
            source.text.as_str()
        );
        assert_eq!(worker.page.as_ref().unwrap().anchors, source.anchors);
        assert_eq!(worker.selection, selected);
        assert_eq!(worker.origin, origin);
        assert_eq!(visual_source(&worker), visible);
        assert_eq!(copied(&mut worker, &output, second.revision), before_copy);
        assert_eq!(worker.positions.acknowledged, acknowledged);
        assert_eq!(worker.positions.generation, generation);
        assert_eq!(second.position_sequence, first.position_sequence);
        if retained {
            let old = first.raster.as_ref().unwrap();
            let new = second.raster.as_ref().unwrap();
            assert_eq!(new.surface, old.surface);
            assert_eq!(new.origin, old.origin);
            assert_eq!(new.pan_sequence, old.pan_sequence);
        }
    }
}

/// Scrolling within the shown page keeps the preparation's target and the work admitted for
/// it: the rows it prepares are still the page's, so its refresh colours them. Before, every
/// scroll aimed preparation at the newly visible row and dropped the work under way.
#[test]
fn in_page_scroll_keeps_the_target_and_its_admitted_work() {
    let (mut worker, output) = source_worker();
    // The page holds the whole file: scrolling within it reads no page.
    let mut whole = decoded(false);
    (whole.has_before, whole.has_after) = (false, false);
    assert!(worker.set_page(Page::text(whole, &worker.appearance), &output));
    worker.origin = [0.0, 24.0];
    let previous = worker.text.syntax.latest.clone().unwrap();
    worker.text.syntax.started(61, previous.clone());
    worker.input(ViewerInput::Scroll { x: 0.0, y: 19.0 }, &output);
    assert_ne!(visual_source(&worker).0, previous.viewport.source_anchor);
    assert_eq!(worker.text.syntax.latest.as_ref(), Some(&previous));
    assert!(
        !worker
            .text
            .syntax
            .needs_preparation(std::time::Instant::now()),
        "the admitted request is still under way"
    );
    worker.event(
        61,
        EventKind::SourcePrepared(messages::SourcePrepared {
            revision: 9,
            frontier: 8192,
            ready: true,
        }),
        &output,
    );
    worker.event(61, EventKind::Completed, &output);
    assert_eq!(
        worker
            .text
            .syntax
            .refresh
            .as_ref()
            .map(|refresh| &refresh.key),
        Some(&previous),
        "its refresh colours the rows it prepared"
    );
    assert_eq!(worker.positions.acknowledged, [0; 2]);
}

/// A page of `count` rows of plain C-like source, larger than one walk, at rest at its start.
fn plain_rows_worker(count: usize) -> (Worker, Mailbox, viewer::TextPage, usize) {
    let (mut worker, output) = source_worker();
    // Larger than one walk: rows are prepared, not the whole document.
    worker.length = 1 << 20;
    let row = "int selected_value = 42; // generated\n";
    let source = source_text(row.repeat(count));
    assert!(worker.set_page(Page::text(source.clone(), &worker.appearance), &output));
    worker.origin = [0.0, 0.0];
    worker.selection = None;
    // The page is new: nothing is prepared for it yet.
    worker.text.syntax.latest = None;
    (worker, output, source, row.len())
}

/// The source offset of the row at page y `y`.
fn row_at(worker: &Worker, y: f32) -> u64 {
    let start = worker
        .shaped
        .as_ref()
        .unwrap()
        .line_at_y(y.max(0.0))
        .unwrap()
        .0
        .start;
    let page = worker.page.as_ref().unwrap();
    page.start + start as u64
}

/// A refresh that colours rows away from where the view has moved colours them where they
/// are, and the visible rows, still plain, are what the next rest prepares: all the plain rows
/// from their start, which one refresh reads through the view. Before, a refresh that missed
/// the visible row was dropped for one corrective target.
#[test]
fn a_refresh_away_from_the_view_colours_its_rows_and_the_visible_rows_come_next() {
    let (mut worker, output, source, row) = plain_rows_worker(96);
    worker.source_target();
    let first = worker.text.syntax.latest.clone().unwrap();
    assert_eq!(first.viewport.source_anchor, source.source.offset);
    assert!(!first.viewport.backwards);
    assert_eq!(first.viewport.max_lines, 97, "all the plain rows");
    // The refresh read ten rows; meanwhile the view moved down to row 60.
    ready(&mut worker, &output, 61);
    worker.text.syntax.refresh_admitted(62);
    let y = worker
        .shaped
        .as_ref()
        .unwrap()
        .caret_geometry(row * 60)
        .unwrap()
        .y0 as f32;
    worker.origin = [0.0, y];
    let visible = visual_source(&worker);
    worker.event(
        62,
        EventKind::TextPage(fragment(&source, 0..row * 10)),
        &output,
    );
    worker.event(62, EventKind::Completed, &output);
    let spans = &worker.page.as_ref().unwrap().spans;
    assert_eq!(spans.len(), 1);
    assert_eq!(spans[0].range, 0..row * 10);
    assert_eq!(visual_source(&worker), visible);
    assert!(worker.visible_rows_plain());
    worker.source_target();
    let next = worker.text.syntax.latest.clone().unwrap();
    assert!(!next.viewport.backwards);
    assert_eq!(
        next.viewport.source_anchor,
        source.source.offset + (row * 10) as u64,
        "from the first plain row"
    );
    assert_eq!(
        next.viewport.max_lines, 87,
        "through the view to the last plain row"
    );
    assert_eq!(worker.positions.acknowledged, [0; 2]);
}

/// Plain rows that start further before the view than one refresh reads through it are
/// prepared from a screen above the view.
#[test]
fn plain_rows_far_above_the_view_are_prepared_from_a_screen_above_it() {
    let (mut worker, _output, source, row) = plain_rows_worker(3000);
    let y = worker
        .shaped
        .as_ref()
        .unwrap()
        .caret_geometry(row * 2500)
        .unwrap()
        .y0 as f32;
    worker.origin = [0.0, y];
    worker.source_target();
    let target = worker.text.syntax.latest.clone().unwrap();
    assert!(source.text.len() > 64 * 1024);
    assert!(!target.viewport.backwards);
    assert_eq!(
        target.viewport.source_anchor,
        row_at(&worker, y - worker.geometry.height as f32),
        "a screen above the view"
    );
}

/// Rows a refresh leaves plain hold no token: they are settled, and nothing prepares them
/// again or shows the notice for them while the page stays. Before, a refresh that missed
/// the view could start one corrective walk.
#[test]
fn rows_a_refresh_leaves_plain_are_settled_and_never_prepared_again() {
    let (mut worker, output, source, _) = long_selection_worker(false);
    worker.length = 1 << 20;
    worker.source_target();
    assert!(worker.visible_rows_plain());
    let target = worker.text.syntax.latest.clone().unwrap();
    ready(&mut worker, &output, 61);
    worker.text.syntax.refresh_admitted(62);
    let mut plain = fragment(&source, 0..source.text.len());
    plain.spans.clear();
    worker.event(62, EventKind::TextPage(plain), &output);
    worker.event(62, EventKind::Completed, &output);
    assert!(worker.status.is_empty(), "{}", worker.status);
    assert!(!worker.visible_rows_plain());
    worker.source_target();
    assert!(
        worker.text.syntax.latest.is_none(),
        "the target {target:?} is not prepared again"
    );
    assert!(
        !worker
            .text
            .syntax
            .needs_preparation(std::time::Instant::now() + Duration::from_secs(30))
    );
}

/// What a refresh leaves of rows found plain is prepared next, however few rows it is: a
/// refresh read only part of a run of plain rows.
#[test]
fn what_a_refresh_leaves_of_plain_rows_is_prepared_next_however_few() {
    let (mut worker, output, source, row) = plain_rows_worker(64);
    worker.source_target();
    let first = worker.text.syntax.latest.clone().unwrap();
    assert_eq!(first.viewport.source_anchor, source.source.offset);
    ready(&mut worker, &output, 61);
    worker.text.syntax.refresh_admitted(62);
    // The refresh's reads ended five rows before the plain run did.
    let read = row * 59;
    worker.event(62, EventKind::TextPage(fragment(&source, 0..read)), &output);
    worker.event(62, EventKind::Completed, &output);
    assert!(source.text.len() - read < 2048);
    worker.source_target();
    let next = worker.text.syntax.latest.clone().unwrap();
    assert_eq!(
        next.viewport.source_anchor,
        source.source.offset + read as u64
    );
    assert!(!next.viewport.backwards);
    ready(&mut worker, &output, 63);
    worker.text.syntax.refresh_admitted(64);
    worker.event(
        64,
        EventKind::TextPage(fragment(&source, read..source.text.len())),
        &output,
    );
    worker.event(64, EventKind::Completed, &output);
    worker.source_target();
    assert!(worker.text.syntax.latest.is_none());
    assert_eq!(
        worker.page.as_ref().unwrap().spans.len(),
        1,
        "the two refreshes' spans join"
    );
}

/// Rows read again with colours are no longer plain, however much of them was found plain
/// before: only the short stretches between their tokens are uncoloured.
#[test]
fn rows_found_plain_and_read_again_with_colours_are_not_prepared() {
    let (mut worker, output, mut source, row) = plain_rows_worker(64);
    worker.source_target();
    assert!(worker.text.syntax.latest.is_some());
    // The same rows read again, coloured as the Source viewer colours them.
    source.spans = (0..64)
        .map(|index| viewer::Span {
            start: (index * row) as u32,
            end: (index * row + 3) as u32,
            role: viewer::STYLE_KEYWORD,
            flags: 0,
        })
        .collect();
    worker.invalidate_source_target();
    assert!(worker.set_page(Page::text(source, &worker.appearance), &output));
    worker.origin = [0.0, 0.0];
    assert!(!worker.visible_rows_plain());
    worker.source_target();
    assert!(worker.text.syntax.latest.is_none());
}

/// A page of unwrapped rows takes more colours than a Source packet (row groups,
/// `midnight_viewer_render::MAX_ROWS_SPANS`): colouring its last plain rows keeps every
/// colour it had. Before, the restyle was refused at a packet's 4,096 colours.
#[test]
fn a_restyle_colours_plain_rows_of_a_page_holding_more_than_a_packet_of_colours() {
    let (mut worker, output) = source_worker();
    worker.length = 1 << 20;
    let row = "1 2 3 4 5 6 7 8 9 0\n";
    let mut source = source_text(row.repeat(450));
    let digits = |rows: std::ops::Range<usize>| {
        rows.flat_map(|index| (0..10).map(move |digit| index * row.len() + digit * 2))
            .map(|at| viewer::Span {
                start: at as u32,
                end: at as u32 + 1,
                role: viewer::STYLE_NUMBER,
                flags: 0,
            })
            .collect::<Vec<_>>()
    };
    source.spans = digits(0..400);
    assert!(worker.set_page(Page::text(source.clone(), &worker.appearance), &output));
    worker.origin = [0.0, 0.0];
    worker.selection = None;
    ready(&mut worker, &output, 61);
    worker.text.syntax.refresh_admitted(62);
    let mut rows = fragment(&source, row.len() * 400..source.text.len());
    rows.spans = digits(0..50);
    worker.event(62, EventKind::TextPage(rows), &output);
    worker.event(62, EventKind::Completed, &output);
    assert!(worker.status.is_empty(), "{}", worker.status);
    assert_eq!(worker.page.as_ref().unwrap().spans.len(), 4500);
}

#[test]
fn mismatched_text_raw_anchor_or_revision_cannot_apply_overlap_styles() {
    for invalid in 0..3 {
        let (mut worker, output, source, row) = long_selection_worker(false);
        let mut incoming = fragment(&source, row * 5..row * 20);
        match invalid {
            0 => incoming.text.replace_range(0..1, "z"),
            1 => incoming.anchors[3].source_offset += 1,
            _ => incoming.revision = 8,
        }
        incoming.validate().unwrap();
        let selection = worker.selection;
        let origin = worker.origin;
        ready(&mut worker, &output, 61);
        worker.text.syntax.refresh_admitted(62);
        worker.event(62, EventKind::TextPage(incoming), &output);
        worker.event(62, EventKind::Completed, &output);
        assert!(worker.page.as_ref().unwrap().spans.is_empty());
        assert_eq!(
            worker.page.as_ref().unwrap().text.as_ref(),
            source.text.as_str()
        );
        assert_eq!(worker.selection, selection);
        assert_eq!(worker.origin, origin);
        assert!(
            !worker
                .text
                .syntax
                .needs_preparation(std::time::Instant::now() + Duration::from_secs(30))
        );
        assert!(!worker.status.is_empty());
    }
}

#[test]
fn repeated_verified_overlap_replaces_style_records_without_accumulating_layers() {
    let (mut worker, output, source, row) = long_selection_worker(false);
    for iteration in 0..32 {
        if iteration != 0 {
            worker.invalidate_source_target();
        }
        let mut incoming = fragment(&source, row * 5..row * 20);
        incoming.spans[0].role = if iteration % 2 == 0 {
            viewer::STYLE_STRING
        } else {
            viewer::STYLE_NUMBER
        };
        incoming.spans[0].flags = if iteration % 2 == 0 { 4 } else { 0 };
        ready(&mut worker, &output, 100 + iteration * 2);
        worker.text.syntax.refresh_admitted(101 + iteration * 2);
        worker.event(101 + iteration * 2, EventKind::TextPage(incoming), &output);
        worker.event(101 + iteration * 2, EventKind::Completed, &output);
        assert_eq!(
            worker.page.as_ref().unwrap().text.as_ref(),
            source.text.as_str()
        );
        assert_eq!(worker.page.as_ref().unwrap().spans.len(), 1);
        assert_eq!(
            worker.page.as_ref().unwrap().styles.len(),
            if iteration % 2 == 0 { 1 } else { 0 }
        );
        assert_eq!(worker.selection, Some((1, source.text.len() - 1)));
    }
}

#[test]
fn overlap_replacement_splits_and_preserves_styles_on_both_outside_edges() {
    let (mut worker, output, mut source, row) = long_selection_worker(false);
    source.spans = vec![viewer::Span {
        start: 0,
        end: source.text.len() as u32,
        role: viewer::STYLE_COMMENT,
        flags: 4,
    }];
    let origin = worker.origin;
    let selection = worker.selection;
    assert!(worker.set_page(Page::text(source.clone(), &worker.appearance), &output));
    worker.origin = origin;
    worker.selection = selection;
    let incoming = fragment(&source, row * 5..row * 20);
    ready(&mut worker, &output, 61);
    worker.text.syntax.refresh_admitted(62);
    worker.event(62, EventKind::TextPage(incoming), &output);
    worker.event(62, EventKind::Completed, &output);
    let page = worker.page.as_ref().unwrap();
    assert_eq!(
        page.spans
            .iter()
            .map(|span| span.range.clone())
            .collect::<Vec<_>>(),
        vec![0..row * 5, row * 5..row * 20, row * 20..source.text.len(),]
    );
    assert_eq!(page.spans[0].color, page.spans[2].color);
    assert_ne!(page.spans[0].color, page.spans[1].color);
    assert_eq!(
        page.styles
            .iter()
            .map(|style| style.range.clone())
            .collect::<Vec<_>>(),
        vec![0..row * 5, row * 20..source.text.len(),]
    );
    assert_eq!(worker.selection, selection);
}

/// A wrapped page is one layout, which takes a Source packet's colours: a restyle past them
/// keeps the page and its selection and says why.
#[test]
fn actual_aggregate_style_budget_exhaustion_keeps_the_loaded_selection_and_page() {
    let (mut worker, output) = source_worker();
    worker.wrap = true;
    let mut source = source_text("x ".repeat(6000));
    source.spans = (0..4096)
        .map(|at| viewer::Span {
            start: at,
            end: at + 1,
            role: if at % 2 == 0 {
                viewer::STYLE_NUMBER
            } else {
                viewer::STYLE_STRING
            },
            flags: 0,
        })
        .collect();
    assert!(worker.set_page(Page::text(source.clone(), &worker.appearance), &output));
    let caret = worker
        .shaped
        .as_ref()
        .unwrap()
        .caret_geometry(5000)
        .unwrap();
    // The wrapped row's visual line that holds offset 5000, inside the refreshed range.
    worker.origin = [0.0, caret.y0 as f32];
    worker.selection = Some((1, source.text.len() - 1));
    let first = painted(&mut worker, &output);
    let original = copied(&mut worker, &output, first.revision);
    ready(&mut worker, &output, 61);
    worker.text.syntax.refresh_admitted(62);
    worker.event(
        62,
        EventKind::TextPage(fragment(&source, 4500..6500)),
        &output,
    );
    worker.event(62, EventKind::Completed, &output);
    assert_eq!(worker.page.as_ref().unwrap().spans.len(), 4096);
    assert_eq!(worker.selection, Some((1, source.text.len() - 1)));
    assert!(worker.status.contains("budget"), "{}", worker.status);
    let second = painted(&mut worker, &output);
    assert_eq!(copied(&mut worker, &output, second.revision), original);
    assert!(
        !worker
            .text
            .syntax
            .needs_preparation(std::time::Instant::now() + Duration::from_secs(30))
    );
}

#[test]
fn palette_recolor_keeps_split_overlap_roles_and_outside_semantics() {
    let (mut worker, output, mut source, row) = long_selection_worker(false);
    source.spans = vec![viewer::Span {
        start: 0,
        end: source.text.len() as u32,
        role: viewer::STYLE_COMMENT,
        flags: 4,
    }];
    let origin = worker.origin;
    let selection = worker.selection;
    assert!(worker.set_page(Page::text(source.clone(), &worker.appearance), &output));
    worker.origin = origin;
    worker.selection = selection;
    ready(&mut worker, &output, 61);
    worker.text.syntax.refresh_admitted(62);
    worker.event(
        62,
        EventKind::TextPage(fragment(&source, row * 5..row * 20)),
        &output,
    );
    worker.event(62, EventKind::Completed, &output);
    let expected_roles = vec![
        crate::page::RoleSpan {
            range: 0..row * 5,
            role: viewer::STYLE_COMMENT,
        },
        crate::page::RoleSpan {
            range: row * 5..row * 20,
            role: viewer::STYLE_STRING,
        },
        crate::page::RoleSpan {
            range: row * 20..source.text.len(),
            role: viewer::STYLE_COMMENT,
        },
    ];
    assert_eq!(worker.page.as_ref().unwrap().roles, expected_roles);
    worker.position_page = Some((0, worker.page.as_ref().unwrap().clone()));
    let mut palette = worker.appearance.clone();
    palette.background = [250; 4];
    palette.foreground = [20, 21, 22, 255];
    worker.palette_state.restore(&palette, || Ok(())).unwrap();
    worker.sync_palette(&output);
    source.spans = expected_roles
        .iter()
        .map(|span| viewer::Span {
            start: span.range.start as u32,
            end: span.range.end as u32,
            role: span.role,
            flags: 0,
        })
        .collect();
    let expected = Page::text(source, &palette);
    let actual = worker.page.as_ref().unwrap();
    assert_eq!(actual.roles, expected_roles);
    for ((actual, cached), expected) in actual
        .spans
        .iter()
        .zip(&worker.position_page.as_ref().unwrap().1.spans)
        .zip(&expected.spans)
    {
        assert_eq!(actual.range, expected.range);
        assert_eq!(actual.color, expected.color);
        assert_eq!(cached.color, expected.color);
    }
    assert_eq!(worker.selection, selection);
    assert_eq!(worker.origin, origin);
}
