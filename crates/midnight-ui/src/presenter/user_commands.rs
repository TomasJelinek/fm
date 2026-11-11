//! Captured command activations and bounded launch receipts. No parsing or process I/O.
use super::*;

#[derive(Default)]
pub(crate) struct CommandLaunches {
    next: u64,
    next_submission: u64,
    pub(crate) jobs: Vec<LaunchJob>,
    pub(crate) preview_ready: Option<UserCommandRequestId>,
    preview_requested: Option<UserCommandRequestId>,
}

pub(crate) struct LaunchJob {
    capture: u64,
    pub(crate) id: UserCommandRequestId,
    pub(crate) command: UserCommandId,
    pub(crate) name: String,
    pub(crate) preview_only: bool,
    pub(crate) accepted: bool,
    pub(crate) state: &'static str,
    pub(crate) started: u64,
    pub(crate) stopping: bool,
    pub(crate) finished: bool,
    pub(crate) error: Option<FsError>,
    pub(crate) summary: Option<UserCommandSummary>,
    pub(crate) preview: Option<Arc<UserCommandPreview>>,
    pub(crate) settings_preview: Option<SettingsPreviewOrigin>,
}

#[derive(Clone)]
pub(crate) struct SettingsPreviewOrigin {
    token: String,
    edit_revision: u64,
    preview_revision: u64,
}

impl SettingsPreviewOrigin {
    pub(crate) fn matches(&self, state: &user_command_settings::State) -> bool {
        self.token == state.token()
            && self.edit_revision == state.edit_revision
            && self.preview_revision == state.preview_revision
    }
}

#[derive(Clone)]
pub(super) struct CommandCapture {
    request: UserCommandRequest,
    name: String,
    preview_only: bool,
    pub(super) settings_preview: Option<SettingsPreviewOrigin>,
}

impl Presenter {
    pub(crate) fn current_settings_command_preview(&self) -> Option<&LaunchJob> {
        self.command_launches.jobs.iter().rev().find(|job| {
            job.settings_preview
                .as_ref()
                .is_some_and(|origin| origin.matches(&self.user_command_settings))
        })
    }

    pub(crate) fn request_user_command(
        &mut self,
        pane: PaneId,
        id: &str,
        preview_only: bool,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        self.capture_user_command(
            pane,
            self.preferences.values.user_commands.clone(),
            id,
            preview_only,
            None,
            send,
        )
    }

    pub(crate) fn preview_user_command_configuration(
        &mut self,
        pane: PaneId,
        configuration: Arc<Vec<UserCommand>>,
        id: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        self.user_command_settings.preview_revision = self
            .user_command_settings
            .preview_revision
            .saturating_add(1);
        let origin = SettingsPreviewOrigin {
            token: self.user_command_settings.token(),
            edit_revision: self.user_command_settings.edit_revision,
            preview_revision: self.user_command_settings.preview_revision,
        };
        self.capture_user_command(pane, configuration, id, true, Some(origin), send)
    }

    fn capture_user_command(
        &mut self,
        pane: PaneId,
        configuration: Arc<Vec<UserCommand>>,
        id: &str,
        preview_only: bool,
        settings_preview: Option<SettingsPreviewOrigin>,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        // An unsaved editor draft can be the configuration's last owner. Keep
        // final destruction off the UI even if validation or transport refuses it.
        self.retired.push(Box::new(configuration.clone()));
        if self.operations.closing || !self.preferences.loaded {
            return false;
        }
        let Some(command) = configuration
            .iter()
            .find(|c| c.id.as_str() == id && (preview_only || c.enabled))
        else {
            return false;
        };
        if self
            .command_launches
            .jobs
            .iter()
            .filter(|j| !j.finished)
            .count()
            >= 32
        {
            self.report_user_command_error(
                settings_preview.as_ref(),
                "Custom Commands",
                "The launch queue is full. Wait for a launch to finish or cancel a queued launch."
                    .into(),
            );
            return false;
        }
        let Some(next) = self.command_launches.next.checked_add(1) else {
            return false;
        };
        self.command_launches.next = next;
        let p = &self.panes[pane.index()];
        let other = &self.panes[pane.other().index()];
        if p.loading || p.search_view_waiting() {
            self.report_user_command_error(
                settings_preview.as_ref(),
                "Custom Commands",
                "Wait for the current panel listing to finish.".into(),
            );
            return false;
        }
        let context = UserCommandContext {
            active: p.native_capabilities().then(|| p.display_path().to_owned()),
            other: other
                .native_capabilities()
                .then(|| other.display_path().to_owned()),
            files: None,
            focus: (p.native_capabilities() && !p.parent_cursor)
                .then(|| p.cursor.and_then(|id| p.entry_by_id(id)).cloned())
                .flatten(),
        };
        let capture = CommandCapture {
            name: command.name.clone(),
            request: UserCommandRequest {
                id: UserCommandRequestId(next),
                command: command.id.clone(),
                configuration,
                context,
            },
            preview_only,
            settings_preview,
        };
        if p.native_capabilities() {
            self.request_selection_action(
                pane,
                selection_actions::SelectionIntent::UserCommand(Box::new(capture)),
                send,
            );
        } else {
            // Missing native values stay explicit; a literal command can still use
            // an explicitly configured native working directory from a virtual pane.
            return self.submit_user_command(capture, None, send);
        }
        self.command_launches
            .jobs
            .iter()
            .any(|job| job.capture == next)
            || matches!(self.selection_actions[pane.index()].as_ref().map(|p| &p.intent),
                Some(selection_actions::SelectionIntent::UserCommand(capture))
                if capture.request.id.0 == next)
    }

