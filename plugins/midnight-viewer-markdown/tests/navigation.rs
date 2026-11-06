#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, document::*, messages::*, viewer::*};
use midnight_viewer_markdown::midnight_plugin_entry_v1 as entry;
use support::Host;

fn open(data: &[u8], mode: u16) -> Host {
    let mut host = Host::new(data, entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(data.len() as u64),
            mode,
            display_name: "test.md".into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
    host
}

fn viewport(host: &mut Host, anchor: u64, backwards: bool, max_bytes: u32) -> DocumentPage {
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: anchor,
            backwards,
            max_bytes,
            max_lines: 20,
        },
    );
    let events = host.finish();
    codec::decode(
        &events
            .iter()
            .find(|e| e.kind == abi::EVENT_DOCUMENT_PAGE)
            .unwrap_or_else(|| panic!("document page missing: {events:?}"))
            .payload,
    )
    .unwrap()
}

#[test]
fn initial_heading_preserves_raw_navigation_bounds() {
    let input = b"# Heading\n";
    let mut host = open(input, MODE_DOCUMENT);
    let page = viewport(&mut host, 0, false, 8192);
    assert_eq!(page.source.offset, 0);
    assert_eq!(page.source.end().unwrap(), 10);
    assert!(!page.has_before);
    assert!(!page.has_after);
    assert_eq!(page.blocks[0].text.text, "Heading");
    assert_eq!(page.blocks[0].text.anchors[0].source_offset, 2);
}

#[test]
fn initial_invisible_prefix_does_not_offer_an_empty_previous_page() {
    for (input, first) in [
        ("\n\n# Heading\n", 4),
        ("[ref]: https://example.org\n\n# Heading\n", 30),
        ("```\ncode\n```\n", 4),
    ] {
        let mut host = open(input.as_bytes(), MODE_DOCUMENT);
        let page = viewport(&mut host, 0, false, 8192);
        assert_eq!(page.source.offset, 0, "{input:?}");
        assert_eq!(page.source.end().unwrap(), input.len() as u64);
        assert!(!page.has_before, "{input:?}");
        assert!(!page.has_after);
        assert_eq!(page.blocks[0].text.anchors[0].source_offset, first);
    }
}

#[test]
fn initial_bom_document_does_not_offer_the_bom_as_previous_content() {
    for (input, bom, first) in [
        (&b"\xef\xbb\xbf# Heading\n"[..], 3, 5),
        (&b"\xff\xfe#\0 \0H\0i\0\n\0"[..], 2, 6),
        (&b"\xfe\xff\0#\0 \0H\0i\0\n"[..], 2, 6),
    ] {
        let mut host = open(input, MODE_DOCUMENT);
        for anchor in [0, bom] {
            let page = viewport(&mut host, anchor, false, 8192);
            assert_eq!(page.source.offset, bom);
            assert_eq!(page.source.end().unwrap(), input.len() as u64);
            assert!(!page.has_before);
            assert!(!page.has_after);
            assert_eq!(page.blocks[0].text.anchors[0].source_offset, first);
        }
    }
}

#[test]
fn backward_complete_document_preserves_consumed_prefix_and_suffix() {
    let input = b"\n# Heading\n\ntext **bold**\n\n";
    let mut host = open(input, MODE_DOCUMENT);
    let page = viewport(&mut host, input.len() as u64, true, 8192);
    assert_eq!(page.source.offset, 0);
    assert_eq!(page.source.end().unwrap(), input.len() as u64);
    assert!(!page.has_before);
    assert!(!page.has_after);
    assert_eq!(page.blocks[0].text.text, "Heading");
    assert_eq!(page.blocks[1].text.text, "text bold");
}

#[test]
fn page_up_to_initial_heading_keeps_raw_bounds_and_search_anchors() {
    let mut host = open(b"# Heading\n", MODE_DOCUMENT);
    let first = viewport(&mut host, 0, false, 3);
    assert_eq!(first.source.offset, 0);
    assert_eq!(first.source.end().unwrap(), 5);
    assert!(!first.has_before);
    assert!(first.has_after);
    assert_eq!(first.blocks[0].text.text, "Hea");
    let next = viewport(&mut host, 5, false, 3);
    assert!(next.has_before);
    assert_eq!(next.source.offset, 5);
    let previous = viewport(&mut host, next.source.offset, true, 3);
    assert_eq!(previous.source.offset, 0);
    assert_eq!(previous.source.end().unwrap(), 5);
    assert!(!previous.has_before);
    assert_eq!(previous.blocks[0].text.text, "Hea");
    host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: "ing".into(),
            source_anchor: 5,
            backwards: false,
            case_sensitive: true,
        },
    );
    let events = host.finish();
    let found: SearchMatch = codec::decode(
        &events
            .iter()
            .find(|e| e.kind == abi::EVENT_SEARCH_MATCH)
            .unwrap()
            .payload,
    )
    .unwrap();
    assert_eq!(found.source.offset, 6);
    assert_eq!(found.source.length, 3);
}

