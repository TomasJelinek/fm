//! Diagnostic gestures capture retained reports; they never operate on display paths.
use super::*;

struct Pending {
    request: DesktopRequestId,
    cancel: Cancellation,
    title: String,
}

#[derive(Default)]
pub(super) struct State {
    pending: Option<Pending>,
    ready: Option<ViewerLaunch>,
}

pub(super) struct PanelError {
    generation: u64,
    text: String,
    content: Arc<dyn GeneratedTextSource>,
}

impl Pane {
    pub(crate) fn error_revision(&self) -> String {
        format!("{}:{}", self.generation, self.error_revision)
    }

    pub(super) fn set_error(&mut self, text: impl Into<String>, retired: &mut Vec<DeferredDrop>) {
        retired.push(Box::new(self.error_details.take()));
        self.error = text.into();
        self.error_revision = self.error_revision.wrapping_add(1);
    }

    pub(super) fn set_tree_error(
        &mut self,
        error: &TreeDiagnostic,
        prefix: &str,
        retired: &mut Vec<DeferredDrop>,
    ) {
        self.error = format!("{prefix}{}", error.as_str());
        self.error_revision = self.error_revision.wrapping_add(1);
        retired.push(Box::new(self.error_details.take()));
        self.error_details = error.details().map(|content| PanelError {
            generation: self.generation,
            text: self.error.clone(),
            content: content.clone(),
        });
    }
}

impl Presenter {
    pub(crate) fn copy_panel_error(
        &mut self,
        pane: PaneId,
        text: &str,
        send: &dyn Fn(Command) -> bool,
    ) {
        let p = &self.panes[pane.index()];
        let visible = if p.error.is_empty() {
            &self.volumes_error
        } else {
            &p.error
        };
        if self.operations.closing || text.is_empty() || visible != text {
            return;
        }
        let content = p
            .error_details
            .as_ref()
            .filter(|details| {
                !p.error.is_empty() && details.generation == p.generation && details.text == text
            })
            .map(|details| details.content.clone());
        let Some(content) = content else {
            self.queue_clipboard_text(text.to_owned(), send);
            return;
        };
        let request = self.desktop.request();
        if send(Command::CopyGeneratedText {
            request,
            content,
            cancel: Cancellation::default(),
        }) {
            self.desktop.requests.insert(request);
        } else {
            self.desktop
                .report("Clipboard queue busy", "Try copying again.".into());
        }
    }

    pub(crate) fn inspect_panel_error(&mut self, pane: PaneId, text: &str) {
        let p = &self.panes[pane.index()];
        if let Some(details) = &p.error_details
            && details.generation == p.generation
            && details.text == text
            && p.error == text
        {
            let content = details.content.clone();
            self.show_generated_diagnostic("Panel error details.txt".into(), content);
        } else {
            self.retire_warning_viewer();
            self.desktop.report("Panel error", text.to_owned());
        }
    }