    pub(super) fn submit_user_command(
        &mut self,
        mut capture: CommandCapture,
        files: Option<SelectedFiles>,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        if self.operations.closing
            || capture
                .settings_preview
                .as_ref()
                .is_some_and(|origin| !origin.matches(&self.user_command_settings))
        {
            self.retired.push(Box::new((capture, files)));
            return false;
        }
        if self
            .command_launches
            .jobs
            .iter()
            .filter(|job| !job.finished)
            .count()
            >= 32
        {
            self.report_user_command_error(
                capture.settings_preview.as_ref(),
                "Custom Commands",
                "The launch queue filled while selection was loading. Try the command again."
                    .into(),
            );
            self.retired.push(Box::new((capture, files)));
            return false;
        }
        if let Some(files) = &files {
            // The bool transport consumes a refused payload. Retain shared
            // listing/selection allocations until backend reclamation drains them.
            self.retired
                .push(Box::new((files.view.clone(), files.selection.clone())));
        }
        capture.request.context.files = files;
        // Selection replies from two panes can arrive in reverse gesture order.
        // Backend IDs follow actual submission, independently of those captures.
        let Some(submission) = self.command_launches.next_submission.checked_add(1) else {
            self.retired.push(Box::new(capture));
            return false;
        };
        self.command_launches.next_submission = submission;
        let capture_id = capture.request.id.0;
        capture.request.id = UserCommandRequestId(submission);
        let job = LaunchJob {
            capture: capture_id,
            id: capture.request.id,
            command: capture.request.command.clone(),
            name: capture.name,
            preview_only: capture.preview_only,
            accepted: false,
            state: "Queued",
            started: 0,
            stopping: false,
            finished: false,
            error: None,
            summary: None,
            preview: None,
            settings_preview: capture.settings_preview,
        };
        let command = if capture.preview_only {
            Command::PreviewUserCommand(Box::new(capture.request))
        } else {
            Command::ExecuteUserCommand(Box::new(capture.request))
        };
        if send(command) {
            if job.settings_preview.is_some() {
                self.user_command_settings.clear_status();
            }
            self.command_launches.preview_ready = None;
            self.command_toolbar.invalidate_preview();
            self.command_launches.preview_requested = job.preview_only.then_some(job.id);
            for old in &mut self.command_launches.jobs {
                if let Some(preview) = old.preview.take() {
                    self.retired.push(Box::new(preview));
                    old.state = "Preview details released";
                }
            }
            self.command_launches.jobs.push(job);
            self.trim_command_history();
            true
        } else {
            self.report_user_command_error(
                job.settings_preview.as_ref(),
                "Custom Commands",
                "Request queue is busy. The command was not submitted.".into(),
            );
            false
        }
    }

    pub(super) fn report_user_command_error(
        &mut self,
        origin: Option<&SettingsPreviewOrigin>,
        title: &str,
        message: String,
    ) {
        if let Some(origin) = origin {
            if origin.matches(&self.user_command_settings) {
                self.user_command_settings.status.clear();
                self.user_command_settings.fail(message);
            }
        } else {
            self.desktop.report(title, message);
        }
    }

