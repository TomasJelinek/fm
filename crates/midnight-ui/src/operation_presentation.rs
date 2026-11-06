use super::*;

fn title(kind: MutationKind) -> &'static str {
    match kind {
        MutationKind::Rename => "Rename",
        MutationKind::CreateFile => "New empty file",
        MutationKind::CreateDirectory => "Create directory",
        MutationKind::Delete(DeleteMode::Trash) => "Move to Trash",
        MutationKind::Delete(DeleteMode::Permanent) => "Permanently delete",
    }
}
pub(super) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let state = &presenter.operations;
    if state.closing && !presenter.has_live_operations() {
        let _ = window.hide();
    }
    let rows = state
        .progress
        .iter()
        .rev()
        .map(|progress| {
            let status = match progress.state {
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
            OperationRow {
                identity: progress.id.0.to_string().into(),
                title: format!("{} — {status}", title(progress.kind)).into(),
                detail: format!(
                    "{} / {} targets · {} errors · {} entries processed\n{}",
                    progress.completed,
                    progress.total,
                    progress.failed,
                    progress.processed_entries,
                    progress
                        .current_path
                        .as_deref()
                        .unwrap_or(&progress.source_directory)
                        .display()
                )
                .into(),
                cancellable: !progress.state.is_terminal(),
            }
        })
        .collect();
    update_model(window.get_operations(), rows, |model| {
        window.set_operations(model)
    });
    let Some(preview) = &state.preview else {
        window.set_operation_token("".into());
        return;
    };
    let token: SharedString = preview.id.0.to_string().into();
    if window.get_operation_token() == token {
        return;
    }
    let kind = preview.kind;
    let has_name = matches!(
        kind,
        MutationKind::Rename | MutationKind::CreateDirectory | MutationKind::CreateFile
    );
    let name = if kind == MutationKind::Rename {
        preview
            .sample
            .first()
            .and_then(|item| item.path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    } else {
        String::new()
    };
    let select = rename_selection_end(&name, preview.sample.first().map(|item| item.kind));
    let mut body = match kind {
        MutationKind::CreateFile => "Create an empty file without overwriting an existing name.".into(),
        MutationKind::CreateDirectory => "Enter a folder name or a relative nested path. Existing intermediate folders are reused; links are not followed.".into(),
        MutationKind::Rename => "Rename the focused entry. Existing names are never overwritten.".into(),
        MutationKind::Delete(mode) => format!("{} files and {} folders. {}", preview.files, preview.folders,
            if mode == DeleteMode::Trash { "Move these items to the system Trash? If unavailable, they will remain in place." }
            else { "Permanently delete these items and folder contents? This cannot be undone. Links are removed without deleting their targets." }),
    };
    if let Some(progress) = state
        .progress
        .iter()
        .find(|progress| progress.id == preview.id)
    {
        body = format!(
            "Location: {}\n\n{body}",
            progress.source_directory.display()
        );
    }
    for item in &preview.sample {
        let size = item.metadata.size.map_or_else(
            || "—".into(),
            |bytes| {
                size_presentation::file_size(bytes, SizeDisplay::Bytes, &presenter.number_locale)
            },
        );
        let date = time_presentation::modified(&item.metadata)
            .map(|value| format!("{} {}", value.date, value.time))
            .unwrap_or_else(|| "—".into());
        body.push_str(&format!(
            "\n{} · {size} bytes · {date}",
            item.path.display()
        ));
    }
    if preview.files + preview.folders > preview.sample.len() as u64 {
        body.push_str("\n… Additional selected targets included.");
    }
    window.set_operation_title(title(kind).into());
    window.set_operation_body(body.into());
    window.set_operation_name(name.into());
    window.set_operation_selection_end(select as i32);
    window.set_operation_has_name(has_name);
    window.set_operation_dangerous(kind == MutationKind::Delete(DeleteMode::Permanent));
    window.set_operation_token(token);
    window.invoke_begin_operation();
}

fn rename_selection_end(name: &str, kind: Option<EntryKind>) -> usize {
    if matches!(kind, Some(EntryKind::Directory | EntryKind::DirectoryLink)) {
        return name.len();
    }
    name.rfind('.')
        .filter(|index| *index > 0)
        .unwrap_or(name.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_selects_whole_dotted_directories_but_only_file_basename() {
        assert_eq!(
            rename_selection_end("folder.ext", Some(EntryKind::Directory)),
            10
        );
        assert_eq!(
            rename_selection_end("link.ext", Some(EntryKind::DirectoryLink)),
            8
        );
        assert_eq!(
            rename_selection_end("文档.txt", Some(EntryKind::File)),
            "文档".len()
        );
        assert_eq!(
            rename_selection_end(".gitignore", Some(EntryKind::File)),
            10
        );
    }
}
