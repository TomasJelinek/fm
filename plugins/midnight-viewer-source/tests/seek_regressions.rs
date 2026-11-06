#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_source::midnight_plugin_entry_v1 as entry;
use support::Host;

fn opened(data: &[u8]) -> Host {
    let mut host = Host::new(data, entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(data.len() as u64),
            mode: MODE_SOURCE,
            display_name: "test.rs".into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
    host
}

#[test]
fn initial_bom_page_has_no_previous_content() {
    for (input, bom) in [
        (&b"\xef\xbb\xbffn hi() {}"[..], 3),
        (&b"\xff\xfef\0n\0"[..], 2),
        (&b"\xfe\xff\0f\0n"[..], 2),
    ] {
        let mut host = opened(input);
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
        assert_eq!(page.source.offset, bom);
        assert_eq!(page.anchors[0].source_offset, bom);
        assert!(!page.has_before);
        assert!(!page.has_after);
    }
}
#[test]
fn random_viewport_does_not_invent_replacement_chars() {
    let data = "é".repeat(20000);
    let mut host = opened(data.as_bytes());
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
    let page: TextPage = codec::decode(
        &events
            .iter()
            .find(|e| e.kind == abi::EVENT_TEXT_PAGE)
            .expect("page")
            .payload,
    )
    .unwrap();
    assert!(!page.text.contains('�'), "{:?}", page.text);
}
#[test]
fn forward_search_mid_utf16_unit_preserves_encoding() {
    let mut data = vec![0xff, 0xfe];
    data.extend("abc needle".encode_utf16().flat_map(u16::to_le_bytes));
    let mut host = opened(&data);
    host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: "needle".into(),
            source_anchor: 3,
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
fn ascii_8192_viewport_fits_wire() {
    let mut host = opened(&vec![b'x'; 20000]);
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
        events.iter().any(|e| e.kind == abi::EVENT_TEXT_PAGE),
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
    let mut host = opened(&data);
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
    let page: TextPage = codec::decode(
        &events
            .iter()
            .find(|e| e.kind == abi::EVENT_TEXT_PAGE)
            .expect("page")
            .payload,
    )
    .unwrap();
    assert!(page.source.end().unwrap() <= 52, "{:?}", page.source);
}

#[test]
fn backward_before_bom_does_not_advance_past_the_upper_bound() {
    let mut data = vec![0xff, 0xfe];
    data.extend("abc".encode_utf16().flat_map(u16::to_le_bytes));
    let mut host = opened(&data);
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: 0,
            backwards: true,
            max_bytes: 10,
            max_lines: 20,
        },
    );
    let events = host.finish();
    let page: TextPage = codec::decode(
        &events
            .iter()
            .find(|e| e.kind == abi::EVENT_TEXT_PAGE)
            .expect("empty page")
            .payload,
    )
    .unwrap();
    assert!(page.text.is_empty());
    assert_eq!(page.source.end().unwrap(), 0);
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
        let mut host = opened(&data);
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
            let page: TextPage = codec::decode(
                &events
                    .iter()
                    .find(|e| e.kind == abi::EVENT_TEXT_PAGE)
                    .expect("page")
                    .payload,
            )
            .unwrap();
            assert!(!page.text.contains('�'));
            assert_eq!(page.source.offset, 35006);
            assert!(page.anchors.iter().all(|a| (a.source_offset - 2) % 4 == 0));
        }
    }
}
#[test]
fn backward_page_selects_nearest_preceding_lines() {
    let input = b"one\ntwo\nthree\n";
    let mut host = opened(input);
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
fn aligned_search_ignores_matches_before_the_requested_anchor() {
    let mut data = vec![0xff, 0xfe];
    data.extend("needle needle".encode_utf16().flat_map(u16::to_le_bytes));
    let mut host = opened(&data);
    host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: "needle".into(),
            source_anchor: 3,
            backwards: false,
            case_sensitive: true,
        },
    );
    let events = host.finish();
    let found: SearchMatch = codec::decode(
        &events
            .iter()
            .find(|e| e.kind == abi::EVENT_SEARCH_MATCH)
            .expect("later match")
            .payload,
    )
    .unwrap();
    assert_eq!(found.source.offset, 16);
}
