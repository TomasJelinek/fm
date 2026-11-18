#[allow(dead_code)]
mod support;
use midnight_plugin_sdk::api::{abi, codec, messages::*, viewer::*};
use midnight_viewer_source::midnight_plugin_entry_v1 as entry;
use support::{Event, Host};

fn open(host: &mut Host, length: usize, name: &str, mode: u16) {
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: Some(length as u64),
            mode,
            display_name: name.into(),
        },
    );
    assert!(host.finish().iter().any(|e| e.kind == abi::EVENT_OPENED));
}

fn viewport(anchor: u64, backwards: bool) -> Viewport {
    Viewport {
        source_anchor: anchor,
        backwards,
        max_bytes: 512,
        max_lines: 8,
    }
}

fn prepared(events: &[Event], id: u64) -> SourcePrepared {
    let kinds: Vec<_> = events
        .iter()
        .filter(|e| e.id == id)
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        kinds,
        [abi::EVENT_SOURCE_PREPARED, abi::EVENT_COMPLETED],
        "{events:?}"
    );
    codec::decode(
        &events
            .iter()
            .find(|e| e.id == id && e.kind == abi::EVENT_SOURCE_PREPARED)
            .unwrap()
            .payload,
    )
    .unwrap()
}

fn prepare(host: &mut Host, target: &Viewport, bytes: u32) -> SourcePrepared {
    let id = host.submit(
        abi::REQUEST_PREPARE_SOURCE,
        &PrepareSource {
            revision: 3,
            viewport: target.clone(),
            max_scan_bytes: bytes,
        },
    );
    prepared(&host.finish(), id)
}

fn page(host: &mut Host, target: &Viewport) -> TextPage {
    let id = host.submit(abi::REQUEST_VIEWPORT, target);
    let events = host.finish();
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
    let at = page.text.find(text).expect("visible token") as u32;
    assert!(
        page.spans
            .iter()
            .any(|s| s.start <= at && s.end >= at + text.len() as u32 && s.role == role),
        "{page:?}"
    );
}

fn until_ready(
    host: &mut Host,
    target: &Viewport,
    bytes: u32,
    short: usize,
    length: usize,
) -> SourcePrepared {
    let mut frontier = 0;
    for _ in 0..2048 {
        let before = host.reads().len();
        let result = prepare(host, target, bytes);
        assert!(
            result.frontier >= frontier,
            "one target's cursor must not regress"
        );
        frontier = result.frontier;
        let reads = host.reads();
        let reads = &reads[before..];
        assert!(reads.len() <= 64, "service-reply cap counts short reads");
        assert!(reads.iter().all(|r| r.length <= 16384));
        let delivered: u64 = reads
            .iter()
            .map(|r| {
                u64::from(r.length)
                    .min(short as u64)
                    .min((length as u64).saturating_sub(r.offset))
            })
            .sum();
        assert!(
            delivered <= u64::from(bytes),
            "delivered={delivered}, budget={bytes}"
        );
        if result.ready {
            return result;
        }
    }
    panic!("preparation exhausted its bounded test quanta");
}

#[test]
fn cold_multiline_target_uses_explicit_bounded_preparation_before_exact_refresh() {
    let input = format!(
        "/*\n{}inside */ return 42;\n",
        "comment line\n".repeat(12000)
    );
    let target = viewport(input.find("inside").unwrap() as u64, false);
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len(), "recovery.cpp", MODE_SOURCE);
    assert!(
        page(&mut host, &target).spans.is_empty(),
        "cold foreground stays bounded"
    );
    let mut frontier = 0;
    for quantum in 0..32 {
        let before = host.reads().len();
        let result = prepare(&mut host, &target, 8192);
        assert_eq!(result.revision, 3);
        assert!(result.frontier >= frontier);
        frontier = result.frontier;
        let reads = host.reads();
        assert!(reads[before..].len() <= 64);
        assert!(reads[before..].iter().all(|r| r.length <= 16384));
        assert!(
            reads[before..]
                .iter()
                .map(|r| u64::from(r.length))
                .sum::<u64>()
                <= 8192
        );
        if result.ready {
            assert!(quantum > 0);
            let page = page(&mut host, &target);
            styled(&page, "inside */", STYLE_COMMENT);
            styled(&page, "return", STYLE_KEYWORD);
            return;
        }
    }
    panic!("bounded quanta never made the target ready");
}

