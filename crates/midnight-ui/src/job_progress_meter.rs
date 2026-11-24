//! Presentation of discovered work. I/O rates never masquerade as committed bytes.
use midnight_core::{
    CopyOperationState, CopyProgress, DeleteMode, DiscoveryScope, DiscoveryState, NumberLocale,
    OperationDiscovery, OperationProgress, OperationState,
};
use slint::SharedString;

/// Below this average size of the files still to copy, a copy's time goes mostly to
/// per-file work (creating, saving, reading back and publishing each file), so its
/// estimate follows the item rate; well above it, the byte rate.
const SMALL_FILE_BYTES: f64 = 1024.0 * 1024.0;

#[derive(Default)]
pub(super) struct Meter {
    pub determinate: bool,
    pub fraction: f32,
    pub label: SharedString,
    pub totals: SharedString,
    pub remaining: SharedString,
    pub eta: SharedString,
    pub discovering: bool,
    pub waiting: bool,
    pub activity_label: &'static str,
    /// Beside the headline of a running transfer (`copy_speeds`).
    pub write_speed: SharedString,
    pub verify_speed: SharedString,
}

pub(super) fn copy_waiting_label(p: &CopyProgress) -> Option<&'static str> {
    if p.state.is_terminal()
        || matches!(
            p.state,
            CopyOperationState::CancelRequested | CopyOperationState::WaitingForDecision
        )
        || p.paused
        || p.pause_requested
    {
        return None;
    }
    use midnight_core::TransferWaitReason as Reason;
    match p.wait_reason.as_ref() {
        Some(Reason::EarlierTransfers) => Some("Queued — waiting for earlier transfers"),
        Some(Reason::Resources) => Some("Waiting for available capacity"),
        Some(Reason::DiskCapacity) => Some("Waiting for disk capacity"),
        Some(Reason::SameFolders) => Some("Waiting for another job in the same folders to finish"),
        // This job or the other cannot confirm which folders it uses (a volume with 8.3
        // short names, for example), so that one runs alone, whichever folders they use.
        Some(Reason::UnconfirmedFolders) => {
            Some("Waiting for another job: jobs whose folders cannot be confirmed run alone")
        }
        Some(Reason::OtherOperation) => Some("Waiting for another file operation"),
        None if p.state == CopyOperationState::Queued => Some("Queued"),
        None => None,
    }
}

pub(super) fn copy_status_label(p: &CopyProgress) -> Option<&'static str> {
    match p.state {
        CopyOperationState::CancelRequested => return Some("Cancelling..."),
        CopyOperationState::WaitingForDecision => return Some("Waiting for your decision"),
        state if state.is_terminal() => return None,
        _ => {}
    }
    if p.paused {
        return Some("Paused");
    }
    if p.pause_requested {
        return Some("Pausing...");
    }
    copy_waiting_label(p)
}

pub(super) fn copy(p: &CopyProgress, locale: &NumberLocale) -> Meter {
    let status = copy_status_label(p);
    if status.is_none()
        && p.state == CopyOperationState::Preparing
        && p.discovery.state == DiscoveryState::NotStarted
    {
        return Meter {
            label: "Preparing".into(),
            ..Default::default()
        };
    }
    let total = p.discovery.entries();
    let recursive = p.discovery.scope == DiscoveryScope::Recursive;
    let completed = if recursive {
        copy_items_completed(p)
    } else {
        Some(p.completed_roots)
    };
    // Counted totals keep the bar and the estimate. They are approximate ("~") when part
    // of the work could not be counted or grew beyond the count, and when skipped or
    // failed items leave part of it undone: an approximate bar is better than none.
    let known = p.discovery.state == DiscoveryState::Complete;
    let approximate = known && (p.discovery.approximate || p.skipped != 0 || p.failed != 0);
    let bytes = recursive.then_some(p.discovery.bytes).flatten();
    let live_bytes = if p.state.is_terminal() {
        Some(p.completed_bytes)
    } else {
        p.completed_bytes.checked_add(p.current_file_copied)
    };
    let work = if let Some(total) = bytes.filter(|total| *total > 0) {
        live_bytes.map(|done| (done, total))
    } else {
        completed.zip(total)
    };
    // An approximation is never passed: the bar holds below 100% until the job ends.
    let work = work.map(|(done, total)| {
        if approximate {
            (done.min(total), total)
        } else {
            (done, total)
        }
    });
    let mut meter = meter(
        &p.discovery,
        completed,
        known.then_some(work).flatten(),
        p.state == CopyOperationState::Completed && p.skipped == 0,
        p.state.is_terminal(),
        true,
        locale,
    );
    meter.totals = copy_totals(p, live_bytes.unwrap_or(p.completed_bytes), locale);
    if approximate && !meter.remaining.is_empty() {
        meter.remaining = meter.remaining.replacen(": ", ": ~", 1).into();
    }
    if let Some(label) = status {
        meter.discovering = false;
        meter.label = label.into();
    } else if meter.discovering
        && (p.discovery.state != DiscoveryState::Scanning
            || p.paused
            || p.pause_requested
            || !matches!(
                p.state,
                CopyOperationState::Preparing | CopyOperationState::Running
            ))
    {
        meter.discovering = false;
        meter.label = format!(
            "{} items completed",
            locale.format_u64(completed.unwrap_or(0))
        )
        .into();
    }
    // A pause holds the estimate still (the work and the averages stop together); a
    // wait, a decision or a cancellation has none.
    if known && p.state == CopyOperationState::Running && (p.paused || p.wait_reason.is_none()) {
        let seconds = copy_estimate(p, bytes, total.zip(completed));
        meter.eta = eta(seconds, p.paused, locale);
        if approximate && !meter.eta.is_empty() {
            meter.eta = meter.eta.replacen("About ", "~", 1).into();
        }
    }
    (meter.write_speed, meter.verify_speed) = copy_speeds(p, locale);
    meter
}

