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

fn targets(page: &DocumentPage) -> Vec<&str> {
    page.blocks
        .iter()
        .flat_map(|block| block.links.iter())
        .map(|link| link.target.as_str())
        .collect()
}

/// The host's link policy decides what a target does; the plugin keeps scripts, HTML and
/// images inert and passes every link target the protocol can carry.
#[test]
fn hostile_html_scripts_and_images_are_inert_and_every_link_target_reaches_the_host() {
    let page = page(
        "<script>alert(1)</script>\n\n![cat](https://remote/image.png) [bad](javascript:alert) [data](data:text/html,x) [ok](https://example.org) [file](file:///secret)\n",
    );
    assert!(page.blocks.iter().any(|b| b.text.text.contains("<script>")));
    assert_eq!(
        targets(&page),
        [
            "javascript:alert",
            "data:text/html,x",
            "https://example.org",
            "file:///secret"
        ],
        "the image is not a link"
    );
    assert!(page.blocks.iter().any(|b| b.text.text.contains("cat")));
}

#[test]
fn relative_file_and_other_link_targets_reach_the_host_unchanged() {
    let page = page(
        "[next](other.md) [up](../README.md#install) [file](file:///C:/x.md) [spaced](<my notes.md>) [mail](mailto:a@example.org) [ftp](ftp://example.org/x)\n",
    );
    assert_eq!(
        targets(&page),
        [
            "other.md",
            "../README.md#install",
            "file:///C:/x.md",
            "my notes.md",
            "mailto:a@example.org",
            "ftp://example.org/x"
        ]
    );
}

#[test]
fn a_target_the_protocol_cannot_carry_drops_only_that_link() {
    let input = format!(
        "[ok](a.md) [long]({}) [control](a&#1;b) **kept**\n",
        "a".repeat(8193)
    );
    let decoded = decode(
        input.as_bytes(),
        100,
        Encoding::Utf8,
        true,
        65536,
        100,
        || false,
    )
    .unwrap();
    let page = render(&decoded, 3, true, || false).unwrap();
    page.validate().unwrap();
    assert_eq!(targets(&page), ["a.md"]);
    assert!(
        page.blocks[0]
            .text
            .spans
            .iter()
            .any(|span| span.flags & 1 != 0),
        "the other links and styles stay"
    );
}

