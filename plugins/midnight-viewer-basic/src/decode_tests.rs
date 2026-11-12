use super::decode::{Encoding, detect, page};
use midnight_plugin_sdk::api::{codec, viewer::MAX_PAGE_BYTES};

#[test]
fn detects_only_bom_marked_utf16_and_utf8() {
    assert_eq!(detect(&[255, 254, 65, 0]), (Encoding::Utf16Le, 2));
    assert_eq!(detect(&[254, 255, 0, 65]), (Encoding::Utf16Be, 2));
    assert_eq!(detect(&[239, 187, 191, 65]), (Encoding::Utf8, 3));
    assert_eq!(detect(&[65, 0]), (Encoding::Utf8, 0));
}

#[test]
fn unicode_newlines_and_source_anchors_are_exact() {
    let p = page(
        "A\r\nČ\ré\n🦀".as_bytes(),
        Encoding::Utf8,
        1 << 34,
        true,
        100,
        10,
    );
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
    assert!(codec::encode(&p).is_ok());
}

#[test]
fn invalid_bytes_and_terminal_controls_are_visible() {
    let p = page(b"x\x1b[31m\0\xff\x7f", Encoding::Utf8, 0, true, 100, 10);
    assert_eq!(p.text, "x\\x1B[31m\\x00\\xFF\\x7F");
    assert!(!p.text.contains('\x1b'));
    assert_eq!(p.anchors.last().unwrap().source_offset, 9);
}

#[test]
fn split_utf8_and_cr_wait_for_more_input() {
    let p = page(b"a\xe2\x82", Encoding::Utf8, 20, false, 100, 10);
    assert_eq!(
        (p.text.as_str(), p.source.length, p.has_after),
        ("a", 1, true)
    );
    let p = page(b"a\r", Encoding::Utf8, 20, false, 100, 10);
    assert_eq!((p.text.as_str(), p.source.length), ("a", 1));
    let p = page(b"\xe2\x82", Encoding::Utf8, 21, true, 100, 10);
    assert_eq!(p.text, "\\xE2\\x82");
}

#[test]
fn utf16_surrogates_malformed_units_and_odd_tail_preserve_bytes() {
    let p = page(
        &[65, 0, 61, 216, 0, 222, 13, 0, 10, 0, 0, 216, 255],
        Encoding::Utf16Le,
        2,
        true,
        100,
        10,
    );
    assert_eq!(p.text, "A😀\n\\uD800\\xFF");
    assert_eq!(
        p.anchors
            .iter()
            .map(|a| (a.text_offset, a.source_offset))
            .collect::<Vec<_>>(),
        vec![(0, 2), (1, 4), (5, 8), (6, 12), (12, 14), (16, 15)]
    );
    let p = page(
        &[0, 65, 216, 61, 222, 0],
        Encoding::Utf16Be,
        2,
        true,
        100,
        10,
    );
    assert_eq!(p.text, "A😀");
    let p = page(&[65, 0, 61, 216], Encoding::Utf16Le, 2, false, 100, 10);
    assert_eq!((p.text.as_str(), p.source.length), ("A", 2));
}

#[test]
fn huge_line_and_envelope_are_bounded_without_losing_the_next_anchor() {
    let bytes = vec![b'x'; MAX_PAGE_BYTES * 2];
    let p = page(&bytes, Encoding::Utf8, 0, false, MAX_PAGE_BYTES, 4096);
    assert!(p.text.len() <= MAX_PAGE_BYTES);
    assert!(p.source.length > 0 && p.source.length < bytes.len() as u64);
    assert!(p.partial_line && p.has_after);
    assert!(codec::encode(&p).is_ok());
    assert_eq!(
        p.anchors.last().unwrap().source_offset,
        p.source.end().unwrap()
    );
}

#[test]
fn text_and_line_budgets_never_split_a_character_or_escape() {
    let p = page("é🦀abc".as_bytes(), Encoding::Utf8, 0, true, 5, 20);
    assert_eq!((p.text.as_str(), p.source.length), ("é", 2));
    let p = page(b"a\nb\nc", Encoding::Utf8, 0, true, 100, 1);
    assert_eq!(p.text, "a\n");
    let p = page(b"\xff", Encoding::Utf8, 0, true, 3, 10);
    assert!(p.text.is_empty());
}
