use midnight_viewer_render::{
    CellAlignment, DocumentBlock, DocumentKind, DocumentStyle, TableCell, TableRow, TextOptions,
    TextRenderer, Viewport,
};

fn fixture(values: &[&[&str]]) -> (String, Vec<DocumentBlock>, Vec<TableRow>) {
    let mut text = String::new();
    let mut blocks = Vec::new();
    let mut rows = Vec::new();
    for (index, values) in values.iter().enumerate() {
        if !text.is_empty() {
            text.push_str("\n\n");
        }
        let start = text.len();
        let mut cells = Vec::new();
        for (column, value) in values.iter().enumerate() {
            if column > 0 {
                text.push('\t');
            }
            let start = text.len();
            text.push_str(value);
            cells.push(TableCell {
                range: start..text.len(),
                alignment: if column == 0 {
                    CellAlignment::Left
                } else {
                    CellAlignment::Right
                },
            });
        }
        blocks.push(DocumentBlock {
            range: start..text.len(),
            kind: DocumentKind::Paragraph,
            depth: 0,
        });
        rows.push(TableRow {
            range: start..text.len(),
            cells,
            header: index == 0,
        });
    }
    (text, blocks, rows)
}

#[test]
fn narrow_github_tables_keep_numeric_tokens_whole_and_scroll_when_minimum_width_is_needed() {
    let (text, blocks, rows) = fixture(&[
        &["workload", "ovstage total", "OpenUSD total"],
        &[
            "A workload with several words",
            "1234567.890",
            "9876543.210",
        ],
    ]);
    let options = TextOptions {
        family: "system-ui",
        size: 16.0,
        line_height: Some(1.5),
        wrap_width: Some(180.0),
        ..Default::default()
    };
    let page = TextRenderer::default()
        .layout_document(
            &text,
            &options,
            &DocumentStyle {
                colors: &[],
                styles: &[],
                tables: &rows,
                blocks: &blocks,
                background: [255; 4],
                start_of_document: false,
            },
        )
        .unwrap();
    assert!(
        page.width() > 250.0,
        "unbreakable content must be horizontally scrollable"
    );
    for cell in &rows[1].cells[1..] {
        let first = page.caret_geometry(cell.range.start).unwrap();
        let last = page.caret_geometry(cell.range.end - 1).unwrap();
        assert!(
            (first.y0 - last.y0).abs() < 0.1,
            "numeric value split across lines"
        );
        assert_eq!(
            page.selected_text(cell.range.clone()).unwrap(),
            &text[cell.range.clone()]
        );
    }
    let first = page.caret_geometry(rows[1].cells[0].range.start).unwrap();
    let last = page.caret_geometry(rows[1].cells[0].range.end - 1).unwrap();
    assert!(
        last.y0 > first.y0,
        "ordinary words still wrap at narrow widths"
    );
    let number = page.caret_geometry(rows[1].cells[1].range.start).unwrap();
    assert!(
        ((number.y0 + number.y1) - (first.y0 + last.y1)).abs() < 1.0,
        "short numeric cells align vertically to the middle of wrapped labels"
    );
}

#[test]
fn github_striping_starts_with_the_second_body_row_and_uses_single_shared_borders() {
    let (text, blocks, rows) = fixture(&[&["Name", "Value"], &["first", "1"], &["second", "2"]]);
    let options = TextOptions {
        family: "system-ui",
        size: 16.0,
        line_height: Some(1.5),
        wrap_width: Some(400.0),
        ..Default::default()
    };
    let mut renderer = TextRenderer::default();
    let page = renderer
        .layout_document(
            &text,
            &options,
            &DocumentStyle {
                colors: &[],
                styles: &[],
                tables: &rows,
                blocks: &blocks,
                background: [255; 4],
                start_of_document: false,
            },
        )
        .unwrap();
    let raster = renderer
        .rasterize(
            &page,
            Viewport {
                width: 400,
                height: 160,
                origin: [0.0; 2],
                background: [255; 4],
                selection_color: [0; 4],
            },
            None,
        )
        .unwrap();
    let sample = |row: usize| {
        let caret = page.caret_geometry(rows[row].cells[0].range.start).unwrap();
        let offset = (caret.y0.ceil() as usize * 400 + 2) * 4;
        &raster.rgba[offset..offset + 4]
    };
    assert_eq!(sample(0), &[255; 4]);
    assert_eq!(sample(1), &[255; 4], "first tbody row is unstriped");
    assert_ne!(
        sample(2),
        &[255; 4],
        "second tbody row receives the alternate background"
    );
    let header = page.caret_geometry(rows[0].cells[0].range.start).unwrap();
    let first = page.caret_geometry(rows[1].cells[0].range.start).unwrap();
    assert!(
        (first.y0 - header.y0 - 37.0).abs() < 0.5,
        "24px line + 12px padding + one shared border"
    );
}

