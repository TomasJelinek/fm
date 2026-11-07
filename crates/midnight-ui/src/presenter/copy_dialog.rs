//! A copy draft retains shared identities; only backend admission expands selected paths.
use super::*;

#[derive(Clone)]
pub(super) struct CopyContext {
    source: PathBuf,
    destination: PathBuf,
}

pub(crate) struct CopyDraft {
    pub(crate) token: String,
    pub(super) input: CopyDraftInput,
    pub(crate) source: String,
    pub(crate) destination: PathBuf,
    pub(crate) summary: String,
    pub(crate) error: String,
}

#[derive(Clone)]
pub(super) enum CopyDraftInput {
    Native(SelectedFiles),
    Archive(ArchiveCopyRequest),
}

impl CopyDraftInput {
    fn roots(&self) -> usize {
        match self {
            Self::Native(files) => files
                .selection
                .len()
                .max(usize::from(files.fallback.is_some())),
            Self::Archive(files) => files
                .selected
                .len()
                .max(usize::from(files.fallback.is_some())),
        }
    }

    fn command(self, id: OperationId, destination: PathBuf) -> Command {
        match self {
            Self::Native(files) => Command::EnqueueSelectedCopy(SelectedCopyRequest {
                id,
                files,
                destination,
            }),
            Self::Archive(mut request) => {
                request.id = id;
                request.destination = destination;
                Command::EnqueueArchiveCopy(request)
            }
        }
    }
}

impl Presenter {
    pub(crate) fn request_copy_dialog(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if self.operations.closing
            || self.copies.draft.is_some()
            || !self.native_action_allowed(pane.other())
        {
            return;
        }
        if self.panes[pane.index()].archive.is_some() {
            self.request_archive_copy_dialog(pane);
            return;
        }
        let context = CopyContext {
            source: self.panes[pane.index()].display_path().to_owned(),
            destination: self.panes[pane.other().index()].display_path().to_owned(),
        };
        self.request_selection_action(
            pane,
            selection_actions::SelectionIntent::CopyDialog(context),
            send,
        );
    }

    pub(super) fn open_captured_copy(
        &mut self,
        request: DesktopRequestId,
        context: CopyContext,
        files: SelectedFiles,
    ) {
        if self.operations.closing || self.copies.draft.is_some() {
            self.retired.push(Box::new(files));
            return;
        }
        let summary = if files.selection.is_empty() {
            files
                .fallback
                .as_deref()
                .and_then(Path::file_name)
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        } else {
            format!(
                "{} selected items",
                self.number_locale.format_u64(files.selection.len() as u64)
            )
        };
        self.copies.draft = Some(CopyDraft {
            token: format!("copy-draft:{}", request.0),
            input: CopyDraftInput::Native(files),
            source: context.source.to_string_lossy().into_owned(),
            destination: context.destination,
            summary,
            error: String::new(),
        });
    }

    pub(crate) fn confirm_copy_dialog(
        &mut self,
        token: &str,
        destination: &str,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(draft) = self
            .copies
            .draft
            .as_mut()
            .filter(|draft| draft.token == token)
        else {
            return;
        };
        // Unchanged lossy presentation must never replace the original native identity.
        let destination = if draft.destination.to_string_lossy() == destination {
            draft.destination.clone()
        } else {
            PathBuf::from(destination)
        };
        if !destination.is_absolute() || destination.as_os_str().as_encoded_bytes().contains(&0) {
            draft.error = "Enter the full path of an existing destination folder.".into();
            return;
        }
        let input = draft.input.clone();
        let roots = input.roots() as u64;
        let target = destination.clone();
        if self.submit_copy(
            destination,
            roots,
            move |id| input.command(id, target),
            send,
        ) {
            self.retire_copy_draft();
        }
    }

    fn request_archive_copy_dialog(&mut self, pane: PaneId) {
        let p = &self.panes[pane.index()];
        let Some(archive) = &p.archive else {
            return;
        };
        if p.loading || archive.selection_pending || !p.search_allows_activation() {
            self.desktop.report(
                "Selection still loading",
                "Wait for the archive listing and selection to finish before copying.".into(),
            );
            return;
        }
        if archive.location.mount.is_none() {
            return;
        }
        let focused = (!p.parent_cursor)
            .then_some(p.cursor)
            .flatten()
            .and_then(|id| p.entry_by_id(id));
        if archive.selected.is_empty() && focused.is_none() {
            return;
        }
        let summary = if archive.selected.is_empty() {
            focused
                .map(|entry| entry.display_name.clone())
                .unwrap_or_default()
        } else {
            "Selected archive items in the current view".into()
        };
        let destination = self.panes[pane.other().index()].display_path().to_owned();
        let source = archive.location.display_text();
        let input = CopyDraftInput::Archive(ArchiveCopyRequest {
            id: OperationId(0), // Assigned only after the user submits the captured draft.
            location: archive.location.clone(),
            entries: p.entries.clone(),
            view: p.display_index().clone(),
            selected: archive.selected.clone(),
            fallback: focused.map(|entry| entry.id),
            destination: destination.clone(),
        });
        let request = self.desktop.request();
        self.copies.draft = Some(CopyDraft {
            token: format!("copy-draft:{}", request.0),
            input,
            source,
            destination,
            summary,
            error: String::new(),
        });
    }

    pub(crate) fn cancel_copy_dialog(&mut self, token: &str) {
        if self
            .copies
            .draft
            .as_ref()
            .is_some_and(|draft| draft.token == token)
        {
            self.retire_copy_draft();
        }
    }

    pub(super) fn retire_copy_draft(&mut self) {
        if let Some(draft) = self.copies.draft.take() {
            self.retired.push(Box::new(draft));
        }
    }
}
