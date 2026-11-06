#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, document::*, messages::*, viewer::*};
use midnight_viewer_markdown::midnight_plugin_entry_v1 as entry;
use support::Host;
fn opened(data: &[u8], mode: u16) -> Host {
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
#[test]
fn text_mode_searches_visible_markers() {
    let mut host = opened(b"hello **world**", MODE_TEXT);
    host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: "**world**".into(),
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
fn expanding_context_cannot_hide_a_forward_search_match() {
    let mut data = vec![1; 32768];
    data.extend_from_slice(b"needle");
    let mut host = opened(&data, MODE_DOCUMENT);
    host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: "needle".into(),
            source_anchor: 32000,
            backwards: false,
            case_sensitive: true,
        },
    );
    let events = host.finish();
    let found: SearchMatch = codec::decode(
        &events
            .iter()
            .find(|e| e.kind == abi::EVENT_SEARCH_MATCH)
            .expect("match after bounded context")
            .payload,
    )
    .unwrap();
    assert_eq!(found.source.offset, 32768);
    assert!(host.reads().iter().all(|r| r.length <= 65536));
}
#[test]
fn random_viewport_does_not_invent_replacement_chars() {
    let data = "é".repeat(20000);
    let mut host = opened(data.as_bytes(), MODE_DOCUMENT);
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: 35001,
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
            .expect("page")
            .payload,
    )
    .unwrap();
    assert!(
        page.blocks.iter().all(|b| !b.text.text.contains('�')),
        "{page:?}"
    );
}
#[test]
fn ascii_8192_viewport_fits_wire() {
    let mut host = opened(&vec![b'x'; 20000], MODE_DOCUMENT);
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
    assert!(
        events.iter().any(|e| e.kind == abi::EVENT_DOCUMENT_PAGE),
        "{events:?}"
    );
}
#[test]
fn backward_utf16_page_ends_before_anchor() {
    let mut data = vec![0xff, 0xfe];
    data.extend(
        "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ"
            .encode_utf16()
            .flat_map(u16::to_le_bytes),
    );
    let mut host = opened(&data, MODE_DOCUMENT);
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: 52,
            backwards: true,
            max_bytes: 10,
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
    assert!(page.source.end().unwrap() <= 52, "{:?}", page.source);
}

#[test]
fn uncached_utf16_surrogates_align_on_both_endiannesses() {
    for big_endian in [false, true] {
        let mut data = if big_endian {
            vec![0xfe, 0xff]
        } else {
            vec![0xff, 0xfe]
        };
        data.extend("😀".repeat(20000).encode_utf16().flat_map(|unit| {
            if big_endian {
                unit.to_be_bytes()
            } else {
                unit.to_le_bytes()
            }
        }));
        let mut host = opened(&data, MODE_DOCUMENT);
        for anchor in [35003, 35004, 35005] {
            host.submit(
                abi::REQUEST_VIEWPORT,
                &Viewport {
                    source_anchor: anchor,
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
                    .expect("page")
                    .payload,
            )
            .unwrap();
            assert_eq!(page.source.offset, 35006);
            assert!(page.blocks.iter().all(|b| {
                !b.text.text.contains('�')
                    && b.text
                        .anchors
                        .iter()
                        .all(|a| (a.source_offset - 2) % 4 == 0)
            }));
        }
    }
}
#[test]
fn backward_page_selects_nearest_preceding_lines() {
    let input = b"one\ntwo\nthree\n";
    let mut host = opened(input, MODE_TEXT);
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: input.len() as u64,
            backwards: true,
            max_bytes: 128,
            max_lines: 2,
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
    assert_eq!(page.text, "two\nthree\n");
}
#[test]
fn backward_document_uses_displayed_bytes_and_preserves_markers_only_in_text_mode() {
    let input = b"**alpha** **beta** **gamma**";
    let mut host = opened(input, MODE_DOCUMENT);
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: input.len() as u64,
            backwards: true,
            max_bytes: 10,
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
    assert_eq!(
        page.blocks
            .iter()
            .map(|b| b.text.text.as_str())
            .collect::<String>(),
        "beta gamma"
    );
    assert!(page.source.end().unwrap() <= input.len() as u64);
}

fn assert_empty_text_page(
    data: &[u8],
    anchor: u64,
    backwards: bool,
    expected_offset: u64,
    has_before: bool,
) {
    let mut host = opened(data, MODE_TEXT);
    let request = host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: anchor,
            backwards,
            max_bytes: 1,
            max_lines: 1,
        },
    );
    let events = host.finish();
    let pages: Vec<_> = events
        .iter()
        .filter(|e| e.id == request && e.kind == abi::EVENT_TEXT_PAGE)
        .collect();
    assert_eq!(pages.len(), 1, "empty viewport needs a page: {events:?}");
    let page: TextPage = codec::decode(&pages[0].payload).unwrap();
    assert_eq!(page.revision, 3);
    assert!(page.text.is_empty());
    assert!(page.spans.is_empty());
    assert_eq!(page.source.offset, expected_offset);
    assert_eq!(page.source.length, 0);
    assert_eq!(page.anchors.len(), 1);
    assert_eq!(page.anchors[0].text_offset, 0);
    assert_eq!(page.anchors[0].source_offset, expected_offset);
    assert_eq!(page.has_before, has_before);
    assert_eq!(page.has_after, expected_offset < data.len() as u64);
    assert!(
        events
            .iter()
            .any(|e| e.id == request && e.kind == abi::EVENT_COMPLETED)
    );
    assert!(events.iter().all(|e| e.kind != abi::EVENT_FAILED));
    assert!(host.reads().iter().all(|r| r.length <= 65536));
}

#[test]
fn empty_text_viewport_publishes_a_page() {
    for backwards in [false, true] {
        assert_empty_text_page(b"", 0, backwards, 0, false);
    }
}

#[test]
fn bom_only_text_viewport_publishes_a_page() {
    for data in [&b"\xef\xbb\xbf"[..], &b"\xff\xfe"[..], &b"\xfe\xff"[..]] {
        let end = data.len() as u64;
        assert_empty_text_page(data, 0, false, end, false);
        assert_empty_text_page(data, end, false, end, false);
        assert_empty_text_page(data, end, true, end, false);
        assert_empty_text_page(data, 0, true, 0, false);
    }
}

#[test]
fn eof_text_viewport_publishes_a_page() {
    for (data, end) in [
        (&b"hello **world**"[..], 15),
        (&b"\xef\xbb\xbfhello"[..], 8),
        (&b"\xff\xfeh\0i\0"[..], 6),
        (&b"\xfe\xff\0h\0i"[..], 6),
    ] {
        assert_empty_text_page(data, end, false, end, true);
    }
}
