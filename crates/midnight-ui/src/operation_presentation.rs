use super::*;

#[path = "delete_summary.rs"]
mod delete_summary;

/// A Delete confirmation of `preview`, with the folder sizes the panels have measured
/// (nothing is read from disk to open it).
fn delete_confirmation(
    presenter: &Presenter,
    preview: &OperationPreview,
    mode: DeleteMode,
) -> delete_summary::Confirmation {
    let folder_size = |path: &std::path::Path| {
        presenter
            .panes
            .iter()
            .find_map(|pane| pane.folder_sizes.get(path).cloned())
    };
    let context = delete_summary::Context {
        locale: &presenter.number_locale,
        folder_size: &folder_size,
        source: operation_source(presenter, preview.id),
    };
    delete_summary::confirmation(preview, mode, &context)
}

fn operation_source(presenter: &Presenter, id: OperationId) -> Option<&std::path::Path> {
    presenter
        .operations
        .progress
        .iter()
        .find(|progress| progress.id == id)
        .map(|progress| progress.source_directory.as_path())
}

/// The directory the name is relative to. A rename from an expanded tree or
/// search result can target a child of a different directory than the pane.
pub(super) fn folder_location(presenter: &Presenter) -> Option<&std::path::Path> {
    let preview = presenter.operations.preview.as_ref()?;
    match preview.kind {
        MutationKind::CreateDirectory => operation_source(presenter, preview.id),
        MutationKind::Rename => preview
            .sample
            .first()
            .filter(|item| matches!(item.kind, EntryKind::Directory | EntryKind::DirectoryLink))?
            .path
            .parent(),
        _ => None,
    }
}

/// What the shared form draws of the pending Delete confirmation besides its warning.
pub(super) struct DeleteForm {
    pub(super) question: SharedString,
    /// A folder's path is marked `path`, with its last component as the explanation.
    pub(super) details: Vec<DetailRow>,
    pub(super) labels: SharedString,
    /// That folder's components as the path bar has them: the form drops whole middle
    /// components when it does not fit, as the path bar does.
    pub(super) path_parts: Vec<PathPartRow>,
}

pub(super) fn delete_form(presenter: &Presenter) -> Option<DeleteForm> {
    let (preview, _) = presenter.operation_form()?;
    let MutationKind::Delete(mode) = preview.kind else {
        return None;
    };
    let confirmation = delete_confirmation(presenter, preview, mode);
    let details = confirmation
        .details
        .iter()
        .enumerate()
        .map(|(index, (label, value))| {
            let value = shown_text(value);
            let path = confirmation.path == Some(index);
            DetailRow {
                identity: if path { "path".into() } else { "".into() },
                explanation: if path {
                    crate::path_tail(&value)
                } else {
                    "".into()
                },
                label: (*label).into(),
                value,
            }
        })
        .collect();
    let path_parts = confirmation
        .folder
        .as_deref()
        .map(path_rows)
        .unwrap_or_default();
    Some(DeleteForm {
        question: shown_text(&confirmation.question),
        details,
        labels: confirmation.labels().into(),
        path_parts,
    })
}