#[test]
fn compressed_table_borders_keep_one_physical_pixel_at_unit_scale() {
    let (text, blocks, rows) = fixture(&[
        &["Description", "Backend", "Result"],
        &[
            "several words with unequal widths",
            "more words of different lengths",
            "123.456",
        ],
    ]);
    let mut renderer = TextRenderer::default();
    let page = renderer
        .layout_document(
            &text,
            &TextOptions {
                family: "system-ui",
                size: 16.0,
                line_height: Some(1.5),
                foreground: [31, 35, 40, 255],
                wrap_width: Some(340.0),
                ..Default::default()
            },
            &DocumentStyle {
                colors: &[],
                styles: &[],
                tables: &rows,
                blocks: &blocks,
                background: [255; 4],
                start_of_document: false,
            },
        )
        .unwrap();
    let raster = renderer
        .rasterize(
            &page,
            Viewport {
                width: 400,
                height: 200,
                origin: [0.0; 2],
                background: [255; 4],
                selection_color: [0; 4],
            },
            None,
        )
        .unwrap();
    let mut widths = Vec::new();
    let mut width = 0;
    // Sample inside top padding: only vertical borders cross this scanline.
    for pixel in raster.rgba[(2 * 400) * 4..(3 * 400) * 4].as_chunks::<4>().0 {
        if pixel[0] < 230 {
            width += 1;
        } else if width != 0 {
            widths.push(width);
            width = 0;
        }
    }
    assert_eq!(
        widths,
        [1, 1, 1, 1],
        "fractional columns must not double the border stroke"
    );
}

#[test]
fn table_raster_stays_identical_when_preceding_document_blocks_change() {
    let (table, table_blocks, table_rows) = fixture(&[
        &["Phase", "Time"],
        &["Computing initial mesh data", "0.001"],
        &["Cooking meshes", "0.004"],
    ]);
    let mut renderer = TextRenderer::default();
    let mut reference = None;
    for paragraphs in 0..12 {
        let mut text = String::new();
        let mut blocks = Vec::new();
        for prefix in 0..paragraphs {
            let start = text.len();
            text.push_str("Prelude\n\n");
            blocks.push(DocumentBlock {
                range: start..start + 7,
                kind: if prefix % 3 == 0 {
                    DocumentKind::Heading(1)
                } else {
                    DocumentKind::Paragraph
                },
                depth: 0,
            });
        }
        let heading = text.len();
        text.push_str("Heading\n\n");
        blocks.push(DocumentBlock {
            range: heading..heading + 7,
            kind: DocumentKind::Heading(2),
            depth: 0,
        });
        let offset = text.len();
        text.push_str(&table);
        blocks.extend(table_blocks.iter().map(|block| DocumentBlock {
            range: block.range.start + offset..block.range.end + offset,
            ..block.clone()
        }));
        let rows: Vec<_> = table_rows
            .iter()
            .map(|row| TableRow {
                range: row.range.start + offset..row.range.end + offset,
                cells: row
                    .cells
                    .iter()
                    .map(|cell| TableCell {
                        range: cell.range.start + offset..cell.range.end + offset,
                        alignment: cell.alignment,
                    })
                    .collect(),
                header: row.header,
            })
            .collect();
        let page = renderer
            .layout_document(
                &text,
                &TextOptions {
                    family: "system-ui",
                    size: 16.0,
                    line_height: Some(1.5),
                    foreground: [31, 35, 40, 255],
                    wrap_width: Some(400.0),
                    ..Default::default()
                },
                &DocumentStyle {
                    colors: &[],
                    styles: &[],
                    tables: &rows,
                    blocks: &blocks,
                    background: [255; 4],
                    start_of_document: false,
                },
            )
            .unwrap();
        let anchor = page.caret_geometry(offset).unwrap();
        let raster = renderer
            .rasterize(
                &page,
                Viewport {
                    width: 400,
                    height: 100,
                    origin: [0.0, anchor.y0 as f32 - 7.0],
                    background: [255; 4],
                    selection_color: [0; 4],
                },
                None,
            )
            .unwrap();
        if let Some(reference) = &reference {
            let changed = raster
                .rgba
                .iter()
                .zip(reference)
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(
                changed, 0,
                "table moved after {paragraphs} preceding paragraphs"
            );
        } else {
            reference = Some(raster.rgba);
        }
    }
}

