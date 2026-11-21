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
    diagnostic: Option<PendingDiagnostic>,
    notice_source: Option<NoticeSource>,
    pub(super) modal_errors: [Option<(u64, u64)>; 2],
}

struct PendingDiagnostic {
    request: DesktopRequestId,
    cancel: Cancellation,
    generation: i32,
}
struct NoticeSource {
    generation: i32,
    content: Arc<dyn GeneratedTextSource>,
}

/// What Copy details copies of a notice with a retained report: the message the
/// notice shows, then the report. The clipboard worker reads it; the UI never reads
/// the report.
#[derive(Debug)]
struct NoticeReport {
    header: String,
    report: Arc<dyn GeneratedTextSource>,
}

impl GeneratedTextSource for NoticeReport {
    fn length(&self) -> u64 {
        (self.header.len() as u64).saturating_add(self.report.length())
    }

    fn read_at(&self, offset: u64, output: &mut [u8]) -> std::io::Result<usize> {
        let header = self.header.as_bytes();
        match usize::try_from(offset) {
            Ok(start) if start < header.len() => {
                let count = (header.len() - start).min(output.len());
                output[..count].copy_from_slice(&header[start..start + count]);
                Ok(count)
            }
            _ => self.report.read_at(offset - header.len() as u64, output),
        }
    }
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

    /// Report congestion of a send that is retried automatically. A repeated
    /// refusal does not re-signal the unchanged notice.
    pub(super) fn report_busy(&mut self, notice: &'static str, retired: &mut Vec<DeferredDrop>) {
        if self.error != notice {
            self.set_error(notice, retired);
        }
    }