    pub(crate) fn inspect_panel_warning(
        &mut self,
        pane: PaneId,
        identity: &str,
        tree_error: bool,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing {
            return;
        }
        let p = &self.panes[pane.index()];
        let Some(id) = identity
            .rsplit(':')
            .next()
            .and_then(|id| id.parse::<u64>().ok())
        else {
            return;
        };
        let entry = if let Some(view) = p.tree_view() {
            view.index
                .row_positions
                .get(&TreeRowId(id))
                .and_then(|index| view.rows.get(*index))
                .map(|row| &row.entry)
        } else {
            p.index
                .by_id
                .get(&EntryId(id))
                .and_then(|index| p.entries.get(*index))
        };
        let Some(entry) = entry.filter(|entry| p.row_identity(entry) == identity) else {
            return;
        };
        if tree_error {
            if p.tree_view().is_none() {
                let Some(error) = p.root_presence_diagnostic(entry.id) else {
                    return;
                };
                if let Some(content) = error.details().cloned() {
                    let title = format!("Folder error - {}.txt", entry.display_name);
                    self.show_generated_diagnostic(title, content);
                } else {
                    let mut body = error.as_str().to_owned();
                    body.push_str("\n\nFolder\n");
                    if let Some(path) = entry.native_path() {
                        crate::error_presentation::append_path(&mut body, path);
                    } else {
                        body.push_str(&entry.display_name);
                    }
                    self.retire_warning_viewer();
                    self.desktop.report("Folder listing error", body);
                }
                return;
            }
            let Some(row) = p.tree_view().and_then(|view| {
                view.index
                    .row_positions
                    .get(&TreeRowId(id))
                    .and_then(|index| view.rows.get(*index))
            }) else {
                return;
            };
            if let Some(content) = row.error_details.clone() {
                let title = format!("Folder error - {}.txt", row.entry.display_name);
                self.show_generated_diagnostic(title, content);
                return;
            }
            if let Some(error) = &row.error {
                let mut body = format!("{error}\n\nPath\n");
                crate::error_presentation::append_path(&mut body, &row.entry.path);
                self.retire_warning_viewer();
                self.desktop.report("Folder listing error", body);
            }
            return;
        }
        let Some(size) = p.folder_size(entry).filter(|size| size.errors > 0) else {
            return;
        };
        let Some(report) = size.warnings.clone() else {
            let mut body = format!(
                "{} issues were reported, but detailed diagnostics are unavailable for this result.\n\nPath\n",
                size.errors,
            );
            crate::error_presentation::append_path(&mut body, &entry.path);
            body.push_str("\n\nCalculate its size again to collect details.");
            self.retire_warning_viewer();
            self.desktop.report("Folder size warnings", body);
            return;
        };
        let title = format!("Size warnings - {}.txt", entry.display_name);
        let request = self.desktop.request();
        let cancel = Cancellation::default();
        if send(Command::OpenSizeWarnings {
            request,
            report,
            cancel: cancel.clone(),
        }) {
            self.retire_warning_viewer();
            self.panel_warnings.pending = Some(Pending {
                request,
                cancel,
                title,
            });
        } else {
            self.desktop.report(
                "Folder size warnings",
                "The report queue is busy. Try again.".into(),
            );
        }
    }

    pub(super) fn apply_size_warning_report(
        &mut self,
        request: DesktopRequestId,
        result: Result<Arc<dyn GeneratedTextSource>, FsError>,
    ) {
        if self.operations.closing
            || self
                .panel_warnings
                .pending
                .as_ref()
                .is_none_or(|pending| pending.request != request)
        {
            self.retired.push(Box::new(result));
            return;
        }
        let pending = self.panel_warnings.pending.take().expect("matched request");
        match result {
            Ok(content) => self.show_generated_diagnostic(pending.title, content),
            Err(error) => self.desktop.report(
                "Folder size warnings",
                crate::error_presentation::describe(&error),
            ),
        }
    }

    fn show_generated_diagnostic(&mut self, title: String, content: Arc<dyn GeneratedTextSource>) {
        self.retire_warning_viewer();
        let Some(disabled_plugins) = self.viewer_plugins.startup_disabled.clone() else {
            self.retired.push(Box::new(content));
            self.desktop.report(
                "Diagnostic details",
                "Viewer settings are still loading. Try again.".into(),
            );
            return;
        };
        self.panel_warnings.ready = Some(ViewerLaunch {
            associations: self.viewer_plugins.startup_associations.clone(),
            tree: None,
            generated: Some(content),
            preferences: Arc::new(self.preferences.values.viewer.clone()),
            archive: None,
            entries: Arc::from([]),
            view: Arc::new(DirectoryIndex::default()),
            focused: PathBuf::from(title),
            basic: true,
            disabled_plugins,
        });
    }

    /// Move a retained report to the ordinary asynchronous basic viewer service.
    pub fn take_warning_viewer(&mut self) -> Option<ViewerLaunch> {
        self.panel_warnings.ready.take()
    }

    pub(super) fn retire_warning_viewer(&mut self) {
        if let Some(pending) = self.panel_warnings.pending.take() {
            pending.cancel.cancel();
        }
        self.retired
            .push(Box::new(self.panel_warnings.ready.take()));
    }
}
