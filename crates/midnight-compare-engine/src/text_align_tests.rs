use super::*;
use similar::DiffTag;
use std::sync::mpsc;

/// Ids of a log whose lines are unique (numbered), with every `every`-th line changed.
fn scattered(lines: u32, every: u32) -> [Vec<u32>; 2] {
    let old: Vec<u32> = (0..lines).collect();
    let new = old
        .iter()
        .map(|&line| {
            if line % every == every / 2 {
                lines + line
            } else {
                line
            }
        })
        .collect();
    [old, new]
}

/// The operations cover each sequence once, in order, and equal ranges pair equal ids.
/// (An empty range, such as the new side of a deletion, has no meaningful position.)
/// Consecutive changes are one operation, so each changed operation is one group.
fn assert_valid(ids: &[Vec<u32>; 2], ops: &[DiffOp]) {
    let mut at = [0, 0];
    for op in ops {
        let ranges = [op.old_range(), op.new_range()];
        for (side, range) in ranges.iter().enumerate() {
            if !range.is_empty() {
                assert_eq!(range.start, at[side], "{op:?}");
                at[side] = range.end;
            }
        }
        if op.tag() == DiffTag::Equal {
            assert_eq!(
                ids[0][ranges[0].clone()],
                ids[1][ranges[1].clone()],
                "{op:?}"
            );
        }
    }
    assert_eq!(at, [ids[0].len(), ids[1].len()]);
    for pair in ops.windows(2) {
        assert!(pair.iter().any(|op| op.tag() == DiffTag::Equal), "{pair:?}");
    }
}

fn changed_groups(ops: &[DiffOp]) -> Vec<(usize, usize)> {
    ops.iter()
        .filter(|op| op.tag() != DiffTag::Equal)
        .map(|op| (op.old_range().start, op.new_range().start))
        .collect()
}

#[test]
fn anchors_align_scattered_changes_like_myers_when_myers_runs_out_of_time() {
    let ids = scattered(20_000, 37);
    let cancel = AtomicBool::new(false);
    let optimal = align_ids(&ids, &cancel, Duration::from_secs(60), Duration::ZERO).unwrap();
    assert!(!optimal.approximate);
    // No time for Myers: the anchors still find every changed line on its own.
    let anchored = align_ids(&ids, &cancel, Duration::ZERO, Duration::from_secs(60)).unwrap();
    assert!(!anchored.approximate);
    assert_valid(&ids, &anchored.ops);
    assert_eq!(changed_groups(&anchored.ops), changed_groups(&optimal.ops));
    assert_eq!(changed_groups(&anchored.ops).len(), 20_000 / 37 + 1);
}

#[test]
fn repeated_lines_moved_blocks_and_expired_gaps_never_pair_unequal_lines() {
    let cancel = AtomicBool::new(false);
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    for round in 0..40 {
        // Few distinct values: many repeated lines and few anchors.
        let values = if round % 2 == 0 { 4 } else { 400 };
        let old: Vec<u32> = (0..300).map(|_| (next() % values) as u32).collect();
        let mut new = old.clone();
        for _ in 0..30 {
            let at = (next() % new.len() as u64) as usize;
            match next() % 3 {
                0 => new[at] = (next() % values) as u32,
                1 => {
                    new.remove(at);
                }
                _ => new.insert(at, (next() % values) as u32),
            }
        }
        // Move a block to exercise anchors that change order.
        let block: Vec<u32> = new.drain(10..40).collect();
        new.extend(block);
        let ids = [old, new];
        for (myers, anchored) in [
            (Duration::from_secs(60), Duration::ZERO),
            (Duration::ZERO, Duration::from_secs(60)),
            (Duration::ZERO, Duration::ZERO),
        ] {
            let alignment = align_ids(&ids, &cancel, myers, anchored).unwrap();
            assert_valid(&ids, &alignment.ops);
        }
    }
}

