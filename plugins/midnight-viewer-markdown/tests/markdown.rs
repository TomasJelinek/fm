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