    /// An accepted retry clears its own congestion notice, never another error.
    pub(super) fn clear_busy(&mut self, notice: &'static str, retired: &mut Vec<DeferredDrop>) {
        if self.error == notice {
            self.set_error("", retired);
        }
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
        // The panel shows the error with escaped bidirectional controls; the error itself is
        // copied.
        if self.operations.closing
            || text.is_empty()
            || (visible != text && visible_name(visible) != text)
        {
            return;
        }
        let text = visible.clone();
        let content = p
            .error_details
            .as_ref()
            .filter(|details| {
                !p.error.is_empty() && details.generation == p.generation && details.text == text
            })
            .map(|details| details.content.clone());
        let Some(content) = content else {
            self.queue_clipboard_text(text, send);
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

    pub(crate) fn inspect_panel_error(
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
        // The panel shows the error with escaped bidirectional controls (`copy_panel_error`).
        if self.operations.closing
            || text.is_empty()
            || (visible != text && visible_name(visible) != text)
        {
            return;
        }
        let text = visible.clone();
        if let Some(details) = &p.error_details
            && details.generation == p.generation
            && details.text == text
            && p.error == text
        {
            let content = details.content.clone();
            self.show_diagnostic_notice("Panel error", text, content, send);
        } else {
            self.retire_warning_viewer();
            self.desktop.report("Panel error", text);
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
            let (message, content) = if let Some(view) = p.tree_view() {
                let Some(row) = view
                    .index
                    .row_positions
                    .get(&TreeRowId(id))
                    .and_then(|index| view.rows.get(*index))
                else {
                    return;
                };
                if row.error.is_none() && row.error_details.is_none() {
                    return;
                }
                (
                    row.error
                        .clone()
                        .unwrap_or_else(|| "Folder listing failed".into()),
                    row.error_details.clone(),
                )
            } else {
                let Some(error) = p.root_presence_diagnostic(entry.id) else {
                    return;
                };
                (error.as_str().to_owned(), error.details().cloned())
            };
            let path = entry.path.clone();
            self.show_folder_error(&message, &path, content);
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
                self.viewer_plugins.policy_unavailable().into(),
            );
            return;
        };
        self.panel_warnings.ready = Some(ViewerLaunch {
            find: None,
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
        if let Some(pending) = self.panel_warnings.diagnostic.take() {
            pending.cancel.cancel();
        }
        self.retired
            .push(Box::new(self.panel_warnings.notice_source.take()));
        if let Some(pending) = self.panel_warnings.pending.take() {
            pending.cancel.cancel();
        }
        self.retired
            .push(Box::new(self.panel_warnings.ready.take()));
    }
}

impl Presenter {
    pub(super) fn show_diagnostic_notice(
        &mut self,
        title: &str,
        body: String,
        content: Arc<dyn GeneratedTextSource>,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.retire_warning_viewer();
        self.desktop.report(title, body);
        self.desktop.notice_details = "Loading full details…".into();
        let generation = self.desktop.notice_generation;
        let request = self.desktop.request();
        let cancel = Cancellation::default();
        if send(Command::ReadDiagnosticText {
            request,
            content: content.clone(),
            cancel: cancel.clone(),
        }) {
            self.panel_warnings.diagnostic = Some(PendingDiagnostic {
                request,
                cancel,
                generation,
            });
            self.panel_warnings.notice_source = Some(NoticeSource {
                generation,
                content,
            });
        } else {
            self.retired.push(Box::new(content));
            self.desktop.notice_details = "The details queue is busy. Try again.".into();
        }
    }

    pub(super) fn apply_diagnostic_text(
        &mut self,
        request: DesktopRequestId,
        result: Result<DiagnosticTextPreview, FsError>,
    ) {
        if self
            .panel_warnings
            .diagnostic
            .as_ref()
            .is_none_or(|pending| pending.request != request)
        {
            self.retired.push(Box::new(result));
            return;
        }
        let pending = self
            .panel_warnings
            .diagnostic
            .take()
            .expect("matched diagnostic request");
        if self.operations.closing
            || pending.cancel.is_cancelled()
            || pending.generation != self.desktop.notice_generation
            || self.desktop.notice.is_none()
        {
            self.dismiss_diagnostic_notice(pending.generation);
            self.retired.push(Box::new(result));
            return;
        }
        // Copy details copies the message before the report.
        let header = self
            .notice_report_header()
            .map_or(0, |header| header.len() as u64);
        let body = match result {
            Ok(preview)
                if preview.text.len() <= DIAGNOSTIC_PREVIEW_BYTES
                    && preview.total_bytes >= preview.text.len() as u64 =>
            {
                if preview.truncated {
                    let copy = if preview.total_bytes.saturating_add(header) <= 4 * 1024 * 1024 {
                        "Copy details copies the complete report."
                    } else {
                        "The complete report exceeds the 4 MiB clipboard limit."
                    };
                    format!(
                        "{}\n\nShowing {} of {} bytes. {copy}",
                        preview.text,
                        preview.text.len(),
                        preview.total_bytes
                    )
                } else {
                    preview.text
                }
            }
            Ok(_) => "The diagnostic returned an invalid bounded preview.".into(),
            Err(error) => format!(
                "Could not load full details.\n\n{}",
                crate::error_presentation::describe(&error)
            ),
        };
        // Refresh the existing modal; do not create a new notice generation or viewer.
        self.desktop.notice_details = body;
    }

    pub(super) fn retain_notice_report(&mut self, content: Arc<dyn GeneratedTextSource>) {
        self.panel_warnings.notice_source = Some(NoticeSource {
            generation: self.desktop.notice_generation,
            content,
        });
    }

    /// The retained report of the shown notice, if it has one.
    fn notice_report(&self) -> Option<&Arc<dyn GeneratedTextSource>> {
        self.panel_warnings
            .notice_source
            .as_ref()
            .filter(|source| {
                source.generation == self.desktop.notice_generation && self.desktop.notice.is_some()
            })
            .map(|source| &source.content)
    }

    /// The notice's message as it shows it: title, reason and folder.
    fn notice_message(&self) -> Option<String> {
        let (title, body) = self.desktop.notice.as_ref()?;
        let mut text = format!("{title}\n\n{body}");
        if !self.desktop.notice_path.is_empty() {
            text.push_str("\n\nFolder\n");
            text.push_str(&self.desktop.notice_path);
        }
        Some(text)
    }

    /// What Copy details puts before a retained report: the whole message.
    fn notice_report_header(&self) -> Option<String> {
        self.notice_message()
            .map(|message| format!("{message}\n\nDetails\n"))
    }

    /// Copy details: the complete message the notice shows, then its diagnostic
    /// report: the retained one, the full error or the shown details.
    pub(crate) fn copy_notice_details(&mut self, send: &dyn Fn(Command) -> bool) {
        if let Some(report) = self.notice_report().cloned() {
            let header = self.notice_report_header().unwrap_or_default();
            // The backend reads the report after the message; the UI never reads it.
            let content: Arc<dyn GeneratedTextSource> = Arc::new(NoticeReport { header, report });
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
            return;
        }
        let Some(message) = self.notice_message() else {
            return;
        };
        // The full error, or the shown details unless they only repeat the reason.
        let details = self.desktop.notice_copy.as_deref().or_else(|| {
            let details = self.desktop.notice_details.as_str();
            let body = self.desktop.notice.as_ref().map(|(_, body)| body.as_str());
            (!details.is_empty() && Some(details) != body).then_some(details)
        });
        let text = match details {
            Some(details) => format!("{message}\n\nDetails\n{details}"),
            None => message,
        };
        self.copy_viewer_text(text, send);
    }

    pub(crate) fn dismiss_diagnostic_notice(&mut self, generation: i32) {
        if generation == self.desktop.notice_generation {
            self.desktop.notice_navigation = None;
        }
        self.properties_closed(generation);
        if self
            .panel_warnings
            .diagnostic
            .as_ref()
            .is_some_and(|pending| pending.generation == generation)
            && let Some(pending) = self.panel_warnings.diagnostic.take()
        {
            pending.cancel.cancel();
        }
        if self
            .panel_warnings
            .notice_source
            .as_ref()
            .is_some_and(|source| source.generation == generation)
        {
            self.retired
                .push(Box::new(self.panel_warnings.notice_source.take()));
        }
    }
}