#[test]
fn initial_plain_bom_page_has_no_previous_content() {
    let mut host = open(b"\xef\xbb\xbfplain", MODE_TEXT);
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: 0,
            backwards: false,
            max_bytes: 8192,
            max_lines: 20,
        },
    );
    let events = host.finish();
    let page: TextPage = codec::decode(
        &events
            .iter()
            .find(|e| e.kind == abi::EVENT_TEXT_PAGE)
            .unwrap()
            .payload,
    )
    .unwrap();
    assert_eq!(page.source.offset, 3);
    assert_eq!(page.text, "plain");
    assert!(!page.has_before);
}

#[test]
fn backward_budget_cannot_mark_an_omitted_scalar_as_consumed() {
    let mut host = open("é".as_bytes(), MODE_DOCUMENT);
    let page = viewport(&mut host, 2, true, 1);
    assert!(page.blocks.is_empty());
    assert_eq!(page.source.offset, 2);
    assert_eq!(page.source.length, 0);
    assert!(page.has_before);
    assert!(!page.has_after);
}

#[test]
fn small_pages_preserve_visible_text_in_both_directions() {
    let input = "\n[ref]: https://example.org\n\n# Title\n\n**abc** é😀\n\nlast\n\n";
    let expected = "Titleabc é😀last";
    let mut host = open(input.as_bytes(), MODE_DOCUMENT);
    let mut anchor = 0;
    let mut text = String::new();
    for _ in 0..64 {
        let page = viewport(&mut host, anchor, false, 4);
        let next = page.source.end().unwrap();
        text.extend(page.blocks.iter().map(|b| b.text.text.as_str()));
        if !page.has_after {
            assert_eq!(next, input.len() as u64);
            break;
        }
        assert!(next > anchor, "forward navigation stalled: {page:?}");
        anchor = next;
    }
    assert_eq!(text, expected);
    let mut anchor = input.len() as u64;
    let mut pieces = Vec::new();
    for _ in 0..64 {
        let page = viewport(&mut host, anchor, true, 4);
        let next = page.source.offset;
        pieces.push(
            page.blocks
                .iter()
                .map(|b| b.text.text.as_str())
                .collect::<String>(),
        );
        if !page.has_before {
            assert_eq!(next, 0);
            break;
        }
        assert!(next < anchor, "backward navigation stalled: {page:?}");
        anchor = next;
    }
    assert_eq!(pieces.into_iter().rev().collect::<String>(), expected);
}

#[test]
fn production_page_budget_does_not_skip_a_scalar_before_later_blocks() {
    let input = format!("{}😀\n\nlast", "x".repeat(8190));
    let mut host = open(input.as_bytes(), MODE_DOCUMENT);
    let first = viewport(&mut host, 0, false, 8192);
    assert_eq!(first.source.end().unwrap(), 8190);
    assert_eq!(first.blocks[0].text.text, "x".repeat(8190));
    assert!(first.has_after);
    let next = viewport(&mut host, 8190, false, 8192);
    assert_eq!(next.blocks[0].text.text, "😀");
    assert_eq!(next.blocks[0].text.source.offset, 8190);
    assert_eq!(next.blocks[0].text.source.length, 4);
    assert_eq!(next.blocks[1].text.text, "last");
    assert!(!next.has_after);
}

fn assert_search(input: &str, query: &str, offset: u64, length: u64) {
    let mut host = open(input.as_bytes(), MODE_DOCUMENT);
    host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: query.into(),
            source_anchor: 0,
            backwards: false,
            case_sensitive: true,
        },
    );
    let events = host.finish();
    let found: SearchMatch = codec::decode(
        &events
            .iter()
            .find(|e| e.kind == abi::EVENT_SEARCH_MATCH)
            .unwrap_or_else(|| panic!("missing visible match: {events:?}"))
            .payload,
    )
    .unwrap();
    assert_eq!(found.source.offset, offset);
    assert_eq!(found.source.length, length);
}

#[test]
fn production_search_budget_does_not_skip_a_scalar_before_later_blocks() {
    let input = format!("{}😀\n\nlast", "x".repeat(8190));
    assert_search(&input, "😀", 8190, 4);
}

#[test]
fn search_advances_over_an_invisible_full_raw_window() {
    let input = format!("{}# needle\n", "\n".repeat(40000));
    assert_search(&input, "needle", 40002, 6);
}
