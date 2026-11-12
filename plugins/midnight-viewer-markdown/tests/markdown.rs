use midnight_plugin_sdk::api::{codec::Model, document::*};
use midnight_viewer_markdown::{
    decode::{Encoding, decode},
    markdown::render,
};

fn page(input: &str) -> DocumentPage {
    let decoded = decode(
        input.as_bytes(),
        100,
        Encoding::Utf8,
        true,
        8192,
        100,
        || false,
    )
    .unwrap();
    let page = render(&decoded, 3, true, || false).unwrap();
    page.validate().unwrap();
    page
}

#[test]
fn ordinary_source_newlines_are_soft_wraps_but_explicit_breaks_stay_visible() {
    let page = page("First source line\ncontinues the same paragraph.  \nNew displayed line.\n");
    assert_eq!(
        page.blocks[0].text.text,
        "First source line continues the same paragraph.\nNew displayed line."
    );
}

#[test]
fn semantic_blocks_styles_and_exact_unicode_source_anchors() {
    let page = page(
        "# Café\n\nHello **world** and [web](https://example.org).\n\n> quote\n\n- item\n\n```rs\nlet x = 1;\n```\n",
    );
    assert_eq!(page.blocks[0].kind, BLOCK_HEADING);
    assert_eq!(page.blocks[0].heading_level, 1);
    assert_eq!(page.blocks[0].text.text, "Café");
    assert_eq!(page.blocks[0].text.anchors[0].source_offset, 102);
    assert_eq!(
        page.blocks[0].text.anchors.last().unwrap().source_offset,
        107
    );
    assert!(page.blocks.iter().any(|b| b.kind == BLOCK_QUOTE));
    assert!(page.blocks.iter().any(|b| b.kind == BLOCK_LIST_ITEM));
    assert!(
        page.blocks
            .iter()
            .any(|b| b.kind == BLOCK_CODE && b.text.text.contains("let x"))
    );
    let paragraph = &page.blocks[1];
    assert_eq!(paragraph.text.text, "Hello world and web.");
    assert!(paragraph.text.spans.iter().any(|s| s.flags & 1 != 0));
    assert_eq!(paragraph.links[0].target, "https://example.org");
    assert_eq!(
        &paragraph.text.text[paragraph.links[0].start as usize..paragraph.links[0].end as usize],
        "web"
    );
}

#[test]
fn hostile_html_scripts_images_and_unsafe_urls_are_inert() {
    let page = page(
        "<script>alert(1)</script>\n\n![cat](https://remote/image.png) [bad](javascript:alert) [data](data:text/html,x) [ok](https://example.org) [file](file:///secret)\n",
    );
    assert!(page.blocks.iter().any(|b| b.text.text.contains("<script>")));
    let links: Vec<_> = page.blocks.iter().flat_map(|b| b.links.iter()).collect();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].target, "https://example.org");
    assert!(page.blocks.iter().any(|b| b.text.text.contains("cat")));
}

#[test]
fn entity_escapes_code_and_malformed_input_have_valid_maps() {
    let page = page("a &amp; b \\*literal\\* `x & y` [broken\n");
    assert_eq!(page.blocks[0].text.text, "a & b *literal* x & y [broken");
    let text = &page.blocks[0].text;
    let a = text.anchors.iter().find(|a| a.text_offset == 3).unwrap();
    assert_eq!(a.source_offset, 107);
}

#[test]
fn nesting_and_cancellation_are_bounded() {
    let input = format!("{}text", "> ".repeat(100));
    let page = page(&input);
    assert!(page.blocks.iter().all(|b| b.depth <= 16));
    let decoded = decode(b"hello", 0, Encoding::Utf8, true, 8192, 10, || false).unwrap();
    assert!(render(&decoded, 1, true, || true).is_none());
}

