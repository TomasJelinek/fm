#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, document::*, messages::*, viewer::*};
use midnight_viewer_markdown::midnight_plugin_entry_v1 as entry;
use support::Host;

fn open(host: &mut Host, length: u64) {
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            length: Some(length),
            revision: 3,
            mode: MODE_DOCUMENT,
            display_name: "test.md".into(),
        },
    );
    let events = host.finish();
    assert!(
        events.iter().any(|e| e.kind == abi::EVENT_OPENED),
        "{events:?}"
    );
}

#[test]
fn open_viewport_displayed_text_search_and_close() {
    let input = b"# Heading\n\nhello **beautiful** world &amp; friends\n";
    let mut host = Host::new(input, entry);
    host.short_reads(7);
    open(&mut host, input.len() as u64);
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
    let page: DocumentPage = codec::decode(
        &events
            .iter()
            .find(|e| e.kind == abi::EVENT_DOCUMENT_PAGE)
            .expect("page")
            .payload,
    )
    .unwrap();
    assert_eq!(page.blocks[0].kind, BLOCK_HEADING);
    host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: "beautiful world & friends".into(),
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
            .expect("displayed match")
            .payload,
    )
    .unwrap();
    assert!(found.preview.contains("beautiful world & friends"));
    assert!(found.source.offset <= 19 && found.source.end().unwrap() >= 47);
    host.submit_bytes(abi::REQUEST_CLOSE, &[]);
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_COMPLETED));
    assert!(host.reads().iter().all(|r| r.length <= 65536));
}

#[test]
fn cancelled_delayed_document_and_u64_viewport() {
    let mut host = Host::new(b"# test", entry);
    open(&mut host, 1_u64 << 35);
    host.base(1_u64 << 34);
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: 1_u64 << 34,
            backwards: false,
            max_bytes: 128,
            max_lines: 20,
        },
    );
    let events = host.finish();
    let page: DocumentPage = codec::decode(
        &events
            .iter()
            .find(|e| e.kind == abi::EVENT_DOCUMENT_PAGE)
            .unwrap()
            .payload,
    )
    .unwrap();
    assert!(page.source.offset >= 1_u64 << 34);
    let id = host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: 0,
            backwards: false,
            max_bytes: 128,
            max_lines: 20,
        },
    );
    assert!(host.run_one());
    host.cancel(id);
    let events = host.finish();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.id == id && e.kind == abi::EVENT_CANCELLED)
            .count(),
        1
    );
    assert!(
        !events
            .iter()
            .any(|e| e.id == id && e.kind == abi::EVENT_DOCUMENT_PAGE)
    );
}

#[test]
fn mid_fence_viewports_keep_code_context_and_respect_output_budget() {
    let input = format!(
        "```\n{}# still code\n```\n\n# Heading\n",
        "text line\n".repeat(1000)
    );
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len() as u64);
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: 10004,
            backwards: false,
            max_bytes: 256,
            max_lines: 5,
        },
    );
    let events = host.finish();
    let page: DocumentPage = codec::decode(
        &events
            .iter()
            .find(|e| e.kind == abi::EVENT_DOCUMENT_PAGE)
            .unwrap()
            .payload,
    )
    .unwrap();
    assert_eq!(page.blocks[0].kind, BLOCK_CODE);
    assert!(page.blocks[0].text.text.contains("# still code"));
    assert!(page.blocks.iter().map(|b| b.text.text.len()).sum::<usize>() <= 256);
    host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: "# still code".into(),
            source_anchor: 0,
            backwards: false,
            case_sensitive: true,
        },
    );
    let events = host.finish();
    assert!(
        events.iter().any(|e| e.kind == abi::EVENT_SEARCH_MATCH),
        "{events:?}"
    );
}

#[test]
fn search_casefolds_unicode_and_crosses_content_pages() {
    let input = format!("{}Straße\n", "a".repeat(8190));
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len() as u64);
    host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: "straße".into(),
            source_anchor: 0,
            backwards: false,
            case_sensitive: false,
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
    assert_eq!(found.source.offset, 8190);
}

