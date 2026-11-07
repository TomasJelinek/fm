//! Bounded job snapshots and selection; paths and counters are never read from the filesystem here.
use crate::*;

#[cfg(test)]
#[path = "job_presentation_tests.rs"]
mod tests;

pub(crate) fn install(window: &MainWindow) {
    window.global::<JobsView>().on_find(|rows, identity| {
        rows.iter()
            .position(|row| row.identity == identity)
            .map_or(if rows.row_count() == 0 { -1 } else { 0 }, |index| {
                index as i32
            })
    });
}

pub(crate) fn rows(p: &Presenter) -> Vec<OperationRow> {
    let mut rows: Vec<_> = p
        .operations
        .progress
        .iter()
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
        identity: if progress.kind == MutationKind::EditMetadata {
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
        kind,
    }
}

fn copy(p: &Presenter, progress: &CopyProgress) -> OperationRow {
    use CopyOperationState as S;
    let moving = progress.intent == TransferIntent::RecoverableMove;
    let title = if moving {
        "Move (originals retained)"
    } else {
        "Copy"
    };
    let state = match progress.state {
        S::Queued => "Queued",
        S::Preparing => "Preparing",
        S::Running => match progress.phase {
            CopyOperationPhase::Planning => "Reading folders",
            CopyOperationPhase::Copying => "Copying",
            CopyOperationPhase::Syncing => "Saving",
            CopyOperationPhase::Verifying => "Verifying SHA-256",
            CopyOperationPhase::Publishing => "Finishing",
            CopyOperationPhase::ValidatingBeforeMove => "Checking before Move",
            CopyOperationPhase::CapturingSources => "Retaining originals",
            CopyOperationPhase::ValidatingAfterMove => "Checking retained originals",
        },
        S::WaitingForDecision => "Needs a decision",
        S::CancelRequested => "Cancelling",
        S::Cancelled => "Cancelled",
        S::Completed => "Completed",
        S::CompletedWithErrors => "Completed with errors",
        S::Failed => "Failed",
    };
    let summary = format!(
        "{} files · {} skipped · {} errors",
        p.number_locale.format_u64(progress.files_completed),
        p.number_locale.format_u64(progress.skipped),
        p.number_locale.format_u64(progress.failed)
    );
    let mut detail = format!(
        "{title}\n{state}\n\n{summary}\n{} folders created\n{} bytes copied\n{} bytes verified (SHA-256)",
        p.number_locale.format_u64(progress.directories_created),
        p.number_locale.format_u64(progress.copied_bytes),
        p.number_locale.format_u64(progress.verified_bytes)
    );
    if moving {
        detail.push_str(&format!("\n{} additional source/target bytes checked\n{} source names relocated\n{} source recovery objects retained\nSource space is not freed.",
            p.number_locale.format_u64(progress.validation_bytes),
            p.number_locale.format_u64(progress.source_names_captured),
            p.number_locale.format_u64(progress.source_recovery_count)));
        for recovery in &progress.source_recoveries {
            detail.push_str(&format!(
                "\n\nOriginal source\n{}\nRecovery\n{}\n{}",
                recovery.source.display(),
                recovery.recovery.path.display(),
                if recovery.matched_source {
                    "Original identity retained"
                } else {
                    "Identity conflict: inspect recovery"
                }
            ));
        }
        if progress.source_recovery_count > progress.source_recoveries.len() as u64 {
            detail.push_str("\nAdditional originals remain in their source recovery folders.");
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
            .any(|conflict| conflict.id == progress.id),
        failed: progress.failed > 0
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
            "\nAdditional recovery files are retained in the source or destination recovery folders.",
        );
    }
}
