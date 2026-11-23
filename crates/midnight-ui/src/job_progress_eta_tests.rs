use super::*;
use midnight_core::{
    DeleteMode, MutationKind, OperationId, OperationThroughput, TransferIntent,
    TransferRemainingWork, TransferVerification,
};
use std::sync::Arc;

/// Thousands grouped as "124,000".
fn grouped() -> NumberLocale {
    NumberLocale {
        group_separator: ",".into(),
        grouping: vec![3],
        repeat_last_group: true,
        ..Default::default()
    }
}

fn transfer(intent: TransferIntent, verification: TransferVerification) -> CopyProgress {
    let mut p = CopyProgress::queued(OperationId(1), "target".into(), 1, intent);
    p.state = CopyOperationState::Running;
    p.verification = verification;
    p.discovery = OperationDiscovery {
        state: DiscoveryState::Complete,
        files: 1,
        bytes: Some(1_000_000),
        ..Default::default()
    };
    p.throughput = Arc::new(OperationThroughput {
        smoothed_verification_per_second: 65_536.0,
        ..Default::default()
    });
    p
}
#[test]
fn sampled_copy_estimates_actual_readback_and_not_the_entire_file_again() {
    let mut p = transfer(TransferIntent::Copy, TransferVerification::SizeAndSamples);
    p.current_file_copied = 1_000_000;
    p.remaining_work = Some(TransferRemainingWork {
        write_bytes: 0,
        verification_bytes: 196_608,
    });
    assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 3 s");
    p.current_file_verified = 65_536;
    p.remaining_work.as_mut().unwrap().verification_bytes = 131_072;
    assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 2 s");
    p.remaining_work = None;
    assert!(copy(&p, &NumberLocale::default()).eta.is_empty());
}
#[test]
fn move_estimates_outstanding_proofs_after_destination_publication() {
    for policy in [
        TransferVerification::FullSha256,
        TransferVerification::SizeAndSamples,
    ] {
        let mut p = transfer(TransferIntent::Move, policy);
        let readback = match policy {
            TransferVerification::FullSha256 => 1_000_000,
            TransferVerification::SizeAndSamples => 196_608,
        };
        Arc::make_mut(&mut p.throughput).smoothed_verification_per_second = readback as f64;
        p.completed_bytes = 1_000_000;
        p.files_completed = 1;
        p.remaining_work = Some(TransferRemainingWork {
            write_bytes: 0,
            verification_bytes: 4 * readback,
        });
        assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 4 s");
        p.remaining_work.as_mut().unwrap().verification_bytes = readback / 2;
        assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 1 s");
        // Retried finalization repeats the complete source/destination After pass.
        p.validation_bytes = 9_000_000;
        p.remaining_work.as_mut().unwrap().verification_bytes = 2 * readback;
        assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 2 s");
        assert_eq!(
            copy(&p, &NumberLocale::default()).remaining,
            "Items remaining: 1"
        );
    }
}
#[test]
fn unavailable_work_errors_and_nonfinite_rates_never_fabricate_eta_and_a_pause_holds_it() {
    let mut p = transfer(TransferIntent::Move, TransferVerification::SizeAndSamples);
    p.remaining_work = Some(TransferRemainingWork {
        write_bytes: 10,
        verification_bytes: 20,
    });
    for state in [
        CopyOperationState::WaitingForDecision,
        CopyOperationState::CancelRequested,
        CopyOperationState::Completed,
    ] {
        p.state = state;
        assert!(copy(&p, &NumberLocale::default()).eta.is_empty());
    }
    p.state = CopyOperationState::Running;
    p.paused = true;
    assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 1 s (paused)");
    p.paused = false;
    // A skipped or failed item leaves an approximate estimate, marked so.
    p.skipped = 1;
    assert_eq!(copy(&p, &NumberLocale::default()).eta, "~1 s");
    p.skipped = 0;
    p.failed = 1;
    assert_eq!(copy(&p, &NumberLocale::default()).eta, "~1 s");
    p.failed = 0;
    for rate in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        Arc::make_mut(&mut p.throughput).smoothed_verification_per_second = rate;
        assert!(copy(&p, &NumberLocale::default()).eta.is_empty());
    }
}
#[test]
fn native_root_scope_keeps_measured_item_eta_without_a_byte_budget() {
    let mut p = transfer(TransferIntent::Move, TransferVerification::SizeAndSamples);
    p.discovery.scope = DiscoveryScope::SelectedRoots;
    p.discovery.files = 8;
    p.completed_roots = 2;
    Arc::make_mut(&mut p.throughput).smoothed_entries_per_second = 2.0;
    assert!(p.remaining_work.is_none());
    assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 3 s");
}

/// 124,000 small files: the window's user report, after minutes of copying.
fn small_files(bytes: Option<u64>) -> CopyProgress {
    let mut p = transfer(TransferIntent::Copy, TransferVerification::FullSha256);
    p.discovery.files = 120_000;
    p.discovery.directories = 4_000;
    p.discovery.bytes = bytes;
    p.files_completed = 60_000;
    p.completed_directories = 2_000;
    p.completed_bytes = bytes.map_or(0, |total| total / 2);
    p.remaining_work = bytes.map(|total| TransferRemainingWork {
        write_bytes: total / 2,
        verification_bytes: total / 2,
    });
    p.throughput = Arc::new(OperationThroughput {
        // The latest quarter second's rates: no basis for an estimate.
        transfer_per_second: 0.0,
        items_per_second: 0.0,
        ..Default::default()
    });
    p
}

