use midnight_core::{SelectionMode, SelectionRanges, SelectionSpan};

#[test]
fn disjoint_ranges_preserve_holes_and_apply_explicit_modifier_modes() {
    let ranges = SelectionRanges::new(
        12,
        vec![
            SelectionSpan { first: 1, last: 2 },
            SelectionSpan { first: 7, last: 9 },
        ],
    )
    .unwrap();
    assert_eq!(ranges.len(), 12);
    for (row, hit) in [
        (0, false),
        (1, true),
        (2, true),
        (3, false),
        (7, true),
        (9, true),
        (10, false),
    ] {
        assert_eq!(ranges.contains(row), hit);
        assert_eq!(SelectionMode::Replace.selected(hit, true), hit);
        assert!(SelectionMode::Add.selected(hit, true));
        assert_eq!(SelectionMode::Toggle.selected(hit, true), !hit);
    }
    assert!(!ranges.contains(usize::MAX));
}

#[test]
fn ranges_validate_source_extent_order_overlap_and_bounded_payload() {
    assert!(SelectionRanges::new(0, vec![]).unwrap().is_empty());
    for spans in [
        vec![SelectionSpan { first: 3, last: 2 }],
        vec![SelectionSpan { first: 0, last: 12 }],
        vec![
            SelectionSpan { first: 4, last: 5 },
            SelectionSpan { first: 1, last: 2 },
        ],
        vec![
            SelectionSpan { first: 1, last: 4 },
            SelectionSpan { first: 4, last: 5 },
        ],
    ] {
        assert!(SelectionRanges::new(12, spans).is_err());
    }
    let spans = (0..4097)
        .map(|i| SelectionSpan {
            first: i * 2,
            last: i * 2,
        })
        .collect();
    assert!(SelectionRanges::new(8194, spans).is_err());
    let spans = (0..4096)
        .map(|i| SelectionSpan {
            first: i * 2,
            last: i * 2,
        })
        .collect();
    assert!(SelectionRanges::new(8192, spans).is_ok());
}
