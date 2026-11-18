use super::*;
use crate::{paging::Hints, plugin::Metadata, positions};
use midnight_plugin_sdk::api::viewer::MODE_SOURCE;
use std::sync::{Mutex, atomic::AtomicBool};

#[allow(dead_code)]
#[path = "../tests/support/mod.rs"]
mod support;

#[test]
fn dense_newlines_find_the_target_boundary_with_bounded_anchor_comparisons() {
    use midnight_plugin_sdk::api::{abi, codec, messages};
    let input = vec![b'\n'; 16384];
    let mut host = support::Host::new(&input, crate::midnight_plugin_entry_v1);
    host.submit(
        abi::REQUEST_OPEN,
        &messages::Open {
            reader: 1,
            revision: 3,
            length: Some(input.len() as u64),
            mode: MODE_SOURCE,
            display_name: "dense.cpp".into(),
        },
    );
    host.finish();
    ANCHOR_COMPARISONS.with(|count| count.set(0));
    let id = host.submit(
        abi::REQUEST_PREPARE_SOURCE,
        &PrepareSource {
            revision: 3,
            viewport: Viewport {
                source_anchor: 8192,
                backwards: false,
                max_bytes: 1,
                max_lines: 1,
            },
            max_scan_bytes: 16384,
        },
    );
    let events = host.finish();
    let result: SourcePrepared = codec::decode(
        &events
            .iter()
            .find(|event| event.id == id && event.kind == abi::EVENT_SOURCE_PREPARED)
            .expect("bounded preparation result")
            .payload,
    )
    .unwrap();
    assert!(result.ready);
    let comparisons = ANCHOR_COMPARISONS.with(std::cell::Cell::get);
    println!(
        "dense_newline_anchor_comparisons={comparisons} decoded_scalars={}",
        input.len()
    );
    assert!(
        comparisons <= 64,
        "one callback must locate its target boundary once, not once per line: {comparisons}"
    );
}

fn session() -> Session {
    let mut hints = Hints::new("test.cpp", MODE_SOURCE);
    hints.initialize("", 0);
    Session {
        reader: 1,
        revision: 3,
        length: None,
        closed: AtomicBool::new(false),
        metadata: Mutex::new(Metadata {
            encoding: decode::Encoding::Utf8,
            bom: 0,
            ready: true,
        }),
        hints: Mutex::new(hints),
        positions: Mutex::new(positions::Index::default()),
        preparation: Mutex::new(Index::default()),
    }
}

#[test]
fn reserved_snapshot_has_full_target_identity_and_is_owned_before_io() {
    let session = session();
    let viewport = Viewport {
        source_anchor: 100000,
        backwards: false,
        max_bytes: 512,
        max_lines: 8,
    };
    let mut state = State::default();
    syntax::highlight("/*", Language::Cpp, &mut state, || false).unwrap();
    let expected = Snapshot::new(99999, state, false);
    {
        let mut index = lock(&session.preparation);
        index.key = Some(Key::new(3, &viewport));
        index.target = Some(Target {
            snapshot: expected,
            origin: 0,
            measured: 102560,
            measured_decoded: 2561,
            selection: Selection::Lookahead(128),
            visible_start: Some(0),
            visible_end: Some(512),
            limit: Some(640),
            watch: None,
            suffix: None,
        });
        index.cursor = Some(Cursor::new(expected));
        index.ready = true;
        for changed in [
            Viewport {
                source_anchor: 100001,
                ..viewport.clone()
            },
            Viewport {
                backwards: true,
                ..viewport.clone()
            },
            Viewport {
                max_bytes: 511,
                ..viewport.clone()
            },
            Viewport {
                max_lines: 7,
                ..viewport.clone()
            },
        ] {
            assert!(index.take_ready(3, &changed).is_none());
        }
        assert!(index.take_ready(4, &viewport).is_none());
    }
    let plan = paging::plan(&session, &viewport);
    assert_eq!(plan.offset, expected.offset);
    assert_eq!(plan.snapshot.unwrap().state, expected.state);
    assert!(!plan.snapshot.unwrap().line_start);
    assert!(lock(&session.preparation).target.is_none());
    for turn in 0..1000 {
        let mut batch = Batch::new();
        for at in 0..128 {
            batch.push((turn * 128 + at, State::default()));
        }
        paging::commit_checkpoints(&session, &batch);
    }
    assert_eq!(
        plan.snapshot.unwrap().state,
        expected.state,
        "the callback owns its state independently of checkpoint eviction"
    );
}

#[test]
fn backward_prepared_origin_bounds_decoded_control_expansion() {
    let viewport = Viewport {
        source_anchor: 100000,
        backwards: true,
        max_bytes: 8192,
        max_lines: 4096,
    };
    let lower = lower(&viewport, 3);
    assert_eq!(lower, 100000 - 2 * 8192 - 8);
    assert!((100000 - lower + 256) * 3 < 65536);
    assert!(size_of::<Index>() < 2048);
}

#[test]
fn live_checkpoint_and_recovery_records_stay_inside_both_retained_budgets() {
    let mut checkpoints = crate::checkpoints::Checkpoints::new();
    for at in 0..1000 {
        checkpoints.insert(at * 8192, State::default());
        assert!(checkpoints.retained_len() + 2 <= crate::checkpoints::MAX_RECORDS);
    }
    let retained_upper_bound = size_of::<Index>()
        + size_of::<crate::checkpoints::Checkpoints>()
        + 128 * (size_of::<(u64, State)>() + 128)
        + 4096
        + size_of::<Suffix>()
        + 128;
    assert!(retained_upper_bound <= 65536);
    println!(
        "syntax_state_bytes={} recovery_index_bytes={} backward_suffix_allocation_bytes={} checkpoint_records={} reserved_records=2 retained_allocation_upper_bound_bytes={retained_upper_bound}",
        size_of::<State>(),
        size_of::<Index>(),
        size_of::<Suffix>(),
        checkpoints.retained_len()
    );
}

#[test]
fn backward_suffix_measurement_matches_existing_page_selection_at_every_scalar() {
    let text = format!("\nλ😀a\n\n{}\nlast λ😀\n", "abλ😀".repeat(1500));
    for max_bytes in [1, 2, 3, 4, 7, 64, 8192] {
        for max_lines in [1, 2, 3, 4096] {
            let mut suffix = Suffix::new();
            for (at, ch) in text.char_indices() {
                assert!(suffix.push(ch, max_bytes, max_lines, || false));
                let end = at + ch.len_utf8();
                let expected = decode::suffix_start(&text, end, max_bytes, max_lines as usize);
                assert_eq!(
                    end - suffix.len,
                    expected,
                    "end={end} bytes={max_bytes} lines={max_lines}"
                );
                assert!(suffix.len <= max_bytes.min(SUFFIX_BYTES));
            }
        }
    }
}

#[test]
fn backward_suffix_trimming_checks_cancellation_before_discarding_a_whole_line() {
    let mut suffix = Suffix::new();
    for _ in 0..8191 {
        assert!(suffix.push('x', 8192, 1, || false));
    }
    assert!(suffix.push('\n', 8192, 1, || false));
    let checks = std::cell::Cell::new(0);
    assert!(!suffix.push('x', 8192, 1, || {
        checks.set(checks.get() + 1);
        checks.get() >= 2
    }));
    assert!(
        suffix.len >= 8192 - 128,
        "cancellation must precede draining a full retained line"
    );
}
