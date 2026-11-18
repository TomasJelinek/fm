use super::{
    decode::{Encoding, MAX_INVALID_SPANS, MAX_UNITS, detect, page},
    window::select,
};
use midnight_plugin_sdk::api::{
    codec,
    messages::Viewport,
    viewer::{STYLE_INVALID, Span},
};

#[test]
fn detects_only_bom_marked_utf16_and_utf8() {
    assert_eq!(detect(&[255, 254, 65, 0]), (Encoding::Utf16Le, 2));
    assert_eq!(detect(&[254, 255, 0, 65]), (Encoding::Utf16Be, 2));
    assert_eq!(detect(&[239, 187, 191, 65]), (Encoding::Utf8, 3));
    assert_eq!(detect(&[65, 0]), (Encoding::Utf8, 0));
}

#[test]
fn unicode_newlines_and_source_anchors_are_exact() {
    let p = page("A\r\nČ\ré\n🦀".as_bytes(), Encoding::Utf8, 1 << 34, true);
    assert_eq!(p.text, "A\nČ\né\n🦀");
    assert_eq!(p.source.length, 13);
    assert_eq!(
        p.anchors
            .iter()
            .map(|a| (a.text_offset, a.source_offset - (1 << 34)))
            .collect::<Vec<_>>(),
        vec![
            (0, 0),
            (1, 1),
            (2, 3),
            (4, 5),
            (5, 6),
            (7, 8),
            (8, 9),
            (12, 13)
        ]
    );
    assert!(p.spans.is_empty());
    assert!(codec::encode(&p).is_ok());
}

#[test]
fn invalid_bytes_and_terminal_controls_are_visible() {
    let p = page(b"x\x1b[31m\0\xff\x7f", Encoding::Utf8, 0, true);
    assert_eq!(p.text, "x\\x1B[31m\\x00\\xFF\\x7F");
    assert!(!p.text.contains('\x1b'));
    assert_eq!(p.anchors.last().unwrap().source_offset, 9);
    let invalid = |start, end| Span {
        start,
        end,
        role: STYLE_INVALID,
        flags: 0,
    };
    assert_eq!(p.spans, vec![invalid(1, 5), invalid(9, 21)]);
}

