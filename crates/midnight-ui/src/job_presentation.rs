//! Bounded job snapshots and selection; paths and counters are never read from the filesystem here.
use crate::*;

#[path = "job_file_progress.rs"]
mod file_meter;
#[path = "job_progress_meter.rs"]
mod meter;
#[path = "job_outcome.rs"]
mod outcome;

#[cfg(test)]
#[path = "job_presentation_tests.rs"]
mod tests;

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
    use crate::job_row_cache::Key;
    p.job_rows.retain(
        p.copies.progress.iter().map(|job| Key::Copy(job.id)).chain(
            p.operations
                .progress
                .iter()
                .map(|job| Key::Mutation(job.id)),
        ),
    );
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
        .map(|progress| (progress.id, cached_mutation(p, progress)))
        .chain(
            p.copies
                .progress
                .iter()
                .map(|progress| (progress.id, cached_copy(p, progress))),
        )
        .collect();
    rows.sort_unstable_by_key(|(id, _)| std::cmp::Reverse(*id));
    crate::user_command_presentation::job_rows(p)
        .into_iter()
        .chain(rows.into_iter().map(|(_, row)| row))
        .collect()
}

fn cached_copy(p: &Presenter, progress: &std::sync::Arc<CopyProgress>) -> OperationRow {
    let failure = p
        .copies
        .failures
        .iter()
        .find(|failure| failure.id == progress.id);
    let mut row = p.job_rows.get(
        crate::job_row_cache::Snapshot::Copy(progress, failure),
        &p.number_locale,
        || copy(p, progress),
    );
    // Icons and conflicts arrive independently of progress snapshots. Theme styling
    // remains a Slint binding; never freeze asynchronous artwork or decision state.
    row.icon = p.desktop.image(
        &IconKey::action(copy_artwork(progress.intent)),
        IconPlace::Action,
    );
    row.attention = copy_attention(p, progress.id);
    row
}
fn cached_mutation(p: &Presenter, progress: &std::sync::Arc<OperationProgress>) -> OperationRow {
    let metadata = p.operations.metadata_results.get(&progress.id);
    let mut row = p.job_rows.get(
        crate::job_row_cache::Snapshot::Mutation(progress, metadata),
        &p.number_locale,
        || mutation(p, progress),
    );
    row.icon = p.desktop.image(
        &IconKey::action(mutation_artwork(progress.kind).0),
        IconPlace::Action,
    );
    row.attention = mutation_attention(p, progress.id);
    row
}
fn copy_attention(p: &Presenter, id: OperationId) -> bool {
    p.copies.conflicts.iter().any(|conflict| conflict.id == id)
        || p.copies.failures.iter().any(|failure| failure.id == id)
}
/// A change waits for the user: a metadata or Rename conflict, or a Delete failure,
/// shown or deferred with Decide later. Resolve on the row brings it up.
fn mutation_attention(p: &Presenter, id: OperationId) -> bool {
    p.operations
        .metadata_conflicts
        .iter()
        .any(|conflict| conflict.id == id)
        || p.operations
            .rename_conflicts
            .iter()
            .any(|conflict| conflict.id == id)
        || p.operations
            .delete_decisions
            .failures
            .iter()
            .any(|failure| failure.id == id)
}
fn copy_artwork(intent: TransferIntent) -> ActionIcon {
    if intent == TransferIntent::Move {
        ActionIcon::Move
    } else {
        ActionIcon::Copy
    }
}
fn mutation_artwork(kind: MutationKind) -> (ActionIcon, i32) {
    match kind {
        MutationKind::Rename => (ActionIcon::Rename, 2),
        MutationKind::EditMetadata => (ActionIcon::Properties, 5),
        MutationKind::CreateFile => (ActionIcon::NewFile, 4),
        MutationKind::CreateDirectory => (ActionIcon::NewFolder, 3),
        MutationKind::Delete(_) => (ActionIcon::Delete, 1),
    }
}

