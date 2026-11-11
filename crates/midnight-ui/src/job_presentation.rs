//! Bounded job snapshots and selection; paths and counters are never read from the filesystem here.
use crate::*;

#[cfg(test)]
#[path = "job_presentation_tests.rs"]
mod tests;

pub(crate) fn install(window: &MainWindow) {
    window.global::<JobsView>().on_find(find);
    window.global::<JobsView>().on_has_finished(has_finished);
}

pub(crate) fn has_finished(rows: ModelRc<OperationRow>, without_errors: bool) -> bool {
    rows.iter()
        .any(|row| !row.active && (!without_errors || !row.failed))
}

pub(crate) fn find(rows: ModelRc<OperationRow>, identity: SharedString) -> i32 {
    rows.iter()
        .position(|row| row.identity == identity)
        .map_or(if rows.row_count() == 0 { -1 } else { 0 }, |index| {
            index as i32
        })
}

pub(crate) fn rows(p: &Presenter) -> Vec<OperationRow> {
    p.job_charts.retain(
        p.copies.progress.iter().map(|job| (false, job.id)).chain(
            p.operations
                .progress
                .iter()
                .filter(|job| matches!(job.kind, MutationKind::Delete(_)))
                .map(|job| (true, job.id)),
        ),
    );
    let mut rows: Vec<_> = p
        .operations
        .progress
        .iter()
        .filter(|progress| !p.operations.unconfirmed.contains(&progress.id))
        .map(|progress| (progress.id, mutation(p, progress)))
        .chain(
            p.copies
                .progress
                .iter()
                .map(|progress| (progress.id, copy(p, progress))),
        )
        .collect();
    rows.sort_unstable_by_key(|(id, _)| std::cmp::Reverse(*id));
    crate::user_command_presentation::job_rows(p)
        .into_iter()
        .chain(rows.into_iter().map(|(_, row)| row))
        .collect()
}

fn mutation(p: &Presenter, progress: &OperationProgress) -> OperationRow {
    let state = match progress.state {
        OperationState::Preparing => "Preparing",
        OperationState::AwaitingConfirmation => "Awaiting confirmation",
        OperationState::Queued => "Queued",
        OperationState::Running => "Running",
        OperationState::CancelRequested => "Cancelling",
        OperationState::Cancelled => "Cancelled",
        OperationState::Completed => "Completed",
        OperationState::CompletedWithErrors => "Completed with errors",
        OperationState::Failed => "Failed",
    };
    let (icon, kind) = match progress.kind {
        MutationKind::Rename => (ActionIcon::Rename, 2),
        MutationKind::EditMetadata => (ActionIcon::Properties, 5),
        MutationKind::CreateFile => (ActionIcon::NewFile, 4),
        MutationKind::CreateDirectory => (ActionIcon::NewFolder, 3),
        MutationKind::Delete(_) => (ActionIcon::Delete, 1),
    };
    let metadata = p
        .operations
        .metadata_results
        .iter()
        .find(|result| result.id == progress.id)
        .map(|result| metadata_presentation::result_detail(result))
        .unwrap_or_default();
    let mut detail = format!(
        "{}\n{state}\n\n{}",
        operation_presentation::title(progress.kind),
        operation_detail::detail(progress, &metadata)
    );
    if !progress.message.is_empty() {
        detail.push_str(&format!("\n\n{}", progress.message));
    }
    OperationRow {
        identity: if progress.kind == MutationKind::Rename {
            format!("rename-job:{}", progress.id.0).into()
        } else if progress.kind == MutationKind::EditMetadata {
            format!("metadata-job:{}", progress.id.0).into()
        } else {
            progress.id.0.to_string().into()
        },
        title: operation_presentation::title(progress.kind).into(),
        state: state.into(),
        summary: format!(
            "{} / {} targets · {} errors",
            p.number_locale.format_u64(progress.completed),
            p.number_locale.format_u64(progress.total),
            p.number_locale.format_u64(progress.failed)
        )
        .into(),
        detail: detail.into(),
        cancellable: !progress.state.is_terminal()
            && progress.state != OperationState::CancelRequested,
        attention: p
            .operations
            .metadata_conflicts
            .iter()
            .any(|conflict| conflict.id == progress.id)
            || p.operations
                .rename_conflicts
                .iter()
                .any(|conflict| conflict.id == progress.id),
        failed: progress.failed > 0
            || !progress.errors.is_empty()
            || matches!(
                progress.state,
                OperationState::Failed | OperationState::CompletedWithErrors
            ),
        active: !progress.state.is_terminal(),
        determinate: progress.total > 0,
        progress: if progress.total == 0 {
            0.0
        } else {
            ((progress.completed.saturating_add(progress.failed) as f64 / progress.total as f64)
                .min(1.0)) as f32
        },
        icon: p.desktop.image(&IconKey::action(icon)),
        speed_chart: if matches!(progress.kind, MutationKind::Delete(_)) {
            p.job_charts
                .get(progress.id, true, &progress.throughput, &p.number_locale)
        } else {
            Default::default()
        },
        kind,
    }
}

