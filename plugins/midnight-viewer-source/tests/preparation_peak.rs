#[path = "support/allocation_meter.rs"]
mod meter;
#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_source::midnight_plugin_entry_v1 as entry;
use support::Host;

#[test]
fn multi_megabyte_single_line_keeps_preparation_peak_independent_of_the_line_length() {
    let input = format!("/*{}*/ return 7;\n", "x".repeat(3 * 1024 * 1024));
    let mut host = Host::new(input.as_bytes(), entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(input.len() as u64),
            mode: MODE_SOURCE,
            display_name: "giant.cpp".into(),
        },
    );
    host.finish();
    let target = Viewport {
        source_anchor: 2 * 1024 * 1024,
        backwards: false,
        max_bytes: 8192,
        max_lines: 20,
    };
    let baseline = meter::start();
    let mut ready = false;
    for _ in 0..32 {
        let id = host.submit(
            abi::REQUEST_PREPARE_SOURCE,
            &PrepareSource {
                revision: 3,
                viewport: target.clone(),
                max_scan_bytes: 262144,
            },
        );
        let events = host.finish();
        let result: SourcePrepared = codec::decode(
            &events
                .iter()
                .find(|e| e.id == id && e.kind == abi::EVENT_SOURCE_PREPARED)
                .expect("bounded result")
                .payload,
        )
        .unwrap();
        if result.ready {
            ready = true;
            break;
        }
    }
    assert!(ready);
    let peak = meter::peak_since(baseline);
    assert!(
        peak < 3 * 1024 * 1024,
        "a three MiB line must not be copied/retained: peak={peak}"
    );
    println!("single_line_preparation_incremental_rust_heap_peak_bytes={peak}");
    let id = host.submit(abi::REQUEST_VIEWPORT, &target);
    let events = host.finish();
    let actual: TextPage = codec::decode(
        &events
            .iter()
            .find(|e| e.id == id && e.kind == abi::EVENT_TEXT_PAGE)
            .expect("styled page")
            .payload,
    )
    .unwrap();
    assert!(actual.spans.iter().any(|span| span.start == 0
        && span.end == actual.text.len() as u32
        && span.role == STYLE_COMMENT));
    assert!(actual.partial_line);
}
