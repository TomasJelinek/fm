use midnight_viewer_render::{
    CellAlignment, TableCell, TableRow, TextOptions, TextRenderer, Viewport,
};

fn rows() -> Vec<TableRow> {
    vec![
        TableRow {
            range: 0..13,
            header: true,
            cells: vec![
                TableCell {
                    range: 0..1,
                    alignment: CellAlignment::Left,
                },
                TableCell {
                    range: 2..13,
                    alignment: CellAlignment::Left,
                },
            ],
        },
        TableRow {
            range: 15..28,
            header: false,
            cells: vec![
                TableCell {
                    range: 15..26,
                    alignment: CellAlignment::Left,
                },
                TableCell {
                    range: 27..28,
                    alignment: CellAlignment::Left,
                },
            ],
        },
    ]
}

#[test]
fn table_cells_share_column_positions_and_preserve_clipboard_text() {
    let text = "A\tDescription\n\nLonger name\tB";
    let page = TextRenderer::default()
        .layout_with_tables(
            text,
            &TextOptions {
                wrap_width: Some(420.0),
                ..TextOptions::default()
            },
            &[],
            &[],
            &rows(),
        )
        .unwrap();
    let header = page.caret_geometry(2).unwrap();
    let body = page.caret_geometry(27).unwrap();
    assert!(
        (header.x0 - body.x0).abs() < 1.0,
        "column origins must be shared"
    );
    assert!(
        header.x0 > 100.0,
        "cells need genuine bounded column widths"
    );
    assert!(body.y0 > header.y0);
    assert_eq!(page.selected_text(0..text.len()).unwrap(), text);
}

#[test]
fn narrow_unicode_tables_wrap_cells_and_keep_selection_and_borders_in_place() {
    let text = "Čeština 文 🙂 and a long explanation\t42";
    let second = text.find('\t').unwrap() + 1;
    let rows = [TableRow {
        range: 0..text.len(),
        header: false,
        cells: vec![
            TableCell {
                range: 0..second - 1,
                alignment: CellAlignment::Left,
            },
            TableCell {
                range: second..text.len(),
                alignment: CellAlignment::Right,
            },
        ],
    }];
    let mut renderer = TextRenderer::default();
    let page = renderer
        .layout_with_tables(
            text,
            &TextOptions {
                wrap_width: Some(100.0),
                ..TextOptions::default()
            },
            &[],
            &[],
            &rows,
        )
        .unwrap();
    assert!(
        page.width() > 100.0,
        "narrow views retain readable columns and horizontal scrolling"
    );
    assert!(
        page.height() > 60.0,
        "long cell content wraps within its column"
    );
    let first = page.caret_geometry(0).unwrap();
    let last = page.caret_geometry(second).unwrap();
    assert!(
        last.x0 > page.width() as f64 * 0.7,
        "right alignment applies inside the second column"
    );
    assert_eq!(
        page.hit_test(first.x0 as f32 + 0.1, ((first.y0 + first.y1) / 2.0) as f32)
            .unwrap(),
        0
    );
    assert_eq!(page.selected_text(second..text.len()).unwrap(), "42");
    let selected = page.selection_geometry(second..text.len()).unwrap();
    assert!(
        selected
            .iter()
            .all(|rect| rect.x0 > page.width() as f64 / 2.0)
    );
    let raster = renderer
        .rasterize(
            &page,
            Viewport {
                width: 240,
                height: 200,
                origin: [0.0; 2],
                background: [0; 4],
                selection_color: [0; 4],
            },
            None,
        )
        .unwrap();
    assert!(raster.rgba[3] > 0, "the outer table border is painted");
    assert!(
        raster.rgba[(50 * 240) * 4 + 3] > 0,
        "vertical border continues alongside wrapped cells"
    );
}