/// The Jobs list names a pause as the progress window does, optionally with the
/// phase that the pause holds: "Paused · verifying SHA-256".
fn pause_state(state: &str, phase: bool, paused: bool, pause_requested: bool) -> String {
    match (paused, pause_requested) {
        (true, true) if phase => {
            let mut letters = state.chars();
            let first: String = letters
                .next()
                .into_iter()
                .flat_map(char::to_lowercase)
                .collect();
            format!("Paused · {first}{}", letters.as_str())
        }
        (true, true) => "Paused".into(),
        (true, false) => "Resuming...".into(),
        (false, true) => "Pausing...".into(),
        (false, false) => state.into(),
    }
}

fn mutation(p: &Presenter, progress: &OperationProgress) -> OperationRow {
    let meter = meter::deletion(progress, &p.number_locale);
    let deletion = matches!(progress.kind, MutationKind::Delete(_));
    let active = !progress.state.is_terminal();
    let cancellable = active && progress.state != OperationState::CancelRequested;
    let show_pause = !deletion || cancellable;
    let phase = match progress.state {
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
    // Only a running Delete can pause; its "Running" phase says nothing more.
    let state = if deletion && cancellable {
        pause_state(
            phase,
            progress.state != OperationState::Running,
            progress.paused,
            progress.pause_requested,
        )
    } else {
        phase.to_string()
    };
    let (icon, kind) = mutation_artwork(progress.kind);
    let metadata = p
        .operations
        .metadata_results
        .get(&progress.id)
        .map(|result| metadata_presentation::result_detail(result))
        .unwrap_or_default();
    let title = operation_presentation::title(progress.kind);
    let mut body = operation_detail::detail(progress, &metadata, &p.number_locale);
    append_discovery(&mut body, &progress.discovery, &p.number_locale);
    let mut detail = format!("{title}\n{state}\n\n{body}");
    let message = &progress.message;
    if !message.is_empty() {
        detail.push_str("\n\n");
        detail.push_str(message);
    }
    // The progress window already gives cancellation its own status line.
    let progress_detail = if deletion && progress.state == OperationState::CancelRequested {
        if !message.is_empty() && message.trim_end_matches(['.', '…']) != state {
            body.push_str("\n\n");
            body.push_str(message);
        }
        body.into()
    } else {
        SharedString::default()
    };
    let outcome = match progress.kind {
        MutationKind::Delete(mode) => outcome::deletion(progress, mode, &p.number_locale),
        _ => outcome::change(progress),
    };
    let issues = outcome::mutation_issues(progress, &p.number_locale);
    OperationRow {
        identity: if progress.kind == MutationKind::Rename {
            format!("rename-job:{}", progress.id.0).into()
        } else if progress.kind == MutationKind::EditMetadata {
            format!("metadata-job:{}", progress.id.0).into()
        } else {
            progress.id.0.to_string().into()
        },
        title: title.into(),
        state: state.into(),
        summary: format!(
            "{} / {} · {}",
            p.number_locale.format_u64(progress.completed),
            counted(progress.total, "target", "targets", &p.number_locale),
            counted(progress.failed, "error", "errors", &p.number_locale)
        )
        .into(),
        detail: detail.into(),
        progress_detail,
        cancellable,
        attention: mutation_attention(p, progress.id),
        failed: progress.failed > 0
            || !progress.errors.is_empty()
            || matches!(
                progress.state,
                OperationState::Failed | OperationState::CompletedWithErrors
            ),
        active,
        completed: progress.state == OperationState::Completed,
        pausable: deletion && cancellable,
        pause_requested: show_pause && progress.pause_requested,
        paused: show_pause && progress.paused,
        discovering: meter.discovering,
        current_path: progress
            .current_path
            .as_ref()
            .map_or_else(SharedString::default, |path| {
                path.display().to_string().into()
            }),
        remaining: meter.remaining,
        eta: meter.eta,
        progress_label: meter.label,
        file_determinate: false,
        file_progress: 0.0,
        file_progress_label: SharedString::default(),
        file_progress_title: SharedString::default(),
        file_percent: SharedString::default(),
        verification_view: false,
        waiting: meter.waiting,
        activity_label: if deletion && !active {
            phase.into()
        } else {
            meter.activity_label.into()
        },
        determinate: meter.determinate,
        progress: meter.fraction,
        icon: p.desktop.image(&IconKey::action(icon), IconPlace::Action),
        transfer_directory: Default::default(),
        transfer_name: Default::default(),
        // "Items deleted: 3 of 10" for a Delete; nothing for other changes.
        transfer_totals: meter.totals,
        transfer_outcome: outcome.into(),
        issues: issues.rows,
        issue_summary: issues.summary,
        transfer_speed_chart: Default::default(),
        speed_chart: if deletion {
            p.job_charts
                .get(progress.id, true, &progress.throughput, &p.number_locale)
        } else {
            Default::default()
        },
        kind,
    }
}

fn copy(p: &Presenter, progress: &CopyProgress) -> OperationRow {
    let meter = meter::copy(progress, &p.number_locale);
    let file = file_meter::copy(progress, &p.number_locale);
    let verification_view = file.verifying && !progress.state.is_terminal();
    use CopyOperationState as S;
    let moving = progress.intent == TransferIntent::Move;
    let title = if moving { "Move" } else { "Copy" };
    let waiting = meter::copy_waiting_label(progress);
    let state = if let Some(label) = waiting {
        label
    } else {
        match progress.state {
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
                CopyOperationPhase::Publishing => {
                    if moving {
                        "Moving"
                    } else {
                        "Copying"
                    }
                }
                CopyOperationPhase::ValidatingBeforeMove => "Checking before Move",
                CopyOperationPhase::CapturingSources => "Preparing source removal",
                CopyOperationPhase::ValidatingAfterMove => "Checking before source removal",
            },
            S::WaitingForDecision => "Needs a decision",
            S::CancelRequested => "Cancelling",
            S::Cancelled => "Cancelled",
            S::Completed if progress.closure.is_some() || progress.skipped > 0 => {
                "Completed with warnings"
            }
            S::Completed => "Completed",
            S::CompletedWithErrors => "Completed with errors",
            S::Failed => "Failed",
        }
    };
    let state = if progress.state.is_terminal()
        || matches!(progress.state, S::CancelRequested | S::WaitingForDecision)
    {
        state.to_string()
    } else {
        pause_state(state, true, progress.paused, progress.pause_requested)
    };
    let mut summary = format!(
        "{} · {} skipped · {}",
        counted(progress.files_completed, "file", "files", &p.number_locale),
        p.number_locale.format_u64(progress.skipped),
        counted(progress.failed, "error", "errors", &p.number_locale)
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
    let locale = &p.number_locale;
    let mut detail = format!(
        "{title}\n{state}\n\n{summary}\n{} created\n{} copied\n{} verified ({verification_label})",
        counted(progress.directories_created, "folder", "folders", locale),
        counted(progress.copied_bytes, "byte", "bytes", locale),
        counted(progress.verified_bytes, "byte", "bytes", locale)
    );
    if moving {
        let residuals = progress.source_residual_count;
        detail.push_str(&format!(
            "\n{} checked\n{} removed\n{} {} attention",
            counted(
                progress.validation_bytes,
                "additional source/target byte",
                "additional source/target bytes",
                locale
            ),
            counted(
                progress.source_files_removed,
                "source file",
                "source files",
                locale
            ),
            counted(
                residuals,
                "temporary source location",
                "temporary source locations",
                locale
            ),
            if residuals == 1 { "needs" } else { "need" }
        ));
        if progress.source_delete_pending > 0 {
            detail.push_str(&format!(
                "\n{} accepted by Windows; open handles may delay storage release",
                counted(
                    progress.source_delete_pending,
                    "source removal",
                    "source removals",
                    locale
                )
            ));
        }
        if progress.source_removal_durability_unconfirmed > 0 {
            detail.push_str(&format!(
                "\n{} without a confirmed directory durability flush",
                counted(
                    progress.source_removal_durability_unconfirmed,
                    "source namespace change",
                    "source namespace changes",
                    locale
                )
            ));
        }
        append_move_residuals(&mut detail, progress);
    }
    if let Some(path) = &progress.current_path {
        detail.push_str(&format!("\n\nCurrent source\n{}", path.display()));
    }
    if let Some(path) = &progress.current_destination {
        detail.push_str(&format!("\n\nCurrent destination\n{}", path.display()));
    }
    detail.push_str(&format!(
        "\n\nDestination\n{}",
        progress.destination_directory.display()
    ));
    append_discovery(&mut detail, &progress.discovery, &p.number_locale);
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
    append_recovery(&mut detail, progress, &p.number_locale);
    append_notes(&mut detail, progress, &p.number_locale);
    let failed = (progress.state.is_terminal()
        && (progress.source_residual_count > 0
            || progress.source_removal_durability_unconfirmed > 0))
        || failure.is_some()
        || progress.failed > 0
        || !progress.errors.is_empty()
        || matches!(progress.state, S::Failed | S::CompletedWithErrors);
    let transfer_outcome = outcome::transfer(progress, failed, &p.number_locale);
    if progress.closure.is_some() {
        // Copy details repeat how the job ended, as its window says it.
        detail.push_str("\n\n");
        detail.push_str(&transfer_outcome);
        append_omitted_errors(&mut detail, progress, &p.number_locale);
    }
    let issues = outcome::transfer_issues(progress, &p.number_locale);
    let (transfer_directory, transfer_name) = progress
        .current_path
        .as_deref()
        .map(copy_presentation::path_parts)
        .unwrap_or_default();
    let transfer_speed_chart =
        p.job_charts
            .get(progress.id, false, &progress.throughput, &p.number_locale);
    OperationRow {
        identity: format!("copy-job:{}", progress.id.0).into(),
        title: title.into(),
        state: state.into(),
        summary: summary.clone().into(),
        detail: detail.into(),
        progress_detail: SharedString::default(),
        cancellable: !progress.state.is_terminal() && progress.state != S::CancelRequested,
        attention: copy_attention(p, progress.id),
        failed,
        active: !progress.state.is_terminal(),
        completed: progress.state == S::Completed,
        pausable: !progress.state.is_terminal() && progress.state != S::CancelRequested,
        pause_requested: progress.pause_requested,
        paused: progress.paused,
        discovering: meter.discovering,
        current_path: progress
            .current_path
            .as_ref()
            .map_or_else(SharedString::default, |path| {
                path.display().to_string().into()
            }),
        remaining: meter.remaining,
        eta: meter.eta,
        determinate: meter.determinate,
        progress: meter.fraction,
        progress_label: meter.label,
        file_determinate: file.determinate,
        file_progress: file.fraction,
        file_progress_label: file.label,
        file_progress_title: file.title,
        file_percent: file.percent,
        verification_view,
        waiting: waiting.is_some(),
        activity_label: copy_activity(progress, meter.discovering).into(),
        icon: p.desktop.image(
            &IconKey::action(copy_artwork(progress.intent)),
            IconPlace::Action,
        ),
        kind: 0,
        transfer_directory,
        transfer_name,
        transfer_totals: meter.totals,
        transfer_outcome: transfer_outcome.into(),
        issues: issues.rows,
        issue_summary: issues.summary,
        speed_chart: if !progress.state.is_terminal()
            && progress.phase == CopyOperationPhase::Verifying
        {
            if verification_view {
                p.job_charts.get_verification(
                    progress.id,
                    progress.verification_pass_id,
                    &progress.verification_throughput,
                    &p.number_locale,
                )
            } else {
                // A phase alone does not identify a copied-file readback attempt.
                // Never substitute compressed whole-operation history here.
                Default::default()
            }
        } else {
            transfer_speed_chart.clone()
        },
        transfer_speed_chart,
    }
}

fn copy_activity(progress: &CopyProgress, discovering: bool) -> &'static str {
    use CopyOperationState as S;
    if let Some(label) = meter::copy_status_label(progress) {
        return label;
    }
    if progress.state.is_terminal() {
        return "";
    }
    if discovering && matches!(progress.state, S::Preparing | S::Running) {
        return "Discovering items...";
    }
    match progress.state {
        CopyOperationState::Preparing => "Preparing...",
        CopyOperationState::Running => match progress.phase {
            CopyOperationPhase::Planning => "Preparing...",
            CopyOperationPhase::Copying => "Copying...",
            CopyOperationPhase::Syncing => "Saving...",
            CopyOperationPhase::Verifying => "Verifying current file...",
            CopyOperationPhase::Publishing => {
                if progress.intent == TransferIntent::Move {
                    "Moving..."
                } else {
                    "Copying..."
                }
            }
            CopyOperationPhase::ValidatingBeforeMove | CopyOperationPhase::ValidatingAfterMove => {
                "Checking Move..."
            }
            CopyOperationPhase::CapturingSources => "Preparing source removal...",
        },
        _ => "",
    }
}