/// The smoothed rates the estimate uses, beside the headline while the job runs:
/// "write 187.0 MiB/s" and, when the job reads its copies back in full (SHA-256),
/// "verify 160.4 MiB/s". Both show together, so the line never switches between
/// them; a rate not measured yet, or zero for now, shows "—" in its place. Nothing
/// while the job is paused, waits, needs a decision, is being cancelled or has
/// ended, nor for a Move that only renames entries (it writes no bytes).
fn copy_speeds(p: &CopyProgress, locale: &NumberLocale) -> (SharedString, SharedString) {
    let renames_only = p.native_entries_moved > 0 && p.copied_bytes == 0;
    if p.state != CopyOperationState::Running || p.paused || p.wait_reason.is_some() || renames_only
    {
        return Default::default();
    }
    let rate = |caption: &str, value: f64| -> SharedString {
        if value.is_finite() && value >= 1.0 {
            let bytes = value.min(u64::MAX as f64).round() as u64;
            format!("{caption} {}/s", crate::format_size(bytes, locale)).into()
        } else {
            format!("{caption} —").into()
        }
    };
    let rates = &p.throughput;
    let verify = if p.verification == midnight_core::TransferVerification::FullSha256 {
        rate("verify", rates.smoothed_verification_per_second)
    } else {
        SharedString::default()
    };
    (rate("write", rates.smoothed_transfer_per_second), verify)
}

/// Remaining seconds from the backend's smoothed rates: from bytes when the job has
/// a byte budget, from items when it has none (a link or a size unknown in the tree,
/// or only selected items counted), and weighted toward items the smaller the files
/// that remain are.
fn copy_estimate(p: &CopyProgress, bytes: Option<u64>, items: Option<(u64, u64)>) -> Option<u64> {
    let rates = &p.throughput;
    let remaining_items = items.map(|(total, done)| total.saturating_sub(done));
    let by_items = remaining_items
        .and_then(|items| seconds_at(items as f64, rates.smoothed_entries_per_second));
    // The backend budgets writes and the selected readback, including a Move's
    // outstanding proofs. The rates measure the same categories, retries included;
    // retries never reduce the remaining logical work.
    let budget = p
        .remaining_work
        .filter(|_| bytes.is_some_and(|total| total > 0));
    let by_bytes = budget.and_then(|work| {
        seconds_at(
            work.write_bytes as f64 + work.verification_bytes as f64,
            rates.smoothed_transfer_per_second + rates.smoothed_verification_per_second,
        )
    });
    let seconds = match (by_bytes, by_items, budget, remaining_items) {
        (Some(by_bytes), Some(by_items), Some(work), Some(items)) => {
            // What each remaining item still needs; once written, its readback.
            let left = work.write_bytes.max(work.verification_bytes);
            let average = left as f64 / items.max(1) as f64;
            let toward_items = SMALL_FILE_BYTES / (SMALL_FILE_BYTES + average);
            toward_items * by_items + (1.0 - toward_items) * by_bytes
        }
        (by_bytes, by_items, ..) => by_bytes.or(by_items)?,
    };
    whole_seconds(seconds)
}