#[test]
fn many_small_files_without_a_byte_total_estimate_from_the_item_rate() {
    // A link in the tree leaves the byte total unknown; before, the estimate waited
    // for an item rate that a copy never measured and stayed "Calculating...".
    let mut p = small_files(None);
    let calculating = copy(&p, &grouped());
    assert!(calculating.determinate, "the window shows Calculating...");
    assert!(calculating.eta.is_empty(), "no measured basis yet");
    Arc::make_mut(&mut p.throughput).smoothed_entries_per_second = 500.0;
    assert_eq!(copy(&p, &grouped()).eta, "About 3 min");
    assert_eq!(
        copy(&p, &grouped()).totals,
        "Files: 60,000 of 120,000 · Bytes: 0 B (total size unknown)"
    );
}

#[test]
fn small_files_lean_on_the_item_rate_and_large_ones_on_the_byte_rate() {
    // 62,000 entries remain with 1.0 GiB to write, 17 KiB each on average: the
    // items' 124 s (at 500/s) outweigh the bytes' 64 s (at 32 MiB/s both ways).
    let mut p = small_files(Some(2 << 30));
    Arc::make_mut(&mut p.throughput).smoothed_transfer_per_second = 16.0 * 1024.0 * 1024.0;
    Arc::make_mut(&mut p.throughput).smoothed_verification_per_second = 16.0 * 1024.0 * 1024.0;
    let bytes_only = copy(&p, &NumberLocale::default()).eta;
    assert_eq!(bytes_only, "About 2 min", "64 s from bytes alone");
    Arc::make_mut(&mut p.throughput).smoothed_entries_per_second = 500.0;
    let blended = copy(&p, &grouped());
    assert_eq!(blended.eta, "About 3 min", "mostly the items' 124 s");
    assert_eq!(
        blended.totals,
        "Files: 60,000 of 120,000 · Bytes: 1.0 GiB of 2.0 GiB"
    );

    // One 4 GiB file: the item rate is zero and bytes alone decide.
    let mut big = transfer(TransferIntent::Copy, TransferVerification::FullSha256);
    big.discovery.bytes = Some(4 << 30);
    big.current_file_copied = 1 << 30;
    big.remaining_work = Some(TransferRemainingWork {
        write_bytes: 3 << 30,
        verification_bytes: 4 << 30,
    });
    big.throughput = Arc::new(OperationThroughput {
        smoothed_transfer_per_second: 100.0 * 1024.0 * 1024.0,
        ..Default::default()
    });
    assert_eq!(copy(&big, &NumberLocale::default()).eta, "About 2 min");
    // Large files that remain weigh the byte rate almost alone.
    Arc::make_mut(&mut big.throughput).smoothed_entries_per_second = 0.001;
    assert_eq!(copy(&big, &NumberLocale::default()).eta, "About 2 min");
}

#[test]
fn a_paused_estimate_holds_still_and_resumes_from_the_same_averages() {
    let mut p = small_files(None);
    Arc::make_mut(&mut p.throughput).smoothed_entries_per_second = 1000.0;
    assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 2 min");
    p.pause_requested = true;
    p.paused = true;
    // The backend stops the current rates and keeps the averages while paused.
    stop_current_rates(&mut p.throughput);
    let paused = copy(&p, &NumberLocale::default());
    assert_eq!(paused.eta, "About 2 min (paused)");
    assert_eq!(paused.label, "Paused");
    p.pause_requested = false;
    assert_eq!(
        copy(&p, &NumberLocale::default()).eta,
        "About 2 min (paused)",
        "resuming: still paused until the worker acknowledges"
    );
    p.paused = false;
    assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 2 min");
}

/// What the backend publishes while paused: current rates zero, averages kept.
fn stop_current_rates(throughput: &mut Arc<OperationThroughput>) {
    let stopped = Arc::make_mut(throughput);
    stopped.transfer_per_second = 0.0;
    stopped.verification_per_second = 0.0;
    stopped.items_per_second = 0.0;
}

