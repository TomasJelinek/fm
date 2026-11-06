//! Presentation of verified copy counters and versioned name conflicts.
use super::*;

pub(super) fn rows(presenter: &Presenter) -> Vec<OperationRow> {
    presenter
        .copies
        .progress
        .iter()
        .rev()
        .map(|progress| {
            use CopyOperationState as S;
            let state = match progress.state {
                S::Queued => "Queued",
                S::Preparing => "Preparing",
                S::Running => match progress.phase {
                    CopyOperationPhase::Planning => "Reading folders",
                    CopyOperationPhase::Copying => "Copying",
                    CopyOperationPhase::Syncing => "Saving",
                    CopyOperationPhase::Verifying => "Verifying SHA-256",
                    CopyOperationPhase::Publishing => "Finishing",
                },
                S::WaitingForDecision => "Needs a decision",
                S::CancelRequested => "Cancelling",
                S::Cancelled => "Cancelled",
                S::Completed => "Completed",
                S::CompletedWithErrors => "Completed with errors",
                S::Failed => "Failed",
            };
            OperationRow {
                identity: format!("copy-job:{}", progress.id.0).into(),
                title: format!("Copy — {state}").into(),
                detail: format!(
                    "{} files · {} skipped · {} errors\n{} bytes copied · {} bytes verified\n{}",
                    presenter.number_locale.format_u64(progress.files_completed),
                    presenter.number_locale.format_u64(progress.skipped),
                    presenter.number_locale.format_u64(progress.failed),
                    presenter.number_locale.format_u64(progress.copied_bytes),
                    presenter.number_locale.format_u64(progress.verified_bytes),
                    progress
                        .current_path
                        .as_deref()
                        .unwrap_or(&progress.destination_directory)
                        .display()
                )
                .into(),
                cancellable: !progress.state.is_terminal(),
                attention: presenter
                    .copies
                    .conflicts
                    .iter()
                    .any(|c| c.id == progress.id),
            }
        })
        .collect()
}

pub(super) fn refresh(window: &MainWindow, presenter: &Presenter) {
    if let Some(progress) = presenter.copies.progress.last() {
        let token: SharedString = progress.id.0.to_string().into();
        if window.get_copy_latest() != token {
            window.set_copy_latest(token);
            if !progress.state.is_terminal()
                && !presenter.operations.closing
                && presenter.operations.preview.is_none()
            {
                window.invoke_show_operations();
            }
        }
    }
    let Some(conflict) = presenter.copies.conflicts.first() else {
        if !window.get_copy_token().is_empty() {
            window.set_copy_token("".into());
            window.invoke_close_copy_conflict();
        }
        return;
    };
    if presenter.operations.preview.is_some() {
        return;
    }
    let token: SharedString = format!("copy:{}:{}", conflict.id.0, conflict.version).into();
    if window.get_copy_token() == token {
        return;
    }
    let describe = |item: &MutationItem| {
        let size = item.metadata.size.map_or_else(
            || "Unknown size".into(),
            |size| format!("{} bytes", presenter.number_locale.format_u64(size)),
        );
        let modified = time_presentation::modified(&item.metadata).map_or_else(
            || "Unknown modified time".into(),
            |time| format!("{} {}", time.date, time.time),
        );
        format!("{}\n{size} · {modified}", item.path.display())
    };
    window.set_copy_body(format!("Source\n{}\n\nExisting destination\n{}\n\nChoose another name or skip. Replacement is unavailable.",
        describe(&conflict.source), describe(&conflict.target)).into());
    window.set_copy_name(
        conflict
            .destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .as_ref()
            .into(),
    );
    window.set_copy_token(token);
    window.invoke_show_copy_conflict();
}
