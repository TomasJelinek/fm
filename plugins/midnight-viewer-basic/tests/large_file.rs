#[allow(dead_code)]
mod harness;
#[path = "harness/allocation_meter.rs"]
mod meter;
use harness::Host;
use midnight_plugin_sdk::api::{abi, codec, messages, viewer};

#[test]
fn ten_gigabyte_first_view_and_distant_seek_only_read_bounded_windows() {
    const LENGTH: u64 = 10_000_000_000;
    for mode in [viewer::MODE_TEXT, viewer::MODE_BINARY] {
        let baseline = meter::start();
        let started = std::time::Instant::now();
        // Existing sparse host synthesizes the prefix; only these4 bytes exist in RAM.
        let mut host = Host::new(b"end\n".to_vec());
        host.base = LENGTH - 4;
        host.open(mode);
        assert_eq!(
            host.reads
                .iter()
                .map(|read| u64::from(read.length))
                .sum::<u64>(),
            4
        );
        for (index, anchor) in [0, 9_000_000_000, 4096, LENGTH - 8192]
            .into_iter()
            .enumerate()
        {
            let before = host.reads.len();
            let id = index as u64 + 2;
            let view_started = std::time::Instant::now();
            host.submit(
                id,
                1,
                1,
                abi::REQUEST_VIEWPORT,
                &messages::Viewport {
                    source_anchor: anchor,
                    backwards: false,
                    max_bytes: 8192,
                    max_lines: 80,
                },
            );
            host.pump();
            if mode == viewer::MODE_TEXT {
                let page: viewer::TextPage =
                    codec::decode(&host.event(id, abi::EVENT_TEXT_PAGE).payload).unwrap();
                assert!(!page.text.is_empty());
                assert!(page.text.len() <= 8192);
            } else {
                assert!(!host.event(id, abi::EVENT_BINARY_PAGE).content.is_empty());
                assert!(host.event(id, abi::EVENT_BINARY_PAGE).content.len() <= 8192);
            }
            let reads = &host.reads[before..];
            let bytes = reads.iter().map(|read| u64::from(read.length)).sum::<u64>();
            assert!(bytes <= 8192, "mode {mode}, anchor {anchor}: {bytes}");
            assert!(
                reads
                    .iter()
                    .all(|read| read.offset.abs_diff(anchor) <= 8192)
            );
            println!(
                "basic mode={mode} length={LENGTH} anchor={anchor} requested_bytes={bytes} viewport_us={} since_open_us={}",
                view_started.elapsed().as_micros(),
                started.elapsed().as_micros()
            );
        }
        let peak = meter::peak_since(baseline);
        assert!(
            peak < 16 * 1024 * 1024,
            "10GB content must not become whole-file RAM: {peak}"
        );
        println!("basic mode={mode} incremental_rust_heap_peak_bytes={peak}");
    }
}