#[test]
fn prepare_rejects_wrong_revision_bounds_text_and_unknown_language() {
    for (name, mode, expected) in [
        ("x.cpp", MODE_SOURCE, abi::INVALID),
        ("x.cpp", MODE_TEXT, abi::UNSUPPORTED),
        ("x.unknown", MODE_SOURCE, abi::UNSUPPORTED),
    ] {
        let mut host = Host::new(b"return 1;\n", entry);
        open(&mut host, 10, name, mode);
        let id = host.submit(
            abi::REQUEST_PREPARE_SOURCE,
            &PrepareSource {
                revision: if expected == abi::INVALID { 99 } else { 3 },
                viewport: viewport(0, false),
                max_scan_bytes: 4096,
            },
        );
        let events = host.finish();
        let failure: Failure = codec::decode(
            &events
                .iter()
                .find(|e| e.id == id && e.kind == abi::EVENT_FAILED)
                .expect("failure")
                .payload,
        )
        .unwrap();
        assert_eq!(failure.code, expected);
        assert!(
            !events
                .iter()
                .any(|e| e.id == id && e.kind == abi::EVENT_SOURCE_PREPARED)
        );
    }
    let mut host = Host::new(b"return 1;\n", entry);
    open(&mut host, 10, "x.cpp", MODE_SOURCE);
    let id = host.submit(
        abi::REQUEST_PREPARE_SOURCE,
        &PrepareSource {
            revision: 3,
            viewport: viewport(11, false),
            max_scan_bytes: 1,
        },
    );
    let events = host.finish();
    let failure: Failure = codec::decode(
        &events
            .iter()
            .find(|e| e.id == id && e.kind == abi::EVENT_FAILED)
            .unwrap()
            .payload,
    )
    .unwrap();
    assert_eq!(failure.code, abi::INVALID);
}

#[test]
fn multi_megabyte_raw_strings_recover_forward_backward_eof_and_distributed_revisits() {
    let prefix = "return R\"tag(\n";
    let body = "/* raw body */ \\\" λ😀\r\n".repeat(100000);
    let input = format!("{prefix}{body})tag\"; return 42;\n");
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len(), "large.cpp", MODE_SOURCE);
    for anchor in [1_900_000, 200_000, 1_200_000, input.len() as u64, 750_000] {
        let backwards = anchor == input.len() as u64 || anchor == 200_000;
        let target = viewport(anchor, backwards);
        until_ready(&mut host, &target, 262144, usize::MAX, input.len());
        let actual = page(&mut host, &target);
        if anchor == input.len() as u64 {
            styled(&actual, "return", STYLE_KEYWORD);
        } else {
            styled(&actual, "raw body", STYLE_STRING);
        }
        assert!(actual.text.len() <= 512);
        assert!(
            actual
                .anchors
                .windows(2)
                .all(|a| a[0].source_offset < a[1].source_offset)
        );
    }
}

#[test]
fn one_byte_replies_preserve_bom_utf8_utf16_crlf_and_decoder_tails_between_quanta() {
    let prefix = format!("/*\r\n{}", "λ😀\r\n".repeat(90));
    let text = format!("{prefix}inside */ return 7;\r\n");
    for encoding in 0..3 {
        let mut input = match encoding {
            0 => vec![0xef, 0xbb, 0xbf],
            1 => vec![0xff, 0xfe],
            _ => vec![0xfe, 0xff],
        };
        let anchor = if encoding == 0 {
            input.extend_from_slice(text.as_bytes());
            3 + prefix.len() as u64
        } else {
            input.extend(text.encode_utf16().flat_map(|unit| {
                if encoding == 1 {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                }
            }));
            2 + prefix.encode_utf16().count() as u64 * 2
        };
        let mut host = Host::new(&input, entry);
        host.short_reads(1);
        open(&mut host, input.len(), "boundary.cpp", MODE_SOURCE);
        let target = viewport(anchor, false);
        let before = host.reads().len();
        until_ready(&mut host, &target, 64, 1, input.len());
        let reads = host.reads();
        assert!(
            reads[before..]
                .windows(2)
                .all(|r| r[1].offset == r[0].offset + 1),
            "received tails must not be reread: encoding={encoding}"
        );
        let actual = page(&mut host, &target);
        assert_eq!(actual.text, "inside */ return 7;\n");
        assert_eq!(actual.source.offset, anchor);
        assert_eq!(
            actual.anchors.last().unwrap().source_offset,
            input.len() as u64
        );
        styled(&actual, "inside */", STYLE_COMMENT);
        styled(&actual, "return", STYLE_KEYWORD);
    }
}

