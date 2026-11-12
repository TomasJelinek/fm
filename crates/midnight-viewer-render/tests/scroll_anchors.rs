use midnight_viewer_render::{
    CellAlignment, DocumentBlock, DocumentKind, DocumentStyle, TableCell, TableRow, TextOptions,
    TextRenderer,
};

#[test]
fn wrapped_and_tabbed_lines_have_ordered_original_utf8_boundaries() {
    let text = "first\t東京 long words wrap in a narrow viewport\nsecond café line\n";
    let mut renderer = TextRenderer::default();
    let page = renderer
        .layout_with_options(
            text,
            &TextOptions {
                wrap_width: Some(100.0),
                ..TextOptions::default()
            },
            &[],
        )
        .unwrap();
    let rows = page.scroll_anchors();
    assert!(rows.len() > 4, "wrapped visual rows must be represented");
    assert!(
        rows.iter()
            .all(|(at, y)| text.is_char_boundary(*at) && y.is_finite())
    );
    assert!(
        rows.windows(2)
            .all(|a| a[0].0 <= a[1].0 && a[0].1 <= a[1].1)
    );
    assert_eq!(rows.last(), Some(&(text.len(), page.height())));
    let wide = renderer
        .layout_with_options(
            text,
            &TextOptions {
                wrap_width: Some(500.0),
                ..TextOptions::default()
            },
            &[],
        )
        .unwrap();
    assert!(
        wide.scroll_anchors().len() < rows.len(),
        "width reflow rebuilds visual rows"
    );
}

#[test]
fn centered_uneven_table_cells_share_one_monotone_row_span() {
    let text = "short\tlong cell wraps through many lines at narrow width\nnext\trow";
    let split = text.find('\n').unwrap() + 1;
    let tab1 = text.find('\t').unwrap();
    let tab2 = split + text[split..].find('\t').unwrap();
    let rows = [
        TableRow {
            range: 0..split,
            header: true,
            cells: vec![
                TableCell {
                    range: 0..tab1,
                    alignment: CellAlignment::Left,
                },
                TableCell {
                    range: tab1 + 1..split - 1,
                    alignment: CellAlignment::Left,
                },
            ],
        },
        TableRow {
            range: split..text.len(),
            header: false,
            cells: vec![
                TableCell {
                    range: split..tab2,
                    alignment: CellAlignment::Left,
                },
                TableCell {
                    range: tab2 + 1..text.len(),
                    alignment: CellAlignment::Left,
                },
            ],
        },
    ];
    let blocks: Vec<_> = rows
        .iter()
        .map(|row| DocumentBlock {
            range: row.range.clone(),
            kind: DocumentKind::Paragraph,
            depth: 0,
        })
        .collect();
    let page = TextRenderer::default()
        .layout_document(
            text,
            &TextOptions {
                wrap_width: Some(150.0),
                ..TextOptions::default()
            },
            &DocumentStyle {
                colors: &[],
                styles: &[],
                tables: &rows,
                blocks: &blocks,
                background: [255; 4],
                start_of_document: true,
            },
        )
        .unwrap();
    let anchors = page.scroll_anchors();
    assert_eq!(
        anchors.iter().map(|(at, _)| *at).collect::<Vec<_>>(),
        vec![0, split, split, text.len(), text.len(), text.len()]
    );
    assert!(anchors.windows(2).all(|a| a[0].1 < a[1].1));
    assert!(
        page.caret_geometry(0).unwrap().y0 > page.caret_geometry(tab1 + 1).unwrap().y0,
        "fixture must exercise vertical centering, not equal-height cells"
    );
    let extended = format!("{text}\n\nfollowing paragraph");
    let mut extended_blocks = blocks.clone();
    extended_blocks.push(DocumentBlock {
        range: text.len() + 2..extended.len(),
        kind: DocumentKind::Paragraph,
        depth: 0,
    });
    let next = TextRenderer::default()
        .layout_document(
            &extended,
            &TextOptions {
                wrap_width: Some(150.0),
                ..TextOptions::default()
            },
            &DocumentStyle {
                colors: &[],
                styles: &[],
                tables: &rows,
                blocks: &extended_blocks,
                background: [255; 4],
                start_of_document: true,
            },
        )
        .unwrap();
    assert_eq!(
        &next.scroll_anchors()[..4],
        &anchors[..4],
        "adding following context must not change the last table row's source span"
    );
}
