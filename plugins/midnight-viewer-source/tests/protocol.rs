mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_source::midnight_plugin_entry_v1 as entry;
use support::Host;

fn open(host: &mut Host, length: u64) {
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            length: Some(length),
            revision: 3,
            mode: MODE_SOURCE,
            display_name: "test.rs".into(),
        },
    );
    let events = host.finish();
    assert!(
        events.iter().any(|e| e.kind == abi::EVENT_OPENED),
        "{events:?}"
    );
}

#[test]
fn open_short_reads_viewport_styles_search_and_close() {
    let input = b"/* multi\ncomment */\nlet value = 42;\n";
    let mut host = Host::new(input, entry);
    host.short_reads(7);
    open(&mut host, input.len() as u64);
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: 9,
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
            .expect("page")
            .payload,
    )
    .unwrap();
    assert!(page.text.starts_with("comment */"));
    assert_eq!(page.spans[0].role, STYLE_COMMENT);
    assert!(page.spans.iter().any(|s| s.role == STYLE_KEYWORD));
    host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: "value".into(),
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
            .expect("match")
            .payload,
    )
    .unwrap();
    assert_eq!(
        found.source,
        SourceRange {
            offset: 24,
            length: 5
        }
    );
    host.submit_bytes(abi::REQUEST_CLOSE, &[]);
    let events = host.finish();
    assert!(events.iter().any(|e| e.kind == abi::EVENT_COMPLETED));
    assert!(host.reads().iter().all(|r| r.length <= 65536));
}

#[test]
fn cancelled_delayed_reads_and_large_offsets_do_not_apply_stale_pages() {
    let mut host = Host::new(b"let x = 1;", entry);
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
    let page: TextPage = codec::decode(
        &events
            .iter()
            .find(|e| e.kind == abi::EVENT_TEXT_PAGE)
            .unwrap()
            .payload,
    )
    .unwrap();
    assert!(page.source.offset >= (1_u64 << 34));
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
            .any(|e| e.id == id && e.kind == abi::EVENT_TEXT_PAGE)
    );
}

#[test]
fn cross_page_search_and_reverse_search_preserve_offsets() {
    let mut input = vec![b'x'; 32765];
    input.extend_from_slice(b"needle tail needle");
    let mut host = Host::new(&input, entry);
    open(&mut host, input.len() as u64);
    for (backwards, anchor, expected) in [(false, 0, 32765), (true, input.len() as u64, 32777)] {
        host.submit(
            abi::REQUEST_SEARCH,
            &Search {
                query: "NEEDLE".into(),
                source_anchor: anchor,
                backwards,
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
        assert_eq!(found.source.offset, expected);
    }
}

#[test]
fn dense_tokens_fall_back_to_plain_when_the_wire_item_budget_is_exhausted() {
    let input = "'' ".repeat(2730);
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
    let page: TextPage = codec::decode(
        &events
            .iter()
            .find(|e| e.kind == abi::EVENT_TEXT_PAGE)
            .expect("bounded plain fallback")
            .payload,
    )
    .unwrap();
    assert_eq!(page.text, input);
    assert!(page.spans.is_empty());
}
