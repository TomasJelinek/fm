use super::*;

#[test]
fn source_overlapping_seek_chain_keeps_known_lines_when_first_column_is_unknown() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    let source: String = (0..90)
        .map(|i| format!("{{\"row\":{i},\"name\":\"東京\"}}\n"))
        .collect();
    let starts: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    worker.length = source.len() as u64;
    for (start, end) in [
        (0, starts[40]),
        (starts[20] + 5, starts[60]),
        (starts[10] + 7, starts[50]),
        (starts[2] + 3, starts[18]),
    ] {
        assert!(worker.set_page(source_page(&source, start, end), &output));
    }
    worker.prepare_gutter().unwrap();
    assert_eq!(
        worker
            .text
            .gutter
            .as_ref()
            .unwrap()
            .first()
            .expect("known lines survive even when column prefix is outside overlap")
            .0
            .text(),
        "3"
    );
}

#[test]
fn source_unwrapped_bounded_fragment_navigation_always_makes_progress() {
    let (mut worker, _) = worker();
    let source = format!("prior\n{}tail", "word".repeat(2048));
    let page = source_page(&source, 6, 6 + 8192);
    let shaped = worker.layout_page(&page).unwrap();
    assert!(shaped.height() < worker.geometry.height as f32);
    let forward =
        crate::viewport::Continuation::capture(&page, &shaped, worker.geometry, [0.0; 2], 17.0)
            .unwrap();
    assert_eq!(
        forward.anchor, page.end,
        "a huge logical line must not reload its first fragment"
    );
    let backward =
        crate::viewport::Continuation::capture(&page, &shaped, worker.geometry, [0.0; 2], -17.0)
            .unwrap();
    assert_eq!(
        backward.anchor, page.start,
        "backward endpoint excludes the current bounded fragment"
    );
}

#[test]
fn source_page_refills_before_exposing_its_unloaded_last_visual_line() {
    for trailing_newline in [false, true] {
        let (mut worker, _) = worker();
        let mut source: String = (0..40).map(|i| format!("row {i}\n")).collect();
        if !trailing_newline {
            source.pop();
        }
        let mut page = source_page(&source, 0, source.len());
        page.after = true;
        let shaped = worker.layout_page(&page).unwrap();
        let (_, last_top) = shaped.line_at_y(shaped.height()).unwrap();
        let geometry = ViewerGeometry {
            width: 240,
            height: 100,
            scale: 1.0,
        };
        let origin = [0.0, last_top - geometry.height as f32 - 1.0];
        assert!(
            crate::viewport::Continuation::capture(&page, &shaped, geometry, origin, 2.0).is_some(),
            "a not-yet-loaded row must not enter the displayed viewport (newline={trailing_newline})"
        );
        page.after = false;
        assert!(
            crate::viewport::Continuation::capture(&page, &shaped, geometry, origin, 2.0).is_none(),
            "real EOF keeps its ordinary final line"
        );
    }
}

#[test]
fn source_empty_optional_locate_is_per_page_and_near_coordinates_can_recover() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    assert!(worker.set_page(text_page("far fragment".into()), &output));
    worker.text.locate_request = Some((77, 8000));
    worker.event(77, EventKind::Completed, &output);
    assert!(worker.text.unavailable);
    assert_eq!(worker.text.located_start, Some(8000));
    let source = "first\nsecond\nthird\n";
    assert!(worker.set_page(source_page(source, 6, source.len()), &output));
    assert!(
        worker
            .source_status(
                worker.page.as_ref().unwrap(),
                worker.shaped.as_ref().unwrap()
            )
            .contains("indexing"),
        "a later page must retry its cheap optional lookup"
    );
    worker.text.locate_request = Some((78, 6));
    worker.event(
        78,
        EventKind::TextPosition(viewer::TextPosition {
            revision: 1,
            source_offset: 6,
            line: 2,
            column: 1,
        }),
        &output,
    );
    worker.event(78, EventKind::Completed, &output);
    assert!(
        worker
            .source_status(
                worker.page.as_ref().unwrap(),
                worker.shaped.as_ref().unwrap()
            )
            .starts_with("Ln 2, Col 1")
    );
}

fn source_page(source: &str, start: usize, end: usize) -> Page {
    let text = source[start..end].to_owned();
    Page::text(
        viewer::TextPage {
            revision: 1,
            source: viewer::SourceRange {
                offset: start as u64,
                length: (end - start) as u64,
            },
            anchors: text
                .char_indices()
                .map(|(at, _)| viewer::Anchor {
                    text_offset: at as u32,
                    source_offset: (start + at) as u64,
                })
                .chain(std::iter::once(viewer::Anchor {
                    text_offset: text.len() as u32,
                    source_offset: end as u64,
                }))
                .collect(),
            text,
            spans: Vec::new(),
            has_before: start != 0,
            has_after: end != source.len(),
            partial_line: false,
        },
        &appearance(),
    )
}

fn appearance() -> ViewerAppearance {
    ViewerAppearance {
        family: "monospace".into(),
        font_size: 14.0,
        foreground: [31, 35, 40, 255],
        background: [255; 4],
        selection: [180, 210, 250, 255],
        accent: [9, 105, 218, 255],
    }
}

#[test]
fn source_scroll_first_frame_keeps_exact_overlap_line_numbers_without_locate() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    let source: String = (0..90)
        .map(|i| format!("{{\"row\":{i},\"name\":\"東京\"}}\n"))
        .collect();
    let starts: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    worker.length = source.len() as u64;
    assert!(worker.set_page(source_page(&source, 0, starts[40]), &output));
    assert!(worker.set_page(source_page(&source, starts[5], starts[45]), &output));
    let coordinate = worker
        .text
        .coordinate
        .expect("overlap keeps its known source coordinate");
    assert_eq!(
        (coordinate.source_offset, coordinate.line, coordinate.column),
        (starts[5] as u64, 6, Some(1))
    );
    worker.prepare_gutter().unwrap();
    let labels = worker.text.gutter.as_ref().unwrap();
    assert_eq!(labels.first().unwrap().0.text(), "6");
    assert!(labels.len() >= 40);
    assert!(worker.set_page(source_page(&source, starts[2], starts[42]), &output));
    worker.prepare_gutter().unwrap();
    assert_eq!(
        worker
            .text
            .gutter
            .as_ref()
            .unwrap()
            .first()
            .unwrap()
            .0
            .text(),
        "3"
    );
}