/// "Files: 2 of 8 · Bytes: 1.2 MiB of 3.4 MiB" once discovery has the totals. While
/// it still counts, the headline says so and shows the count so far.
fn copy_totals(p: &CopyProgress, copied: u64, locale: &NumberLocale) -> SharedString {
    let n = |value: u64| locale.format_u64(value);
    let size = |bytes: u64| crate::format_size(bytes, locale);
    let complete = p.discovery.state == DiscoveryState::Complete;
    // An approximate total says so: "Bytes: 1.2 GiB of ~3.4 GiB".
    let about = if complete && (p.discovery.approximate || p.skipped != 0 || p.failed != 0) {
        "~"
    } else {
        ""
    };
    if p.discovery.scope == DiscoveryScope::SelectedRoots {
        return match p
            .discovery
            .entries()
            .filter(|total| complete && p.completed_roots <= *total)
        {
            Some(total) => format!("Selected items: {} of {}", n(p.completed_roots), n(total)),
            None if complete => format!("Selected items: {}", n(p.completed_roots)),
            None => String::new(),
        }
        .into();
    }
    if !matches!(
        p.discovery.state,
        DiscoveryState::Complete | DiscoveryState::Unavailable
    ) {
        return SharedString::default();
    }
    let done = copy_files_completed(p);
    let files = if complete && (done <= p.discovery.files || !about.is_empty()) {
        let total = p.discovery.files.max(done);
        format!("Files: {} of {about}{}", n(done), n(total))
    } else {
        format!("Files: {}", n(done))
    };
    let bytes = match p.discovery.bytes.filter(|_| complete) {
        Some(total) if copied <= total || !about.is_empty() => format!(
            "Bytes: {} of {about}{}",
            size(copied),
            size(total.max(copied))
        ),
        None if complete => format!("Bytes: {} (total size unknown)", size(copied)),
        _ => format!("Bytes: {}", size(copied)),
    };
    format!("{files} · {bytes}").into()
}

fn copy_files_completed(p: &CopyProgress) -> u64 {
    if p.intent == midnight_core::TransferIntent::Move {
        p.source_files_removed
    } else {
        p.files_completed
    }
}

fn copy_items_completed(p: &CopyProgress) -> Option<u64> {
    copy_files_completed(p).checked_add(p.completed_directories)
}

pub(super) fn deletion(p: &OperationProgress, locale: &NumberLocale) -> Meter {
    let midnight_core::MutationKind::Delete(mode) = p.kind else {
        return meter(
            &OperationDiscovery {
                state: DiscoveryState::Complete,
                scope: DiscoveryScope::SelectedRoots,
                files: p.total,
                ..Default::default()
            },
            Some(p.completed),
            Some((p.completed, p.total)),
            p.state == OperationState::Completed,
            p.state.is_terminal(),
            false,
            locale,
        );
    };
    let completed = if p.discovery.scope == DiscoveryScope::SelectedRoots {
        p.completed
    } else {
        p.processed_entries
    };
    let known = p.discovery.state == DiscoveryState::Complete && p.failed == 0;
    // A Delete counts items; the size of what it removes is no measure of its work.
    let mut meter = meter(
        &p.discovery,
        Some(completed),
        known
            .then(|| p.discovery.entries().map(|total| (completed, total)))
            .flatten(),
        p.state == OperationState::Completed,
        p.state.is_terminal(),
        false,
        locale,
    );
    meter.totals = deletion_totals(p, mode, completed, locale);
    use OperationState as S;
    let status = match p.state {
        S::CancelRequested => Some("Cancelling..."),
        state if state.is_terminal() => None,
        _ if p.paused && p.pause_requested => Some("Paused"),
        _ if p.paused => Some("Resuming..."),
        _ if p.pause_requested => Some("Pausing..."),
        S::Queued => Some("Queued — waiting to start"),
        S::AwaitingConfirmation => Some("Awaiting confirmation"),
        S::Preparing => Some("Preparing"),
        S::Running if p.discovery.state == DiscoveryState::NotStarted => Some("Preparing"),
        _ => None,
    };
    meter.waiting = matches!(p.state, S::Queued | S::AwaitingConfirmation);
    if let Some(label) = status {
        meter.discovering = false;
        meter.label = label.into();
        meter.activity_label = label;
    } else if meter.discovering {
        meter.activity_label = "Discovering items...";
    } else if p.state == S::Running && !meter.determinate {
        meter.activity_label = "Deleting items...";
    }
    // A pause holds the estimate still; it resumes from the same averages.
    if known && p.state == S::Running {
        let remaining = p.discovery.entries().unwrap_or(0).saturating_sub(completed);
        let seconds = seconds_at(remaining as f64, p.throughput.smoothed_entries_per_second);
        meter.eta = eta(seconds.and_then(whole_seconds), p.paused, locale);
    }
    meter
}

