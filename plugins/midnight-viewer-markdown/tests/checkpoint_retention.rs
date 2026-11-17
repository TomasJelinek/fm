//! Proven checkpoints survive long reads and searches, so revisited regions
//! keep their semantic rendering instead of showing literal Markdown.
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
            display_name: "readme.md".into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
    host
}

fn page(host: &mut Host, anchor: u64, backwards: bool, lines: u32) -> DocumentPage {
    let request = host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: anchor,
            backwards,
            max_bytes: 65536,
            max_lines: lines,
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

/// About 430 KB in 2,600 top-level blocks: more checkpoints than a session
/// retains (256), even at the 1 KiB recording spacing. CJK prose keeps each
/// page large (three source bytes per displayed anchor), so few reads suffice.
fn readme() -> String {
    (0..1300)
        .map(|index| {
            format!(
                "## Section {index}\n\n段落{index}有**粗体**和`代码`以及{}。\n\n",
                "中文".repeat(45)
            )
        })
        .collect()
}

fn assert_semantic(page: &DocumentPage, at: u64) {
    assert!(!page.blocks.is_empty(), "page at {at} is empty");
    for block in &page.blocks {
        assert!(
            !block.text.text.contains("**") && !block.text.text.contains("## "),
            "page at {at} fell back to literal Markdown: {:?}",
            &block.text.text[..block.text.text.len().min(40)]
        );
    }
    assert!(
        page.blocks.iter().any(|block| block.kind == BLOCK_HEADING),
        "page at {at} lost its headings"
    );
}

#[test]
fn scrolling_back_through_a_read_document_stays_semantic() {
    let input = readme();
    let mut host = open(&input);
    let mut anchor = 0;
    for _ in 0..500 {
        let page = page(&mut host, anchor, false, 60);
        assert_semantic(&page, anchor);
        anchor = page.source.end().unwrap();
        if !page.has_after {
            break;
        }
    }
    assert_eq!(anchor, input.len() as u64);
    for _ in 0..500 {
        let page = page(&mut host, anchor, true, 60);
        assert_semantic(&page, anchor);
        assert!(page.source.offset < anchor);
        anchor = page.source.offset;
        if !page.has_before {
            break;
        }
    }
    assert_eq!(anchor, 0, "scrolled back to the beginning");
}

#[test]
fn a_search_to_the_end_keeps_the_viewport_context() {
    let input = readme();
    let mut host = open(&input);
    let position = input.find("## Section 200").unwrap() as u64;
    // Read sequentially up to the position, as a user scrolling there would.
    let mut anchor = 0;
    while anchor <= position {
        let page = page(&mut host, anchor, false, 60);
        assert_semantic(&page, anchor);
        anchor = page.source.end().unwrap();
    }
    let request = host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: "absent needle".into(),
            source_anchor: position,
            backwards: false,
            case_sensitive: true,
        },
    );
    let events = host.finish();
    assert!(
        events
            .iter()
            .any(|e| e.id == request && e.kind == abi::EVENT_COMPLETED)
    );
    // Different budgets bypass the page cache and the retained context.
    let before = page(&mut host, position, true, 23);
    assert_semantic(&before, position);
    let after = page(&mut host, position + 3000, false, 17);
    assert_semantic(&after, position + 3000);
}