#[test]
fn far_future_roles_are_reserved_as_tiny_hints_for_the_actual_viewport() {
    for (name, input, anchor, needle, role) in [
        (
            "key.yaml",
            format!("{}: value\n", "a".repeat(300000)),
            100000,
            "aaa",
            STYLE_TYPE,
        ),
        (
            "quoted.yaml",
            format!("\"{}\": value\n", "a".repeat(300000)),
            100000,
            "aaa",
            STYLE_TYPE,
        ),
        (
            "header.yaml",
            format!("key: |{}\n  content\n", " ".repeat(300000)),
            5,
            "|",
            STYLE_OPERATOR,
        ),
        (
            "indent.yaml",
            format!("key: |\n{}body\n", " ".repeat(300000)),
            7,
            "   ",
            STYLE_STRING,
        ),
        (
            "entity.xml",
            format!("&{}; tail\n", "a".repeat(300000)),
            100000,
            "aaa",
            STYLE_OPERATOR,
        ),
        (
            "opener.lua",
            format!("[{}[body]{}]\n", "=".repeat(300000), "=".repeat(300000)),
            100000,
            "===",
            STYLE_STRING,
        ),
    ] {
        for backwards in [false, true] {
            let anchor = if backwards { anchor + 512 } else { anchor };
            let target = viewport(anchor, backwards);
            let mut host = Host::new(input.as_bytes(), entry);
            open(&mut host, input.len(), name, MODE_SOURCE);
            let result = until_ready(&mut host, &target, 8192, usize::MAX, input.len());
            assert!(
                result.frontier >= 300000,
                "{name}: numeric target alone cannot prove a future role"
            );
            let actual = page(&mut host, &target);
            styled(&actual, needle, role);
        }
    }
}

#[test]
fn malformed_unterminated_future_roles_finish_at_eof_without_endless_not_ready() {
    for (name, input, expected) in [
        ("entity.xml", format!("&{}", "a".repeat(90000)), None),
        ("opener.lua", format!("[{}", "=".repeat(90000)), None),
        ("scalar.yaml", "a".repeat(90000), Some(STYLE_STRING)),
    ] {
        let target = viewport(1000, false);
        let mut host = Host::new(input.as_bytes(), entry);
        open(&mut host, input.len(), name, MODE_SOURCE);
        let result = until_ready(&mut host, &target, 4096, usize::MAX, input.len());
        assert_eq!(result.frontier, input.len() as u64);
        let actual = page(&mut host, &target);
        if let Some(role) = expected {
            styled(&actual, "aaa", role);
        } else {
            assert!(actual.spans.is_empty(), "{name}: {actual:?}");
        }
    }
}

