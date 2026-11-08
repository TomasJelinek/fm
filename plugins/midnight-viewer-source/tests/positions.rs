#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_source::midnight_plugin_entry_v1 as entry;
use support::Host;

fn open(host: &mut Host, length: u64) {
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            length: Some(length),
            revision: 3,
            mode: MODE_SOURCE,
            display_name: "test.rs".into(),
        },
    );
    assert!(
        host.finish()
            .iter()
            .any(|event| event.kind == abi::EVENT_OPENED)
    );
}

fn locate(host: &mut Host, anchor: u64) -> TextPosition {
    let id = host.submit(
        abi::REQUEST_LOCATE,
        &Locate {
            source_anchor: anchor,
        },
    );
    let events = host.finish();
    let event = events
        .iter()
        .find(|event| event.id == id && event.kind == abi::EVENT_TEXT_POSITION)
        .unwrap_or_else(|| panic!("missing position: {events:?}"));
    assert!(
        events
            .iter()
            .any(|event| event.id == id && event.kind == abi::EVENT_COMPLETED)
    );
    codec::decode(&event.payload).unwrap()
}

#[test]
fn unicode_columns_and_crlf_are_exact_with_short_reads_and_reverse_queries() {
    let text = "a\t文e\u{301}🙂\r\nb\rc\n\u{2028}d\u{2029}e";
    for utf16 in [false, true] {
        let bytes = if utf16 {
            [
                vec![0xff, 0xfe],
                text.encode_utf16().flat_map(u16::to_le_bytes).collect(),
            ]
            .concat()
        } else {
            [vec![0xef, 0xbb, 0xbf], text.as_bytes().to_vec()].concat()
        };
        let mut host = Host::new(&bytes, entry);
        host.short_reads(3);
        open(&mut host, bytes.len() as u64);
        let offset = |prefix: &str| {
            if utf16 {
                2 + prefix.encode_utf16().count() as u64 * 2
            } else {
                3 + prefix.len() as u64
            }
        };
        for (prefix, line, column) in [
            (text, 6, 2),
            ("a\t文e\u{301}🙂", 1, 7),
            ("a\t文e\u{301}🙂\r\nb", 2, 2),
            ("", 1, 1),
            ("a\t文e\u{301}", 1, 6),
        ] {
            let result = locate(&mut host, offset(prefix));
            assert_eq!(
                (
                    result.revision,
                    result.source_offset,
                    result.line,
                    result.column
                ),
                (3, offset(prefix), line, column)
            );
        }
        let cr = offset("a\t文e\u{301}🙂");
        let between = cr + if utf16 { 2 } else { 1 };
        let result = locate(&mut host, between);
        assert_eq!(
            (result.source_offset, result.line, result.column),
            (offset("a\t文e\u{301}🙂\r\n"), 2, 1)
        );
    }
}

#[test]
fn malformed_scalars_controls_and_partial_characters_keep_source_boundaries() {
    for (bytes, anchor, expected) in [
        (vec![b'a', 0xff, 0xe2, b'\t', 0, b'\n', b'z'], 5, (5, 1, 6)),
        (
            vec![0xff, 0xfe, b'a', 0, 0, 0xd8, b'\n', 0, b'z'],
            8,
            (8, 2, 1),
        ),
        ("a🙂z".as_bytes().to_vec(), 3, (5, 1, 3)),
    ] {
        let mut host = Host::new(&bytes, entry);
        open(&mut host, bytes.len() as u64);
        let result = locate(&mut host, anchor);
        assert_eq!((result.source_offset, result.line, result.column), expected);
    }
}

#[test]
fn viewport_completes_while_distant_position_is_still_scanning_and_cancel_stops_reads() {
    let bytes = b"let line = 1;\r\n".repeat(100_000);
    let mut host = Host::new(&bytes, entry);
    open(&mut host, bytes.len() as u64);
    let locate = host.submit(
        abi::REQUEST_LOCATE,
        &Locate {
            source_anchor: bytes.len() as u64,
        },
    );
    assert!(host.run_one());
    let viewport = host.submit(
        abi::REQUEST_VIEWPORT,
        &Viewport {
            source_anchor: bytes.len() as u64 - 1024,
            backwards: false,
            max_bytes: 512,
            max_lines: 20,
        },
    );
    let mut events = Vec::new();
    for _ in 0..30 {
        assert!(host.run_one());
        host.drain(&mut events);
        if events
            .iter()
            .any(|event| event.id == viewport && event.kind == abi::EVENT_COMPLETED)
        {
            break;
        }
    }
    assert!(
        events
            .iter()
            .any(|event| event.id == viewport && event.kind == abi::EVENT_TEXT_PAGE)
    );
    assert!(
        events
            .iter()
            .any(|event| event.id == viewport && event.kind == abi::EVENT_COMPLETED)
    );
    assert!(
        !events
            .iter()
            .any(|event| event.id == locate && event.kind == abi::EVENT_TEXT_POSITION)
    );
    let reads_before = host.reads().len();
    host.cancel(locate);
    events.extend(host.finish());
    assert!(
        !events
            .iter()
            .any(|event| event.id == locate && event.kind == abi::EVENT_TEXT_POSITION)
    );
    assert!(
        events
            .iter()
            .any(|event| event.id == locate && event.kind == abi::EVENT_CANCELLED)
    );
    assert!(
        host.reads().len() <= reads_before + 1,
        "cancelled index continued reading"
    );
    assert!(host.reads().iter().all(|read| read.length <= 32768));
}

#[test]
fn repeated_distant_lookup_uses_checkpoint_and_out_of_range_fails_without_read() {
    let bytes = b"hello\n".repeat(30_000);
    let mut host = Host::new(&bytes, entry);
    open(&mut host, bytes.len() as u64);
    let first = locate(&mut host, bytes.len() as u64);
    assert_eq!((first.line, first.column), (30_001, 1));
    let before = host.reads().len();
    assert_eq!(locate(&mut host, bytes.len() as u64), first);
    assert!(host.reads().len() - before <= 1);
    let id = host.submit(
        abi::REQUEST_LOCATE,
        &Locate {
            source_anchor: bytes.len() as u64 + 1,
        },
    );
    let before = host.reads().len();
    let events = host.finish();
    assert!(
        events
            .iter()
            .any(|event| event.id == id && event.kind == abi::EVENT_FAILED)
    );
    assert_eq!(host.reads().len(), before);
}
