//! Safe public-API boundaries. Allocation instrumentation lives only in a private harness.
#![forbid(unsafe_code)]
#[path = "support/performance_fixtures.rs"]
mod fixtures;
#[path = "support/performance_source.rs"]
mod source;

use midnight_office_preview::{
    Cancellation, OfficeError, OfficeFormat, OfficeSession, OfficeTarget, SheetId,
};
use std::time::Instant;

#[test]
fn ten_thousand_entries_with_near_one_mib_names_open_and_render_real_cells() {
    let fixture = fixtures::ten_thousand_entries();
    assert_eq!(fixture.entries, 10_000);
    assert_eq!(fixture.cumulative_name_bytes, 1024 * 1024 - 64);
    let source = fixture.source(65_536);
    let mut session =
        OfficeSession::open(source.clone(), OfficeFormat::Xlsx, &Cancellation::default()).unwrap();
    let html = session
        .render(
            OfficeTarget::SheetFirst { sheet: SheetId(0) },
            &Cancellation::default(),
        )
        .unwrap();
    assert!(
        std::str::from_utf8(&html.bytes)
            .unwrap()
            .contains(fixtures::FIRST_TEXT)
    );
    assert!(source.counts().max_requested <= 65_536);
    eprintln!(
        "office_max_index entries={} cumulative_name_bytes={} encoded={} fixture_literal_payload_bytes={} source={:?}",
        fixture.entries,
        fixture.cumulative_name_bytes,
        fixture.tape.len(),
        fixture.tape.literal_payload_bytes(),
        source.counts()
    );
}

#[test]
fn first_far_and_revisited_deflated_windows_replay_captured_bytes() {
    // The two-string compressed SST proves member replay, not a large-SST memory bound.
    let fixture = fixtures::replay();
    let source = fixture.source(4096);
    let mut session =
        OfficeSession::open(source.clone(), OfficeFormat::Xlsx, &Cancellation::default()).unwrap();
    assert_eq!(session.catalog().sheets.len(), 2);
    let first_target = OfficeTarget::SheetFirst { sheet: SheetId(0) };
    let far_target = OfficeTarget::SheetWindow {
        sheet: SheetId(0),
        row: 1_000_000,
        column: 16_384,
    };
    let mut output = Vec::new();
    for (label, target) in [
        ("first", first_target),
        ("far", far_target),
        ("revisited", first_target),
    ] {
        let before = source.counts();
        let started = Instant::now();
        let html = session.render(target, &Cancellation::default()).unwrap();
        let elapsed = started.elapsed();
        let counts = source.counts().since(before);
        assert!(
            counts.calls > 0 && counts.payload_calls > 0 && counts.payload_delivered > 0,
            "{label} did not replay the captured worksheet deflate stream: {counts:?}"
        );
        assert!(
            counts.shared_string_calls > 0 && counts.shared_string_delivered > 0,
            "{label} did not replay the captured shared-string deflate stream: {counts:?}"
        );
        assert!(counts.max_requested <= 65_536);
        eprintln!(
            "office_public_render label={label} elapsed_ns={} source={counts:?}",
            elapsed.as_nanos()
        );
        output.push(html);
    }
    let first = std::str::from_utf8(&output[0].bytes).unwrap();
    let far = std::str::from_utf8(&output[1].bytes).unwrap();
    assert!(first.contains(fixtures::FIRST_TEXT));
    assert!(far.contains(fixtures::FAR_TEXT));
    assert_eq!(output[0].bytes.as_ref(), output[2].bytes.as_ref());
    assert!(
        output[2].generation > output[1].generation && output[1].generation > output[0].generation
    );
}