#[test]
fn cancellation_close_zero_progress_and_same_session_queue_do_not_publish_readiness() {
    let input = format!("/*\n{}*/ return 1;\n", "comment\n".repeat(20000));
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len(), "cancel.cpp", MODE_SOURCE);
    let target = viewport(100000, false);
    let request = PrepareSource {
        revision: 3,
        viewport: target.clone(),
        max_scan_bytes: 262144,
    };
    let cancelled = host.submit(abi::REQUEST_PREPARE_SOURCE, &request);
    assert!(host.run_one());
    host.cancel(cancelled);
    let events = host.finish();
    assert!(
        events
            .iter()
            .any(|e| e.id == cancelled && e.kind == abi::EVENT_CANCELLED)
    );
    assert!(
        !events
            .iter()
            .any(|e| e.id == cancelled && e.kind == abi::EVENT_SOURCE_PREPARED)
    );

    let active = host.submit(abi::REQUEST_PREPARE_SOURCE, &request);
    let queued = host.submit(abi::REQUEST_PREPARE_SOURCE, &request);
    let events = host.finish();
    prepared(&events, active);
    let failure: Failure = codec::decode(
        &events
            .iter()
            .find(|e| e.id == queued && e.kind == abi::EVENT_FAILED)
            .unwrap()
            .payload,
    )
    .unwrap();
    assert_eq!(failure.code, abi::QUEUE_FULL);

    host.short_reads(0);
    let different = PrepareSource {
        viewport: viewport(130000, false),
        ..request.clone()
    };
    let zero = host.submit(abi::REQUEST_PREPARE_SOURCE, &different);
    let events = host.finish();
    let failure: Failure = codec::decode(
        &events
            .iter()
            .find(|e| e.id == zero && e.kind == abi::EVENT_FAILED)
            .unwrap()
            .payload,
    )
    .unwrap();
    assert_eq!(failure.code, abi::INVALID);
    assert!(
        !events
            .iter()
            .any(|e| e.id == zero && e.kind == abi::EVENT_SOURCE_PREPARED)
    );
    host.short_reads(127);
    let pending = host.submit(abi::REQUEST_PREPARE_SOURCE, &different);
    assert!(host.run_one());
    host.submit_bytes(abi::REQUEST_CLOSE, &[]);
    let events = host.finish();
    assert!(
        !events
            .iter()
            .any(|e| e.id == pending && e.kind == abi::EVENT_SOURCE_PREPARED)
    );
}

#[test]
fn ready_reservation_survives_foreground_pressure_and_latest_prepare_supersedes_it() {
    let input = format!(
        "/*\n{}{}*/ return 8;\n",
        "\n".repeat(40000),
        "comment ".repeat(20000)
    );
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len(), "reservation.cpp", MODE_SOURCE);
    let target = viewport(85000, false);
    until_ready(&mut host, &target, 262144, usize::MAX, input.len());
    // A different viewport must not consume the target. Its replay inserts
    // thousands of checkpoints while the target remains separately reserved.
    page(
        &mut host,
        &Viewport {
            source_anchor: 30000,
            backwards: false,
            max_bytes: 1,
            max_lines: 1,
        },
    );
    styled(&page(&mut host, &target), "comment", STYLE_COMMENT);

    let first = viewport(95000, false);
    let latest = viewport(input.len() as u64, true);
    until_ready(&mut host, &first, 262144, usize::MAX, input.len());
    until_ready(&mut host, &latest, 262144, usize::MAX, input.len());
    let actual = page(&mut host, &latest);
    styled(&actual, "return", STYLE_KEYWORD);
}

#[test]
fn prepared_snapshot_never_passes_a_target_inside_a_held_keyword_or_giant_identifier() {
    for (input, anchor, needle, role) in [
        (
            format!("{}; return 1;\n", "a".repeat(90000)),
            10,
            "aaa",
            STYLE_PLAIN,
        ),
        ("return 1;\n".into(), 0, "return", STYLE_KEYWORD),
        ("return 1;\n".into(), 3, "urn", STYLE_KEYWORD),
        (
            "return R\"tag(hello)tag\";\n".into(),
            11,
            "g(hello",
            STYLE_STRING,
        ),
    ] {
        let mut host = Host::new(input.as_bytes(), entry);
        open(&mut host, input.len(), "held.cpp", MODE_SOURCE);
        let target = viewport(anchor, false);
        until_ready(&mut host, &target, 4096, usize::MAX, input.len());
        let actual = page(&mut host, &target);
        assert_eq!(actual.source.offset, anchor);
        if role == STYLE_PLAIN {
            assert!(actual.spans.is_empty());
        } else {
            styled(&actual, needle, role);
        }
    }
}

