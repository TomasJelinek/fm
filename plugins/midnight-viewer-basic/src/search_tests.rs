use super::{
    casefold::FOLD,
    decode::Encoding,
    search::{Matcher, Want, fold},
};
use midnight_plugin_sdk::api::viewer::SourceRange;

fn range(offset: u64, length: u64) -> SourceRange {
    SourceRange { offset, length }
}

/// First and last match of a whole complete text.
fn first_and_last(
    matcher: &mut Matcher,
    bytes: &[u8],
    encoding: Encoding,
    offset: u64,
) -> (Option<SourceRange>, Option<SourceRange>) {
    let first = matcher
        .scan(bytes, encoding, offset, true, Want::FirstFrom(0), || false)
        .unwrap()
        .found;
    matcher.reset();
    let last = matcher
        .scan(
            bytes,
            encoding,
            offset,
            true,
            Want::LastUntil(u64::MAX),
            || false,
        )
        .unwrap()
        .found;
    (first, last)
}

#[test]
fn matches_across_reads_with_u64_source_offsets() {
    let base = 1 << 35;
    let mut m = Matcher::new("foobar", true);
    let scan = m
        .scan(
            b"xxfoo",
            Encoding::Utf8,
            base,
            false,
            Want::FirstFrom(0),
            || false,
        )
        .unwrap();
    assert_eq!((scan.found, scan.consumed), (None, 5));
    let scan = m
        .scan(
            b"bar!",
            Encoding::Utf8,
            base + 5,
            true,
            Want::FirstFrom(0),
            || false,
        )
        .unwrap();
    assert_eq!(scan.found, Some(range(base + 2, 6)));
}

#[test]
fn reset_forgets_a_partial_match_before_an_unrelated_window() {
    let mut m = Matcher::new("foobar", true);
    let scan = m
        .scan(
            b"xxfoo",
            Encoding::Utf8,
            0,
            false,
            Want::LastUntil(5),
            || false,
        )
        .unwrap();
    assert_eq!(scan.found, None);
    m.reset();
    let scan = m
        .scan(
            b"bar",
            Encoding::Utf8,
            100,
            true,
            Want::LastUntil(103),
            || false,
        )
        .unwrap();
    assert_eq!(scan.found, None);
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
        let only = Some(range(100, length));
        assert_eq!(
            first_and_last(
                &mut Matcher::new(query, false),
                text.as_bytes(),
                Encoding::Utf8,
                100
            ),
            (only.clone(), only),
            "{text}"
        );
    }
}

#[test]
fn ascii_fast_path_matches_the_case_folding_table() {
    for c in (0..=127u8).map(char::from) {
        let table = FOLD
            .binary_search_by_key(&c, |entry| entry.0)
            .map_or_else(|_| vec![c], |index| FOLD[index].1.chars().collect());
        assert_eq!(fold(c, false).collect::<Vec<_>>(), table, "{c:?}");
        assert_eq!(fold(c, true).collect::<Vec<_>>(), vec![c]);
    }
}

#[test]
fn case_sensitive_and_overlapping_matches_are_correct() {
    let text = b"aAaAA";
    assert_eq!(
        first_and_last(&mut Matcher::new("AA", true), text, Encoding::Utf8, 0),
        (Some(range(3, 2)), Some(range(3, 2)))
    );
    let mut m = Matcher::new("aaa", false);
    assert_eq!(
        first_and_last(&mut m, text, Encoding::Utf8, 0),
        (Some(range(0, 3)), Some(range(2, 3)))
    );
    m.reset();
    let scan = m
        .scan(text, Encoding::Utf8, 0, true, Want::FirstFrom(1), || false)
        .unwrap();
    assert_eq!(scan.found, Some(range(1, 3)));
    m.reset();
    let scan = m
        .scan(text, Encoding::Utf8, 0, true, Want::LastUntil(4), || false)
        .unwrap();
    assert_eq!(scan.found, Some(range(1, 3)));
}

#[test]
fn utf16_search_and_visible_escape_match_exact_source_bytes() {
    assert_eq!(
        first_and_last(
            &mut Matcher::new("😀\n", true),
            &[65, 0, 61, 216, 0, 222, 13, 0, 10, 0],
            Encoding::Utf16Le,
            2
        ),
        (Some(range(4, 8)), Some(range(4, 8)))
    );
    assert_eq!(
        first_and_last(&mut Matcher::new("\\xFF", true), b"\xff", Encoding::Utf8, 0),
        (Some(range(0, 1)), Some(range(0, 1)))
    );
}

#[test]
fn cancellation_is_polled_at_bounded_intervals() {
    let bytes = vec![b'x'; 20_000];
    let mut polls = 0;
    let scan = Matcher::new("y", true)
        .scan(&bytes, Encoding::Utf8, 0, true, Want::FirstFrom(0), || {
            polls += 1;
            false
        })
        .unwrap();
    assert_eq!((scan.found, scan.consumed, polls), (None, 20_000, 5));
    assert!(
        Matcher::new("x", true)
            .scan(&bytes, Encoding::Utf8, 0, true, Want::FirstFrom(0), || true)
            .is_err()
    );
}