#[test]
fn table_borders_survive_caret_restoration_across_fractional_document_origins() {
    let (table, table_blocks, table_rows) = fixture(&[
        &["Phase", "Time"],
        &["Setup", "0.246"],
        &["Release previous data", "0.001"],
        &["Find meshes", "0.006"],
        &["Compute initial data", "0.000"],
        &["Find shared meshes", "0.000"],
        &["Cook meshes", "0.000"],
        &["Compute global data", "0.004"],
        &["Process results", "0.000"],
    ]);
    let mut renderer = TextRenderer::default();
    let mut reference = None;
    let mut caret_offset = 0.0;
    for paragraphs in 0..48 {
        let mut text = String::new();
        let mut blocks = Vec::new();
        for _ in 0..paragraphs {
            let start = text.len();
            text.push_str("Prelude\n\n");
            blocks.push(DocumentBlock {
                range: start..start + 7,
                // The H2 underline's 0.3em spacing creates fractional origins.
                kind: DocumentKind::Heading(2),
                depth: 0,
            });
        }
        let offset = text.len();
        text.push_str(&table);
        blocks.extend(table_blocks.iter().map(|block| DocumentBlock {
            range: block.range.start + offset..block.range.end + offset,
            ..block.clone()
        }));
        let rows: Vec<_> = table_rows
            .iter()
            .map(|row| TableRow {
                range: row.range.start + offset..row.range.end + offset,
                cells: row
                    .cells
                    .iter()
                    .map(|cell| TableCell {
                        range: cell.range.start + offset..cell.range.end + offset,
                        alignment: cell.alignment,
                    })
                    .collect(),
                header: row.header,
            })
            .collect();
        text.push_str("\n\n");
        let anchor = text.len();
        text.push_str("Following source anchor");
        blocks.push(DocumentBlock {
            range: anchor..text.len(),
            kind: DocumentKind::Heading(3),
            depth: 0,
        });
        let page = renderer
            .layout_document(
                &text,
                &TextOptions {
                    family: "system-ui",
                    size: 16.0,
                    line_height: Some(1.5),
                    foreground: [31, 35, 40, 255],
                    wrap_width: Some(400.0),
                    ..Default::default()
                },
                &DocumentStyle {
                    colors: &[],
                    styles: &[],
                    tables: &rows,
                    blocks: &blocks,
                    background: [255; 4],
                    start_of_document: false,
                },
            )
            .unwrap();
        let caret = page.caret_geometry(anchor).unwrap().y0 as f32;
        if reference.is_none() {
            caret_offset = 6.0 - caret;
        }
        // A continuation preserves a later source caret's screen offset while
        // restoring a larger page with different leading semantic blocks.
        let raster = renderer
            .rasterize(
                &page,
                Viewport {
                    width: 400,
                    height: 320,
                    origin: [0.0, caret + caret_offset],
                    background: [255; 4],
                    selection_color: [0; 4],
                },
                None,
            )
            .unwrap();
        if let Some(reference) = &reference {
            assert_eq!(
                raster
                    .rgba
                    .iter()
                    .zip(reference)
                    .filter(|(a, b)| a != b)
                    .count(),
                0,
                "border phase changed after restoring across {paragraphs} preceding headings"
            );
        } else {
            reference = Some(raster.rgba);
        }
    }
}

#[test]
fn composite_paint_normalizes_both_error_signs_without_erasing_fractional_scroll() {
    let (text, blocks, rows) = fixture(&[
        &["Phase", "Time"],
        &["Setup", "0.246"],
        &["Find meshes", "0.006"],
    ]);
    let mut renderer = TextRenderer::default();
    let page = renderer
        .layout_document(
            &text,
            &TextOptions {
                family: "system-ui",
                size: 16.0,
                line_height: Some(1.5),
                foreground: [31, 35, 40, 255],
                wrap_width: Some(400.0),
                ..Default::default()
            },
            &DocumentStyle {
                colors: &[],
                styles: &[],
                tables: &rows,
                blocks: &blocks,
                background: [255; 4],
                start_of_document: false,
            },
        )
        .unwrap();
    let paint = |renderer: &mut TextRenderer, y| {
        renderer
            .rasterize(
                &page,
                Viewport {
                    width: 400,
                    height: 100,
                    origin: [0.0, y],
                    background: [255; 4],
                    selection_color: [0; 4],
                },
                None,
            )
            .unwrap()
            .rgba
    };
    let reference = paint(&mut renderer, 6.0);
    for error in [-0.0001, 0.0001] {
        let pixels = paint(&mut renderer, 6.0 + error);
        assert!(
            pixels == reference,
            "cancellation error {error} changed pixels"
        );
    }
    for fraction in [0.25, 0.5] {
        let pixels = paint(&mut renderer, 6.0 + fraction);
        assert!(
            pixels != reference,
            "fractional scroll {fraction} was erased"
        );
    }
}