pub(super) fn path_rows(path: &std::path::Path) -> Vec<PathPartRow> {
    crate::presenter::path_labels(path)
        .into_iter()
        .enumerate()
        .map(|(index, (label, prefix))| PathPartRow {
            identity: index.to_string().into(),
            label: shown_text(&label),
            prefix: shown_text(&prefix),
        })
        .collect()
}

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
        // Exit waited for live work, showing Jobs. Close every tool window
        // with the main window, or a visible Jobs window keeps the app running.
        close_tool_windows(window);
        let _ = window.hide();
    }
    let mut rows = job_presentation::rows(presenter);
    // Copy details reads the presenter's rows again, so it keeps the raw paths.
    for row in &mut rows {
        make_shown(&mut row.title);
        make_shown(&mut row.detail);
        make_shown(&mut row.current_path);
        make_shown(&mut row.transfer_directory);
        make_shown(&mut row.transfer_name);
        job_presentation::current_file_tails(row);
        make_shown(&mut row.transfer_outcome);
        make_shown(&mut row.file_progress_title);
    }
    // A progress report changes its job's row in place, so the Jobs list keeps its row
    // delegates; the model is replaced only when jobs come, go or change order. The
    // Jobs window's bindings follow rows changed in place (`job_presentation::find`).
    // Copy/Move/Run often have no native icon; an absent image never equals itself,
    // so rows compare by their presentation, not by plain equality.
    navigation_models::update(
        window.get_operations(),
        rows,
        |row| row.identity.clone(),
        |model| window.set_operations(model),
    );
    copy_presentation::refresh(window, presenter);
    let Some((preview, token)) = presenter.operation_form() else {
        window.set_operation_token("".into());
        return;
    };
    let token: SharedString = token.into();
    if window.get_operation_token() == token {
        // The same dialog stays open; only an existing-name question can appear in it.
        present_replace(window, state.replace_name.as_ref(), preview);
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
    let item_kind = preview.sample.first().map(|item| item.kind);
    let select = rename_selection_end(&name, item_kind);
    let delete = match kind {
        MutationKind::Delete(mode) => Some(delete_confirmation(presenter, preview, mode)),
        _ => None,
    };
    let body = match kind {
        MutationKind::Rename => preview.sample.first().map_or_else(
            || "Choose a new name.".into(),
            |item| format!("Current item\n{}", item.path.display()),
        ),
        MutationKind::CreateFile | MutationKind::CreateDirectory => {
            operation_source(presenter, preview.id).map_or_else(
                || "Enter a name.".into(),
                |path| format!("Location\n{}", path.display()),
            )
        }
        MutationKind::EditMetadata => String::new(),
        // The question first, then the details; the warning is the form's own line.
        MutationKind::Delete(_) => delete
            .as_ref()
            .map(|delete| delete.text())
            .unwrap_or_default(),
    };
    let form_title = match (kind, item_kind) {
        (MutationKind::Rename, Some(EntryKind::Directory)) => "Rename folder",
        (MutationKind::Rename, Some(EntryKind::DirectoryLink)) => "Rename folder link",
        _ => title(kind),
    };
    window.set_operation_title(form_title.into());
    let (role, kind_id, warning) = match kind {
        MutationKind::Rename => (ActionIcon::Rename, 0, ""),
        MutationKind::Delete(mode) => (
            ActionIcon::Delete,
            if mode == DeleteMode::Trash { 1 } else { 2 },
            delete.as_ref().map_or("", |delete| delete.warning.as_str()),
        ),
        MutationKind::CreateDirectory => (ActionIcon::NewFolder, 3, ""),
        MutationKind::CreateFile => (ActionIcon::NewFile, 4, ""),
        MutationKind::EditMetadata => return,
    };
    window.set_operation_icon(
        presenter
            .desktop
            .image(&IconKey::action(role), IconPlace::Action),
    );
    window.set_operation_kind(kind_id);
    window.set_operation_warning(warning.into());
    window.set_operation_body(shown_text(&body));
    // The name field edits the raw name: what it holds is what the operation receives.
    window.set_operation_name(name.into());
    window.set_operation_selection_end(select as i32);
    window.set_operation_has_name(has_name);
    window.set_operation_dangerous(kind == MutationKind::Delete(DeleteMode::Permanent));
    window.set_operation_token(token);
    present_replace(window, state.replace_name.as_ref(), preview);
}

/// A new empty file over an existing name: the dialog asks before replacing it.
fn present_replace(
    window: &MainWindow,
    replace_name: Option<&(OperationId, String)>,
    preview: &OperationPreview,
) {
    let Some((_, name)) = replace_name
        .filter(|(id, _)| preview.kind == MutationKind::CreateFile && *id == preview.id)
    else {
        return;
    };
    window.set_operation_title("Replace file".into());
    // Enter in the name field does not answer this question (see OperationDialog).
    window.set_operation_warning(
        format!(
            "{} already exists. Choose the Replace file button to replace it with an empty \
             file; the original is kept in a recovery folder. Or change the name.",
            shown_text(name)
        )
        .into(),
    );
    window.set_operation_dangerous(true);
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

    /// A Delete confirmation's folder splits into the path bar's components: the root
    /// (a drive, `/` or a UNC share) first, an archive like any other folder.
    #[test]
    fn a_folder_splits_into_the_path_bars_components() {
        let separator = std::path::MAIN_SEPARATOR_STR;
        let path = std::path::PathBuf::from(separator)
            .join("data")
            .join("photos.zip")
            .join("2026");
        let labels = crate::presenter::path_labels(&path);
        let expected: Vec<(String, String)> = vec![
            (separator.to_owned(), separator.to_owned()),
            ("data".to_owned(), format!("{separator}data")),
            (
                format!("{separator}photos.zip"),
                format!("{separator}data{separator}photos.zip"),
            ),
            (format!("{separator}2026"), path.display().to_string()),
        ];
        assert_eq!(labels, expected);
        if cfg!(windows) {
            let unc = crate::presenter::path_labels(std::path::Path::new(
                r"\\server\share\Projects\2026",
            ));
            let labels: Vec<_> = unc.iter().map(|(label, _)| label.as_str()).collect();
            assert_eq!(labels, [r"\\server\share\", "Projects", r"\2026"]);
            assert_eq!(unc[2].1, r"\\server\share\Projects\2026");
        }
    }

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
