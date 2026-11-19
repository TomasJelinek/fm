#[path = "support/allocation_meter.rs"]
mod meter;
#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_source::midnight_plugin_entry_v1 as entry;
use std::sync::{Mutex, MutexGuard};
use support::Host;

// The allocator meter is process-global: tests in this binary never overlap.
static METERED: Mutex<()> = Mutex::new(());

fn metered() -> MutexGuard<'static, ()> {
    METERED.lock().unwrap_or_else(|poison| poison.into_inner())
}

#[test]
fn one_byte_newline_dense_viewport_has_a_bounded_checkpoint_heap_peak() {
    // This is the original trigger, through the public ABI: BOM state qualifies
    // for 32 KiB replay and 32,769 completed LF lines precede/end a one-byte page.
    // The old Vec and bulk BTreeMap insert coexist before the 256-entry trim.
    let _metered = metered();
    let input = vec![b'\n'; 32780];
    let mut peaks = Vec::new();
    for name in ["dense.cpp", "dense.json"] {
        let mut host = Host::new(&input, entry);
        host.submit(
            abi::REQUEST_OPEN,
            &Open {
                reader: 1,
                revision: 19,
                length: Some(input.len() as u64),
                mode: MODE_SOURCE,
                display_name: name.into(),
            },
        );
        assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
        let before = host.reads().len();
        let baseline = meter::start();
        let id = host.submit(
            abi::REQUEST_VIEWPORT,
            &Viewport {
                source_anchor: 32768,
                backwards: false,
                max_bytes: 1,
                max_lines: 1,
            },
        );
        let events = host.finish();
        let peak = meter::peak_since(baseline);
        let reads = host.reads();
        assert_eq!(reads[before..].len(), 1);
        assert_eq!(reads[before].offset, 0);
        // BOM replay plus the one-byte, one-line page's span and margin, and the
        // 512-byte lexical lookahead of a highlighted read from a checkpoint
        // (preparation::LOOKAHEAD).
        assert_eq!(reads[before].length, 32768 + 1 + 1 + 8 + 512);
        let page: TextPage = codec::decode(
            &events
                .iter()
                .find(|e| e.id == id && e.kind == abi::EVENT_TEXT_PAGE)
                .expect("one-byte page")
                .payload,
        )
        .unwrap();
        assert_eq!(page.text, "\n");
        assert_eq!(page.revision, 19);
        assert_eq!(
            page.source,
            SourceRange {
                offset: 32768,
                length: 1
            }
        );
        println!("{name}: newline_dense_checkpoint_incremental_heap_peak_bytes={peak}");
        peaks.push((name, peak));
    }
    // Includes unchanged per-scalar decoded anchors (~1 MiB capacity). Build must
    // confirm this ceiling rejects the old bulk staging/map peak and passes now.
    assert!(
        peaks.iter().all(|(_, peak)| *peak < 2 * 1024 * 1024),
        "newline-dense incremental Rust heap peaks: {peaks:?}"
    );
}

#[test]
fn forward_viewport_decodes_only_its_replay_prefix_and_page() {
    // A 64 KiB read for a page 30 KB after the BOM checkpoint must not decode
    // and anchor the 32 KiB that follow the page.
    let _metered = metered();
    let input = "let value = 42; // comment\n".repeat(2400);
    let mut host = Host::new(input.as_bytes(), entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 5,
            length: Some(input.len() as u64),
            mode: MODE_SOURCE,
            display_name: "tail.rs".into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
    let baseline = meter::start();
    let id = host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: 30_000,
            backwards: false,
            max_bytes: 12 * 1024,
            max_lines: 50,
        },
    );
    let events = host.finish();
    let peak = meter::peak_since(baseline);
    let page: TextPage = codec::decode(
        &events
            .iter()
            .find(|e| e.id == id && e.kind == abi::EVENT_TEXT_PAGE)
            .expect("page")
            .payload,
    )
    .unwrap();
    assert!(page.spans.iter().any(|s| s.role == STYLE_KEYWORD));
    println!("forward_viewport_incremental_heap_peak_bytes={peak}");
    assert!(peak < 1280 * 1024, "forward viewport heap peak {peak}");
}