#[test]
fn a_link_that_starts_an_ordered_item_covers_its_text_not_the_number() {
    let page = page("1. [first](a.md) item\n2. [second](b.md)\n\n3) [third](c.md)\n");
    let covered: Vec<_> = page
        .blocks
        .iter()
        .flat_map(|block| {
            block
                .links
                .iter()
                .map(|link| &block.text.text[link.start as usize..link.end as usize])
        })
        .collect();
    assert_eq!(covered, ["first", "second", "third"]);
    assert!(page.blocks[0].text.text.starts_with("1. first"));
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
    assert_eq!(page.blocks.len(), 1);
    assert!(
        page.blocks[0].text.text.ends_with("> text"),
        "literal fallback"
    );
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
fn ordered_lists_keep_their_numbers_and_loose_items_have_one_marker() {
    let ordered = page("7. Download\n8. Unpack\n9) wrong delimiter starts a new list\n");
    let texts: Vec<_> = ordered
        .blocks
        .iter()
        .map(|b| b.text.text.as_str())
        .collect();
    assert_eq!(
        texts,
        [
            "7. Download",
            "8. Unpack",
            "9) wrong delimiter starts a new list"
        ]
    );
    assert!(
        ordered
            .blocks
            .iter()
            .all(|b| b.kind == BLOCK_PARAGRAPH && b.depth == 1),
        "numbers replace the host bullet"
    );
    // The number maps to its source marker; the text keeps exact anchors.
    let first = &ordered.blocks[0].text;
    assert_eq!(first.anchors[0].source_offset, 100);
    assert_eq!(first.anchors[1].text_offset, 3);
    assert_eq!(first.anchors[1].source_offset, 102);
    let download = first.anchors.iter().find(|a| a.text_offset == 4).unwrap();
    assert_eq!(download.source_offset, 104);

    let loose = page(
        "1. First paragraph\n\n   Second paragraph\n\n2. Next\n\n- bullet\n\n  more of the bullet\n",
    );
    let blocks: Vec<_> = loose
        .blocks
        .iter()
        .map(|b| (b.kind, b.depth, b.text.text.as_str()))
        .collect();
    assert_eq!(
        blocks,
        [
            (BLOCK_PARAGRAPH, 1, "1. First paragraph"),
            (BLOCK_PARAGRAPH, 1, "Second paragraph"),
            (BLOCK_PARAGRAPH, 1, "2. Next"),
            (BLOCK_LIST_ITEM, 1, "bullet"),
            (BLOCK_PARAGRAPH, 1, "more of the bullet"),
        ]
    );
    let tasks = page("1. [x] done\n2. [ ] open\n");
    assert_eq!(tasks.blocks[0].text.text, "1. [x] done");
    assert_eq!(tasks.blocks[1].text.text, "2. [ ] open");
}

#[test]
fn character_references_and_bidi_overrides_cannot_inject_controls() {
    let page = page(
        "Run &#x202E;exe.dab&#x202C; then &#27;]52;c;x&#7; or \u{202E}raw\u{202C}, \
         &#x9B;&#127;&#13;! [x](https://a.example/&#x202E;) \u{200F}rtl\n",
    );
    let text: String = page.blocks.iter().map(|b| b.text.text.as_str()).collect();
    assert!(
        !text
            .chars()
            .any(|c| c.is_control() || ('\u{202a}'..='\u{202e}').contains(&c)),
        "{text:?}"
    );
    for visible in ['\u{241b}', '\u{2407}', '\u{2421}', '\u{240d}', '\u{fffd}'] {
        assert!(text.contains(visible), "{visible:?} in {text:?}");
    }
    assert!(text.contains("exe.dab"), "logical order stays visible");
    assert!(text.contains('\u{200f}'), "implicit direction marks remain");
    // The link keeps its target. The host's link policy shows it with the bidirectional
    // control escaped and hands the address over percent-encoded.
    assert_eq!(targets(&page), ["https://a.example/\u{202e}"]);
}

#[test]
#[ignore = "manual timing: cargo test --test markdown -- --ignored --nocapture"]
fn render_throughput() {
    let input: String = (0..200)
        .map(|index| {
            format!(
                "## Part {index}\n\nSome **styled** prose with `code` and a \
                 [link](https://example.org/{index}) {}\n\n",
                "words ".repeat(40)
            )
        })
        .collect();
    let input = &input[..input.floor_char_boundary(65536)];
    let decoded = decode(
        input.as_bytes(),
        0,
        Encoding::Utf8,
        false,
        65536,
        usize::MAX,
        || false,
    )
    .unwrap();
    // The minimum and median resist unrelated load on shared machines.
    let mut times: Vec<u128> = (0..60)
        .map(|_| {
            let started = std::time::Instant::now();
            assert!(render(&decoded, 3, false, || false).is_some());
            started.elapsed().as_micros()
        })
        .collect();
    times.sort_unstable();
    println!(
        "markdown_render bytes={} min_us={} median_us={}",
        decoded.text.len(),
        times[0],
        times[times.len() / 2]
    );
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

#[test]
fn an_over_budget_unit_is_literal_without_downgrading_its_neighbours() {
    let row = std::iter::repeat_n(" v ", 65).collect::<Vec<_>>().join("|");
    let delimiter = std::iter::repeat_n("---", 65).collect::<Vec<_>>().join("|");
    let nested = format!("{}deep", "> ".repeat(20));
    let input = format!(
        "# Title\n\n|{row}|\n|{delimiter}|\n|{row}|\n\nMiddle **bold**\n\n{nested}\n\nAfter `code`\n"
    );
    let page = page(&input);
    let kinds: Vec<_> = page.blocks.iter().map(|b| b.kind).collect();
    assert_eq!(
        kinds,
        [
            BLOCK_HEADING,
            BLOCK_PARAGRAPH,
            BLOCK_PARAGRAPH,
            BLOCK_PARAGRAPH,
            BLOCK_PARAGRAPH
        ]
    );
    assert_eq!(page.blocks[0].text.text, "Title");
    assert!(
        page.blocks[1].text.text.starts_with("| v |"),
        "literal table"
    );
    assert!(
        page.blocks[1].text.text.ends_with("| v |"),
        "no trailing break"
    );
    assert_eq!(page.blocks[2].text.text, "Middle bold");
    assert!(page.blocks[2].text.spans.iter().any(|s| s.flags & 1 != 0));
    assert_eq!(page.blocks[3].text.text, nested, "literal nesting");
    assert_eq!(page.blocks[4].text.text, "After code");
    // Literal text keeps one exact anchor per source scalar.
    let literal = &page.blocks[1].text;
    assert_eq!(literal.anchors.len(), literal.text.chars().count() + 1);
    assert_eq!(
        literal.anchors[0].source_offset,
        100 + input.find("|").unwrap() as u64
    );
}
