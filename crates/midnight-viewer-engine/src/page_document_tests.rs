use super::*;

#[test]
fn semantic_tables_and_overlapping_inline_decorations_keep_source_anchors() {
    let appearance = ViewerAppearance {
        family: "sans-serif".into(),
        font_size: 18.0,
        foreground: [220; 4],
        background: [0; 4],
        selection: [80; 4],
        accent: [180; 4],
    };
    let block = document::Block {
        kind: document::BLOCK_PARAGRAPH,
        depth: 0,
        heading_level: 0,
        links: vec![],
        text: viewer::TextPage {
            revision: 3,
            source: viewer::SourceRange {
                offset: 1 << 35,
                length: 8,
            },
            text: "文\tcode".into(),
            spans: vec![viewer::Span {
                start: 4,
                end: 8,
                role: viewer::STYLE_CODE,
                flags: 1,
            }],
            anchors: vec![
                viewer::Anchor {
                    text_offset: 0,
                    source_offset: 1 << 35,
                },
                viewer::Anchor {
                    text_offset: 4,
                    source_offset: (1 << 35) + 4,
                },
                viewer::Anchor {
                    text_offset: 8,
                    source_offset: (1 << 35) + 8,
                },
            ],
            has_before: true,
            has_after: true,
            partial_line: false,
        },
        table: Some(document::TableRow {
            header: true,
            cells: vec![
                document::TableCell {
                    start: 0,
                    end: 3,
                    alignment: document::TABLE_ALIGN_LEFT,
                },
                document::TableCell {
                    start: 4,
                    end: 8,
                    alignment: document::TABLE_ALIGN_RIGHT,
                },
            ],
        }),
        strikethrough: Some(vec![document::TextRange { start: 5, end: 7 }]),
    };
    let source = block.text.source.clone();
    let page = Page::document(
        document::DocumentPage {
            revision: 3,
            source,
            blocks: vec![block],
            has_before: true,
            has_after: true,
        },
        &appearance,
    );
    let shaped = midnight_viewer_render::TextRenderer::default()
        .layout_with_tables(
            &page.text,
            &midnight_viewer_render::TextOptions {
                family: "sans-serif",
                wrap_width: Some(360.0),
                ..Default::default()
            },
            &page.spans,
            &page.styles,
            &page.tables,
        )
        .unwrap();
    assert_eq!(shaped.selected_text(0..8).unwrap(), "文\tcode");
    let source_anchor = page
        .anchors
        .iter()
        .find(|anchor| anchor.source_offset == (1 << 35) + 4)
        .unwrap();
    assert!(
        shaped
            .caret_geometry(source_anchor.text_offset as usize)
            .unwrap()
            .x0
            > 240.0
    );
    let struck = page
        .styles
        .iter()
        .find(|style| style.range == (5..7))
        .unwrap();
    assert!(struck.strikethrough && struck.monospace);
    assert_eq!(
        struck.weight, 700,
        "strikethrough cannot erase bold code styling"
    );
    assert!(page.before && page.after);
}