/// "Items deleted: 3 of 10" once discovery has counted them.
fn deletion_totals(
    p: &OperationProgress,
    mode: DeleteMode,
    completed: u64,
    locale: &NumberLocale,
) -> SharedString {
    let Some(total) = p
        .discovery
        .entries()
        .filter(|_| p.discovery.state == DiscoveryState::Complete)
    else {
        return SharedString::default();
    };
    let scope = if p.discovery.scope == DiscoveryScope::SelectedRoots {
        "Selected items"
    } else {
        "Items"
    };
    let verb = if mode == DeleteMode::Trash {
        "moved to the Trash"
    } else {
        "deleted"
    };
    let n = |value: u64| locale.format_u64(value);
    if completed <= total {
        format!("{scope} {verb}: {} of {}", n(completed), n(total))
    } else {
        format!("{scope} {verb}: {}", n(completed))
    }
    .into()
}

fn meter(
    discovery: &OperationDiscovery,
    completed: Option<u64>,
    work: Option<(u64, u64)>,
    success: bool,
    terminal: bool,
    with_bytes: bool,
    locale: &NumberLocale,
) -> Meter {
    let discovering = !terminal
        && matches!(
            discovery.state,
            DiscoveryState::NotStarted | DiscoveryState::Scanning
        );
    let mut output = Meter {
        discovering,
        ..Meter::default()
    };
    if discovering {
        // The count so far, updated as discovery goes.
        output.label = format!(
            "Discovering: {} items{}",
            locale.format_u64(discovery.entries().unwrap_or(0)),
            discovery
                .bytes
                .filter(|_| with_bytes)
                .map_or(String::new(), |bytes| format!(
                    " ({})",
                    crate::format_size(bytes, locale)
                ))
        )
        .into();
        return output;
    }
    if success {
        output.determinate = true;
        output.fraction = 1.0;
        output.label = "100% complete".into();
    } else if let Some((done, total)) = work.filter(|(done, total)| *total > 0 && done <= total) {
        output.determinate = true;
        output.fraction = ((done as f64 / total as f64).min(0.99)) as f32;
        // Integer percentage: the f32 bar fraction rounds 53/100 to 52.99…%.
        let percent = (u128::from(done) * 100 / u128::from(total)).min(99);
        output.label = format!("{percent}% complete").into();
    } else {
        output.label = format!(
            "{} items completed",
            locale.format_u64(completed.unwrap_or(0))
        )
        .into();
    }
    if discovery.state == DiscoveryState::Complete
        && let Some((total, done)) = discovery
            .entries()
            .zip(completed)
            .filter(|(total, done)| done <= total)
    {
        let scope = if discovery.scope == DiscoveryScope::SelectedRoots {
            "Selected items"
        } else {
            "Items"
        };
        output.remaining = format!("{scope} remaining: {}", locale.format_u64(total - done)).into();
    }
    output
}

/// Seconds that `remaining` work takes at `rate`; none without work or a rate.
fn seconds_at(remaining: f64, rate: f64) -> Option<f64> {
    if remaining <= 0.0 || !remaining.is_finite() || !rate.is_finite() || rate <= 0.0 {
        return None;
    }
    Some(remaining / rate).filter(|seconds| seconds.is_finite())
}