#[test]
fn dense_markdown_stays_readable_at_the_combined_wire_item_limit() {
    let input = "a*b*".repeat(4095);
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len() as u64);
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
    let page: DocumentPage = codec::decode(
        &events
            .iter()
            .find(|e| e.kind == abi::EVENT_DOCUMENT_PAGE)
            .expect("readable bounded page")
            .payload,
    )
    .unwrap();
    let text: String = page.blocks.iter().map(|b| b.text.text.as_str()).collect();
    assert_eq!(text, "ab".repeat(4095));
    assert!(page.blocks.iter().all(|b| b.text.spans.is_empty()));
    assert!(!events.iter().any(|e| e.kind == abi::EVENT_FAILED));
    host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: "ababa".into(),
            source_anchor: 0,
            backwards: false,
            case_sensitive: true,
        },
    );
    assert!(
        host.finish()
            .iter()
            .any(|e| e.kind == abi::EVENT_SEARCH_MATCH)
    );
}

fn viewport_bytes() -> Vec<u8> {
    codec::encode(&Viewport {
        source_anchor: 0,
        backwards: false,
        max_bytes: 8192,
        max_lines: 20,
    })
    .unwrap()
}

fn terminal(events: &[support::Event], request: u64) -> u32 {
    events
        .iter()
        .find(|e| {
            e.id == request
                && [
                    abi::EVENT_COMPLETED,
                    abi::EVENT_FAILED,
                    abi::EVENT_CANCELLED,
                ]
                .contains(&e.kind)
        })
        .unwrap_or_else(|| panic!("request {request} has no terminal event: {events:?}"))
        .kind
}

#[test]
fn requests_of_a_stale_generation_are_rejected_after_reopen() {
    let input = b"# Heading\n\ntext\n";
    let mut host = Host::new(input, entry);
    open(&mut host, input.len() as u64);
    host.submit_bytes(abi::REQUEST_CLOSE, &[]);
    host.finish();
    let reopen = codec::encode(&Open {
        reader: 1,
        length: Some(input.len() as u64),
        revision: 4,
        mode: MODE_DOCUMENT,
        display_name: "test.md".into(),
    })
    .unwrap();
    host.submit_generation(abi::REQUEST_OPEN, &reopen, 3);
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
    let stale = host.submit_generation(abi::REQUEST_VIEWPORT, &viewport_bytes(), 2);
    let events = host.finish();
    assert_eq!(terminal(&events, stale), abi::EVENT_FAILED, "{events:?}");
    assert!(events.iter().all(|e| e.kind != abi::EVENT_DOCUMENT_PAGE));
    // A stale Close leaves the live generation open.
    host.submit_generation(abi::REQUEST_CLOSE, &[], 2);
    host.finish();
    let live = host.submit_generation(abi::REQUEST_VIEWPORT, &viewport_bytes(), 3);
    let events = host.finish();
    let page: DocumentPage = codec::decode(
        &events
            .iter()
            .find(|e| e.id == live && e.kind == abi::EVENT_DOCUMENT_PAGE)
            .expect("live generation page")
            .payload,
    )
    .unwrap();
    assert_eq!(page.revision, 4);
}

#[test]
fn superseded_viewports_waiting_for_reads_do_not_exhaust_work_slots() {
    let input = "# Heading\n\ntext\n".repeat(100);
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len() as u64);
    // Each viewport is admitted and cancelled while its read is still queued,
    // as when the user scrolls faster than slow storage answers.
    for anchor in 0..6_u64 {
        let request = host.submit(
            abi::REQUEST_VIEWPORT,
            &Viewport {
                source_anchor: anchor * 16,
                backwards: false,
                max_bytes: 8192,
                max_lines: 20,
            },
        );
        let events = host.run_steps(1);
        assert!(
            events.iter().all(|e| e.kind != abi::EVENT_FAILED),
            "{events:?}"
        );
        host.cancel(request);
    }
    let latest = host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: 160,
            backwards: false,
            max_bytes: 8192,
            max_lines: 20,
        },
    );
    let events = host.finish();
    assert!(
        events.iter().all(|e| e.kind != abi::EVENT_FAILED),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|e| e.id == latest && e.kind == abi::EVENT_DOCUMENT_PAGE)
    );
}