#[test]
fn explicit_bidi_controls_are_escaped_and_implicit_marks_kept() {
    // RLO and LRI would reorder the displayed text; RLM is an ordinary RTL mark.
    let p = page(
        "a\u{202e}b\u{2066}c\u{200f}d".as_bytes(),
        Encoding::Utf8,
        0,
        true,
    );
    assert_eq!(p.text, "a\\u202Eb\\u2066c\u{200f}d");
    assert!(
        !p.text
            .chars()
            .any(|c| matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')),
        "no explicit bidi control reaches the displayed text"
    );
    let invalid = |start, end| Span {
        start,
        end,
        role: STYLE_INVALID,
        flags: 0,
    };
    assert_eq!(p.spans, vec![invalid(1, 7), invalid(8, 14)]);
    assert_eq!(
        p.anchors
            .iter()
            .map(|a| (a.text_offset, a.source_offset))
            .collect::<Vec<_>>(),
        vec![
            (0, 0),
            (1, 1),
            (7, 4),
            (8, 5),
            (14, 8),
            (15, 9),
            (18, 12),
            (19, 13)
        ]
    );
    for (bytes, encoding) in [
        ([0x2e, 0x20], Encoding::Utf16Le),
        ([0x20, 0x2e], Encoding::Utf16Be),
    ] {
        let p = page(&bytes, encoding, 2, true);
        assert_eq!(p.text, "\\u202E", "{encoding:?}");
        assert_eq!(p.spans, vec![invalid(0, 6)], "{encoding:?}");
        assert_eq!(
            p.anchors
                .iter()
                .map(|a| (a.text_offset, a.source_offset))
                .collect::<Vec<_>>(),
            vec![(0, 2), (6, 4)],
            "{encoding:?}"
        );
    }
    assert!(codec::encode(&p).is_ok());
}

#[test]
fn split_utf8_and_cr_wait_for_more_input() {
    let p = page(b"a\xe2\x82", Encoding::Utf8, 20, false);
    assert_eq!(
        (p.text.as_str(), p.source.length, p.has_after),
        ("a", 1, true)
    );
    let p = page(b"a\r", Encoding::Utf8, 20, false);
    assert_eq!((p.text.as_str(), p.source.length), ("a", 1));
    let p = page(b"\xe2\x82", Encoding::Utf8, 21, true);
    assert_eq!(p.text, "\\xE2\\x82");
}

#[test]
fn utf16_surrogates_malformed_units_and_odd_tail_preserve_bytes() {
    let p = page(
        &[65, 0, 61, 216, 0, 222, 13, 0, 10, 0, 0, 216, 255],
        Encoding::Utf16Le,
        2,
        true,
    );
    assert_eq!(p.text, "A😀\n\\uD800\\xFF");
    assert_eq!(
        p.anchors
            .iter()
            .map(|a| (a.text_offset, a.source_offset))
            .collect::<Vec<_>>(),
        vec![(0, 2), (1, 4), (5, 8), (6, 12), (12, 14), (16, 15)]
    );
    let p = page(&[0, 65, 216, 61, 222, 0], Encoding::Utf16Be, 2, true);
    assert_eq!(p.text, "A😀");
    let p = page(&[65, 0, 61, 216], Encoding::Utf16Le, 2, false);
    assert_eq!((p.text.as_str(), p.source.length), ("A", 2));
}

#[test]
fn utf16_carriage_return_waits_for_an_incomplete_next_unit() {
    let p = page(&[13, 0, 10], Encoding::Utf16Le, 2, false);
    assert_eq!((p.text.as_str(), p.source.length), ("", 0));
    let p = page(&[13, 0, 10], Encoding::Utf16Le, 2, true);
    assert_eq!(p.text, "\n\\x0A");
}

#[test]
fn unit_cap_bounds_huge_lines_and_escape_runs_without_losing_the_next_anchor() {
    for (byte, text_bytes) in [(b'x', 1), (0xff, 4)] {
        let bytes = vec![byte; 3 * MAX_UNITS];
        let p = page(&bytes, Encoding::Utf8, 0, false);
        assert_eq!(p.anchors.len(), MAX_UNITS + 1);
        assert_eq!(p.source.length, MAX_UNITS as u64);
        assert_eq!(p.text.len(), MAX_UNITS * text_bytes);
        assert!(p.partial_line && p.has_after);
        assert_eq!(
            p.anchors.last().unwrap().source_offset,
            p.source.end().unwrap()
        );
        assert!(codec::encode(&p).is_ok());
    }
}

#[test]
fn alternating_escapes_cap_spans_inside_the_wire_item_budget() {
    let p = page(&[0, b'a'].repeat(MAX_UNITS), Encoding::Utf8, 0, false);
    assert_eq!(p.spans.len(), MAX_INVALID_SPANS);
    assert_eq!(p.anchors.len(), MAX_UNITS + 1);
    assert!(codec::encode(&p).is_ok());
}

#[test]
fn text_and_line_budgets_never_split_a_character_or_escape() {
    let forward = |max_bytes, max_lines| Viewport {
        source_anchor: 0,
        backwards: false,
        max_bytes,
        max_lines,
    };
    let selected = |bytes: &[u8], viewport: &Viewport| {
        select(page(bytes, Encoding::Utf8, 0, true), 0, 0, 0, viewport)
    };
    let p = selected("é🦀abc".as_bytes(), &forward(5, 20));
    assert_eq!((p.text.as_str(), p.source.length), ("é", 2));
    let p = selected(b"a\nb\nc", &forward(100, 1));
    assert_eq!(p.text, "a\n");
    let p = selected(b"\xff", &forward(3, 10));
    assert!(p.text.is_empty() && p.spans.is_empty());
}

#[test]
fn selected_pages_keep_only_rebased_spans_of_their_units() {
    let bytes = b"ab\x00\x01cd\x02ef";
    let full = page(bytes, Encoding::Utf8, 0, true);
    let invalid = |start, end| Span {
        start,
        end,
        role: STYLE_INVALID,
        flags: 0,
    };
    assert_eq!(full.spans, vec![invalid(2, 10), invalid(12, 16)]);
    // Backward page of four source bytes ending at "e": "\x01cd\x02".
    let backwards = Viewport {
        source_anchor: 7,
        backwards: true,
        max_bytes: 100,
        max_lines: 10,
    };
    let p = select(full.clone(), 0, 7, 3, &backwards);
    assert_eq!(p.text, "\\x01cd\\x02");
    assert_eq!(p.spans, vec![invalid(0, 4), invalid(6, 10)]);
    let forward = Viewport {
        source_anchor: 3,
        backwards: false,
        max_bytes: 6,
        max_lines: 10,
    };
    let p = select(full, 0, 3, 3, &forward);
    assert_eq!(p.text, "\\x01cd");
    assert_eq!(p.spans, vec![invalid(0, 4)]);
    assert!(codec::encode(&p).is_ok());
}
