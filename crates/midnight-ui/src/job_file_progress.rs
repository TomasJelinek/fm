//! Per-file byte presentation; readback never implies publication succeeded.

use midnight_core::{CopyOperationPhase, CopyProgress, NumberLocale};
use slint::SharedString;

#[derive(Default)]
pub(super) struct Meter {
    pub determinate: bool,
    pub fraction: f32,
    pub label: SharedString,
    pub title: SharedString,
    pub verifying: bool,
    /// "45%", or "Verifying 30%" during readback, for every current file of a running
    /// job: its window shows the file's own bar under the job's chart, always in the same
    /// place, so a quick file simply passes through to 100%.
    pub percent: SharedString,
}

pub(super) fn copy(p: &CopyProgress, locale: &NumberLocale) -> Meter {
    let verifying = p.phase == CopyOperationPhase::Verifying && p.verification_pass_id != 0;
    let writing = matches!(
        p.phase,
        CopyOperationPhase::Copying | CopyOperationPhase::Syncing | CopyOperationPhase::Publishing
    );
    if !verifying && !writing {
        return Meter::default();
    }
    let title = if verifying {
        "Current file verification"
    } else {
        "Current file written"
    };
    let Some(size) = p.current_file_size else {
        return Meter {
            title: title.into(),
            label: if verifying {
                "Verifying current file".into()
            } else {
                SharedString::default()
            },
            ..Default::default()
        };
    };
    let (done, total) = if verifying {
        (p.current_file_verified, p.verification.readback_bytes(size))
    } else {
        (p.current_file_copied, size)
    };
    if total == 0 {
        // Nothing to write or read back: the file's own bar is full at once.
        let running = !p.state.is_terminal();
        return Meter {
            verifying,
            title: title.into(),
            label: if verifying {
                "Verifying empty file".into()
            } else {
                "Current file is empty".into()
            },
            fraction: if running { 1.0 } else { 0.0 },
            percent: match (running, verifying) {
                (false, _) => SharedString::default(),
                (true, false) => "100%".into(),
                (true, true) => "Verifying 100%".into(),
            },
            ..Default::default()
        };
    }
    let caption = if verifying {
        if p.verification == midnight_core::TransferVerification::SizeAndSamples {
            "Verifying samples"
        } else {
            "Verifying"
        }
    } else {
        "Current file written"
    };
    let running = !p.state.is_terminal();
    // Whole percent, rounded down: a file never shows 100% before its last byte.
    let percent = (u128::from(done.min(total)) * 100 / u128::from(total)) as u64;
    Meter {
        determinate: true,
        verifying,
        title: title.into(),
        fraction: (done as f64 / total as f64).min(1.0) as f32,
        label: format!(
            "{caption}: {} / {} bytes",
            locale.format_u64(done),
            locale.format_u64(total)
        )
        .into(),
        percent: match (running, verifying) {
            (false, _) => SharedString::default(),
            (true, false) => format!("{percent}%").into(),
            (true, true) => format!("Verifying {percent}%").into(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{CopyOperationState, OperationId, TransferIntent, TransferVerification};

    fn file(size: Option<u64>) -> CopyProgress {
        let mut p = CopyProgress::queued(OperationId(1), "target".into(), 1, TransferIntent::Copy);
        p.state = CopyOperationState::Running;
        p.phase = CopyOperationPhase::Copying;
        p.verification_pass_id = 1;
        p.current_file_size = size;
        p
    }

    #[test]
    fn same_line_switches_from_writes_to_fresh_readback_and_back_for_next_file() {
        let locale = NumberLocale::default();
        let mut p = file(Some(1000));
        p.current_file_copied = 1000;
        let written = copy(&p, &locale);
        assert_eq!(written.fraction, 1.0);
        assert!(!written.verifying);
        p.phase = CopyOperationPhase::Verifying;
        let start = copy(&p, &locale);
        assert_eq!(start.fraction, 0.0);
        assert_eq!(start.label, "Verifying: 0 / 1000 bytes");
        p.current_file_verified = 250;
        let next = copy(&p, &locale);
        assert_eq!(next.fraction, 0.25);
        assert_eq!(next.label, "Verifying: 250 / 1000 bytes");
        p.paused = true;
        assert_eq!(copy(&p, &locale).fraction, 0.25);
        p.paused = false;
        p.phase = CopyOperationPhase::Copying;
        p.current_file_size = Some(2000);
        p.current_file_copied = 100;
        p.current_file_verified = 0;
        let next_file = copy(&p, &locale);
        assert!(!next_file.verifying);
        assert_eq!(next_file.fraction, 0.05);
        assert_eq!(next_file.label, "Current file written: 100 / 2000 bytes");
    }

    #[test]
    fn quick_readback_uses_sample_budget_and_full_sha_uses_file_length() {
        let locale = NumberLocale::default();
        let mut p = file(Some(1_000_000));
        p.phase = CopyOperationPhase::Verifying;
        p.verification = TransferVerification::SizeAndSamples;
        p.current_file_verified = 98_304;
        assert_eq!(copy(&p, &locale).fraction, 0.5);
        assert_eq!(
            copy(&p, &locale).label,
            format!(
                "Verifying samples: {} / {} bytes",
                locale.format_u64(98_304),
                locale.format_u64(196_608)
            )
        );
        p.verification = TransferVerification::FullSha256;
        assert_eq!(copy(&p, &locale).fraction, 0.098304);
    }

    #[test]
    fn empty_unknown_and_namespace_only_move_show_status_without_readback_percentage() {
        let locale = NumberLocale::default();
        for size in [None, Some(0)] {
            let mut p = file(size);
            p.phase = CopyOperationPhase::Verifying;
            let status = copy(&p, &locale);
            assert!(!status.determinate);
            assert_eq!(status.verifying, size.is_some());
            assert!(!status.label.contains('/'));
            // An empty file's own bar is full at once; an unknown length has none.
            assert_eq!(status.fraction, if size.is_some() { 1.0 } else { 0.0 });
        }
        let mut p = file(None);
        p.intent = TransferIntent::Move;
        p.phase = CopyOperationPhase::CapturingSources;
        p.native_entries_moved = 1;
        let renamed = copy(&p, &locale);
        assert!(!renamed.verifying && !renamed.determinate);
        assert!(renamed.label.is_empty());
    }

    #[test]
    fn completed_readback_keeps_failure_and_move_proofs_outside_byte_receipt() {
        let locale = NumberLocale::default();
        let mut p = file(Some(1000));
        p.phase = CopyOperationPhase::Verifying;
        p.current_file_verified = 1000;
        p.state = CopyOperationState::Failed;
        let failed = copy(&p, &locale);
        assert_eq!(failed.fraction, 1.0);
        assert!(!failed.label.contains("complete"));
        p.phase = CopyOperationPhase::ValidatingAfterMove;
        p.validation_bytes = 4000;
        assert!(!copy(&p, &locale).verifying);
        assert!(!copy(&p, &locale).determinate);
    }

    #[test]
    fn a_missing_attempt_never_turns_retained_counters_into_verification() {
        let mut p = file(Some(1000));
        p.phase = CopyOperationPhase::Verifying;
        p.verification_pass_id = 0;
        p.current_file_copied = 1000;
        p.current_file_verified = 1000;
        let status = copy(&p, &NumberLocale::default());
        assert!(!status.verifying && !status.determinate);
        assert_eq!(status.fraction, 0.0);
        assert!(status.label.is_empty());
    }

    #[test]
    fn retry_identity_does_not_reset_a_retained_verification_checkpoint() {
        let mut p = file(Some(1000));
        p.phase = CopyOperationPhase::Verifying;
        p.current_file_verified = 750;
        p.verification_pass_id = 9;
        let locale = NumberLocale::default();
        assert_eq!(copy(&p, &locale).fraction, 0.75);
        p.verification_pass_id = 10;
        p.verification_throughput = std::sync::Arc::default();
        let retried = copy(&p, &locale);
        assert_eq!(retried.fraction, 0.75);
        assert_eq!(retried.label, "Verifying: 750 / 1000 bytes");
    }

    #[test]
    fn every_current_file_shows_its_own_percentage() {
        let locale = NumberLocale::default();
        let mut p = file(Some(1000));
        p.current_file_copied = 456;
        // However quickly it goes: the bar is always there, so it never appears late.
        p.current_file_active_millis = 0;
        let quick = copy(&p, &locale);
        assert_eq!(quick.percent, "45%");
        assert_eq!(quick.fraction, 0.456);
        p.current_file_active_millis = 2000;
        let slow = copy(&p, &locale);
        assert_eq!(slow.percent, "45%");
        assert_eq!(slow.fraction, 0.456);
        p.phase = CopyOperationPhase::Verifying;
        p.current_file_copied = 1000;
        p.current_file_verified = 300;
        assert_eq!(copy(&p, &locale).percent, "Verifying 30%");
        p.paused = true;
        assert_eq!(
            copy(&p, &locale).percent,
            "Verifying 30%",
            "a pause keeps it"
        );
        p.paused = false;
        // The next file shows its own share at once.
        p.phase = CopyOperationPhase::Copying;
        p.verification_pass_id = 2;
        p.current_file_active_millis = 0;
        p.current_file_copied = 999;
        assert_eq!(copy(&p, &locale).percent, "99%", "never 100% too early");
        p.current_file_size = None;
        assert!(copy(&p, &locale).percent.is_empty(), "no length, no share");
        p.current_file_size = Some(0);
        assert_eq!(
            copy(&p, &locale).percent,
            "100%",
            "an empty file is done at once"
        );
        p.current_file_size = Some(1000);
        p.state = CopyOperationState::Failed;
        assert!(
            copy(&p, &locale).percent.is_empty(),
            "a finished job has no bar"
        );
    }

    #[test]
    fn an_empty_write_uses_status_instead_of_zero_over_zero() {
        let p = file(Some(0));
        let status = copy(&p, &NumberLocale::default());
        assert!(!status.verifying && !status.determinate);
        assert_eq!(status.fraction, 1.0);
        assert_eq!(status.label, "Current file is empty");
    }
}
