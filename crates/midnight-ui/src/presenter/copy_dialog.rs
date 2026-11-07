//! A copy draft retains shared identities; only backend admission expands selected paths.
use super::*;

#[derive(Clone)]
pub(super) struct CopyContext {
    source: PathBuf,
    destination: PathBuf,
}

pub(crate) struct CopyDraft {
    pub(crate) token: String,
    pub(crate) files: SelectedFiles,
    pub(crate) source: PathBuf,
    pub(crate) destination: PathBuf,
    pub(crate) summary: String,
    pub(crate) error: String,
}

impl Presenter {
    pub(crate) fn request_copy_dialog(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if self.operations.closing
            || self.copies.draft.is_some()
            || !self.native_action_allowed(pane)
            || !self.native_action_allowed(pane.other())
        {
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
            files,
            source: context.source,
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
        let files = draft.files.clone();
        let roots = files
            .selection
            .len()
            .max(usize::from(files.fallback.is_some())) as u64;
        let target = destination.clone();
        if self.submit_copy(
            destination,
            roots,
            move |id| {
                Command::EnqueueSelectedCopy(SelectedCopyRequest {
                    id,
                    files,
                    destination: target,
                })
            },
            send,
        ) {
            self.retire_copy_draft();
        }
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