#[test]
fn pending_role_resolution_between_visible_lookahead_and_raw_read_end_is_retained() {
    let input = format!("{}: value\n", "a".repeat(1000));
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len(), "lookahead.yaml", MODE_SOURCE);
    let target = viewport(0, false);
    until_ready(&mut host, &target, 4096, usize::MAX, input.len());
    styled(&page(&mut host, &target), "aaa", STYLE_TYPE);
}

#[test]
fn backward_control_expansion_fits_the_prepared_context_and_keeps_exact_styles() {
    let mut input = b"/*".to_vec();
    input.resize(2 * 1024 * 1024, 0);
    input.extend_from_slice(b"*/ return 9;\n");
    let mut host = Host::new(&input, entry);
    open(&mut host, input.len(), "controls.cpp", MODE_SOURCE);
    let target = Viewport {
        source_anchor: 1_900_000,
        backwards: true,
        max_bytes: 8192,
        max_lines: 4096,
    };
    until_ready(&mut host, &target, 262144, usize::MAX, input.len());
    let actual = page(&mut host, &target);
    assert!(!actual.text.is_empty());
    assert!(actual.text.len() <= 8192);
    assert_eq!(
        actual.anchors.last().unwrap().source_offset,
        target.source_anchor
    );
    assert!(
        actual
            .spans
            .iter()
            .any(|s| s.start == 0 && s.end == actual.text.len() as u32 && s.role == STYLE_COMMENT)
    );
    assert!(host.reads().iter().all(|r| r.length <= 65536));
}

#[test]
fn foreground_delivery_has_a_work_slot_when_preparation_and_locate_are_pending() {
    let input = format!("/*\n{}*/ return 1;\n", "comment\n".repeat(30000));
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len(), "slots.cpp", MODE_SOURCE);
    let prepare = host.submit(
        abi::REQUEST_PREPARE_SOURCE,
        &PrepareSource {
            revision: 3,
            viewport: viewport(200000, false),
            max_scan_bytes: 262144,
        },
    );
    host.submit(
        abi::REQUEST_LOCATE,
        &Locate {
            source_anchor: 20000,
        },
    );
    host.submit(
        abi::REQUEST_LOCATE,
        &Locate {
            source_anchor: 20000,
        },
    );
    let view = host.submit(abi::REQUEST_VIEWPORT, &viewport(3, false));
    let events = host.finish();
    prepared(&events, prepare);
    let actual: TextPage = codec::decode(
        &events
            .iter()
            .find(|e| e.id == view && e.kind == abi::EVENT_TEXT_PAGE)
            .expect("foreground reserved capacity")
            .payload,
    )
    .unwrap();
    styled(&actual, "comment", STYLE_COMMENT);
}

#[test]
fn prepared_reservation_does_not_survive_close_and_a_new_revision() {
    let input = format!("/*\n{}*/ return 1;\n", "comment\n".repeat(20000));
    let target = viewport(100000, false);
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len(), "revision.cpp", MODE_SOURCE);
    until_ready(&mut host, &target, 262144, usize::MAX, input.len());
    host.submit_bytes(abi::REQUEST_CLOSE, &[]);
    host.finish();
    host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 4,
            length: Some(input.len() as u64),
            mode: MODE_SOURCE,
            display_name: "revision.cpp".into(),
        },
    );
    host.finish();
    let actual = page(&mut host, &target);
    assert_eq!(actual.revision, 4);
    assert!(
        actual.spans.is_empty(),
        "a new session must not reuse the previous revision's target"
    );
}

#[test]
fn unknown_length_is_preserved_and_tiny_raw_string_viewports_are_preflighted() {
    let input = b"return R\"sixteen_chars___(body)sixteen_chars___\";\n";
    let mut host = Host::new(input, entry);
    let id = host.submit(
        abi::REQUEST_OPEN,
        &Open {
            reader: 1,
            revision: 3,
            length: None,
            mode: MODE_SOURCE,
            display_name: "unknown.cpp".into(),
        },
    );
    let events = host.finish();
    let opened: Opened = codec::decode(
        &events
            .iter()
            .find(|e| e.id == id && e.kind == abi::EVENT_OPENED)
            .unwrap()
            .payload,
    )
    .unwrap();
    assert_eq!(opened.length, None);
    let target = Viewport {
        source_anchor: 7,
        backwards: false,
        max_bytes: 1,
        max_lines: 1,
    };
    until_ready(&mut host, &target, 64, usize::MAX, input.len());
    let actual = page(&mut host, &target);
    assert_eq!(actual.text, "R");
    styled(&actual, "R", STYLE_STRING);
}