#[test]
fn totals_show_the_size_once_known_and_selected_items_in_root_scope() {
    let locale = grouped();
    let mut p = transfer(TransferIntent::Copy, TransferVerification::FullSha256);
    p.discovery = OperationDiscovery::scanning(DiscoveryScope::Recursive);
    p.discovery.files = 1_234;
    p.discovery.bytes = Some(5 << 20);
    let counting = copy(&p, &locale);
    assert!(counting.totals.is_empty(), "the headline counts instead");
    assert_eq!(counting.label, "Discovering: 1,234 items (5.0 MiB)");
    p.discovery.files = 2_345;
    p.discovery.bytes = Some(7 << 20);
    assert_eq!(
        copy(&p, &locale).label,
        "Discovering: 2,345 items (7.0 MiB)",
        "the count goes on live"
    );
    p.discovery.finish();
    p.files_completed = 12;
    p.completed_bytes = 1 << 20;
    p.current_file_copied = 512 << 10;
    assert_eq!(
        copy(&p, &locale).totals,
        "Files: 12 of 2,345 · Bytes: 1.5 MiB of 7.0 MiB"
    );
    p.intent = TransferIntent::Move;
    p.source_files_removed = 10;
    assert_eq!(
        copy(&p, &locale).totals,
        "Files: 10 of 2,345 · Bytes: 1.5 MiB of 7.0 MiB",
        "a Move counts files once their source is removed"
    );
    p.discovery
        .unavailable("The source changed after discovery; total progress is unavailable.");
    assert_eq!(copy(&p, &locale).totals, "Files: 10 · Bytes: 1.5 MiB");
    p.discovery = OperationDiscovery {
        state: DiscoveryState::Complete,
        scope: DiscoveryScope::SelectedRoots,
        files: 3,
        ..Default::default()
    };
    p.completed_roots = 2;
    assert_eq!(copy(&p, &locale).totals, "Selected items: 2 of 3");
}

fn delete_job(mode: DeleteMode) -> OperationProgress {
    OperationProgress {
        id: OperationId(5),
        kind: MutationKind::Delete(mode),
        state: OperationState::Running,
        pause_requested: false,
        paused: false,
        total: 2,
        completed: 0,
        failed: 0,
        processed_entries: 0,
        current_path: None,
        source_directory: "source".into(),
        renamed_to: None,
        message: String::new(),
        errors: vec![],
        throughput: Arc::default(),
        discovery: OperationDiscovery::scanning(DiscoveryScope::Recursive),
    }
}

#[test]
fn delete_counts_items_without_bytes_and_estimates_from_its_item_rate() {
    let locale = grouped();
    let mut p = delete_job(DeleteMode::Permanent);
    p.discovery.files = 900;
    p.discovery.directories = 100;
    p.discovery.bytes = Some(5 << 30);
    let counting = deletion(&p, &locale);
    assert_eq!(counting.label, "Discovering: 1,000 items");
    assert!(counting.totals.is_empty() && counting.eta.is_empty());
    p.discovery.finish();
    p.processed_entries = 400;
    let running = deletion(&p, &locale);
    assert_eq!(running.totals, "Items deleted: 400 of 1,000");
    assert!(running.eta.is_empty(), "no measured rate yet");
    p.throughput = Arc::new(OperationThroughput {
        items_per_second: 0.0,
        smoothed_entries_per_second: 20.0,
        ..Default::default()
    });
    assert_eq!(deletion(&p, &locale).eta, "About 30 s");
    p.pause_requested = true;
    p.paused = true;
    assert_eq!(deletion(&p, &locale).eta, "About 30 s (paused)");
    let mut trash = delete_job(DeleteMode::Trash);
    trash.discovery = OperationDiscovery {
        state: DiscoveryState::Complete,
        scope: DiscoveryScope::SelectedRoots,
        files: 3,
        ..Default::default()
    };
    trash.completed = 1;
    assert_eq!(
        deletion(&trash, &locale).totals,
        "Selected items moved to the Trash: 1 of 3"
    );
}

/// Totals that are only approximate (part of the tree could not be counted, or a
/// skipped item left part of the work undone) keep the bar and the estimate, marked
/// "~", and the bar never passes them.
#[test]
fn approximate_totals_keep_a_determinate_bar_and_an_estimate_marked_approximate() {
    let locale = grouped();
    let mut p = transfer(TransferIntent::Copy, TransferVerification::FullSha256);
    p.discovery.files = 4;
    p.discovery.approximate = true;
    p.files_completed = 1;
    p.completed_bytes = 250_000;
    p.remaining_work = Some(TransferRemainingWork {
        write_bytes: 750_000,
        verification_bytes: 750_000,
    });
    p.throughput = Arc::new(OperationThroughput {
        smoothed_transfer_per_second: 50_000.0,
        smoothed_verification_per_second: 50_000.0,
        ..Default::default()
    });
    let approximate = copy(&p, &locale);
    assert!(approximate.determinate);
    assert_eq!(approximate.fraction, 0.25);
    assert_eq!(
        approximate.totals,
        "Files: 1 of ~4 · Bytes: 244.1 KiB of ~976.6 KiB"
    );
    assert_eq!(approximate.eta, "~15 s");
    // More done than counted: the bar holds below the whole, never past it.
    p.completed_bytes = 1_200_000;
    let beyond = copy(&p, &locale);
    assert!(beyond.determinate);
    assert!(
        beyond.fraction < 1.0 && beyond.fraction >= 0.99,
        "{}",
        beyond.fraction
    );
    // A skipped item: still a bar and an estimate, approximate.
    p.discovery.approximate = false;
    p.completed_bytes = 250_000;
    p.skipped = 1;
    let skipped = copy(&p, &locale);
    assert!(skipped.determinate);
    assert!(skipped.totals.contains("of ~"), "{}", skipped.totals);
    assert_eq!(skipped.eta, "~15 s");
}
