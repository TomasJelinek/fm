use midnight_viewer_render::{
    DocumentBlock, DocumentKind, DocumentStyle, TextOptions, TextRenderer, Viewport,
};

fn block(range: std::ops::Range<usize>, kind: DocumentKind) -> DocumentBlock {
    DocumentBlock {
        range,
        kind,
        depth: 0,
    }
}

#[test]
fn github_headings_have_a_real_hierarchy_and_keep_unicode_hit_testing() {
    let text = "Title\n\nSecond\n\nThird\n\nBody 文";
    let blocks = [
        block(0..5, DocumentKind::Heading(1)),
        block(7..13, DocumentKind::Heading(2)),
        block(15..20, DocumentKind::Heading(3)),
        block(22..text.len(), DocumentKind::Paragraph),
    ];
    let mut renderer = TextRenderer::default();
    let page = renderer
        .layout_document(text, &options(), &style(&blocks))
        .unwrap();
    let height = |index| {
        let rect = page.caret_geometry(index).unwrap();
        rect.y1 - rect.y0
    };
    assert!(height(0) > height(7) * 1.2);
    assert!(height(7) > height(15) * 1.1);
    // GitHub h3 is 1.25em with 1.25 line-height, body is 1em with 1.5 line-height.
    assert!(height(15) > height(22));
    let caret = page.caret_geometry(27).unwrap();
    assert_eq!(
        page.hit_test(caret.x0 as f32 + 0.1, ((caret.y0 + caret.y1) / 2.0) as f32)
            .unwrap(),
        27
    );
    assert_eq!(page.selected_text(0..text.len()).unwrap(), text);
}

#[test]
fn code_and_quotes_have_real_insets_and_visible_theme_aware_decorations() {
    let text = "let value = 1;\n\nQuoted text";
    let blocks = [
        block(0..14, DocumentKind::Code),
        block(16..text.len(), DocumentKind::Quote),
    ];
    for (background, foreground) in [
        ([255, 255, 255, 255], [31, 35, 40, 255]),
        ([13, 17, 23, 255], [230, 237, 243, 255]),
    ] {
        let mut renderer = TextRenderer::default();
        let options = TextOptions {
            foreground,
            ..options()
        };
        let doc = DocumentStyle {
            background,
            ..style(&blocks)
        };
        let page = renderer.layout_document(text, &options, &doc).unwrap();
        let code = page.caret_geometry(0).unwrap();
        let quote = page.caret_geometry(16).unwrap();
        assert!(
            code.x0 >= 15.0 && quote.x0 >= 15.0,
            "block insets are real layout geometry"
        );
        let view = Viewport {
            width: 400,
            height: 200,
            origin: [0.0; 2],
            background,
            selection_color: [80, 130, 200, 255],
        };
        let raster = renderer.rasterize(&page, view, None).unwrap();
        let offset = ((code.y0 as usize + 1) * 400 + 2) * 4;
        assert_ne!(&raster.rgba[offset..offset + 4], &background);
        let quote_offset = ((quote.y0 as usize + 1) * 400 + 1) * 4;
        assert_ne!(&raster.rgba[quote_offset..quote_offset + 4], &background);
        let selected = renderer.rasterize(&page, view, Some(0..14)).unwrap();
        assert_ne!(
            raster.rgba, selected.rgba,
            "block backgrounds must not hide selection"
        );
    }
}

#[test]
fn list_depth_changes_indentation_without_expanding_every_gap_to_a_blank_line() {
    let text = "• First\n\n• Nested\n\n• Last";
    let second = text.find("• Nested").unwrap();
    let third = text.find("• Last").unwrap();
    let blocks = [
        DocumentBlock {
            depth: 1,
            ..block(0..second - 2, DocumentKind::ListItem)
        },
        DocumentBlock {
            depth: 2,
            ..block(second..third - 2, DocumentKind::ListItem)
        },
        DocumentBlock {
            depth: 1,
            ..block(third..text.len(), DocumentKind::ListItem)
        },
    ];
    let page = TextRenderer::default()
        .layout_document(text, &options(), &style(&blocks))
        .unwrap();
    let first = page.caret_geometry(0).unwrap();
    let nested = page.caret_geometry(second).unwrap();
    assert!(nested.x0 >= first.x0 + 30.0);
    assert!(nested.y0 - first.y0 < 40.0);
}

#[test]
fn github_tables_size_to_content_with_cell_padding_and_alternating_rows() {
    use midnight_viewer_render::{CellAlignment, TableCell, TableRow};
    let text = "Name\tValue\n\nx\t1\n\ny\t2";
    let blocks = [
        block(0..10, DocumentKind::Paragraph),
        block(12..15, DocumentKind::Paragraph),
        block(17..20, DocumentKind::Paragraph),
    ];
    let rows = [
        TableRow {
            range: 0..10,
            header: true,
            cells: vec![
                TableCell {
                    range: 0..4,
                    alignment: CellAlignment::Left,
                },
                TableCell {
                    range: 5..10,
                    alignment: CellAlignment::Left,
                },
            ],
        },
        TableRow {
            range: 12..15,
            header: false,
            cells: vec![
                TableCell {
                    range: 12..13,
                    alignment: CellAlignment::Left,
                },
                TableCell {
                    range: 14..15,
                    alignment: CellAlignment::Right,
                },
            ],
        },
    ];
    let mut renderer = TextRenderer::default();
    let doc = DocumentStyle {
        tables: &[
            rows[0].clone(),
            rows[1].clone(),
            TableRow {
                range: 17..20,
                header: false,
                cells: vec![
                    TableCell {
                        range: 17..18,
                        alignment: CellAlignment::Left,
                    },
                    TableCell {
                        range: 19..20,
                        alignment: CellAlignment::Right,
                    },
                ],
            },
        ],
        ..style(&blocks)
    };
    let page = renderer.layout_document(text, &options(), &doc).unwrap();
    assert!((page.caret_geometry(0).unwrap().x0 - 14.0).abs() < 1.0);
    assert!(
        page.width() < 300.0,
        "short tables use intrinsic column sizes"
    );
    let body = page.caret_geometry(17).unwrap();
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
    let pixel = ((body.y0 as usize + 1) * 400 + 2) * 4;
    assert_ne!(&raster.rgba[pixel..pixel + 4], &[255; 4]);
    assert_eq!(page.selected_text(0..text.len()).unwrap(), text);
}

#[test]
fn semantic_ranges_cannot_hide_text_or_split_unicode() {
    let mut renderer = TextRenderer::default();
    for blocks in [
        vec![block(0..1, DocumentKind::Paragraph)],
        vec![block(1..3, DocumentKind::Paragraph)],
        vec![block(0..4, DocumentKind::Heading(0))],
    ] {
        assert!(
            renderer
                .layout_document("文x", &options(), &style(&blocks))
                .is_err()
        );
    }
}

fn options() -> TextOptions<'static> {
    TextOptions {
        family: "sans-serif",
        size: 16.0,
        wrap_width: Some(400.0),
        line_height: Some(1.5),
        ..Default::default()
    }
}

fn style(blocks: &[DocumentBlock]) -> DocumentStyle<'_> {
    DocumentStyle {
        colors: &[],
        styles: &[],
        tables: &[],
        blocks,
        background: [255; 4],
        start_of_document: true,
    }
}