#[test]
fn lexical_state_exhaustion_is_an_explicit_failure_without_false_readiness() {
    let input = format!("{}body{}\n", "/*".repeat(17), "*/".repeat(17));
    let mut host = Host::new(input.as_bytes(), entry);
    open(&mut host, input.len(), "nested.rs", MODE_SOURCE);
    let id = host.submit(
        abi::REQUEST_PREPARE_SOURCE,
        &PrepareSource {
            revision: 3,
            viewport: viewport(40, false),
            max_scan_bytes: 4096,
        },
    );
    let events = host.finish();
    assert!(
        !events
            .iter()
            .any(|e| e.id == id && e.kind == abi::EVENT_SOURCE_PREPARED)
    );
    let failure: Failure = codec::decode(
        &events
            .iter()
            .find(|e| e.id == id && e.kind == abi::EVENT_FAILED)
            .expect("state budget failure")
            .payload,
    )
    .unwrap();
    assert_eq!(failure.code, abi::INVALID);
}

#[test]
fn delivered_byte_budget_and_short_reply_cap_independently_end_a_quantum() {
    let input = format!("/*{}*/\n", "x".repeat(400000));
    for (short, budget, expected_replies, expected_bytes) in [
        (1, 262144, 64, 64),
        (127, 262144, 64, 8128),
        (127, 1000, 8, 1000),
    ] {
        let mut host = Host::new(input.as_bytes(), entry);
        open(&mut host, input.len(), "reply-cap.cpp", MODE_SOURCE);
        host.short_reads(short);
        let target = viewport(200000, false);
        let before = host.reads().len();
        let first = prepare(&mut host, &target, budget);
        assert!(!first.ready);
        let reads = host.reads();
        let first_reads = &reads[before..];
        assert_eq!(first_reads.len(), expected_replies);
        let delivered: u64 = first_reads
            .iter()
            .map(|read| u64::from(read.length).min(short as u64))
            .sum();
        assert_eq!(delivered, expected_bytes);
        let next_offset = first_reads.last().unwrap().offset
            + u64::from(first_reads.last().unwrap().length).min(short as u64);
        let before = reads.len();
        let second = prepare(&mut host, &target, budget);
        assert!(!second.ready);
        assert!(second.frontier >= first.frontier);
        assert_eq!(
            host.reads()[before].offset,
            next_offset,
            "received short tails must be retained, not reread"
        );
    }
}

#[test]
fn malformed_units_and_mid_scalar_targets_keep_exact_prepared_source_boundaries() {
    let mut utf8 = b"\xef\xbb\xbf/*\r\n".to_vec();
    utf8.extend_from_slice("λ😀".as_bytes());
    utf8.extend_from_slice(b"\xff\r\n*/ return 1;\r\n");
    let mut inputs = vec![utf8];
    for big_endian in [false, true] {
        let mut input = if big_endian {
            vec![0xfe, 0xff]
        } else {
            vec![0xff, 0xfe]
        };
        input.extend(
            "/*\r\nλ😀\r\n*/ return 1;\r\n"
                .encode_utf16()
                .flat_map(|unit| {
                    if big_endian {
                        unit.to_be_bytes()
                    } else {
                        unit.to_le_bytes()
                    }
                }),
        );
        input.push(0xff);
        inputs.push(input);
    }
    for input in inputs {
        let (encoding, bom) = midnight_viewer_source::decode::encoding(&input);
        let whole = midnight_viewer_source::decode::decode(
            &input[bom..],
            bom as u64,
            encoding,
            true,
            65536,
            usize::MAX,
            || false,
        )
        .unwrap();
        for anchor in [
            bom as u64 + 1,
            bom as u64 + 7,
            bom as u64 + 9,
            input.len() as u64 - 1,
        ] {
            for backwards in [false, true] {
                let mut host = Host::new(&input, entry);
                open(&mut host, input.len(), "units.cpp", MODE_SOURCE);
                host.short_reads(1);
                let target = viewport(anchor, backwards);
                until_ready(&mut host, &target, 64, 1, input.len());
                let actual = page(&mut host, &target);
                let boundary = if backwards {
                    whole
                        .anchors
                        .iter()
                        .rev()
                        .find(|a| a.source_offset <= anchor)
                        .unwrap()
                } else {
                    whole
                        .anchors
                        .iter()
                        .find(|a| a.source_offset >= anchor)
                        .unwrap()
                };
                if backwards {
                    assert_eq!(
                        actual.anchors.last().unwrap().source_offset,
                        boundary.source_offset
                    );
                    assert_eq!(actual.text, whole.text[..boundary.text_offset as usize]);
                } else {
                    assert_eq!(actual.source.offset, boundary.source_offset);
                    assert_eq!(actual.text, whole.text[boundary.text_offset as usize..]);
                }
            }
        }
    }
}

