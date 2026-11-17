//! Opt-in timing evidence for search and viewport hot paths. Run alone, in the
//! release profile: `cargo test --release --test throughput -- --ignored --nocapture`.
#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_source::midnight_plugin_entry_v1 as entry;
use std::time::Instant;
use support::Host;

const MIB: f64 = 1024.0 * 1024.0;

fn rust_lines(total: usize) -> String {
    let mut text = String::with_capacity(total + 128);
    let mut n = 0;
    while text.len() < total {
        text.push_str(&format!(
            "    let value_{n:08} = compute(&items[{n}], \"text {n}\"); // note {n}\n"
        ));
        n += 1;
    }
    text
}

#[test]
#[ignore = "timing evidence; run in release with --ignored --nocapture"]
fn search_and_viewport_throughput() {
    let input = rust_lines(24 << 20);
    let length = input.len() as f64;
    let mut host = Host::new(input.as_bytes(), entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(input.len() as u64),
            mode: MODE_SOURCE,
            display_name: "throughput.rs".into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
    for case_sensitive in [true, false] {
        let reads = host.reads().len();
        let polls = host.cancel_polls();
        let started = Instant::now();
        let id = host.submit(
            abi::REQUEST_SEARCH,
            &Search {
                query: "absent needle".into(),
                source_anchor: 0,
                backwards: false,
                case_sensitive,
            },
        );
        let events = host.finish();
        let seconds = started.elapsed().as_secs_f64();
        assert!(
            events
                .iter()
                .any(|e| e.id == id && e.kind == abi::EVENT_COMPLETED)
        );
        let requested: u64 = host.reads()[reads..]
            .iter()
            .map(|read| u64::from(read.length))
            .sum();
        println!(
            "search case_sensitive={case_sensitive} mib_per_s={:.1} requested_per_byte={:.2} polls_per_mib={:.1}",
            length / MIB / seconds,
            requested as f64 / length,
            (host.cancel_polls() - polls) as f64 / (length / MIB),
        );
    }
    let polls = host.cancel_polls();
    let started = Instant::now();
    let (mut anchor, mut pages) = (0, 0);
    while anchor < 4 << 20 {
        let id = host.submit(
            abi::REQUEST_VIEWPORT,
            &Viewport {
                source_anchor: anchor,
                backwards: false,
                max_bytes: 12 * 1024,
                max_lines: 60,
            },
        );
        let events = host.finish();
        let page: TextPage = codec::decode(
            &events
                .iter()
                .find(|e| e.id == id && e.kind == abi::EVENT_TEXT_PAGE)
                .expect("page")
                .payload,
        )
        .unwrap();
        assert!(!page.spans.is_empty());
        anchor = page.source.end().unwrap();
        pages += 1;
    }
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "viewport pages={pages} us_per_page={:.1} source_mib_per_s={:.1} polls_per_page={:.1}",
        seconds * 1e6 / f64::from(pages),
        anchor as f64 / MIB / seconds,
        (host.cancel_polls() - polls) as f64 / f64::from(pages),
    );
}

#[test]
#[ignore = "timing evidence; run in release with --ignored --nocapture"]
fn decode_and_lex_throughput() {
    use midnight_viewer_source::{
        decode::{Encoding, decode},
        syntax::{self, Language, State},
    };
    let input = rust_lines(4 << 20);
    let started = Instant::now();
    let mut scalars = 0;
    for chunk in input.as_bytes().chunks(32768) {
        let decoded = decode(chunk, 0, Encoding::Utf8, true, 65536, usize::MAX, || false).unwrap();
        scalars += decoded.anchors.len();
    }
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "decode mib_per_s={:.1} scalars={scalars}",
        input.len() as f64 / MIB / seconds
    );
    let started = Instant::now();
    let mut state = State::default();
    let mut spans = 0;
    for line in input.split_inclusive('\n') {
        let lexed = syntax::highlight(line, Language::Rust, &mut state, || false).unwrap();
        spans += lexed.spans.len();
    }
    let seconds = started.elapsed().as_secs_f64();
    println!(
        "lex rust mib_per_s={:.1} spans={spans}",
        input.len() as f64 / MIB / seconds
    );
}
