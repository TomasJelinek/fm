use midnight_plugin_api::{viewer::*, wire::WireError};

fn page() -> TextPage {
    TextPage {
        revision: 7,
        source: SourceRange {
            offset: 1u64 << 33,
            length: 8,
        },
        text: "a文🙂".into(),
        spans: vec![Span {
            start: 1,
            end: 4,
            role: STYLE_KEYWORD,
            flags: 0,
        }],
        anchors: vec![
            Anchor {
                text_offset: 0,
                source_offset: 1u64 << 33,
            },
            Anchor {
                text_offset: 8,
                source_offset: (1u64 << 33) + 8,
            },
        ],
        has_before: true,
        has_after: true,
        partial_line: false,
    }
}

#[test]
fn source_offsets_above_four_gib_remain_exact() {
    assert!(page().validate().is_ok());
}

#[test]
fn source_overflow_and_ambiguous_utf8_ranges_are_refused() {
    let mut invalid = page();
    invalid.source.offset = u64::MAX;
    assert!(invalid.validate().is_err());
    for (start, end) in [(2, 3), (4, 2), (0, 9)] {
        let mut invalid = page();
        invalid.spans[0].start = start;
        invalid.spans[0].end = end;
        assert!(invalid.validate().is_err());
    }
    let mut invalid = page();
    invalid.spans.push(Span {
        start: 3,
        end: 4,
        role: STYLE_STRING,
        flags: 0,
    });
    assert!(invalid.validate().is_err());
}

#[test]
fn anchors_must_cover_the_page_with_monotonic_valid_boundaries() {
    let mut invalid = page();
    invalid.anchors.clear();
    assert!(invalid.validate().is_err());
    for anchor in [
        Anchor {
            text_offset: 2,
            source_offset: (1u64 << 33) + 2,
        },
        Anchor {
            text_offset: 4,
            source_offset: 1,
        },
        Anchor {
            text_offset: 4,
            source_offset: u64::MAX,
        },
    ] {
        let mut invalid = page();
        invalid.anchors.insert(1, anchor);
        assert!(invalid.validate().is_err());
    }
}

#[test]
fn decoded_text_and_model_collections_have_separate_limits() {
    let mut invalid = page();
    invalid.text = "x".repeat(MAX_PAGE_BYTES + 1);
    assert_eq!(invalid.validate(), Err(WireError::Limit));
    let mut invalid = page();
    invalid.spans = vec![invalid.spans[0].clone(); MAX_SPANS + 1];
    assert_eq!(invalid.validate(), Err(WireError::Limit));
    let mut invalid = page();
    invalid.anchors = vec![invalid.anchors[0].clone(); MAX_ANCHORS + 1];
    assert_eq!(invalid.validate(), Err(WireError::Limit));
}