    pub(crate) fn cancel_user_command(&mut self, token: &str, send: &dyn Fn(Command) -> bool) {
        let Some(id) = token
            .strip_prefix("command-job:")
            .and_then(|id| id.parse::<u64>().ok())
        else {
            return;
        };
        if let Some(job) = self
            .command_launches
            .jobs
            .iter_mut()
            .find(|j| j.id.0 == id && !j.finished && !j.stopping)
        {
            if send(Command::CancelUserCommand(job.id)) {
                job.stopping = true;
                job.state = "Stopping future launches";
            } else {
                self.desktop.report(
                    "Custom Commands",
                    "Cancellation queue is busy. Try Cancel again.".into(),
                );
            }
        }
    }

    pub(super) fn apply_user_command_event(&mut self, event: Event) -> Option<Event> {
        let Event::UserCommand(event) = event else {
            return Some(event);
        };
        let Some(job) = self
            .command_launches
            .jobs
            .iter_mut()
            .find(|j| j.id == event.request && j.command == event.command && !j.finished)
        else {
            self.retired.push(Box::new(event));
            return None;
        };
        let mut failure = None;
        match event.update {
            UserCommandUpdate::Queued => {
                job.accepted = true;
            }
            UserCommandUpdate::Preparing => {
                job.accepted = true;
                job.state = "Preparing";
            }
            UserCommandUpdate::Started { index, .. } => {
                job.accepted = true;
                job.started = job.started.max(index.saturating_add(1));
                job.state = "Launching";
            }
            UserCommandUpdate::Preview(preview) => {
                job.accepted = true;
                job.finished = true;
                job.state = "Preview ready";
                job.summary = Some(UserCommandSummary {
                    planned: preview.processes.len() as u64,
                    started: 0,
                    not_started: preview.processes.len() as u64,
                    cancelled: false,
                    error: None,
                });
                if self.command_launches.preview_requested == Some(job.id) {
                    self.command_launches.preview_ready =
                        job.settings_preview.is_none().then_some(job.id);
                    if let Some(old) = job.preview.replace(preview) {
                        self.retired.push(Box::new(old));
                    }
                } else {
                    self.retired.push(Box::new(preview));
                    job.state = "Preview details released";
                }
            }
            UserCommandUpdate::Rejected(error) => {
                if !job.accepted {
                    job.finished = true;
                    job.state = "Not launched";
                    job.error = Some(error.clone());
                }
                failure = Some((
                    "Custom Command not accepted",
                    crate::error_presentation::describe(&error),
                ));
            }
            UserCommandUpdate::Finished(summary) => {
                job.finished = true;
                job.started = summary.started;
                job.state = if summary.cancelled {
                    "Cancelled"
                } else if summary.error.is_some() {
                    "Launch failed"
                } else if job.preview_only {
                    "Preview ready"
                } else {
                    "Launch completed"
                };
                job.error = summary.error.clone();
                if let Some(error) = &summary.error {
                    failure = Some((
                        "Custom Command needs attention",
                        format!(
                            "{}\n\n{} processes started · {} not started\n\n{}",
                            job.name,
                            self.number_locale.format_u64(summary.started),
                            self.number_locale.format_u64(summary.not_started),
                            crate::error_presentation::describe(error)
                        ),
                    ));
                }
                job.summary = Some(summary);
            }
        }
        if job.finished
            && (job.error.is_some()
                || job
                    .summary
                    .as_ref()
                    .is_some_and(|summary| summary.cancelled))
        {
            self.copies.automatic_jobs.retain_attention();
        }
        if let Some((title, message)) = failure {
            let origin = job.settings_preview.clone();
            self.report_user_command_error(origin.as_ref(), title, message);
        }
        self.trim_command_history();
        None
    }

    fn trim_command_history(&mut self) {
        while self
            .command_launches
            .jobs
            .iter()
            .filter(|j| j.finished)
            .count()
            > 16
        {
            let Some(index) = self.command_launches.jobs.iter().position(|j| j.finished) else {
                break;
            };
            self.retired
                .push(Box::new(self.command_launches.jobs.remove(index)));
        }
    }
}

impl Presenter {
    pub(super) fn clear_finished_command_jobs(&mut self, without_errors: bool) {
        for index in (0..self.command_launches.jobs.len()).rev() {
            let job = &self.command_launches.jobs[index];
            if !job.finished
                || (without_errors
                    && (job.error.is_some()
                        || job
                            .summary
                            .as_ref()
                            .is_some_and(|summary| summary.error.is_some())))
            {
                continue;
            }
            let id = job.id;
            let job = self.command_launches.jobs.remove(index);
            if self.command_launches.preview_ready == Some(id) {
                self.command_launches.preview_ready = None;
            }
            if self.command_launches.preview_requested == Some(id) {
                self.command_launches.preview_requested = None;
            }
            self.retired.push(Box::new(job));
        }
    }
}