#[test]
fn a_pair_without_anchors_becomes_one_changed_block_after_the_time_runs_out() {
    let ids = [vec![1; 5000], vec![2; 5000]];
    let alignment = align_ids(
        &ids,
        &AtomicBool::new(false),
        Duration::ZERO,
        Duration::from_secs(60),
    )
    .unwrap();
    assert!(alignment.approximate);
    assert_valid(&ids, &alignment.ops);
    assert_eq!(changed_groups(&alignment.ops), [(0, 0)]);
}

#[test]
fn common_ends_of_an_expired_gap_stay_equal() {
    let mut old = vec![7, 8, 9];
    old.extend(std::iter::repeat_n(1, 50));
    old.extend([10, 11]);
    let mut new = vec![7, 8, 9];
    new.extend(std::iter::repeat_n(2, 60));
    new.extend([10, 11]);
    let ids = [old, new];
    let alignment = align_ids(
        &ids,
        &AtomicBool::new(false),
        Duration::ZERO,
        Duration::ZERO,
    )
    .unwrap();
    assert_valid(&ids, &alignment.ops);
    assert_eq!(changed_groups(&alignment.ops), [(3, 3)]);
    assert_eq!(
        alignment.ops.first().map(|op| op.tag()),
        Some(DiffTag::Equal)
    );
    assert_eq!(
        alignment.ops.last().map(|op| op.tag()),
        Some(DiffTag::Equal)
    );
}

#[test]
fn only_a_gap_that_myers_gave_up_on_makes_the_alignment_approximate() {
    let cancel = AtomicBool::new(false);
    // One insertion between common ends needs no search, so Myers never reaches its
    // deadline, even with no time at all, and its alignment is kept as it is.
    let ids = [vec![1, 2, 3], vec![1, 2, 9, 3]];
    let alignment = align_ids(&ids, &cancel, Duration::ZERO, Duration::ZERO).unwrap();
    assert!(!alignment.approximate);
    assert_eq!(changed_groups(&alignment.ops), [(2, 2)]);
    // Myers may give up on the whole pair, but each gap between the anchors is one
    // insertion, which needs no search either.
    let ids = [vec![1, 2, 3], vec![1, 7, 2, 8, 3]];
    let alignment = align_ids(&ids, &cancel, Duration::ZERO, Duration::ZERO).unwrap();
    assert!(!alignment.approximate);
    assert_valid(&ids, &alignment.ops);
    assert_eq!(changed_groups(&alignment.ops), [(1, 1), (2, 3)]);
}

fn hunk(start: [usize; 2], end: [usize; 2]) -> Hunk {
    Hunk { start, end }
}

/// Compacts `hunks` of an alignment of `ids` and checks the operations it gives.
fn compacted(ids: &[Vec<u32>; 2], mut hunks: Vec<Hunk>, budget: usize) -> Vec<Hunk> {
    let cancel = AtomicBool::new(false);
    compact(&mut hunks, ids, budget, &cancel).unwrap();
    let ops = operations(&hunks, [ids[0].len(), ids[1].len()], &cancel).unwrap();
    assert_valid(ids, &ops);
    assert_eq!(
        ops.iter().filter(|op| op.tag() != DiffTag::Equal).count(),
        hunks.len()
    );
    hunks
}