#[test]
fn github_tables_and_task_lists_are_rendered_instead_of_showing_markup() {
    let table = page("| Name | Value |\n| :--- | ---: |\n| **Café** | 42 |\n| Empty | |\n");
    let rows: Vec<_> = table
        .blocks
        .iter()
        .map(|block| block.text.text.as_str())
        .collect();
    assert_eq!(rows, ["Name\tValue", "Café\t42", "Empty\t"]);
    assert!(
        table.blocks[1]
            .text
            .spans
            .iter()
            .any(|span| span.flags & 1 != 0)
    );
    let tasks = page("- [x] Done\n- [ ] Pending\n\n~~obsolete~~ and ordinary\n");
    assert!(
        tasks
            .blocks
            .iter()
            .any(|block| block.text.text.contains("[x] Done"))
    );
    assert!(
        tasks
            .blocks
            .iter()
            .any(|block| block.text.text.contains("[ ] Pending"))
    );
    assert!(
        tasks
            .blocks
            .iter()
            .any(|block| block.text.text == "obsolete and ordinary")
    );
}

#[test]
fn table_cells_retain_alignment_inline_styles_and_exact_unicode_maps() {
    let page = page(
        "| Název | Center | Value |\n| :--- | :---: | ---: |\n| a\\|b | **字** | `42` |\n| | | |\n",
    );
    assert_eq!(page.blocks.len(), 3);
    for (index, block) in page.blocks.iter().enumerate() {
        let table = block.table.as_ref().unwrap();
        assert_eq!(table.header, index == 0);
        assert_eq!(table.cells.len(), 3);
        assert_eq!(
            table
                .cells
                .iter()
                .map(|cell| cell.alignment)
                .collect::<Vec<_>>(),
            [TABLE_ALIGN_LEFT, TABLE_ALIGN_CENTER, TABLE_ALIGN_RIGHT]
        );
    }
    let row = &page.blocks[1];
    assert_eq!(row.text.text, "a|b\t字\t42");
    assert_eq!(
        row.table
            .as_ref()
            .unwrap()
            .cells
            .iter()
            .map(|cell| { &row.text.text[cell.start as usize..cell.end as usize] })
            .collect::<Vec<_>>(),
        ["a|b", "字", "42"]
    );
    assert!(
        row.text
            .spans
            .iter()
            .any(|span| span.role == midnight_plugin_sdk::api::viewer::STYLE_CODE)
    );
    assert_eq!(page.blocks[2].text.text, "\t\t");
}

#[test]
fn table_fragments_and_strikethrough_keep_valid_ranges_in_both_directions() {
    use midnight_viewer_markdown::markdown::{render_backward, render_window};
    let source = "| Name | Value |\n| --- | ---: |\n| ~~Café~~ | **字** |\n| empty | |\n\nAfter\n";
    let decoded = decode(
        source.as_bytes(),
        100,
        Encoding::Utf8,
        true,
        8192,
        100,
        || false,
    )
    .unwrap();
    let full = render(&decoded, 3, true, || false).unwrap();
    let row = &full.blocks[1];
    let strike = &row.strikethrough.as_ref().unwrap()[0];
    assert_eq!(
        &row.text.text[strike.start as usize..strike.end as usize],
        "Café"
    );
    for anchor in decoded.anchors.iter().map(|anchor| anchor.source_offset) {
        for budget in [1, 4, 12, 8192] {
            render_window(&decoded, 3, true, anchor, budget, 20, || false)
                .unwrap()
                .validate()
                .unwrap();
            render_backward(&decoded, 3, true, anchor, budget, 20, || false)
                .unwrap()
                .validate()
                .unwrap();
        }
    }
}

#[test]
fn excessive_columns_fall_back_to_bounded_readable_text() {
    let row = std::iter::repeat_n(" value ", 65)
        .collect::<Vec<_>>()
        .join("|");
    let delimiter = std::iter::repeat_n(" --- ", 65)
        .collect::<Vec<_>>()
        .join("|");
    let input = format!("|{row}|\n|{delimiter}|\n|{row}|\n");
    let page = page(&input);
    assert!(page.blocks.iter().all(|block| block.table.is_none()));
    assert!(page.blocks[0].text.text.starts_with("| value |"));
}
