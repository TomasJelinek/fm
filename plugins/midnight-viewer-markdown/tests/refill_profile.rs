//! Manual public-ABI timings, excluding host shaping, rasterization and presentation.
#[path = "support/allocation_meter.rs"]
mod meter;
#[allow(dead_code)]
mod support;

use midnight_plugin_sdk::api::{abi, codec, document::DocumentPage, messages::*, viewer::*};
use midnight_viewer_markdown::midnight_plugin_entry_v1 as entry;
use std::{io::Write, time::Instant};
use support::Host;

fn viewport(host: &mut Host, anchor: u64, backwards: bool) -> Vec<u8> {
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: anchor,
            backwards,
            max_bytes: 65536,
            max_lines: 64,
        },
    );
    let events = host.finish();
    events
        .into_iter()
        .find(|event| event.kind == abi::EVENT_DOCUMENT_PAGE)
        .unwrap_or_else(|| panic!("missing document at {anchor}, backwards={backwards}"))
        .payload
}

#[test]
#[ignore = "manual measurement; set ME_MARKDOWN_PROFILE_FIXTURE and ME_MARKDOWN_PROFILE_OUTPUT"]
fn exact_fixture_refill_profile() {
    let fixture = std::env::var_os("ME_MARKDOWN_PROFILE_FIXTURE").expect("fixture path");
    let output = std::env::var_os("ME_MARKDOWN_PROFILE_OUTPUT").expect("wire output path");
    let nearby = std::env::var("ME_MARKDOWN_PROFILE_PATTERN").as_deref() == Ok("nearby");
    // Only the test broker materializes its fixture. Production reads remain bounded.
    let input = std::fs::read(&fixture).expect("fixture");
    let mut wire = std::fs::File::create(output).expect("wire output under _build");
    let mut timings = Vec::new();
    let mut peak = 0;
    let mut reads = 0;
    let mut requested_bytes = 0;
    for round in 0..5 {
        let mut host = Host::new(&input, entry);
        host.submit(
            abi::REQUEST_OPEN,
            &Open {
                reader: 1,
                revision: 3,
                length: Some(input.len() as u64),
                mode: MODE_DOCUMENT,
                display_name: "profile.md".into(),
            },
        );
        assert!(
            host.finish()
                .iter()
                .any(|event| event.kind == abi::EVENT_OPENED)
        );
        viewport(&mut host, 0, false);
        let before = host.reads().len();
        for backwards in [false, true] {
            for step in 0..60 {
                let step = if backwards { 59 - step } else { step };
                let anchor = if nearby {
                    input.len() as u64 / 10 + step * (input.len() as u64 / 1000).clamp(1, 97)
                } else {
                    input.len() as u64 * (10 + step) / 100
                };
                let baseline = meter::start();
                let started = Instant::now();
                let payload = viewport(&mut host, anchor, backwards);
                timings.push(started.elapsed().as_micros());
                peak = peak.max(meter::peak_since(baseline));
                let page: DocumentPage = codec::decode(&payload).expect("valid document");
                assert_eq!(page.revision, 3);
                assert!(page.source.end().unwrap() <= input.len() as u64);
                if round == 0 {
                    wire.write_all(&(payload.len() as u32).to_le_bytes())
                        .unwrap();
                    wire.write_all(&payload).unwrap();
                }
            }
        }
        let requests = host.reads();
        reads += requests.len() - before;
        requested_bytes += requests[before..]
            .iter()
            .map(|request| u64::from(request.length))
            .sum::<u64>();
    }
    timings.sort_unstable();
    println!(
        "refill-profile fixture={fixture:?} nearby={nearby} samples={} median_us={} p95_us={} max_us={} \
         source_reads={reads} requested_bytes={requested_bytes} incremental_heap_peak_bytes={peak}",
        timings.len(),
        timings[timings.len() / 2],
        timings[(timings.len() - 1) * 95 / 100],
        timings.last().unwrap(),
    );
}
