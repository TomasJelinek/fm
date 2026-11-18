use super::*;
use crate::checkpoints::{BATCH_HOT, MAX_RECORDS};
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
        assert!(batch.retained_len() < MAX_RECORDS);
    }
    let retained = batch.iter().collect::<Vec<_>>();
    assert!(
        retained.ends_with(
            &(32770 - BATCH_HOT as u64..=32769)
                .map(|at| (at, state))
                .collect::<Vec<_>>()
        )
    );
    assert!(
        retained.iter().any(|(at, _)| *at <= 4096),
        "keep distributed early coverage too"
    );
    assert!(size_of::<CheckpointBatch>() <= 8192);
    println!(
        "checkpoint_state_bytes={} staging_bytes={} staging_slots={}",
        size_of::<State>(),
        size_of::<CheckpointBatch>(),
        batch.retained_len()
    );
}

#[test]
fn live_cache_is_bounded_before_insertion_and_keeps_the_bom() {
    let mut hints = Hints::new("dense.cpp", MODE_SOURCE);
    hints.initialize("", 3);
    for offset in 4..=32769 {
        hints.insert_checkpoint(offset, State::default());
        assert!(hints.checkpoints.retained_len() <= MAX_RECORDS);
    }
    assert_eq!(hints.checkpoints.get(&3), Some(&State::default()));
    assert_eq!(hints.checkpoints.nearest(16390).unwrap().0, 16387);
    assert_eq!(hints.checkpoints.nearest(32769).unwrap().0, 32769);

    // Revisited older lines retain their exact state without losing the newest point.
    hints.insert_checkpoint(4, comment_state());
    assert_eq!(hints.checkpoints.get(&4), Some(&comment_state()));
    assert!(hints.checkpoints.get(&32769).is_some());
    assert!(hints.checkpoints.retained_len() <= MAX_RECORDS);
    // Updating an existing state must not evict an unrelated checkpoint.
    hints.insert_checkpoint(32769, comment_state());
    let records = hints.checkpoints.retained_len();
    assert_eq!(hints.checkpoints.get(&32769), Some(&comment_state()));
    hints.insert_checkpoint(32769, comment_state());
    assert_eq!(
        hints.checkpoints.retained_len(),
        records,
        "updating must not allocate/evict"
    );
    assert_eq!(hints.checkpoints.get(&3), Some(&State::default()));
}

#[test]
fn eviction_does_not_expand_the_replay_horizon_or_overflow_u64() {
    let mut hints = Hints::new("dense.cpp", MODE_SOURCE);
    hints.initialize("", 3);
    for offset in 4..=70000 {
        hints.insert_checkpoint(offset, State::default());
    }
    let session = session(hints);
    for (anchor, expected) in [
        (3, 3),
        (32771, 32771),
        (32772, 32771),
        (70000, 70000),
        (u64::MAX, u64::MAX - 4),
    ] {
        let request = Viewport {
            source_anchor: anchor,
            backwards: false,
            max_bytes: 1,
            max_lines: 1,
        };
        let ReadPlan {
            offset,
            count,
            anchor: selected,
            ..
        } = plan(&session, &request);
        assert_eq!(offset, expected, "anchor {anchor}");
        assert_eq!(selected, anchor);
        assert!(count <= 65536);
        assert!(offset.checked_add(u64::from(count)).is_some());
    }
}

#[test]
fn uncached_prefix_replay_still_stops_at_the_exact_32k_horizon() {
    let mut hints = Hints::new("dense.cpp", MODE_SOURCE);
    hints.initialize("", 3);
    let session = session(hints);
    for (anchor, expected) in [(32771, 3), (32772, 32768)] {
        let request = Viewport {
            source_anchor: anchor,
            backwards: false,
            max_bytes: 1,
            max_lines: 1,
        };
        assert_eq!(plan(&session, &request).offset, expected);
    }
}

fn session(hints: Hints) -> Session {
    Session {
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
        preparation: Mutex::new(preparation::Index::default()),
    }
}
