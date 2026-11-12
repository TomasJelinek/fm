//! Presentation of discovered work. I/O rates never masquerade as committed bytes.
use midnight_core::{
    CopyOperationState, CopyProgress, DiscoveryScope, DiscoveryState, NumberLocale,
    OperationDiscovery, OperationProgress, OperationState,
};
use slint::SharedString;

#[derive(Default)]
pub(super) struct Meter {
    pub determinate: bool,
    pub fraction: f32,
    pub label: SharedString,
    pub remaining: SharedString,
    pub eta: SharedString,
    pub discovering: bool,
}

pub(super) fn copy(p: &CopyProgress, locale: &NumberLocale) -> Meter {
    let total = p.discovery.entries();
    let recursive = p.discovery.scope == DiscoveryScope::Recursive;
    let completed = if recursive {
        let files = if p.intent == midnight_core::TransferIntent::Move {
            p.source_files_removed
        } else {
            p.files_completed
        };
        files.checked_add(p.completed_directories)
    } else {
        Some(p.completed_roots)
    };
    let known = p.discovery.state == DiscoveryState::Complete && p.skipped == 0 && p.failed == 0;
    let bytes = recursive.then_some(p.discovery.bytes).flatten();
    let live_bytes = p.completed_bytes.checked_add(p.current_file_copied);
    let work = if let Some(total) = bytes.filter(|total| *total > 0) {
        live_bytes.map(|done| (done, total))
    } else {
        completed.zip(total)
    };
    let mut meter = meter(
        &p.discovery,
        completed,
        known.then_some(work).flatten(),
        p.state == CopyOperationState::Completed,
        p.state.is_terminal(),
        locale,
    );
    if known && !p.pause_requested && !p.paused && p.state == CopyOperationState::Running {
        let seconds = if bytes.is_some_and(|total| total > 0) {
            // The backend budgets selected sample ranges and outstanding Move proof
            // passes. Rates measure the same writes/readback categories, including
            // actual retry activity; retries never reduce remaining logical work.
            p.remaining_work.and_then(|work| {
                estimate_seconds(
                    work.write_bytes as f64 + work.verification_bytes as f64,
                    p.throughput.transfer_per_second + p.throughput.verification_per_second,
                )
            })
        } else {
            estimate_seconds(
                total.unwrap_or(0).saturating_sub(completed.unwrap_or(0)) as f64,
                p.throughput.items_per_second,
            )
        };
        meter.eta = eta(seconds, locale);
    }
    meter
}

pub(super) fn deletion(p: &OperationProgress, locale: &NumberLocale) -> Meter {
    if !matches!(p.kind, midnight_core::MutationKind::Delete(_)) {
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
            locale,
        );
    }
    let completed = if p.discovery.scope == DiscoveryScope::SelectedRoots {
        p.completed
    } else {
        p.processed_entries
    };
    let known = p.discovery.state == DiscoveryState::Complete && p.failed == 0;
    let mut meter = meter(
        &p.discovery,
        Some(completed),
        known
            .then(|| p.discovery.entries().map(|total| (completed, total)))
            .flatten(),
        p.state == OperationState::Completed,
        p.state.is_terminal(),
        locale,
    );
    if known && !p.pause_requested && !p.paused && p.state == OperationState::Running {
        meter.eta = eta(
            estimate_seconds(
                p.discovery.entries().unwrap_or(0).saturating_sub(completed) as f64,
                p.throughput.items_per_second,
            ),
            locale,
        );
    }
    meter
}

fn meter(
    discovery: &OperationDiscovery,
    completed: Option<u64>,
    work: Option<(u64, u64)>,
    success: bool,
    terminal: bool,
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
        output.label = format!(
            "Discovering: {} items{}",
            locale.format_u64(discovery.entries().unwrap_or(0)),
            discovery.bytes.map_or(String::new(), |bytes| format!(
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
        output.label = format!("{}% complete", (output.fraction * 100.0).floor() as u32).into();
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

fn estimate_seconds(remaining: f64, rate: f64) -> Option<u64> {
    if remaining <= 0.0 || !remaining.is_finite() || !rate.is_finite() || rate <= 0.0 {
        return None;
    }
    let seconds = (remaining / rate).ceil();
    (seconds.is_finite() && seconds <= u64::MAX as f64).then_some(seconds as u64)
}
fn eta(seconds: Option<u64>, locale: &NumberLocale) -> SharedString {
    seconds.map_or_else(SharedString::default, |seconds| {
        if seconds < 60 {
            format!("About {} s", locale.format_u64(seconds))
        } else if seconds < 3600 {
            format!("About {} min", locale.format_u64(seconds.div_ceil(60)))
        } else {
            format!("About {} h", locale.format_u64(seconds.div_ceil(3600)))
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
            transfer_per_second: 100.0,
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
        p.pause_requested = true;
        assert!(copy(&p, &NumberLocale::default()).eta.is_empty());
        p.pause_requested = false;
        p.discovery.state = DiscoveryState::Unavailable;
        assert!(!copy(&p, &NumberLocale::default()).determinate);
        assert!(copy(&p, &NumberLocale::default()).eta.is_empty());
        p.discovery.state = DiscoveryState::Complete;
        p.failed = 1;
        assert!(!copy(&p, &NumberLocale::default()).determinate);
        assert!(copy(&p, &NumberLocale::default()).eta.is_empty());
    }
}

#[cfg(test)]
#[path = "job_progress_eta_tests.rs"]
mod eta_tests;
