use super::{
    decode::{Encoding, page},
    search::Matcher,
};
use midnight_plugin_sdk::api::viewer::SourceRange;

#[test]
fn matches_across_pages_with_u64_source_offsets() {
    let base = 1 << 35;
    let mut m = Matcher::new("foobar", true);
    assert!(
        m.feed(
            &page(b"xxfoo", Encoding::Utf8, base, false, 100, 10),
            || false
        )
        .unwrap()
        .is_empty()
    );
    let result = m
        .feed(
            &page(b"bar!", Encoding::Utf8, base + 5, true, 100, 10),
            || false,
        )
        .unwrap();
    assert_eq!(
        result,
        vec![SourceRange {
            offset: base + 2,
            length: 6
        }]
    );
}

#[test]
fn full_unicode_case_folding_keeps_original_source_ranges() {
    for (text, query, length) in [
        ("Straße", "STRASSE", 7),
        ("Σςσ", "σσσ", 6),
        ("İ", "i\u{307}", 2),
        ("ﬃ", "FFI", 3),
        ("ČESKÝ", "český", 7),
    ] {
        let p = page(text.as_bytes(), Encoding::Utf8, 100, true, 100, 10);
        assert_eq!(
            Matcher::new(query, false).feed(&p, || false).unwrap(),
            vec![SourceRange {
                offset: 100,
                length
            }]
        );
    }
}

#[test]
fn case_sensitive_and_overlapping_matches_are_correct() {
    let p = page(b"aAaAA", Encoding::Utf8, 0, true, 100, 10);
    assert_eq!(
        Matcher::new("AA", true).feed(&p, || false).unwrap(),
        vec![SourceRange {
            offset: 3,
            length: 2
        }]
    );
    assert_eq!(
        Matcher::new("aaa", false).feed(&p, || false).unwrap(),
        vec![
            SourceRange {
                offset: 0,
                length: 3
            },
            SourceRange {
                offset: 1,
                length: 3
            },
            SourceRange {
                offset: 2,
                length: 3
            }
        ]
    );
}

#[test]
fn utf16_search_and_visible_escape_match_exact_source_bytes() {
    let p = page(
        &[65, 0, 61, 216, 0, 222, 13, 0, 10, 0],
        Encoding::Utf16Le,
        2,
        true,
        100,
        10,
    );
    assert_eq!(
        Matcher::new("😀\n", true).feed(&p, || false).unwrap(),
        vec![SourceRange {
            offset: 4,
            length: 8
        }]
    );
    let p = page(b"\xff", Encoding::Utf8, 0, true, 100, 10);
    assert_eq!(
        Matcher::new("\\xFF", true).feed(&p, || false).unwrap(),
        vec![SourceRange {
            offset: 0,
            length: 1
        }]
    );
}

#[test]
fn cancellation_stops_cpu_search_before_a_match_is_reported() {
    let p = page(&vec![b'x'; 8000], Encoding::Utf8, 0, true, 9000, 10);
    assert!(Matcher::new("x", true).feed(&p, || true).is_err());
}