#[test]
fn cancellation_during_actual_deflate_read_stops_further_source_delivery() {
    // Trigger after real compressed payload delivery, never during metadata preflight/open.
    let fixture = fixtures::replay();
    assert!(fixture.worksheet.end - fixture.worksheet.start > 2048);
    let source = fixture.source(97);
    let mut session =
        OfficeSession::open(source.clone(), OfficeFormat::Xlsx, &Cancellation::default()).unwrap();
    let cancel = Cancellation::default();
    source.arm_cancel_after_payload_bytes(1024);
    let before = source.counts();
    let result = session.render(OfficeTarget::SheetFirst { sheet: SheetId(0) }, &cancel);
    let counts = source.counts().since(before);
    assert!(
        matches!(result, Err(OfficeError::Cancelled)),
        "expected typed cancellation, got {result:?}"
    );
    assert_eq!(counts.cancel_triggers, 1);
    assert!(counts.payload_calls > 1 && counts.payload_delivered >= 1024);
    assert_eq!(
        counts.calls_entered_cancelled, 0,
        "source called after cancellation: {counts:?}"
    );
    let stopped = source.counts();
    assert!(matches!(
        session.render(OfficeTarget::Catalog, &cancel),
        Err(OfficeError::Cancelled)
    ));
    assert_eq!(source.counts().calls, stopped.calls);
    // Indexed partial-member cleanup must not drain the unread tail on cancellation.
    assert!(counts.payload_delivered < fixture.worksheet.end - fixture.worksheet.start);
    let fresh = session
        .render(
            OfficeTarget::SheetFirst { sheet: SheetId(0) },
            &Cancellation::default(),
        )
        .unwrap();
    assert!(
        std::str::from_utf8(&fresh.bytes)
            .unwrap()
            .contains(fixtures::FIRST_TEXT)
    );
    eprintln!("office_actual_deflate_cancel source={counts:?}");
}

#[test]
#[ignore = "serial real 256-MiB work-boundary qualification; run exact with --ignored --test-threads=1"]
fn combined_encoded_and_decoded_generation_work_is_bounded() {
    // Counting only delivered encoded bytes OR only decoded bytes makes this render succeed.
    let fixture = fixtures::combined_work_limit();
    assert!(fixture.tape.len() < 256 * 1024 * 1024);
    assert!(fixture.worksheet_uncompressed < 256 * 1024 * 1024);
    assert!(fixture.tape.literal_payload_bytes() < 8 * 1024 * 1024);
    let source = fixture.source(65_536);
    let mut session =
        OfficeSession::open(source.clone(), OfficeFormat::Xlsx, &Cancellation::default()).unwrap();
    let before = source.counts();
    let started = Instant::now();
    let result = session.render(
        OfficeTarget::SheetFirst { sheet: SheetId(0) },
        &Cancellation::default(),
    );
    let elapsed = started.elapsed();
    let counts = source.counts().since(before);
    assert!(
        matches!(
            result,
            Err(OfficeError::LimitExceeded(
                "generation delivered/decompressed bytes"
            ))
        ),
        "combined work did not hit its typed bound: {result:?}"
    );
    assert!(counts.payload_delivered < fixture.worksheet.end - fixture.worksheet.start);
    assert!(counts.max_requested <= 65_536);
    assert!(
        counts.payload_delivered > 120 * 1024 * 1024,
        "fixture failed before reaching the actual combined-work boundary: {counts:?}"
    );
    // Actual source/decoder work is required after exhaustion; Catalog performs no I/O.
    // An errored decoder read can refund at most one 64-KiB reservation. Exceed
    // that residual with real small generations so a missing reset cannot pass.
    let before_recovery = source.counts();
    let mut recovered_generations = 0;
    for _ in 0..2048 {
        let before_generation = source.counts();
        let recovered = session
            .render(
                OfficeTarget::SheetFirst { sheet: SheetId(1) },
                &Cancellation::default(),
            )
            .unwrap();
        assert!(
            std::str::from_utf8(&recovered.bytes)
                .unwrap()
                .contains("Observed second")
        );
        let generation_counts = source.counts().since(before_generation);
        assert!(
            generation_counts.calls > 0 && generation_counts.delivered > 0,
            "recovery generation did not perform source work: {generation_counts:?}"
        );
        recovered_generations += 1;
        if source.counts().since(before_recovery).delivered > 65_536 {
            break;
        }
    }
    let recovery_counts = source.counts().since(before_recovery);
    assert!(
        recovery_counts.delivered > 65_536,
        "recovery stayed within potentially refunded credit: {recovery_counts:?}"
    );
    eprintln!(
        "office_combined_work elapsed_ns={} encoded_fixture={} fixture_declared_decoded={} fixture_literal_payload_bytes={} source={counts:?}",
        elapsed.as_nanos(),
        fixture.tape.len(),
        fixture.worksheet_uncompressed,
        fixture.tape.literal_payload_bytes()
    );
    eprintln!(
        "office_combined_work_recovery generations={recovered_generations} source={recovery_counts:?}"
    );
}
