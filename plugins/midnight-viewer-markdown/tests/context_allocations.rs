//! Small cached viewports must not copy the rest of their retained parse context.
#[path = "support/allocation_meter.rs"]
mod meter;
#[allow(dead_code)]
mod support;

use midnight_plugin_sdk::api::{abi, codec, document::*, messages::*, viewer::*};
use midnight_viewer_markdown::midnight_plugin_entry_v1 as entry;
use support::Host;

fn viewport(host: &mut Host, anchor: u64, backwards: bool) -> DocumentPage {
    host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: anchor,
            backwards,
            max_bytes: 65536,
            max_lines: 1,
        },
    );
    let events = host.finish();
    codec::decode(
        &events
            .iter()
            .find(|event| event.kind == abi::EVENT_DOCUMENT_PAGE)
            .unwrap_or_else(|| panic!("missing document: {events:?}"))
            .payload,
    )
    .unwrap()
}

#[test]
fn nearby_small_pages_do_not_copy_the_retained_context() {
    let input: String = (0..180)
        .map(|index| {
            format!(
                "## Heading {index}\n\nA paragraph with **emphasis**, `code`, and enough words \
                 to give each unrelated block its own exact source anchors.\n\n"
            )
        })
        .collect();
    assert!(input.len() < 32768);
    let mut host = Host::new(input.as_bytes(), entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(input.len() as u64),
            mode: MODE_DOCUMENT,
            display_name: "cached-context.md".into(),
        },
    );
    assert!(
        host.finish()
            .iter()
            .any(|event| event.kind == abi::EVENT_OPENED)
    );
    assert_eq!(viewport(&mut host, 0, false).blocks[0].kind, BLOCK_HEADING);
    let reads = host.reads().len();
    for (index, backwards) in [(80, false), (100, true)] {
        let anchor = input.find(&format!("## Heading {index}\n")).unwrap() as u64;
        let baseline = meter::start();
        let page = viewport(&mut host, anchor, backwards);
        let peak = meter::peak_since(baseline);
        assert_eq!(host.reads().len(), reads, "the source context is resident");
        assert_eq!(page.revision, 3);
        assert!(!page.blocks.is_empty());
        println!("cached-context backwards={backwards} incremental_heap_peak_bytes={peak}");
        assert!(
            peak < 128 * 1024,
            "a one-line viewport must not duplicate unrelated retained blocks: {peak} bytes"
        );
    }
}
