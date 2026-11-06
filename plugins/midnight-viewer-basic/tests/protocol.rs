mod harness;
use harness::Host;
use midnight_plugin_sdk::api::{abi, codec, messages, viewer};
fn viewport(anchor: u64) -> messages::Viewport {
    messages::Viewport {
        source_anchor: anchor,
        backwards: false,
        max_bytes: 65536,
        max_lines: 4096,
    }
}

#[test]
fn open_decodes_split_bom_then_short_read_utf16_without_false_eof() {
    let mut h = Host::new(vec![255, 254, 61, 216, 0, 222, 13, 0, 10, 0, 65, 0]);
    h.short = 1;
    h.open(viewer::MODE_TEXT);
    let opened: messages::Opened = codec::decode(&h.event(1, abi::EVENT_OPENED).payload).unwrap();
    assert_eq!(opened.initial_anchor, 2);
    h.submit(2, 1, 1, abi::REQUEST_VIEWPORT, &viewport(2));
    h.pump();
    let p: viewer::TextPage = codec::decode(&h.event(2, abi::EVENT_TEXT_PAGE).payload).unwrap();
    assert_eq!(p.text, "😀\nA");
    assert!(!p.has_after);
    assert_eq!(
        p.source,
        viewer::SourceRange {
            offset: 2,
            length: 10
        }
    );
}

#[test]
fn binary_page_keeps_raw_bytes_and_u64_offset() {
    let mut h = Host::new(vec![0, 255, 65, 127]);
    h.base = 1 << 35;
    h.open(viewer::MODE_BINARY);
    h.submit(2, 1, 1, abi::REQUEST_VIEWPORT, &viewport(1 << 35));
    h.pump();
    let e = h.event(2, abi::EVENT_BINARY_PAGE);
    let p: viewer::BinaryPage = codec::decode(&e.payload).unwrap();
    assert_eq!(
        p.source,
        viewer::SourceRange {
            offset: 1 << 35,
            length: 4
        }
    );
    assert_eq!(e.content, vec![0, 255, 65, 127]);
}

#[test]
fn explicit_text_mode_displays_binary_control_bytes() {
    let mut h = Host::new(vec![0, 255, 27]);
    h.open(viewer::MODE_TEXT);
    h.submit(2, 1, 1, abi::REQUEST_VIEWPORT, &viewport(0));
    h.pump();
    let p: viewer::TextPage = codec::decode(&h.event(2, abi::EVENT_TEXT_PAGE).payload).unwrap();
    assert_eq!(p.text, "\\x00\\xFF\\x1B");
}

#[test]
fn search_crosses_read_boundaries_and_backwards_returns_nearest_match() {
    let mut bytes = vec![b'x'; 8190];
    bytes.extend_from_slice(b"foobar---foobar");
    let mut h = Host::new(bytes);
    h.open(viewer::MODE_TEXT);
    let mut query = messages::Search {
        query: "foobar".into(),
        source_anchor: 0,
        backwards: false,
        case_sensitive: true,
    };
    h.submit(2, 1, 1, abi::REQUEST_SEARCH, &query);
    h.pump();
    let m: messages::SearchMatch =
        codec::decode(&h.event(2, abi::EVENT_SEARCH_MATCH).payload).unwrap();
    assert_eq!(
        m.source,
        viewer::SourceRange {
            offset: 8190,
            length: 6
        }
    );
    query.backwards = true;
    query.source_anchor = 8205;
    h.submit(3, 1, 1, abi::REQUEST_SEARCH, &query);
    h.pump();
    let m: messages::SearchMatch =
        codec::decode(&h.event(3, abi::EVENT_SEARCH_MATCH).payload).unwrap();
    assert_eq!(
        m.source,
        viewer::SourceRange {
            offset: 8199,
            length: 6
        }
    );
}

#[test]
fn close_with_pending_read_suppresses_stale_page_and_releases_lease() {
    let mut h = Host::new(b"hello".to_vec());
    h.open(viewer::MODE_TEXT);
    h.submit(2, 1, 1, abi::REQUEST_VIEWPORT, &viewport(0));
    h.jobs();
    assert_eq!(h.pending(), 1);
    h.close(3);
    h.jobs();
    h.pump();
    h.event(3, abi::EVENT_COMPLETED);
    assert!(
        !h.events
            .iter()
            .any(|e| e.id == 2 && e.kind == abi::EVENT_TEXT_PAGE)
    );
    h.event(2, abi::EVENT_FAILED);
}

#[test]
fn cancelled_search_drops_late_content_and_never_reports_a_match() {
    let mut h = Host::new(b"needle".to_vec());
    h.open(viewer::MODE_TEXT);
    h.submit(
        2,
        1,
        1,
        abi::REQUEST_SEARCH,
        &messages::Search {
            query: "needle".into(),
            source_anchor: 0,
            backwards: false,
            case_sensitive: true,
        },
    );
    h.jobs();
    h.cancel(2);
    h.pump();
    h.event(2, abi::EVENT_CANCELLED);
    assert!(
        !h.events
            .iter()
            .any(|e| e.id == 2 && e.kind == abi::EVENT_SEARCH_MATCH)
    );
}

