//! Highlighting must survive the host's real scrolling patterns: deep forward
//! paging followed by upward paging, revisits below the latest pages, a
//! token-dense line and plain sessions that need no lexical replay.
#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_source::midnight_plugin_entry_v1 as entry;
use support::Host;

// The host asks for 12 KiB and about one window of lines per text page.
const MAX_BYTES: u32 = 12 * 1024;
const LINES: u32 = 50;

fn rust_source(lines: usize) -> String {
    (0..lines)
        .map(|n| format!("let value_{n:06} = {n:08}; // note\n"))
        .collect()
}

fn opened(input: &[u8], name: &str, mode: u16) -> Host {
    let mut host = Host::new(input, entry);
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(input.len() as u64),
            mode,
            display_name: name.into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
    host
}

fn viewport(host: &mut Host, anchor: u64, backwards: bool) -> TextPage {
    let id = host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: anchor,
            backwards,
            max_bytes: MAX_BYTES,
            max_lines: LINES,
        },
    );
    let events = host.finish();
    codec::decode(
        &events
            .iter()
            .find(|e| e.id == id && e.kind == abi::EVENT_TEXT_PAGE)
            .unwrap_or_else(|| panic!("page at {anchor}: {events:?}"))
            .payload,
    )
    .unwrap()
}

fn keywords(page: &TextPage, word: &str) -> usize {
    page.spans
        .iter()
        .filter(|s| s.role == STYLE_KEYWORD && &page.text[s.start as usize..s.end as usize] == word)
        .count()
}

/// Pages forward from `anchor` like the host, asserting colour on each page.
fn scroll_forward(host: &mut Host, mut anchor: u64, until: u64) -> u64 {
    while anchor < until {
        let page = viewport(host, anchor, false);
        assert!(
            keywords(&page, "let") > 0,
            "forward page at {anchor} lost highlighting"
        );
        anchor = page.source.end().unwrap();
    }
    anchor
}

#[test]
fn upward_pages_after_deep_forward_scrolling_stay_highlighted() {
    let input = rust_source(5000);
    let mut host = opened(input.as_bytes(), "deep.rs", MODE_SOURCE);
    let mut upper = scroll_forward(&mut host, 0, 120_000);
    // Scroll back up through about 70 KiB, far beyond the BOM replay horizon.
    for _ in 0..40 {
        let page = viewport(&mut host, upper, true);
        assert!(page.source.end().unwrap() <= upper);
        assert!(
            keywords(&page, "let") as u32 >= LINES - 2,
            "backward page ending at {upper} lost highlighting"
        );
        upper = page.source.offset;
    }
    assert!(upper < 50_000, "scrolled back to {upper}");
}

#[test]
fn revisits_below_the_latest_pages_restore_highlighting() {
    let input = rust_source(5000);
    let mut host = opened(input.as_bytes(), "revisit.rs", MODE_SOURCE);
    scroll_forward(&mut host, 0, 120_000);
    // A distant seek into the visited range finds a retained checkpoint.
    let page = viewport(&mut host, 60_000, false);
    assert!(
        keywords(&page, "let") > 0,
        "visited 60 KB lost highlighting"
    );
    // Ctrl+Home and page down again past the BOM's 32 KiB replay horizon.
    scroll_forward(&mut host, 0, 90_000);
}

#[test]
fn a_token_dense_line_does_not_disable_later_highlighting() {
    let numbers: String = (0..5000).map(|n| format!("{n},")).collect();
    let mut input = format!("const TABLE = [{numbers}];\n");
    let second_line = input.len() as u64;
    input.push_str(&"let x = 1; // c\n".repeat(4000));
    let mut host = opened(input.as_bytes(), "dense.js", MODE_SOURCE);
    // The page after the dense line replays it from the BOM checkpoint.
    let page = viewport(&mut host, second_line, false);
    assert!(
        keywords(&page, "let") > 0,
        "page after the dense line is plain"
    );
    // Checkpoints keep propagating beyond the BOM's 32 KiB replay horizon.
    scroll_forward(&mut host, second_line, 60_000);
}

#[test]
fn plain_sessions_read_only_a_lookbehind_window() {
    let input = rust_source(2000);
    for (name, mode) in [("text.rs", MODE_TEXT), ("data.unknown", MODE_SOURCE)] {
        let mut host = opened(input.as_bytes(), name, mode);
        let before = host.reads().len();
        let page = viewport(&mut host, 30_000, false);
        assert!(page.spans.is_empty());
        assert_eq!(page.source.offset, 30_000);
        let reads = host.reads();
        assert_eq!(
            reads[before].offset, 29_996,
            "{name}: no replay from the BOM without highlighting"
        );
    }
}