#[test]
fn deletions_and_insertions_slide_through_repeated_lines_to_join_their_neighbours() {
    // File B repeats `1` twice more: the insertion above the equal `1` slides below it
    // and joins the other one.
    let ids = [vec![0, 1, 2], vec![0, 1, 1, 1, 2]];
    let split = vec![hunk([1, 1], [1, 2]), hunk([2, 3], [2, 4])];
    assert_eq!(
        compacted(&ids, split.clone(), usize::MAX),
        [hunk([2, 2], [2, 4])]
    );
    // Deletions slide the same way.
    let ids = [vec![0, 1, 1, 1, 2], vec![0, 1, 2]];
    let deletions = vec![hunk([1, 1], [2, 1]), hunk([3, 2], [4, 2])];
    assert_eq!(
        compacted(&ids, deletions, usize::MAX),
        [hunk([2, 2], [4, 2])]
    );
    // An insertion joins a replacement it reaches; the replacement itself never moves.
    let ids = [vec![0, 5, 1, 2], vec![0, 6, 1, 1, 2]];
    let beside = vec![hunk([1, 1], [2, 2]), hunk([3, 3], [3, 4])];
    assert_eq!(compacted(&ids, beside, usize::MAX), [hunk([1, 1], [2, 3])]);
    let ids = [vec![1, 1, 5, 1], vec![1, 1, 6, 1]];
    let replaced = vec![hunk([2, 2], [3, 3])];
    assert_eq!(compacted(&ids, replaced.clone(), usize::MAX), replaced);
    // Hunks move at most `budget` lines: the join above takes one.
    let ids = [vec![0, 1, 2], vec![0, 1, 1, 1, 2]];
    assert_eq!(compacted(&ids, split.clone(), 0), split);
    assert_eq!(compacted(&ids, split, 1), [hunk([2, 2], [2, 4])]);
}

#[test]
fn cancellation_stops_both_passes() {
    let ids = scattered(5000, 3);
    let cancel = AtomicBool::new(true);
    assert!(align_ids(&ids, &cancel, Duration::from_secs(60), Duration::ZERO).is_err());
    assert!(align_ids(&ids, &cancel, Duration::ZERO, Duration::from_secs(60)).is_err());
}

#[test]
fn cancellation_stops_compaction_and_the_operations() {
    let ids = [vec![0, 1, 2], vec![0, 1, 1, 1, 2]];
    let mut hunks = vec![hunk([1, 1], [1, 2]), hunk([2, 3], [2, 4])];
    let cancel = AtomicBool::new(true);
    assert!(compact(&mut hunks, &ids, usize::MAX, &cancel).is_err());
    assert!(operations(&hunks, [3, 5], &cancel).is_err());
}

#[test]
fn many_changes_between_anchors_align_in_bounded_time() {
    // From a review: Myers runs out of time on 250,000 blocks of a unique line, an empty
    // line and a changed line (with one more empty line in File B), and the anchored
    // pass then emits about a million operations. `similar`'s `Compact` merged them with
    // a vector insert or removal each, for minutes after both deadlines and deaf to
    // cancellation.
    const BLOCKS: usize = 250_000;
    let mut texts = [String::new(), String::new()];
    for block in 0..BLOCKS {
        texts[0].push_str(&format!("U{block}\n\nX{block}\n"));
        texts[1].push_str(&format!("U{block}\n\nY{block}\n\n"));
    }
    let lines = texts.map(TextLines::unchecked);
    let ids = intern(&lines, &AtomicBool::new(false)).unwrap();
    let (sender, receiver) = mpsc::channel();
    let started = Instant::now();
    std::thread::spawn(move || {
        let _ = sender.send(align(&lines, &AtomicBool::new(false)));
    });
    // Both budgets and the linear passes take about two seconds on a loaded machine.
    let alignment = receiver
        .recv_timeout(Duration::from_secs(30))
        .expect("the alignment ends within 30 seconds")
        .unwrap();
    eprintln!("aligned {BLOCKS} blocks in {:?}", started.elapsed());
    assert_valid(&ids, &alignment.ops);
    // Each block is one group: `X<i>` replaced by `Y<i>` and an empty line.
    assert_eq!(changed_groups(&alignment.ops).len(), BLOCKS);
}

#[test]
fn interning_gives_equal_lines_equal_ids() {
    let lines = [
        TextLines::unchecked("a\nb\na\nc".into()),
        TextLines::unchecked("c\na\r\nb\n".into()),
    ];
    let [old, new] = intern(&lines, &AtomicBool::new(false)).unwrap();
    assert_eq!(old.len(), 4);
    assert_eq!(new.len(), 3);
    assert_eq!(old[0], old[2], "a\\n");
    assert_ne!(old[3], new[0], "c without a newline differs from c\\n");
    assert_ne!(old[0], new[1], "CRLF differs from LF");
    assert_eq!(old[1], new[2], "b\\n");
}