#[test]
fn center_alignment_and_following_prose_use_the_same_document_geometry() {
    let text = "x\ty\n\nafter";
    let rows = [TableRow {
        range: 0..3,
        header: true,
        cells: vec![
            TableCell {
                range: 0..1,
                alignment: CellAlignment::Left,
            },
            TableCell {
                range: 2..3,
                alignment: CellAlignment::Center,
            },
        ],
    }];
    let page = TextRenderer::default()
        .layout_with_tables(
            text,
            &TextOptions {
                wrap_width: Some(400.0),
                ..TextOptions::default()
            },
            &[],
            &[],
            &rows,
        )
        .unwrap();
    let center = page.caret_geometry(2).unwrap();
    assert!(center.x0 > 275.0 && center.x0 < 310.0);
    assert!(page.caret_geometry(5).unwrap().y0 > center.y1);
    assert_eq!(page.line_at_y(center.y0 as f32 + 1.0).unwrap().0.start, 0);
}

#[test]
fn malformed_table_ranges_cannot_panic_or_escape_the_page_budget() {
    let mut renderer = TextRenderer::default();
    for row in [
        TableRow {
            range: 0..3,
            header: false,
            cells: vec![TableCell {
                range: 1..3,
                alignment: CellAlignment::Left,
            }],
        },
        TableRow {
            range: 0..3,
            header: false,
            cells: vec![TableCell {
                range: 0..4,
                alignment: CellAlignment::Left,
            }],
        },
        TableRow {
            range: 0..3,
            header: false,
            cells: vec![],
        },
    ] {
        assert!(
            renderer
                .layout_with_tables("文", &TextOptions::default(), &[], &[], &[row])
                .is_err()
        );
    }
}

#[test]
fn table_paging_ranges_include_later_cells_and_keep_wrapped_cell_progress() {
    use midnight_viewer_render::{DocumentBlock, DocumentKind, DocumentStyle};

    for suffix in ["", "\t42"] {
        let text =
            format!("before\n\nČeština 文 and a long explanation that wraps{suffix}\n\nafter");
        let start = "before\n\n".len();
        let end = text.len() - "\n\nafter".len();
        let cell_end = text[start..end].find('\t').map_or(end, |at| start + at);
        let mut cells = vec![TableCell {
            range: start..cell_end,
            alignment: CellAlignment::Left,
        }];
        if cell_end < end {
            cells.push(TableCell {
                range: cell_end + 1..end,
                alignment: CellAlignment::Right,
            });
        }
        let rows = [TableRow {
            range: start..end,
            header: false,
            cells,
        }];
        let blocks: Vec<_> = [0..6, start..end, end + 2..text.len()]
            .into_iter()
            .map(|range| DocumentBlock {
                range,
                kind: DocumentKind::Paragraph,
                depth: 0,
            })
            .collect();
        let options = TextOptions {
            wrap_width: Some(100.0),
            ..TextOptions::default()
        };
        let mut renderer = TextRenderer::default();
        for github in [false, true] {
            let page = if github {
                renderer.layout_document(
                    &text,
                    &options,
                    &DocumentStyle {
                        colors: &[],
                        styles: &[],
                        tables: &rows,
                        blocks: &blocks,
                        background: [255; 4],
                        start_of_document: true,
                    },
                )
            } else {
                renderer.layout_with_tables(&text, &options, &[], &[], &rows)
            }
            .unwrap();
            let first = page.caret_geometry(start).unwrap();
            let wrapped = page.caret_geometry(cell_end - 1).unwrap();
            assert!(wrapped.y0 > first.y1, "fixture must wrap its first cell");
            let (first_line, _) = page
                .line_at_y(((first.y0 + first.y1) / 2.0) as f32)
                .unwrap();
            let (last_line, _) = page
                .line_at_y(((wrapped.y0 + wrapped.y1) / 2.0) as f32)
                .unwrap();
            assert_eq!(first_line.start, start);
            assert!(
                last_line.start > start,
                "forward paging through a tall cell must progress"
            );
            for index in [start, cell_end - 1, end - 1] {
                let caret = page.caret_geometry(index).unwrap();
                let (range, _) = page
                    .line_at_y(((caret.y0 + caret.y1) / 2.0) as f32)
                    .unwrap();
                assert!(range.start >= start && range.end <= end);
                if !suffix.is_empty() {
                    assert_eq!(range.end, end, "backward paging must retain the later cell");
                }
            }
            if suffix.is_empty() {
                assert!(
                    first_line.end < end,
                    "backward paging through a tall cell must progress"
                );
            }
            assert_eq!(page.selected_text(start..end).unwrap(), &text[start..end]);
        }
    }
}
