//! Document pages at the host's production request budget (64 KiB, ~60 lines).
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
            display_name: "article.md".into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
    host
}

fn page(host: &mut Host, anchor: u64, backwards: bool) -> DocumentPage {
    let request = host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: anchor,
            backwards,
            max_bytes: 65536,
            max_lines: 60,
        },
    );
    let events = host.finish();
    codec::decode(
        &events
            .iter()
            .find(|e| e.id == request && e.kind == abi::EVENT_DOCUMENT_PAGE)
            .unwrap_or_else(|| panic!("document missing: {events:?}"))
            .payload,
    )
    .unwrap()
}

fn styled_article() -> String {
    let mut input = String::from("# Title\n\n| Name | Value |\n| --- | ---: |\n| café | 1 |\n\n");
    for index in 0..80 {
        input.push_str(&format!(
            "Paragraph {index:03} has **bold words** and `inline code` with a \
             [link](https://example.org/{index}) {}end\n\n",
            "lorem ipsum ".repeat(22)
        ));
    }
    input
}

fn assert_styled(page: &DocumentPage) {
    let paragraphs: Vec<_> = page
        .blocks
        .iter()
        .filter(|block| block.text.text.starts_with("Paragraph"))
        .collect();
    assert!(
        paragraphs.len() >= 30,
        "complete semantic paragraphs, not an 8 KiB scalar fragment: {}",
        paragraphs.len()
    );
    for block in &paragraphs {
        assert!(block.text.text.ends_with("end"), "complete paragraph");
        assert!(
            block.text.spans.iter().any(|span| span.role == STYLE_CODE),
            "code style survives"
        );
        assert!(
            block.text.spans.iter().any(|span| span.flags & 1 != 0),
            "strong style survives"
        );
        assert_eq!(block.links.len(), 1, "link metadata survives");
    }
    assert!(page.blocks.iter().map(|b| b.text.text.len()).sum::<usize>() > 8192);
    assert!(codec::encode(page).is_ok());
}

#[test]
fn sequential_pages_never_present_a_paragraph_cut_by_the_read_window() {
    // CJK prose: few anchors per byte, so a page reaches the end of its
    // 64 KiB read before the line or wire budget stops it.
    let input: String = (0..150)
        .map(|index| format!("{}終{index:04}\n\n", "中".repeat(290)))
        .collect();
    let mut host = open(&input);
    let mut anchor = 0;
    let mut next = 0;
    for _ in 0..200 {
        let page = page(&mut host, anchor, false);
        assert!(page.source.offset <= anchor);
        for block in &page.blocks {
            assert!(
                block.text.text.ends_with(&format!("終{next:04}")),
                "page at {anchor} shows paragraph {next} incompletely or out of order: {:?}",
                &block.text.text[block.text.text.len().saturating_sub(12)..]
            );
            next += 1;
        }
        anchor = page.source.end().unwrap();
        if !page.has_after {
            break;
        }
    }
    assert_eq!(next, 150);
    assert_eq!(anchor, input.len() as u64);
}

#[test]
fn an_over_budget_table_stays_literal_alone_and_later_sections_stay_semantic() {
    // About 20,000 parser events in one table, more than one unit may build.
    let mut input = String::from("# Before\n\nIntro **bold**.\n\n| a | b | c | d | e |\n");
    input.push_str("| - | - | - | - | - |\n");
    for row in 0..1200 {
        input.push_str(&format!("| r{row} | x | y | z | w |\n"));
    }
    let table_end = input.len() as u64;
    for index in 0..60 {
        input.push_str(&format!(
            "\n## After {index}\n\nParagraph {index} with **bold** text.\n"
        ));
    }
    let mut host = open(&input);
    let first = page(&mut host, 0, false);
    assert_eq!(first.blocks[0].kind, BLOCK_HEADING);
    assert_eq!(first.blocks[0].text.text, "Before");
    assert_eq!(first.blocks[1].text.text, "Intro bold.");
    let mut anchor = first.source.end().unwrap();
    let mut after = 0;
    for _ in 0..100 {
        let page = page(&mut host, anchor, false);
        for block in &page.blocks {
            if block.text.source.offset >= table_end {
                assert!(
                    !block.text.text.contains("**") && !block.text.text.contains("## "),
                    "content after the table stays semantic: {:?}",
                    block.text.text
                );
                after += 1;
            }
        }
        anchor = page.source.end().unwrap();
        if !page.has_after {
            break;
        }
    }
    assert_eq!(anchor, input.len() as u64);
    assert_eq!(after, 120, "every later heading and paragraph");
}

#[test]
fn a_fenced_log_longer_than_a_read_does_not_end_semantic_rendering() {
    let mut input = String::from("# Intro\n\nText **bold**.\n\n```text\n");
    for line in 0..5000 {
        input.push_str(&format!("log line {line:05} with some content\n"));
    }
    input.push_str("```\n");
    let fence_end = input.len() as u64;
    for index in 0..300 {
        input.push_str(&format!(
            "\n## After {index}\n\nParagraph {index} with **bold** text.\n"
        ));
    }
    let mut host = open(&input);
    let mut anchor = 0;
    let mut semantic = 0;
    for _ in 0..500 {
        let page = page(&mut host, anchor, false);
        for block in &page.blocks {
            if block.text.source.offset >= fence_end {
                assert!(
                    !block.text.text.contains("**") && !block.text.text.contains("## "),
                    "content after the fence stays semantic: {:?}",
                    block.text.text
                );
                semantic += 1;
            }
        }
        anchor = page.source.end().unwrap();
        if !page.has_after {
            break;
        }
    }
    assert_eq!(anchor, input.len() as u64);
    assert!(
        semantic >= 500,
        "{semantic} semantic blocks after the fence"
    );
    // Search continues through the fence and matches displayed text after it.
    let request = host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: "Paragraph 250 with bold text".into(),
            source_anchor: 0,
            backwards: false,
            case_sensitive: true,
        },
    );
    let events = host.finish();
    let found: SearchMatch = codec::decode(
        &events
            .iter()
            .find(|e| e.id == request && e.kind == abi::EVENT_SEARCH_MATCH)
            .expect("displayed text after the fence")
            .payload,
    )
    .unwrap();
    assert_eq!(
        found.source.offset,
        input.find("Paragraph 250 ").unwrap() as u64
    );
}

#[test]
fn ordinary_styled_prose_keeps_styles_links_and_tables_at_production_budget() {
    let input = styled_article();
    let mut host = open(&input);
    let first = page(&mut host, 0, false);
    assert_eq!(first.blocks[0].kind, BLOCK_HEADING);
    assert!(
        first.blocks.iter().any(|block| block.table.is_some()),
        "table geometry survives"
    );
    assert_styled(&first);
    assert!(first.has_after);
    let next = page(&mut host, first.source.end().unwrap(), false);
    assert_eq!(next.source.offset, first.source.end().unwrap());
    assert_styled(&next);
    let last = page(&mut host, input.len() as u64, true);
    assert_styled(&last);
    assert!(!last.has_after && last.has_before);
}