#[test]
fn unrelated_lookahead_tokens_do_not_delay_the_actual_visible_roles() {
    let unrelated = "z".repeat(3 * 1024 * 1024);
    let whitespaces = " ".repeat(3 * 1024 * 1024);
    let mut failures = Vec::new();
    for (name, input, target, expected, role) in [
        (
            "forward-byte",
            format!("a\n{unrelated}"),
            Viewport {
                source_anchor: 0,
                backwards: false,
                max_bytes: 1,
                max_lines: 1,
            },
            "a",
            Some(STYLE_STRING),
        ),
        (
            "forward-line",
            format!("a\n{unrelated}"),
            Viewport {
                source_anchor: 0,
                backwards: false,
                max_bytes: 8192,
                max_lines: 1,
            },
            "a\n",
            Some(STYLE_STRING),
        ),
        (
            "backward-byte",
            format!("a\n{unrelated}"),
            Viewport {
                source_anchor: 1,
                backwards: true,
                max_bytes: 1,
                max_lines: 1,
            },
            "a",
            Some(STYLE_STRING),
        ),
        (
            "backward-line",
            format!("a\n{unrelated}"),
            Viewport {
                source_anchor: 2,
                backwards: true,
                max_bytes: 8192,
                max_lines: 1,
            },
            "a\n",
            Some(STYLE_STRING),
        ),
        (
            "backward-whitespace",
            format!("        a{whitespaces}: value\n"),
            Viewport {
                source_anchor: 32,
                backwards: true,
                max_bytes: 8,
                max_lines: 1,
            },
            "        ",
            None,
        ),
        (
            "block-lookahead",
            format!("key: |\n{whitespaces}body\n"),
            Viewport {
                source_anchor: 0,
                backwards: false,
                max_bytes: 1,
                max_lines: 1,
            },
            "k",
            Some(STYLE_TYPE),
        ),
        (
            "empty-scalar-window",
            format!("{}: value\n", "😀".repeat(786432)),
            Viewport {
                source_anchor: 4,
                backwards: false,
                max_bytes: 1,
                max_lines: 1,
            },
            "",
            None,
        ),
    ] {
        let mut host = Host::new(input.as_bytes(), entry);
        open(&mut host, input.len(), "unrelated.yaml", MODE_SOURCE);
        let before = host.reads().len();
        let result = prepare(&mut host, &target, 262144);
        let reads = host.reads().len() - before;
        println!(
            "unrelated_case={name} ready={} preparation_reads={reads} frontier={}",
            result.ready, result.frontier
        );
        if !result.ready || reads != 1 {
            failures.push(format!(
                "{name}: ready={}, preparation_reads={reads}",
                result.ready
            ));
            continue;
        }
        let actual = page(&mut host, &target);
        assert_eq!(actual.text, expected);
        if let Some(role) = role {
            styled(&actual, expected.trim_end(), role);
        } else {
            assert!(actual.spans.is_empty());
        }
    }
    assert!(
        failures.is_empty(),
        "unrelated pending roles delayed visible pages: {failures:?}"
    );
}
