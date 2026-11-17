//! Whole-file search cost, block separation and cancellation.
#[allow(dead_code)]
mod support;

use midnight_plugin_sdk::api::{abi, codec, document::*, messages::*, viewer::*};
use midnight_viewer_markdown::midnight_plugin_entry_v1 as entry;
use support::Host;

fn open(input: &str) -> Host {
    let mut host = Host::new(input.as_bytes(), entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(input.len() as u64),
            mode: MODE_DOCUMENT,
            display_name: "search.md".into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
    host
}

fn search(host: &mut Host, query: &str, anchor: u64, backwards: bool) -> Option<SearchMatch> {
    let request = host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: query.into(),
            source_anchor: anchor,
            backwards,
            case_sensitive: true,
        },
    );
    let events = host.finish();
    assert!(
        events.iter().all(|e| e.kind != abi::EVENT_FAILED),
        "{events:?}"
    );
    events
        .iter()
        .find(|e| e.id == request && e.kind == abi::EVENT_SEARCH_MATCH)
        .map(|e| codec::decode(&e.payload).unwrap())
}

/// About 1 MB of ordinary Markdown sections.
fn document() -> String {
    (0..3000)
        .map(|index| {
            format!(
                "## Part {index}\n\nSome **styled** prose with `code` {}\n\n",
                "words ".repeat(45)
            )
        })
        .collect()
}

#[test]
fn a_no_match_search_reads_each_byte_about_once_and_polls_sparsely() {
    let input = document();
    let mut host = open(&input);
    let reads = host.reads().len();
    let polls = host.polls();
    assert_eq!(search(&mut host, "absent needle", 0, false), None);
    let bytes: u64 = host.reads()[reads..]
        .iter()
        .map(|read| u64::from(read.length))
        .sum();
    let length = input.len() as u64;
    assert!(
        bytes <= length + length / 4,
        "search requested {bytes} bytes for {length}"
    );
    let polls = host.polls() - polls;
    assert!(
        polls <= input.len() / 256,
        "{polls} host cancellation polls for {length} bytes"
    );
    // Backward search scans from the beginning with the same amplification.
    let reads = host.reads().len();
    assert_eq!(search(&mut host, "absent needle", length, true), None);
    let bytes: u64 = host.reads()[reads..]
        .iter()
        .map(|read| u64::from(read.length))
        .sum();
    assert!(bytes <= length + length / 4);
}

#[test]
fn matches_are_found_across_reads_in_both_directions() {
    let mut input = document();
    let first = input.len();
    input.push_str("A paragraph with the needle inside.\n\n");
    input.push_str(&document());
    let second = input.len();
    input.push_str("Another needle here.\n");
    let mut host = open(&input);
    let found = search(&mut host, "needle", 0, false).unwrap();
    assert_eq!(found.source.offset, (first + 21) as u64);
    let found = search(&mut host, "needle", found.source.offset + 1, false).unwrap();
    assert_eq!(found.source.offset, (second + 8) as u64);
    let found = search(&mut host, "needle", input.len() as u64, true).unwrap();
    assert_eq!(found.source.offset, (second + 8) as u64);
    let found = search(&mut host, "needle", (second + 8) as u64, true).unwrap();
    assert_eq!(found.source.offset, (first + 21) as u64);
}

#[test]
fn a_match_never_joins_two_paragraphs_even_across_reads() {
    // An 8 KiB paragraph filled a whole former search page; a 58 KB one ends
    // at the last proven boundary of the first 64 KiB read, so the next one
    // starts the next read.
    for first in ["x".repeat(8187), "x ".repeat(29_000)] {
        let input = format!(
            "{first}foo\n\nbar {}\n\nshort foo\n\nbar\n",
            "y ".repeat(20_000)
        );
        let mut host = open(&input);
        assert_eq!(search(&mut host, "foobar", 0, false), None);
        assert_eq!(search(&mut host, "foo bar", 0, false), None);
        let joined = search(&mut host, "foo\nbar", 0, false).unwrap();
        assert_eq!(joined.source.offset, first.len() as u64);
    }
}

#[test]
fn a_long_multiline_code_span_keeps_exact_interior_positions() {
    let code = "code line\n".repeat(1000);
    let input = format!("Intro\n\n`{code}` after\n\nneedle here\n");
    let mut host = open(&input);
    let found = search(&mut host, "needle", 0, false).unwrap();
    assert_eq!(found.source.offset, input.find("needle").unwrap() as u64);
    // A viewport inside the oversized code span shows a bounded fragment and
    // advances instead of returning an empty page at the same anchor.
    let anchor = 8 + 5000;
    let request = host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: anchor,
            backwards: false,
            max_bytes: 8192,
            max_lines: 60,
        },
    );
    let events = host.finish();
    let page: DocumentPage = codec::decode(
        &events
            .iter()
            .find(|e| e.id == request && e.kind == abi::EVENT_DOCUMENT_PAGE)
            .unwrap()
            .payload,
    )
    .unwrap();
    assert!(page.blocks.iter().any(|b| !b.text.text.is_empty()));
    assert!(page.source.end().unwrap() > anchor);
    let text = &page.blocks[0].text;
    assert!(text.text.starts_with("code line code line"));
    assert!(text.anchors.len() > text.text.len() / 2, "interior anchors");
}

#[test]
#[ignore = "manual timing: cargo test --test search_scan -- --ignored --nocapture"]
fn search_throughput() {
    let input: String = (0..4).map(|_| document()).collect();
    let mut host = open(&input);
    let started = std::time::Instant::now();
    assert_eq!(search(&mut host, "absent needle", 0, false), None);
    let elapsed = started.elapsed();
    let bytes: u64 = host.reads().iter().map(|read| u64::from(read.length)).sum();
    println!(
        "markdown_search length={} requested_bytes={bytes} host_polls={} elapsed_ms={} mb_per_s={:.2}",
        input.len(),
        host.polls(),
        elapsed.as_millis(),
        input.len() as f64 / 1e6 / elapsed.as_secs_f64()
    );
}

#[test]
fn host_side_cancellation_stops_a_running_search() {
    let input = document();
    let mut host = open(&input);
    let request = host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: "absent needle".into(),
            source_anchor: 0,
            backwards: false,
            case_sensitive: true,
        },
    );
    host.run_steps(6);
    let reads = host.reads().len();
    assert!(reads >= 3, "the scan is running");
    host.cancel_in_host(request);
    let events = host.finish();
    assert!(
        events
            .iter()
            .all(|e| e.id != request || e.kind != abi::EVENT_COMPLETED),
        "a cancelled search does not complete normally"
    );
    assert!(
        host.reads().len() - reads <= 1,
        "the scan stops within one more read"
    );
}