/// Informational notes, such as a link copied as a link because it could not be
/// followed. They are not errors and do not change the job's outcome.
fn append_notes(detail: &mut String, progress: &CopyProgress, locale: &NumberLocale) {
    if progress.note_count == 0 {
        return;
    }
    detail.push_str("\n\nNotes");
    for note in &progress.notes {
        detail.push_str(&format!("\n{note}"));
    }
    let omitted = progress
        .note_count
        .saturating_sub(progress.notes.len() as u64);
    if omitted > 0 {
        detail.push_str(&format!(
            "\n{} more {} not shown.",
            locale.format_u64(omitted),
            if omitted == 1 { "note is" } else { "notes are" }
        ));
    }
}

fn append_omitted_errors(text: &mut String, progress: &CopyProgress, locale: &NumberLocale) {
    if let Some(receipt) = progress.closure {
        match receipt.omitted_error_details {
            0 => {}
            1 => text.push_str("\n\n1 more error detail was not retained."),
            count => text.push_str(&format!(
                "\n\n{} more error details were not retained.",
                locale.format_u64(count)
            )),
        }
    }
}

fn append_move_residuals(detail: &mut String, progress: &CopyProgress) {
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

/// A count with the noun that agrees with it: "1 file", "0 files", "2 errors".
/// `count` and its noun, singular for exactly one.
pub(crate) fn counted(count: u64, singular: &str, plural: &str, locale: &NumberLocale) -> String {
    let noun = if count == 1 { singular } else { plural };
    format!("{} {noun}", locale.format_u64(count))
}

fn append_recovery(detail: &mut String, progress: &CopyProgress, locale: &NumberLocale) {
    if progress.recovery_count == 0 {
        return;
    }
    detail.push_str(&format!(
        "\n\n{} retained",
        counted(
            progress.recovery_count,
            "recovery file",
            "recovery files",
            locale
        )
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

fn append_discovery(detail: &mut String, discovery: &OperationDiscovery, locale: &NumberLocale) {
    if discovery.state == DiscoveryState::Complete {
        let scope = match discovery.scope {
            DiscoveryScope::Recursive => "Selected items and discovered folder contents",
            DiscoveryScope::SelectedRoots => {
                "Selected items only; folder contents were not counted"
            }
        };
        let bytes = discovery.bytes.map_or_else(
            || "Unknown".to_owned(),
            |bytes| {
                format!(
                    "{} ({})",
                    counted(bytes, "byte", "bytes", locale),
                    format_size(bytes, locale)
                )
            },
        );
        detail.push_str(&format!(
            "\n\nDiscovered totals\nScope: {scope}\nFiles and other entries: {}\nFolders: {}\nLogical file bytes: {bytes}",
            locale.format_u64(discovery.files),
            locale.format_u64(discovery.directories)
        ));
    }
    if !discovery.message.is_empty() {
        detail.push_str(&format!("\n\nDiscovery\n{}", discovery.message));
    }
}
