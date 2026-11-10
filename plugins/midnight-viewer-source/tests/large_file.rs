#[path = "support/allocation_meter.rs"]
mod meter;
#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_source::midnight_plugin_entry_v1 as entry;
use support::Host;

const LENGTH: u64 = 10_000_000_000;

fn host(mode: u16) -> Host {
    // Only four physical fixture bytes. Every earlier byte is virtual zero data.
    let mut host = Host::new(b"end\n", entry);
    host.base(LENGTH - 4);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(LENGTH),
            mode,
            display_name: "large.json".into(),
        },
    );
    assert!(
        host.finish()
            .iter()
            .any(|event| event.kind == abi::EVENT_OPENED)
    );
    host
}

#[test]
fn ten_gigabyte_first_view_and_distant_seek_only_read_bounded_windows() {
    for mode in [MODE_TEXT, MODE_SOURCE] {
        let baseline = meter::start();
        let started = std::time::Instant::now();
        let mut host = host(mode);
        assert_eq!(
            host.reads()
                .iter()
                .map(|read| u64::from(read.length))
                .sum::<u64>(),
            512
        );
        for anchor in [0, 9_000_000_000, 4096, LENGTH - 8192] {
            let before = host.reads().len();
            let view_started = std::time::Instant::now();
            let id = host.submit(
                abi::REQUEST_VIEWPORT,
                &Viewport {
                    source_anchor: anchor,
                    backwards: false,
                    max_bytes: 8192,
                    max_lines: 80,
                },
            );
            let events = host.finish();
            let page: TextPage = codec::decode(
                &events
                    .iter()
                    .find(|event| event.id == id && event.kind == abi::EVENT_TEXT_PAGE)
                    .expect("bounded page")
                    .payload,
            )
            .unwrap();
            assert!(!page.text.is_empty());
            assert!(page.text.len() <= 8192);
            let reads = host.reads();
            let reads = &reads[before..];
            let bytes = reads.iter().map(|read| u64::from(read.length)).sum::<u64>();
            assert!(bytes <= 65536, "mode {mode}, anchor {anchor}: {bytes}");
            assert!(
                reads
                    .iter()
                    .all(|read| read.offset.abs_diff(anchor) <= 65536)
            );
            println!(
                "source mode={mode} length={LENGTH} anchor={anchor} requested_bytes={bytes} viewport_us={} since_open_us={}",
                view_started.elapsed().as_micros(),
                started.elapsed().as_micros()
            );
        }
        let peak = meter::peak_since(baseline);
        assert!(
            peak < 16 * 1024 * 1024,
            "10GB content must not become whole-file RAM: {peak}"
        );
        println!("source mode={mode} incremental_rust_heap_peak_bytes={peak}");
    }
}

#[test]
fn distant_optional_coordinates_do_not_scan_and_near_coordinates_still_recover() {
    let mut host = host(MODE_SOURCE);
    let before = host.reads().len();
    let id = host.submit(
        abi::REQUEST_LOCATE,
        &Locate {
            source_anchor: 9_000_000_000,
        },
    );
    let mut events = Vec::new();
    // Bound the regression itself: the old implementation starts a multi-GB chain.
    for _ in 0..24 {
        if !host.run_one() {
            break;
        }
        host.drain(&mut events);
    }
    let completed = events
        .iter()
        .any(|event| event.id == id && event.kind == abi::EVENT_COMPLETED);
    let after = host.reads().len();
    if !completed {
        host.cancel(id);
        host.finish();
    }
    assert!(
        completed,
        "distant optional lookup must finish without indexing the file"
    );
    assert_eq!(
        after, before,
        "distant coordinates must not read toward the anchor"
    );
    assert!(
        !events
            .iter()
            .any(|event| event.id == id && event.kind == abi::EVENT_TEXT_POSITION)
    );
    let near = host.submit(abi::REQUEST_LOCATE, &Locate { source_anchor: 32 });
    let events = host.finish();
    let position: TextPosition = codec::decode(
        &events
            .iter()
            .find(|event| event.id == near && event.kind == abi::EVENT_TEXT_POSITION)
            .expect("nearby exact coordinates recover")
            .payload,
    )
    .unwrap();
    assert_eq!(position.source_offset, 32);
    assert_eq!(position.line, 1);
}

#[test]
fn coordinate_budget_counts_decoding_overlap_and_short_read_delivery() {
    const BUDGET: u64 = 256 * 1024;
    for short in [65536, 127] {
        let mut host = host(MODE_SOURCE);
        host.short_reads(short);
        let before = host.reads().len();
        let id = host.submit(
            abi::REQUEST_LOCATE,
            &Locate {
                source_anchor: BUDGET,
            },
        );
        let events = host.finish();
        let requests = host.reads();
        // Each reply in this virtual source delivers exactly min(request, short).
        // Repeated requests for an unreturned short-read suffix are not I/O;
        // rereading bytes discarded by the decoder is I/O and must be charged.
        let delivered: u64 = requests[before..]
            .iter()
            .map(|read| u64::from(read.length).min(short as u64))
            .sum();
        assert!(
            delivered <= BUDGET,
            "expanded controls/short={short}: read {delivered} bytes"
        );
        assert!(
            events
                .iter()
                .any(|event| event.id == id && event.kind == abi::EVENT_COMPLETED)
        );
        assert!(
            !events
                .iter()
                .any(|event| event.id == id && event.kind == abi::EVENT_TEXT_POSITION),
            "exhausted budget must not fabricate exact coordinates"
        );
    }
}
