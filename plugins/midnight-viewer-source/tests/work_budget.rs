//! Search and viewport work stays proportional to the content: each source
//! byte is requested about once, host cancellation is polled at bounded
//! intervals, and superseded requests cannot exhaust the operation budget.
#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_source::midnight_plugin_entry_v1 as entry;
use support::Host;

fn opened(input: &[u8], name: &str) -> Host {
    let mut host = Host::new(input, entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(input.len() as u64),
            mode: MODE_SOURCE,
            display_name: name.into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
    host
}

fn requested(host: &Host, from: usize) -> u64 {
    host.reads()[from..]
        .iter()
        .map(|read| u64::from(read.length))
        .sum()
}

fn lines(total: usize) -> String {
    let mut text = String::with_capacity(total + 64);
    let mut n = 0;
    while text.len() < total {
        text.push_str(&format!("let value_{n:08} = compute({n}); // note\n"));
        n += 1;
    }
    text
}

fn search(host: &mut Host, query: &str, anchor: u64, backwards: bool) -> Vec<support::Event> {
    let id = host.submit(
        abi::REQUEST_SEARCH,
        &Search {
            query: query.into(),
            source_anchor: anchor,
            backwards,
            case_sensitive: false,
        },
    );
    let events = host.finish();
    assert!(
        events
            .iter()
            .any(|e| e.id == id && e.kind == abi::EVENT_COMPLETED),
        "search did not complete: {events:?}"
    );
    events
}

#[test]
fn search_requests_each_source_byte_about_once() {
    let input = lines(1 << 20);
    let length = input.len() as u64;
    let mut host = opened(input.as_bytes(), "search.rs");
    for (anchor, backwards) in [(0, false), (length, true)] {
        let before = host.reads().len();
        let events = search(&mut host, "absent needle", anchor, backwards);
        assert!(!events.iter().any(|e| e.kind == abi::EVENT_SEARCH_MATCH));
        let bytes = requested(&host, before);
        assert!(
            bytes <= length + length / 10,
            "backwards={backwards}: requested {bytes} bytes to scan {length}"
        );
    }
}

#[test]
fn search_and_viewports_poll_host_cancellation_at_bounded_intervals() {
    let input = lines(256 * 1024);
    let mut host = opened(input.as_bytes(), "poll.rs");
    let before = host.cancel_polls();
    search(&mut host, "absent needle", 0, false);
    let polls = host.cancel_polls() - before;
    assert!(
        polls <= input.len() as u64 / 1024,
        "{polls} host cancellation polls for {} bytes",
        input.len()
    );

    // A viewport near the BOM replays about 30 KiB of XML lexical context.
    let xml = "<item name='value'>text &amp; more</item>\n".repeat(1000);
    let mut host = opened(xml.as_bytes(), "poll.xml");
    let before = host.cancel_polls();
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
    let page: TextPage = codec::decode(
        &events
            .iter()
            .find(|e| e.id == id && e.kind == abi::EVENT_TEXT_PAGE)
            .expect("XML page")
            .payload,
    )
    .unwrap();
    assert!(!page.spans.is_empty(), "replayed XML page is highlighted");
    let polls = host.cancel_polls() - before;
    assert!(
        polls <= 64,
        "{polls} host cancellation polls for one viewport"
    );
}

#[test]
fn superseded_viewports_do_not_exhaust_the_work_budget() {
    let input = lines(64 * 1024);
    let mut host = opened(input.as_bytes(), "seek.rs");
    let mut ids = Vec::new();
    let mut events = Vec::new();
    // Like a scrollbar drag on slow storage: every new viewport cancels the
    // previous one while its read is still queued at the host.
    for step in 0..8_u64 {
        if let Some(&previous) = ids.last() {
            host.cancel(previous);
        }
        ids.push(host.submit(
            abi::REQUEST_VIEWPORT,
            &Viewport {
                source_anchor: step * 4096,
                backwards: false,
                max_bytes: 12 * 1024,
                max_lines: 50,
            },
        ));
        assert!(host.run_one(), "admit viewport {step} and queue its read");
        host.drain(&mut events);
        assert!(
            !events.iter().any(|e| e.kind == abi::EVENT_FAILED),
            "viewport {step} rejected while superseded reads were pending: {events:?}"
        );
    }
    events.extend(host.finish());
    assert!(!events.iter().any(|e| e.kind == abi::EVENT_FAILED));
    let last = *ids.last().unwrap();
    assert!(
        events
            .iter()
            .any(|e| e.id == last && e.kind == abi::EVENT_TEXT_PAGE)
    );
}
