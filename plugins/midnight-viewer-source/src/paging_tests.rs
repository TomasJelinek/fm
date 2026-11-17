use super::*;
use crate::{decode::Encoding, plugin::Metadata, positions};
use std::sync::{Mutex, atomic::AtomicBool};

fn comment_state() -> State {
    let mut state = State::default();
    syntax::highlight("/*\n", Language::Cpp, &mut state, || false).unwrap();
    state
}

fn opened(hints: Hints) -> Session {
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
    }
}

/// Stages one pass of newline-dense line starts, as a viewport would.
fn pass(hints: &mut Hints, offsets: impl IntoIterator<Item = u64>, state: State) {
    let mut staged = Staged::new(hints.checkpoints.spacing);
    for offset in offsets {
        staged.push(offset, state);
    }
    hints.checkpoints.commit(&staged);
}

#[test]
fn staging_keeps_the_latest_and_spaced_states_in_a_fixed_memory_budget() {
    let mut staged = Staged::new(INITIAL_SPACING);
    let state = comment_state();
    for offset in 1..=65536 {
        staged.push(offset, state);
    }
    assert_eq!(
        staged.recent().collect::<Vec<_>>(),
        (65473..=65536).map(|at| (at, state)).collect::<Vec<_>>()
    );
    let spaced: Vec<_> = staged.spaced[..staged.spaced_len]
        .iter()
        .map(|&(at, _)| at)
        .collect();
    assert_eq!(spaced, (0..32).map(|n| 1 + n * 2048).collect::<Vec<_>>());
    println!(
        "checkpoint_state_bytes={} staging_bytes={} recent_slots={MAX_RECENT} spaced_slots={MAX_SPACED}",
        size_of::<State>(),
        size_of::<Staged>(),
    );
}

#[test]
fn cover_is_bounded_keeps_the_bom_and_accepts_lower_revisits() {
    let mut hints = Hints::new("dense.cpp", MODE_SOURCE);
    hints.initialize("", 3);
    // Page through 4 MiB of newline-dense text, 32 KiB per pass.
    for pass_start in (4..4 << 20).step_by(32768) {
        pass(&mut hints, pass_start..pass_start + 32768, State::default());
        let checkpoints = &hints.checkpoints;
        assert!(checkpoints.coverage.len() <= MAX_COVERAGE);
        assert!(checkpoints.recent.len() <= MAX_RECENT);
        assert_eq!(checkpoints.coverage.keys().next(), Some(&3));
        if pass_start < 3 << 20 {
            // Every forward and backward replay window still holds a state.
            let keys: Vec<_> = checkpoints.coverage.keys().copied().collect();
            assert!(keys.windows(2).all(|pair| pair[1] - pair[0] < 32752));
        }
    }
    let spacing = hints.checkpoints.spacing;
    assert!(spacing <= 32768, "4 MiB stays replayable: {spacing}");
    // Every visited region keeps a state within one spacing (plus a gap).
    let keys: Vec<_> = hints.checkpoints.coverage.keys().copied().collect();
    assert!(keys.windows(2).all(|pair| pair[1] - pair[0] < 2 * spacing));
    assert!(*keys.last().unwrap() > (4 << 20) - 2 * spacing);
    // The latest pass is retained densely for continuation.
    assert_eq!(hints.checkpoints.recent.keys().next_back(), Some(&4194307));

    // Revisiting an early region replaces dense continuation and refines the
    // cover there instead of being rejected as lower than the latest window.
    pass(&mut hints, 100_000..100_100, comment_state());
    assert_eq!(
        hints.checkpoints.recent.keys().copied().collect::<Vec<_>>(),
        (100_036..100_100).collect::<Vec<_>>()
    );
    assert_eq!(
        hints.checkpoints.latest(100_050).unwrap().1,
        comment_state()
    );
    assert_eq!(hints.checkpoints.coverage[&3], State::default());
}

#[test]
fn replay_horizon_and_u64_bounds_hold_for_checkpoint_and_plain_plans() {
    let mut hints = Hints::new("dense.cpp", MODE_SOURCE);
    hints.initialize("", 3);
    pass(&mut hints, 4..=70000, State::default());
    let session = opened(hints);
    // The pass covers line starts every 2 KiB (4 + n * 2048) after the BOM.
    for (anchor, expected) in [
        (3, 3),
        (2051, 3),
        (32771, 30724),
        (70000, 70000),
        (u64::MAX, u64::MAX - 4),
    ] {
        let request = Viewport {
            source_anchor: anchor,
            backwards: false,
            max_bytes: 1,
            max_lines: 1,
        };
        let planned = plan(&session, &request);
        assert_eq!(planned.offset, expected, "anchor {anchor}");
        assert_eq!(planned.anchor, anchor);
        assert_eq!(
            planned.state.is_some(),
            anchor != u64::MAX,
            "anchor {anchor}"
        );
        assert!(planned.count <= 65536);
        assert!(
            planned
                .offset
                .checked_add(u64::from(planned.count))
                .is_some()
        );
    }
    // A backward page replays from a covered state at most 32 KiB before the
    // 8,192 text bytes and 51 line breaks (8,251 UTF-8 bytes) it can occupy.
    let request = Viewport {
        source_anchor: 70000,
        backwards: true,
        max_bytes: 8192,
        max_lines: 50,
    };
    let planned = plan(&session, &request);
    assert!(planned.state.is_some());
    let span = 8192 + 51 + 8;
    assert!((70000 - span - 32768..=70000 - span).contains(&planned.offset));
    assert_eq!(u64::from(planned.count), 70000 + 8 - planned.offset);
    // Forward reads stop at the page's maximum source span.
    let request = Viewport {
        backwards: false,
        ..request
    };
    let planned = plan(&session, &request);
    assert_eq!(
        planned.offset + u64::from(planned.count),
        70000 + 8192 + 50 + 8
    );

    // Plain sessions never replay from the BOM.
    let mut hints = Hints::new("text.cpp", MODE_TEXT);
    hints.initialize("", 3);
    let session = opened(hints);
    let request = Viewport {
        source_anchor: 30000,
        backwards: false,
        max_bytes: 8192,
        max_lines: 50,
    };
    let planned = plan(&session, &request);
    assert_eq!((planned.offset, planned.state), (29996, None));
}
