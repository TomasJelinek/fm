#[allow(dead_code)]
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
fn huge_unbroken_unicode_line_has_contiguous_bounded_forward_and_backward_pages() {
    let text = "é😀".repeat(50_000);
    let mut h = Host::new(text.as_bytes().to_vec());
    h.open(viewer::MODE_TEXT);
    h.submit(2, 1, 1, abi::REQUEST_VIEWPORT, &viewport(0));
    h.pump();
    let first: viewer::TextPage = codec::decode(&h.event(2, abi::EVENT_TEXT_PAGE).payload).unwrap();
    assert!(first.has_after && first.partial_line);
    assert!(first.text.len() <= 8192);
    let end = first.source.offset + first.source.length;
    h.submit(3, 1, 1, abi::REQUEST_VIEWPORT, &viewport(end));
    h.pump();
    let second: viewer::TextPage =
        codec::decode(&h.event(3, abi::EVENT_TEXT_PAGE).payload).unwrap();
    assert_eq!(second.source.offset, end);
    assert_eq!(
        format!("{}{}", first.text, second.text),
        &text[..(end + second.source.length) as usize]
    );
    let mut backwards = viewport(end);
    backwards.backwards = true;
    h.submit(4, 1, 1, abi::REQUEST_VIEWPORT, &backwards);
    h.pump();
    let previous: viewer::TextPage =
        codec::decode(&h.event(4, abi::EVENT_TEXT_PAGE).payload).unwrap();
    assert_eq!(previous.source.offset + previous.source.length, end);
    assert_eq!(previous.text, first.text);
}

