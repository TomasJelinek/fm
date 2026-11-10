#[allow(dead_code)]
mod support;

use midnight_plugin_sdk::api::{abi, codec, document::*, messages::*, viewer::*};
use midnight_viewer_markdown::midnight_plugin_entry_v1 as entry;
use support::Host;

fn open(input: &str) -> Host {
    let mut host = Host::new(input.as_bytes(), entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(input.len() as u64),
            mode: MODE_DOCUMENT,
            display_name: "scroll.md".into(),
        },
    );
    assert!(
        host.finish()
            .iter()
            .any(|event| event.kind == abi::EVENT_OPENED)
    );
    host
}

fn viewport(host: &mut Host, anchor: u64, backwards: bool, lines: u32) -> DocumentPage {
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: anchor,
            backwards,
            max_bytes: 8192,
            max_lines: lines,
        },
    );
    let events = host.finish();
    codec::decode(
        &events
            .iter()
            .find(|event| event.kind == abi::EVENT_DOCUMENT_PAGE)
            .unwrap_or_else(|| panic!("document missing: {events:?}"))
            .payload,
    )
    .unwrap()
}

fn mixed_document() -> String {
    (0..25)
        .map(|index| {
            format!(
                "## Heading {index}\n\nParagraph **bold** and `code` café.\n\n\
                 - first item\n- second item\n\n```rust\nlet answer = {index};\n\
                 println!(\"answer\");\n```\n\n\
                 | Name | Value |\n| --- | ---: |\n| café | {index} |\n\n"
            )
        })
        .collect()
}

#[test]
fn visible_line_budget_counts_markdown_blocks_without_literal_newlines() {
    let input = (0..120)
        .map(|i| format!("## Heading {i}\n\n"))
        .collect::<String>();
    let mut host = open(&input);
    let page = viewport(&mut host, 0, false, 8);
    assert_eq!(
        page.blocks.len(),
        8,
        "each heading occupies a displayed line"
    );
    assert!(page.blocks.iter().all(|block| block.kind == BLOCK_HEADING));
    assert!(page.has_after);
}

#[test]
fn repeated_scroll_up_down_reuses_exact_semantic_pages_without_source_reads() {
    let input = mixed_document();
    let mut host = open(&input);
    let first = viewport(&mut host, 0, false, 12);
    let middle_anchor = input.find("## Heading 8").unwrap() as u64;
    let middle = viewport(&mut host, middle_anchor, false, 12);
    let previous = viewport(&mut host, middle_anchor, true, 12);
    let reads = host.reads().len();
    for _ in 0..6 {
        for (anchor, backwards, expected) in [
            (0, false, &first),
            (middle_anchor, false, &middle),
            (middle_anchor, true, &previous),
        ] {
            let actual = viewport(&mut host, anchor, backwards, 12);
            assert_eq!(
                codec::encode(&actual).unwrap(),
                codec::encode(expected).unwrap()
            );
        }
    }
    assert_eq!(
        host.reads().len(),
        reads,
        "cached scroll pages must not reread source"
    );
    assert!(first.blocks.iter().any(|block| block.kind == BLOCK_HEADING));
    assert!(first.blocks.iter().any(|block| block.kind == BLOCK_CODE));
    assert!(first.blocks.iter().any(|block| block.table.is_some()));
}

#[test]
fn many_later_blocks_do_not_turn_the_first_visible_heading_into_plain_text() {
    let input = (0..500)
        .map(|i| format!("## Heading {i}\n\n"))
        .collect::<String>();
    let mut host = open(&input);
    let page = viewport(&mut host, 0, false, 8);
    assert_eq!(page.blocks[0].kind, BLOCK_HEADING);
    assert_eq!(page.blocks[0].text.text, "Heading 0");
}

#[test]
fn evicted_pages_are_reconstructed_stably_and_recent_pages_stay_cached() {
    let input = mixed_document();
    let mut host = open(&input);
    let first = viewport(&mut host, 0, false, 12);
    let mut recent_anchor = 0;
    for index in 1..22 {
        recent_anchor = input.find(&format!("## Heading {index}\n")).unwrap() as u64;
        viewport(&mut host, recent_anchor, false, 12);
    }
    let reads = host.reads().len();
    let repeated = viewport(&mut host, 0, false, 12);
    assert_eq!(
        first, repeated,
        "reconstruction must preserve source bounds and styles"
    );
    assert_eq!(
        host.reads().len(),
        reads,
        "evicted page can be rebuilt from the retained bounded semantic context"
    );
    let reads = host.reads().len();
    viewport(&mut host, recent_anchor, false, 12);
    assert_eq!(
        host.reads().len(),
        reads,
        "the recent page should remain resident"
    );
}

#[test]
fn cached_pages_respect_new_viewport_budget_and_cancellation() {
    let input = mixed_document();
    let mut host = open(&input);
    let first = viewport(&mut host, 0, false, 4);
    let wider = viewport(&mut host, 0, false, 12);
    assert!(wider.source.length > first.source.length);
    let reads = host.reads().len();
    let id = host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: 0,
            backwards: false,
            max_bytes: 8192,
            max_lines: 12,
        },
    );
    host.cancel(id);
    let events = host.finish();
    assert!(
        events
            .iter()
            .all(|event| event.kind != abi::EVENT_DOCUMENT_PAGE)
    );
    assert_eq!(host.reads().len(), reads);
}

