use super::*;
use crate::{decode::Encoding, plugin::Metadata, positions};
use std::sync::{Mutex, atomic::AtomicBool};

fn comment_state() -> State {
    let mut state = State::default();
    syntax::highlight("/*\n", Language::Cpp, &mut state, || false).unwrap();
    state
}

#[test]
fn staging_keeps_the_latest_states_in_a_fixed_memory_budget() {
    let mut batch = CheckpointBatch::new();
    let state = comment_state();
    for offset in 1..=32769 {
        batch.push((offset, state));
        assert!(batch.len < MAX_CHECKPOINTS);
    }
    assert_eq!(
        batch.iter().collect::<Vec<_>>(),
        (32515..=32769).map(|at| (at, state)).collect::<Vec<_>>()
    );
    assert!(size_of::<CheckpointBatch>() <= 8192);
    println!(
        "checkpoint_state_bytes={} staging_bytes={} staging_slots={}",
        size_of::<State>(),
        size_of::<CheckpointBatch>(),
        MAX_CHECKPOINTS - 1
    );
}

#[test]
fn live_cache_is_bounded_before_insertion_and_keeps_the_bom() {
    let mut hints = Hints::new("dense.cpp", MODE_SOURCE);
    hints.initialize("", 3);
    for offset in 4..=32769 {
        hints.insert_checkpoint(offset, State::default());
        assert!(hints.checkpoints.len() <= MAX_CHECKPOINTS);
    }
    let expected: Vec<_> = std::iter::once(3).chain(32515..=32769).collect();
    assert_eq!(
        hints.checkpoints.keys().copied().collect::<Vec<_>>(),
        expected
    );

    // Revisited older lines cannot displace the retained latest coverage.
    hints.insert_checkpoint(4, comment_state());
    assert_eq!(
        hints.checkpoints.keys().copied().collect::<Vec<_>>(),
        expected
    );
    // Updating an existing state must not evict an unrelated checkpoint.
    hints.insert_checkpoint(32769, comment_state());
    assert_eq!(hints.checkpoints[&32769], comment_state());
    assert_eq!(
        hints.checkpoints.keys().copied().collect::<Vec<_>>(),
        expected
    );
    assert_eq!(hints.checkpoints[&3], State::default());
}

#[test]
fn eviction_does_not_expand_the_replay_horizon_or_overflow_u64() {
    let mut hints = Hints::new("dense.cpp", MODE_SOURCE);
    hints.initialize("", 3);
    for offset in 4..=70000 {
        hints.insert_checkpoint(offset, State::default());
    }
    let session = Session {
        reader: 1,
        revision: 3,
        length: None,
        closed: AtomicBool::new(false),
        metadata: Mutex::new(Metadata {
            encoding: Encoding::Utf8,
            bom: 3,
            ready: true,
        }),
        hints: Mutex::new(hints),
        positions: Mutex::new(positions::Index::default()),
    };
    for (anchor, expected) in [
        (3, 3),
        (32771, 3),
        (32772, 32768),
        (70000, 70000),
        (u64::MAX, u64::MAX - 4),
    ] {
        let request = Viewport {
            source_anchor: anchor,
            backwards: false,
            max_bytes: 1,
            max_lines: 1,
        };
        let (offset, count, selected) = plan(&session, &request);
        assert_eq!(offset, expected, "anchor {anchor}");
        assert_eq!(selected, anchor);
        assert!(count <= 65536);
        assert!(offset.checked_add(u64::from(count)).is_some());
    }
}