fn whole_seconds(seconds: f64) -> Option<u64> {
    let seconds = seconds.ceil();
    (seconds.is_finite() && seconds >= 0.0 && seconds <= u64::MAX as f64).then_some(seconds as u64)
}
fn eta(seconds: Option<u64>, paused: bool, locale: &NumberLocale) -> SharedString {
    seconds.map_or_else(SharedString::default, |seconds| {
        let estimate = if seconds < 60 {
            format!("About {} s", locale.format_u64(seconds))
        } else if seconds < 3600 {
            format!("About {} min", locale.format_u64(seconds.div_ceil(60)))
        } else {
            format!("About {} h", locale.format_u64(seconds.div_ceil(3600)))
        };
        if paused {
            format!("{estimate} (paused)")
        } else {
            estimate
        }
        .into()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{OperationId, OperationThroughput, TransferIntent};
    use std::sync::Arc;
    fn transfer() -> CopyProgress {
        let mut p = CopyProgress::queued(OperationId(1), "target".into(), 1, TransferIntent::Copy);
        p.state = CopyOperationState::Running;
        p.discovery = OperationDiscovery {
            state: DiscoveryState::Complete,
            files: 2,
            directories: 1,
            bytes: Some(1000),
            ..Default::default()
        };
        p.remaining_work = Some(midnight_core::TransferRemainingWork {
            write_bytes: 1000,
            verification_bytes: 1000,
        });
        p
    }
    #[test]
    fn discovery_partial_counts_do_not_become_a_completion_denominator() {
        let mut p = transfer();
        p.discovery.state = DiscoveryState::Scanning;
        let value = copy(&p, &NumberLocale::default());
        assert!(!value.determinate && value.discovering);
        assert!(value.label.contains("3 items"));
        assert!(value.eta.is_empty() && value.remaining.is_empty());
    }
    #[test]
    fn percentage_label_uses_exact_integer_division() {
        let discovery = OperationDiscovery {
            state: DiscoveryState::Complete,
            ..Default::default()
        };
        let label = |done, total| {
            meter(
                &discovery,
                None,
                Some((done, total)),
                false,
                false,
                true,
                &NumberLocale::default(),
            )
            .label
        };
        // f32 fractions put these just below the whole percentage.
        for (done, total, percent) in [(53, 100, 53), (59, 100, 59), (530, 1000, 53)] {
            assert_eq!(label(done, total), format!("{percent}% complete"));
        }
        assert_eq!(label(177, 300), "59% complete");
        assert_eq!(label(u64::MAX - 1, u64::MAX), "99% complete");
        assert_eq!(label(100, 100), "99% complete", "100% requires success");
    }
    #[test]
    fn retry_io_cannot_inflate_progress_and_100_requires_success() {
        let mut p = transfer();
        p.completed_bytes = 500;
        p.current_file_copied = 250;
        p.files_completed = 1;
        p.copied_bytes = 4000;
        p.verified_bytes = 3000;
        let value = copy(&p, &NumberLocale::default());
        assert_eq!(value.fraction, 0.75);
        assert!(value.remaining.contains("2"));
        p.current_file_copied = 500;
        assert!(copy(&p, &NumberLocale::default()).fraction < 1.0);
        p.state = CopyOperationState::Completed;
        assert_eq!(copy(&p, &NumberLocale::default()).fraction, 1.0);
    }
    #[test]
    fn move_keeps_items_remaining_until_source_removal_finishes() {
        let mut p = transfer();
        p.intent = TransferIntent::Move;
        p.files_completed = 2;
        p.completed_bytes = 1000;
        p.source_files_removed = 1;
        let value = copy(&p, &NumberLocale::default());
        assert_eq!(value.remaining, "Items remaining: 2");
        assert!(value.fraction < 1.0);
        p.source_files_removed = 2;
        assert_eq!(
            copy(&p, &NumberLocale::default()).remaining,
            "Items remaining: 1"
        );
        p.completed_directories = 1;
        p.state = CopyOperationState::Completed;
        let value = copy(&p, &NumberLocale::default());
        assert_eq!(value.remaining, "Items remaining: 0");
        assert_eq!(value.fraction, 1.0);
    }
    #[test]
    fn changed_counts_errors_and_pauses_do_not_produce_misleading_eta() {
        let mut p = transfer();
        p.throughput = Arc::new(OperationThroughput {
            smoothed_transfer_per_second: 100.0,
            ..Default::default()
        });
        assert!(copy(&p, &NumberLocale::default()).eta.contains("20 s"));
        p.current_file_copied = 500;
        p.remaining_work.as_mut().unwrap().write_bytes = 500;
        assert!(copy(&p, &NumberLocale::default()).eta.contains("15 s"));
        p.current_file_copied = 1000;
        p.current_file_verified = 500;
        p.remaining_work = Some(midnight_core::TransferRemainingWork {
            write_bytes: 0,
            verification_bytes: 500,
        });
        assert!(copy(&p, &NumberLocale::default()).eta.contains("5 s"));
        p.copied_bytes = 50_000;
        p.verified_bytes = 30_000;
        assert!(copy(&p, &NumberLocale::default()).eta.contains("5 s"));
        // Until its checkpoint the job runs on; once paused its estimate holds.
        p.pause_requested = true;
        assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 5 s");
        p.paused = true;
        assert_eq!(copy(&p, &NumberLocale::default()).eta, "About 5 s (paused)");
        p.pause_requested = false;
        p.paused = false;
        p.discovery.state = DiscoveryState::Unavailable;
        assert!(!copy(&p, &NumberLocale::default()).determinate);
        assert!(copy(&p, &NumberLocale::default()).eta.is_empty());
        // A failed item leaves the totals approximate: the bar and the estimate stay.
        p.discovery.state = DiscoveryState::Complete;
        p.failed = 1;
        assert!(copy(&p, &NumberLocale::default()).determinate);
        assert!(copy(&p, &NumberLocale::default()).eta.starts_with('~'));
    }
    #[test]
    fn running_copies_show_their_smoothed_write_and_full_readback_rates_side_by_side() {
        let speeds = |p: &CopyProgress, locale: &NumberLocale| {
            let meter = copy(p, locale);
            (
                meter.write_speed.to_string(),
                meter.verify_speed.to_string(),
            )
        };
        let locale = NumberLocale::default();
        let mut p = transfer();
        p.verification = midnight_core::TransferVerification::FullSha256;
        // Nothing measured yet: both places hold a dash.
        assert_eq!(speeds(&p, &locale), ("write —".into(), "verify —".into()));
        // The estimate's smoothed rates, not the momentary ones.
        p.throughput = Arc::new(OperationThroughput {
            transfer_per_second: 999.0,
            verification_per_second: 999.0,
            smoothed_transfer_per_second: 187.0 * 1024.0 * 1024.0,
            smoothed_verification_per_second: 160.4 * 1024.0 * 1024.0,
            ..Default::default()
        });
        assert_eq!(
            speeds(&p, &locale),
            ("write 187.0 MiB/s".into(), "verify 160.4 MiB/s".into())
        );
        // A rate that is zero for now keeps its place.
        Arc::make_mut(&mut p.throughput).smoothed_verification_per_second = 0.0;
        assert_eq!(speeds(&p, &locale).1, "verify —");
        Arc::make_mut(&mut p.throughput).smoothed_verification_per_second = f64::NAN;
        assert_eq!(speeds(&p, &locale).1, "verify —");
        // The locale's decimal separator.
        let comma = NumberLocale {
            decimal_separator: ",".into(),
            ..Default::default()
        };
        assert_eq!(speeds(&p, &comma).0, "write 187,0 MiB/s");
        // Sampled readback is a few KiB per file: the write rate alone.
        p.verification = midnight_core::TransferVerification::SizeAndSamples;
        assert_eq!(
            speeds(&p, &locale),
            ("write 187.0 MiB/s".into(), String::new())
        );
        p.verification = midnight_core::TransferVerification::FullSha256;
        // A pause, a wait, a decision, a cancellation and the end show none.
        let mut paused = p.clone();
        paused.paused = true;
        paused.pause_requested = true;
        assert_eq!(speeds(&paused, &locale), Default::default());
        assert_eq!(copy(&paused, &locale).label, "Paused");
        let mut waiting = p.clone();
        waiting.wait_reason = Some(midnight_core::TransferWaitReason::Resources);
        assert_eq!(speeds(&waiting, &locale), Default::default());
        for state in [
            CopyOperationState::Queued,
            CopyOperationState::WaitingForDecision,
            CopyOperationState::CancelRequested,
            CopyOperationState::Completed,
            CopyOperationState::Failed,
        ] {
            let mut other = p.clone();
            other.state = state;
            assert_eq!(speeds(&other, &locale), Default::default(), "{state:?}");
        }
        // Pausing runs on to its checkpoint and keeps the rates until then.
        let mut pausing = p.clone();
        pausing.pause_requested = true;
        assert_eq!(speeds(&pausing, &locale).0, "write 187.0 MiB/s");
        // A Move that only renames writes no bytes.
        let mut renamed = p.clone();
        renamed.intent = TransferIntent::Move;
        renamed.native_entries_moved = 3;
        assert_eq!(speeds(&renamed, &locale), Default::default());
        renamed.copied_bytes = 1;
        assert_eq!(speeds(&renamed, &locale).0, "write 187.0 MiB/s");
    }
}

#[cfg(test)]
#[path = "job_progress_eta_tests.rs"]
mod eta_tests;
