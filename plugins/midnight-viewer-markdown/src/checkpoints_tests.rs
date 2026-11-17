use super::*;

/// The largest distance between consecutive retained checkpoints in `range`.
fn largest_gap(checkpoints: &Checkpoints, range: RangeInclusive<u64>) -> u64 {
    let values: Vec<u64> = checkpoints.set.range(range).copied().collect();
    values
        .windows(2)
        .map(|pair| pair[1] - pair[0])
        .max()
        .unwrap_or(0)
}

/// Forward reading: each parse records boundaries about 1 KiB apart.
fn read_forward(checkpoints: &mut Checkpoints, end: u64) {
    let mut anchor = 0;
    while anchor < end {
        checkpoints.focus(anchor);
        checkpoints.extend((1..=64).map(|step| anchor + step * 1024));
        anchor += 64 * 1024;
    }
}

#[test]
fn a_read_document_stays_replayable_backwards_after_reaching_its_end() {
    let mut checkpoints = Checkpoints::default();
    checkpoints.initialize(0);
    read_forward(&mut checkpoints, 400 * 1024);
    assert!(checkpoints.len() <= MAX_CHECKPOINTS);
    assert_eq!(checkpoints.earliest(0..=0), Some(0), "the BOM is kept");
    // Every viewport may replay at most 32 KiB, so any gap below that keeps
    // scrolling back through the already read document semantic.
    assert!(largest_gap(&checkpoints, 0..=400 * 1024) < 32 * 1024);
}

#[test]
fn a_huge_read_stays_dense_near_the_reader_and_replayable_nearby() {
    let mut checkpoints = Checkpoints::default();
    checkpoints.initialize(0);
    let end = 10 * 1024 * 1024;
    read_forward(&mut checkpoints, end);
    assert!(checkpoints.len() <= MAX_CHECKPOINTS);
    assert!(largest_gap(&checkpoints, end - 64 * 1024..=end) <= 8 * 1024);
    assert!(largest_gap(&checkpoints, end - 1024 * 1024..=end) < 32 * 1024);
    assert_eq!(checkpoints.latest(0..=u64::MAX), Some(end), "furthest kept");
    // Returning to an old region recovers coverage there as it is parsed again.
    let anchor = 2 * 1024 * 1024;
    checkpoints.focus(anchor);
    checkpoints.extend((1..=64).map(|step| anchor + step * 1024));
    assert!(largest_gap(&checkpoints, anchor..=anchor + 64 * 1024) < 32 * 1024);
    assert_eq!(checkpoints.earliest(0..=0), Some(0));
}

#[test]
fn nearest_lookups_respect_their_bounds() {
    let mut checkpoints = Checkpoints::default();
    checkpoints.initialize(3);
    checkpoints.extend([100, 200, 300]);
    assert_eq!(checkpoints.earliest(150..=350), Some(200));
    assert_eq!(checkpoints.latest(150..=350), Some(300));
    assert_eq!(checkpoints.latest(301..=399), None);
}