fn copy(p: &Presenter, progress: &CopyProgress) -> OperationRow {
    use CopyOperationState as S;
    let moving = progress.intent == TransferIntent::Move;
    let title = if moving { "Move" } else { "Copy" };
    let state = match progress.state {
        S::Queued => "Queued",
        S::Preparing => "Preparing",
        S::Running => match progress.phase {
            CopyOperationPhase::Planning => "Reading folders",
            CopyOperationPhase::Copying => "Copying",
            CopyOperationPhase::Syncing => "Saving",
            CopyOperationPhase::Verifying => {
                if progress.verification == TransferVerification::FullSha256 {
                    "Verifying SHA-256"
                } else {
                    "Verifying size and samples"
                }
            }
            CopyOperationPhase::Publishing => "Finishing",
            CopyOperationPhase::ValidatingBeforeMove => "Checking before Move",
            CopyOperationPhase::CapturingSources => "Preparing source removal",
            CopyOperationPhase::ValidatingAfterMove => "Checking before source removal",
        },
        S::WaitingForDecision => "Needs a decision",
        S::CancelRequested => "Cancelling",
        S::Cancelled => "Cancelled",
        S::Completed => "Completed",
        S::CompletedWithErrors => "Completed with errors",
        S::Failed => "Failed",
    };
    let mut summary = format!(
        "{} files · {} skipped · {} errors",
        p.number_locale.format_u64(progress.files_completed),
        p.number_locale.format_u64(progress.skipped),
        p.number_locale.format_u64(progress.failed)
    );
    if progress.native_entries_moved != 0 {
        summary = format!(
            "{} renamed · {summary}",
            p.number_locale.format_u64(progress.native_entries_moved)
        );
    }
    let verification_label = if progress.verification == TransferVerification::FullSha256 {
        "SHA-256"
    } else {
        "size + first/middle/last 64 KiB; gaps not checked"
    };
    let mut detail = format!(
        "{title}\n{state}\n\n{summary}\n{} folders created\n{} bytes copied\n{} bytes verified ({verification_label})",
        p.number_locale.format_u64(progress.directories_created),
        p.number_locale.format_u64(progress.copied_bytes),
        p.number_locale.format_u64(progress.verified_bytes)
    );
    if moving {
        detail.push_str(&format!("\n{} additional source/target bytes checked\n{} source files removed\n{} temporary source locations need attention",
            p.number_locale.format_u64(progress.validation_bytes),
            p.number_locale.format_u64(progress.source_files_removed),
            p.number_locale.format_u64(progress.source_residual_count)));
        if progress.source_delete_pending > 0 {
            detail.push_str(&format!(
                "\n{} source removals accepted by Windows; open handles may delay storage release",
                p.number_locale.format_u64(progress.source_delete_pending)
            ));
        }
        if progress.source_removal_durability_unconfirmed > 0 {
            detail.push_str(&format!(
                "\n{} source namespace changes without a confirmed directory durability flush",
                p.number_locale
                    .format_u64(progress.source_removal_durability_unconfirmed)
            ));
        }
        for recovery in &progress.source_residuals {
            detail.push_str(&format!(
                "\n\nOriginal source\n{}\nTemporary location\n{}\n{}",
                recovery.source.display(),
                recovery.temporary.path.display(),
                if recovery.matched_source {
                    "Source restoration failed; original retained here"
                } else {
                    "Temporary cleanup failed; inspect this location"
                }
            ));
        }
        if progress.source_residual_count > progress.source_residuals.len() as u64 {
            detail.push_str("\nAdditional temporary source locations need attention.");
        }
    }
    if let Some(path) = &progress.current_path {
        detail.push_str(&format!("\n\nCurrent source\n{}", path.display()));
    }
    detail.push_str(&format!(
        "\n\nDestination\n{}",
        progress.destination_directory.display()
    ));
    if !progress.message.is_empty() {
        detail.push_str(&format!("\n\n{}", progress.message));
    }
    error_presentation::append_issues(&mut detail, &progress.errors);
    let failure = p
        .copies
        .failures
        .iter()
        .find(|failure| failure.id == progress.id);
    if let Some(failure) = failure {
        detail.push_str("\n\nWaiting for your decision\n\n");
        detail.push_str(&copy_presentation::failure_details(failure));
    }
    append_recovery(&mut detail, progress, p);
    OperationRow {
        identity: format!("copy-job:{}", progress.id.0).into(),
        title: title.into(),
        state: state.into(),
        summary: summary.into(),
        detail: detail.into(),
        cancellable: !progress.state.is_terminal() && progress.state != S::CancelRequested,
        attention: p
            .copies
            .conflicts
            .iter()
            .any(|conflict| conflict.id == progress.id)
            || failure.is_some(),
        failed: (progress.state.is_terminal()
            && (progress.source_residual_count > 0
                || progress.source_removal_durability_unconfirmed > 0))
            || failure.is_some()
            || progress.failed > 0
            || !progress.errors.is_empty()
            || matches!(progress.state, S::Failed | S::CompletedWithErrors),
        active: !progress.state.is_terminal(),
        // Recursive byte totals are unknown. Never present a guessed completion percentage.
        determinate: false,
        progress: 0.0,
        icon: p.desktop.image(&IconKey::action(if moving {
            ActionIcon::Move
        } else {
            ActionIcon::Copy
        })),
        kind: 0,
        speed_chart: p
            .job_charts
            .get(progress.id, false, &progress.throughput, &p.number_locale),
    }
}

fn append_recovery(detail: &mut String, progress: &CopyProgress, p: &Presenter) {
    if progress.recovery_count == 0 {
        return;
    }
    detail.push_str(&format!(
        "\n\n{} recovery files retained",
        p.number_locale.format_u64(progress.recovery_count)
    ));
    for path in &progress.recovery_paths {
        detail.push_str(&format!("\n{}", path.display()));
    }
    if progress.recovery_count > progress.recovery_paths.len() as u64 {
        detail.push_str(
            "\nAdditional displaced destination files are retained in destination recovery folders.",
        );
    }
}
