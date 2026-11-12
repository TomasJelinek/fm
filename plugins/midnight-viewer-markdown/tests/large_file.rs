#[path = "support/allocation_meter.rs"]
mod meter;
#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_markdown::midnight_plugin_entry_v1 as entry;
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
            display_name: "large.md".into(),
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
    for mode in [MODE_TEXT, MODE_DOCUMENT] {
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
            let payload = &events
                .iter()
                .find(|event| {
                    event.id == id
                        && event.kind
                            == if mode == MODE_TEXT {
                                abi::EVENT_TEXT_PAGE
                            } else {
                                abi::EVENT_DOCUMENT_PAGE
                            }
                })
                .expect("bounded page")
                .payload;
            let text_bytes = if mode == MODE_TEXT {
                codec::decode::<TextPage>(payload).unwrap().text.len()
            } else {
                codec::decode::<midnight_plugin_sdk::api::document::DocumentPage>(payload)
                    .unwrap()
                    .blocks
                    .iter()
                    .map(|block| block.text.text.len())
                    .sum()
            };
            assert!(text_bytes > 0);
            assert!(text_bytes <= 8192);
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
                "markdown mode={mode} length={LENGTH} anchor={anchor} requested_bytes={bytes} viewport_us={} since_open_us={}",
                view_started.elapsed().as_micros(),
                started.elapsed().as_micros()
            );
        }
        let peak = meter::peak_since(baseline);
        assert!(
            peak < 16 * 1024 * 1024,
            "10GB content must not become whole-file RAM: {peak}"
        );
        println!("markdown mode={mode} incremental_rust_heap_peak_bytes={peak}");
    }
}
