#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_source::midnight_plugin_entry_v1 as entry;
use support::{Event, Host};

fn open(host: &mut Host, length: usize, revision: u64) {
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision,
            length: Some(length as u64),
            mode: MODE_SOURCE,
            display_name: "dense.cpp".into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
}

fn request(anchor: u64, backwards: bool, bytes: u32, lines: u32) -> Viewport {
    Viewport {
        source_anchor: anchor,
        backwards,
        max_bytes: bytes,
        max_lines: lines,
    }
}

fn page(events: &[Event], id: u64) -> TextPage {
    codec::decode(
        &events
            .iter()
            .find(|e| e.id == id && e.kind == abi::EVENT_TEXT_PAGE)
            .expect("page")
            .payload,
    )
    .unwrap()
}

fn styled(page: &TextPage, text: &str, role: u16) {
    let start = page.text.find(text).expect("styled text") as u32;
    let end = start + text.len() as u32;
    assert!(
        page.spans
            .iter()
            .any(|s| s.start <= start && s.end >= end && s.role == role)
    );
}

#[test]
fn dense_eviction_revisits_keep_bom_and_multiline_comment_state() {
    let input = format!("/*\n{}inside */ return 7;\r\n", "\n".repeat(32765));
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len(), 3);
    let id = host.submit(abi::REQUEST_VIEWPORT, &request(32768, false, 1, 1));
    let first = page(&host.finish(), id);
    assert_eq!(first.text, "i");
    styled(&first, "i", STYLE_COMMENT);

    let before = host.reads().len();
    let id = host.submit(abi::REQUEST_VIEWPORT, &request(32768, false, 64, 1));
    let tail = page(&host.finish(), id);
    assert_eq!(host.reads()[before].offset, 32768);
    assert_eq!(tail.text, "inside */ return 7;\n");
    styled(&tail, "inside */", STYLE_COMMENT);
    styled(&tail, "return", STYLE_KEYWORD);
    assert_eq!(
        tail.anchors.last().unwrap().source_offset,
        input.len() as u64
    );

    let before = host.reads().len();
    let id = host.submit(abi::REQUEST_VIEWPORT, &request(100, false, 1, 1));
    let revisit = page(&host.finish(), id);
    assert_eq!(
        host.reads()[before].offset,
        0,
        "evicted early lines reuse BOM state"
    );
    assert_eq!(revisit.text, "\n");
    styled(&revisit, "\n", STYLE_COMMENT);
    assert!(host.reads().iter().all(|read| read.length <= 65536));
}

#[test]
fn utf16_crlf_eviction_keeps_exact_unicode_boundaries_in_both_directions() {
    let prefix = format!("/*\r\n{}", "λ😀\r\n".repeat(400));
    let text = format!("{prefix}inside */ return 7;\r\n");
    let tail_anchor = 2 + prefix.encode_utf16().count() as u64 * 2;
    for big_endian in [false, true] {
        let mut input = if big_endian {
            vec![0xfe, 0xff]
        } else {
            vec![0xff, 0xfe]
        };
        input.extend(text.encode_utf16().flat_map(|unit| {
            if big_endian {
                unit.to_be_bytes()
            } else {
                unit.to_le_bytes()
            }
        }));
        let mut host = Host::new(&input, entry);
        host.short_reads(7);
        open(&mut host, input.len(), 31);
        let id = host.submit(abi::REQUEST_VIEWPORT, &request(2, false, 8192, 512));
        let first = page(&host.finish(), id);
        assert_eq!(first.text, text.replace("\r\n", "\n"));
        assert_eq!(
            first.anchors.last().unwrap().source_offset,
            input.len() as u64
        );
        for backwards in [false, true] {
            let anchor = if backwards {
                input.len() as u64
            } else {
                tail_anchor
            };
            let id = host.submit(abi::REQUEST_VIEWPORT, &request(anchor, backwards, 64, 1));
            let tail = page(&host.finish(), id);
            assert_eq!(tail.text, "inside */ return 7;\n");
            assert_eq!(tail.revision, 31);
            assert_eq!(tail.source.offset, tail_anchor);
            assert_eq!(
                tail.anchors.last().unwrap().source_offset,
                input.len() as u64
            );
            styled(&tail, "inside */", STYLE_COMMENT);
            styled(&tail, "return", STYLE_KEYWORD);
        }
        // The BOM state survives all 402 completed lines and later revisits.
        let before = host.reads().len();
        let id = host.submit(abi::REQUEST_VIEWPORT, &request(2, false, 64, 2));
        let revisit = page(&host.finish(), id);
        assert_eq!(host.reads()[before].offset, 2);
        assert_eq!(revisit.text, "/*\nλ😀\n");
        assert_eq!(revisit.anchors.last().unwrap().source_offset, 20);
        styled(&revisit, "λ😀", STYLE_COMMENT);
        assert!(host.reads().iter().all(|read| read.length <= 65536));
    }
}

#[test]
fn cancellation_and_close_reopen_do_not_publish_stale_checkpoint_pages() {
    let input = format!("/*\n{}inside */ return 7;\n", "\n".repeat(32765));
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len(), 3);
    let cancelled = host.submit(abi::REQUEST_VIEWPORT, &request(32768, false, 1, 1));
    assert!(host.run_one(), "start delayed read");
    host.cancel(cancelled);
    let events = host.finish();
    assert!(
        !events
            .iter()
            .any(|e| e.id == cancelled && e.kind == abi::EVENT_TEXT_PAGE)
    );
    assert!(
        events
            .iter()
            .any(|e| e.id == cancelled && e.kind == abi::EVENT_CANCELLED)
    );

    let pending = host.submit(abi::REQUEST_VIEWPORT, &request(32768, false, 1, 1));
    assert!(host.run_one(), "start read before close");
    let closed = host.submit_bytes(abi::REQUEST_CLOSE, &[]);
    let events = host.finish();
    assert!(
        events
            .iter()
            .any(|e| e.id == closed && e.kind == abi::EVENT_COMPLETED)
    );
    assert!(
        !events
            .iter()
            .any(|e| e.id == pending && e.kind == abi::EVENT_TEXT_PAGE)
    );
    open(&mut host, input.len(), 91);
    let id = host.submit(abi::REQUEST_VIEWPORT, &request(32768, false, 64, 1));
    let reopened = page(&host.finish(), id);
    assert_eq!(reopened.revision, 91);
    assert_eq!(reopened.text, "inside */ return 7;\n");
    styled(&reopened, "inside */", STYLE_COMMENT);
    styled(&reopened, "return", STYLE_KEYWORD);
    assert!(host.reads().iter().all(|read| read.length <= 65536));
}