#[test]
fn close_and_reopen_never_reuses_a_previous_revision_page() {
    let input = mixed_document();
    let mut host = open(&input);
    assert_eq!(viewport(&mut host, 0, false, 8).revision, 3);
    host.submit_bytes(abi::REQUEST_CLOSE, &[]);
    host.finish();
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 4,
            length: Some(input.len() as u64),
            mode: MODE_DOCUMENT,
            display_name: "scroll.md".into(),
        },
    );
    host.finish();
    let reads = host.reads().len();
    assert_eq!(viewport(&mut host, 0, false, 8).revision, 4);
    assert!(host.reads().len() > reads);
}

#[test]
fn scrolling_inside_a_paragraph_keeps_its_complete_layout_context() {
    let paragraph = "A long paragraph with **emphasis** and enough words to wrap across several \
                     lines without changing their original alignment. The second sentence must \
                     not become a new first line when the viewer returns from the bottom.";
    let input = format!("# Report\n\n{paragraph}\n\n## Next section\n\nLast paragraph.\n");
    let mut host = open(&input);
    let full = viewport(&mut host, 0, false, 64);
    let expected = &full.blocks[1];
    let anchor = input.find("second sentence").unwrap() as u64;
    for backwards in [false, true] {
        let slice = viewport(&mut host, anchor, backwards, 2);
        assert!(
            slice.blocks.iter().any(|block| block == expected),
            "direction {backwards}: an interior anchor must not reflow a paragraph suffix"
        );
    }
}

#[test]
fn scrolling_inside_a_table_preserves_header_all_rows_and_column_measurements() {
    let input = "# Measurements\n\n| Workload | Time |\n| --- | ---: |\n\
                 | short | 1 |\n| the longest workload determines the column width | 100.25 |\n\
                 | last | 0.001 |\n\n## Following\n\nOther text.\n";
    let mut host = open(input);
    let full = viewport(&mut host, 0, false, 64);
    let expected: Vec<_> = full
        .blocks
        .iter()
        .filter(|block| block.table.is_some())
        .collect();
    for word in ["short", "longest", "last"] {
        for backwards in [false, true] {
            let slice = viewport(&mut host, input.find(word).unwrap() as u64, backwards, 1);
            let table: Vec<_> = slice
                .blocks
                .iter()
                .filter(|block| block.table.is_some())
                .collect();
            assert_eq!(table, expected, "{word}, backwards={backwards}");
        }
    }
}

#[test]
fn nearby_new_anchors_reuse_the_retained_semantic_context_without_source_reads() {
    let input = mixed_document();
    let mut host = open(&input);
    viewport(&mut host, 0, false, 12);
    let reads = host.reads().len();
    for index in (1..20).rev() {
        let at = input.find(&format!("Heading {index}\n")).unwrap() as u64;
        for backwards in [true, false] {
            let page = viewport(&mut host, at, backwards, 12);
            assert!(!page.blocks.is_empty());
        }
    }
    assert_eq!(
        host.reads().len(),
        reads,
        "retained context already contains these bytes"
    );
}

#[test]
fn zero_length_horizontal_rule_survives_initial_and_sequential_pages() {
    let mut host = open("---\n\nAfter\n");
    let page = viewport(&mut host, 0, false, 1);
    assert_eq!(page.blocks[0].kind, BLOCK_RULE);
}

#[test]
fn long_document_pagination_progresses_and_evicts_old_context() {
    let input = mixed_document().repeat(20);
    let mut host = open(&input);
    let first = viewport(&mut host, 0, false, 12);
    let mut page = first.clone();
    for _ in 0..2000 {
        if !page.has_after {
            break;
        }
        let end = page.source.end().unwrap();
        let next = viewport(&mut host, end, false, 12);
        assert!(
            next.source.end().unwrap() > end,
            "forward paging must progress"
        );
        page = next;
    }
    assert!(!page.has_after);
    let reads = host.reads().len();
    let actual = viewport(&mut host, 0, false, 12);
    assert_eq!(actual, first);
    assert!(
        host.reads().len() > reads,
        "old contexts must leave bounded storage"
    );
}

#[test]
fn backward_raw_window_boundary_does_not_publish_a_truncated_table_as_complete() {
    let prefix = "\n".repeat(65_200);
    let input = format!(
        "{prefix}## Table\n\n| Name | Count |\n| --- | ---: |\n| {} | 123 |\n| tail | 4 |\n",
        "long workload ".repeat(60)
    );
    let mut host = open(&input);
    let page = viewport(&mut host, 65_500, true, 40);
    let rows: Vec<_> = page
        .blocks
        .iter()
        .filter(|block| block.table.is_some())
        .collect();
    assert!(
        rows.is_empty() || rows.iter().any(|block| block.text.text.contains("tail")),
        "an unknown/truncated context must not claim to be the complete table"
    );
}

#[test]
fn a_full_budget_paragraph_cannot_trap_overlapping_forward_scroll() {
    let input = format!("{}\n\nFollowing paragraph.\n", "word ".repeat(1638) + "xy");
    let mut host = open(&input);
    let first = viewport(&mut host, 0, false, 40);
    let next = viewport(&mut host, 8100, false, 40);
    assert!(
        next.source.end().unwrap() > first.source.end().unwrap(),
        "an overlapping line near a full-budget block must reach following content"
    );
    assert!(
        next.blocks
            .iter()
            .any(|block| block.text.text.contains("Following"))
    );
}