#[test]
fn read_failure_has_terminal_error_and_no_page() {
    let mut h = Host::new(b"hello".to_vec());
    h.open(viewer::MODE_TEXT);
    h.submit(2, 1, 1, abi::REQUEST_VIEWPORT, &viewport(0));
    h.jobs();
    let s = h.service();
    h.reply(s, vec![], false, abi::UNKNOWN_HANDLE);
    h.pump();
    let f: messages::Failure = codec::decode(&h.event(2, abi::EVENT_FAILED).payload).unwrap();
    assert_eq!(f.code, abi::UNKNOWN_HANDLE);
}

#[test]
fn viewport_inside_utf8_or_utf16_scalar_starts_at_its_exact_boundary() {
    for (data, anchor, start, want) in [
        ("A😀B".as_bytes().to_vec(), 3, 1, "😀B"),
        (vec![255, 254, 65, 0, 61, 216, 0, 222, 66, 0], 6, 4, "😀B"),
    ] {
        let mut h = Host::new(data);
        h.open(viewer::MODE_TEXT);
        h.submit(2, 1, 1, abi::REQUEST_VIEWPORT, &viewport(anchor));
        h.pump();
        let p: viewer::TextPage = codec::decode(&h.event(2, abi::EVENT_TEXT_PAGE).payload).unwrap();
        assert_eq!(p.text, want);
        assert_eq!(p.source.offset, start);
    }
}

#[test]
fn backwards_viewport_selects_nearest_lines_before_anchor() {
    let mut h = Host::new(b"aaa\nbbb\nccc\nddd\n".to_vec());
    h.open(viewer::MODE_TEXT);
    let mut v = viewport(16);
    v.backwards = true;
    v.max_lines = 2;
    h.submit(2, 1, 1, abi::REQUEST_VIEWPORT, &v);
    h.pump();
    let p: viewer::TextPage = codec::decode(&h.event(2, abi::EVENT_TEXT_PAGE).payload).unwrap();
    assert_eq!(p.text, "ccc\nddd\n");
    assert_eq!(p.source.offset, 8);
}

#[test]
fn binary_viewport_respects_row_budget() {
    let mut h = Host::new(vec![42; 100]);
    h.open(viewer::MODE_BINARY);
    let mut v = viewport(0);
    v.max_lines = 1;
    h.submit(2, 1, 1, abi::REQUEST_VIEWPORT, &v);
    h.pump();
    assert_eq!(h.event(2, abi::EVENT_BINARY_PAGE).content.len(), 16);
}

#[test]
fn forwards_search_aligns_utf16_without_reporting_matches_before_anchor() {
    let mut h = Host::new(vec![255, 254, 65, 0, 66, 0, 65, 0]);
    h.open(viewer::MODE_TEXT);
    h.submit(
        2,
        1,
        1,
        abi::REQUEST_SEARCH,
        &messages::Search {
            query: "A".into(),
            source_anchor: 3,
            backwards: false,
            case_sensitive: true,
        },
    );
    h.pump();
    let m: messages::SearchMatch =
        codec::decode(&h.event(2, abi::EVENT_SEARCH_MATCH).payload).unwrap();
    assert_eq!(
        m.source,
        viewer::SourceRange {
            offset: 6,
            length: 2
        }
    );
}

#[test]
fn bom_offset_zero_is_valid_for_forward_and_backward_viewports() {
    let mut h = Host::new(vec![255, 254, 65, 0]);
    h.open(viewer::MODE_TEXT);
    let mut v = viewport(0);
    h.submit(2, 1, 1, abi::REQUEST_VIEWPORT, &v);
    h.pump();
    let p: viewer::TextPage = codec::decode(&h.event(2, abi::EVENT_TEXT_PAGE).payload).unwrap();
    assert_eq!((p.source.offset, p.text.as_str()), (2, "A"));
    v.backwards = true;
    h.submit(3, 1, 1, abi::REQUEST_VIEWPORT, &v);
    h.pump();
    let p: viewer::TextPage = codec::decode(&h.event(3, abi::EVENT_TEXT_PAGE).payload).unwrap();
    assert!(p.text.is_empty());
}

#[test]
fn backward_search_cut_inside_valid_utf8_never_creates_invalid_byte_matches() {
    let mut h = Host::new("A😀B".as_bytes().to_vec());
    h.open(viewer::MODE_TEXT);
    h.submit(
        2,
        1,
        1,
        abi::REQUEST_SEARCH,
        &messages::Search {
            query: "\\xF0".into(),
            source_anchor: 3,
            backwards: true,
            case_sensitive: true,
        },
    );
    h.pump();
    h.event(2, abi::EVENT_COMPLETED);
    assert!(
        !h.events
            .iter()
            .any(|e| e.id == 2 && e.kind == abi::EVENT_SEARCH_MATCH)
    );
}
