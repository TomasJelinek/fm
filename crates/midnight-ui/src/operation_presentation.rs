use super::*;

#[path = "delete_summary.rs"]
mod delete_summary;

pub(super) fn title(kind: MutationKind) -> &'static str {
    match kind {
        MutationKind::Rename => "Rename",
        MutationKind::EditMetadata => "Attributes, times and case",
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
    let rows = job_presentation::rows(presenter);
    update_model(window.get_operations(), rows, |model| {
        window.set_operations(model)
    });
    copy_presentation::refresh(window, presenter);
    let Some(preview) = &state.preview else {
        window.set_operation_token("".into());
        return;
    };
    let token: SharedString = preview.id.0.to_string().into();
    if window.get_operation_token() == token {
        return;
    }
    let kind = preview.kind;
    if kind == MutationKind::EditMetadata {
        window.set_operation_token("".into());
        return;
    }
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
        MutationKind::CreateDirectory => "Enter an absolute path or a path relative to this panel, including '..'. Missing folders are created; existing intermediate folders are reused. Links are not followed.".into(),
        MutationKind::Rename => "Rename the focused entry. Existing names are never overwritten.".into(),
        MutationKind::EditMetadata => String::new(),
        MutationKind::Delete(mode) => format!("{}\n\n{}", delete_summary::summary(preview, &presenter.number_locale),
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
    if !matches!(kind, MutationKind::Delete(_)) {
        for item in &preview.sample {
            let size = item.metadata.size.map_or_else(
                || "—".into(),
                |bytes| {
                    size_presentation::file_size(
                        bytes,
                        SizeDisplay::Bytes,
                        &presenter.number_locale,
                    )
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
    }
    window.set_operation_title(title(kind).into());
    let (role, kind_id, warning) = match kind {
        MutationKind::Rename => (ActionIcon::Rename, 0, ""),
        MutationKind::Delete(DeleteMode::Trash) => (
            ActionIcon::Delete,
            1,
            "Selected items will be removed from this folder and moved to Trash.",
        ),
        MutationKind::Delete(DeleteMode::Permanent) => (
            ActionIcon::Delete,
            2,
            "Permanent deletion cannot be undone.",
        ),
        MutationKind::CreateDirectory => (ActionIcon::NewFolder, 3, ""),
        MutationKind::CreateFile => (ActionIcon::NewFile, 4, ""),
        MutationKind::EditMetadata => return,
    };
    window.set_operation_icon(presenter.desktop.image(&IconKey::action(role)));
    window.set_operation_kind(kind_id);
    window.set_operation_warning(warning.into());
    window.set_operation_body(body.into());
    window.set_operation_name(name.into());
    window.set_operation_selection_end(select as i32);
    window.set_operation_has_name(has_name);
    window.set_operation_dangerous(kind == MutationKind::Delete(DeleteMode::Permanent));
    window.set_operation_token(token);
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