#[test]
fn binary_search_returns_raw_offsets_across_reads_and_in_reverse() {
    let mut bytes = vec![0xff; 8189];
    bytes.extend_from_slice(b"needle\x00\xffneedle");
    let mut h = Host::new(bytes);
    h.open(viewer::MODE_BINARY);
    for (request, source_anchor, backwards, expected) in [
        (2, 0, false, 8189),
        (3, 8190, false, 8197),
        (4, 8203, true, 8197),
        (5, 8197, true, 8189),
    ] {
        h.submit(
            request,
            1,
            1,
            abi::REQUEST_SEARCH,
            &messages::Search {
                query: "needle".into(),
                source_anchor,
                backwards,
                case_sensitive: true,
            },
        );
        h.pump();
        let found: messages::SearchMatch =
            codec::decode(&h.event(request, abi::EVENT_SEARCH_MATCH).payload).unwrap();
        assert_eq!(
            found.source,
            viewer::SourceRange {
                offset: expected,
                length: 6
            }
        );
    }
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

fn text_page(h: &Host, id: u64) -> viewer::TextPage {
    codec::decode(&h.event(id, abi::EVENT_TEXT_PAGE).payload).unwrap()
}

fn host_viewport(anchor: u64, backwards: bool) -> messages::Viewport {
    // The application requests text pages with a 12 KiB byte budget.
    messages::Viewport {
        source_anchor: anchor,
        backwards,
        max_bytes: 12288,
        max_lines: 40,
    }
}

#[test]
fn backward_utf8_pages_start_at_scalar_boundaries_and_stay_contiguous() {
    // Long lines: anchor - 8180 lands inside a three-byte scalar on every page.
    let text = "中".repeat(100_000);
    let mut h = Host::new(text.as_bytes().to_vec());
    h.open(viewer::MODE_TEXT);
    let mut anchor = 60_000;
    for id in 2..7 {
        h.submit(
            id,
            1,
            1,
            abi::REQUEST_VIEWPORT,
            &host_viewport(anchor, true),
        );
        h.pump();
        let p = text_page(&h, id);
        let start = p.source.offset as usize;
        assert_eq!(p.source.end().unwrap(), anchor, "page {id}");
        assert_eq!(start % 3, 0, "page {id} starts inside a scalar");
        assert_eq!(p.text, text[start..anchor as usize], "page {id}");
        assert!(p.has_before);
        anchor = p.source.offset;
    }
}

#[test]
fn backward_utf16_pages_never_start_inside_a_surrogate_pair() {
    // BOM, 5000 pairs, one BMP unit, 5000 pairs: reads from the second run reach
    // back to an offset at a low surrogate of the first run.
    let units: Vec<u16> = "😀"
        .repeat(5000)
        .encode_utf16()
        .chain("a".encode_utf16())
        .chain("😀".repeat(5000).encode_utf16())
        .collect();
    let mut bytes = vec![255, 254];
    bytes.extend(units.iter().flat_map(|unit| unit.to_le_bytes()));
    let mut h = Host::new(bytes);
    h.open(viewer::MODE_TEXT);
    let mut anchor = 20_004 + 4 * 1000;
    for id in 2..6 {
        h.submit(
            id,
            1,
            1,
            abi::REQUEST_VIEWPORT,
            &host_viewport(anchor, true),
        );
        h.pump();
        let p = text_page(&h, id);
        assert_eq!(p.source.end().unwrap(), anchor, "page {id}");
        let start = (p.source.offset as usize - 2) / 2;
        let expected = char::decode_utf16(units[start..(anchor as usize - 2) / 2].iter().copied())
            .collect::<Result<String, _>>()
            .unwrap_or_else(|_| panic!("page {id} starts inside a surrogate pair"));
        assert_eq!(p.text, expected, "page {id}");
        anchor = p.source.offset;
    }
}

#[test]
fn backward_viewport_inside_the_first_unit_returns_an_empty_page() {
    for (data, anchor) in [
        ("é and more".as_bytes().to_vec(), 1),
        (b"\r\nabc".to_vec(), 1),
        (vec![255, 254, 61, 216, 0, 222, 65, 0], 4),
    ] {
        let mut h = Host::new(data);
        h.open(viewer::MODE_TEXT);
        h.submit(2, 1, 1, abi::REQUEST_VIEWPORT, &host_viewport(anchor, true));
        h.pump();
        let p = text_page(&h, 2);
        assert!(p.text.is_empty() && p.source.length == 0 && !p.has_before);
    }
}

#[test]
fn utf16_carriage_return_waits_for_a_split_line_feed_at_the_read_end() {
    // An odd anchor aligns the read to anchor - 5, so the 8185-byte read ends with
    // CR plus the first byte of its LF.
    let mut units: Vec<u16> = vec![u16::from(b'x'); 4138];
    units.extend([13, 10]);
    units.extend(vec![u16::from(b'y'); 100]);
    let mut bytes = vec![255, 254];
    bytes.extend(units.iter().flat_map(|unit| unit.to_le_bytes()));
    let mut h = Host::new(bytes);
    h.open(viewer::MODE_TEXT);
    h.submit(2, 1, 1, abi::REQUEST_VIEWPORT, &host_viewport(101, false));
    h.pump();
    let first = text_page(&h, 2);
    let end = first.source.end().unwrap();
    h.submit(3, 1, 1, abi::REQUEST_VIEWPORT, &host_viewport(end, false));
    h.pump();
    let second = text_page(&h, 3);
    assert_eq!(second.source.offset, end);
    assert_eq!(
        format!("{}{}", first.text, second.text),
        format!("{}\n{}", "x".repeat(4089), "y".repeat(100))
    );
}

#[test]
fn escaped_bytes_and_controls_are_marked_invalid_unlike_literal_backslash_text() {
    let mut h = Host::new(b"a\x1b[31m\xff\x00\\x1B".to_vec());
    h.open(viewer::MODE_TEXT);
    h.submit(2, 1, 1, abi::REQUEST_VIEWPORT, &viewport(0));
    h.pump();
    let p = text_page(&h, 2);
    assert_eq!(p.text, "a\\x1B[31m\\xFF\\x00\\x1B");
    let marked: Vec<(&str, u16)> = p
        .spans
        .iter()
        .map(|span| (&p.text[span.start as usize..span.end as usize], span.role))
        .collect();
    // Adjacent escapes merge; the literal `\x1B` text at the end stays unmarked.
    assert_eq!(
        marked,
        vec![
            ("\\x1B", viewer::STYLE_INVALID),
            ("\\xFF\\x00", viewer::STYLE_INVALID)
        ]
    );
}

#[test]
fn dense_escapes_stay_within_wire_budgets_in_both_directions() {
    let data = [0u8, b'a'].repeat(9000);
    let length = data.len() as u64;
    let mut h = Host::new(data);
    h.open(viewer::MODE_TEXT);
    h.submit(2, 1, 1, abi::REQUEST_VIEWPORT, &viewport(0));
    let mut backwards = viewport(length);
    backwards.backwards = true;
    h.submit(3, 1, 1, abi::REQUEST_VIEWPORT, &backwards);
    h.pump();
    for id in [2, 3] {
        let p = text_page(&h, id);
        assert!(!p.spans.is_empty() && p.source.length > 0, "page {id}");
        for span in &p.spans {
            assert_eq!(&p.text[span.start as usize..span.end as usize], "\\x00");
        }
    }
}
